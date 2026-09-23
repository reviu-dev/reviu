use std::path::PathBuf;

use gpui::{Context, IntoElement, Render, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CenterFileDragMode {
  File,
  Diff,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CenterFileDrag {
  pub(crate) path: PathBuf,
  pub(crate) mode: CenterFileDragMode,
}

impl Render for CenterFileDrag {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    gpui::Empty
  }
}
