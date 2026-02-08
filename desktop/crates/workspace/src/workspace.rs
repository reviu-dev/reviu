use gpui::{App, Context, Entity, FocusHandle, Focusable, Global, Render, Window, prelude::*};

use crate::api::ApiClient;
use crate::auth_state::AuthStateStore;
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
  pub settings_return: Option<WorkspacePage>,
}

impl Default for WorkspaceRoute {
  fn default() -> Self {
    Self {
      page: WorkspacePage::Git,
      settings_return: None,
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

  pub fn open_settings(cx: &mut App) {
    let current = cx.global::<Self>().page;
    let route = cx.global_mut::<Self>();
    if route.page != WorkspacePage::Settings {
      route.settings_return = Some(current);
    }
    route.page = WorkspacePage::Settings;
  }

  pub fn close_settings(cx: &mut App) {
    let route = cx.global_mut::<Self>();
    let target = route.settings_return.take().unwrap_or(WorkspacePage::Git);
    route.page = target;
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
  last_page: Option<WorkspacePage>,
}

impl WorkspaceView {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    cx.set_global(WorkspaceRoute::default());
    cx.set_global(WorkspaceApi::new());
    cx.set_global(AuthStateStore::default());
    let git_page = cx.new(|cx| GitPage::new(window, cx));
    let github_page = cx.new(|cx| GithubPage::new(window, cx));
    let github_pr_details_page = cx.new(|cx| GithubPrDetailsPage::new(window, cx));
    let settings_page = cx.new(|cx| SettingsPage::new(window, cx));

    Self {
      git_page,
      github_page,
      github_pr_details_page,
      settings_page,
      last_page: None,
    }
  }
}

impl Render for WorkspaceView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let page = WorkspaceRoute::global(cx).page;
    if self.last_page != Some(page) {
      self.last_page = Some(page);
      let focus_handle = self.focus_handle(cx);
      window.focus(&focus_handle, cx);
    }

    match page {
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
