use gpui::{
  App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Window, div, prelude::*,
};
use workspace::WorkspaceView;

pub struct AppRoot {
  view: Entity<WorkspaceView>,
  focus_handle: FocusHandle,
}

impl AppRoot {
  pub fn new(view: Entity<WorkspaceView>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
    Self {
      view,
      focus_handle: cx.focus_handle(),
    }
  }
}

impl Focusable for AppRoot {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for AppRoot {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    div().relative().size_full().child(self.view.clone())
  }
}
