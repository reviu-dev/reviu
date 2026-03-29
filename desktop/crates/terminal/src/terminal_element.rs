use gpui::{
  App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, Entity, FontStyle, FontWeight,
  GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
  MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent,
  ShapedLine, StrikethroughStyle, Style, TextAlign, TextRun, UnderlineStyle, Window, fill, point,
  px, relative,
};
use std::sync::Arc;

use alacritty_terminal::{
  term::cell::Flags,
  vte::ansi::{Color, NamedColor},
};

use crate::{
  ScreenSnapshot, TerminalCellSnapshot, ViewportPoint, colors::TerminalPalette,
  terminal_view::TerminalView,
};

#[derive(Clone)]
struct RowLayout {
  row: usize,
  byte_offsets: Arc<[usize]>,
  shaped: ShapedLine,
}

pub(crate) struct TerminalPrepaintState {
  hitbox: Hitbox,
  screen: ScreenSnapshot,
  row_layouts: Arc<[RowLayout]>,
  line_height: Pixels,
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
    let screen = self.view.read(cx).screen().clone();
    let line_height = line_height_for_screen(bounds, &screen);
    let row_layouts = build_row_layouts(&screen, &self.palette, window);

    TerminalPrepaintState {
      hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
      screen,
      row_layouts: row_layouts.into(),
      line_height,
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
    window.paint_quad(fill(bounds, self.palette.background()));

    if prepaint.hitbox.is_hovered(window) {
      let cursor_style = viewport_point_for_position(
        window.mouse_position(),
        bounds,
        &prepaint.row_layouts,
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

    paint_cursor(
      window,
      &self.palette,
      bounds,
      &prepaint.screen,
      &prepaint.row_layouts,
      prepaint.line_height,
      self.is_focused,
    );

    window.on_mouse_event({
      let view = self.view.clone();
      let hitbox = prepaint.hitbox.clone();
      let row_layouts = Arc::clone(&prepaint.row_layouts);
      move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
          return;
        }

        let Some(point) = viewport_point_for_position(event.position, bounds, &row_layouts, true)
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
      move |event: &MouseMoveEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
          return;
        }

        let hovered = hitbox.is_hovered(window);
        if hovered {
          window.refresh();
        }
        let hover_point = if hovered {
          viewport_point_for_position(event.position, bounds, &row_layouts, false)
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
          viewport_point_for_position(event.position, bounds, &row_layouts, !hovered)
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
          viewport_point_for_position(event.position, bounds, &row_layouts, !hovered)
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

        let delta_lines = match event.delta {
          ScrollDelta::Pixels(point) => pixels_to_scroll_lines(-point.y, line_height),
          ScrollDelta::Lines(point) => (-point.y).round() as i32,
        };
        if delta_lines == 0 {
          return;
        }

        let Some(point) = viewport_point_for_position(event.position, bounds, &row_layouts, true)
        else {
          return;
        };

        view.update(cx, |view, cx| {
          view.handle_scroll(delta_lines, point, event.modifiers, cx);
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

      byte_offsets.push(0);

      for col in 0..screen.cols {
        let cell = cell_grid[row][col];
        let ch = rendered_char(cell);
        text.push(ch);
        byte_offsets.push(text.len());

        let style = style_for_cell(cell, palette, &screen.colors);
        let char_len = ch.len_utf8();
        if active_style == Some(style) {
          active_len += char_len;
        } else {
          if let Some(previous_style) = active_style.take() {
            runs.push(text_run_for_style(active_len, &font, previous_style));
          }
          active_style = Some(style);
          active_len = char_len;
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

fn rendered_char(cell: Option<&TerminalCellSnapshot>) -> char {
  let Some(cell) = cell else {
    return ' ';
  };
  if cell.flags.contains(Flags::HIDDEN) {
    ' '
  } else {
    cell.c
  }
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

fn line_height_for_screen(bounds: Bounds<Pixels>, screen: &ScreenSnapshot) -> Pixels {
  if screen.rows == 0 {
    bounds.size.height.max(px(1.0))
  } else {
    (bounds.size.height / screen.rows as f32).max(px(1.0))
  }
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

fn paint_cursor(
  window: &mut Window,
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
        cursor_color.opacity(0.35),
      ));
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

  let line_height = (bounds.size.height / row_layouts.len() as f32).max(px(1.0));
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

fn pixels_to_scroll_lines(pixel_delta: Pixels, line_height: Pixels) -> i32 {
  (pixel_delta / line_height).round().clamp(-10.0, 10.0) as i32
}

#[cfg(test)]
mod tests {
  use super::{column_for_byte_index, pixels_to_scroll_lines, style_for_cell};
  use crate::{TerminalCellSnapshot, colors::TerminalPalette};
  use alacritty_terminal::{
    term::{cell::Flags, color::Colors},
    vte::ansi::{Color, NamedColor, Rgb},
  };
  use gpui::px;
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
  fn pixels_to_scroll_lines_clamps_large_deltas() {
    assert_eq!(pixels_to_scroll_lines(px(60.0), px(20.0)), 3);
    assert_eq!(pixels_to_scroll_lines(px(-300.0), px(20.0)), -10);
  }

  #[test]
  fn style_for_cell_maps_font_and_decoration_flags() {
    let palette = TerminalPalette::default();
    let colors = Colors::default();
    let cell = TerminalCellSnapshot {
      row: 0,
      col: 0,
      c: 'x',
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
