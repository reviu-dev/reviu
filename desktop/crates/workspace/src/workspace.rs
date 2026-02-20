use editor::set_indent_rainbow_enabled;
use gpui::{
  AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, Global, Render, Subscription,
  Task, Window, prelude::*,
};
use gpui_component::{ActiveTheme as _, Theme, ThemeMode, notification::Notification};
use smol::unblock;

use crate::about_page::AboutPage;
use crate::api::ApiClient;
use crate::app_update::{
  AppUpdateNotificationId, AppUpdateStore, AvailableAppUpdate, current_arch, current_platform,
  download_update_artifact, open_installer, resolve_effective_current_version, UpdateArtifact,
};
use crate::auth_state::AuthStateStore;
use crate::billing_page::BillingPage;
use crate::config::{AppSettings as PersistedSettings, ConfigStore};
use crate::git_config_page::GitConfigPage;
use crate::git_page::GitPage;
use crate::github_page::GithubPage;
use crate::github_pr_details_page::GithubPrDetailsPage;
use crate::settings_page::SettingsPage;
use ui::{Button, ButtonVariants as _, UiIconName, WindowExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspacePage {
  Git,
  Github,
  GithubPrDetails,
  Billing,
  GitConfig,
  Settings,
  About,
}

fn github_access_required(page: WorkspacePage) -> bool {
  matches!(page, WorkspacePage::Github | WorkspacePage::GithubPrDetails)
}

fn page_for_subscription_access(page: WorkspacePage, has_access: bool) -> WorkspacePage {
  if github_access_required(page) && !has_access {
    WorkspacePage::Billing
  } else {
    page
  }
}

fn billing_return_target_for_subscription(
  target: WorkspacePage,
  has_access: bool,
) -> WorkspacePage {
  if github_access_required(target) && !has_access {
    WorkspacePage::Git
  } else {
    target
  }
}

#[derive(Clone)]
pub(crate) struct WorkspaceRoute {
  pub page: WorkspacePage,
  pub settings_return: Option<WorkspacePage>,
  pub billing_return: Option<WorkspacePage>,
  pub git_config_return: Option<WorkspacePage>,
  pub about_return: Option<WorkspacePage>,
}

impl Default for WorkspaceRoute {
  fn default() -> Self {
    Self {
      page: WorkspacePage::Git,
      settings_return: None,
      billing_return: None,
      git_config_return: None,
      about_return: None,
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

  pub fn can_access_github(cx: &App) -> bool {
    AuthStateStore::has_active_subscription(cx)
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

  pub fn open_billing(cx: &mut App) {
    let current = cx.global::<Self>().page;
    let route = cx.global_mut::<Self>();
    if route.page != WorkspacePage::Billing {
      route.billing_return = Some(current);
    }
    route.page = WorkspacePage::Billing;
  }

  pub fn close_billing(cx: &mut App) {
    let target = {
      let route = cx.global_mut::<Self>();
      route.billing_return.take().unwrap_or(WorkspacePage::Git)
    };

    let target = billing_return_target_for_subscription(target, Self::can_access_github(cx));

    cx.global_mut::<Self>().page = target;
  }

  pub fn open_github(cx: &mut App) {
    if Self::can_access_github(cx) {
      cx.global_mut::<Self>().page = WorkspacePage::Github;
    } else {
      Self::open_billing(cx);
    }
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

  pub fn open_about(cx: &mut App) {
    let current = cx.global::<Self>().page;
    let route = cx.global_mut::<Self>();
    if route.page != WorkspacePage::About {
      route.about_return = Some(current);
    }
    route.page = WorkspacePage::About;
  }

  pub fn close_about(cx: &mut App) {
    let route = cx.global_mut::<Self>();
    let target = route.about_return.take().unwrap_or(WorkspacePage::Git);
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
  billing_page: Entity<BillingPage>,
  settings_page: Entity<SettingsPage>,
  about_page: Entity<AboutPage>,
  window_handle: AnyWindowHandle,
  last_page: Option<WorkspacePage>,
  _update_check_task: Option<Task<()>>,
  _update_download_task: Option<Task<()>>,
  _subscriptions: Vec<Subscription>,
}

impl WorkspaceView {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    cx.set_global(WorkspaceRoute::default());
    cx.set_global(WorkspaceApi::new());
    cx.set_global(AuthStateStore::default());
    cx.set_global(AppUpdateStore::default());

    let settings = ConfigStore::load_app_settings();
    set_indent_rainbow_enabled(settings.indent_rainbow);
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
    let billing_page = cx.new(|cx| BillingPage::new(window, cx));
    let settings_page = cx.new(|cx| SettingsPage::new(window, cx, settings));
    let about_page = cx.new(|cx| AboutPage::new(window, cx));

    let view = Self {
      git_page,
      git_config_page,
      github_page,
      github_pr_details_page,
      billing_page,
      settings_page,
      about_page,
      window_handle: window.window_handle(),
      last_page: None,
      _update_check_task: None,
      _update_download_task: None,
      _subscriptions: Vec::new(),
    };

    let mut view = view;
    let subscription = cx.observe_window_appearance(window, |this, window, cx| {
      this.on_window_appearance_changed(window, cx);
    });
    view._subscriptions.push(subscription);
    view.check_for_updates(cx);

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
      indent_rainbow: self.settings_page.read(cx).indent_rainbow_enabled(),
    });
  }

  fn check_for_updates(&mut self, cx: &mut Context<Self>) {
    let api = WorkspaceApi::global(cx).api.clone();
    let current_version = resolve_effective_current_version(env!("CARGO_PKG_VERSION"));
    let platform = current_platform().to_string();
    let arch = current_arch().to_string();
    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.check_desktop_update(&current_version, &platform, &arch)).await;
      let _ = this.update(cx, |this, cx| match result {
        Ok(payload) if payload.update_available => {
          let Some(artifact) = payload.artifact else {
            AppUpdateStore::set_error(
              cx,
              None,
              "Update artifact is missing for this platform.",
            );
            return;
          };
          let update = AvailableAppUpdate {
            latest_version: payload.latest_version,
            minimum_supported_version: payload.minimum_supported_version,
            release_notes_url: payload.release_notes_url,
            force_update: payload.force_update,
            artifact: UpdateArtifact {
              url: artifact.url,
              sha256: artifact.sha256,
              size: artifact.size,
            },
          };
          AppUpdateStore::set_available_update(cx, Some(update.clone()));
          this.show_update_notification(update, cx);
        }
        Ok(_) => {
          AppUpdateStore::clear_available_update(cx);
          this.dismiss_update_notification(cx);
        }
        Err(_) => {}
      });
    });

    self._update_check_task = Some(task);
  }

  fn trigger_update_download(&mut self, cx: &mut Context<Self>) {
    if AppUpdateStore::is_downloading(cx) {
      return;
    }

    if let Some(ready) = AppUpdateStore::try_ready_to_install(cx) {
      match open_installer(&ready.artifact_path) {
        Ok(()) => {
          AppUpdateStore::mark_install_started(cx, &ready.update);
          self.dismiss_update_notification(cx);
        }
        Err(err) => {
          AppUpdateStore::set_error(cx, Some(ready.update), err.to_string());
        }
      }
      return;
    }

    let Some(update) = AppUpdateStore::try_available_update(cx) else {
      return;
    };

    AppUpdateStore::set_downloading(cx, update.clone());
    let task = cx.spawn(async move |this, cx| {
      let download_result = unblock({
        let update = update.clone();
        move || download_update_artifact(&update)
      })
      .await;

      match download_result {
        Ok(ready) => {
          let install_path = ready.artifact_path.clone();
          let install_result = unblock(move || open_installer(&install_path)).await;
          let _ = this.update(cx, |this, cx| {
            AppUpdateStore::set_ready_to_install(cx, ready.clone());
            match install_result {
              Ok(()) => {
                AppUpdateStore::mark_install_started(cx, &ready.update);
                this.dismiss_update_notification(cx);
              }
              Err(err) => {
                AppUpdateStore::set_error(cx, Some(ready.update.clone()), err.to_string());
              }
            }
          });
        }
        Err(err) => {
          let _ = this.update(cx, |_, cx| {
            AppUpdateStore::set_error(cx, Some(update.clone()), err.to_string());
          });
        }
      }
    });

    self._update_download_task = Some(task);
  }

  fn dismiss_update_notification(&self, cx: &mut Context<Self>) {
    let _ = cx.update_window(self.window_handle, |_, window, cx| {
      window.remove_notification::<AppUpdateNotificationId>(cx);
    });
  }

  fn show_update_notification(&self, update: AvailableAppUpdate, cx: &mut Context<Self>) {
    let latest_version = update.latest_version.clone();
    let view = cx.entity();
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      let view = view.clone();
      window.push_notification(
        Notification::new()
          .id::<AppUpdateNotificationId>()
          .title(format!("New Reviu version {} available", latest_version))
          .message("Download the latest version.")
          .autohide(false)
          .action(move |_, _, _cx| {
            let view = view.clone();
            Button::new("workspace-update-download")
              .primary()
              .icon(UiIconName::Download)
              .label("Download")
              .on_click(move |_, window, cx| {
                let _ = view.update(cx, |this, cx| this.trigger_update_download(cx));
                window.on_next_frame(|window, cx| {
                  window.remove_notification::<AppUpdateNotificationId>(cx);
                });
              })
          }),
        cx,
      );
    });
  }
}

impl Render for WorkspaceView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let mut page = WorkspaceRoute::global(cx).page;
    let gated_page = page_for_subscription_access(page, WorkspaceRoute::can_access_github(cx));
    if gated_page != page {
      WorkspaceRoute::open_billing(cx);
      page = WorkspaceRoute::global(cx).page;
    }

    if self.last_page != Some(page) {
      self.last_page = Some(page);
      let focus_handle = self.focus_handle(cx);
      window.focus(&focus_handle, cx);
    }

    match page {
      WorkspacePage::Git => self.git_page.clone().into_any_element(),
      WorkspacePage::Github => self.github_page.clone().into_any_element(),
      WorkspacePage::GithubPrDetails => self.github_pr_details_page.clone().into_any_element(),
      WorkspacePage::Billing => self.billing_page.clone().into_any_element(),
      WorkspacePage::GitConfig => self.git_config_page.clone().into_any_element(),
      WorkspacePage::Settings => self.settings_page.clone().into_any_element(),
      WorkspacePage::About => self.about_page.clone().into_any_element(),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::{
    WorkspacePage, billing_return_target_for_subscription, page_for_subscription_access,
  };

  #[test]
  fn page_for_subscription_access_redirects_restricted_pages_without_subscription() {
    assert_eq!(
      page_for_subscription_access(WorkspacePage::Github, false),
      WorkspacePage::Billing
    );
    assert_eq!(
      page_for_subscription_access(WorkspacePage::GithubPrDetails, false),
      WorkspacePage::Billing
    );
  }

  #[test]
  fn page_for_subscription_access_keeps_allowed_pages() {
    assert_eq!(
      page_for_subscription_access(WorkspacePage::Git, false),
      WorkspacePage::Git
    );
    assert_eq!(
      page_for_subscription_access(WorkspacePage::About, false),
      WorkspacePage::About
    );
    assert_eq!(
      page_for_subscription_access(WorkspacePage::Settings, false),
      WorkspacePage::Settings
    );
    assert_eq!(
      page_for_subscription_access(WorkspacePage::Github, true),
      WorkspacePage::Github
    );
  }

  #[test]
  fn billing_return_target_for_subscription_falls_back_to_git_when_needed() {
    assert_eq!(
      billing_return_target_for_subscription(WorkspacePage::Github, false),
      WorkspacePage::Git
    );
    assert_eq!(
      billing_return_target_for_subscription(WorkspacePage::GithubPrDetails, false),
      WorkspacePage::Git
    );
    assert_eq!(
      billing_return_target_for_subscription(WorkspacePage::Settings, false),
      WorkspacePage::Settings
    );
  }
}

impl Focusable for WorkspaceView {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    match WorkspaceRoute::global(cx).page {
      WorkspacePage::Git => self.git_page.read(cx).focus_handle(cx),
      WorkspacePage::Github => self.github_page.read(cx).focus_handle(cx),
      WorkspacePage::GithubPrDetails => self.github_pr_details_page.read(cx).focus_handle(cx),
      WorkspacePage::Billing => self.billing_page.read(cx).focus_handle(cx),
      WorkspacePage::GitConfig => self.git_config_page.read(cx).focus_handle(cx),
      WorkspacePage::Settings => self.settings_page.read(cx).focus_handle(cx),
      WorkspacePage::About => self.about_page.read(cx).focus_handle(cx),
    }
  }
}
