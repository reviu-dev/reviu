use gpui::{
  App, Bounds, ElementId, Entity, GlobalElementId, InspectorElementId, LayoutId, PaintQuad, Pixels,
  Style, TextAlign, TextRun, Window, fill, point, prelude::*, px, relative, size,
};
use std::{collections::HashMap, ops::Range};

use git::DiffLineKind;

use crate::{
  editor::Editor,
  projection::{ChangeKind, DisplayLine, GAP_MARKER_TEXT, HunkState},
};

pub struct GutterElement {
  editor: Entity<Editor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupKind {
  Added,
  Removed,
  Mixed,
}

pub struct GutterPrepaintState {
  line_numbers: Vec<(usize, String)>,
  viewport: Range<usize>,
  line_height: Pixels,
  line_number_color: gpui::Hsla,
  line_backgrounds: Vec<PaintQuad>,
  stripe_quads: Vec<PaintQuad>,
  group_borders: Vec<PaintQuad>,
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
    let (
      viewport,
      line_numbers,
      line_height,
      line_number_color,
      line_backgrounds,
      stripe_quads,
      group_borders,
    ) = {
      let editor = self.editor.read(cx);
      let document = editor.document().read(cx);
      let line_height = window.line_height();
      let scroll_offset = editor.scroll_offset_y;
      let doc_line_count = document.len_lines();
      let total_lines = editor.display_line_count(doc_line_count);
      let theme = editor.theme.clone();
      let projection = editor.projection.clone();

      // Calculate viewport (same logic as EditorElement)
      let visible_line_count = ((bounds.size.height / line_height).ceil() as usize).max(1);
      let start_line = (scroll_offset.floor() as usize).min(total_lines.saturating_sub(1));
      let end_line = (start_line + visible_line_count).min(total_lines);
      let viewport = start_line..end_line;

      let mut group_kinds = HashMap::new();
      let mut group_border_colors = HashMap::new();
      if let Some(projection) = projection.as_ref() {
        for (group_id, group) in &projection.groups {
          let mut has_add = false;
          let mut has_remove = false;
          for line in &group.hunk.lines {
            match line.kind {
              DiffLineKind::Add => has_add = true,
              DiffLineKind::Remove => has_remove = true,
              DiffLineKind::Context => {}
            }
          }

          let kind = match (has_add, has_remove) {
            (true, false) => Some(GroupKind::Added),
            (false, true) => Some(GroupKind::Removed),
            (true, true) => Some(GroupKind::Mixed),
            _ => None,
          };

          if let Some(kind) = kind {
            group_kinds.insert(group_id.clone(), kind);
          }

          if group.state == HunkState::Staged {
            let mut first_kind: Option<DiffLineKind> = None;
            let mut last_kind: Option<DiffLineKind> = None;
            for line in &group.hunk.lines {
              match line.kind {
                DiffLineKind::Add | DiffLineKind::Remove => {
                  if first_kind.is_none() {
                    first_kind = Some(line.kind);
                  }
                  last_kind = Some(line.kind);
                }
                DiffLineKind::Context => {}
              }
            }

            if let (Some(first_kind), Some(last_kind)) = (first_kind, last_kind) {
              let top_color = match first_kind {
                DiffLineKind::Add => theme.diff_gutter_added(),
                DiffLineKind::Remove => theme.diff_gutter_removed(),
                DiffLineKind::Context => theme.diff_gutter_modified(),
              };
              let bottom_color = match last_kind {
                DiffLineKind::Add => theme.diff_gutter_added(),
                DiffLineKind::Remove => theme.diff_gutter_removed(),
                DiffLineKind::Context => theme.diff_gutter_modified(),
              };
              group_border_colors.insert(group_id.clone(), (top_color, bottom_color));
            }
          }
        }
      }

      let added_bg = theme.diff_added_background();
      let added_staged_bg = theme.diff_added_staged_background();
      let removed_bg = theme.diff_removed_background();
      let removed_staged_bg = theme.diff_removed_staged_background();
      let stripe_added = theme.diff_gutter_added();
      let stripe_removed = theme.diff_gutter_removed();
      let stripe_modified = theme.diff_gutter_modified();

      // Format line numbers for visible lines
      let mut line_numbers = Vec::new();
      let mut line_backgrounds = Vec::new();
      let mut stripe_quads = Vec::new();
      let mut group_borders = Vec::new();
      for display_idx in viewport.clone() {
        let display_line = editor.display_line(display_idx, doc_line_count);
        let line_number = match display_line {
          Some(DisplayLine::Doc { doc_line, .. }) => format!("{}", doc_line + 1),
          Some(DisplayLine::Gap { .. }) => GAP_MARKER_TEXT.to_string(),
          _ => String::new(),
        };
        line_numbers.push((display_idx, line_number));

        let background = match &display_line {
          Some(DisplayLine::Doc {
            change: Some(ChangeKind::Added),
            secondary,
            ..
          }) => Some(if *secondary {
            added_staged_bg
          } else {
            added_bg
          }),
          Some(DisplayLine::Removed { secondary, .. }) => Some(if *secondary {
            removed_staged_bg
          } else {
            removed_bg
          }),
          _ => None,
        };

        if let Some(color) = background {
          let y = bounds.top() + line_height * (display_idx - viewport.start) as f32;
          line_backgrounds.push(fill(
            Bounds::new(
              point(bounds.left(), y),
              size(bounds.size.width, line_height),
            ),
            color,
          ));
        }

        let group_id = match &display_line {
          Some(DisplayLine::Doc {
            change: Some(ChangeKind::Added),
            group_id,
            ..
          }) => group_id.clone(),
          Some(DisplayLine::Removed { group_id, .. }) => group_id.clone(),
          Some(DisplayLine::NoNewline { group_id, .. }) => group_id.clone(),
          _ => None,
        };

        if let Some(group_id) = group_id {
          if let Some(kind) = group_kinds.get(&group_id) {
            let stripe_color = match kind {
              GroupKind::Added => stripe_added,
              GroupKind::Removed => stripe_removed,
              GroupKind::Mixed => stripe_modified,
            };
            let y = bounds.top() + line_height * (display_idx - viewport.start) as f32;
            stripe_quads.push(fill(
              Bounds::new(point(bounds.left(), y), size(px(4.0), line_height)),
              stripe_color,
            ));
          }

          if let (Some(projection), Some((top_color, bottom_color))) = (
            projection.as_ref(),
            group_border_colors.get(&group_id),
          ) {
            let prev_group = display_idx
              .checked_sub(1)
              .and_then(|idx| projection.lines.get(idx))
              .and_then(|line| match line {
                DisplayLine::Doc { group_id, .. } => group_id.as_ref(),
                DisplayLine::Removed { group_id, .. } => group_id.as_ref(),
                DisplayLine::NoNewline { group_id, .. } => group_id.as_ref(),
                _ => None,
              });
            let next_group = projection
              .lines
              .get(display_idx + 1)
              .and_then(|line| match line {
                DisplayLine::Doc { group_id, .. } => group_id.as_ref(),
                DisplayLine::Removed { group_id, .. } => group_id.as_ref(),
                DisplayLine::NoNewline { group_id, .. } => group_id.as_ref(),
                _ => None,
              });

            let is_top = prev_group.map(|id| id.as_ref()) != Some(group_id.as_ref());
            let is_bottom = next_group.map(|id| id.as_ref()) != Some(group_id.as_ref());
            let border_thickness = px(1.0);
            let stripe_width = px(4.0);
            let width = if bounds.size.width > stripe_width {
              bounds.size.width - stripe_width
            } else {
              px(0.0)
            };
            let x = bounds.left() + stripe_width;
            let y = bounds.top() + line_height * (display_idx - viewport.start) as f32;

            if is_top {
              group_borders.push(fill(
                Bounds::new(point(x, y), size(width, border_thickness)),
                *top_color,
              ));
            }

            if is_bottom {
              group_borders.push(fill(
                Bounds::new(
                  point(x, y + line_height - border_thickness),
                  size(width, border_thickness),
                ),
                *bottom_color,
              ));
            }
          }
        }
      }

      let line_number_color = editor.theme.line_number();

      (
        viewport,
        line_numbers,
        line_height,
        line_number_color,
        line_backgrounds,
        stripe_quads,
        group_borders,
      )
    };

    GutterPrepaintState {
      line_numbers,
      viewport,
      line_height,
      line_number_color,
      line_backgrounds,
      stripe_quads,
      group_borders,
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

    for quad in &prepaint.line_backgrounds {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.stripe_quads {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.group_borders {
      window.paint_quad(quad.clone());
    }

    for (line_idx, line_number) in &prepaint.line_numbers {
      let y = bounds.top() + prepaint.line_height * (*line_idx - prepaint.viewport.start) as f32;

      let runs = vec![TextRun {
        len: line_number.len(),
        font: text_style.font(),
        color: text_color,
        background_color: None,
        underline: None,
        strikethrough: None,
      }];

      let shaped =
        window
          .text_system()
          .shape_line(line_number.clone().into(), font_size, &runs, None);

      // Align to the right with padding
      let text_width = shaped.width;
      let right_padding = px(20.0);
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
