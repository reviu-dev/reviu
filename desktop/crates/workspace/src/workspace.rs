use gpui::{App, Context, Entity, FocusHandle, Focusable, Global, Render, Window, prelude::*};

use crate::api::ApiClient;
use crate::git_page::GitPage;
use crate::github_page::GithubPage;
use crate::github_pr_details_page::GithubPrDetailsPage;
use crate::settings_page::SettingsPage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspacePage {
  Git,
  Github,
  GithubPrDetails,
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

#[derive(Clone)]
pub struct WorkspaceApi {
  pub api: ApiClient,
}

impl Global for WorkspaceApi {}

impl WorkspaceApi {
  pub fn new() -> Self {
    Self {
      api: ApiClient::new(),
    }
  }

  pub fn global(cx: &App) -> &Self {
    cx.global::<Self>()
  }
}

pub struct WorkspaceView {
  git_page: Entity<GitPage>,
  github_page: Entity<GithubPage>,
  github_pr_details_page: Entity<GithubPrDetailsPage>,
  settings_page: Entity<SettingsPage>,
}

impl WorkspaceView {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    cx.set_global(WorkspaceRoute::default());
    cx.set_global(WorkspaceApi::new());
    let git_page = cx.new(|cx| GitPage::new(window, cx));
    let github_page = cx.new(|cx| GithubPage::new(window, cx));
    let github_pr_details_page = cx.new(|cx| GithubPrDetailsPage::new(window, cx));
    let settings_page = cx.new(|cx| SettingsPage::new(window, cx));

    Self {
      git_page,
      github_page,
      github_pr_details_page,
      settings_page,
    }
  }
}

impl Render for WorkspaceView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    match WorkspaceRoute::global(cx).page {
      WorkspacePage::Git => self.git_page.clone().into_any_element(),
      WorkspacePage::Github => self.github_page.clone().into_any_element(),
      WorkspacePage::GithubPrDetails => self.github_pr_details_page.clone().into_any_element(),
      WorkspacePage::Settings => self.settings_page.clone().into_any_element(),
    }
  }
}

impl Focusable for WorkspaceView {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    match WorkspaceRoute::global(cx).page {
      WorkspacePage::Git => self.git_page.read(cx).focus_handle(cx),
      WorkspacePage::Github => self.github_page.read(cx).focus_handle(cx),
      WorkspacePage::GithubPrDetails => self.github_pr_details_page.read(cx).focus_handle(cx),
      WorkspacePage::Settings => self.settings_page.read(cx).focus_handle(cx),
    }
  }
}
