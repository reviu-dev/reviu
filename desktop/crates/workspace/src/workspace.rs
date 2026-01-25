use gpui::{App, Context, Entity, FocusHandle, Focusable, Global, Render, Window, prelude::*};

use crate::git_page::GitPage;
use crate::settings_page::SettingsPage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspacePage {
  Git,
  Settings,
}

#[derive(Clone)]
pub(crate) struct WorkspaceRoute {
  pub page: WorkspacePage,
}

impl Default for WorkspaceRoute {
  fn default() -> Self {
    Self {
      page: WorkspacePage::Git,
    }
  }
}

impl Global for WorkspaceRoute {}

impl WorkspaceRoute {
  pub fn global(cx: &App) -> &Self {
    cx.global::<Self>()
  }

  pub fn global_mut(cx: &mut App) -> &mut Self {
    cx.global_mut::<Self>()
  }
}

pub struct WorkspaceView {
  git_page: Entity<GitPage>,
  settings_page: Entity<SettingsPage>,
}

impl WorkspaceView {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    cx.set_global(WorkspaceRoute::default());
    let git_page = cx.new(|cx| GitPage::new(window, cx));
    let settings_page = cx.new(|cx| SettingsPage::new(window, cx));

    Self {
      git_page,
      settings_page,
    }
  }
}

impl Render for WorkspaceView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    match WorkspaceRoute::global(cx).page {
      WorkspacePage::Git => self.git_page.clone().into_any_element(),
      WorkspacePage::Settings => self.settings_page.clone().into_any_element(),
    }
  }
}

impl Focusable for WorkspaceView {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    match WorkspaceRoute::global(cx).page {
      WorkspacePage::Git => self.git_page.read(cx).focus_handle(cx),
      WorkspacePage::Settings => self.settings_page.read(cx).focus_handle(cx),
    }
  }
}
