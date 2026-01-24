use gpui::{
  App, Bounds, ContentMask, DispatchPhase, ElementId, ElementInputHandler, Entity, GlobalElementId,
  Hitbox, HitboxBehavior, InspectorElementId, LayoutId, MouseButton, MouseDownEvent,
  MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ScrollDelta, ScrollWheelEvent,
  ShapedLine, Style, TextAlign, TextRun, TextStyle, Window, black, fill, pattern_slash, point,
  prelude::*, px, relative, size, white,
};
use std::{
  ops::Range,
  rc::Rc,
  sync::Arc,
  time::{Duration, Instant},
};

use crate::{
  document::{DiffPanelSide, Document},
  editor::{
    DEFAULT_MAX_LINE_WIDTH, DiffViewMode, Editor, ScrollAxis, STAGED_DIFF_OPACITY_MULTIPLIER,
  },
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
const SPLIT_DIVIDER_WIDTH: f32 = 1.0;
const STAGED_HUNK_BORDER_HEIGHT: f32 = 2.0;

/// Encapsulates layout information for mouse position -> text offset conversion
#[derive(Clone)]
pub struct PositionMap {
  pub shaped_lines: Vec<(usize, Arc<ShapedLine>)>,
  pub row_to_line: Vec<Option<usize>>,
  pub bounds: Bounds<Pixels>,
  pub line_height: Pixels,
  pub viewport: Range<usize>,
  pub scroll_x: Pixels,
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
    if row_in_viewport >= self.row_to_line.len() {
      return Some(document.len());
    }
    let line_idx = self.row_to_line.get(row_in_viewport).copied().flatten()?;

    let shaped = self
      .shaped_lines
      .iter()
      .find(|(idx, _)| *idx == line_idx)
      .map(|(_, s)| s)?;

    let x_offset = position.x - self.bounds.left() - self.scroll_x;
    let column = shaped.closest_index_for_x(x_offset);

    let line_start = document.line_to_char(line_idx);
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

fn split_bounds(
  bounds: Bounds<Pixels>,
  right_width: Pixels,
  gutter_width: Pixels,
) -> (Bounds<Pixels>, Bounds<Pixels>, Bounds<Pixels>) {
  let total_width = bounds.size.width;
  let right_width = right_width.min(total_width).max(px(0.0));
  let panel_left = (bounds.right() - right_width).max(bounds.left());

  let left = Bounds::from_corners(
    point(bounds.left(), bounds.top()),
    point(panel_left, bounds.bottom()),
  );

  let right_text_left = (panel_left + gutter_width).min(bounds.right());
  let right = Bounds::from_corners(
    point(right_text_left, bounds.top()),
    point(bounds.right(), bounds.bottom()),
  );

  let divider = Bounds::from_corners(
    point(panel_left, bounds.top()),
    point(panel_left + px(SPLIT_DIVIDER_WIDTH), bounds.bottom()),
  );

  (left, right, divider)
}

pub struct EditorElement {
  editor: Entity<Editor>,
}

pub struct PrepaintState {
  shaped_lines: Vec<(usize, Arc<ShapedLine>)>,
  shaped_lines_left: Vec<(usize, Arc<ShapedLine>)>,
  cursor_quad: Option<PaintQuad>,
  diff_background_quads_left: Vec<PaintQuad>,
  diff_background_quads_right: Vec<PaintQuad>,
  diff_border_quads_left: Vec<PaintQuad>,
  diff_border_quads_right: Vec<PaintQuad>,
  diff_hatch_quads_left: Vec<PaintQuad>,
  diff_hatch_quads_right: Vec<PaintQuad>,
  diff_word_quads_left: Vec<PaintQuad>,
  diff_word_quads_right: Vec<PaintQuad>,
  selection_quads: Vec<PaintQuad>,
  divider_quads: Vec<PaintQuad>,
  split_rows: Vec<crate::document::SplitDiffRow>,
  viewport: Range<usize>,
  bounds: Bounds<Pixels>,
  left_bounds: Bounds<Pixels>,
  right_bounds: Bounds<Pixels>,
  line_height: Pixels,
  scroll_hitbox: Hitbox,
  split_mode: bool,
  left_scroll_x: Pixels,
  right_scroll_x: Pixels,
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
      let editor = self.editor.read(cx);
      let document = editor.document().read(cx);
      let epoch = *document.highlights_epoch.read();
      let version = *document.highlights_version.read();
      let dirty = document.drain_dirty_highlight_lines();
      let diff_epoch = *document.diff_epoch.read();
      let diff_version = *document.diff_version.read();
      let diff_dirty = document.drain_dirty_diff_lines();
      (epoch, version, dirty, diff_epoch, diff_version, diff_dirty)
    };
    let diff_changed = {
      let editor = self.editor.read(cx);
      diff_epoch > editor.last_diff_epoch || diff_version > editor.last_diff_version
    };
    self.editor.update(cx, |editor, cx| {
      editor.viewport_height = bounds.size.height;
      editor.viewport_width = bounds.size.width + px(crate::editor::GUTTER_WIDTH);

      if diff_epoch > editor.last_diff_epoch || diff_version > editor.last_diff_version {
        editor.sync_view_selection_from_buffer(cx);
      }

      if diff_epoch > editor.last_diff_epoch {
        editor.line_layouts.clear();
        editor.line_layouts_left.clear();
        editor.last_diff_epoch = diff_epoch;
        editor.last_diff_version = diff_version;
      } else if diff_version > editor.last_diff_version {
        for line_idx in &dirty_diff_lines {
          editor.line_layouts.remove(line_idx);
          editor.line_layouts_left.remove(line_idx);
        }
        editor.last_diff_version = diff_version;
      }

      if highlights_epoch > editor.last_highlights_epoch {
        editor.line_layouts.clear();
        editor.line_layouts_left.clear();
        editor.last_highlights_epoch = highlights_epoch;
        editor.last_highlights_version = highlights_version;
      } else if highlights_version > editor.last_highlights_version {
        for line_idx in &dirty_highlight_lines {
          editor.line_layouts.remove(line_idx);
          editor.line_layouts_left.remove(line_idx);
        }
        editor.last_highlights_version = highlights_version;
      }
    });

    let (split_mode, left_bounds, right_bounds, divider_bounds) = {
      let editor = self.editor.read(cx);
      let document = editor.document().read(cx);
      let split_mode = editor.diff_view_mode == DiffViewMode::Split && document.diff_enabled();
      if split_mode {
        let max_width = bounds.size.width.max(px(1.0));
        let min_width = px(crate::editor::SPLIT_RIGHT_MIN_WIDTH).min(max_width);
        let max_right_width = (max_width - px(crate::editor::SPLIT_LEFT_MIN_WIDTH)).max(min_width);
        let right_width = editor.split_right_width.max(min_width).min(max_right_width);
        let gutter_width =
          px(crate::editor::GUTTER_WIDTH + crate::editor::SPLIT_RESIZE_HANDLE_WIDTH);
        let (left, right, divider) = split_bounds(bounds, right_width, gutter_width);
        (true, left, right, Some(divider))
      } else {
        (false, bounds, bounds, None)
      }
    };
    let (left_scroll_x, right_scroll_x) = {
      let editor = self.editor.read(cx);
      (editor.scroll_offset_x_left, editor.scroll_offset_x_right)
    };
    let active_panel = {
      let editor = self.editor.read(cx);
      if split_mode {
        editor.active_diff_panel
      } else {
        DiffPanelSide::Right
      }
    };

    let (
      viewport,
      selected_range,
      cursor_offset,
      mut shaped_lines,
      mut shaped_lines_left,
      lines_to_shape,
      lines_to_shape_left,
      split_rows,
    ) = {
      let editor = self.editor.read(cx);
      let document = editor.document().read(cx);
      let line_height = window.line_height();
      let scroll_offset = editor.scroll_offset_y;

      let total_rows = if split_mode {
        document.split_row_count()
      } else {
        document.len_lines()
      };
      let viewport = self.calculate_viewport(bounds, line_height, scroll_offset, total_rows);

      let mut lines_to_shape = Vec::new();
      let mut lines_to_shape_left = Vec::new();
      let mut shaped_lines = Vec::new();
      let mut shaped_lines_left = Vec::new();
      let mut split_rows = Vec::new();

      if split_mode {
        for row_idx in viewport.clone() {
          let row = document
            .split_row(row_idx)
            .unwrap_or(crate::document::SplitDiffRow {
              left_line: None,
              right_line: None,
            });
          split_rows.push(row);

          if let Some(line_idx) = row.left_line {
            match editor.line_layouts_left.get(&line_idx) {
              Some(shaped) => {
                shaped_lines_left.push((line_idx, Arc::clone(shaped)));
              }
              None => {
                let line_content = document
                  .line_content(line_idx)
                  .map(|cow| cow.into_owned())
                  .unwrap_or_default();
                lines_to_shape_left.push((line_idx, line_content));
              }
            }
          }

          if let Some(line_idx) = row.right_line {
            match editor.line_layouts.get(&line_idx) {
              Some(shaped) => {
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
        }
      } else {
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
      }

      (
        viewport,
        editor.selected_range.clone(),
        editor.cursor_offset(),
        shaped_lines,
        shaped_lines_left,
        lines_to_shape,
        lines_to_shape_left,
        split_rows,
      )
    };
    let viewport_line_range = {
      let document = self.editor.read(cx).document().read(cx);
      if split_mode {
        document
          .split_row_range_to_line_range(viewport.clone())
          .or_else(|| Some(viewport.clone()))
      } else {
        Some(viewport.clone())
      }
    };
    if diff_changed {
      if let Some(viewport) = viewport_line_range.clone() {
        self.editor.update(cx, |editor, cx| {
          editor.document.update(cx, |doc, cx| {
            if doc.diff_enabled() {
              doc.schedule_viewport_highlights(
                viewport,
                None,
                crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
                cx,
              );
            }
          });
        });
      }
    }

    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let line_height = window.line_height();

    // Get theme for syntax highlighting and diff colors
    let theme = self.editor.read(cx).theme.clone();
    let added_bg = theme.diff_added_background();
    let removed_bg = theme.diff_removed_background();
    let mut added_bg_staged = added_bg;
    let mut removed_bg_staged = removed_bg;
    added_bg_staged.a = (added_bg_staged.a * STAGED_DIFF_OPACITY_MULTIPLIER).min(1.0);
    removed_bg_staged.a = (removed_bg_staged.a * STAGED_DIFF_OPACITY_MULTIPLIER).min(1.0);
    let mut left_border_color = theme.diff_removed_word_background();
    left_border_color.a = 1.0;
    let mut right_border_color = theme.diff_added_word_background();
    right_border_color.a = 1.0;
    let top_border_color = left_border_color;
    let bottom_border_color = right_border_color;
    let border_height = px(STAGED_HUNK_BORDER_HEIGHT);

    let mut diff_background_quads_left = Vec::new();
    let mut diff_background_quads_right = Vec::new();
    let mut diff_border_quads_left = Vec::new();
    let mut diff_border_quads_right = Vec::new();
    let mut diff_hatch_quads_left = Vec::new();
    let mut diff_hatch_quads_right = Vec::new();
    let mut divider_quads = Vec::new();
    {
      let editor = self.editor.read(cx);
      let document = editor.document().read(cx);
      if let Some(divider_bounds) = divider_bounds {
        divider_quads.push(fill(divider_bounds, theme.line_number()));
      }
      if split_mode {
        let hatch_color = theme.line_number().opacity(0.35);
        let line_height_f32: f32 = line_height.into();
        let hatch_width = (line_height_f32 / 5.5).max(2.0);
        let hatch_interval = hatch_width * 6.0;
        let mut left_hatch_start: Option<usize> = None;
        let mut right_hatch_start: Option<usize> = None;
        for (row_offset, row) in split_rows.iter().enumerate() {
          let y_left = left_bounds.top() + line_height * row_offset as f32;
          if let Some(line_idx) = row.left_line {
            if let Some(info) = document.diff_line_info(line_idx)
              && matches!(info.kind, crate::document::DiffLineKind::Deleted)
            {
              let color = if editor.diff_line_is_staged(line_idx, cx) {
                removed_bg_staged
              } else {
                removed_bg
              };
              diff_background_quads_left.push(fill(
                Bounds::from_corners(
                  point(left_bounds.left(), y_left),
                  point(left_bounds.right(), y_left + line_height),
                ),
                color,
              ));
            }
          }
          let y_right = right_bounds.top() + line_height * row_offset as f32;
          if let Some(line_idx) = row.right_line {
            if let Some(info) = document.diff_line_info(line_idx)
              && matches!(info.kind, crate::document::DiffLineKind::Added)
            {
              let color = if editor.diff_line_is_staged(line_idx, cx) {
                added_bg_staged
              } else {
                added_bg
              };
              diff_background_quads_right.push(fill(
                Bounds::from_corners(
                  point(right_bounds.left(), y_right),
                  point(right_bounds.right(), y_right + line_height),
                ),
                color,
              ));
            }
          }
          let left_hatch = row.left_line.is_none() && row.right_line.is_some();
          if left_hatch {
            if left_hatch_start.is_none() {
              left_hatch_start = Some(row_offset);
            }
          } else if let Some(start) = left_hatch_start.take() {
            let start_y = left_bounds.top() + line_height * start as f32;
            diff_hatch_quads_left.push(fill(
              Bounds::from_corners(
                point(left_bounds.left(), start_y),
                point(left_bounds.right(), y_left),
              ),
              pattern_slash(hatch_color, hatch_width, hatch_interval),
            ));
          }

          let right_hatch = row.right_line.is_none() && row.left_line.is_some();
          if right_hatch {
            if right_hatch_start.is_none() {
              right_hatch_start = Some(row_offset);
            }
          } else if let Some(start) = right_hatch_start.take() {
            let start_y = right_bounds.top() + line_height * start as f32;
            diff_hatch_quads_right.push(fill(
              Bounds::from_corners(
                point(right_bounds.left(), start_y),
                point(right_bounds.right(), y_right),
              ),
              pattern_slash(hatch_color, hatch_width, hatch_interval),
            ));
          }
        }
        if let Some(start) = left_hatch_start.take() {
          let start_y = left_bounds.top() + line_height * start as f32;
          let end_y = left_bounds.top() + line_height * split_rows.len() as f32;
          diff_hatch_quads_left.push(fill(
            Bounds::from_corners(
              point(left_bounds.left(), start_y),
              point(left_bounds.right(), end_y),
            ),
            pattern_slash(hatch_color, hatch_width, hatch_interval),
          ));
        }
        if let Some(start) = right_hatch_start.take() {
          let start_y = right_bounds.top() + line_height * start as f32;
          let end_y = right_bounds.top() + line_height * split_rows.len() as f32;
          diff_hatch_quads_right.push(fill(
            Bounds::from_corners(
              point(right_bounds.left(), start_y),
              point(right_bounds.right(), end_y),
            ),
            pattern_slash(hatch_color, hatch_width, hatch_interval),
          ));
        }

        let is_left_row_staged = |row: &crate::document::SplitDiffRow| {
          row
            .left_line
            .map(|line_idx| editor.diff_line_is_staged(line_idx, cx))
            .unwrap_or(false)
        };
        let is_right_row_staged = |row: &crate::document::SplitDiffRow| {
          row
            .right_line
            .map(|line_idx| editor.diff_line_is_staged(line_idx, cx))
            .unwrap_or(false)
        };

        let mut prev_left_staged = false;
        let mut prev_right_staged = false;
        for (row_offset, row) in split_rows.iter().enumerate() {
          let left_staged = is_left_row_staged(row);
          let right_staged = is_right_row_staged(row);
          if left_staged {
            let y_left = left_bounds.top() + line_height * row_offset as f32;
            if !prev_left_staged {
              diff_border_quads_left.push(fill(
                Bounds::from_corners(
                  point(left_bounds.left(), y_left),
                  point(left_bounds.right(), y_left + border_height),
                ),
                left_border_color,
              ));
            }
            let next_left_staged = split_rows
              .get(row_offset + 1)
              .map(is_left_row_staged)
              .unwrap_or(false);
            if !next_left_staged {
              diff_border_quads_left.push(fill(
                Bounds::from_corners(
                  point(left_bounds.left(), y_left + line_height - border_height),
                  point(left_bounds.right(), y_left + line_height),
                ),
                left_border_color,
              ));
            }
          }
          if right_staged {
            let y_right = right_bounds.top() + line_height * row_offset as f32;
            if !prev_right_staged {
              diff_border_quads_right.push(fill(
                Bounds::from_corners(
                  point(right_bounds.left(), y_right),
                  point(right_bounds.right(), y_right + border_height),
                ),
                right_border_color,
              ));
            }
            let next_right_staged = split_rows
              .get(row_offset + 1)
              .map(is_right_row_staged)
              .unwrap_or(false);
            if !next_right_staged {
              diff_border_quads_right.push(fill(
                Bounds::from_corners(
                  point(right_bounds.left(), y_right + line_height - border_height),
                  point(right_bounds.right(), y_right + line_height),
                ),
                right_border_color,
              ));
            }
          }
          prev_left_staged = left_staged;
          prev_right_staged = right_staged;
        }
      } else {
        for line_idx in viewport.clone() {
          let Some(info) = document.diff_line_info(line_idx) else {
            continue;
          };
          let is_staged = editor.diff_line_is_staged(line_idx, cx);
          match info.kind {
            crate::document::DiffLineKind::Added => {
              let y = right_bounds.top() + line_height * (line_idx - viewport.start) as f32;
              let start = bounds.left();
              let end = bounds.right();
              let color = if is_staged { added_bg_staged } else { added_bg };
              diff_background_quads_right.push(fill(
                Bounds::from_corners(point(start, y), point(end, y + line_height)),
                color,
              ));
            }
            crate::document::DiffLineKind::Deleted => {
              let y = left_bounds.top() + line_height * (line_idx - viewport.start) as f32;
              let color = if is_staged { removed_bg_staged } else { removed_bg };
              diff_background_quads_right.push(fill(
                Bounds::from_corners(
                  point(bounds.left(), y),
                  point(bounds.right(), y + line_height),
                ),
                color,
              ));
            }
            _ => {}
          }
        }
        let mut prev_staged = false;
        let line_count = document.len_lines();
        for line_idx in viewport.clone() {
          let staged = editor.diff_line_is_staged(line_idx, cx);
          if staged {
            let y = bounds.top() + line_height * (line_idx - viewport.start) as f32;
            if !prev_staged {
              diff_border_quads_right.push(fill(
                Bounds::from_corners(
                  point(bounds.left(), y),
                  point(bounds.right(), y + border_height),
                ),
                top_border_color,
              ));
            }
            let next_staged = if line_idx + 1 < line_count {
              editor.diff_line_is_staged(line_idx + 1, cx)
            } else {
              false
            };
            if !next_staged {
              diff_border_quads_right.push(fill(
                Bounds::from_corners(
                  point(bounds.left(), y + line_height - border_height),
                  point(bounds.right(), y + line_height),
                ),
                bottom_border_color,
              ));
            }
          }
          prev_staged = staged;
        }
      }
    }

    let cache_range = viewport_line_range
      .clone()
      .unwrap_or_else(|| viewport.clone());

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
        editor.ensure_cache_size(cache_range.clone());
      });
    }

    let mut newly_shaped_left = Vec::new();
    if split_mode {
      for (line_idx, line_content) in lines_to_shape_left {
        let document = self.editor.read(cx).document().read(cx);
        let runs = if let Some(highlights) = document.get_highlights_for_line(line_idx) {
          highlights_to_text_runs(highlights.as_ref(), &line_content, &theme, &style)
        } else {
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
        newly_shaped_left.push((line_idx, shaped));
      }
    }

    if split_mode && !newly_shaped_left.is_empty() {
      self.editor.update(cx, |editor, _| {
        for (line_idx, shaped) in newly_shaped_left {
          let shaped_arc = Arc::new(shaped);
          editor
            .line_layouts_left
            .insert(line_idx, shaped_arc.clone());
          shaped_lines_left.push((line_idx, shaped_arc));
        }
        editor.ensure_cache_size(cache_range.clone());
      });
    }

    let max_width_right = shaped_lines
      .iter()
      .map(|(_, shaped)| shaped.width)
      .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
      .unwrap_or(px(DEFAULT_MAX_LINE_WIDTH));
    let max_width_left = shaped_lines_left
      .iter()
      .map(|(_, shaped)| shaped.width)
      .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
      .unwrap_or(px(0.0));
    let left_panel_width = left_bounds.size.width;
    let right_panel_width = if split_mode {
      right_bounds.size.width
    } else {
      bounds.size.width
    };

    self.editor.update(cx, |editor, _| {
      editor.max_line_width = editor.max_line_width.max(max_width_right);
      editor.max_line_width_left = editor.max_line_width_left.max(max_width_left);

      let right_content_width = editor.max_line_width + px(crate::editor::EXTRA_EDITOR_WIDTH);
      editor.scroll_offset_x_right = editor.clamp_scroll_x(
        editor.scroll_offset_x_right,
        right_content_width,
        right_panel_width,
      );

      if split_mode {
        let left_content_width = editor.max_line_width_left + px(crate::editor::EXTRA_EDITOR_WIDTH);
        editor.scroll_offset_x_left = editor.clamp_scroll_x(
          editor.scroll_offset_x_left,
          left_content_width,
          left_panel_width,
        );
      } else {
        editor.scroll_offset_x_left = px(0.0);
      }
    });

    let document = self.editor.read(cx).document().read(cx);

    let cursor_line = document.char_to_line(cursor_offset);
    let cursor_quad = if split_mode {
      let (panel_bounds, panel_scroll_x, panel_lines) = match active_panel {
        DiffPanelSide::Left => (left_bounds, left_scroll_x, &shaped_lines_left),
        DiffPanelSide::Right => (right_bounds, right_scroll_x, &shaped_lines),
      };
      let shaped_opt = panel_lines
        .iter()
        .find(|(idx, _)| *idx == cursor_line)
        .map(|(_, shaped)| shaped);
      let row_idx = document.split_row_for_line(cursor_line, active_panel);
      match (shaped_opt, row_idx) {
        (Some(shaped), Some(row_idx)) if row_idx >= viewport.start && row_idx < viewport.end => {
          let line_start = document.line_to_char(cursor_line);
          let cursor_in_line = cursor_offset - line_start;
          let cursor_x = shaped.x_for_index(cursor_in_line);
          let y = panel_bounds.top() + line_height * (row_idx - viewport.start) as f32;
          Some(fill(
            Bounds::new(
              point(panel_bounds.left() + cursor_x + panel_scroll_x, y),
              size(px(2.), line_height),
            ),
            theme.cursor(),
          ))
        }
        _ => None,
      }
    } else if viewport.contains(&cursor_line) {
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
            point(bounds.left() + cursor_x + right_scroll_x, y),
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

    let mut diff_word_quads_left = Vec::new();
    let mut diff_word_quads_right = Vec::new();
    if split_mode {
      for (line_idx, shaped_line) in &shaped_lines_left {
        let Some(info) = document.diff_line_info(*line_idx) else {
          continue;
        };
        if !matches!(info.kind, crate::document::DiffLineKind::Deleted) {
          continue;
        }
        let Some(ranges) = document.diff_word_ranges(*line_idx) else {
          continue;
        };
        let Some(row_idx) = document.split_row_for_line(*line_idx, DiffPanelSide::Left) else {
          continue;
        };
        if row_idx < viewport.start || row_idx >= viewport.end {
          continue;
        }
        let line_len = document
          .line_range(*line_idx)
          .map(|range| range.end - range.start)
          .unwrap_or(0);
        let y = left_bounds.top() + line_height * (row_idx - viewport.start) as f32;
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
          diff_word_quads_left.push(fill(
            Bounds::from_corners(
              point(left_bounds.left() + x_start + left_scroll_x, y),
              point(left_bounds.left() + x_end + left_scroll_x, y + line_height),
            ),
            theme.diff_removed_word_background(),
          ));
        }
      }

      for (line_idx, shaped_line) in &shaped_lines {
        let Some(info) = document.diff_line_info(*line_idx) else {
          continue;
        };
        if !matches!(info.kind, crate::document::DiffLineKind::Added) {
          continue;
        }
        let Some(ranges) = document.diff_word_ranges(*line_idx) else {
          continue;
        };
        let Some(row_idx) = document.split_row_for_line(*line_idx, DiffPanelSide::Right) else {
          continue;
        };
        if row_idx < viewport.start || row_idx >= viewport.end {
          continue;
        }
        let line_len = document
          .line_range(*line_idx)
          .map(|range| range.end - range.start)
          .unwrap_or(0);
        let y = right_bounds.top() + line_height * (row_idx - viewport.start) as f32;
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
          diff_word_quads_right.push(fill(
            Bounds::from_corners(
              point(right_bounds.left() + x_start + right_scroll_x, y),
              point(
                right_bounds.left() + x_end + right_scroll_x,
                y + line_height,
              ),
            ),
            theme.diff_added_word_background(),
          ));
        }
      }
    } else {
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
          diff_word_quads_right.push(fill(
            Bounds::from_corners(
              point(bounds.left() + x_start + right_scroll_x, y),
              point(bounds.left() + x_end + right_scroll_x, y + line_height),
            ),
            color,
          ));
        }
      }
    }

    let mut selection_quads = Vec::new();
    if !selected_range.is_empty() {
      let sel_start = selected_range.start;
      let sel_end = selected_range.end;
      let sel_start_line = document.char_to_line(sel_start);
      let sel_end_line = document.char_to_line(sel_end);

      for line_idx in sel_start_line..=sel_end_line {
        let (panel_bounds, panel_scroll_x, panel_lines, panel_side) = if split_mode {
          match active_panel {
            DiffPanelSide::Left => (
              left_bounds,
              left_scroll_x,
              &shaped_lines_left,
              DiffPanelSide::Left,
            ),
            DiffPanelSide::Right => (
              right_bounds,
              right_scroll_x,
              &shaped_lines,
              DiffPanelSide::Right,
            ),
          }
        } else {
          (bounds, right_scroll_x, &shaped_lines, DiffPanelSide::Right)
        };

        let row_idx = if split_mode {
          document.split_row_for_line(line_idx, panel_side)
        } else {
          Some(line_idx)
        };
        let Some(row_idx) = row_idx else {
          continue;
        };
        if row_idx < viewport.start || row_idx >= viewport.end {
          continue;
        }

        let line_range = document.line_range(line_idx).unwrap();
        let shaped_opt = panel_lines
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
          let y = panel_bounds.top() + line_height * (row_idx - viewport.start) as f32;

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
              point(panel_bounds.left() + x_start + panel_scroll_x, y),
              point(
                panel_bounds.left() + visual_x_end + panel_scroll_x,
                y + line_height,
              ),
            ),
            theme.selection(),
          ));
        }
      }
    }

    PrepaintState {
      shaped_lines,
      shaped_lines_left,
      cursor_quad,
      diff_background_quads_left,
      diff_background_quads_right,
      diff_border_quads_left,
      diff_border_quads_right,
      diff_hatch_quads_left,
      diff_hatch_quads_right,
      diff_word_quads_left,
      diff_word_quads_right,
      selection_quads,
      divider_quads,
      split_rows,
      viewport,
      bounds,
      left_bounds,
      right_bounds,
      line_height,
      scroll_hitbox,
      split_mode,
      left_scroll_x,
      right_scroll_x,
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

    let position_bounds = if prepaint.split_mode {
      prepaint.right_bounds
    } else {
      prepaint.bounds
    };

    let (left_row_to_line, right_row_to_line) = if prepaint.split_mode {
      (
        prepaint
          .split_rows
          .iter()
          .map(|row| row.left_line)
          .collect(),
        prepaint
          .split_rows
          .iter()
          .map(|row| row.right_line)
          .collect(),
      )
    } else {
      let row_to_line: Vec<Option<usize>> = (0..prepaint.viewport.len())
        .map(|offset| Some(prepaint.viewport.start + offset))
        .collect();
      (row_to_line.clone(), row_to_line)
    };

    // Use Rc to avoid cloning PositionMap in closures
    let left_position_map = Rc::new(PositionMap {
      shaped_lines: prepaint.shaped_lines_left.clone(),
      row_to_line: left_row_to_line,
      bounds: prepaint.left_bounds,
      line_height: prepaint.line_height,
      viewport: prepaint.viewport.clone(),
      scroll_x: prepaint.left_scroll_x,
    });
    let right_position_map = Rc::new(PositionMap {
      shaped_lines: prepaint.shaped_lines.clone(),
      row_to_line: right_row_to_line,
      bounds: position_bounds,
      line_height: prepaint.line_height,
      viewport: prepaint.viewport.clone(),
      scroll_x: prepaint.right_scroll_x,
    });

    window.on_mouse_event({
      let editor = self.editor.clone();
      let left_position_map = Rc::clone(&left_position_map);
      let right_position_map = Rc::clone(&right_position_map);
      let split_mode = prepaint.split_mode;
      let left_bounds = prepaint.left_bounds;
      let right_bounds = prepaint.right_bounds;
      move |event: &MouseDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
          editor.update(cx, |editor, cx| {
            if split_mode {
              let (panel, map) = if left_bounds.contains(&event.position) {
                (DiffPanelSide::Left, Rc::clone(&left_position_map))
              } else if right_bounds.contains(&event.position) {
                (DiffPanelSide::Right, Rc::clone(&right_position_map))
              } else {
                return;
              };
              editor.active_diff_panel = panel;
              editor.mouse_left_down(event, &map, window, cx);
            } else {
              editor.active_diff_panel = DiffPanelSide::Right;
              editor.mouse_left_down(event, &right_position_map, window, cx);
            }
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
      let left_position_map = Rc::clone(&left_position_map);
      let right_position_map = Rc::clone(&right_position_map);
      let split_mode = prepaint.split_mode;
      let left_bounds = prepaint.left_bounds;
      let right_bounds = prepaint.right_bounds;
      move |event: &MouseMoveEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble {
          let (is_selecting, active_panel) = {
            let editor = editor.read(cx);
            (
              editor.is_selecting,
              if split_mode {
                editor.active_diff_panel
              } else {
                DiffPanelSide::Right
              },
            )
          };
          if is_selecting {
            let map = if split_mode && active_panel == DiffPanelSide::Left {
              Rc::clone(&left_position_map)
            } else {
              Rc::clone(&right_position_map)
            };
            editor.update(cx, |editor, cx| {
              editor.mouse_dragged(event, &map, window, cx);
            });
          } else {
            let map = if split_mode {
              if left_bounds.contains(&event.position) {
                Some(Rc::clone(&left_position_map))
              } else if right_bounds.contains(&event.position) {
                Some(Rc::clone(&right_position_map))
              } else {
                None
              }
            } else {
              Some(Rc::clone(&right_position_map))
            };

            editor.update(cx, |editor, cx| {
              let Some(map) = map else {
                editor.set_hovered_change_range(None, cx);
                return;
              };
              let document = editor.document.read(cx);
              let Some(offset) = map.point_for_position(event.position, &document) else {
                editor.set_hovered_change_range(None, cx);
                return;
              };
              let line_idx = document.char_to_line(offset);
              let range = document.diff_change_range_at_line(line_idx);
              editor.set_hovered_change_range(range, cx);
            });
          }
        }
      }
    });

    // Handle mouse wheel scroll
    window.on_mouse_event({
      let editor = self.editor.clone();
      let scroll_hitbox = prepaint.scroll_hitbox.clone();
      let left_bounds = prepaint.left_bounds;
      let right_bounds = prepaint.right_bounds;
      let split_mode = prepaint.split_mode;
      move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && scroll_hitbox.should_handle_scroll(window) {
          editor.update(cx, |editor, cx| {
            let document = editor.document().read(cx);
            let total_rows = if split_mode {
              document.split_row_count()
            } else {
              document.len_lines()
            };
            let now = Instant::now();
            let reset_lock = editor
              .last_scroll_time
              .map(|last| now.duration_since(last) > Duration::from_millis(SCROLL_AXIS_TIMEOUT_MS))
              .unwrap_or(true);
            if reset_lock {
              editor.scroll_axis_lock = None;
            }
            editor.last_scroll_time = Some(now);

            // Extract scroll delta (handle both pixel and line scrolling)
            // Note: Negative delta because scrolling down should increase scroll_offset
            let pixel_delta = event.delta.pixel_delta(window.line_height());
            let delta_x_px = pixel_delta.x;
            let delta_y = match event.delta {
              ScrollDelta::Pixels(point) => -(point.y / px(PIXEL_SCROLL_DIVISOR)),
              ScrollDelta::Lines(point) => -(point.y * LINE_SCROLL_MULTIPLIER),
            };
            let abs_x = delta_x_px.abs();
            let abs_y = pixel_delta.y.abs();
            let axis = match editor.scroll_axis_lock {
              None => {
                let axis = if abs_x > abs_y * SCROLL_AXIS_RATIO {
                  ScrollAxis::Horizontal
                } else {
                  ScrollAxis::Vertical
                };
                editor.scroll_axis_lock = Some(axis);
                axis
              }
              Some(axis) => {
                if axis == ScrollAxis::Vertical && abs_x > abs_y * SCROLL_AXIS_SWITCH_RATIO {
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
              let use_right = !split_mode || right_bounds.contains(&event.position);
              let panel_width = if use_right {
                right_bounds.size.width
              } else {
                left_bounds.size.width
              };
              let content_width = if use_right {
                editor.max_line_width + px(crate::editor::EXTRA_EDITOR_WIDTH)
              } else {
                editor.max_line_width_left + px(crate::editor::EXTRA_EDITOR_WIDTH)
              };
              let current_scroll = if use_right {
                editor.scroll_offset_x_right
              } else {
                editor.scroll_offset_x_left
              };
              let new_scroll =
                editor.clamp_scroll_x(current_scroll + delta_x_px, content_width, panel_width);
              if use_right {
                editor.scroll_offset_x_right = new_scroll;
              } else {
                editor.scroll_offset_x_left = new_scroll;
              }
              cx.notify();
              return;
            }

            let new_scroll = (editor.scroll_offset_y + delta_y)
              .max(0.0)
              .min((total_rows.saturating_sub(1)) as f32);

            editor.scroll_offset_y = new_scroll;
            let viewport = editor.viewport_range(window.line_height(), total_rows);
            let viewport_line_range = if split_mode {
              document
                .split_row_range_to_line_range(viewport.clone())
                .or_else(|| Some(viewport.clone()))
            } else {
              Some(viewport.clone())
            };
            editor.document.update(cx, |doc, cx| {
              if let Some(viewport) = viewport_line_range.clone() {
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
              }
            });
            cx.notify();
          });
          cx.stop_propagation();
        }
      }
    });

    if prepaint.split_mode {
      let (active_panel, is_dark) = {
        let editor = self.editor.read(cx);
        (editor.active_diff_panel, editor.theme.is_dark)
      };
      let draw_left_selection = active_panel == DiffPanelSide::Left;
      let draw_right_selection = active_panel == DiffPanelSide::Right;
      let left_mask = ContentMask {
        bounds: prepaint.left_bounds,
      };
      let right_mask = ContentMask {
        bounds: prepaint.right_bounds,
      };
      let mut left_lines_to_paint = Vec::new();
      let mut right_lines_to_paint = Vec::new();
      for (row_offset, row) in prepaint.split_rows.iter().enumerate() {
        if let Some(line_idx) = row.left_line {
          if let Some((_, shaped_line)) = prepaint
            .shaped_lines_left
            .iter()
            .find(|(idx, _)| *idx == line_idx)
          {
            let y = prepaint.left_bounds.top() + prepaint.line_height * row_offset as f32;
            left_lines_to_paint.push((Arc::clone(shaped_line), y));
          }
        }
        if let Some(line_idx) = row.right_line {
          if let Some((_, shaped_line)) = prepaint
            .shaped_lines
            .iter()
            .find(|(idx, _)| *idx == line_idx)
          {
            let y = prepaint.right_bounds.top() + prepaint.line_height * row_offset as f32;
            right_lines_to_paint.push((Arc::clone(shaped_line), y));
          }
        }
      }

      window.with_content_mask(Some(left_mask), |window| {
        for quad in &prepaint.diff_background_quads_left {
          window.paint_quad(quad.clone());
        }
        for quad in &prepaint.diff_border_quads_left {
          window.paint_quad(quad.clone());
        }
        for quad in &prepaint.diff_hatch_quads_left {
          window.paint_quad(quad.clone());
        }
        for quad in &prepaint.diff_word_quads_left {
          window.paint_quad(quad.clone());
        }
        if draw_left_selection {
          for quad in &prepaint.selection_quads {
            window.paint_quad(quad.clone());
          }
        }
        for (shaped_line, y) in &left_lines_to_paint {
          shaped_line
            .paint(
              point(prepaint.left_bounds.left() + prepaint.left_scroll_x, *y),
              prepaint.line_height,
              TextAlign::Left,
              None,
              window,
              cx,
            )
            .ok();
        }
      });

      let content_bg = if is_dark { black() } else { white() };
      window.paint_quad(fill(prepaint.right_bounds, content_bg));

      window.with_content_mask(Some(right_mask), |window| {
        for quad in &prepaint.diff_background_quads_right {
          window.paint_quad(quad.clone());
        }
        for quad in &prepaint.diff_border_quads_right {
          window.paint_quad(quad.clone());
        }
        for quad in &prepaint.diff_hatch_quads_right {
          window.paint_quad(quad.clone());
        }
        for quad in &prepaint.diff_word_quads_right {
          window.paint_quad(quad.clone());
        }
        if draw_right_selection {
          for quad in &prepaint.selection_quads {
            window.paint_quad(quad.clone());
          }
        }
        for (shaped_line, y) in &right_lines_to_paint {
          shaped_line
            .paint(
              point(prepaint.right_bounds.left() + prepaint.right_scroll_x, *y),
              prepaint.line_height,
              TextAlign::Left,
              None,
              window,
              cx,
            )
            .ok();
        }
      });

      for quad in &prepaint.divider_quads {
        window.paint_quad(quad.clone());
      }
    } else {
      for quad in &prepaint.diff_background_quads_right {
        window.paint_quad(quad.clone());
      }
      for quad in &prepaint.diff_border_quads_right {
        window.paint_quad(quad.clone());
      }

      for quad in &prepaint.diff_word_quads_right {
        window.paint_quad(quad.clone());
      }

      for quad in &prepaint.selection_quads {
        window.paint_quad(quad.clone());
      }

      for (line_idx, shaped_line) in &prepaint.shaped_lines {
        let y = bounds.top() + prepaint.line_height * (*line_idx - prepaint.viewport.start) as f32;
        shaped_line
          .paint(
            point(bounds.left() + prepaint.right_scroll_x, y),
            prepaint.line_height,
            TextAlign::Left,
            None,
            window,
            cx,
          )
          .ok();
      }
    }

    // Paint cursor (if focused and visible from blink)
    let cursor_visible = self.editor.read(cx).cursor_blink.read(cx).visible();
    if is_focused
      && cursor_visible
      && let Some(cursor_quad) = &prepaint.cursor_quad
    {
      if prepaint.split_mode {
        let active_panel = self.editor.read(cx).active_diff_panel;
        let bounds = if active_panel == DiffPanelSide::Left {
          prepaint.left_bounds
        } else {
          prepaint.right_bounds
        };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
          window.paint_quad(cursor_quad.clone());
        });
      } else {
        window.paint_quad(cursor_quad.clone());
      }
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
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, Theme::light(), cx));
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
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, Theme::light(), cx));
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
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, Theme::light(), cx));
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
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, Theme::light(), cx));
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
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, Theme::light(), cx));
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
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, Theme::light(), cx));
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
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, Theme::light(), cx));
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
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, Theme::light(), cx));
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
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, Theme::light(), cx));
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
    let editor = cx.new(|cx| crate::editor::Editor::new("", None, None, Theme::light(), cx));
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
