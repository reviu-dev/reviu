use gpui::{
  App, Bounds, DispatchPhase, ElementId, ElementInputHandler, Entity, GlobalElementId, Hitbox,
  HitboxBehavior, InspectorElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
  MouseUpEvent, PaintQuad, Pixels, Point, ScrollDelta, ScrollWheelEvent, ShapedLine, Style,
  TextAlign, TextRun, TextStyle, Window, fill, point, prelude::*, px, relative, size,
};
use std::{
  ops::Range,
  rc::Rc,
  sync::Arc,
  time::{Duration, Instant},
};

use crate::{
  document::Document,
  editor::{DEFAULT_MAX_LINE_WIDTH, Editor, ScrollAxis},
};
use syntax::{HighlightSpan, Theme};

// Visual width for empty line selection indicator
const NEWLINE_SELECTION_WIDTH: f32 = 4.0;
// Scroll sensitivity for pixel-based scrolling (trackpad)
const PIXEL_SCROLL_DIVISOR: f32 = 20.0;
// Scroll sensitivity for line-based scrolling (mouse wheel)
const LINE_SCROLL_MULTIPLIER: f32 = 3.0;
const SCROLL_AXIS_RATIO: f32 = 1.1;
const SCROLL_AXIS_SWITCH_RATIO: f32 = 1.4;
const SCROLL_AXIS_TIMEOUT_MS: u64 = 150;

/// Encapsulates layout information for mouse position -> text offset conversion
#[derive(Clone)]
pub struct PositionMap {
  pub shaped_lines: Vec<(usize, Arc<ShapedLine>)>,
  pub bounds: Bounds<Pixels>,
  pub line_height: Pixels,
  pub viewport: Range<usize>,
}

impl PositionMap {
  pub fn point_for_position(&self, position: Point<Pixels>, document: &Document) -> Option<usize> {
    if !self.bounds.contains(&position) {
      return None;
    }

    if document.is_empty() {
      return Some(0);
    }

    let y_offset = position.y - self.bounds.top();
    let row_in_viewport = (y_offset / self.line_height).floor() as usize;
    let actual_row = self.viewport.start + row_in_viewport;

    if actual_row >= document.len_lines() {
      return Some(document.len());
    }

    let shaped = self
      .shaped_lines
      .iter()
      .find(|(idx, _)| *idx == actual_row)
      .map(|(_, s)| s)?;

    let x_offset = position.x - self.bounds.left();
    let column = shaped.closest_index_for_x(x_offset);

    let line_start = document.line_to_char(actual_row);
    Some(line_start + column)
  }
}

/// Helper to convert syntax highlights to TextRuns for rendering
pub(crate) fn highlights_to_text_runs(
  highlights: &[HighlightSpan],
  line_text: &str,
  theme: &Theme,
  base_style: &TextStyle,
) -> Vec<TextRun> {
  let mut runs = Vec::new();
  let line_len = line_text.len();
  let mut current_pos = 0;

  // Filter and clip highlights for this line
  let mut line_highlights: Vec<_> = highlights
    .iter()
    .filter_map(|h| {
      let start = h.byte_range.start.min(line_len);
      let end = h.byte_range.end.min(line_len);
      (end > start).then_some((start..end, h.token_type))
    })
    .collect();

  line_highlights.sort_by_key(|(range, _)| range.start);

  for (range, token_type) in line_highlights {
    // Gap before highlight (normal text)
    if range.start > current_pos {
      runs.push(TextRun {
        len: range.start - current_pos,
        font: base_style.font(),
        color: base_style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
      });
    }

    // The highlighted span
    runs.push(TextRun {
      len: range.len(),
      font: base_style.font(),
      color: theme.syntax().color_for_token(token_type),
      background_color: None,
      underline: None,
      strikethrough: None,
    });

    current_pos = range.end;
  }

  // Final gap
  if current_pos < line_len {
    runs.push(TextRun {
      len: line_len - current_pos,
      font: base_style.font(),
      color: base_style.color,
      background_color: None,
      underline: None,
      strikethrough: None,
    });
  }

  runs
}

pub struct EditorElement {
  editor: Entity<Editor>,
}

pub struct PrepaintState {
  shaped_lines: Vec<(usize, Arc<ShapedLine>)>,
  cursor_quad: Option<PaintQuad>,
  diff_background_quads: Vec<PaintQuad>,
  diff_word_quads: Vec<PaintQuad>,
  selection_quads: Vec<PaintQuad>,
  viewport: Range<usize>,
  bounds: Bounds<Pixels>,
  line_height: Pixels,
  scroll_hitbox: Hitbox,
}

impl EditorElement {
  pub fn new(editor: Entity<Editor>) -> Self {
    Self { editor }
  }

  fn calculate_viewport(
    &self,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    scroll_offset: f32,
    total_lines: usize,
  ) -> Range<usize> {
    let visible_line_count = ((bounds.size.height / line_height).ceil() as usize).max(1);

    let start_line = (scroll_offset.floor() as usize).min(total_lines.saturating_sub(1));
    let end_line = (start_line + visible_line_count).min(total_lines);

    start_line..end_line
  }
}

impl IntoElement for EditorElement {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for EditorElement {
  type RequestLayoutState = ();
  type PrepaintState = PrepaintState;

  fn id(&self) -> Option<ElementId> {
    None
  }

  fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
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
    let scroll_hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

    // Check if syntax highlights or diff state have been updated and invalidate cache if needed
    let (
      highlights_epoch,
      highlights_version,
      dirty_highlight_lines,
      diff_epoch,
      diff_version,
      dirty_diff_lines,
    ) = {
      let document = self.editor.read(cx).document().read(cx);
      let epoch = *document.highlights_epoch.read();
      let version = *document.highlights_version.read();
      let dirty = document.drain_dirty_highlight_lines();
      let diff_epoch = *document.diff_epoch.read();
      let diff_version = *document.diff_version.read();
      let diff_dirty = document.drain_dirty_diff_lines();
      (epoch, version, dirty, diff_epoch, diff_version, diff_dirty)
    };
    self.editor.update(cx, |editor, cx| {
      editor.viewport_height = bounds.size.height;
      editor.viewport_width = window.bounds().size.width;

      if editor.scroll_axis_lock == Some(ScrollAxis::Vertical)
        && editor.scroll_handle.offset().x != editor.last_scroll_x
      {
        editor
          .scroll_handle
          .set_offset(point(editor.last_scroll_x, px(0.0)));
      }

      if diff_epoch > editor.last_diff_epoch || diff_version > editor.last_diff_version {
        editor.sync_view_selection_from_buffer(cx);
      }

      if diff_epoch > editor.last_diff_epoch {
        editor.line_layouts.clear();
        editor.last_diff_epoch = diff_epoch;
        editor.last_diff_version = diff_version;
      } else if diff_version > editor.last_diff_version {
        for line_idx in &dirty_diff_lines {
          editor.line_layouts.remove(line_idx);
        }
        editor.last_diff_version = diff_version;
      }

      if highlights_epoch > editor.last_highlights_epoch {
        editor.line_layouts.clear();
        editor.last_highlights_epoch = highlights_epoch;
        editor.last_highlights_version = highlights_version;
      } else if highlights_version > editor.last_highlights_version {
        for line_idx in &dirty_highlight_lines {
          editor.line_layouts.remove(line_idx);
        }
        editor.last_highlights_version = highlights_version;
      }
    });

    let (viewport, selected_range, cursor_offset, mut shaped_lines, lines_to_shape) = {
      let editor = self.editor.read(cx);
      let document = editor.document().read(cx);
      let line_height = window.line_height();
      let scroll_offset = editor.scroll_offset_y;

      let viewport =
        self.calculate_viewport(bounds, line_height, scroll_offset, document.len_lines());

      let mut lines_to_shape = Vec::new();
      let mut shaped_lines = Vec::new();

      for line_idx in viewport.clone() {
        if line_idx >= document.len_lines() {
          break;
        }
        match editor.line_layouts.get(&line_idx) {
          Some(shaped) => {
            // Arc::clone is cheap - just incrementing reference count
            shaped_lines.push((line_idx, Arc::clone(shaped)));
          }
          None => {
            let line_content = document
              .line_content(line_idx)
              .map(|cow| cow.into_owned())
              .unwrap_or_default();
            lines_to_shape.push((line_idx, line_content));
          }
        }
      }

      (
        viewport,
        editor.selected_range.clone(),
        editor.cursor_offset(),
        shaped_lines,
        lines_to_shape,
      )
    };

    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let line_height = window.line_height();

    // Get theme for syntax highlighting and diff colors
    let theme = self.editor.read(cx).theme.clone();

    let mut diff_background_quads = Vec::new();
    {
      let document = self.editor.read(cx).document().read(cx);
      for line_idx in viewport.clone() {
        let Some(info) = document.diff_line_info(line_idx) else {
          continue;
        };
        let color = match info.kind {
          crate::document::DiffLineKind::Added => Some(theme.diff_added_background()),
          crate::document::DiffLineKind::Deleted => Some(theme.diff_removed_background()),
          _ => None,
        };
        if let Some(color) = color {
          let y = bounds.top() + line_height * (line_idx - viewport.start) as f32;
          diff_background_quads.push(fill(
            Bounds::from_corners(
              point(bounds.left(), y),
              point(bounds.right(), y + line_height),
            ),
            color,
          ));
        }
      }
    }

    let mut newly_shaped = Vec::new();
    for (line_idx, line_content) in lines_to_shape {
      // Try to get syntax highlights for this line
      let document = self.editor.read(cx).document().read(cx);
      let runs = if let Some(highlights) = document.get_highlights_for_line(line_idx) {
        // Render with syntax highlighting colors
        highlights_to_text_runs(highlights.as_ref(), &line_content, &theme, &style)
      } else {
        // Fallback: plain text rendering (progressive rendering!)
        vec![TextRun {
          len: line_content.len(),
          font: style.font(),
          color: style.color,
          background_color: None,
          underline: None,
          strikethrough: None,
        }]
      };

      let shaped = window
        .text_system()
        .shape_line(line_content.into(), font_size, &runs, None);
      newly_shaped.push((line_idx, shaped));
    }

    if !newly_shaped.is_empty() {
      self.editor.update(cx, |editor, _| {
        for (line_idx, shaped) in newly_shaped {
          // Wrap in Arc for cheap cloning
          let shaped_arc = Arc::new(shaped);
          editor.line_layouts.insert(line_idx, shaped_arc.clone());
          shaped_lines.push((line_idx, shaped_arc));
        }
        // Limit cache size to prevent memory issues with large files
        editor.ensure_cache_size(viewport.clone());
      });
    }

    // Calculate maximum line width for horizontal scrolling
    let max_width = shaped_lines
      .iter()
      .map(|(_, shaped)| shaped.width)
      .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
      .unwrap_or(px(DEFAULT_MAX_LINE_WIDTH));

    self.editor.update(cx, |editor, _| {
      editor.max_line_width = editor.max_line_width.max(max_width);
    });

    let document = self.editor.read(cx).document().read(cx);

    let cursor_line = document.char_to_line(cursor_offset);
    let cursor_quad = if viewport.contains(&cursor_line) {
      let shaped_opt = shaped_lines
        .iter()
        .find(|(idx, _)| *idx == cursor_line)
        .map(|(_, shaped)| shaped);
      if let Some(shaped) = shaped_opt {
        let line_start = document.line_to_char(cursor_line);
        let cursor_in_line = cursor_offset - line_start;
        let cursor_x = shaped.x_for_index(cursor_in_line);
        let y = bounds.top() + line_height * (cursor_line - viewport.start) as f32;
        Some(fill(
          Bounds::new(
            point(bounds.left() + cursor_x, y),
            size(px(2.), line_height),
          ),
          theme.cursor(),
        ))
      } else {
        None
      }
    } else {
      None
    };

    let mut diff_word_quads = Vec::new();
    for (line_idx, shaped_line) in &shaped_lines {
      let Some(ranges) = document.diff_word_ranges(*line_idx) else {
        continue;
      };
      let Some(info) = document.diff_line_info(*line_idx) else {
        continue;
      };
      let color = match info.kind {
        crate::document::DiffLineKind::Added => theme.diff_added_word_background(),
        crate::document::DiffLineKind::Deleted => theme.diff_removed_word_background(),
        _ => continue,
      };
      let line_len = document
        .line_range(*line_idx)
        .map(|range| range.end - range.start)
        .unwrap_or(0);
      let y = bounds.top() + line_height * (*line_idx - viewport.start) as f32;
      for range in ranges.iter() {
        let start = range.start.min(line_len);
        let end = range.end.min(line_len);
        if end <= start {
          continue;
        }
        let x_start = shaped_line.x_for_index(start);
        let x_end = shaped_line.x_for_index(end);
        if x_end <= x_start {
          continue;
        }
        diff_word_quads.push(fill(
          Bounds::from_corners(
            point(bounds.left() + x_start, y),
            point(bounds.left() + x_end, y + line_height),
          ),
          color,
        ));
      }
    }

    let mut selection_quads = Vec::new();
    if !selected_range.is_empty() {
      let sel_start = selected_range.start;
      let sel_end = selected_range.end;
      let sel_start_line = document.char_to_line(sel_start);
      let sel_end_line = document.char_to_line(sel_end);

      for line_idx in sel_start_line..=sel_end_line {
        if !viewport.contains(&line_idx) {
          continue;
        }
        let line_range = document.line_range(line_idx).unwrap();
        let shaped_opt = shaped_lines
          .iter()
          .find(|(idx, _)| *idx == line_idx)
          .map(|(_, shaped)| shaped);

        if let Some(shaped) = shaped_opt {
          let line_start = line_range.start;
          let line_end = line_range.end;
          let sel_line_start = sel_start.max(line_start) - line_start;
          let sel_line_end = sel_end.min(line_end) - line_start;
          let x_start = shaped.x_for_index(sel_line_start);
          let x_end = shaped.x_for_index(sel_line_end);
          let y = bounds.top() + line_height * (line_idx - viewport.start) as f32;

          // If selection is empty on this line (selecting just the newline),
          // Only add width if we're actually selecting the newline character
          let is_selecting_newline = sel_line_end > sel_line_start && x_start == x_end;
          let visual_x_end = if is_selecting_newline {
            x_end + px(NEWLINE_SELECTION_WIDTH) // Small width to show newline selection
          } else {
            x_end
          };

          selection_quads.push(fill(
            Bounds::from_corners(
              point(bounds.left() + x_start, y),
              point(bounds.left() + visual_x_end, y + line_height),
            ),
            theme.selection(),
          ));
        }
      }
    }

    PrepaintState {
      shaped_lines,
      cursor_quad,
      diff_background_quads,
      diff_word_quads,
      selection_quads,
      viewport,
      bounds,
      line_height,
      scroll_hitbox,
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
    let (focus_handle, is_focused) = {
      let editor = self.editor.read(cx);
      (
        editor.focus_handle.clone(),
        editor.focus_handle.is_focused(window),
      )
    };

    window.handle_input(
      &focus_handle,
      ElementInputHandler::new(bounds, self.editor.clone()),
      cx,
    );

    // Use Rc to avoid cloning PositionMap in closures
    let position_map = Rc::new(PositionMap {
      shaped_lines: prepaint.shaped_lines.clone(),
      bounds: prepaint.bounds,
      line_height: prepaint.line_height,
      viewport: prepaint.viewport.clone(),
    });

    window.on_mouse_event({
      let editor = self.editor.clone();
      let position_map = Rc::clone(&position_map);
      move |event: &MouseDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
          editor.update(cx, |editor, cx| {
            editor.mouse_left_down(event, &position_map, window, cx);
          });
        }
      }
    });

    window.on_mouse_event({
      let editor = self.editor.clone();
      move |event: &MouseUpEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
          editor.update(cx, |editor, cx| {
            editor.mouse_left_up(event, window, cx);
          });
        }
      }
    });

    window.on_mouse_event({
      let editor = self.editor.clone();
      let position_map = Rc::clone(&position_map);
      move |event: &MouseMoveEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble {
          let is_selecting = editor.read(cx).is_selecting;
          if is_selecting {
            editor.update(cx, |editor, cx| {
              editor.mouse_dragged(event, &position_map, window, cx);
            });
          }
        }
      }
    });

    // Handle mouse wheel scroll
    window.on_mouse_event({
      let editor = self.editor.clone();
      let scroll_hitbox = prepaint.scroll_hitbox.clone();
      move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && scroll_hitbox.should_handle_scroll(window) {
          editor.update(cx, |editor, cx| {
            let document = editor.document().read(cx);
            let total_lines = document.len_lines();
            let now = Instant::now();
            let reset_lock = editor
              .last_scroll_time
              .map(|last| now.duration_since(last) > Duration::from_millis(SCROLL_AXIS_TIMEOUT_MS))
              .unwrap_or(true);
            if reset_lock {
              editor.scroll_axis_lock = None;
              editor.last_scroll_x = editor.scroll_handle.offset().x;
            }
            editor.last_scroll_time = Some(now);

            // Extract scroll delta (handle both pixel and line scrolling)
            // Note: Negative delta because scrolling down should increase scroll_offset
            let pixel_delta = event.delta.pixel_delta(window.line_height());
            let delta_x_px = pixel_delta.x;
            let delta_y_px = -pixel_delta.y;
            let delta_y = match event.delta {
              ScrollDelta::Pixels(point) => -(point.y / px(PIXEL_SCROLL_DIVISOR)),
              ScrollDelta::Lines(point) => -(point.y * LINE_SCROLL_MULTIPLIER),
            };
            let abs_x = delta_x_px.abs();
            let abs_y = delta_y_px.abs();
            let axis = match editor.scroll_axis_lock {
              None => {
                let axis = if abs_x > abs_y * SCROLL_AXIS_RATIO {
                  ScrollAxis::Horizontal
                } else {
                  ScrollAxis::Vertical
                };
                if axis == ScrollAxis::Horizontal {
                  editor.last_scroll_x = editor.scroll_handle.offset().x;
                }
                editor.scroll_axis_lock = Some(axis);
                axis
              }
              Some(axis) => {
                if axis == ScrollAxis::Vertical && abs_x > abs_y * SCROLL_AXIS_SWITCH_RATIO {
                  editor.last_scroll_x = editor.scroll_handle.offset().x;
                  editor.scroll_axis_lock = Some(ScrollAxis::Horizontal);
                  ScrollAxis::Horizontal
                } else if axis == ScrollAxis::Horizontal && abs_y > abs_x * SCROLL_AXIS_SWITCH_RATIO
                {
                  editor.scroll_axis_lock = Some(ScrollAxis::Vertical);
                  ScrollAxis::Vertical
                } else {
                  axis
                }
              }
            };

            if axis == ScrollAxis::Horizontal {
              let new_scroll_x = editor.scroll_handle.offset().x + delta_x_px;
              editor
                .scroll_handle
                .set_offset(point(new_scroll_x, px(0.0)));
              editor.last_scroll_x = new_scroll_x;
              cx.notify();
              return;
            }

            let new_scroll = (editor.scroll_offset_y + delta_y)
              .max(0.0)
              .min((total_lines.saturating_sub(1)) as f32);

            editor.scroll_offset_y = new_scroll;
            if editor.scroll_handle.offset().x != editor.last_scroll_x {
              editor
                .scroll_handle
                .set_offset(point(editor.last_scroll_x, px(0.0)));
            }
            let viewport = editor.viewport_range(window.line_height(), total_lines);
            editor.document.update(cx, |doc, cx| {
              doc.schedule_viewport_highlights(
                viewport.clone(),
                None,
                crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
                cx,
              );
              if doc.diff_enabled() {
                doc.schedule_viewport_diff(
                  viewport,
                  crate::document::VIEWPORT_DIFF_MARGIN_LINES,
                  cx,
                );
              }
            });
            cx.notify();
          });
          cx.stop_propagation();
        }
      }
    });

    // Paint diff backgrounds
    for quad in &prepaint.diff_background_quads {
      window.paint_quad(quad.clone());
    }

    // Paint diff word highlights
    for quad in &prepaint.diff_word_quads {
      window.paint_quad(quad.clone());
    }

    // Paint selection
    for quad in &prepaint.selection_quads {
      window.paint_quad(quad.clone());
    }

    // Paint text lines
    for (line_idx, shaped_line) in &prepaint.shaped_lines {
      let y = bounds.top() + prepaint.line_height * (*line_idx - prepaint.viewport.start) as f32;
      shaped_line
        .paint(
          point(bounds.left(), y),
          prepaint.line_height,
          TextAlign::Left,
          None,
          window,
          cx,
        )
        .ok();
    }

    // Paint cursor (if focused and visible from blink)
    let cursor_visible = self.editor.read(cx).cursor_blink.read(cx).visible();
    if is_focused
      && cursor_visible
      && let Some(cursor_quad) = &prepaint.cursor_quad
    {
      window.paint_quad(cursor_quad.clone());
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::{TestAppContext, px, size};

  // Helper to create test bounds
  fn test_bounds(width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(Point::default(), size(px(width), px(height)))
  }

  // ============================================================================
  // Viewport Calculation Tests
  // ============================================================================

  #[gpui::test]
  fn test_calculate_viewport_simple(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, cx));
    let element = EditorElement::new(editor);

    // 400px height, 20px line height = 20 visible lines
    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 0..20);
  }

  #[gpui::test]
  fn test_calculate_viewport_with_scroll(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, cx));
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 10.0; // Scrolled down 10 lines
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 10..30);
  }

  #[gpui::test]
  fn test_calculate_viewport_at_end(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, cx));
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 90.0; // Near end
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Should clamp to total_lines
    assert_eq!(viewport, 90..100);
  }

  #[gpui::test]
  fn test_calculate_viewport_short_document(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, cx));
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 5; // Document shorter than viewport

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 0..5);
  }

  #[gpui::test]
  fn test_calculate_viewport_fractional_scroll(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, cx));
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 5.5; // Fractional scroll
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Should floor scroll_offset
    assert_eq!(viewport, 5..25);
  }

  #[gpui::test]
  fn test_calculate_viewport_scroll_past_end(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, cx));
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 150.0; // Way past end
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Should clamp start_line to total_lines - 1
    assert_eq!(viewport, 99..100);
  }

  #[gpui::test]
  fn test_calculate_viewport_minimum_one_line(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, cx));
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 10.0); // Very small height
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Should show at least 1 line even if height is too small
    assert_eq!(viewport, 0..1);
  }

  #[gpui::test]
  fn test_calculate_viewport_large_line_height(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, cx));
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(40.0); // Large line height
    let scroll_offset = 0.0;
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // 400 / 40 = 10 visible lines
    assert_eq!(viewport, 0..10);
  }

  #[gpui::test]
  fn test_calculate_viewport_single_line_document(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, cx));
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 1;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 0..1);
  }

  #[gpui::test]
  fn test_calculate_viewport_empty_document(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, cx));
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 0;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Empty document edge case - start_line gets clamped to 0
    // This creates a 0..0 range which is valid but empty
    assert!(viewport.is_empty());
  }
}
