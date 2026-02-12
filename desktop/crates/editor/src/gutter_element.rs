use gpui::{
  App, Bounds, ContentMask, DispatchPhase, ElementId, Entity, GlobalElementId, Hitbox,
  HitboxBehavior, InspectorElementId, LayoutId, MouseMoveEvent, PaintQuad, Path, PathBuilder,
  Pixels, ScrollDelta, ScrollWheelEvent, Style, TextAlign, TextRun, Window, fill, point,
  prelude::*, px, relative, size,
};
use gpui_component::ActiveTheme as _;
use std::{collections::HashMap, sync::Arc};

use git::DiffLineKind;

use crate::{
  editor::{Editor, ScrollAxis},
  projection::{
    ChangeKind, DisplayLine, HunkState, ReviewCommentBackground, ReviewCommentSide,
  },
};

const DIAGONAL_STRIPE_SPACING: f32 = 6.0;
const DIAGONAL_STRIPE_WIDTH: f32 = 1.0;
const PIXEL_SCROLL_DIVISOR: f32 = 20.0;
const LINE_SCROLL_MULTIPLIER: f32 = 3.0;
const FRACTIONAL_SCROLL_EPSILON: f32 = 0.001;
const SCROLL_AXIS_RATIO: f32 = 1.1;
const SCROLL_AXIS_SWITCH_RATIO: f32 = 1.4;
const SCROLL_AXIS_TIMEOUT_MS: u64 = 150;

fn has_fractional_scroll(scroll_offset: f32) -> bool {
  (scroll_offset - scroll_offset.floor()) > FRACTIONAL_SCROLL_EPSILON
}

fn line_y(
  bounds_top: Pixels,
  line_height: Pixels,
  display_line: usize,
  scroll_offset: f32,
) -> Pixels {
  bounds_top + line_height * (display_line as f32 - scroll_offset)
}

pub struct GutterElement {
  editor: Entity<Editor>,
  view: GutterView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupKind {
  Added,
  Removed,
  Mixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GutterView {
  Inline,
  SplitLeft,
  SplitRight,
}

pub struct GutterPrepaintState {
  line_numbers: Vec<(usize, String)>,
  line_height: Pixels,
  scroll_offset: f32,
  line_number_color: gpui::Hsla,
  line_backgrounds: Vec<PaintQuad>,
  gap_separators: Vec<PaintQuad>,
  stripe_quads: Vec<PaintQuad>,
  diag_paths: Vec<Path<Pixels>>,
  group_borders: Vec<PaintQuad>,
  scroll_hitbox: Hitbox,
}

impl GutterElement {
  pub fn new(editor: Entity<Editor>) -> Self {
    Self {
      editor,
      view: GutterView::Inline,
    }
  }

  pub fn split_left(editor: Entity<Editor>) -> Self {
    Self {
      editor,
      view: GutterView::SplitLeft,
    }
  }

  pub fn split_right(editor: Entity<Editor>) -> Self {
    Self {
      editor,
      view: GutterView::SplitRight,
    }
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
      line_numbers,
      line_height,
      scroll_offset,
      line_number_color,
      line_backgrounds,
      gap_separators,
      stripe_quads,
      diag_paths,
      group_borders,
      scroll_hitbox,
    ) = {
      let editor = self.editor.read(cx);
      let document = editor.document().read(cx);
      let line_height = window.line_height();
      let scroll_offset = editor.scroll_offset_y;
      let doc_line_count = document.len_lines();
      let total_lines = editor.display_line_count(doc_line_count);
      let theme = editor.theme.clone();
      let projection = editor.projection.clone();
      let show_stripes = matches!(self.view, GutterView::Inline);
      let scroll_hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

      // Calculate viewport (same logic as EditorElement)
      let mut visible_line_count = ((bounds.size.height / line_height).ceil() as usize).max(1);
      if has_fractional_scroll(scroll_offset) {
        visible_line_count += 1;
      }
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
            if has_add && has_remove {
              let removed = theme.diff_gutter_removed();
              let added = theme.diff_gutter_added();
              let (top_color, bottom_color) = match self.view {
                GutterView::SplitLeft => (removed, removed),
                GutterView::SplitRight => (added, added),
                GutterView::Inline => (removed, added),
              };
              group_border_colors.insert(group_id.clone(), (top_color, bottom_color));
            } else {
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
      }

      let added_bg = theme.diff_added_background();
      let added_staged_bg = theme.diff_added_staged_background();
      let removed_bg = theme.diff_removed_background();
      let removed_staged_bg = theme.diff_removed_staged_background();
      let stripe_added = theme.diff_gutter_added();
      let stripe_removed = theme.diff_gutter_removed();
      let stripe_modified = theme.diff_gutter_modified();

      let is_blank_for_view = |line: &DisplayLine| match self.view {
        GutterView::SplitLeft => {
          matches!(
            line,
            DisplayLine::Doc {
              change: Some(ChangeKind::Added),
              ..
            }
          ) || matches!(
            line,
            DisplayLine::ReviewComment {
              side: ReviewCommentSide::Right,
              ..
            }
          )
        }
        GutterView::SplitRight => {
          matches!(line, DisplayLine::Removed { .. })
            || matches!(
              line,
              DisplayLine::ReviewComment {
                side: ReviewCommentSide::Left,
                ..
              }
            )
        }
        GutterView::Inline => false,
      };

      let group_id_for_line = |line: &DisplayLine| -> Option<Arc<str>> {
        match line {
          DisplayLine::Doc {
            change: Some(ChangeKind::Added),
            group_id,
            ..
          } => group_id.clone(),
          DisplayLine::Modified { group_id, .. } => group_id.clone(),
          DisplayLine::Removed { group_id, .. } => group_id.clone(),
          DisplayLine::NoNewline { group_id, .. } => group_id.clone(),
          DisplayLine::ReviewComment { group_id, .. } => group_id.clone(),
          _ => None,
        }
      };

      // Format line numbers for visible lines
      let mut line_numbers = Vec::new();
      let mut line_backgrounds = Vec::new();
      let mut gap_separators = Vec::new();
      let mut stripe_quads = Vec::new();
      let mut diag_paths = Vec::new();
      let mut group_borders = Vec::new();
      let mut blank_ranges = Vec::new();
      let mut current_blank_start: Option<usize> = None;
      for display_idx in viewport.clone() {
        let display_line = editor.display_line(display_idx, doc_line_count);
        if let Some(DisplayLine::Gap { id, .. }) = display_line.as_ref() {
          let is_start_gap = id.start == 0;
          let is_end_gap = id.end == doc_line_count;
          if !is_start_gap && !is_end_gap {
            let y = line_y(bounds.top(), line_height, display_idx, scroll_offset) + line_height * 0.5;
            gap_separators.push(fill(
              Bounds::new(point(bounds.left(), y), size(bounds.size.width, px(1.0))),
              cx.theme().muted_foreground.opacity(0.35),
            ));
          }
        }

        let line_number = match (self.view, &display_line) {
          (GutterView::SplitLeft, Some(DisplayLine::Doc { old_line, .. })) => old_line
            .map(|line| format!("{}", line + 1))
            .unwrap_or_default(),
          (GutterView::SplitLeft, Some(DisplayLine::Modified { old_line, .. })) => {
            format!("{}", old_line + 1)
          }
          (GutterView::SplitLeft, Some(DisplayLine::Removed { old_line, .. })) => {
            format!("{}", old_line + 1)
          }
          (GutterView::SplitRight, Some(DisplayLine::Modified { doc_line, .. })) => {
            format!("{}", doc_line + 1)
          }
          (_, Some(DisplayLine::Doc { doc_line, .. })) => format!("{}", doc_line + 1),
          (_, Some(DisplayLine::Gap { .. })) => String::new(),
          _ => String::new(),
        };
        line_numbers.push((display_idx, line_number));

        let is_blank = display_line
          .as_ref()
          .map(|line| is_blank_for_view(line))
          .unwrap_or(false);

        let background = if is_blank {
          None
        } else {
          match &display_line {
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
            Some(DisplayLine::Modified { secondary, .. }) => match self.view {
              GutterView::SplitLeft => Some(if *secondary {
                removed_staged_bg
              } else {
                removed_bg
              }),
              GutterView::SplitRight => Some(if *secondary {
                added_staged_bg
              } else {
                added_bg
              }),
              GutterView::Inline => None,
            },
            Some(DisplayLine::ReviewComment {
              background: Some(ReviewCommentBackground::Added),
              secondary,
              ..
            }) => Some(if *secondary {
              added_staged_bg
            } else {
              added_bg
            }),
            Some(DisplayLine::ReviewComment {
              background: Some(ReviewCommentBackground::Removed),
              secondary,
              ..
            }) => Some(if *secondary {
              removed_staged_bg
            } else {
              removed_bg
            }),
            _ => None,
          }
        };

        if let Some(color) = background {
          let y = line_y(bounds.top(), line_height, display_idx, scroll_offset);
          line_backgrounds.push(fill(
            Bounds::new(
              point(bounds.left(), y),
              size(bounds.size.width, line_height),
            ),
            color,
          ));
        }

        if is_blank {
          if current_blank_start.is_none() {
            current_blank_start = Some(display_idx);
          }
        } else if let Some(start) = current_blank_start.take() {
          blank_ranges.push((start, display_idx.saturating_sub(1)));
        }

        let group_id: Option<Arc<str>> = display_line
          .as_ref()
          .and_then(|line| group_id_for_line(line));

        if let Some(group_id) = group_id {
          if show_stripes {
            if let Some(kind) = group_kinds.get(&group_id) {
              let stripe_color = match kind {
                GroupKind::Added => stripe_added,
                GroupKind::Removed => stripe_removed,
                GroupKind::Mixed => stripe_modified,
              };
              let y = line_y(bounds.top(), line_height, display_idx, scroll_offset);
              stripe_quads.push(fill(
                Bounds::new(point(bounds.left(), y), size(px(4.0), line_height)),
                stripe_color,
              ));
            }
          }

          if let (Some(projection), Some((top_color, bottom_color))) =
            (projection.as_ref(), group_border_colors.get(&group_id))
          {
            let prev_group = display_idx
              .checked_sub(1)
              .and_then(|idx| projection.lines.get(idx))
              .and_then(|line| group_id_for_line(line));
            let next_group = projection
              .lines
              .get(display_idx + 1)
              .and_then(|line| group_id_for_line(line));

            let is_top = prev_group.as_deref() != Some(group_id.as_ref());
            let is_bottom = next_group.as_deref() != Some(group_id.as_ref());
            let border_thickness = px(1.0);
            let stripe_width = if show_stripes { px(4.0) } else { px(0.0) };
            let width = if bounds.size.width > stripe_width {
              bounds.size.width - stripe_width
            } else {
              px(0.0)
            };
            let x = bounds.left() + stripe_width;
            let y = line_y(bounds.top(), line_height, display_idx, scroll_offset);

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

      if let Some(start) = current_blank_start.take() {
        if viewport.start < viewport.end {
          blank_ranges.push((start, viewport.end.saturating_sub(1)));
        }
      }

      if !blank_ranges.is_empty() {
        let stripe_spacing = px(DIAGONAL_STRIPE_SPACING);
        let stripe_width = px(DIAGONAL_STRIPE_WIDTH);
        for (start, end) in blank_ranges {
          let y = line_y(bounds.top(), line_height, start, scroll_offset);
          let height = line_height * (end - start + 1) as f32;
          let top = y;
          let bottom = y + height;
          let left = bounds.left();
          let right = bounds.right();
          let mut builder = PathBuilder::stroke(stripe_width);
          let start_n = ((left - height + bottom) / stripe_spacing).floor();
          let mut x = start_n * stripe_spacing - bottom;
          let end_x = right + height;
          while x < end_x {
            builder.move_to(point(x, bottom));
            builder.line_to(point(x + height, top));
            x += stripe_spacing;
          }
          if let Ok(path) = builder.build() {
            diag_paths.push(path);
          }
        }
      }

      let line_number_color = editor.theme.line_number();

      (
        line_numbers,
        line_height,
        scroll_offset,
        line_number_color,
        line_backgrounds,
        gap_separators,
        stripe_quads,
        diag_paths,
        group_borders,
        scroll_hitbox,
      )
    };

    GutterPrepaintState {
      line_numbers,
      line_height,
      scroll_offset,
      line_number_color,
      line_backgrounds,
      gap_separators,
      stripe_quads,
      diag_paths,
      group_borders,
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
    let text_style = window.text_style();
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let text_color = prepaint.line_number_color;

    for quad in &prepaint.line_backgrounds {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.gap_separators {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.stripe_quads {
      window.paint_quad(quad.clone());
    }

    if !prepaint.diag_paths.is_empty() {
      let stripe_color = cx.theme().muted_foreground.opacity(0.35);
      let mask = ContentMask { bounds };
      window.with_content_mask(Some(mask), |window| {
        for path in &prepaint.diag_paths {
          window.paint_path(path.clone(), stripe_color);
        }
      });
    }

    for quad in &prepaint.group_borders {
      window.paint_quad(quad.clone());
    }

    window.on_mouse_event({
      let editor = self.editor.clone();
      let line_height = prepaint.line_height;
      let bounds = bounds;
      move |event: &MouseMoveEvent, phase, _window, cx| {
        if phase != DispatchPhase::Bubble {
          return;
        }
        if !bounds.contains(&event.position) {
          return;
        }
        editor.update(cx, |editor, cx| {
          let hovered = {
            let y_offset = event.position.y - bounds.top();
            let line_float = editor.scroll_offset_y + (y_offset / line_height);
            if line_float.is_sign_negative() {
              None
            } else {
              let display_line = line_float.floor() as usize;
              let doc_line_count = editor.document().read(cx).len_lines();
              let total_lines = editor.display_line_count(doc_line_count);
              if display_line < total_lines {
                editor.group_id_for_modified_display_line(display_line)
              } else {
                None
              }
            }
          };

          if editor.hovered_group_id.as_deref() != hovered.as_deref() {
            editor.hovered_group_id = hovered;
            cx.notify();
          }
        });
      }
    });

    window.on_mouse_event({
      let editor = self.editor.clone();
      let scroll_hitbox = prepaint.scroll_hitbox.clone();
      move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !scroll_hitbox.should_handle_scroll(window) {
          return;
        }

        editor.update(cx, |editor, cx| {
          let document = editor.document().read(cx);
          let doc_line_count = document.len_lines();
          let total_lines = editor.display_line_count(doc_line_count);
          let now = std::time::Instant::now();
          let reset_lock = editor
            .last_scroll_time
            .map(|last| {
              now.duration_since(last) > std::time::Duration::from_millis(SCROLL_AXIS_TIMEOUT_MS)
            })
            .unwrap_or(true);
          if reset_lock {
            editor.scroll_axis_lock = None;
            editor.last_scroll_x = editor.scroll_handle.offset().x;
          }
          editor.last_scroll_time = Some(now);

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
              if matches!(axis, ScrollAxis::Horizontal) {
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
              } else if axis == ScrollAxis::Horizontal && abs_y > abs_x * SCROLL_AXIS_SWITCH_RATIO {
                editor.scroll_axis_lock = Some(ScrollAxis::Vertical);
                ScrollAxis::Vertical
              } else {
                axis
              }
            }
          };

          if axis == ScrollAxis::Horizontal {
            let new_scroll_x =
              editor.clamp_horizontal_scroll_x(editor.scroll_handle.offset().x + delta_x_px);
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
          let clamped_scroll_x = editor.clamp_horizontal_scroll_x(editor.last_scroll_x);
          if editor.scroll_handle.offset().x != clamped_scroll_x {
            editor
              .scroll_handle
              .set_offset(point(clamped_scroll_x, px(0.0)));
          }
          editor.last_scroll_x = clamped_scroll_x;
          let viewport = editor.viewport_range(window.line_height(), total_lines);
          let doc_viewport = editor.doc_range_for_display_viewport(viewport.clone());
          editor.document.update(cx, |doc, cx| {
            doc.schedule_viewport_highlights(
              doc_viewport.clone(),
              None,
              crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
              cx,
            );
          });
          cx.notify();
        });

        cx.stop_propagation();
      }
    });

    for (line_idx, line_number) in &prepaint.line_numbers {
      let y = line_y(
        bounds.top(),
        prepaint.line_height,
        *line_idx,
        prepaint.scroll_offset,
      );

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
