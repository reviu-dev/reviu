use alacritty_terminal::{event::Event as TerminalEvent, term::cell::Flags};
use gpui::{
  Action, App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, Font,
  FontFallbacks, FontFeatures, FontStyle, FontWeight, InteractiveElement, IntoElement,
  KeyDownEvent, Keystroke, Modifiers, MouseButton, ParentElement, Pixels, Render, ScrollWheelEvent,
  Styled, Subscription, Task, TouchPhase, Window, div, prelude::*, px, relative, rgb,
};
use gpui_component::ActiveTheme as _;
use gpui_component::Disableable as _;
use gpui_component::IconName;
use gpui_component::Sizable as _;
use gpui_component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::{Scrollbar, ScrollbarMode};
use gpui_component::tooltip::Tooltip;
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc, time::Duration};

use crate::{
  ScreenSnapshot, TerminalBounds, TerminalSelectionMode, TerminalSession, ViewportPoint,
  ViewportSelectionRange,
  colors::TerminalPalette,
  links::{TerminalLink, TerminalLinkTarget, link_at},
  session::TerminalSearchMatch,
  terminal_element::TerminalElement,
  terminal_scrollbar::TerminalScrollHandle,
};

const MAX_COALESCED_SESSION_EVENTS: usize = 100;
const TEST_SESSION_POLL_INTERVAL: Duration = Duration::from_millis(16);
const TERMINAL_SCREEN_DEBUG_SELECTOR: &str = "terminal-screen-bounds";
const TERMINAL_SURFACE_DEBUG_SELECTOR: &str = "terminal-surface-bounds";
const TERMINAL_BANNER_DEBUG_SELECTOR: &str = "terminal-banner";
const TERMINAL_SEARCH_DEBUG_SELECTOR: &str = "terminal-search";
const TERMINAL_SCROLLBAR_DEBUG_SELECTOR: &str = "terminal-scrollbar";
const TERMINAL_JUMP_TO_BOTTOM_DEBUG_SELECTOR: &str = "terminal-jump-to-bottom";

fn collect_pending_session_events(
  first_event: TerminalEvent,
  receiver: &async_channel::Receiver<TerminalEvent>,
) -> Vec<TerminalEvent> {
  let mut wakeup_collected = matches!(first_event, TerminalEvent::Wakeup);
  let mut events = vec![first_event];

  for _ in 1..MAX_COALESCED_SESSION_EVENTS {
    let Ok(event) = receiver.try_recv() else {
      break;
    };
    if matches!(event, TerminalEvent::Wakeup) {
      if !wakeup_collected {
        events.push(event);
        wakeup_collected = true;
      }
    } else {
      events.push(event);
    }
  }

  events
}

fn should_defer_to_ime(event: &KeyDownEvent) -> bool {
  event.prefer_character_input
    && !event.keystroke.modifiers.control
    && !event.keystroke.modifiers.platform
    && !event.keystroke.modifiers.function
    && event
      .keystroke
      .key_char
      .as_deref()
      .is_some_and(|text| text.chars().any(|character| !character.is_control()))
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct HoveredTerminalLink {
  point: ViewportPoint,
  link: TerminalLink,
}

#[derive(Clone)]
struct PendingLinkActivation {
  target: TerminalLinkTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalViewEvent {
  OpenFile {
    path: PathBuf,
    line: Option<u32>,
    column: Option<u32>,
  },
  WorkingDirectoryChanged {
    path: PathBuf,
  },
}

gpui::actions!(
  terminal,
  [
    OpenSearch,
    CloseSearch,
    SearchNext,
    SearchPrevious,
    ScrollLineUp,
    ScrollLineDown,
    ScrollPageUp,
    ScrollPageDown,
    ScrollToTop,
    ScrollToBottom,
  ]
);

#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = terminal, no_json)]
pub struct SendKeystroke(pub String);

pub const TERMINAL_CONTEXT: &str = "Terminal";
pub const TERMINAL_SEARCH_CONTEXT: &str = "TerminalSearch";

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
  hovered_hyperlink: Option<HoveredTerminalLink>,
  pending_link_activation: Option<PendingLinkActivation>,
  last_reported_mouse_state: Option<(ViewportPoint, Option<MouseButton>)>,
  scroll_remainder: Pixels,
  scroll_handle: TerminalScrollHandle,
  marked_text: Option<String>,
  search_open: bool,
  search_input: Option<Entity<InputState>>,
  search_input_subscription: Option<Subscription>,
  search_query: String,
  search_matches: Vec<TerminalSearchMatch>,
  search_active_match: Option<usize>,
  visible_search_matches: Vec<ViewportSelectionRange>,
  visible_active_search_match: Option<ViewportSelectionRange>,
  search_generation: u64,
  _event_task: Task<()>,
  _working_directory_task: Task<()>,
  _search_task: Task<()>,
}

impl TerminalView {
  pub fn new(working_directory: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
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
      scroll_handle: TerminalScrollHandle::new(),
      marked_text: None,
      search_open: false,
      search_input: None,
      search_input_subscription: None,
      search_query: String::new(),
      search_matches: Vec::new(),
      search_active_match: None,
      visible_search_matches: Vec::new(),
      visible_active_search_match: None,
      search_generation: 0,
      _event_task: Task::ready(()),
      _working_directory_task: Task::ready(()),
      _search_task: Task::ready(()),
    };
    view.set_working_directory(working_directory, cx);
    view
  }

  pub fn working_directory(&self) -> Option<&std::path::Path> {
    self.working_directory.as_deref()
  }

  #[doc(hidden)]
  pub fn visible_text_for_driver(&self) -> String {
    if self.screen.rows == 0 || self.screen.cols == 0 {
      return String::new();
    }
    selection_text_from_screen(
      &self.screen,
      ViewportSelectionRange {
        start: ViewportPoint { row: 0, col: 0 },
        end: ViewportPoint {
          row: self.screen.rows - 1,
          col: self.screen.cols - 1,
        },
      },
    )
    .unwrap_or_default()
    .lines()
    .map(str::trim_end)
    .collect::<Vec<_>>()
    .join("\n")
  }

  #[doc(hidden)]
  pub fn scrollback_state_for_driver(&self) -> (usize, usize) {
    (self.screen.display_offset, self.screen.total_lines)
  }

  #[doc(hidden)]
  pub fn search_state_for_driver(&self) -> (bool, Option<usize>, usize) {
    (
      self.search_open,
      self.search_active_match.map(|index| index + 1),
      self.search_matches.len(),
    )
  }

  #[doc(hidden)]
  pub fn title_for_driver(&self) -> Option<&str> {
    self.screen.title.as_deref()
  }

  #[doc(hidden)]
  pub fn first_visible_file_link_for_driver(&self) -> Option<TerminalViewEvent> {
    for row in 0..self.screen.rows {
      for col in 0..self.screen.cols {
        let Some(link) = self.hyperlink_at(ViewportPoint { row, col }) else {
          continue;
        };
        if let TerminalLinkTarget::Path { path, line, column } = link.target {
          return Some(TerminalViewEvent::OpenFile { path, line, column });
        }
      }
    }
    None
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
    self.restart_session(cx);
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
    self.hyperlink_activation_enabled(modifiers)
      && self
        .hovered_hyperlink
        .as_ref()
        .is_some_and(|hovered| hovered.point == point)
  }

  pub(crate) fn update_hovered_hyperlink(
    &mut self,
    point: Option<ViewportPoint>,
    cx: &mut Context<Self>,
  ) {
    let next = point.and_then(|point| {
      self
        .hyperlink_at(point)
        .map(|link| HoveredTerminalLink { point, link })
    });
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
      && let Some(link) = self.hyperlink_at(point)
    {
      self.last_reported_mouse_state = None;
      self.pending_link_activation = Some(PendingLinkActivation {
        target: link.target,
      });
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
          .is_some_and(|link| link.target == pending.target);
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
          .is_some_and(|link| link.target == pending.target)
      {
        match pending.target {
          TerminalLinkTarget::Url(url) => cx.open_url(url.as_ref()),
          TerminalLinkTarget::Path { path, line, column } => {
            cx.emit(TerminalViewEvent::OpenFile { path, line, column });
          }
        }
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
    self.finish_scrollback_change(cx);
  }

  fn scroll_scrollback_by(&mut self, delta_lines: i32, cx: &mut Context<Self>) {
    let Some(session) = self.session.as_mut() else {
      return;
    };
    session.scroll_display(delta_lines);
    self.finish_scrollback_change(cx);
  }

  fn scroll_to_display_offset(&mut self, display_offset: usize, cx: &mut Context<Self>) {
    let Some(session) = self.session.as_mut() else {
      return;
    };
    session.set_display_offset(display_offset);
    self.finish_scrollback_change(cx);
  }

  fn finish_scrollback_change(&mut self, cx: &mut Context<Self>) {
    self.reset_selection();
    self.hovered_hyperlink = None;
    self.pending_link_activation = None;
    self.refresh_snapshot();
    self.refresh_visible_search_matches();
    cx.notify();
  }

  pub fn restart_session(&mut self, cx: &mut Context<Self>) {
    self.error = None;
    self.reset_selection();
    self.hovered_hyperlink = None;
    self.pending_link_activation = None;
    self.last_reported_mouse_state = None;
    self.scroll_remainder = px(0.0);
    self.marked_text = None;
    self.search_matches.clear();
    self.search_active_match = None;
    self.visible_search_matches.clear();
    self.visible_active_search_match = None;
    self.search_generation = self.search_generation.wrapping_add(1);
    self._working_directory_task = Task::ready(());
    self._search_task = Task::ready(());
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
    self.subscribe_to_session_events(cx);
    if self.search_open && !self.search_query.is_empty() {
      self.refresh_search_matches(cx);
    }
  }

  fn subscribe_to_session_events(&mut self, cx: &mut Context<Self>) {
    let Some(receiver) = self.session.as_ref().map(TerminalSession::event_receiver) else {
      self._event_task = Task::ready(());
      return;
    };

    if cx
      .background_executor()
      .scheduler_executor()
      .scheduler()
      .as_test()
      .is_some()
    {
      self._event_task = cx.spawn(async move |this, cx| {
        loop {
          cx.background_executor()
            .timer(TEST_SESSION_POLL_INTERVAL)
            .await;
          let Ok(first_event) = receiver.try_recv() else {
            continue;
          };
          let events = collect_pending_session_events(first_event, &receiver);
          if this
            .update(cx, |this, cx| {
              this.process_session_events(events, cx);
            })
            .is_err()
          {
            return;
          }
        }
      });
      return;
    }

    self._event_task = cx.spawn(async move |this, cx| {
      while let Ok(first_event) = receiver.recv().await {
        let events = collect_pending_session_events(first_event, &receiver);
        if this
          .update(cx, |this, cx| {
            this.process_session_events(events, cx);
          })
          .is_err()
        {
          return;
        }
      }
    });
  }

  fn refresh_working_directory(&mut self, cx: &mut Context<Self>) {
    let Some(tracker) = self
      .session
      .as_ref()
      .map(TerminalSession::working_directory_tracker)
    else {
      return;
    };
    if !tracker.begin_refresh() {
      return;
    }

    let refreshed_tracker = Arc::clone(&tracker);
    let refresh = cx.background_spawn(async move { refreshed_tracker.refresh() });
    self._working_directory_task = cx.spawn(async move |this, cx| {
      let Some(working_directory) = refresh.await else {
        return;
      };
      let _ = this.update(cx, |this, cx| {
        let belongs_to_current_session = this
          .session
          .as_ref()
          .map(TerminalSession::working_directory_tracker)
          .is_some_and(|current| Arc::ptr_eq(&current, &tracker));
        if belongs_to_current_session && this.working_directory.as_ref() != Some(&working_directory)
        {
          this.working_directory = Some(working_directory.clone());
          this.hovered_hyperlink = None;
          this.pending_link_activation = None;
          cx.emit(TerminalViewEvent::WorkingDirectoryChanged {
            path: working_directory,
          });
          cx.notify();
        }
      });
    });
  }

  fn ensure_search_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<InputState> {
    if let Some(input) = self.search_input.clone() {
      return input;
    }

    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in terminal..."));
    self.search_input_subscription = Some(cx.subscribe_in(&input, window, Self::on_search_input));
    self.search_input = Some(input.clone());
    input
  }

  fn on_search_input(
    &mut self,
    input: &Entity<InputState>,
    event: &InputEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match event {
      InputEvent::Change => {
        self.search_query = input.read(cx).value().to_string();
        self.search_active_match = None;
        self.refresh_search_matches(cx);
      }
      InputEvent::PressEnter { secondary, .. } => {
        if *secondary {
          self.search_previous(cx);
        } else {
          self.search_next(cx);
        }
      }
      _ => {}
    }
  }

  fn refresh_search_matches(&mut self, cx: &mut Context<Self>) {
    self.search_generation = self.search_generation.wrapping_add(1);
    let generation = self.search_generation;
    let query = self.search_query.clone();
    let preferred_match = self.search_active_match;
    self.visible_search_matches.clear();
    self.visible_active_search_match = None;

    let Some(search_handle) = self.session.as_ref().map(TerminalSession::search_handle) else {
      self.search_matches.clear();
      self.search_active_match = None;
      self._search_task = Task::ready(());
      cx.notify();
      return;
    };
    if query.is_empty() {
      self.search_matches.clear();
      self.search_active_match = None;
      self._search_task = Task::ready(());
      cx.notify();
      return;
    }

    let search = cx.background_spawn(async move { search_handle.find_matches(&query) });
    self._search_task = cx.spawn(async move |this, cx| {
      let matches = search.await;
      let _ = this.update(cx, |this, cx| {
        if !this.search_open || this.search_generation != generation {
          return;
        }

        this.search_matches = matches;
        if this.search_matches.is_empty() {
          this.search_active_match = None;
          this.visible_search_matches.clear();
          this.visible_active_search_match = None;
          cx.notify();
          return;
        }

        let active_match = preferred_match
          .unwrap_or_else(|| this.search_matches.len() - 1)
          .min(this.search_matches.len() - 1);
        this.activate_search_match(active_match, cx);
      });
    });
  }

  fn activate_search_match(&mut self, index: usize, cx: &mut Context<Self>) {
    let Some(found) = self.search_matches.get(index).copied() else {
      return;
    };
    let Some(session) = self.session.as_mut() else {
      return;
    };

    self.search_active_match = Some(index);
    session.scroll_to_search_match(found);
    self.refresh_snapshot();
    self.refresh_visible_search_matches();
    cx.notify();
  }

  fn refresh_visible_search_matches(&mut self) {
    let Some(session) = self.session.as_ref() else {
      self.visible_search_matches.clear();
      self.visible_active_search_match = None;
      return;
    };

    self.visible_search_matches = session.visible_search_ranges(&self.search_matches);
    self.visible_active_search_match = self
      .search_active_match
      .and_then(|index| self.search_matches.get(index).copied())
      .and_then(|found| session.visible_search_ranges(&[found]).into_iter().next());
  }

  fn search_next(&mut self, cx: &mut Context<Self>) {
    if self.search_matches.is_empty() {
      return;
    }
    let next = self
      .search_active_match
      .map(|index| (index + 1) % self.search_matches.len())
      .unwrap_or(0);
    self.activate_search_match(next, cx);
  }

  fn search_previous(&mut self, cx: &mut Context<Self>) {
    if self.search_matches.is_empty() {
      return;
    }
    let previous = self
      .search_active_match
      .map(|index| {
        if index == 0 {
          self.search_matches.len() - 1
        } else {
          index - 1
        }
      })
      .unwrap_or_else(|| self.search_matches.len() - 1);
    self.activate_search_match(previous, cx);
  }

  fn search_query_from_selection(&self) -> Option<String> {
    let selected = self.selection_text_for_copy()?.replace('\r', "");
    let first_line = selected.split('\n').next().unwrap_or_default().trim_end();
    (!first_line.is_empty()).then(|| first_line.to_string())
  }

  pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let was_open = self.search_open;
    self.search_open = true;
    let input = self.ensure_search_input(window, cx);
    if !was_open {
      let query = self
        .search_query_from_selection()
        .unwrap_or_else(|| input.read(cx).value().to_string());
      input.update(cx, |input, cx| {
        input.set_value(query.clone(), window, cx);
      });
      self.search_query = query;
      self.search_active_match = None;
      self.refresh_search_matches(cx);
    }
    input.update(cx, |input, cx| {
      input.focus(window, cx);
      input.select_all(window, cx);
    });
    cx.on_next_frame(window, |this, window, cx| {
      if let Some(input) = this.search_input.clone() {
        input.update(cx, |input, cx| {
          input.focus(window, cx);
          input.select_all(window, cx);
        });
      }
    });
    cx.notify();
  }

  pub fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
    if !self.search_open {
      return false;
    }

    self.search_open = false;
    self.search_query.clear();
    self.search_matches.clear();
    self.search_active_match = None;
    self.visible_search_matches.clear();
    self.visible_active_search_match = None;
    self.search_generation = self.search_generation.wrapping_add(1);
    self._search_task = Task::ready(());
    if let Some(input) = self.search_input.clone() {
      input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
    }
    self.focus_terminal(window, cx);
    cx.notify();
    true
  }

  pub fn is_search_open(&self) -> bool {
    self.search_open
  }

  pub(crate) fn visible_search_matches(&self) -> &[ViewportSelectionRange] {
    &self.visible_search_matches
  }

  pub(crate) fn visible_active_search_match(&self) -> Option<ViewportSelectionRange> {
    self.visible_active_search_match
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

  fn process_session_events(
    &mut self,
    events: impl IntoIterator<Item = TerminalEvent>,
    cx: &mut Context<Self>,
  ) {
    let Some(session) = self.session.as_mut() else {
      return;
    };

    let result = session.process_events(events);
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
      if self.search_open && !self.search_query.is_empty() {
        self.refresh_search_matches(cx);
      }
      cx.notify();
    }
    if result.wakeup {
      self.refresh_working_directory(cx);
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
      if self.search_open && !self.search_query.is_empty() {
        self.refresh_search_matches(cx);
      } else {
        self.refresh_visible_search_matches();
      }
      cx.notify();
    }
  }

  pub(crate) fn focus_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.focus_handle.focus(window, cx);
  }

  pub(crate) fn marked_text(&self) -> Option<&str> {
    self.marked_text.as_deref()
  }

  pub(crate) fn marked_text_range(&self) -> Option<std::ops::Range<usize>> {
    self
      .marked_text
      .as_ref()
      .map(|text| 0..text.encode_utf16().count())
  }

  pub(crate) fn set_marked_text(&mut self, text: &str, cx: &mut Context<Self>) {
    self.marked_text = (!text.is_empty()).then(|| text.to_string());
    cx.notify();
  }

  pub(crate) fn clear_marked_text(&mut self, cx: &mut Context<Self>) {
    if self.marked_text.take().is_some() {
      cx.notify();
    }
  }

  pub(crate) fn commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
    self.clear_marked_text(cx);
    if text.is_empty() {
      return;
    }

    self.reset_selection();
    if let Some(session) = self.session.as_mut() {
      session.input(text);
      self.refresh_snapshot();
      cx.notify();
    }
  }

  fn open_search_action(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
    self.open_search(window, cx);
    cx.stop_propagation();
  }

  fn close_search_action(&mut self, _: &CloseSearch, window: &mut Window, cx: &mut Context<Self>) {
    if self.close_search(window, cx) {
      cx.stop_propagation();
    }
  }

  fn search_next_action(&mut self, _: &SearchNext, _window: &mut Window, cx: &mut Context<Self>) {
    self.search_next(cx);
    cx.stop_propagation();
  }

  fn search_previous_action(
    &mut self,
    _: &SearchPrevious,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.search_previous(cx);
    cx.stop_propagation();
  }

  fn scroll_line_up_action(
    &mut self,
    _: &ScrollLineUp,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.scroll_scrollback_by(1, cx);
    cx.stop_propagation();
  }

  fn scroll_line_down_action(
    &mut self,
    _: &ScrollLineDown,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.scroll_scrollback_by(-1, cx);
    cx.stop_propagation();
  }

  fn scroll_page_up_action(
    &mut self,
    _: &ScrollPageUp,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let page_lines = self.screen.rows.saturating_sub(1).max(1) as i32;
    self.scroll_scrollback_by(page_lines, cx);
    cx.stop_propagation();
  }

  fn scroll_page_down_action(
    &mut self,
    _: &ScrollPageDown,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let page_lines = self.screen.rows.saturating_sub(1).max(1) as i32;
    self.scroll_scrollback_by(-page_lines, cx);
    cx.stop_propagation();
  }

  fn scroll_to_top_action(
    &mut self,
    _: &ScrollToTop,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let maximum_offset = self.screen.total_lines.saturating_sub(self.screen.rows);
    self.scroll_to_display_offset(maximum_offset, cx);
    cx.stop_propagation();
  }

  fn scroll_to_bottom_action(
    &mut self,
    _: &ScrollToBottom,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.scroll_to_display_offset(0, cx);
    cx.stop_propagation();
  }

  fn send_keystroke(
    &mut self,
    action: &SendKeystroke,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Ok(keystroke) = Keystroke::parse(&action.0) else {
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
    if self.search_open {
      return;
    }
    self.focus_terminal(window, cx);

    if should_defer_to_ime(event) {
      return;
    }

    self.clear_marked_text(cx);

    if self.matches_copy_shortcut(event) {
      self.copy_selection_to_clipboard(cx);
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

  fn hyperlink_at(&self, point: ViewportPoint) -> Option<TerminalLink> {
    link_at(&self.screen, point, self.working_directory.as_deref())
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

  fn render_jump_to_bottom(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
    if self.screen.display_offset == 0 {
      return None;
    }

    let theme = cx.theme().clone();
    Some(
      div()
        .debug_selector(|| TERMINAL_JUMP_TO_BOTTOM_DEBUG_SELECTOR.to_string())
        .absolute()
        .bottom(px(12.0))
        .left_0()
        .right(Scrollbar::width())
        .flex()
        .justify_center()
        .child(
          div()
            .rounded(px(999.0))
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .child(
              Button::new("terminal-jump-to-bottom")
                .icon(IconName::ChevronDown)
                .ghost()
                .small()
                .rounded(px(999.0))
                .tooltip("Jump to latest output")
                .on_click(cx.listener(|this, _, _, cx| {
                  this.scroll_to_display_offset(0, cx);
                })),
            ),
        )
        .into_any_element(),
    )
  }

  fn render_search_panel(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<gpui::AnyElement> {
    if !self.search_open {
      return None;
    }

    let input = self.ensure_search_input(window, cx);
    let theme = cx.theme().clone();
    let total_matches = self.search_matches.len();
    let active_match = self
      .search_active_match
      .map(|index| index + 1)
      .unwrap_or(0)
      .min(total_matches);
    let has_matches = total_matches > 0;
    let previous_view = cx.entity().clone();
    let next_view = cx.entity().clone();
    let close_view = cx.entity().clone();

    Some(
      div()
        .debug_selector(|| TERMINAL_SEARCH_DEBUG_SELECTOR.to_string())
        .absolute()
        .top(px(8.0))
        .right(px(0.0))
        .w_full()
        .max_w(px(340.0))
        .p_2()
        .occlude()
        .flex()
        .items_center()
        .gap_2()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
          div()
            .flex_1()
            .min_w(px(0.0))
            .child(Input::new(&input).small().border_color(theme.border)),
        )
        .child(
          div()
            .w(px(52.0))
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("{active_match}/{total_matches}")),
        )
        .child(
          Button::new("terminal-search-previous")
            .icon(IconName::ArrowUp)
            .ghost()
            .xsmall()
            .compact()
            .tooltip("Previous match")
            .disabled(!has_matches)
            .on_click(move |_, _, cx| {
              previous_view.update(cx, |view, cx| view.search_previous(cx));
            }),
        )
        .child(
          Button::new("terminal-search-next")
            .icon(IconName::ArrowDown)
            .ghost()
            .xsmall()
            .compact()
            .tooltip("Next match")
            .disabled(!has_matches)
            .on_click(move |_, _, cx| {
              next_view.update(cx, |view, cx| view.search_next(cx));
            }),
        )
        .child(
          Button::new("terminal-search-close")
            .icon(IconName::Close)
            .ghost()
            .xsmall()
            .compact()
            .tooltip("Close find")
            .on_click(move |_, window, cx| {
              close_view.update(cx, |view, cx| {
                view.close_search(window, cx);
              });
            }),
        )
        .into_any_element(),
    )
  }
}

impl EventEmitter<TerminalViewEvent> for TerminalView {}

impl Focusable for TerminalView {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for TerminalView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let line_height = px(f32::from(self.last_bounds.cell_height.max(1)));
    self.scroll_handle.update(&self.screen, line_height);
    if let Some(display_offset) = self.scroll_handle.take_pending_display_offset() {
      self.scroll_to_display_offset(display_offset, cx);
      self.scroll_handle.update(&self.screen, line_height);
    }

    let theme = cx.theme().clone();
    let is_dark = theme.is_dark();
    let (cursor_color, selection_color) = if is_dark {
      (rgb(0x7aa2f7).into(), rgb(0x33467a).into())
    } else {
      (rgb(0x2563eb).into(), rgb(0x60a5fa).into())
    };
    let terminal_palette = TerminalPalette::themed(
      is_dark,
      theme.background,
      theme.foreground,
      cursor_color,
      selection_color,
    );
    let terminal_screen = div()
      .id("terminal-screen")
      .debug_selector(|| TERMINAL_SCREEN_DEBUG_SELECTOR.to_string())
      .absolute()
      .top_0()
      .left_0()
      .right(Scrollbar::width())
      .bottom_0()
      .overflow_hidden()
      .bg(theme.background)
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
    let terminal_screen = if let Some(hovered) = self.hovered_hyperlink.clone() {
      terminal_screen.tooltip(move |window, cx| {
        Tooltip::new(hovered.link.tooltip.as_ref().to_string()).build(window, cx)
      })
    } else {
      terminal_screen
    };

    let banner_message = self
      .error
      .clone()
      .or_else(|| self.screen.exit_status.clone());
    let search_panel = self.render_search_panel(window, cx);
    let jump_to_bottom = self.render_jump_to_bottom(cx);
    let scrollbar = (self.screen.total_lines > self.screen.rows).then(|| {
      div()
        .debug_selector(|| TERMINAL_SCROLLBAR_DEBUG_SELECTOR.to_string())
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .w(Scrollbar::width())
        .child(
          Scrollbar::vertical(&self.scroll_handle)
            .id("terminal-scrollback")
            .mode(ScrollbarMode::Always)
            .viewport_from_layout(),
        )
    });

    div()
      .id("terminal-scaffold")
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, _, window, cx| {
          this.focus_terminal(window, cx);
        }),
      )
      .on_key_down(cx.listener(Self::on_key_down))
      .on_action(cx.listener(Self::open_search_action))
      .on_action(cx.listener(Self::close_search_action))
      .on_action(cx.listener(Self::search_next_action))
      .on_action(cx.listener(Self::search_previous_action))
      .on_action(cx.listener(Self::scroll_line_up_action))
      .on_action(cx.listener(Self::scroll_line_down_action))
      .on_action(cx.listener(Self::scroll_page_up_action))
      .on_action(cx.listener(Self::scroll_page_down_action))
      .on_action(cx.listener(Self::scroll_to_top_action))
      .on_action(cx.listener(Self::scroll_to_bottom_action))
      .on_action(cx.listener(Self::send_keystroke))
      .key_context(if self.search_open {
        TERMINAL_SEARCH_CONTEXT
      } else {
        TERMINAL_CONTEXT
      })
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
                  this.restart_session(cx);
                  cx.notify();
                })),
            ),
        )
      })
      .child(
        div()
          .relative()
          .flex_1()
          .min_h_0()
          .child(terminal_screen)
          .when_some(scrollbar, |this, scrollbar| this.child(scrollbar))
          .when_some(search_panel, |this, search_panel| this.child(search_panel))
          .when_some(jump_to_bottom, |this, jump| this.child(jump)),
      )
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
      "FiraCode Nerd Font Mono".into(),
      "FiraCode Nerd Font".into(),
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
  let mut cells = vec![" ".to_string(); screen.rows * screen.cols];
  for cell in &screen.cells {
    if cell.row < screen.rows && cell.col < screen.cols {
      let text = if cell.flags.contains(Flags::HIDDEN) {
        " ".to_string()
      } else if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
        String::new()
      } else {
        let mut text = cell.c.to_string();
        text.extend(cell.zerowidth.iter().copied());
        text
      };
      cells[cell.row * screen.cols + cell.col] = text;
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
      text.push_str(&cells[row * screen.cols + col]);
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
    TERMINAL_BANNER_DEBUG_SELECTOR, TERMINAL_JUMP_TO_BOTTOM_DEBUG_SELECTOR,
    TERMINAL_SCROLLBAR_DEBUG_SELECTOR, TERMINAL_SEARCH_DEBUG_SELECTOR,
    TERMINAL_SURFACE_DEBUG_SELECTOR, TerminalEvent, TerminalView, TerminalViewEvent,
    collect_pending_session_events, selection_matches_screen_text, selection_mode_for_click_count,
    selection_text_from_screen, should_defer_to_ime,
  };
  use crate::{
    ScreenSnapshot, TerminalBounds, TerminalCellSnapshot, TerminalSelectionMode, TerminalSession,
    ViewportPoint, ViewportSelectionRange,
  };
  use alacritty_terminal::term::cell::Flags;
  use alacritty_terminal::vte::ansi::{Color, NamedColor};
  use gpui::{
    AppContext, ClipboardItem, Context, Focusable, InteractiveElement, KeyDownEvent, Keystroke,
    Modifiers, MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, Render, ScrollDelta,
    ScrollWheelEvent, Styled, TestAppContext, TouchPhase, VisualTestContext, Window, div, point,
    px,
  };
  use std::{cell::RefCell, rc::Rc, sync::Arc};

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
          zerowidth: Arc::default(),
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
  fn pending_terminal_events_coalesce_wakeups() {
    let (sender, receiver) = async_channel::unbounded();
    for _ in 0..4 {
      sender
        .try_send(TerminalEvent::Wakeup)
        .expect("wakeup should queue");
    }
    sender
      .try_send(TerminalEvent::Title("shell".to_string()))
      .expect("title should queue");
    sender
      .try_send(TerminalEvent::Wakeup)
      .expect("wakeup should queue");

    let first_event = receiver.try_recv().expect("first event should be queued");
    let events = collect_pending_session_events(first_event, &receiver);

    assert_eq!(events.len(), 2);
    assert_eq!(
      events
        .iter()
        .filter(|event| matches!(event, TerminalEvent::Wakeup))
        .count(),
      1
    );
    assert!(
      events
        .iter()
        .any(|event| matches!(event, TerminalEvent::Title(title) if title == "shell"))
    );
  }

  #[gpui::test]
  fn clicking_a_path_emits_its_file_position(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let root =
      std::env::temp_dir().join(format!("reviu-terminal-path-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("link fixture directory should be created");
    let path = root.join("src/main.rs");
    std::fs::write(&path, "fn main() {}\n").expect("link fixture should be written");
    let canonical_path = path.canonicalize().expect("link fixture should resolve");

    let view = cx.new(|cx| TerminalView::new(None, cx));
    let events: Rc<RefCell<Vec<TerminalViewEvent>>> = Rc::default();
    cx.update(|cx| {
      let events = events.clone();
      cx.subscribe(&view, move |_, event: &TerminalViewEvent, _| {
        events.borrow_mut().push(event.clone());
      })
      .detach();
    });
    view.update(cx, |view, cx| {
      view.working_directory = Some(root.clone());
      view.screen = screen_from_lines(&["src/main.rs:42:8"]);
      let modifiers = secondary_click_modifiers();
      let point = ViewportPoint { row: 0, col: 4 };
      view.handle_mouse_down(MouseButton::Left, point, 1, modifiers, cx);
      view.handle_mouse_up(MouseButton::Left, point, modifiers, cx);
    });

    assert_eq!(
      events.borrow().as_slice(),
      [TerminalViewEvent::OpenFile {
        path: canonical_path,
        line: Some(42),
        column: Some(8),
      }]
    );
    std::fs::remove_dir_all(root).expect("link fixture should be removed");
  }

  #[test]
  fn ime_only_handles_printable_unmodified_keys() {
    let mut printable = key_event("a", Modifiers::default());
    printable.prefer_character_input = true;
    assert!(should_defer_to_ime(&printable));

    let mut enter = key_event("enter", Modifiers::default());
    enter.keystroke.key_char = Some("\n".to_string());
    enter.prefer_character_input = true;
    assert!(!should_defer_to_ime(&enter));

    let modifiers = Modifiers {
      control: true,
      ..Modifiers::default()
    };
    let mut control = key_event("c", modifiers);
    control.prefer_character_input = true;
    assert!(!should_defer_to_ime(&control));
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
          zerowidth: Arc::default(),
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
          zerowidth: Arc::default(),
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
          zerowidth: Arc::default(),
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
          zerowidth: Arc::default(),
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
          zerowidth: Arc::default(),
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
          zerowidth: Arc::default(),
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
  fn selection_text_omits_wide_spacers_and_keeps_combining_marks() {
    let mut screen = screen_from_lines(&["日 e"]);
    screen.cols = 3;
    screen.cells[0].flags = Flags::WIDE_CHAR;
    screen.cells[1].flags = Flags::WIDE_CHAR_SPACER;
    screen.cells[2].c = 'e';
    screen.cells[2].zerowidth = Arc::from(['\u{301}']);

    assert_eq!(
      selection_text_from_screen(
        &screen,
        ViewportSelectionRange {
          start: ViewportPoint { row: 0, col: 0 },
          end: ViewportPoint { row: 0, col: 2 },
        },
      ),
      Some("日e\u{301}".to_string())
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
          zerowidth: Arc::default(),
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
          zerowidth: Arc::default(),
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
          zerowidth: Arc::default(),
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
  fn marked_text_range_uses_utf16_offsets(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let view = cx.new(|cx| TerminalView::new(None, cx));
    view.update(cx, |view, cx| {
      view.set_marked_text("日😀", cx);
      assert_eq!(view.marked_text(), Some("日😀"));
      assert_eq!(view.marked_text_range(), Some(0..3));

      view.clear_marked_text(cx);
      assert_eq!(view.marked_text(), None);
      assert_eq!(view.marked_text_range(), None);
    });
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
  fn terminal_search_opens_and_restores_terminal_focus_when_closed(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(None, cx));
    let cx: &mut VisualTestContext = cx;
    view.update_in(cx, |view, window, cx| view.open_search(window, cx));

    assert!(view.read_with(cx, |view, _| view.is_search_open()));
    assert!(cx.debug_bounds(TERMINAL_SEARCH_DEBUG_SELECTOR).is_some());
    let search_input = view
      .read_with(cx, |view, _| view.search_input.clone())
      .expect("search input should exist");
    let search_focused =
      cx.update(|window, app| search_input.read(app).focus_handle(app).is_focused(window));
    assert!(search_focused);

    view.update_in(cx, |view, window, cx| {
      assert!(view.close_search(window, cx));
    });

    assert!(!view.read_with(cx, |view, _| view.is_search_open()));
    assert!(cx.debug_bounds(TERMINAL_SEARCH_DEBUG_SELECTOR).is_none());
    let terminal_focused =
      cx.update(|window, app| view.read_with(app, |view, _| view.focus_handle.is_focused(window)));
    assert!(terminal_focused);
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
  fn terminal_scrollbar_and_jump_button_reflect_scrollback(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(None, cx));
    let cx: &mut VisualTestContext = cx;
    view.update(cx, |view, cx| {
      view.screen = screen_from_lines(&["one", "two", "three"]);
      view.screen.total_lines = 12;
      view.screen.display_offset = 4;
      cx.notify();
    });

    assert!(cx.debug_bounds(TERMINAL_SCROLLBAR_DEBUG_SELECTOR).is_some());
    assert!(
      cx.debug_bounds(TERMINAL_JUMP_TO_BOTTOM_DEBUG_SELECTOR)
        .is_some()
    );

    view.update(cx, |view, cx| {
      view.screen.display_offset = 0;
      cx.notify();
    });
    assert!(
      cx.debug_bounds(TERMINAL_JUMP_TO_BOTTOM_DEBUG_SELECTOR)
        .is_none()
    );
    assert!(cx.debug_bounds(TERMINAL_SCROLLBAR_DEBUG_SELECTOR).is_some());

    view.update(cx, |view, cx| {
      view.screen.total_lines = view.screen.rows;
      cx.notify();
    });
    assert!(cx.debug_bounds(TERMINAL_SCROLLBAR_DEBUG_SELECTOR).is_none());
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

    let hovered = view.read_with(cx, |view, _| {
      view
        .hovered_hyperlink
        .as_ref()
        .map(|hovered| hovered.link.tooltip.clone())
    });
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

      let shift = Modifiers {
        shift: true,
        ..Default::default()
      };
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

    view.update(cx, |view, cx| {
      assert!(view.session.is_some(), "initial spawn should succeed");
      view.error = Some("forced error".to_string());
      view.restart_session(cx);

      assert!(view.error.is_none(), "restart should clear error");
      assert!(view.session.is_some(), "restart should respawn session");
    });
  }
}
