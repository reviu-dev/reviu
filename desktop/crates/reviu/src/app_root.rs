use gpui::{
  App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Window, div, prelude::*,
};
use gpui_component::Root;
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
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let sheet_layer = Root::render_sheet_layer(window, cx);
    let dialog_layer = Root::render_dialog_layer(window, cx);
    let notification_layer = Root::render_notification_layer(window, cx);

    div()
      .size_full()
      .child(self.view.clone())
      .children(sheet_layer)
      .children(dialog_layer)
      .children(notification_layer.map(gpui::deferred))
  }
}
