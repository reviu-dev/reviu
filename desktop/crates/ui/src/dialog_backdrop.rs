use gpui::{App, IntoElement, Styled, Window, div};
use gpui_component::{ActiveTheme as _, WindowExt as _};

pub fn render_dialog_backdrop(
  window: &mut Window,
  cx: &mut App,
) -> Option<impl IntoElement + use<>> {
  window
    .has_active_dialog(cx)
    .then(|| div().absolute().inset_0().bg(cx.theme().overlay))
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::{Context, Render, TestAppContext, prelude::*};
  use gpui_component::Root;

  struct Host;

  impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
      div()
    }
  }

  #[gpui::test]
  fn backdrop_follows_active_dialog_state(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (_, cx) = cx.add_window_view(|window, cx| {
      let host = cx.new(|_| Host);
      Root::new(host, window, cx)
    });

    cx.update(|window, cx| assert!(render_dialog_backdrop(window, cx).is_none()));
    cx.update(|window, cx| {
      window.open_dialog(cx, |dialog, _, _| dialog.child(div()));
      assert!(render_dialog_backdrop(window, cx).is_some());
    });
  }
}
