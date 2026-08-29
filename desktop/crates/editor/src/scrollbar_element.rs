use gpui::{
  App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, Entity,
  GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, LayoutId, MouseButton,
  MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Position, Style, Window, fill,
  point, prelude::*, px, relative, size,
};

use gpui_component::ActiveTheme as _;

use crate::editor::{
  Editor, EditorScrollbarDrag, ScrollAxis, ScrollbarMarker, ScrollbarMarkerKind,
};

const TRACK_THICKNESS: Pixels = px(16.0);
const THUMB_THICKNESS: Pixels = px(6.0);
const THUMB_ACTIVE_THICKNESS: Pixels = px(8.0);
const MIN_THUMB_LENGTH: Pixels = px(48.0);
const EDGE_INSET: Pixels = px(4.0);
const MARKER_THICKNESS: Pixels = px(3.0);
const MARKER_THUMB_GAP: Pixels = px(1.0);
const MIN_MARKER_LENGTH: Pixels = px(2.0);

#[derive(Clone, Copy, Debug)]
enum ScrollbarKind {
  Horizontal {
    id: &'static str,
    left_inset: Pixels,
  },
  Vertical,
}

pub(crate) struct EditorScrollbarElement {
  editor: Entity<Editor>,
  kind: ScrollbarKind,
}

#[derive(Clone, Debug)]
pub(crate) struct ScrollbarMetrics {
  id: &'static str,
  axis: ScrollAxis,
  track_bounds: Bounds<Pixels>,
  thumb_bounds: Option<Bounds<Pixels>>,
  scroll_amount: f32,
  scroll_max: f32,
  track_travel: Pixels,
  line_height: Pixels,
  total_lines: usize,
  viewport_height: Pixels,
  markers: Vec<ScrollbarMarker>,
}

pub(crate) struct ScrollbarPrepaintState {
  metrics: Option<ScrollbarMetrics>,
  hitbox: Option<Hitbox>,
}

impl EditorScrollbarElement {
  pub(crate) fn horizontal(editor: Entity<Editor>, id: &'static str, left_inset: Pixels) -> Self {
    Self {
      editor,
      kind: ScrollbarKind::Horizontal { id, left_inset },
    }
  }

  pub(crate) fn vertical(editor: Entity<Editor>) -> Self {
    Self {
      editor,
      kind: ScrollbarKind::Vertical,
    }
  }

  fn metrics(&self, bounds: Bounds<Pixels>, cx: &App) -> Option<ScrollbarMetrics> {
    let editor = self.editor.read(cx);
    match self.kind {
      ScrollbarKind::Horizontal { id, left_inset } => {
        let track_width = (bounds.size.width - left_inset).max(px(0.0));
        if track_width <= px(0.0) {
          return None;
        }
        let content_width = editor.horizontal_scrollbar_content_width();
        let viewport_width = track_width;
        let max_scroll = (content_width - viewport_width).max(px(0.0));
        if max_scroll <= px(0.0) {
          return None;
        }
        let track_bounds = Bounds::new(
          point(
            bounds.left() + left_inset,
            bounds.bottom() - TRACK_THICKNESS,
          ),
          size(track_width, TRACK_THICKNESS),
        );
        let scroll_amount = (-editor.scroll_handle.offset().x / px(1.0)).max(0.0);
        let scroll_max = max_scroll / px(1.0);
        let thumb_bounds = thumb_bounds_for_metrics(
          track_bounds,
          ScrollAxis::Horizontal,
          scroll_amount,
          scroll_max,
        )?;
        Some(ScrollbarMetrics {
          id,
          axis: ScrollAxis::Horizontal,
          track_bounds,
          thumb_bounds: Some(thumb_bounds),
          scroll_amount,
          scroll_max,
          track_travel: (track_width - thumb_bounds.size.width).max(px(0.0)),
          line_height: editor.measured_editor_line_height(),
          total_lines: 0,
          viewport_height: bounds.size.height,
          markers: Vec::new(),
        })
      }
      ScrollbarKind::Vertical => {
        let track_height = (bounds.size.height - TRACK_THICKNESS).max(px(0.0));
        if track_height <= px(0.0) {
          return None;
        }
        let line_height = editor.measured_editor_line_height();
        let total_lines = editor.display_line_count(editor.document.read(cx).len_lines());
        let metrics =
          Editor::vertical_scroll_metrics_for_height(bounds.size.height, line_height, total_lines);
        let markers = editor.scrollbar_markers(cx);
        if !should_show_vertical_scrollbar(metrics.max_scroll, !markers.is_empty()) {
          return None;
        }
        let track_bounds = Bounds::new(
          point(bounds.right() - TRACK_THICKNESS, bounds.top()),
          size(TRACK_THICKNESS, track_height),
        );
        let line_height_px = (line_height / px(1.0)).max(1.0);
        let scroll_amount = editor.scroll_offset_y * line_height_px;
        let scroll_max = metrics.max_scroll * line_height_px;
        let thumb_bounds = if scroll_max > 0.0 {
          Some(thumb_bounds_for_metrics(
            track_bounds,
            ScrollAxis::Vertical,
            scroll_amount,
            scroll_max,
          )?)
        } else {
          None
        };
        Some(ScrollbarMetrics {
          id: "vertical",
          axis: ScrollAxis::Vertical,
          track_bounds,
          thumb_bounds,
          scroll_amount,
          scroll_max,
          track_travel: thumb_bounds
            .map(|thumb_bounds| (track_height - thumb_bounds.size.height).max(px(0.0)))
            .unwrap_or(px(0.0)),
          line_height,
          total_lines,
          viewport_height: bounds.size.height,
          markers,
        })
      }
    }
  }
}

impl IntoElement for EditorScrollbarElement {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for EditorScrollbarElement {
  type RequestLayoutState = ();
  type PrepaintState = ScrollbarPrepaintState;

  fn id(&self) -> Option<ElementId> {
    None
  }

  fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
    None
  }

  fn request_layout(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    let style = Style {
      position: Position::Absolute,
      size: gpui::Size {
        width: relative(1.0).into(),
        height: relative(1.0).into(),
      },
      ..Default::default()
    };
    (window.request_layout(style, None, cx), ())
  }

  fn prepaint(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _: &mut Self::RequestLayoutState,
    window: &mut Window,
    cx: &mut App,
  ) -> Self::PrepaintState {
    let metrics = self.metrics(bounds, cx);
    let hitbox = metrics.as_ref().map(|metrics| {
      window.with_content_mask(
        Some(ContentMask {
          bounds: metrics.track_bounds,
        }),
        |window| window.insert_hitbox(metrics.track_bounds, HitboxBehavior::BlockMouseExceptScroll),
      )
    });
    ScrollbarPrepaintState { metrics, hitbox }
  }

  fn paint(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    _: Bounds<Pixels>,
    _: &mut Self::RequestLayoutState,
    prepaint: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
    let Some(metrics) = prepaint.metrics.clone() else {
      return;
    };
    let Some(hitbox) = prepaint.hitbox.clone() else {
      return;
    };

    let active = self
      .editor
      .read(cx)
      .scrollbar_drag
      .is_some_and(|drag| drag.id == metrics.id);
    let hovered = hitbox.is_hovered(window)
      || self.editor.read(cx).scrollbar_hovered_axis == Some(metrics.axis)
      || active;
    let thumb_hovered = active
      || metrics
        .thumb_bounds
        .is_some_and(|bounds| bounds.contains(&window.mouse_position()));
    let theme = cx.theme();
    let marker_theme = self.editor.read(cx).theme.clone();
    let track_color = theme.scrollbar;
    let thumb_color = if thumb_hovered {
      theme.scrollbar_thumb_hover
    } else {
      theme.scrollbar_thumb
    };
    let thumb_thickness = if thumb_hovered {
      THUMB_ACTIVE_THICKNESS
    } else {
      THUMB_THICKNESS
    };
    let thumb_radius = thumb_thickness / 2.0;

    window.with_content_mask(
      Some(ContentMask {
        bounds: metrics.track_bounds,
      }),
      |window| {
        if hovered && track_color.a > 0.0 {
          window.paint_quad(fill(metrics.track_bounds, track_color));
        }
        for marker in &metrics.markers {
          if let Some(bounds) = marker_fill_bounds(&metrics, marker) {
            window.paint_quad(
              fill(bounds, marker_color(marker.kind, &marker_theme)).corner_radii(px(1.5)),
            );
          }
        }
        if let Some(thumb_bounds) = thumb_fill_bounds(&metrics, thumb_thickness) {
          window.paint_quad(fill(thumb_bounds, thumb_color).corner_radii(thumb_radius));
        }
      },
    );
    window.set_cursor_style(CursorStyle::Arrow, &hitbox);

    window.on_mouse_event({
      let editor = self.editor.clone();
      let metrics = metrics.clone();
      let hitbox = hitbox.clone();
      move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble
          || event.button != MouseButton::Left
          || !hitbox.is_hovered(window)
        {
          return;
        }

        editor.update(cx, |editor, cx| {
          let Some(thumb_bounds) = metrics.thumb_bounds else {
            return;
          };
          let pointer = pointer_along_axis(event.position, metrics.axis);
          let mut scroll_start = metrics.scroll_amount;
          if !thumb_bounds.contains(&event.position) {
            scroll_start = scroll_amount_for_track_position(&metrics, pointer);
            set_scroll_amount(editor, &metrics, scroll_start, cx);
          }
          editor.scrollbar_drag = Some(EditorScrollbarDrag {
            id: metrics.id,
            axis: metrics.axis,
            pointer_start: pointer,
            scroll_start,
            scroll_max: metrics.scroll_max,
            track_travel: metrics.track_travel,
          });
          editor.set_scrollbar_hovered_axis(Some(metrics.axis), cx);
          cx.notify();
        });
        cx.stop_propagation();
      }
    });

    window.on_mouse_event({
      let editor = self.editor.clone();
      let metrics = metrics.clone();
      let hitbox = hitbox.clone();
      move |event: &MouseMoveEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
          return;
        }

        editor.update(cx, |editor, cx| {
          if let Some(drag) = editor.scrollbar_drag.filter(|drag| drag.id == metrics.id) {
            let pointer = pointer_along_axis(event.position, drag.axis);
            let travel = (drag.track_travel / px(1.0)).max(1.0);
            let delta = (pointer - drag.pointer_start) / px(1.0);
            let amount =
              (drag.scroll_start + (delta / travel) * drag.scroll_max).clamp(0.0, drag.scroll_max);
            set_scroll_amount(editor, &metrics, amount, cx);
            editor.set_scrollbar_hovered_axis(Some(drag.axis), cx);
            cx.notify();
            cx.stop_propagation();
          } else if hitbox.is_hovered(window) {
            editor.set_scrollbar_hovered_axis(Some(metrics.axis), cx);
          } else if editor.scrollbar_hovered_axis == Some(metrics.axis) {
            editor.set_scrollbar_hovered_axis(None, cx);
          }
        });
      }
    });

    window.on_mouse_event({
      let editor = self.editor.clone();
      let id = metrics.id;
      move |event: &MouseUpEvent, phase, _window, cx| {
        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
          return;
        }
        editor.update(cx, |editor, cx| {
          if editor.scrollbar_drag.is_some_and(|drag| drag.id == id) {
            editor.scrollbar_drag = None;
            editor.set_scrollbar_hovered_axis(None, cx);
            cx.notify();
            cx.stop_propagation();
          }
        });
      }
    });
  }
}

fn pointer_along_axis(position: Point<Pixels>, axis: ScrollAxis) -> Pixels {
  match axis {
    ScrollAxis::Horizontal => position.x,
    ScrollAxis::Vertical => position.y,
  }
}

fn set_scroll_amount(
  editor: &mut Editor,
  metrics: &ScrollbarMetrics,
  amount: f32,
  cx: &mut gpui::Context<Editor>,
) {
  match metrics.axis {
    ScrollAxis::Horizontal => editor.set_horizontal_scroll_offset(px(-amount)),
    ScrollAxis::Vertical => {
      let line_height_px = (metrics.line_height / px(1.0)).max(1.0);
      editor.set_vertical_scroll_offset_for_height(
        amount / line_height_px,
        metrics.viewport_height,
        metrics.line_height,
        metrics.total_lines,
        cx,
      );
    }
  }
}

fn scroll_amount_for_track_position(metrics: &ScrollbarMetrics, pointer: Pixels) -> f32 {
  let start = pointer_along_axis(metrics.track_bounds.origin, metrics.axis);
  let Some(thumb_bounds) = metrics.thumb_bounds else {
    return 0.0;
  };
  let thumb_length = match metrics.axis {
    ScrollAxis::Horizontal => thumb_bounds.size.width,
    ScrollAxis::Vertical => thumb_bounds.size.height,
  };
  let travel = (metrics.track_travel / px(1.0)).max(1.0);
  let offset = (pointer - start - thumb_length / 2.0) / px(1.0);
  ((offset / travel) * metrics.scroll_max).clamp(0.0, metrics.scroll_max)
}

fn thumb_bounds_for_metrics(
  track_bounds: Bounds<Pixels>,
  axis: ScrollAxis,
  scroll_amount: f32,
  scroll_max: f32,
) -> Option<Bounds<Pixels>> {
  if scroll_max <= 0.0 {
    return None;
  }
  let track_length = match axis {
    ScrollAxis::Horizontal => track_bounds.size.width,
    ScrollAxis::Vertical => track_bounds.size.height,
  };
  if track_length <= px(0.0) {
    return None;
  }

  let content_length = track_length + px(scroll_max);
  let thumb_length = (track_length / content_length * track_length)
    .max(MIN_THUMB_LENGTH.min(track_length))
    .min(track_length);
  let track_travel = (track_length - thumb_length).max(px(0.0));
  let progress = (scroll_amount / scroll_max).clamp(0.0, 1.0);
  let thumb_start = track_travel * progress;

  Some(match axis {
    ScrollAxis::Horizontal => Bounds::new(
      point(track_bounds.left() + thumb_start, track_bounds.top()),
      size(thumb_length, track_bounds.size.height),
    ),
    ScrollAxis::Vertical => Bounds::new(
      point(track_bounds.left(), track_bounds.top() + thumb_start),
      size(track_bounds.size.width, thumb_length),
    ),
  })
}

fn should_show_vertical_scrollbar(max_scroll: f32, has_markers: bool) -> bool {
  max_scroll > 0.0 || has_markers
}

fn thumb_fill_bounds(metrics: &ScrollbarMetrics, thickness: Pixels) -> Option<Bounds<Pixels>> {
  let thumb_bounds = metrics.thumb_bounds?;
  Some(match metrics.axis {
    ScrollAxis::Horizontal => Bounds::new(
      point(
        thumb_bounds.left(),
        metrics.track_bounds.bottom() - EDGE_INSET - thickness,
      ),
      size(thumb_bounds.size.width, thickness),
    ),
    ScrollAxis::Vertical => Bounds::new(
      point(
        metrics.track_bounds.right() - EDGE_INSET - thickness,
        thumb_bounds.top(),
      ),
      size(thickness, thumb_bounds.size.height),
    ),
  })
}

fn marker_fill_bounds(
  metrics: &ScrollbarMetrics,
  marker: &ScrollbarMarker,
) -> Option<Bounds<Pixels>> {
  if metrics.axis != ScrollAxis::Vertical || metrics.total_lines == 0 {
    return None;
  }

  let start_line = marker.range.start.min(metrics.total_lines);
  let end_line = marker
    .range
    .end
    .max(start_line + 1)
    .min(metrics.total_lines);
  let total_lines = metrics.total_lines as f32;
  let track_length = metrics.track_bounds.size.height;
  let start = metrics.track_bounds.top() + track_length * (start_line as f32 / total_lines);
  let end = metrics.track_bounds.top() + track_length * (end_line as f32 / total_lines);
  let length = (end - start).max(MIN_MARKER_LENGTH).min(track_length);
  let top = start.min(metrics.track_bounds.bottom() - length);

  let marker_left = (metrics.track_bounds.right()
    - EDGE_INSET
    - THUMB_ACTIVE_THICKNESS
    - MARKER_THUMB_GAP
    - MARKER_THICKNESS)
    .max(metrics.track_bounds.left());

  Some(Bounds::new(
    point(marker_left, top),
    size(MARKER_THICKNESS, length),
  ))
}

fn marker_color(kind: ScrollbarMarkerKind, theme: &ui::Theme) -> Hsla {
  let mut color = match kind {
    ScrollbarMarkerKind::DiffAdded => theme.diff_gutter_added(),
    ScrollbarMarkerKind::DiffRemoved => theme.diff_gutter_removed(),
    ScrollbarMarkerKind::DiffModified => theme.diff_gutter_modified(),
    ScrollbarMarkerKind::FindMatch => Hsla {
      h: 48.0 / 360.0,
      s: 0.95,
      l: 0.55,
      a: 1.0,
    },
    ScrollbarMarkerKind::Conflict => theme.current_conflict_stripe(),
    ScrollbarMarkerKind::ReviewComment => theme.hunk_focused_border(),
  };
  color.a = color.a.min(0.9);
  color
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn horizontal_thumb_moves_with_scroll_amount() {
    let track = Bounds::new(point(px(10.0), px(90.0)), size(px(100.0), px(12.0)));

    let start =
      thumb_bounds_for_metrics(track, ScrollAxis::Horizontal, 0.0, 100.0).expect("start thumb");
    let end =
      thumb_bounds_for_metrics(track, ScrollAxis::Horizontal, 100.0, 100.0).expect("end thumb");

    assert_eq!(start.left(), px(10.0));
    assert!(end.left() > start.left());
    assert_eq!(end.right(), track.right());
  }

  fn test_vertical_metrics(total_lines: usize) -> ScrollbarMetrics {
    let track = Bounds::new(point(px(0.0), px(0.0)), size(px(16.0), px(100.0)));
    let thumb = thumb_bounds_for_metrics(track, ScrollAxis::Vertical, 0.0, 100.0).expect("thumb");
    ScrollbarMetrics {
      id: "test",
      axis: ScrollAxis::Vertical,
      track_bounds: track,
      thumb_bounds: Some(thumb),
      scroll_amount: 0.0,
      scroll_max: 100.0,
      track_travel: track.size.height - thumb.size.height,
      line_height: px(20.0),
      total_lines,
      viewport_height: px(100.0),
      markers: Vec::new(),
    }
  }

  #[test]
  fn marker_bounds_place_line_on_vertical_track() {
    let metrics = test_vertical_metrics(100);
    let marker = ScrollbarMarker {
      range: 50..51,
      kind: ScrollbarMarkerKind::DiffAdded,
    };

    let bounds = marker_fill_bounds(&metrics, &marker).expect("marker bounds");

    assert_eq!(bounds.left(), px(0.0));
    assert_eq!(bounds.size.width, MARKER_THICKNESS);
    assert!(bounds.top() >= px(50.0));
    assert!(bounds.bottom() <= px(53.0));
  }

  #[test]
  fn marker_bounds_clamps_last_line_to_track() {
    let metrics = test_vertical_metrics(100);
    let marker = ScrollbarMarker {
      range: 99..100,
      kind: ScrollbarMarkerKind::DiffRemoved,
    };

    let bounds = marker_fill_bounds(&metrics, &marker).expect("marker bounds");

    assert_eq!(bounds.bottom(), metrics.track_bounds.bottom());
  }

  #[test]
  fn vertical_scrollbar_shows_when_markers_exist_without_overflow() {
    assert!(should_show_vertical_scrollbar(0.0, true));
    assert!(should_show_vertical_scrollbar(1.0, false));
    assert!(!should_show_vertical_scrollbar(0.0, false));
  }

  #[test]
  fn vertical_track_click_centers_thumb() {
    let track = Bounds::new(point(px(0.0), px(0.0)), size(px(12.0), px(100.0)));
    let thumb = thumb_bounds_for_metrics(track, ScrollAxis::Vertical, 0.0, 100.0).expect("thumb");
    let metrics = ScrollbarMetrics {
      id: "test",
      axis: ScrollAxis::Vertical,
      track_bounds: track,
      thumb_bounds: Some(thumb),
      scroll_amount: 0.0,
      scroll_max: 100.0,
      track_travel: track.size.height - thumb.size.height,
      line_height: px(20.0),
      total_lines: 10,
      viewport_height: px(100.0),
      markers: Vec::new(),
    };

    let amount = scroll_amount_for_track_position(&metrics, px(100.0));

    assert_eq!(amount, 100.0);
  }
}
