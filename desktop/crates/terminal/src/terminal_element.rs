use gpui::{
  App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, Entity, Focusable, FontStyle,
  FontWeight, GlobalElementId, Hitbox, HitboxBehavior, InputHandler, InspectorElementId,
  IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
  ScrollWheelEvent, ShapedLine, StrikethroughStyle, Style, TextAlign, TextRun, UTF16Selection,
  UnderlineStyle, Window, fill, point, px, relative,
};
use std::sync::Arc;

use alacritty_terminal::{
  term::cell::Flags,
  vte::ansi::{Color, NamedColor},
};

use crate::{
  ScreenSnapshot, TerminalBounds, TerminalCellSnapshot, ViewportPoint, colors::TerminalPalette,
  terminal_view::TerminalView,
};

#[derive(Clone)]
struct RowLayout {
  row: usize,
  byte_offsets: Arc<[usize]>,
  shaped: ShapedLine,
}

#[derive(Default)]
struct RenderCellState {
  previous_cell_had_joiner: bool,
  awaiting_join_target: bool,
  joined_width_compensation: usize,
}

pub(crate) struct TerminalPrepaintState {
  hitbox: Hitbox,
  screen: ScreenSnapshot,
  row_layouts: Arc<[RowLayout]>,
  line_height: Pixels,
  cell_width: Pixels,
  cursor_bounds: Option<Bounds<Pixels>>,
}

#[derive(Clone, Copy, PartialEq)]
struct RowStyle {
  foreground: gpui::Hsla,
  background: Option<gpui::Hsla>,
  bold: bool,
  italic: bool,
  underline: Option<UnderlineStyle>,
  strikethrough: Option<StrikethroughStyle>,
}

pub(crate) struct TerminalElement {
  view: Entity<TerminalView>,
  palette: TerminalPalette,
  is_focused: bool,
}

impl TerminalElement {
  pub(crate) fn new(
    view: Entity<TerminalView>,
    palette: TerminalPalette,
    is_focused: bool,
  ) -> Self {
    Self {
      view,
      palette,
      is_focused,
    }
  }
}

impl IntoElement for TerminalElement {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

struct TerminalInputHandler {
  view: Entity<TerminalView>,
  cursor_bounds: Option<Bounds<Pixels>>,
  cell_width: Pixels,
  element_bounds: Bounds<Pixels>,
}

impl InputHandler for TerminalInputHandler {
  fn selected_text_range(
    &mut self,
    _ignore_disabled_input: bool,
    _window: &mut Window,
    _cx: &mut App,
  ) -> Option<UTF16Selection> {
    Some(UTF16Selection {
      range: 0..0,
      reversed: false,
    })
  }

  fn marked_text_range(
    &mut self,
    _window: &mut Window,
    cx: &mut App,
  ) -> Option<std::ops::Range<usize>> {
    self.view.read(cx).marked_text_range()
  }

  fn text_for_range(
    &mut self,
    _range_utf16: std::ops::Range<usize>,
    _adjusted_range: &mut Option<std::ops::Range<usize>>,
    _window: &mut Window,
    _cx: &mut App,
  ) -> Option<String> {
    None
  }

  fn replace_text_in_range(
    &mut self,
    _replacement_range: Option<std::ops::Range<usize>>,
    text: &str,
    _window: &mut Window,
    cx: &mut App,
  ) {
    self.view.update(cx, |view, cx| view.commit_text(text, cx));
  }

  fn replace_and_mark_text_in_range(
    &mut self,
    _range_utf16: Option<std::ops::Range<usize>>,
    new_text: &str,
    _new_selected_range: Option<std::ops::Range<usize>>,
    _window: &mut Window,
    cx: &mut App,
  ) {
    self
      .view
      .update(cx, |view, cx| view.set_marked_text(new_text, cx));
  }

  fn unmark_text(&mut self, _window: &mut Window, cx: &mut App) {
    self.view.update(cx, TerminalView::clear_marked_text);
  }

  fn bounds_for_range(
    &mut self,
    range_utf16: std::ops::Range<usize>,
    _window: &mut Window,
    _cx: &mut App,
  ) -> Option<Bounds<Pixels>> {
    let mut bounds = self.cursor_bounds?;
    bounds.origin.x += self.cell_width * range_utf16.start as f32;
    Some(bounds)
  }

  fn character_index_for_point(
    &mut self,
    _point: Point<Pixels>,
    _window: &mut Window,
    _cx: &mut App,
  ) -> Option<usize> {
    None
  }

  fn element_bounds(&mut self, _window: &mut Window, _cx: &mut App) -> Option<Bounds<Pixels>> {
    Some(self.element_bounds)
  }

  fn apple_press_and_hold_enabled(&mut self) -> bool {
    false
  }
}

impl Element for TerminalElement {
  type RequestLayoutState = ();
  type PrepaintState = TerminalPrepaintState;

  fn id(&self) -> Option<ElementId> {
    None
  }

  fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
    None
  }

  fn request_layout(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    let mut style = Style::default();
    style.size.width = relative(1.).into();
    style.size.height = relative(1.).into();
    (window.request_layout(style, [], cx), ())
  }

  fn prepaint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    window: &mut Window,
    cx: &mut App,
  ) -> Self::PrepaintState {
    let terminal_bounds = terminal_bounds_for_surface(bounds, window, cx);
    self.view.update(cx, |view, cx| {
      view.sync_bounds(terminal_bounds, cx);
    });
    let screen = self.view.read(cx).screen().clone();
    let row_layouts = build_row_layouts(&screen, &self.palette, window);
    let line_height = px(f32::from(terminal_bounds.cell_height));
    let cell_width = px(f32::from(terminal_bounds.cell_width));

    TerminalPrepaintState {
      hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
      cursor_bounds: terminal_cursor_bounds(bounds, &screen, &row_layouts, line_height),
      screen,
      row_layouts: row_layouts.into(),
      line_height,
      cell_width,
    }
  }

  fn paint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    prepaint: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
    let marked_text = self.view.read(cx).marked_text().map(str::to_string);
    window.handle_input(
      &self.view.read(cx).focus_handle(cx),
      TerminalInputHandler {
        view: self.view.clone(),
        cursor_bounds: prepaint.cursor_bounds,
        cell_width: prepaint.cell_width,
        element_bounds: bounds,
      },
      cx,
    );
    window.paint_quad(fill(bounds, self.palette.background()));

    if prepaint.hitbox.is_hovered(window) {
      let cursor_style = viewport_point_for_position(
        window.mouse_position(),
        bounds,
        &prepaint.row_layouts,
        prepaint.line_height,
        false,
      )
      .filter(|point| {
        self
          .view
          .read(cx)
          .should_show_link_cursor(*point, window.modifiers())
      })
      .map(|_| CursorStyle::PointingHand)
      .unwrap_or(CursorStyle::IBeam);
      window.set_cursor_style(cursor_style, &prepaint.hitbox);
    }

    let selection = self.view.read(cx).selection_range();
    for row_layout in prepaint.row_layouts.iter() {
      let row_origin = point(
        bounds.left(),
        row_top(bounds, row_layout.row, prepaint.line_height),
      );
      row_layout
        .shaped
        .paint_background(
          row_origin,
          prepaint.line_height,
          TextAlign::Left,
          Some(bounds.size.width),
          window,
          cx,
        )
        .ok();

      if let Some(selection) = selection
        && let Some((selection_start, selection_end)) =
          row_selection_bounds(row_layout, &prepaint.screen, selection)
      {
        let selection_bounds = Bounds::from_corners(
          point(bounds.left() + selection_start, row_origin.y),
          point(
            bounds.left() + selection_end,
            row_origin.y + prepaint.line_height,
          ),
        );
        window.paint_quad(fill(selection_bounds, self.palette.selection()));
      }

      row_layout
        .shaped
        .paint(
          row_origin,
          prepaint.line_height,
          TextAlign::Left,
          Some(bounds.size.width),
          window,
          cx,
        )
        .ok();
    }

    if let Some(marked_text) = marked_text {
      paint_marked_text(
        &marked_text,
        window,
        cx,
        &self.palette,
        &prepaint.screen,
        prepaint.cursor_bounds,
        prepaint.line_height,
      );
    } else {
      paint_cursor(
        window,
        cx,
        &self.palette,
        bounds,
        &prepaint.screen,
        &prepaint.row_layouts,
        prepaint.line_height,
        self.is_focused,
      );
    }

    window.on_mouse_event({
      let view = self.view.clone();
      let hitbox = prepaint.hitbox.clone();
      let row_layouts = Arc::clone(&prepaint.row_layouts);
      let line_height = prepaint.line_height;
      move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
          return;
        }

        let Some(point) =
          viewport_point_for_position(event.position, bounds, &row_layouts, line_height, true)
        else {
          return;
        };

        view.update(cx, |view, cx| {
          view.focus_terminal(window, cx);
          view.handle_mouse_down(event.button, point, event.click_count, event.modifiers, cx);
        });
        cx.stop_propagation();
      }
    });

    window.on_mouse_event({
      let view = self.view.clone();
      let hitbox = prepaint.hitbox.clone();
      let row_layouts = Arc::clone(&prepaint.row_layouts);
      let line_height = prepaint.line_height;
      move |event: &MouseMoveEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
          return;
        }

        let hovered = hitbox.is_hovered(window);
        if hovered {
          window.refresh();
        }
        let hover_point = if hovered {
          viewport_point_for_position(event.position, bounds, &row_layouts, line_height, false)
        } else {
          None
        };
        view.update(cx, |view, cx| {
          view.update_hovered_hyperlink(hover_point, cx);
        });

        if !view
          .read(cx)
          .should_handle_mouse_move(hovered, event.pressed_button, event.modifiers)
        {
          return;
        }

        let Some(point) =
          viewport_point_for_position(event.position, bounds, &row_layouts, line_height, !hovered)
        else {
          return;
        };

        view.update(cx, |view, cx| {
          view.handle_mouse_move(point, event.pressed_button, event.modifiers, cx);
        });
        cx.stop_propagation();
      }
    });

    window.on_modifiers_changed({
      let hitbox = prepaint.hitbox.clone();
      move |_event, window, _cx| {
        if hitbox.is_hovered(window) {
          window.refresh();
        }
      }
    });

    window.on_mouse_event({
      let view = self.view.clone();
      let hitbox = prepaint.hitbox.clone();
      let row_layouts = Arc::clone(&prepaint.row_layouts);
      let line_height = prepaint.line_height;
      move |event: &MouseUpEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
          return;
        }

        if !view
          .read(cx)
          .should_handle_mouse_up(event.button, event.modifiers)
        {
          return;
        }

        let hovered = hitbox.is_hovered(window);
        let Some(point) =
          viewport_point_for_position(event.position, bounds, &row_layouts, line_height, !hovered)
        else {
          return;
        };

        view.update(cx, |view, cx| {
          view.handle_mouse_up(event.button, point, event.modifiers, cx);
        });
        cx.stop_propagation();
      }
    });

    window.on_mouse_event({
      let view = self.view.clone();
      let scroll_hitbox = prepaint.hitbox.clone();
      let row_layouts = Arc::clone(&prepaint.row_layouts);
      let line_height = prepaint.line_height;
      move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !scroll_hitbox.should_handle_scroll(window) {
          return;
        }

        let Some(point) =
          viewport_point_for_position(event.position, bounds, &row_layouts, line_height, true)
        else {
          return;
        };

        view.update(cx, |view, cx| {
          view.handle_scroll(event, point, cx);
        });
        cx.stop_propagation();
      }
    });
  }
}

fn build_row_layouts(
  screen: &ScreenSnapshot,
  palette: &TerminalPalette,
  window: &mut Window,
) -> Vec<RowLayout> {
  if screen.rows == 0 || screen.cols == 0 {
    return Vec::new();
  }

  let mut cell_grid = vec![vec![None; screen.cols]; screen.rows];
  for cell in &screen.cells {
    if cell.row < screen.rows && cell.col < screen.cols {
      cell_grid[cell.row][cell.col] = Some(cell);
    }
  }

  let text_style = window.text_style();
  let font_size = text_style.font_size.to_pixels(window.rem_size());
  let font = text_style.font();

  (0..screen.rows)
    .map(|row| {
      let mut text = String::with_capacity(screen.cols);
      let mut byte_offsets = Vec::with_capacity(screen.cols + 1);
      let mut runs = Vec::new();
      let mut active_style = None;
      let mut active_len = 0usize;
      let mut render_state = RenderCellState::default();

      byte_offsets.push(0);

      // Bound by the screen geometry, which can be narrower than the stored row.
      #[allow(clippy::needless_range_loop)]
      for col in 0..screen.cols {
        let cell = cell_grid[row][col];
        let text_start = text.len();
        append_rendered_cell(&mut text, cell, &mut render_state);
        byte_offsets.push(text.len());

        let cell_len = text.len() - text_start;
        if cell_len == 0 {
          continue;
        }
        let style = style_for_cell(cell, palette, &screen.colors);
        if active_style == Some(style) {
          active_len += cell_len;
        } else {
          if let Some(previous_style) = active_style.take() {
            runs.push(text_run_for_style(active_len, &font, previous_style));
          }
          active_style = Some(style);
          active_len = cell_len;
        }
      }

      if let Some(previous_style) = active_style {
        runs.push(text_run_for_style(active_len, &font, previous_style));
      }

      let shaped = window
        .text_system()
        .shape_line(text.into(), font_size, &runs, None);

      RowLayout {
        row,
        byte_offsets: byte_offsets.into(),
        shaped,
      }
    })
    .collect()
}

fn append_rendered_cell(
  text: &mut String,
  cell: Option<&TerminalCellSnapshot>,
  state: &mut RenderCellState,
) {
  let Some(cell) = cell else {
    flush_joined_width_compensation(text, state);
    text.push(' ');
    state.previous_cell_had_joiner = false;
    state.awaiting_join_target = false;
    return;
  };
  if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
    if state.previous_cell_had_joiner {
      state.awaiting_join_target = true;
      state.previous_cell_had_joiner = false;
    } else {
      flush_joined_width_compensation(text, state);
    }
    return;
  }

  if state.previous_cell_had_joiner {
    state.awaiting_join_target = true;
  } else if !state.awaiting_join_target {
    flush_joined_width_compensation(text, state);
  }

  if cell.flags.contains(Flags::HIDDEN) {
    text.push(' ');
  } else {
    text.push(cell.c);
    text.extend(cell.zerowidth.iter().copied());
  }

  if state.awaiting_join_target {
    state.joined_width_compensation += usize::from(cell.flags.contains(Flags::WIDE_CHAR)) + 1;
    state.awaiting_join_target = false;
  }
  state.previous_cell_had_joiner = cell.zerowidth.contains(&'\u{200d}');
}

fn flush_joined_width_compensation(text: &mut String, state: &mut RenderCellState) {
  text.extend(std::iter::repeat_n(' ', state.joined_width_compensation));
  state.joined_width_compensation = 0;
}

fn style_for_cell(
  cell: Option<&TerminalCellSnapshot>,
  palette: &TerminalPalette,
  colors: &alacritty_terminal::term::color::Colors,
) -> RowStyle {
  let flags = cell.map(|cell| cell.flags).unwrap_or_else(Flags::empty);
  let explicit_underline_color = cell.and_then(|cell| cell.underline_color);
  let has_hyperlink = cell.is_some_and(|cell| cell.hyperlink_uri.is_some());
  let mut foreground = palette.resolve(
    cell
      .map(|cell| cell.fg)
      .unwrap_or(Color::Named(NamedColor::Foreground)),
    colors,
  );
  let mut background = palette.resolve(
    cell
      .map(|cell| cell.bg)
      .unwrap_or(Color::Named(NamedColor::Background)),
    colors,
  );

  if flags.contains(Flags::INVERSE) {
    std::mem::swap(&mut foreground, &mut background);
  }
  if flags.contains(Flags::DIM) {
    foreground = foreground.opacity(0.72);
  }

  let underline =
    (flags.intersects(Flags::ALL_UNDERLINES) || has_hyperlink).then_some(UnderlineStyle {
      color: Some(
        explicit_underline_color
          .map(|color| palette.resolve(color, colors))
          .unwrap_or(foreground),
      ),
      thickness: px(1.0),
      wavy: flags.contains(Flags::UNDERCURL),
    });
  let strikethrough = flags
    .contains(Flags::STRIKEOUT)
    .then_some(StrikethroughStyle {
      color: Some(foreground),
      thickness: px(1.0),
    });

  RowStyle {
    foreground,
    background: (background != palette.background()).then_some(background),
    bold: flags.contains(Flags::BOLD),
    italic: flags.contains(Flags::ITALIC),
    underline,
    strikethrough,
  }
}

fn text_run_for_style(len: usize, font: &gpui::Font, style: RowStyle) -> TextRun {
  let mut font = font.clone();
  if style.bold {
    font.weight = FontWeight::BOLD;
  }
  if style.italic {
    font.style = FontStyle::Italic;
  }

  TextRun {
    len,
    font,
    color: style.foreground,
    background_color: style.background,
    underline: style.underline,
    strikethrough: style.strikethrough,
  }
}

fn terminal_bounds_for_surface(
  bounds: Bounds<Pixels>,
  window: &mut Window,
  cx: &mut App,
) -> TerminalBounds {
  let text_style = window.text_style();
  let font_size = text_style.font_size.to_pixels(window.rem_size());
  let font_id = cx.text_system().resolve_font(&text_style.font());
  let default_bounds = TerminalBounds::default();
  let cell_width = cx
    .text_system()
    .em_advance(font_id, font_size)
    .unwrap_or(px(f32::from(default_bounds.cell_width)))
    .ceil()
    .max(px(1.0));
  let line_height = text_style
    .line_height_in_pixels(window.rem_size())
    .ceil()
    .max(px(1.0));

  TerminalBounds::from_size(
    f32::from(bounds.size.width.max(px(0.0))),
    f32::from(bounds.size.height.max(px(0.0))),
    f32::from(cell_width) as u16,
    f32::from(line_height) as u16,
  )
}

fn row_top(bounds: Bounds<Pixels>, row: usize, line_height: Pixels) -> Pixels {
  bounds.top() + line_height * row as f32
}

fn row_selection_bounds(
  row_layout: &RowLayout,
  screen: &ScreenSnapshot,
  selection: crate::ViewportSelectionRange,
) -> Option<(Pixels, Pixels)> {
  if screen.cols == 0 {
    return None;
  }

  let selection = selection.normalized();
  if row_layout.row < selection.start.row || row_layout.row > selection.end.row {
    return None;
  }

  let start_col = if row_layout.row == selection.start.row {
    selection.start.col
  } else {
    0
  };
  let end_col = if row_layout.row == selection.end.row {
    selection.end.col
  } else {
    screen.cols.saturating_sub(1)
  };
  if start_col > end_col || end_col >= screen.cols {
    return None;
  }

  Some((
    x_for_column(row_layout, start_col),
    x_for_column(row_layout, end_col + 1),
  ))
}

fn terminal_cursor_bounds(
  bounds: Bounds<Pixels>,
  screen: &ScreenSnapshot,
  row_layouts: &[RowLayout],
  line_height: Pixels,
) -> Option<Bounds<Pixels>> {
  let cursor = screen.cursor?;
  let row_layout = row_layouts.get(cursor.point.row)?;
  let cursor_width = cursor_span(screen, cursor.point);
  let left = bounds.left() + x_for_column(row_layout, cursor.point.col);
  let right = bounds.left() + x_for_column(row_layout, cursor.point.col + cursor_width);
  let top = row_top(bounds, cursor.point.row, line_height);
  Some(Bounds::from_corners(
    point(left, top),
    point(right, top + line_height),
  ))
}

fn paint_marked_text(
  marked_text: &str,
  window: &mut Window,
  cx: &mut App,
  palette: &TerminalPalette,
  screen: &ScreenSnapshot,
  cursor_bounds: Option<Bounds<Pixels>>,
  line_height: Pixels,
) {
  let Some(cursor_bounds) = cursor_bounds else {
    return;
  };
  let text = marked_text.replace(['\r', '\n'], " ");
  if text.is_empty() {
    return;
  }

  let text_style = window.text_style();
  let font_size = text_style.font_size.to_pixels(window.rem_size());
  let foreground = palette.resolve(Color::Named(NamedColor::Foreground), &screen.colors);
  let run = TextRun {
    len: text.len(),
    font: text_style.font(),
    color: foreground,
    background_color: Some(palette.background()),
    underline: Some(UnderlineStyle {
      color: Some(foreground),
      thickness: px(1.0),
      wavy: false,
    }),
    strikethrough: None,
  };
  let shaped = window
    .text_system()
    .shape_line(text.into(), font_size, &[run], None);
  let right = cursor_bounds.left() + shaped.width().max(cursor_bounds.size.width);
  window.paint_quad(fill(
    Bounds::from_corners(
      cursor_bounds.origin,
      point(right, cursor_bounds.top() + line_height),
    ),
    palette.background(),
  ));
  let _ = shaped.paint(
    cursor_bounds.origin,
    line_height,
    TextAlign::Left,
    None,
    window,
    cx,
  );
}

fn paint_cursor(
  window: &mut Window,
  cx: &mut App,
  palette: &TerminalPalette,
  bounds: Bounds<Pixels>,
  screen: &ScreenSnapshot,
  row_layouts: &[RowLayout],
  line_height: Pixels,
  is_focused: bool,
) {
  let Some(cursor) = screen.cursor else {
    return;
  };
  let Some(row_layout) = row_layouts.get(cursor.point.row) else {
    return;
  };

  let cursor_width = cursor_span(screen, cursor.point);
  let cursor_left = bounds.left() + x_for_column(row_layout, cursor.point.col);
  let cursor_right = bounds.left() + x_for_column(row_layout, cursor.point.col + cursor_width);
  let cursor_top = row_top(bounds, cursor.point.row, line_height);
  let cursor_bottom = cursor_top + line_height;
  let cursor_color = palette.cursor();

  let shape = if is_focused {
    cursor.shape
  } else {
    alacritty_terminal::vte::ansi::CursorShape::HollowBlock
  };

  match shape {
    alacritty_terminal::vte::ansi::CursorShape::Hidden => {}
    alacritty_terminal::vte::ansi::CursorShape::Block => {
      window.paint_quad(fill(
        Bounds::from_corners(
          point(cursor_left, cursor_top),
          point(cursor_right, cursor_bottom),
        ),
        cursor_color,
      ));
      paint_cursor_glyph(
        window,
        cx,
        palette,
        screen,
        cursor.point,
        point(cursor_left, cursor_top),
        line_height,
      );
    }
    alacritty_terminal::vte::ansi::CursorShape::Underline => {
      let height = px(2.0);
      window.paint_quad(fill(
        Bounds::from_corners(
          point(cursor_left, cursor_bottom - height),
          point(cursor_right, cursor_bottom),
        ),
        cursor_color,
      ));
    }
    alacritty_terminal::vte::ansi::CursorShape::Beam => {
      let width = px(2.0);
      window.paint_quad(fill(
        Bounds::from_corners(
          point(cursor_left, cursor_top),
          point((cursor_left + width).min(cursor_right), cursor_bottom),
        ),
        cursor_color,
      ));
    }
    alacritty_terminal::vte::ansi::CursorShape::HollowBlock => {
      let stroke = px(1.0);
      window.paint_quad(fill(
        Bounds::from_corners(
          point(cursor_left, cursor_top),
          point(cursor_right, cursor_top + stroke),
        ),
        cursor_color,
      ));
      window.paint_quad(fill(
        Bounds::from_corners(
          point(cursor_left, cursor_bottom - stroke),
          point(cursor_right, cursor_bottom),
        ),
        cursor_color,
      ));
      window.paint_quad(fill(
        Bounds::from_corners(
          point(cursor_left, cursor_top),
          point(cursor_left + stroke, cursor_bottom),
        ),
        cursor_color,
      ));
      window.paint_quad(fill(
        Bounds::from_corners(
          point(cursor_right - stroke, cursor_top),
          point(cursor_right, cursor_bottom),
        ),
        cursor_color,
      ));
    }
  }
}

fn paint_cursor_glyph(
  window: &mut Window,
  cx: &mut App,
  palette: &TerminalPalette,
  screen: &ScreenSnapshot,
  cursor_point: ViewportPoint,
  origin: Point<Pixels>,
  line_height: Pixels,
) {
  let Some(cell) = screen
    .cells
    .iter()
    .find(|cell| cell.row == cursor_point.row && cell.col == cursor_point.col)
  else {
    return;
  };
  if cell.flags.contains(Flags::HIDDEN) {
    return;
  }
  let ch = match cell.c {
    '\t' | '\0' | ' ' => return,
    other => other,
  };

  let text_style = window.text_style();
  let font_size = text_style.font_size.to_pixels(window.rem_size());
  let mut font = text_style.font();
  if cell.flags.contains(Flags::BOLD) {
    font.weight = FontWeight::BOLD;
  }
  if cell.flags.contains(Flags::ITALIC) {
    font.style = FontStyle::Italic;
  }

  let mut buf = String::with_capacity(ch.len_utf8() + cell.zerowidth.len());
  buf.push(ch);
  buf.extend(cell.zerowidth.iter().copied());
  let run = TextRun {
    len: buf.len(),
    font,
    color: palette.background(),
    background_color: None,
    underline: None,
    strikethrough: None,
  };
  let shaped = window
    .text_system()
    .shape_line(buf.into(), font_size, &[run], None);
  let _ = shaped.paint(origin, line_height, TextAlign::Left, None, window, cx);
}

fn cursor_span(screen: &ScreenSnapshot, point: ViewportPoint) -> usize {
  if screen.cells.iter().any(|cell| {
    cell.row == point.row && cell.col == point.col && cell.flags.contains(Flags::WIDE_CHAR)
  }) {
    2
  } else {
    1
  }
}

fn viewport_point_for_position(
  position: Point<Pixels>,
  bounds: Bounds<Pixels>,
  row_layouts: &[RowLayout],
  line_height: Pixels,
  clamp_to_bounds: bool,
) -> Option<ViewportPoint> {
  if row_layouts.is_empty() {
    return None;
  }
  if !clamp_to_bounds && !bounds.contains(&position) {
    return None;
  }

  let x = if clamp_to_bounds {
    position.x.max(bounds.left()).min(bounds.right())
  } else {
    position.x
  };
  let y = if clamp_to_bounds {
    position.y.max(bounds.top()).min(bounds.bottom())
  } else {
    position.y
  };

  let row = (((y - bounds.top()) / line_height).floor() as usize).min(row_layouts.len() - 1);
  let row_layout = &row_layouts[row];
  if row_layout.byte_offsets.len() <= 1 {
    return None;
  }

  let byte_index = row_layout
    .shaped
    .closest_index_for_x((x - bounds.left()).max(px(0.0)));
  Some(ViewportPoint {
    row,
    col: column_for_byte_index(&row_layout.byte_offsets, byte_index),
  })
}

fn column_for_byte_index(byte_offsets: &[usize], byte_index: usize) -> usize {
  byte_offsets
    .partition_point(|offset| *offset <= byte_index)
    .saturating_sub(1)
    .min(byte_offsets.len().saturating_sub(2))
}

fn x_for_column(row_layout: &RowLayout, column: usize) -> Pixels {
  let byte_index = row_layout.byte_offsets[column.min(row_layout.byte_offsets.len() - 1)];
  row_layout.shaped.x_for_index(byte_index)
}

#[cfg(test)]
mod tests {
  use super::{RenderCellState, append_rendered_cell, column_for_byte_index, style_for_cell};
  use crate::{TerminalCellSnapshot, colors::TerminalPalette};
  use alacritty_terminal::{
    term::{cell::Flags, color::Colors},
    vte::ansi::{Color, NamedColor, Rgb},
  };
  use std::sync::Arc;

  #[test]
  fn column_for_byte_index_clamps_to_last_cell() {
    let offsets = [0, 1, 2, 3, 4];

    assert_eq!(column_for_byte_index(&offsets, 0), 0);
    assert_eq!(column_for_byte_index(&offsets, 2), 2);
    assert_eq!(column_for_byte_index(&offsets, 4), 3);
    assert_eq!(column_for_byte_index(&offsets, 99), 3);
  }

  #[test]
  fn column_for_byte_index_handles_multibyte_cells() {
    let offsets = [0, 1, 4, 5];

    assert_eq!(column_for_byte_index(&offsets, 0), 0);
    assert_eq!(column_for_byte_index(&offsets, 1), 1);
    assert_eq!(column_for_byte_index(&offsets, 3), 1);
    assert_eq!(column_for_byte_index(&offsets, 4), 2);
  }

  #[test]
  fn rendered_cells_preserve_combining_marks_and_wide_spacing() {
    let combined = TerminalCellSnapshot {
      row: 0,
      col: 0,
      c: 'e',
      zerowidth: Arc::from(['\u{301}']),
      fg: Color::Named(NamedColor::Foreground),
      bg: Color::Named(NamedColor::Background),
      flags: Flags::empty(),
      underline_color: None,
      hyperlink_uri: None,
    };
    let mut text = String::new();
    let mut state = RenderCellState::default();

    append_rendered_cell(&mut text, Some(&combined), &mut state);

    assert_eq!(text, "e\u{301}");
  }

  #[test]
  fn rendered_cells_shape_emoji_zwj_sequences_without_losing_grid_width() {
    let base = TerminalCellSnapshot {
      row: 0,
      col: 0,
      c: '👩',
      zerowidth: Arc::from(['\u{200d}']),
      fg: Color::Named(NamedColor::Foreground),
      bg: Color::Named(NamedColor::Background),
      flags: Flags::WIDE_CHAR,
      underline_color: None,
      hyperlink_uri: None,
    };
    let mut spacer = base.clone();
    spacer.col = 1;
    spacer.c = ' ';
    spacer.zerowidth = Arc::default();
    spacer.flags = Flags::WIDE_CHAR_SPACER;
    let mut laptop = base.clone();
    laptop.col = 2;
    laptop.c = '💻';
    laptop.zerowidth = Arc::default();
    let mut trailing_spacer = spacer.clone();
    trailing_spacer.col = 3;
    let mut ascii = base.clone();
    ascii.col = 4;
    ascii.c = 'x';
    ascii.zerowidth = Arc::default();
    ascii.flags = Flags::empty();
    let mut text = String::new();
    let mut state = RenderCellState::default();

    for cell in [&base, &spacer, &laptop, &trailing_spacer, &ascii] {
      append_rendered_cell(&mut text, Some(cell), &mut state);
    }

    assert_eq!(text, "👩\u{200d}💻  x");
  }

  #[test]
  fn style_for_cell_maps_font_and_decoration_flags() {
    let palette = TerminalPalette::default();
    let colors = Colors::default();
    let cell = TerminalCellSnapshot {
      row: 0,
      col: 0,
      c: 'x',
      zerowidth: Arc::default(),
      fg: Color::Named(NamedColor::Green),
      bg: Color::Named(NamedColor::Background),
      flags: Flags::BOLD | Flags::ITALIC | Flags::UNDERCURL | Flags::STRIKEOUT,
      underline_color: None,
      hyperlink_uri: None,
    };

    let style = style_for_cell(Some(&cell), &palette, &colors);

    assert!(style.bold);
    assert!(style.italic);
    assert_eq!(style.background, None);

    let foreground = palette.resolve(cell.fg, &colors);
    assert_eq!(style.foreground, foreground);
    assert_eq!(
      style.underline.map(|underline| underline.color),
      Some(Some(foreground))
    );
    assert_eq!(style.underline.map(|underline| underline.wavy), Some(true));
    assert_eq!(
      style.strikethrough.map(|strike| strike.color),
      Some(Some(foreground))
    );
  }

  #[test]
  fn style_for_cell_maps_all_underline_variants() {
    let palette = TerminalPalette::default();
    let colors = Colors::default();
    let cell = TerminalCellSnapshot {
      row: 0,
      col: 0,
      c: 'x',
      zerowidth: Arc::default(),
      fg: Color::Named(NamedColor::Blue),
      bg: Color::Named(NamedColor::Black),
      flags: Flags::DOUBLE_UNDERLINE | Flags::DASHED_UNDERLINE,
      underline_color: None,
      hyperlink_uri: None,
    };

    let style = style_for_cell(Some(&cell), &palette, &colors);

    assert_eq!(style.underline.map(|underline| underline.wavy), Some(false));
    assert_eq!(style.background, Some(palette.resolve(cell.bg, &colors)));
  }

  #[test]
  fn style_for_cell_uses_explicit_underline_color_for_hyperlinks() {
    let palette = TerminalPalette::default();
    let colors = Colors::default();
    let underline_color = Color::Spec(Rgb {
      r: 255,
      g: 0,
      b: 255,
    });
    let cell = TerminalCellSnapshot {
      row: 0,
      col: 0,
      c: 'x',
      zerowidth: Arc::default(),
      fg: Color::Named(NamedColor::Foreground),
      bg: Color::Named(NamedColor::Background),
      flags: Flags::empty(),
      underline_color: Some(underline_color),
      hyperlink_uri: Some(Arc::<str>::from("https://example.com")),
    };

    let style = style_for_cell(Some(&cell), &palette, &colors);

    assert_eq!(
      style.underline.map(|underline| underline.color),
      Some(Some(palette.resolve(underline_color, &colors)))
    );
    assert_eq!(style.underline.map(|underline| underline.wavy), Some(false));
  }
}
