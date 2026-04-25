use gpui::{
  App, Bounds, DispatchPhase, Div, Element, ElementId, GlobalElementId, Hitbox, HitboxBehavior,
  InspectorElementId, IntoElement, LayoutId, Pixels, Point, ScrollHandle, ScrollWheelEvent,
  Stateful, Styled, Window, px,
};

const SCROLL_CHAIN_EDGE_TOLERANCE_PX: f32 = 0.5;

/// Configures which axes a [`ScrollChainGuard`] should consume from a scroll
/// wheel event before letting it bubble up to ancestor scrollers.
#[derive(Clone, Copy)]
pub struct ScrollChainAxes {
  pub horizontal: bool,
  pub vertical: bool,
  pub restrict_to_wheel_axis: bool,
}

impl ScrollChainAxes {
  /// Consume both horizontal and vertical wheel events as they arrive.
  pub fn both() -> Self {
    Self {
      horizontal: true,
      vertical: true,
      restrict_to_wheel_axis: true,
    }
  }

  /// Consume only horizontal wheel events.
  pub fn horizontal() -> Self {
    Self {
      horizontal: true,
      vertical: false,
      restrict_to_wheel_axis: true,
    }
  }

  /// Consume only vertical wheel events.
  pub fn vertical() -> Self {
    Self {
      horizontal: false,
      vertical: true,
      restrict_to_wheel_axis: true,
    }
  }
}

#[derive(Clone, Copy)]
enum ScrollChainAxis {
  Horizontal,
  Vertical,
}

/// Forces a scrollable container to only react to wheel deltas along the axis
/// the user actually scrolled, instead of treating any wheel movement as a
/// generic scroll.
pub fn restrict_scroll_to_wheel_axis(mut container: Stateful<Div>) -> Stateful<Div> {
  container.style().restrict_scroll_to_axis = Some(true);
  container
}

/// Wraps a scrollable child so that wheel events apply to it first; the wheel
/// only chains to the parent scroller when the child has reached its edge on
/// the requested axis. This avoids "double scroll" where moving the wheel
/// inside an inner scroller also scrolls an outer container.
pub fn scroll_chain_guard(
  child: Stateful<Div>,
  scroll_handle: &ScrollHandle,
  axes: ScrollChainAxes,
) -> ScrollChainGuard {
  ScrollChainGuard {
    child,
    scroll_handle: scroll_handle.clone(),
    axes,
  }
}

pub struct ScrollChainGuard {
  child: Stateful<Div>,
  scroll_handle: ScrollHandle,
  axes: ScrollChainAxes,
}

impl IntoElement for ScrollChainGuard {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for ScrollChainGuard {
  type RequestLayoutState = <Stateful<Div> as Element>::RequestLayoutState;
  type PrepaintState = (Hitbox, <Stateful<Div> as Element>::PrepaintState);

  fn id(&self) -> Option<ElementId> {
    <Stateful<Div> as Element>::id(&self.child)
  }

  fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
    <Stateful<Div> as Element>::source_location(&self.child)
  }

  fn request_layout(
    &mut self,
    id: Option<&GlobalElementId>,
    inspector_id: Option<&InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    self.child.request_layout(id, inspector_id, window, cx)
  }

  fn prepaint(
    &mut self,
    id: Option<&GlobalElementId>,
    inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    state: &mut Self::RequestLayoutState,
    window: &mut Window,
    cx: &mut App,
  ) -> Self::PrepaintState {
    let hitbox = window.insert_hitbox(bounds, HitboxBehavior::BlockMouseExceptScroll);
    let child_state = self
      .child
      .prepaint(id, inspector_id, bounds, state, window, cx);
    (hitbox, child_state)
  }

  fn paint(
    &mut self,
    id: Option<&GlobalElementId>,
    inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    request_layout: &mut Self::RequestLayoutState,
    prepaint: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
    let current_view = window.current_view();
    let hitbox = prepaint.0.clone();
    let scroll_handle = self.scroll_handle.clone();
    let axes = self.axes;
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
      if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
        return;
      }

      if scroll_handle_apply_wheel(&scroll_handle, event, window.line_height(), axes) {
        cx.notify(current_view);
        cx.stop_propagation();
      }
    });

    self.child.paint(
      id,
      inspector_id,
      bounds,
      request_layout,
      &mut prepaint.1,
      window,
      cx,
    );
  }
}

fn scroll_handle_apply_wheel(
  scroll_handle: &ScrollHandle,
  event: &ScrollWheelEvent,
  line_height: Pixels,
  axes: ScrollChainAxes,
) -> bool {
  let Some((axis, delta)) = scroll_chain_axis_delta(event.delta.pixel_delta(line_height), axes)
  else {
    return false;
  };
  let mut offset = scroll_handle.offset();
  let max_offset = scroll_handle.max_offset();

  match axis {
    ScrollChainAxis::Horizontal => {
      let Some(next_x) = scroll_axis_next_offset(offset.x, max_offset.x, delta) else {
        return false;
      };
      offset.x = next_x;
    }
    ScrollChainAxis::Vertical => {
      let Some(next_y) = scroll_axis_next_offset(offset.y, max_offset.y, delta) else {
        return false;
      };
      offset.y = next_y;
    }
  }

  scroll_handle.set_offset(offset);
  true
}

fn scroll_chain_axis_delta(
  delta: Point<Pixels>,
  axes: ScrollChainAxes,
) -> Option<(ScrollChainAxis, Pixels)> {
  let mut delta_x = px(0.0);
  if axes.horizontal {
    if delta.x != px(0.0) {
      delta_x = delta.x;
    } else if !axes.restrict_to_wheel_axis && !axes.vertical {
      delta_x = delta.y;
    }
  }

  let mut delta_y = px(0.0);
  if axes.vertical {
    if delta.y != px(0.0) {
      delta_y = delta.y;
    } else if !axes.restrict_to_wheel_axis && !axes.horizontal {
      delta_y = delta.x;
    }
  }

  match (delta_x != px(0.0), delta_y != px(0.0)) {
    (true, true) if delta_x.abs() > delta_y.abs() => Some((ScrollChainAxis::Horizontal, delta_x)),
    (true, true) => Some((ScrollChainAxis::Vertical, delta_y)),
    (true, false) => Some((ScrollChainAxis::Horizontal, delta_x)),
    (false, true) => Some((ScrollChainAxis::Vertical, delta_y)),
    (false, false) => None,
  }
}

fn scroll_axis_next_offset(offset: Pixels, max_offset: Pixels, delta: Pixels) -> Option<Pixels> {
  let edge_tolerance = px(SCROLL_CHAIN_EDGE_TOLERANCE_PX);

  if max_offset <= edge_tolerance {
    return None;
  }

  if delta < px(0.0) {
    (offset > -max_offset + edge_tolerance).then_some((offset + delta).max(-max_offset))
  } else if delta > px(0.0) {
    (offset < -edge_tolerance).then_some((offset + delta).min(px(0.0)))
  } else {
    None
  }
}
