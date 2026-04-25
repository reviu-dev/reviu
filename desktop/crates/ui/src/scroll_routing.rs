use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use gpui::{
  App, Bounds, DispatchPhase, Div, Element, ElementId, GlobalElementId, InspectorElementId,
  IntoElement, LayoutId, Pixels, Point, ScrollHandle, ScrollWheelEvent, Stateful, Styled, Window,
  px,
};

const EDGE_TOLERANCE_PX: f32 = 0.5;

/// How long a wheel gesture stays latched on its target. Continuous bursts
/// arrive much faster than this; the latch only releases when the user
/// actually pauses, at which point the next event re-hit-tests under the
/// cursor — same as how a browser routes scrolls.
const GESTURE_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy)]
pub struct ScrollAxes {
  pub horizontal: bool,
  pub vertical: bool,
  pub restrict_to_wheel_axis: bool,
}

impl ScrollAxes {
  pub fn both() -> Self {
    Self {
      horizontal: true,
      vertical: true,
      restrict_to_wheel_axis: true,
    }
  }

  pub fn horizontal() -> Self {
    Self {
      horizontal: true,
      vertical: false,
      restrict_to_wheel_axis: true,
    }
  }

  pub fn vertical() -> Self {
    Self {
      horizontal: false,
      vertical: true,
      restrict_to_wheel_axis: true,
    }
  }
}

#[derive(Clone, Copy)]
enum ScrollAxis {
  Horizontal,
  Vertical,
}

/// Forces a scrollable container to only react to wheel deltas along the axis
/// the user actually scrolled, instead of treating any wheel movement as a
/// generic scroll. Useful with overflow_scroll() containers.
pub fn restrict_scroll_to_wheel_axis(mut container: Stateful<Div>) -> Stateful<Div> {
  container.style().restrict_scroll_to_axis = Some(true);
  container
}

// -- Registry & dispatcher state ---------------------------------------------

#[derive(Clone)]
struct ScrollNode {
  id: u64,
  bounds: Bounds<Pixels>,
  axes: ScrollAxes,
  handle: ScrollHandle,
}

#[derive(Clone, Copy)]
enum LatchedTarget {
  /// No active gesture (or the previous gesture timed out).
  None,
  /// The dispatcher is routing wheel events to this inner scrollable.
  Inner(u64),
  /// The dispatcher decided not to claim wheel events for this gesture; let
  /// them propagate to whatever ancestor handles them (typically the page).
  Outer,
}

#[derive(Clone, Copy)]
struct DispatcherState {
  target: LatchedTarget,
  last_event_at: Option<Instant>,
}

impl DispatcherState {
  const fn empty() -> Self {
    Self {
      target: LatchedTarget::None,
      last_event_at: None,
    }
  }
}

thread_local! {
  /// Rebuilt each frame as scroll-aware nodes paint themselves. Innermost
  /// nodes end up at the back of the vec (depth-first paint order).
  static REGISTRY: RefCell<Vec<ScrollNode>> = RefCell::new(Vec::new());
  static DISPATCHER: Cell<DispatcherState> = const { Cell::new(DispatcherState::empty()) };
}

fn registry_clear() {
  REGISTRY.with(|r| r.borrow_mut().clear());
}

fn registry_push(node: ScrollNode) {
  REGISTRY.with(|r| r.borrow_mut().push(node));
}

fn registry_find_by_id(id: u64) -> Option<ScrollNode> {
  REGISTRY.with(|r| r.borrow().iter().find(|n| n.id == id).cloned())
}

/// Walk the registry from innermost (last pushed) to outermost and return the
/// first node whose bounds contain `cursor` and that can consume `delta` along
/// `axis`. If no node is hit, the wheel falls through to whatever ancestor
/// scroller GPUI's overflow_*_scroll mechanism dispatches to.
fn registry_hit_test(cursor: Point<Pixels>, axis: ScrollAxis, delta: Pixels) -> Option<ScrollNode> {
  REGISTRY.with(|r| {
    let nodes = r.borrow();
    nodes
      .iter()
      .rev()
      .find(|node| node.bounds.contains(&cursor) && node_can_scroll(node, axis, delta))
      .cloned()
  })
}

fn node_can_scroll(node: &ScrollNode, axis: ScrollAxis, delta: Pixels) -> bool {
  match axis {
    ScrollAxis::Horizontal => {
      if !node.axes.horizontal {
        return false;
      }
      let offset = node.handle.offset().x;
      let max_offset = node.handle.max_offset().x;
      can_scroll_axis(offset, max_offset, delta)
    }
    ScrollAxis::Vertical => {
      if !node.axes.vertical {
        return false;
      }
      let offset = node.handle.offset().y;
      let max_offset = node.handle.max_offset().y;
      can_scroll_axis(offset, max_offset, delta)
    }
  }
}

fn can_scroll_axis(offset: Pixels, max_offset: Pixels, delta: Pixels) -> bool {
  let edge_tolerance = px(EDGE_TOLERANCE_PX);
  if max_offset <= edge_tolerance {
    return false;
  }
  if delta < px(0.0) {
    offset > -max_offset + edge_tolerance
  } else if delta > px(0.0) {
    offset < -edge_tolerance
  } else {
    false
  }
}

fn scroll_axis_next_offset(offset: Pixels, max_offset: Pixels, delta: Pixels) -> Option<Pixels> {
  let edge_tolerance = px(EDGE_TOLERANCE_PX);

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

/// Pick the dominant axis from a wheel delta and return the signed amount we
/// should apply along it. Returns `None` if the event has no movement on the
/// requested axes.
fn pick_axis(delta: Point<Pixels>, axes: ScrollAxes) -> Option<(ScrollAxis, Pixels)> {
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
    (true, true) if delta_x.abs() > delta_y.abs() => Some((ScrollAxis::Horizontal, delta_x)),
    (true, true) => Some((ScrollAxis::Vertical, delta_y)),
    (true, false) => Some((ScrollAxis::Horizontal, delta_x)),
    (false, true) => Some((ScrollAxis::Vertical, delta_y)),
    (false, false) => None,
  }
}

fn apply_to_node(node: &ScrollNode, axis: ScrollAxis, delta: Pixels) {
  let mut offset = node.handle.offset();
  let max_offset = node.handle.max_offset();
  match axis {
    ScrollAxis::Horizontal => {
      if let Some(next) = scroll_axis_next_offset(offset.x, max_offset.x, delta) {
        offset.x = next;
      }
    }
    ScrollAxis::Vertical => {
      if let Some(next) = scroll_axis_next_offset(offset.y, max_offset.y, delta) {
        offset.y = next;
      }
    }
  }
  node.handle.set_offset(offset);
}

// -- Public element wrappers -------------------------------------------------

/// Wraps a scrollable child into a node the central [`ScrollDispatcher`] knows
/// about. The wrapper itself does not listen to wheel events; the dispatcher
/// hit-tests the registered nodes, picks one to latch onto for the gesture,
/// and routes wheel events to it.
///
/// `id` must be stable across renders (e.g. derived from a long-lived scroll
/// handle key or an entity-scoped counter); it's how the dispatcher remembers
/// the latched target between events.
pub fn scrollable_node(
  child: Stateful<Div>,
  scroll_handle: &ScrollHandle,
  axes: ScrollAxes,
  id: u64,
) -> ScrollableNode {
  ScrollableNode {
    child,
    handle: scroll_handle.clone(),
    axes,
    id,
  }
}

pub struct ScrollableNode {
  child: Stateful<Div>,
  handle: ScrollHandle,
  axes: ScrollAxes,
  id: u64,
}

impl IntoElement for ScrollableNode {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for ScrollableNode {
  type RequestLayoutState = <Stateful<Div> as Element>::RequestLayoutState;
  type PrepaintState = <Stateful<Div> as Element>::PrepaintState;

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
    self
      .child
      .prepaint(id, inspector_id, bounds, state, window, cx)
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
    registry_push(ScrollNode {
      id: self.id,
      bounds,
      axes: self.axes,
      handle: self.handle.clone(),
    });
    self.child.paint(
      id,
      inspector_id,
      bounds,
      request_layout,
      prepaint,
      window,
      cx,
    );
  }
}

/// Marker element that owns the centralized scroll dispatcher. Mount it once
/// near the root of the window — it must paint **before** any
/// [`scrollable_node`] so the registry is empty when nodes start to
/// register themselves.
///
/// The dispatcher resets the registry on each paint, then installs a single
/// window-level wheel listener that hit-tests the registry, latches onto a
/// target for the duration of the gesture, and routes events to it.
pub fn scroll_dispatcher() -> ScrollDispatcher {
  ScrollDispatcher
}

pub struct ScrollDispatcher;

impl IntoElement for ScrollDispatcher {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for ScrollDispatcher {
  type RequestLayoutState = ();
  type PrepaintState = ();

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
    // Reset the registry at the start of each frame, BEFORE the rest of the
    // tree paints. Inner scroll-chain guards will repopulate it as they paint.
    registry_clear();
    let layout_id = window.request_layout(gpui::Style::default(), [], cx);
    (layout_id, ())
  }

  fn prepaint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    _bounds: Bounds<Pixels>,
    _state: &mut Self::RequestLayoutState,
    _window: &mut Window,
    _cx: &mut App,
  ) -> Self::PrepaintState {
  }

  fn paint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    _bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    _prepaint: &mut Self::PrepaintState,
    window: &mut Window,
    _cx: &mut App,
  ) {
    let current_view = window.current_view();
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
      if phase != DispatchPhase::Capture {
        return;
      }
      let line_height = window.line_height();
      let pixel_delta = event.delta.pixel_delta(line_height);
      // Use a generous default for axis selection; per-node axes act as a
      // filter in `node_can_scroll`.
      let Some((axis, delta)) = pick_axis(pixel_delta, ScrollAxes::both()) else {
        return;
      };

      let now = Instant::now();
      let mut state = DISPATCHER.with(|cell| cell.get());
      let in_gesture = state
        .last_event_at
        .is_some_and(|prev| now.saturating_duration_since(prev) <= GESTURE_TIMEOUT);
      if !in_gesture {
        state.target = LatchedTarget::None;
      }

      let target = match state.target {
        LatchedTarget::Inner(id) => registry_find_by_id(id).map(LatchResolution::Inner),
        LatchedTarget::Outer => Some(LatchResolution::Outer),
        LatchedTarget::None => match registry_hit_test(event.position, axis, delta) {
          Some(node) => {
            state.target = LatchedTarget::Inner(node.id);
            Some(LatchResolution::Inner(node))
          }
          None => {
            state.target = LatchedTarget::Outer;
            Some(LatchResolution::Outer)
          }
        },
      };

      state.last_event_at = Some(now);
      DISPATCHER.with(|cell| cell.set(state));

      match target {
        Some(LatchResolution::Inner(node)) => {
          // Mid-gesture containment: keep swallowing the wheel even when the
          // inner scroller has hit its edge, so a single swipe's inertia
          // doesn't leak to the parent. The 250ms gesture timeout takes care
          // of releasing the latch — the next gesture started while still at
          // the edge will re-hit-test, get rejected, and chain to the page.
          apply_to_node(&node, axis, delta);
          cx.notify(current_view);
          cx.stop_propagation();
        }
        Some(LatchResolution::Outer) | None => {
          // Don't claim — let GPUI's native overflow_*_scroll on the ancestor
          // pick up the wheel event normally.
        }
      }
    });
  }
}

enum LatchResolution {
  Inner(ScrollNode),
  Outer,
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::point;

  // -- can_scroll_axis ------------------------------------------------------

  #[test]
  fn can_scroll_axis_at_top_blocks_upward_and_allows_downward() {
    // offset = 0 means we're at the top of the scrollable. Negative delta
    // (scroll down → content moves up → offset decreases) should be allowed;
    // positive delta (scroll up at top) should be blocked.
    let max = px(200.0);
    assert!(can_scroll_axis(px(0.0), max, px(-10.0)));
    assert!(!can_scroll_axis(px(0.0), max, px(10.0)));
  }

  #[test]
  fn can_scroll_axis_at_bottom_blocks_downward_and_allows_upward() {
    let max = px(200.0);
    let bottom = px(-200.0);
    assert!(!can_scroll_axis(bottom, max, px(-10.0)));
    assert!(can_scroll_axis(bottom, max, px(10.0)));
  }

  #[test]
  fn can_scroll_axis_in_middle_allows_both_directions() {
    let max = px(200.0);
    let middle = px(-100.0);
    assert!(can_scroll_axis(middle, max, px(-10.0)));
    assert!(can_scroll_axis(middle, max, px(10.0)));
  }

  #[test]
  fn can_scroll_axis_with_no_overflow_always_returns_false() {
    // Content fits the viewport (max_offset is essentially zero) — there's
    // nothing to scroll, in either direction.
    assert!(!can_scroll_axis(px(0.0), px(0.0), px(-10.0)));
    assert!(!can_scroll_axis(px(0.0), px(0.0), px(10.0)));
    // Even within the edge tolerance window.
    assert!(!can_scroll_axis(px(0.0), px(0.4), px(-10.0)));
  }

  #[test]
  fn can_scroll_axis_zero_delta_returns_false() {
    assert!(!can_scroll_axis(px(-50.0), px(200.0), px(0.0)));
  }

  #[test]
  fn can_scroll_axis_respects_edge_tolerance() {
    // Within the 0.5px tolerance of the bottom — treated as already at the
    // edge so we don't keep dispatching tiny deltas that don't move anything.
    let max = px(200.0);
    assert!(!can_scroll_axis(px(-199.9), max, px(-10.0)));
    assert!(!can_scroll_axis(px(-0.4), max, px(10.0)));
  }

  // -- scroll_axis_next_offset ---------------------------------------------

  #[test]
  fn scroll_axis_next_offset_clamps_at_bottom() {
    // Scrolling further than the remaining room saturates at -max_offset.
    let max = px(200.0);
    let next = scroll_axis_next_offset(px(-195.0), max, px(-50.0));
    assert_eq!(next, Some(px(-200.0)));
  }

  #[test]
  fn scroll_axis_next_offset_clamps_at_top() {
    let max = px(200.0);
    let next = scroll_axis_next_offset(px(-5.0), max, px(50.0));
    assert_eq!(next, Some(px(0.0)));
  }

  #[test]
  fn scroll_axis_next_offset_at_edge_returns_none() {
    let max = px(200.0);
    // Already at the bottom and trying to scroll further down → can't move.
    assert_eq!(scroll_axis_next_offset(px(-200.0), max, px(-10.0)), None);
    // Already at the top and trying to scroll further up.
    assert_eq!(scroll_axis_next_offset(px(0.0), max, px(10.0)), None);
  }

  #[test]
  fn scroll_axis_next_offset_in_middle_applies_full_delta() {
    let max = px(200.0);
    let next = scroll_axis_next_offset(px(-50.0), max, px(-30.0));
    assert_eq!(next, Some(px(-80.0)));
  }

  // -- pick_axis -----------------------------------------------------------

  #[test]
  fn pick_axis_prefers_dominant_axis_when_both_present() {
    let axes = ScrollAxes::both();
    // Vertical delta is bigger → vertical wins.
    let (axis, delta) = pick_axis(point(px(5.0), px(-30.0)), axes).unwrap();
    assert!(matches!(axis, ScrollAxis::Vertical));
    assert_eq!(delta, px(-30.0));
    // Horizontal bigger.
    let (axis, delta) = pick_axis(point(px(-30.0), px(5.0)), axes).unwrap();
    assert!(matches!(axis, ScrollAxis::Horizontal));
    assert_eq!(delta, px(-30.0));
  }

  #[test]
  fn pick_axis_returns_none_for_zero_delta() {
    assert!(pick_axis(point(px(0.0), px(0.0)), ScrollAxes::both()).is_none());
  }

  #[test]
  fn pick_axis_filters_disabled_axis() {
    // Only vertical is allowed; a purely horizontal delta should be ignored.
    let axes = ScrollAxes::vertical();
    assert!(pick_axis(point(px(-50.0), px(0.0)), axes).is_none());
    let (axis, delta) = pick_axis(point(px(-50.0), px(-10.0)), axes).unwrap();
    assert!(matches!(axis, ScrollAxis::Vertical));
    assert_eq!(delta, px(-10.0));
  }

  #[test]
  fn pick_axis_horizontal_only_picks_horizontal_for_x_delta() {
    let axes = ScrollAxes::horizontal();
    let (axis, delta) = pick_axis(point(px(-15.0), px(0.0)), axes).unwrap();
    assert!(matches!(axis, ScrollAxis::Horizontal));
    assert_eq!(delta, px(-15.0));
    // Pure vertical movement on a horizontal-only scroller is dropped.
    assert!(pick_axis(point(px(0.0), px(-15.0)), axes).is_none());
  }

  #[test]
  fn pick_axis_restrict_to_wheel_axis_blocks_cross_axis_fallback() {
    // restrict_to_wheel_axis = true means we don't repurpose a Y wheel
    // delta as horizontal scroll on a horizontal-only scroller — the user
    // didn't actually scroll horizontally.
    let axes = ScrollAxes::horizontal();
    assert!(pick_axis(point(px(0.0), px(-20.0)), axes).is_none());
    // With it disabled the Y delta could fall through to X.
    let permissive = ScrollAxes {
      horizontal: true,
      vertical: false,
      restrict_to_wheel_axis: false,
    };
    let (axis, delta) = pick_axis(point(px(0.0), px(-20.0)), permissive).unwrap();
    assert!(matches!(axis, ScrollAxis::Horizontal));
    assert_eq!(delta, px(-20.0));
  }
}
