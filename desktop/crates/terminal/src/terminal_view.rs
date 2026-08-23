use alacritty_terminal::term::cell::Flags;
use gpui::{
  App, ClipboardItem, Context, FocusHandle, Focusable, Font, FontFallbacks, FontFeatures,
  FontStyle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, Keystroke, Modifiers,
  MouseButton, ParentElement, Pixels, Render, ScrollWheelEvent, Styled, Task, TouchPhase, Window,
  actions, div, prelude::*, px, relative, rgb,
};
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_component::tooltip::Tooltip;
use std::time::Duration;
use std::{path::PathBuf, sync::Arc};

use crate::{
  ScreenSnapshot, TerminalBounds, TerminalSelectionMode, TerminalSession, ViewportPoint,
  ViewportSelectionRange, colors::TerminalPalette, terminal_element::TerminalElement,
};

const SESSION_POLL_INTERVAL: Duration = Duration::from_millis(16);
const TERMINAL_SCREEN_DEBUG_SELECTOR: &str = "terminal-screen-bounds";
const TERMINAL_SURFACE_DEBUG_SELECTOR: &str = "terminal-surface-bounds";
const TERMINAL_BANNER_DEBUG_SELECTOR: &str = "terminal-banner";

fn selection_mode_for_click_count(click_count: usize) -> TerminalSelectionMode {
  match click_count {
    2 => TerminalSelectionMode::Semantic,
    3.. => TerminalSelectionMode::Lines,
    _ => TerminalSelectionMode::Simple,
  }
}

#[derive(Clone)]
struct PreservedSelection {
  anchor: ViewportPoint,
  head: ViewportPoint,
  mode: TerminalSelectionMode,
  text: String,
  dragging: bool,
}

#[derive(Clone)]
struct PendingLinkActivation {
  uri: Arc<str>,
}

actions!(terminal, [SendTab, SendBackTab]);

/// Tab belongs to the shell, so the terminal has to claim it: the window's own
/// Tab moves the focus, and it runs before any key reaches the pty.
pub const TERMINAL_CONTEXT: &str = "Terminal";

pub struct TerminalView {
  focus_handle: FocusHandle,
  working_directory: Option<PathBuf>,
  session: Option<TerminalSession>,
  screen: ScreenSnapshot,
  last_bounds: TerminalBounds,
  error: Option<String>,
  selection_anchor: Option<ViewportPoint>,
  selection_head: Option<ViewportPoint>,
  selection_mode: TerminalSelectionMode,
  resolved_selection: Option<ViewportSelectionRange>,
  selection_dragging: bool,
  hovered_hyperlink: Option<Arc<str>>,
  pending_link_activation: Option<PendingLinkActivation>,
  last_reported_mouse_state: Option<(ViewportPoint, Option<MouseButton>)>,
  scroll_remainder: Pixels,
  _poll_task: Task<()>,
}

impl TerminalView {
  pub fn new(working_directory: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
    let poll_task = cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(SESSION_POLL_INTERVAL).await;
        let Some(this) = this.upgrade() else {
          break;
        };
        this.update(cx, |this, cx| this.poll_session(cx));
      }
    });

    let mut view = Self {
      focus_handle: cx.focus_handle(),
      working_directory: None,
      session: None,
      screen: ScreenSnapshot::default(),
      last_bounds: TerminalBounds::default(),
      error: None,
      selection_anchor: None,
      selection_head: None,
      selection_mode: TerminalSelectionMode::Simple,
      resolved_selection: None,
      selection_dragging: false,
      hovered_hyperlink: None,
      pending_link_activation: None,
      last_reported_mouse_state: None,
      scroll_remainder: px(0.0),
      _poll_task: poll_task,
    };
    view.set_working_directory(working_directory, cx);
    view
  }

  pub fn working_directory(&self) -> Option<&std::path::Path> {
    self.working_directory.as_deref()
  }

  pub fn set_working_directory(
    &mut self,
    working_directory: Option<PathBuf>,
    cx: &mut Context<Self>,
  ) {
    if self.working_directory == working_directory {
      return;
    }

    self.working_directory = working_directory;
    self.restart_session();
    cx.notify();
  }

  pub(crate) fn screen(&self) -> &ScreenSnapshot {
    &self.screen
  }

  pub(crate) fn selection_range(&self) -> Option<ViewportSelectionRange> {
    self.resolved_selection
  }

  pub(crate) fn start_selection(&mut self, point: ViewportPoint, mode: TerminalSelectionMode) {
    self.selection_anchor = Some(point);
    self.selection_head = Some(point);
    self.selection_mode = mode;
    self.selection_dragging = true;
    self.recompute_selection_range();
  }

  pub(crate) fn update_selection(&mut self, point: ViewportPoint) {
    if !self.selection_dragging {
      return;
    }
    self.selection_head = Some(point);
    self.recompute_selection_range();
  }

  pub(crate) fn finish_selection(&mut self, point: ViewportPoint) {
    if !self.selection_dragging {
      return;
    }

    self.selection_head = Some(point);
    self.selection_dragging = false;
    self.recompute_selection_range();
    if self.resolved_selection.is_none() {
      self.reset_selection();
    }
  }

  pub(crate) fn reset_selection(&mut self) {
    self.selection_anchor = None;
    self.selection_head = None;
    self.resolved_selection = None;
    self.selection_mode = TerminalSelectionMode::Simple;
    self.selection_dragging = false;
  }

  pub(crate) fn should_show_link_cursor(&self, point: ViewportPoint, modifiers: Modifiers) -> bool {
    self.hyperlink_activation_enabled(modifiers) && self.hyperlink_at(point).is_some()
  }

  pub(crate) fn update_hovered_hyperlink(
    &mut self,
    point: Option<ViewportPoint>,
    cx: &mut Context<Self>,
  ) {
    let next = point.and_then(|point| self.hyperlink_at(point));
    if self.hovered_hyperlink != next {
      self.hovered_hyperlink = next;
      cx.notify();
    }
  }

  fn extend_selection(&mut self, point: ViewportPoint) {
    if self.selection_anchor.is_none() {
      self.selection_anchor = Some(point);
    }
    self.selection_head = Some(point);
    self.selection_mode = TerminalSelectionMode::Simple;
    self.selection_dragging = true;
    self.recompute_selection_range();
  }

  fn recompute_selection_range(&mut self) {
    let (Some(anchor), Some(head)) = (self.selection_anchor, self.selection_head) else {
      self.resolved_selection = None;
      return;
    };

    self.resolved_selection = match self.selection_mode {
      TerminalSelectionMode::Simple => {
        let selection = ViewportSelectionRange {
          start: anchor,
          end: head,
        };
        (!selection.is_collapsed()).then_some(selection.normalized())
      }
      TerminalSelectionMode::Semantic | TerminalSelectionMode::Lines => self
        .session
        .as_ref()
        .and_then(|session| session.selection_range_for_mode(anchor, head, self.selection_mode)),
    };
  }

  pub(crate) fn should_handle_mouse_move(
    &self,
    hovered: bool,
    pressed_button: Option<MouseButton>,
    modifiers: Modifiers,
  ) -> bool {
    if self.pending_link_activation.is_some() {
      return pressed_button == Some(MouseButton::Left);
    }

    if self.local_mouse_selection_enabled(modifiers) {
      return self.selection_dragging;
    }

    if hovered {
      return self
        .session
        .as_ref()
        .is_some_and(|session| session.can_report_mouse_move(pressed_button));
    }

    pressed_button.is_some()
      && self
        .last_reported_mouse_state
        .is_some_and(|(_, tracked_button)| tracked_button == pressed_button)
      && self
        .session
        .as_ref()
        .is_some_and(|session| session.can_report_mouse_move(pressed_button))
  }

  pub(crate) fn should_handle_mouse_up(&self, button: MouseButton, modifiers: Modifiers) -> bool {
    if self.pending_link_activation.is_some() {
      return button == MouseButton::Left;
    }

    if self.local_mouse_selection_enabled(modifiers) {
      return button == MouseButton::Left && self.selection_dragging;
    }

    self
      .last_reported_mouse_state
      .is_some_and(|(_, tracked_button)| tracked_button == Some(button))
  }

  pub(crate) fn local_mouse_selection_enabled(&self, modifiers: Modifiers) -> bool {
    modifiers.shift
      || self
        .session
        .as_ref()
        .is_some_and(|session| !session.mouse_mode_enabled())
  }

  pub(crate) fn handle_mouse_down(
    &mut self,
    button: MouseButton,
    point: ViewportPoint,
    click_count: usize,
    modifiers: Modifiers,
    cx: &mut Context<Self>,
  ) {
    self.pending_link_activation = None;

    if button == MouseButton::Left
      && self.hyperlink_activation_enabled(modifiers)
      && let Some(uri) = self.hyperlink_at(point)
    {
      self.last_reported_mouse_state = None;
      self.pending_link_activation = Some(PendingLinkActivation { uri });
      cx.notify();
      return;
    }

    if self.local_mouse_selection_enabled(modifiers) {
      if button == MouseButton::Left {
        self.last_reported_mouse_state = None;
        let selection_mode = selection_mode_for_click_count(click_count);
        if selection_mode == TerminalSelectionMode::Simple && modifiers.shift {
          self.extend_selection(point);
        } else {
          self.start_selection(point, selection_mode);
        }
        cx.notify();
      }
      return;
    }

    self.reset_selection();
    if let Some(session) = self.session.as_mut()
      && session.send_mouse_press(button, point, modifiers)
    {
      self.last_reported_mouse_state = Some((point, Some(button)));
      cx.notify();
      return;
    }

    self.last_reported_mouse_state = None;
  }

  pub(crate) fn handle_mouse_move(
    &mut self,
    point: ViewportPoint,
    pressed_button: Option<MouseButton>,
    modifiers: Modifiers,
    cx: &mut Context<Self>,
  ) {
    if let Some(pending) = self.pending_link_activation.as_ref() {
      let still_hovering_same_link = pressed_button == Some(MouseButton::Left)
        && self
          .hyperlink_at(point)
          .is_some_and(|uri| uri.as_ref() == pending.uri.as_ref());
      if !still_hovering_same_link {
        self.pending_link_activation = None;
        cx.notify();
      }
      return;
    }

    if self.local_mouse_selection_enabled(modifiers) {
      if self.selection_dragging {
        self.update_selection(point);
        cx.notify();
      }
      return;
    }

    if self.last_reported_mouse_state == Some((point, pressed_button)) {
      return;
    }

    if let Some(session) = self.session.as_mut()
      && session.send_mouse_move(point, pressed_button, modifiers)
    {
      self.last_reported_mouse_state = Some((point, pressed_button));
      cx.notify();
    }
  }

  pub(crate) fn handle_mouse_up(
    &mut self,
    button: MouseButton,
    point: ViewportPoint,
    modifiers: Modifiers,
    cx: &mut Context<Self>,
  ) {
    self.last_reported_mouse_state = None;

    if let Some(pending) = self.pending_link_activation.take() {
      if button == MouseButton::Left
        && self.hyperlink_activation_enabled(modifiers)
        && self
          .hyperlink_at(point)
          .is_some_and(|uri| uri.as_ref() == pending.uri.as_ref())
      {
        cx.open_url(pending.uri.as_ref());
      }
      return;
    }

    if self.local_mouse_selection_enabled(modifiers) {
      if button == MouseButton::Left && self.selection_dragging {
        self.finish_selection(point);
        cx.notify();
      }
      return;
    }

    if let Some(session) = self.session.as_mut()
      && session.send_mouse_release(button, point, modifiers)
    {
      cx.notify();
    }
  }

  pub(crate) fn handle_scroll(
    &mut self,
    event: &ScrollWheelEvent,
    point: ViewportPoint,
    cx: &mut Context<Self>,
  ) {
    let Some(delta_lines) = self.scroll_lines_for_event(event) else {
      return;
    };

    let Some(session) = self.session.as_mut() else {
      return;
    };

    if session.send_scroll(delta_lines, point, event.modifiers) {
      self.reset_selection();
      self.hovered_hyperlink = None;
      cx.notify();
      return;
    }

    session.scroll_display(delta_lines);
    self.reset_selection();
    self.hovered_hyperlink = None;
    self.refresh_snapshot();
    cx.notify();
  }

  pub fn restart_session(&mut self) {
    self.error = None;
    self.reset_selection();
    self.hovered_hyperlink = None;
    self.pending_link_activation = None;
    self.last_reported_mouse_state = None;
    self.scroll_remainder = px(0.0);
    self.session = self.working_directory.clone().and_then(|cwd| {
      match TerminalSession::spawn(cwd, self.last_bounds) {
        Ok(session) => Some(session),
        Err(error) => {
          self.error = Some(error.to_string());
          None
        }
      }
    });

    self.refresh_snapshot();
  }

  fn refresh_snapshot(&mut self) {
    self.screen = self
      .session
      .as_ref()
      .map(TerminalSession::snapshot)
      .unwrap_or_default();
  }

  fn preserve_selection_before_refresh(&self) -> Option<PreservedSelection> {
    let range = self.resolved_selection?;
    let text = selection_text_from_screen(&self.screen, range)?;
    if text.is_empty() {
      return None;
    }

    Some(PreservedSelection {
      anchor: self.selection_anchor?,
      head: self.selection_head?,
      mode: self.selection_mode,
      text,
      dragging: self.selection_dragging,
    })
  }

  fn restore_selection_after_refresh(&mut self, preserved: Option<PreservedSelection>) {
    let Some(preserved) = preserved else {
      return;
    };
    if self.screen.rows == 0 || self.screen.cols == 0 {
      self.reset_selection();
      return;
    }

    self.selection_anchor = Some(clamp_viewport_point(
      preserved.anchor,
      self.screen.rows,
      self.screen.cols,
    ));
    self.selection_head = Some(clamp_viewport_point(
      preserved.head,
      self.screen.rows,
      self.screen.cols,
    ));
    self.selection_mode = preserved.mode;
    self.selection_dragging = preserved.dragging;
    self.recompute_selection_range();

    let Some(range) = self.resolved_selection else {
      self.reset_selection();
      return;
    };

    if !selection_matches_screen_text(&self.screen, range, &preserved.text) {
      self.reset_selection();
    }
  }

  fn refresh_snapshot_preserving_selection(&mut self) {
    let preserved = self.preserve_selection_before_refresh();
    self.refresh_snapshot();
    self.restore_selection_after_refresh(preserved);
  }

  fn poll_session(&mut self, cx: &mut Context<Self>) {
    let Some(session) = self.session.as_mut() else {
      return;
    };

    let result = session.poll();
    if result.clipboard_store.is_empty()
      && result.clipboard_load_requests.is_empty()
      && !result.changed
    {
      return;
    }

    for text in result.clipboard_store {
      cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    for formatter in result.clipboard_load_requests {
      if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
        session.paste(&formatter(&text));
      }
    }

    if result.changed {
      self.hovered_hyperlink = None;
      self.pending_link_activation = None;
      self.last_reported_mouse_state = None;
      self.refresh_snapshot_preserving_selection();
      cx.notify();
    }
  }

  pub(crate) fn sync_bounds(&mut self, bounds: TerminalBounds, cx: &mut Context<Self>) {
    if self.last_bounds == bounds {
      return;
    }

    self.last_bounds = bounds;
    self.hovered_hyperlink = None;
    self.pending_link_activation = None;
    self.scroll_remainder = px(0.0);
    if let Some(session) = self.session.as_mut() {
      session.resize(bounds);
      self.reset_selection();
      self.last_reported_mouse_state = None;
      self.refresh_snapshot();
      cx.notify();
    }
  }

  pub(crate) fn focus_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.focus_handle.focus(window, cx);
  }

  fn send_tab(&mut self, _: &SendTab, window: &mut Window, cx: &mut Context<Self>) {
    self.send_keystroke("tab", window, cx);
  }

  fn send_back_tab(&mut self, _: &SendBackTab, window: &mut Window, cx: &mut Context<Self>) {
    self.send_keystroke("shift-tab", window, cx);
  }

  /// Replays the key through the ordinary path, so the shell gets whatever the
  /// encoder says it should, back-tab included.
  fn send_keystroke(&mut self, keystroke: &str, window: &mut Window, cx: &mut Context<Self>) {
    let Ok(keystroke) = Keystroke::parse(keystroke) else {
      return;
    };
    self.on_key_down(
      &KeyDownEvent {
        keystroke,
        is_held: false,
        prefer_character_input: false,
      },
      window,
      cx,
    );
  }

  fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
    self.focus_terminal(window, cx);

    if self.matches_copy_shortcut(event) && self.copy_selection_to_clipboard(cx) {
      cx.stop_propagation();
      return;
    }

    if self.matches_paste_shortcut(event) {
      if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
        self.reset_selection();
        if let Some(session) = self.session.as_mut() {
          session.paste(&text);
          self.refresh_snapshot();
          cx.notify();
        }
      }
      cx.stop_propagation();
      return;
    }

    let Some(session) = self.session.as_mut() else {
      return;
    };

    if session.send_key_down(event) {
      self.reset_selection();
      self.refresh_snapshot();
      cx.notify();
      cx.stop_propagation();
    }
  }

  fn copy_selection_to_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
    let Some(text) = self.selection_text_for_copy() else {
      return false;
    };

    cx.write_to_clipboard(ClipboardItem::new_string(text));
    true
  }

  fn selection_text_for_copy(&self) -> Option<String> {
    let selection = self.selection_range()?;
    self
      .session
      .as_ref()
      .and_then(|session| session.selection_text(selection))
      .filter(|text| !text.is_empty())
      .or_else(|| {
        selection_text_from_screen(&self.screen, selection).filter(|text| !text.is_empty())
      })
  }

  fn matches_copy_shortcut(&self, event: &KeyDownEvent) -> bool {
    let modifiers = event.keystroke.modifiers;
    event.keystroke.key == "c"
      && ((cfg!(target_os = "macos") && modifiers.platform)
        || (!cfg!(target_os = "macos") && modifiers.control && modifiers.shift))
  }

  fn matches_paste_shortcut(&self, event: &KeyDownEvent) -> bool {
    let modifiers = event.keystroke.modifiers;
    event.keystroke.key == "v"
      && ((cfg!(target_os = "macos") && modifiers.platform)
        || (!cfg!(target_os = "macos") && modifiers.control && modifiers.shift))
  }

  fn hyperlink_activation_enabled(&self, modifiers: Modifiers) -> bool {
    modifiers.secondary() && !modifiers.shift
  }

  fn hyperlink_at(&self, point: ViewportPoint) -> Option<Arc<str>> {
    if point.row >= self.screen.rows || point.col >= self.screen.cols {
      return None;
    }

    self
      .screen
      .cells
      .iter()
      .find(|cell| cell.row == point.row && cell.col == point.col)
      .and_then(|cell| cell.hyperlink_uri.clone())
  }

  fn scroll_lines_for_event(&mut self, event: &ScrollWheelEvent) -> Option<i32> {
    let line_height = px(f32::from(self.last_bounds.cell_height.max(1)));

    match event.touch_phase {
      TouchPhase::Started => {
        self.scroll_remainder = px(0.0);
        None
      }
      TouchPhase::Moved => {
        self.scroll_remainder += event.delta.pixel_delta(line_height).y;
        let delta_lines = (self.scroll_remainder / line_height) as i32;
        self.scroll_remainder %= line_height;
        (delta_lines != 0).then_some(delta_lines)
      }
      TouchPhase::Ended | TouchPhase::Cancelled => None,
    }
  }
}

impl Focusable for TerminalView {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for TerminalView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let is_dark = theme.is_dark();
    let (cursor_color, selection_color) = if is_dark {
      (rgb(0x7aa2f7).into(), rgb(0x33467a).into())
    } else {
      (rgb(0x2563eb).into(), rgb(0x60a5fa).into())
    };
    let terminal_palette = TerminalPalette::themed(
      is_dark,
      theme.sidebar,
      theme.foreground,
      cursor_color,
      selection_color,
    );
    let terminal_screen = div()
      .id("terminal-screen")
      .debug_selector(|| TERMINAL_SCREEN_DEBUG_SELECTOR.to_string())
      .size_full()
      .overflow_hidden()
      .bg(theme.sidebar)
      .child(
        div()
          .debug_selector(|| TERMINAL_SURFACE_DEBUG_SELECTOR.to_string())
          .size_full()
          .overflow_hidden()
          .font(terminal_font(theme.mono_font_family.clone()))
          .text_sm()
          .line_height(relative(1.2))
          .text_color(theme.foreground)
          .child(TerminalElement::new(
            cx.entity().clone(),
            terminal_palette,
            self.focus_handle.is_focused(window),
          )),
      );
    let terminal_screen = if let Some(url) = self.hovered_hyperlink.clone() {
      terminal_screen
        .tooltip(move |window, cx| Tooltip::new(url.as_ref().to_string()).build(window, cx))
    } else {
      terminal_screen
    };

    let banner_message = self
      .error
      .clone()
      .or_else(|| self.screen.exit_status.clone());

    div()
      .id("terminal-scaffold")
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.sidebar)
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, _, window, cx| {
          this.focus_terminal(window, cx);
        }),
      )
      .on_key_down(cx.listener(Self::on_key_down))
      .on_action(cx.listener(Self::send_tab))
      .on_action(cx.listener(Self::send_back_tab))
      .key_context(TERMINAL_CONTEXT)
      .track_focus(&self.focus_handle)
      .when_some(banner_message, |this, message| {
        this.child(
          div()
            .debug_selector(|| TERMINAL_BANNER_DEBUG_SELECTOR.to_string())
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_3()
            .py_2()
            .bg(theme.muted)
            .border_b_1()
            .border_color(theme.border)
            .text_sm()
            .text_color(theme.foreground)
            .child(div().flex_1().min_w_0().child(message))
            .child(
              Button::new("terminal-restart")
                .label("Restart")
                .with_variant(ButtonVariant::Secondary)
                .xsmall()
                .on_click(cx.listener(|this, _, _window, cx| {
                  this.restart_session();
                  cx.notify();
                })),
            ),
        )
      })
      .child(div().flex_1().min_h_0().child(terminal_screen))
  }
}

fn terminal_font(family: gpui::SharedString) -> Font {
  Font {
    family,
    features: FontFeatures::default(),
    weight: FontWeight::NORMAL,
    style: FontStyle::Normal,
    fallbacks: Some(FontFallbacks::from_fonts(vec![
      "Lilex".into(),
      "Symbols Nerd Font Mono".into(),
      "Symbols Nerd Font".into(),
      "Apple Color Emoji".into(),
    ])),
  }
}

fn clamp_viewport_point(point: ViewportPoint, rows: usize, cols: usize) -> ViewportPoint {
  ViewportPoint {
    row: point.row.min(rows.saturating_sub(1)),
    col: point.col.min(cols.saturating_sub(1)),
  }
}

fn clamp_selection_to_screen(
  range: ViewportSelectionRange,
  rows: usize,
  cols: usize,
) -> ViewportSelectionRange {
  ViewportSelectionRange {
    start: clamp_viewport_point(range.start, rows, cols),
    end: clamp_viewport_point(range.end, rows, cols),
  }
  .normalized()
}

fn selection_text_from_screen(
  screen: &ScreenSnapshot,
  range: ViewportSelectionRange,
) -> Option<String> {
  if screen.rows == 0 || screen.cols == 0 {
    return None;
  }

  let range = clamp_selection_to_screen(range, screen.rows, screen.cols);
  let mut cells = vec![' '; screen.rows * screen.cols];
  for cell in &screen.cells {
    if cell.row < screen.rows && cell.col < screen.cols {
      cells[cell.row * screen.cols + cell.col] = if cell.flags.contains(Flags::HIDDEN) {
        ' '
      } else {
        cell.c
      };
    }
  }

  let mut text = String::new();
  for row in range.start.row..=range.end.row {
    let start_col = if row == range.start.row {
      range.start.col
    } else {
      0
    };
    let end_col = if row == range.end.row {
      range.end.col
    } else {
      screen.cols.saturating_sub(1)
    };

    for col in start_col..=end_col {
      text.push(cells[row * screen.cols + col]);
    }

    if row < range.end.row {
      text.push('\n');
    }
  }

  Some(text)
}

fn selection_matches_screen_text(
  screen: &ScreenSnapshot,
  range: ViewportSelectionRange,
  expected: &str,
) -> bool {
  selection_text_from_screen(screen, range).is_some_and(|text| text == expected)
}

#[cfg(test)]
mod tests {
  use super::{
    TERMINAL_BANNER_DEBUG_SELECTOR, TERMINAL_SURFACE_DEBUG_SELECTOR, TerminalView,
    selection_matches_screen_text, selection_mode_for_click_count, selection_text_from_screen,
  };
  use crate::{
    ScreenSnapshot, TerminalBounds, TerminalCellSnapshot, TerminalSelectionMode, TerminalSession,
    ViewportPoint, ViewportSelectionRange,
  };
  use alacritty_terminal::term::cell::Flags;
  use alacritty_terminal::vte::ansi::{Color, NamedColor};
  use gpui::{
    AppContext, ClipboardItem, Context, InteractiveElement, KeyDownEvent, Keystroke, Modifiers,
    MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, Render, ScrollDelta,
    ScrollWheelEvent, Styled, TestAppContext, TouchPhase, VisualTestContext, Window, div, point,
    px,
  };
  use std::sync::Arc;

  fn init_gpui_test(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
  }

  fn screen_from_lines(lines: &[&str]) -> ScreenSnapshot {
    let rows = lines.len();
    let cols = lines
      .iter()
      .map(|line| line.chars().count())
      .max()
      .unwrap_or(0);
    let mut cells = Vec::new();

    for (row, line) in lines.iter().enumerate() {
      for (col, ch) in line.chars().enumerate() {
        cells.push(TerminalCellSnapshot {
          row,
          col,
          c: ch,
          fg: Color::Named(NamedColor::Foreground),
          bg: Color::Named(NamedColor::Background),
          flags: Flags::empty(),
          underline_color: None,
          hyperlink_uri: None,
        });
      }
    }

    ScreenSnapshot {
      rows,
      cols,
      cells,
      ..ScreenSnapshot::default()
    }
  }

  fn test_session() -> TerminalSession {
    TerminalSession::spawn(std::env::temp_dir(), TerminalBounds::default())
      .expect("test terminal session should spawn")
  }

  fn screen_with_hyperlink(line: &str, hyperlink_range: std::ops::Range<usize>) -> ScreenSnapshot {
    let mut screen = screen_from_lines(&[line]);
    for col in hyperlink_range {
      let cell = screen
        .cells
        .iter_mut()
        .find(|cell| cell.row == 0 && cell.col == col)
        .expect("hyperlink cell should exist");
      cell.hyperlink_uri = Some(Arc::<str>::from("https://example.com"));
    }
    screen
  }

  fn key_event(key: &str, modifiers: Modifiers) -> KeyDownEvent {
    KeyDownEvent {
      keystroke: Keystroke {
        modifiers,
        key: key.to_string(),
        key_char: Some(key.to_string()),
      },
      is_held: false,
      prefer_character_input: false,
    }
  }

  fn scroll_event(delta_y: f32, touch_phase: TouchPhase) -> ScrollWheelEvent {
    ScrollWheelEvent {
      delta: ScrollDelta::Pixels(point(px(0.0), px(delta_y))),
      touch_phase,
      ..Default::default()
    }
  }

  fn copy_shortcut_modifiers() -> Modifiers {
    let mut modifiers = Modifiers::default();
    if cfg!(target_os = "macos") {
      modifiers.platform = true;
    } else {
      modifiers.control = true;
      modifiers.shift = true;
    }
    modifiers
  }

  fn paste_shortcut_modifiers() -> Modifiers {
    copy_shortcut_modifiers()
  }

  fn secondary_click_modifiers() -> Modifiers {
    let mut modifiers = Modifiers::default();
    if cfg!(target_os = "macos") {
      modifiers.platform = true;
    } else {
      modifiers.control = true;
    }
    modifiers
  }

  struct TerminalProbeHarness {
    terminal: gpui::Entity<TerminalView>,
    probe_mouse_moves: usize,
    probe_mouse_ups: usize,
  }

  struct NarrowTerminalHarness {
    terminal: gpui::Entity<TerminalView>,
  }

  impl TerminalProbeHarness {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
      Self {
        terminal: cx.new(|cx| TerminalView::new(None, cx)),
        probe_mouse_moves: 0,
        probe_mouse_ups: 0,
      }
    }

    fn handle_probe_move(
      &mut self,
      _event: &MouseMoveEvent,
      _window: &mut Window,
      cx: &mut Context<Self>,
    ) {
      self.probe_mouse_moves += 1;
      cx.notify();
    }

    fn handle_probe_up(
      &mut self,
      _event: &MouseUpEvent,
      _window: &mut Window,
      cx: &mut Context<Self>,
    ) {
      self.probe_mouse_ups += 1;
      cx.notify();
    }
  }

  impl NarrowTerminalHarness {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
      Self {
        terminal: cx.new(|cx| TerminalView::new(None, cx)),
      }
    }
  }

  impl Render for TerminalProbeHarness {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
      div()
        .id("terminal-probe-harness")
        .size_full()
        .flex()
        .flex_col()
        .child(div().h(px(240.)).child(self.terminal.clone()))
        .child(
          div()
            .id("terminal-test-probe")
            .h(px(120.))
            .w_full()
            .on_mouse_move(cx.listener(Self::handle_probe_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_probe_up))
            .child("probe"),
        )
    }
  }

  impl Render for NarrowTerminalHarness {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
      div()
        .id("narrow-terminal-harness")
        .size_full()
        .child(div().w(px(240.)).h(px(180.)).child(self.terminal.clone()))
    }
  }

  #[test]
  fn selection_mode_for_click_count_uses_word_and_line_modes() {
    assert_eq!(
      selection_mode_for_click_count(1),
      TerminalSelectionMode::Simple
    );
    assert_eq!(
      selection_mode_for_click_count(2),
      TerminalSelectionMode::Semantic
    );
    assert_eq!(
      selection_mode_for_click_count(3),
      TerminalSelectionMode::Lines
    );
    assert_eq!(
      selection_mode_for_click_count(8),
      TerminalSelectionMode::Lines
    );
  }

  #[test]
  fn selection_text_from_screen_collects_visible_cells() {
    let screen = ScreenSnapshot {
      rows: 2,
      cols: 4,
      cells: vec![
        TerminalCellSnapshot {
          row: 0,
          col: 0,
          c: 't',
          fg: Color::Named(NamedColor::Foreground),
          bg: Color::Named(NamedColor::Background),
          flags: Flags::empty(),
          underline_color: None,
          hyperlink_uri: None,
        },
        TerminalCellSnapshot {
          row: 0,
          col: 1,
          c: 'e',
          fg: Color::Named(NamedColor::Foreground),
          bg: Color::Named(NamedColor::Background),
          flags: Flags::empty(),
          underline_color: None,
          hyperlink_uri: None,
        },
        TerminalCellSnapshot {
          row: 0,
          col: 2,
          c: 's',
          fg: Color::Named(NamedColor::Foreground),
          bg: Color::Named(NamedColor::Background),
          flags: Flags::empty(),
          underline_color: None,
          hyperlink_uri: None,
        },
        TerminalCellSnapshot {
          row: 0,
          col: 3,
          c: 't',
          fg: Color::Named(NamedColor::Foreground),
          bg: Color::Named(NamedColor::Background),
          flags: Flags::empty(),
          underline_color: None,
          hyperlink_uri: None,
        },
        TerminalCellSnapshot {
          row: 1,
          col: 0,
          c: 'o',
          fg: Color::Named(NamedColor::Foreground),
          bg: Color::Named(NamedColor::Background),
          flags: Flags::empty(),
          underline_color: None,
          hyperlink_uri: None,
        },
        TerminalCellSnapshot {
          row: 1,
          col: 1,
          c: 'k',
          fg: Color::Named(NamedColor::Foreground),
          bg: Color::Named(NamedColor::Background),
          flags: Flags::empty(),
          underline_color: None,
          hyperlink_uri: None,
        },
      ],
      ..ScreenSnapshot::default()
    };

    let selection = ViewportSelectionRange {
      start: ViewportPoint { row: 0, col: 1 },
      end: ViewportPoint { row: 1, col: 1 },
    };

    assert_eq!(
      selection_text_from_screen(&screen, selection),
      Some("est\nok".to_string())
    );
  }

  #[test]
  fn selection_matches_screen_text_detects_screen_changes() {
    let mut screen = ScreenSnapshot {
      rows: 1,
      cols: 3,
      cells: vec![
        TerminalCellSnapshot {
          row: 0,
          col: 0,
          c: 'c',
          fg: Color::Named(NamedColor::Foreground),
          bg: Color::Named(NamedColor::Background),
          flags: Flags::empty(),
          underline_color: None,
          hyperlink_uri: None,
        },
        TerminalCellSnapshot {
          row: 0,
          col: 1,
          c: 'a',
          fg: Color::Named(NamedColor::Foreground),
          bg: Color::Named(NamedColor::Background),
          flags: Flags::empty(),
          underline_color: None,
          hyperlink_uri: None,
        },
        TerminalCellSnapshot {
          row: 0,
          col: 2,
          c: 't',
          fg: Color::Named(NamedColor::Foreground),
          bg: Color::Named(NamedColor::Background),
          flags: Flags::empty(),
          underline_color: None,
          hyperlink_uri: None,
        },
      ],
      ..ScreenSnapshot::default()
    };
    let selection = ViewportSelectionRange {
      start: ViewportPoint { row: 0, col: 0 },
      end: ViewportPoint { row: 0, col: 2 },
    };

    assert!(selection_matches_screen_text(&screen, selection, "cat"));

    screen.cells[2].c = 'r';

    assert!(!selection_matches_screen_text(&screen, selection, "cat"));
  }

  #[gpui::test]
  fn scroll_lines_for_event_accumulates_pixel_deltas_until_a_full_line(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let view = cx.new(|cx| TerminalView::new(None, cx));
    view.update(cx, |view, _| {
      view.last_bounds = TerminalBounds {
        cell_height: 10,
        ..TerminalBounds::default()
      };

      assert_eq!(
        view.scroll_lines_for_event(&scroll_event(0.0, TouchPhase::Started)),
        None
      );
      assert_eq!(
        view.scroll_lines_for_event(&scroll_event(4.0, TouchPhase::Moved)),
        None
      );
      assert_eq!(
        view.scroll_lines_for_event(&scroll_event(4.0, TouchPhase::Moved)),
        None
      );
      assert_eq!(
        view.scroll_lines_for_event(&scroll_event(4.0, TouchPhase::Moved)),
        Some(1)
      );
      assert_eq!(view.scroll_remainder, px(2.0));
    });
  }

  #[gpui::test]
  fn scroll_lines_for_event_uses_positive_delta_for_scroll_up(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let view = cx.new(|cx| TerminalView::new(None, cx));
    view.update(cx, |view, _| {
      view.last_bounds = TerminalBounds {
        cell_height: 10,
        ..TerminalBounds::default()
      };

      assert_eq!(
        view.scroll_lines_for_event(&scroll_event(0.0, TouchPhase::Started)),
        None
      );
      assert_eq!(
        view.scroll_lines_for_event(&scroll_event(12.0, TouchPhase::Moved)),
        Some(1)
      );
      assert_eq!(
        view.scroll_lines_for_event(&scroll_event(0.0, TouchPhase::Started)),
        None
      );
      assert_eq!(
        view.scroll_lines_for_event(&scroll_event(-12.0, TouchPhase::Moved)),
        Some(-1)
      );
    });
  }

  #[gpui::test]
  fn selection_refresh_preserves_matching_simple_selection(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let view = cx.new(|cx| TerminalView::new(None, cx));
    let selection = ViewportSelectionRange {
      start: ViewportPoint { row: 0, col: 1 },
      end: ViewportPoint { row: 0, col: 3 },
    };

    view.update(cx, |view, _| {
      view.screen = screen_from_lines(&["hello"]);
      view.selection_anchor = Some(selection.start);
      view.selection_head = Some(selection.end);
      view.selection_mode = TerminalSelectionMode::Simple;
      view.resolved_selection = Some(selection);

      let preserved = view.preserve_selection_before_refresh();
      view.screen = screen_from_lines(&["hello"]);
      view.restore_selection_after_refresh(preserved);

      assert_eq!(view.selection_range(), Some(selection));
    });
  }

  #[gpui::test]
  fn selection_refresh_clears_when_visible_text_changes(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let view = cx.new(|cx| TerminalView::new(None, cx));
    let selection = ViewportSelectionRange {
      start: ViewportPoint { row: 0, col: 1 },
      end: ViewportPoint { row: 0, col: 3 },
    };

    view.update(cx, |view, _| {
      view.screen = screen_from_lines(&["hello"]);
      view.selection_anchor = Some(selection.start);
      view.selection_head = Some(selection.end);
      view.selection_mode = TerminalSelectionMode::Simple;
      view.resolved_selection = Some(selection);

      let preserved = view.preserve_selection_before_refresh();
      view.screen = screen_from_lines(&["hullo"]);
      view.restore_selection_after_refresh(preserved);

      assert_eq!(view.selection_range(), None);
    });
  }

  #[gpui::test]
  fn clicking_terminal_screen_focuses_terminal_view(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(None, cx));
    let cx: &mut VisualTestContext = cx;
    view.update(cx, |view, _| {
      view.screen = screen_from_lines(&["hello"]);
    });

    let screen_bounds = cx
      .debug_bounds(TERMINAL_SURFACE_DEBUG_SELECTOR)
      .expect("terminal surface bounds");
    cx.simulate_click(
      point(screen_bounds.left() + px(8.), screen_bounds.top() + px(8.)),
      Modifiers::default(),
    );

    let focused =
      cx.update(|window, app| view.read_with(app, |view, _| view.focus_handle.is_focused(window)));
    assert!(
      focused,
      "terminal should receive focus after clicking its screen"
    );
  }

  #[gpui::test]
  fn clicking_terminal_screen_refocuses_terminal_view_after_focus_leaves(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(None, cx));
    let cx: &mut VisualTestContext = cx;
    view.update(cx, |view, _| {
      view.screen = screen_from_lines(&["hello"]);
    });

    let screen_bounds = cx
      .debug_bounds(TERMINAL_SURFACE_DEBUG_SELECTOR)
      .expect("terminal surface bounds");
    let click_point = point(screen_bounds.left() + px(8.), screen_bounds.top() + px(8.));

    cx.simulate_click(click_point, Modifiers::default());
    let initially_focused =
      cx.update(|window, app| view.read_with(app, |view, _| view.focus_handle.is_focused(window)));
    assert!(
      initially_focused,
      "terminal should receive focus on the initial click"
    );

    view.update_in(cx, |view, window, cx| {
      let external_focus = cx.focus_handle();
      window.focus(&external_focus, cx);
      assert!(
        !view.focus_handle.is_focused(window),
        "terminal focus should leave after focusing another handle"
      );
    });

    cx.simulate_click(click_point, Modifiers::default());
    let refocused =
      cx.update(|window, app| view.read_with(app, |view, _| view.focus_handle.is_focused(window)));
    assert!(
      refocused,
      "terminal should regain focus after clicking it again"
    );
  }

  #[gpui::test]
  fn terminal_bounds_follow_narrow_surface_size(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let (harness, cx) = cx.add_window_view(NarrowTerminalHarness::new);
    let cx: &mut VisualTestContext = cx;

    let surface_bounds = cx
      .debug_bounds(TERMINAL_SURFACE_DEBUG_SELECTOR)
      .expect("terminal surface bounds");
    let bounds = harness.read_with(cx, |harness, app| {
      let terminal = harness.terminal.read(app);
      terminal.last_bounds
    });
    let expected = TerminalBounds::from_size(
      f32::from(surface_bounds.size.width),
      f32::from(surface_bounds.size.height),
      bounds.cell_width,
      bounds.cell_height,
    );

    assert!(f32::from(surface_bounds.size.width) <= 240.0);
    assert_eq!(bounds, expected);
    assert!(
      bounds.columns < 48,
      "expected sidebar width to yield fewer columns than the old full-window sizing"
    );
  }

  #[gpui::test]
  fn hovering_terminal_hyperlink_updates_hover_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let view = cx.new(|cx| TerminalView::new(None, cx));
    view.update(cx, |view, cx| {
      view.screen = screen_with_hyperlink("link", 0..4);
      view.update_hovered_hyperlink(Some(ViewportPoint { row: 0, col: 1 }), cx);
    });

    let hovered = view.read_with(cx, |view, _| view.hovered_hyperlink.clone());
    assert_eq!(hovered.as_deref(), Some("https://example.com"));
  }

  #[gpui::test]
  fn terminal_does_not_capture_mouse_events_outside_its_screen(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let (harness, cx) = cx.add_window_view(TerminalProbeHarness::new);
    let cx: &mut VisualTestContext = cx;
    let probe_point = point(px(20.), px(280.));

    cx.simulate_mouse_move(probe_point, None, Modifiers::default());
    cx.simulate_mouse_up(probe_point, MouseButton::Left, Modifiers::default());

    let (mouse_moves, mouse_ups) = harness.read_with(cx, |harness, _| {
      (harness.probe_mouse_moves, harness.probe_mouse_ups)
    });
    assert_eq!(
      mouse_moves, 1,
      "probe should receive mouse move outside terminal"
    );
    assert_eq!(
      mouse_ups, 1,
      "probe should receive mouse up outside terminal"
    );
  }

  #[gpui::test]
  fn dragging_inside_terminal_creates_local_selection(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let screen = screen_from_lines(&["hello"]);
    let view = cx.new(|cx| TerminalView::new(None, cx));
    view.update(cx, |view, cx| {
      view.session = Some(test_session());
      view.screen = screen.clone();
      view.handle_mouse_down(
        MouseButton::Left,
        ViewportPoint { row: 0, col: 0 },
        1,
        Modifiers::default(),
        cx,
      );
      view.handle_mouse_move(
        ViewportPoint { row: 0, col: 3 },
        Some(MouseButton::Left),
        Modifiers::default(),
        cx,
      );
      view.handle_mouse_up(
        MouseButton::Left,
        ViewportPoint { row: 0, col: 3 },
        Modifiers::default(),
        cx,
      );

      assert_eq!(
        view.selection_range(),
        Some(ViewportSelectionRange {
          start: ViewportPoint { row: 0, col: 0 },
          end: ViewportPoint { row: 0, col: 3 },
        })
      );
    });
  }

  #[gpui::test]
  fn shift_click_extends_existing_local_selection(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let screen = screen_from_lines(&["hello"]);
    let view = cx.new(|cx| TerminalView::new(None, cx));
    view.update(cx, |view, cx| {
      view.session = Some(test_session());
      view.screen = screen.clone();

      view.handle_mouse_down(
        MouseButton::Left,
        ViewportPoint { row: 0, col: 0 },
        1,
        Modifiers::default(),
        cx,
      );
      view.handle_mouse_move(
        ViewportPoint { row: 0, col: 1 },
        Some(MouseButton::Left),
        Modifiers::default(),
        cx,
      );
      view.handle_mouse_up(
        MouseButton::Left,
        ViewportPoint { row: 0, col: 1 },
        Modifiers::default(),
        cx,
      );

      let mut shift = Modifiers::default();
      shift.shift = true;
      view.handle_mouse_down(
        MouseButton::Left,
        ViewportPoint { row: 0, col: 4 },
        1,
        shift,
        cx,
      );
      view.handle_mouse_up(
        MouseButton::Left,
        ViewportPoint { row: 0, col: 4 },
        shift,
        cx,
      );

      assert_eq!(
        view.selection_range(),
        Some(ViewportSelectionRange {
          start: ViewportPoint { row: 0, col: 0 },
          end: ViewportPoint { row: 0, col: 4 },
        })
      );
    });
  }

  #[gpui::test]
  fn copy_shortcut_copies_selected_text_from_screen_snapshot(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(None, cx));
    let cx: &mut VisualTestContext = cx;

    view.update_in(cx, |view, window, cx| {
      view.screen = screen_from_lines(&["hello"]);
      view.selection_anchor = Some(ViewportPoint { row: 0, col: 1 });
      view.selection_head = Some(ViewportPoint { row: 0, col: 3 });
      view.selection_mode = TerminalSelectionMode::Simple;
      view.resolved_selection = Some(ViewportSelectionRange {
        start: ViewportPoint { row: 0, col: 1 },
        end: ViewportPoint { row: 0, col: 3 },
      });

      view.on_key_down(&key_event("c", copy_shortcut_modifiers()), window, cx);
    });

    let clipboard = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(clipboard.as_deref(), Some("ell"));
  }

  #[gpui::test]
  fn paste_shortcut_clears_selection_when_clipboard_has_text(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(None, cx));
    let cx: &mut VisualTestContext = cx;
    cx.write_to_clipboard(ClipboardItem::new_string("echo test".to_string()));

    view.update_in(cx, |view, window, cx| {
      view.session = Some(test_session());
      view.screen = screen_from_lines(&["hello"]);
      view.selection_anchor = Some(ViewportPoint { row: 0, col: 0 });
      view.selection_head = Some(ViewportPoint { row: 0, col: 4 });
      view.selection_mode = TerminalSelectionMode::Simple;
      view.resolved_selection = Some(ViewportSelectionRange {
        start: ViewportPoint { row: 0, col: 0 },
        end: ViewportPoint { row: 0, col: 4 },
      });

      view.on_key_down(&key_event("v", paste_shortcut_modifiers()), window, cx);

      assert_eq!(view.selection_range(), None);
    });
  }

  #[gpui::test]
  fn secondary_click_on_hyperlink_opens_url(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let view = cx.new(|cx| TerminalView::new(None, cx));
    view.update(cx, |view, cx| {
      view.session = Some(test_session());
      view.screen = screen_with_hyperlink("link", 0..4);

      let modifiers = secondary_click_modifiers();
      view.handle_mouse_down(
        MouseButton::Left,
        ViewportPoint { row: 0, col: 1 },
        1,
        modifiers,
        cx,
      );
      view.handle_mouse_up(
        MouseButton::Left,
        ViewportPoint { row: 0, col: 1 },
        modifiers,
        cx,
      );
    });

    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com"));
    let selection = view.read_with(cx, |view, _| view.selection_range());
    assert_eq!(selection, None);
  }

  #[gpui::test]
  fn dragging_away_cancels_pending_hyperlink_activation(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let view = cx.new(|cx| TerminalView::new(None, cx));
    view.update(cx, |view, cx| {
      view.session = Some(test_session());
      view.screen = screen_with_hyperlink("link ok", 0..4);

      let modifiers = secondary_click_modifiers();
      view.handle_mouse_down(
        MouseButton::Left,
        ViewportPoint { row: 0, col: 1 },
        1,
        modifiers,
        cx,
      );
      view.handle_mouse_move(
        ViewportPoint { row: 0, col: 5 },
        Some(MouseButton::Left),
        modifiers,
        cx,
      );
      view.handle_mouse_up(
        MouseButton::Left,
        ViewportPoint { row: 0, col: 5 },
        modifiers,
        cx,
      );

      assert!(view.pending_link_activation.is_none());
      assert_eq!(view.selection_range(), None);
    });

    assert_eq!(cx.opened_url(), None);
  }

  #[gpui::test]
  fn banner_renders_when_session_spawn_fails(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let missing = std::env::temp_dir().join("reviu-terminal-missing-cwd-banner");
    let _ = std::fs::remove_dir_all(&missing);

    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(Some(missing.clone()), cx));
    let cx: &mut VisualTestContext = cx;

    let banner_bounds = cx.debug_bounds(TERMINAL_BANNER_DEBUG_SELECTOR);
    assert!(
      banner_bounds.is_some(),
      "banner should render when session spawn fails"
    );

    let (has_error, has_session) =
      view.read_with(cx, |view, _| (view.error.is_some(), view.session.is_some()));
    assert!(has_error, "spawn failure should populate error");
    assert!(
      !has_session,
      "no session should be active after spawn failure"
    );
  }

  #[gpui::test]
  fn restart_session_clears_error_and_respawns(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let view = cx.new(|cx| TerminalView::new(Some(std::env::temp_dir()), cx));

    view.update(cx, |view, _| {
      assert!(view.session.is_some(), "initial spawn should succeed");
      view.error = Some("forced error".to_string());
      view.restart_session();

      assert!(view.error.is_none(), "restart should clear error");
      assert!(view.session.is_some(), "restart should respawn session");
    });
  }
}
