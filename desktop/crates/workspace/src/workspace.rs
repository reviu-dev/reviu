use gpui::{
  App, Context, Entity, FocusHandle, Focusable, Global, Render, Subscription, Window, prelude::*,
};
use gpui_component::{ActiveTheme as _, Theme, ThemeMode};

use crate::api::ApiClient;
use crate::auth_state::AuthStateStore;
use crate::config::{AppSettings as PersistedSettings, ConfigStore};
use crate::git_page::GitPage;
use crate::git_config_page::GitConfigPage;
use crate::github_page::GithubPage;
use crate::github_pr_details_page::GithubPrDetailsPage;
use crate::settings_page::SettingsPage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspacePage {
  Git,
  Github,
  GithubPrDetails,
  GitConfig,
  Settings,
}

#[derive(Clone)]
pub(crate) struct WorkspaceRoute {
  pub page: WorkspacePage,
  pub settings_return: Option<WorkspacePage>,
  pub git_config_return: Option<WorkspacePage>,
}

impl Default for WorkspaceRoute {
  fn default() -> Self {
    Self {
      page: WorkspacePage::Git,
      settings_return: None,
      git_config_return: None,
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

  pub fn open_git_config(cx: &mut App) {
    let current = cx.global::<Self>().page;
    let route = cx.global_mut::<Self>();
    if route.page != WorkspacePage::GitConfig {
      route.git_config_return = Some(current);
    }
    route.page = WorkspacePage::GitConfig;
  }

  pub fn close_git_config(cx: &mut App) {
    let route = cx.global_mut::<Self>();
    let target = route.git_config_return.take().unwrap_or(WorkspacePage::Git);
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
  git_config_page: Entity<GitConfigPage>,
  github_page: Entity<GithubPage>,
  github_pr_details_page: Entity<GithubPrDetailsPage>,
  settings_page: Entity<SettingsPage>,
  last_page: Option<WorkspacePage>,
  _subscriptions: Vec<Subscription>,
}

impl WorkspaceView {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    cx.set_global(WorkspaceRoute::default());
    cx.set_global(WorkspaceApi::new());
    cx.set_global(AuthStateStore::default());

    let settings = ConfigStore::load_app_settings();
    if settings.auto_switch_theme {
      Theme::sync_system_appearance(Some(window), cx);
    } else {
      let mode = if settings.dark_mode {
        ThemeMode::Dark
      } else {
        ThemeMode::Light
      };
      Theme::change(mode, Some(window), cx);
    }

    let git_page = cx.new(|cx| GitPage::new(window, cx));
    let git_config_page = cx.new(|cx| GitConfigPage::new(window, cx));
    let github_page = cx.new(|cx| GithubPage::new(window, cx));
    let github_pr_details_page = cx.new(|cx| GithubPrDetailsPage::new(window, cx));
    let settings_page = cx.new(|cx| SettingsPage::new(window, cx, settings));

    let view = Self {
      git_page,
      git_config_page,
      github_page,
      github_pr_details_page,
      settings_page,
      last_page: None,
      _subscriptions: Vec::new(),
    };

    let mut view = view;
    let subscription = cx.observe_window_appearance(window, |this, window, cx| {
      this.on_window_appearance_changed(window, cx);
    });
    view._subscriptions.push(subscription);

    view
  }

  fn on_window_appearance_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.settings_page.read(cx).auto_switch_theme_enabled() {
      return;
    }

    Theme::sync_system_appearance(Some(window), cx);
    ConfigStore::persist_app_settings(PersistedSettings {
      auto_switch_theme: true,
      dark_mode: cx.theme().mode.is_dark(),
    });
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
      WorkspacePage::GitConfig => self.git_config_page.clone().into_any_element(),
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
      WorkspacePage::GitConfig => self.git_config_page.read(cx).focus_handle(cx),
      WorkspacePage::Settings => self.settings_page.read(cx).focus_handle(cx),
    }
  }
}
