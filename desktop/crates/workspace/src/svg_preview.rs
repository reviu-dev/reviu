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

  #[cfg(test)]
  pub(crate) fn rendered_image(&self) -> Option<&Result<Arc<RenderImage>, SharedString>> {
    self.image.as_ref()
  }

  #[cfg(test)]
  pub(crate) fn has_pending_render(&self) -> bool {
    self.task.is_some()
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;

  const SQUARE: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\"><rect width=\"8\" height=\"8\"/></svg>";
  const CIRCLE: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\"><circle cx=\"4\" cy=\"4\" r=\"3\"/></svg>";

  /// A repository holding `logo.svg`, with an editor on it and a preview beside.
  fn add_preview_window<'a>(
    contents: &str,
    cx: &'a mut TestAppContext,
  ) -> (
    Entity<SvgPreview>,
    Entity<Editor>,
    TempRepo,
    &'a mut gpui::VisualTestContext,
  ) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("svg-preview");
    commit_text_file(&repo.path, Path::new("logo.svg"), contents, "initial");
    let repo_root = repo.path.clone();

    let (editor, cx) = cx.add_window_view(|_window, cx| {
      Editor::new_with_paths(repo_root.clone(), repo_root.join("logo.svg"), cx)
    });
    let preview = cx.new(|_| SvgPreview::new());
    (preview, editor, repo, cx)
  }

  #[gpui::test]
  async fn the_preview_renders_the_editor_content_and_follows_its_changes(cx: &mut TestAppContext) {
    let (preview, editor, repo, cx) = add_preview_window(SQUARE, cx);
    cx.executor().allow_parking();

    preview.update_in(cx, |preview, window, cx| {
      preview.refresh_from_editor(&editor, window, cx)
    });
    cx.run_until_parked();

    preview.read_with(cx, |preview, _| {
      assert!(
        matches!(preview.rendered_image(), Some(Ok(_))),
        "the SVG source becomes an image"
      );
    });

    // Same source: nothing to redo.
    preview.update_in(cx, |preview, window, cx| {
      preview.refresh_from_editor(&editor, window, cx);
      assert!(
        matches!(preview.rendered_image(), Some(Ok(_))),
        "the image stays while the source is unchanged"
      );
    });

    // A new source replaces the image.
    editor.update(cx, |editor, cx| {
      editor.load_readonly_snapshot(CIRCLE.to_string(), None, cx)
    });
    preview.update_in(cx, |preview, window, cx| {
      preview.refresh_from_editor(&editor, window, cx)
    });
    cx.run_until_parked();
    preview.read_with(cx, |preview, _| {
      assert!(matches!(preview.rendered_image(), Some(Ok(_))));
    });
    drop(repo);
  }

  #[gpui::test]
  async fn clearing_drops_the_image_and_the_render_in_flight(cx: &mut TestAppContext) {
    let (preview, editor, repo, cx) = add_preview_window(SQUARE, cx);
    cx.executor().allow_parking();

    preview.update_in(cx, |preview, window, cx| {
      preview.refresh_from_editor(&editor, window, cx);
      // Leaving the file while the render runs must kill it: a late frame would
      // paint the previous file's image.
      assert!(preview.has_pending_render());
      preview.clear();
      assert!(!preview.has_pending_render());
    });
    cx.run_until_parked();

    preview.read_with(cx, |preview, _| {
      assert!(preview.rendered_image().is_none());
    });
    drop(repo);
  }
}
