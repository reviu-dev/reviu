use gpui::{
  App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
  Render, Styled, Window, div,
};
use terminal::TerminalView;

use crate::active_local_repo::ActiveLocalRepoStore;

pub struct TerminalPage {
  terminal_view: Entity<TerminalView>,
  focus_handle: FocusHandle,
}

impl TerminalPage {
  pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
    let working_directory = ActiveLocalRepoStore::get(cx).map(|repo| repo.repo_root);
    let terminal_view = cx.new(|cx| TerminalView::new(working_directory, cx));

    Self {
      terminal_view,
      focus_handle: cx.focus_handle(),
    }
  }
}

impl Focusable for TerminalPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for TerminalPage {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let working_directory = ActiveLocalRepoStore::get(cx).map(|repo| repo.repo_root);
    let terminal_view = self.terminal_view.clone();

    terminal_view.update(cx, |view, cx| {
      view.set_working_directory(working_directory, cx);
    });

    div().size_full().child(self.terminal_view.clone())
  }
}
