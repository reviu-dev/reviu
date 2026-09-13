use std::{
  cell::{Cell, RefCell},
  rc::Rc,
};

use gpui::{Bounds, Pixels, Point, Size, point, px, size};
use gpui_component::scroll::ScrollbarHandle;

use crate::ScreenSnapshot;

#[derive(Clone, Copy, Debug)]
struct TerminalScrollState {
  total_lines: usize,
  viewport_lines: usize,
  display_offset: usize,
  line_height: Pixels,
}

#[derive(Clone)]
pub(crate) struct TerminalScrollHandle {
  state: Rc<RefCell<TerminalScrollState>>,
  pending_display_offset: Rc<Cell<Option<usize>>>,
}

impl TerminalScrollHandle {
  pub(crate) fn new() -> Self {
    Self {
      state: Rc::new(RefCell::new(TerminalScrollState {
        total_lines: 0,
        viewport_lines: 0,
        display_offset: 0,
        line_height: px(1.0),
      })),
      pending_display_offset: Rc::new(Cell::new(None)),
    }
  }

  pub(crate) fn update(&self, screen: &ScreenSnapshot, line_height: Pixels) {
    *self.state.borrow_mut() = TerminalScrollState {
      total_lines: screen.total_lines,
      viewport_lines: screen.rows,
      display_offset: screen.display_offset,
      line_height: line_height.max(px(1.0)),
    };
  }

  pub(crate) fn take_pending_display_offset(&self) -> Option<usize> {
    self.pending_display_offset.take()
  }
}

impl ScrollbarHandle for TerminalScrollHandle {
  fn viewport_bounds(&self) -> Bounds<Pixels> {
    let state = self.state.borrow();
    Bounds::new(
      point(px(0.0), px(0.0)),
      size(px(0.0), state.viewport_lines as f32 * state.line_height),
    )
  }

  fn offset(&self) -> Point<Pixels> {
    let state = self.state.borrow();
    let history_lines = state.total_lines.saturating_sub(state.viewport_lines);
    let lines_from_top = history_lines.saturating_sub(state.display_offset);
    point(px(0.0), -(lines_from_top as f32 * state.line_height))
  }

  fn set_offset(&self, offset: Point<Pixels>) {
    let state = self.state.borrow();
    let history_lines = state.total_lines.saturating_sub(state.viewport_lines);
    let lines_from_top = (-offset.y / state.line_height).round() as i32;
    let display_offset =
      history_lines.saturating_sub(lines_from_top.clamp(0, history_lines as i32) as usize);
    self.pending_display_offset.set(Some(display_offset));
  }

  fn content_size(&self) -> Size<Pixels> {
    let state = self.state.borrow();
    size(px(0.0), state.total_lines as f32 * state.line_height)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn screen(display_offset: usize) -> ScreenSnapshot {
    ScreenSnapshot {
      rows: 20,
      total_lines: 100,
      display_offset,
      ..ScreenSnapshot::default()
    }
  }

  #[test]
  fn scrollbar_maps_terminal_history_from_oldest_to_latest() {
    let handle = TerminalScrollHandle::new();
    handle.update(&screen(0), px(10.0));

    assert_eq!(handle.offset().y, px(-800.0));
    assert_eq!(handle.content_size().height, px(1000.0));

    handle.set_offset(point(px(0.0), px(0.0)));
    assert_eq!(handle.take_pending_display_offset(), Some(80));

    handle.set_offset(point(px(0.0), px(-800.0)));
    assert_eq!(handle.take_pending_display_offset(), Some(0));
  }

  #[test]
  fn scrollbar_tracks_an_intermediate_terminal_offset() {
    let handle = TerminalScrollHandle::new();
    handle.update(&screen(30), px(10.0));

    assert_eq!(handle.offset().y, px(-500.0));
  }
}
