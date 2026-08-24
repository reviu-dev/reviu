//! Reaching the workspace window from an `&mut App`, for work that arrives
//! outside any window: deep links, and callbacks that only carry the app.

use gpui::{AnyWindowHandle, App, AppContext as _, Global, Window};

pub(crate) struct WorkspaceWindow {
  window: Option<AnyWindowHandle>,
}

impl Global for WorkspaceWindow {}

impl WorkspaceWindow {
  pub(crate) fn register(window: AnyWindowHandle, cx: &mut App) {
    cx.set_global(Self {
      window: Some(window),
    });
  }

  pub(crate) fn with_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    let Some(window) = cx.try_global::<Self>().and_then(|this| this.window) else {
      return;
    };
    // Deferred: callers often sit inside this very window's update, where a
    // re-entrant `update_window` is a silent no-op.
    cx.defer(move |cx| {
      let _ = cx.update_window(window, |_, window, cx| f(window, cx));
    });
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::{Context, IntoElement, Render, TestAppContext, div};
  use std::cell::Cell;
  use std::rc::Rc;

  struct Page;

  impl Render for Page {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
      div()
    }
  }

  #[gpui::test]
  fn work_without_a_window_of_its_own_reaches_the_registered_one(cx: &mut TestAppContext) {
    let (_root, cx) = cx.add_window_view(|_window, _cx| Page);
    let handle = cx.update(|window, _| window.window_handle());
    cx.update(|_, cx| WorkspaceWindow::register(handle, cx));

    let reached = Rc::new(Cell::new(false));
    let flag = reached.clone();
    let observed_inside = reached.clone();
    cx.update(|_, cx| {
      WorkspaceWindow::with_window(cx, move |_, _| flag.set(true));
      assert!(
        !observed_inside.get(),
        "a re-entrant update_window would be a silent no-op, so it waits"
      );
    });

    cx.run_until_parked();
    assert!(reached.get());
  }

  #[gpui::test]
  fn nothing_happens_when_no_window_was_registered(cx: &mut TestAppContext) {
    let reached = Rc::new(Cell::new(false));
    let flag = reached.clone();
    cx.update(|cx| {
      WorkspaceWindow::with_window(cx, move |_, _| flag.set(true));
    });
    cx.run_until_parked();

    assert!(!reached.get());
  }
}
