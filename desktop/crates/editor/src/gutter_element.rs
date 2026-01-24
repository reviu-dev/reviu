use gpui::{
  App, Bounds, ElementId, Entity, GlobalElementId, InspectorElementId, LayoutId, Pixels, Style,
  TextAlign, TextRun, Window, fill, point, prelude::*, px, relative,
};
use std::ops::Range;

use crate::editor::{
  DiffViewMode, Editor, GUTTER_MARKER_WIDTH, GUTTER_RIGHT_PADDING,
  STAGED_DIFF_OPACITY_MULTIPLIER,
};

const STAGED_HUNK_BORDER_HEIGHT: f32 = 2.0;
const GUTTER_LEFT_BORDER_WIDTH: f32 = 3.0;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GutterSide {
  Inline,
  Left,
  Right,
}

pub struct GutterElement {
  editor: Entity<Editor>,
  side: GutterSide,
}

pub struct GutterPrepaintState {
  lines: Vec<GutterLine>,
  viewport: Range<usize>,
  line_height: Pixels,
  line_number_color: gpui::Hsla,
}

struct GutterLine {
  line_idx: usize,
  line_number: String,
  marker_color: Option<gpui::Hsla>,
  is_changed: bool,
  diff_kind: Option<crate::document::DiffLineKind>,
  is_staged: bool,
}

impl GutterElement {
  pub fn new(editor: Entity<Editor>, side: GutterSide) -> Self {
    Self { editor, side }
  }
}

impl IntoElement for GutterElement {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for GutterElement {
  type RequestLayoutState = ();
  type PrepaintState = GutterPrepaintState;

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
    let (viewport, lines, line_height, line_number_color) = {
      let editor = self.editor.read(cx);
      let document = editor.document().read(cx);
      let line_height = window.line_height();
      let scroll_offset = editor.scroll_offset_y;
      let split_mode = editor.diff_view_mode == DiffViewMode::Split && document.diff_enabled();

      // Calculate viewport (same logic as EditorElement)
      let visible_line_count = ((bounds.size.height / line_height).ceil() as usize).max(1);
      let total_rows = if split_mode {
        document.split_row_count()
      } else {
        document.len_lines()
      };
      let start_line = (scroll_offset.floor() as usize).min(total_rows.saturating_sub(1));
      let end_line = (start_line + visible_line_count).min(total_rows);
      let viewport = start_line..end_line;

      // Format line numbers for visible lines
      let mut lines = Vec::new();
      for line_idx in viewport.clone() {
        let split_row = if split_mode {
          document.split_row(line_idx)
        } else {
          None
        };
        let row_line = if split_mode {
          if let Some(row) = split_row {
            match self.side {
              GutterSide::Left => row.left_line,
              GutterSide::Right => row.right_line,
              GutterSide::Inline => Some(line_idx),
            }
          } else {
            Some(line_idx)
          }
        } else {
          Some(line_idx)
        };
        let diff_info = row_line.and_then(|row_line| document.diff_line_info(row_line));
        let hide_line_number = split_mode && split_row.is_some() && row_line.is_none();
        let line_number = if hide_line_number {
          String::new()
        } else {
          match (self.side, diff_info) {
            (GutterSide::Inline, Some(info)) => {
              if matches!(info.kind, crate::document::DiffLineKind::Deleted) {
                String::new()
              } else {
                info
                  .current_line
                  .or(info.base_line)
                  .map(|idx| format!("{}", idx + 1))
                  .unwrap_or_default()
              }
            }
            (GutterSide::Left, Some(info)) => {
              if matches!(info.kind, crate::document::DiffLineKind::Added) {
                String::new()
              } else {
                info
                  .base_line
                  .or(info.current_line)
                  .map(|idx| format!("{}", idx + 1))
                  .unwrap_or_default()
              }
            }
            (GutterSide::Right, Some(info)) => {
              if matches!(info.kind, crate::document::DiffLineKind::Deleted) {
                String::new()
              } else {
                info
                  .current_line
                  .or(info.base_line)
                  .map(|idx| format!("{}", idx + 1))
                  .unwrap_or_default()
              }
            }
            (_, None) => format!("{}", line_idx + 1),
          }
        };

        let (diff_kind, is_changed) = match diff_info {
          Some(info) => {
            let changed = info.kind != crate::document::DiffLineKind::Unchanged
              || info.gutter != crate::document::DiffGutterKind::None;
            (Some(info.kind), changed)
          }
          None => (None, false),
        };

        let marker_color = if split_mode && !matches!(self.side, GutterSide::Inline) {
          None
        } else {
          match self.side {
            GutterSide::Inline => diff_info.and_then(|info| match info.gutter {
              crate::document::DiffGutterKind::Added => Some(editor.theme.diff_gutter_added()),
              crate::document::DiffGutterKind::Modified => {
                Some(editor.theme.diff_gutter_modified())
              }
              crate::document::DiffGutterKind::None => None,
            }),
            GutterSide::Left => diff_info.and_then(|info| {
              if matches!(info.kind, crate::document::DiffLineKind::Deleted) {
                Some(editor.theme.diff_gutter_modified())
              } else {
                None
              }
            }),
            GutterSide::Right => diff_info.and_then(|info| {
              if matches!(info.kind, crate::document::DiffLineKind::Added) {
                match info.gutter {
                  crate::document::DiffGutterKind::Added => Some(editor.theme.diff_gutter_added()),
                  crate::document::DiffGutterKind::Modified => {
                    Some(editor.theme.diff_gutter_modified())
                  }
                  crate::document::DiffGutterKind::None => None,
                }
              } else {
                None
              }
            }),
          }
        };

        let is_staged = row_line
          .map(|line_idx| editor.diff_line_is_staged(line_idx, cx))
          .unwrap_or(false);

        lines.push(GutterLine {
          line_idx,
          line_number,
          marker_color,
          is_changed,
          diff_kind,
          is_staged,
        });
      }

      let line_number_color = editor.theme.line_number();

      (viewport, lines, line_height, line_number_color)
    };

    GutterPrepaintState {
      lines,
      viewport,
      line_height,
      line_number_color,
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
    let text_style = window.text_style();
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let text_color = prepaint.line_number_color;

    let editor = self.editor.read(cx);
    let added_bg = editor.theme.diff_added_background();
    let removed_bg = editor.theme.diff_removed_background();
    let mut added_bg_staged = added_bg;
    let mut removed_bg_staged = removed_bg;
    added_bg_staged.a = (added_bg_staged.a * STAGED_DIFF_OPACITY_MULTIPLIER).min(1.0);
    removed_bg_staged.a = (removed_bg_staged.a * STAGED_DIFF_OPACITY_MULTIPLIER).min(1.0);
    let mut removed_border = editor.theme.diff_removed_background();
    removed_border.a = 1.0;
    let mut added_border = editor.theme.diff_added_background();
    added_border.a = 1.0;
    let (top_border_color, bottom_border_color) = match self.side {
      GutterSide::Left => (removed_border, removed_border),
      GutterSide::Right => (added_border, added_border),
      GutterSide::Inline => (removed_border, added_border),
    };
    let marker_width = px(GUTTER_MARKER_WIDTH);

    for (idx, line) in prepaint.lines.iter().enumerate() {
      let y =
        bounds.top() + prepaint.line_height * (line.line_idx - prepaint.viewport.start) as f32;
      let band_offset = if line.marker_color.is_some() {
        marker_width
      } else {
        px(0.0)
      };
      let (prev_staged, next_staged) = if line.is_staged {
        (
          idx > 0 && prepaint.lines[idx - 1].is_staged,
          idx + 1 < prepaint.lines.len() && prepaint.lines[idx + 1].is_staged,
        )
      } else {
        (false, false)
      };

      let bg_color = match line.diff_kind {
        Some(crate::document::DiffLineKind::Added) => {
          Some(if line.is_staged { added_bg_staged } else { added_bg })
        }
        Some(crate::document::DiffLineKind::Deleted) => {
          Some(if line.is_staged {
            removed_bg_staged
          } else {
            removed_bg
          })
        }
        _ => None,
      };
      if let Some(color) = bg_color {
        window.paint_quad(fill(
          Bounds::from_corners(
            point(bounds.left() + band_offset, y),
            point(bounds.right(), y + prepaint.line_height),
          ),
          color,
        ));
      }

      if line.is_staged {
        let border_height = px(STAGED_HUNK_BORDER_HEIGHT);
        if !prev_staged {
          window.paint_quad(fill(
            Bounds::from_corners(
              point(bounds.left() + band_offset, y),
              point(bounds.right(), y + border_height),
            ),
            top_border_color,
          ));
        }
        if !next_staged {
          window.paint_quad(fill(
            Bounds::from_corners(
              point(
                bounds.left() + band_offset,
                y + prepaint.line_height - border_height,
              ),
              point(bounds.right(), y + prepaint.line_height),
            ),
            bottom_border_color,
          ));
        }
      }

      if let Some(color) = line.marker_color {
        let marker_bounds = Bounds::from_corners(
          point(bounds.left(), y),
          point(bounds.left() + marker_width, y + prepaint.line_height),
        );
        if line.is_staged {
          let mut band_color = color;
          band_color.a = (band_color.a * STAGED_DIFF_OPACITY_MULTIPLIER).min(1.0);
          window.paint_quad(fill(marker_bounds, band_color));

          if line.is_changed {
            let mut band_border_color = color;
            band_border_color.a = 1.0;
            let border_height = px(STAGED_HUNK_BORDER_HEIGHT);
            let left_border_width = px(GUTTER_LEFT_BORDER_WIDTH);
            window.paint_quad(fill(
              Bounds::from_corners(
                point(marker_bounds.left(), y),
                point(
                  marker_bounds.left() + left_border_width,
                  y + prepaint.line_height,
                ),
              ),
              band_border_color,
            ));
            if !prev_staged {
              window.paint_quad(fill(
                Bounds::from_corners(
                  point(marker_bounds.left(), y),
                  point(marker_bounds.right(), y + border_height),
                ),
                band_border_color,
              ));
            }
            if !next_staged {
              window.paint_quad(fill(
                Bounds::from_corners(
                  point(
                    marker_bounds.left(),
                    y + prepaint.line_height - border_height,
                  ),
                  point(marker_bounds.right(), y + prepaint.line_height),
                ),
                band_border_color,
              ));
            }
          }
        } else {
          window.paint_quad(fill(marker_bounds, color));
        }
      }

      if line.line_number.is_empty() {
        continue;
      }

      let runs = vec![TextRun {
        len: line.line_number.len(),
        font: text_style.font(),
        color: text_color,
        background_color: None,
        underline: None,
        strikethrough: None,
      }];

      let shaped =
        window
          .text_system()
          .shape_line(line.line_number.clone().into(), font_size, &runs, None);

      // Align to the right with padding
      let text_width = shaped.width;
      let right_padding = px(GUTTER_RIGHT_PADDING);
      let x = bounds.right() - text_width - right_padding;

      let line_origin = point(x, y);
      shaped
        .paint(
          line_origin,
          prepaint.line_height,
          TextAlign::Right,
          None,
          window,
          cx,
        )
        .ok();
    }
  }
}
