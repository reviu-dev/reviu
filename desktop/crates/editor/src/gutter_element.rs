use gpui::{
  App, Bounds, ElementId, Entity, GlobalElementId, InspectorElementId, LayoutId, Pixels, Style,
  TextAlign, TextRun, Window, fill, point, prelude::*, px, relative,
};
use std::ops::Range;

use crate::editor::Editor;

pub struct GutterElement {
  editor: Entity<Editor>,
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
}

impl GutterElement {
  pub fn new(editor: Entity<Editor>) -> Self {
    Self { editor }
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

      // Calculate viewport (same logic as EditorElement)
      let visible_line_count = ((bounds.size.height / line_height).ceil() as usize).max(1);
      let start_line = (scroll_offset.floor() as usize).min(document.len_lines().saturating_sub(1));
      let end_line = (start_line + visible_line_count).min(document.len_lines());
      let viewport = start_line..end_line;

      // Format line numbers for visible lines
      let mut lines = Vec::new();
      for line_idx in viewport.clone() {
        let diff_info = document.diff_line_info(line_idx);
        let line_number = if let Some(info) = diff_info {
          if matches!(info.kind, crate::document::DiffLineKind::Deleted) {
            String::new()
          } else {
            info
              .current_line
              .or(info.base_line)
              .map(|idx| format!("{}", idx + 1))
              .unwrap_or_default()
          }
        } else {
          format!("{}", line_idx + 1)
        };

        let marker_color = diff_info.and_then(|info| match info.gutter {
          crate::document::DiffGutterKind::Added => Some(editor.theme.diff_gutter_added()),
          crate::document::DiffGutterKind::Modified => Some(editor.theme.diff_gutter_modified()),
          crate::document::DiffGutterKind::None => None,
        });

        lines.push(GutterLine {
          line_idx,
          line_number,
          marker_color,
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

    for line in &prepaint.lines {
      let y =
        bounds.top() + prepaint.line_height * (line.line_idx - prepaint.viewport.start) as f32;

      if let Some(color) = line.marker_color {
        let marker_width = px(4.0);
        let marker_bounds = Bounds::from_corners(
          point(bounds.left(), y),
          point(bounds.left() + marker_width, y + prepaint.line_height),
        );
        window.paint_quad(fill(marker_bounds, color));
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
      let right_padding = px(8.0);
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
