//! Renders the SVG being viewed in a diff to an image, off the main thread.

use std::sync::Arc;

use editor::Editor;
use gpui::{
  AnyElement, App, Context, Entity, RenderImage, SharedString, Task, Window, div, img, prelude::*,
  px,
};
use gpui_component::ActiveTheme as _;
use ui::StatusThemeExt as _;

#[derive(Default)]
pub(crate) struct SvgPreview {
  image: Option<Result<Arc<RenderImage>, SharedString>>,
  source: Option<SharedString>,
  task: Option<Task<()>>,
}

impl SvgPreview {
  pub(crate) fn new() -> Self {
    Self::default()
  }

  pub(crate) fn clear(&mut self) {
    self.image = None;
    self.source = None;
    self.task = None;
  }

  /// Re-renders when the editor content changed since the last frame.
  pub(crate) fn refresh_from_editor(
    &mut self,
    editor: &Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let source: SharedString = {
      let document = editor.read(cx).document().read(cx);
      document.slice_to_string(0..document.len()).into()
    };

    if self.source.as_ref() == Some(&source) {
      return;
    }

    self.source = Some(source.clone());
    let renderer = cx.svg_renderer();
    let svg_bytes = source.as_ref().as_bytes().to_vec();
    let background =
      cx.background_spawn(async move { renderer.render_single_frame(svg_bytes.as_slice(), 1.0) });

    self.task = Some(cx.spawn_in(window, async move |this, cx| {
      let result = background.await;
      let _ = this.update_in(cx, |this, window, cx| {
        if let Some(Ok(image)) = this.image.take() {
          let _ = window.drop_image(image);
        }
        this.image = Some(result.map_err(|err| err.to_string().into()));
        cx.notify();
      });
    }));
  }

  pub(crate) fn render(&self, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let preview = match self.image.clone() {
      Some(Ok(image)) => img(image).max_w_full().max_h_full().into_any_element(),
      Some(Err(error)) => div()
        .text_sm()
        .text_color(theme.status_red())
        .child(error)
        .into_any_element(),
      None => div()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("Rendering SVG preview...")
        .into_any_element(),
    };

    div()
      .flex_1()
      .min_h_0()
      .min_w(px(0.0))
      .bg(theme.background)
      .occlude()
      .child(
        div()
          .flex_1()
          .min_h_0()
          .min_w(px(0.0))
          .p_4()
          .items_center()
          .justify_center()
          .child(preview),
      )
      .into_any_element()
  }
}
