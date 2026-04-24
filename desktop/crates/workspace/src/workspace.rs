use std::rc::Rc;
use std::time::Duration;

use editor::{Copy, Cut, Paste, Quit, Redo, SelectAll, Undo, set_indent_rainbow_enabled};
#[cfg(test)]
use gpui::Keystroke;
use gpui::{
  AnyWindowHandle, App, Context, Decorations, Entity, FocusHandle, Focusable, Global, Menu,
  MenuItem, Render, Subscription, Task, Window, WindowButton, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, Sizable as _, Theme, ThemeMode, h_flex,
  kbd::Kbd,
  notification::Notification,
  spinner::Spinner,
  tab::{Tab, TabBar},
  tag::Tag,
};
use gpui_router::{Route, Routes};
use smol::unblock;

use crate::AppProfile;
use crate::AuthCallbackTarget;
use crate::about_page::AboutPage;
use crate::active_local_repo::ActiveLocalRepoStore;
use crate::api::ApiClient;
use crate::app_update::{
  AppUpdateNotificationId, AppUpdateState, AppUpdateStore, AvailableAppUpdate, UpdateArtifact,
  current_arch, current_platform, download_update_artifact, install_update_artifact,
  ready_update_button_label, resolved_build_version, should_install_update_after_download,
};
use crate::auth_state::{AuthState, AuthStateStore};
use crate::billing_page::BillingPage;
use crate::config::{AppSettings as PersistedSettings, ConfigStore};
use crate::dock_badge::set_dock_badge;
use crate::git_config_page::GitConfigPage;
use crate::git_page::{GitPage, GitPageHandle};
use crate::github_commit_details_page::{GithubCommitDetailsPage, GithubCommitDetailsPageHandle};
use crate::github_page::{GithubPage, GithubPageHandle};
use crate::github_pr_details_page::{GithubPrDetailsPage, GithubPrDetailsPageHandle};
use crate::github_profile_page::{GithubProfilePage, GithubProfilePageHandle};
use crate::github_repo_page::{GithubRepoPage, GithubRepoPageHandle};
use crate::navigation::NavigationHistory;
use crate::notification_count::NotificationCountStore;
use crate::sentry_context;
use crate::settings_page::SettingsPage;
use crate::shortcuts::{self, ShortcutId};
use crate::{ShowCommandPalette, ShowFileSearch};
use ui::{
  Button, ButtonVariants as _, GLOBAL_BAR_HEIGHT, StatusTag, StatusThemeExt, UiIconName,
  UserMenuConfig, UserMenuPage, UserMenuState, UserMenuUser, WindowExt, user_menu,
};

const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);

pub const STATUS_BAR_ICON_PNG: &[u8] =
  include_bytes!("../../reviu/assets/reviu_status_bar_icon.png");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspacePage {
  Git,
  Github,
  GithubProfile,
  GithubRepo,
  GithubPrDetails,
  GithubCommitDetails,
  Billing,
  GitConfig,
  Settings,
  About,
}

pub(crate) fn workspace_page_from_pathname(pathname: &str) -> WorkspacePage {
  if pathname.starts_with("/github/") {
    let segments: Vec<&str> = pathname.trim_start_matches('/').split('/').collect();
    // PR details: /github/{owner}/{repo}/pull/{number}[/{tab}]
    if segments.len() >= 5 && segments[3] == "pull" {
      return WorkspacePage::GithubPrDetails;
    }
    // Commit details: /github/{owner}/{repo}/commit/{sha}
    if segments.len() >= 5 && segments[3] == "commit" {
      return WorkspacePage::GithubCommitDetails;
    }
    // Profile page: /github/{login}
    if segments.len() == 2 {
      return WorkspacePage::GithubProfile;
    }
    // Repo page: /github/{owner}/{repo}[/{tab}]
    if segments.len() >= 3 {
      return WorkspacePage::GithubRepo;
    }
  }
  match pathname {
    "/git" => WorkspacePage::Git,
    "/github" => WorkspacePage::Github,
    "/billing" => WorkspacePage::Billing,
    "/settings" => WorkspacePage::Settings,
    "/git-config" => WorkspacePage::GitConfig,
    "/about" => WorkspacePage::About,
    _ => WorkspacePage::Git,
  }
}

/// Returns true when the current path supports file search.
/// Git page always does; repo page on /code tab; PR details on /changes tab.
fn page_has_file_search(pathname: &str) -> bool {
  if pathname == "/git" {
    return true;
  }
  if pathname.starts_with("/github/") {
    let segments: Vec<&str> = pathname.trim_start_matches('/').split('/').collect();
    // PR: /github/{owner}/{repo}/pull/{number}/changes
    if segments.len() >= 6 && segments[3] == "pull" && segments[5] == "changes" {
      return true;
    }
    // Repo: /github/{owner}/{repo}/code
    if segments.len() >= 4 && segments[3] == "code" && !segments.contains(&"pull") {
      return true;
    }
  }
  false
}

fn user_menu_page_for_workspace_page(page: WorkspacePage) -> UserMenuPage {
  match page {
    WorkspacePage::Git => UserMenuPage::Git,
    WorkspacePage::Github
    | WorkspacePage::GithubProfile
    | WorkspacePage::GithubRepo
    | WorkspacePage::GithubCommitDetails => UserMenuPage::Github,
    WorkspacePage::GithubPrDetails => UserMenuPage::GithubPrDetails,
    WorkspacePage::Billing => UserMenuPage::Billing,
    WorkspacePage::GitConfig => UserMenuPage::GitConfig,
    WorkspacePage::Settings => UserMenuPage::Settings,
    WorkspacePage::About => UserMenuPage::About,
  }
}

fn primary_navigation_selected_index(page: WorkspacePage) -> Option<usize> {
  match page {
    WorkspacePage::Git => Some(0),
    WorkspacePage::Github
    | WorkspacePage::GithubProfile
    | WorkspacePage::GithubRepo
    | WorkspacePage::GithubPrDetails
    | WorkspacePage::GithubCommitDetails => Some(1),
    WorkspacePage::Billing
    | WorkspacePage::GitConfig
    | WorkspacePage::Settings
    | WorkspacePage::About => None,
  }
}

fn refresh_label_for_workspace_page(page: WorkspacePage) -> Option<&'static str> {
  match page {
    WorkspacePage::Git => Some("Refresh Git"),
    WorkspacePage::Github => Some("Refresh GitHub"),
    WorkspacePage::GithubProfile => Some("Refresh Profile"),
    WorkspacePage::GithubRepo => Some("Refresh Repo"),
    WorkspacePage::GithubPrDetails => Some("Refresh PR"),
    WorkspacePage::GithubCommitDetails => Some("Refresh Commit"),
    WorkspacePage::Billing
    | WorkspacePage::GitConfig
    | WorkspacePage::Settings
    | WorkspacePage::About => None,
  }
}

fn refresh_in_progress_for_workspace_page(page: WorkspacePage, cx: &App) -> bool {
  match page {
    WorkspacePage::Git => GitPageHandle::is_refreshing(cx),
    WorkspacePage::Github => GithubPageHandle::is_refreshing(cx),
    WorkspacePage::GithubProfile => GithubProfilePageHandle::is_refreshing(cx),
    WorkspacePage::GithubRepo => GithubRepoPageHandle::is_refreshing(cx),
    WorkspacePage::GithubPrDetails => GithubPrDetailsPageHandle::is_refreshing(cx),
    WorkspacePage::GithubCommitDetails => GithubCommitDetailsPageHandle::is_refreshing(cx),
    WorkspacePage::Billing
    | WorkspacePage::GitConfig
    | WorkspacePage::Settings
    | WorkspacePage::About => false,
  }
}

fn page_supports_refresh(page: WorkspacePage) -> bool {
  refresh_label_for_workspace_page(page).is_some()
}

fn github_primary_navigation_count_label(notification_count: usize) -> Option<String> {
  (notification_count > 0).then(|| notification_count.to_string())
}

fn should_run_scheduled_update_check(state: Option<AppUpdateState>) -> bool {
  !matches!(
    state,
    Some(AppUpdateState::Available(_))
      | Some(AppUpdateState::Downloading(_))
      | Some(AppUpdateState::ReadyToInstall(_))
      | Some(AppUpdateState::Error {
        update: Some(_),
        ..
      })
  )
}

pub fn build_app_menus(show_billing_entry: bool) -> Vec<Menu> {
  let mut navigate_items = vec![
    MenuItem::action("Back", crate::NavigateBack),
    MenuItem::separator(),
    MenuItem::action("Git", crate::OpenGitPage),
    MenuItem::action("GitHub", crate::OpenGithubPage),
    MenuItem::separator(),
    MenuItem::action("Git Config", crate::OpenGitConfigPage),
  ];

  if show_billing_entry {
    navigate_items.push(MenuItem::action("Billing", crate::OpenBillingPage));
  }

  vec![
    Menu {
      name: "Reviu".into(),
      disabled: false,
      items: vec![
        MenuItem::action("About Reviu", crate::OpenAboutPage),
        MenuItem::separator(),
        MenuItem::action("Settings...", crate::OpenSettingsPage),
        MenuItem::separator(),
        MenuItem::action("Quit Reviu", Quit),
      ],
    },
    Menu {
      name: "Navigate".into(),
      disabled: false,
      items: navigate_items,
    },
    Menu {
      name: "Edit".into(),
      disabled: false,
      items: vec![
        MenuItem::os_action("Undo", Undo, gpui::OsAction::Undo),
        MenuItem::os_action("Redo", Redo, gpui::OsAction::Redo),
        MenuItem::separator(),
        MenuItem::os_action("Cut", Cut, gpui::OsAction::Cut),
        MenuItem::os_action("Copy", Copy, gpui::OsAction::Copy),
        MenuItem::os_action("Paste", Paste, gpui::OsAction::Paste),
        MenuItem::separator(),
        MenuItem::os_action("Select All", SelectAll, gpui::OsAction::SelectAll),
      ],
    },
  ]
}

/// Lightweight global that tracks the current page for focus delegation and sidebar highlighting.
/// The source of truth for navigation is `NavigationHistory` / `RouterState`.
/// This struct is kept in sync by `WorkspaceView::render`.
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
  github_profile_page: Entity<GithubProfilePage>,
  github_repo_page: Entity<GithubRepoPage>,
  github_pr_details_page: Entity<GithubPrDetailsPage>,
  github_commit_details_page: Entity<GithubCommitDetailsPage>,
  billing_page: Entity<BillingPage>,
  settings_page: Entity<SettingsPage>,
  about_page: Entity<AboutPage>,
  window_handle: AnyWindowHandle,
  last_page: Option<WorkspacePage>,
  _update_check_task: Option<Task<()>>,
  _periodic_update_check_task: Option<Task<()>>,
  _update_download_task: Option<Task<()>>,
  _notification_poll_task: Option<Task<()>>,
  #[cfg(any(target_os = "linux", target_os = "windows"))]
  _status_bar_event_task: Option<Task<()>>,
  _subscriptions: Vec<Subscription>,
}

impl WorkspaceView {
  const GLOBAL_BAR_MACOS_LEFT_PADDING: f32 = 85.0;

  #[cfg(test)]
  fn command_palette_shortcut() -> Keystroke {
    shortcuts::shortcut_keystroke(ShortcutId::ShowCommandPalette)
  }

  fn command_palette_kbd(window: &Window, pathname: &str, cx: &App) -> Kbd {
    Kbd::new(shortcuts::resolved_shortcut_keystroke_in(
      cx,
      window,
      ShortcutId::ShowCommandPalette,
      shortcuts::key_context_for_pathname(pathname),
    ))
  }

  fn file_search_kbd(window: &Window, pathname: &str, cx: &App) -> Kbd {
    Kbd::new(shortcuts::resolved_shortcut_keystroke_in(
      cx,
      window,
      ShortcutId::ShowFileSearch,
      shortcuts::key_context_for_pathname(pathname),
    ))
  }

  fn refresh_kbd(window: &Window, pathname: &str, cx: &App) -> Kbd {
    Kbd::new(shortcuts::resolved_shortcut_keystroke_in(
      cx,
      window,
      ShortcutId::RefreshCurrentPage,
      shortcuts::key_context_for_pathname(pathname),
    ))
  }

  fn sync_app_menus(cx: &mut App) {
    cx.set_menus(build_app_menus(AuthStateStore::should_show_billing_entry(
      cx,
    )));
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    gpui_router::init(cx);
    NavigationHistory::init(cx);
    // Set initial route to /git
    NavigationHistory::navigate_replace("/git", cx);

    cx.set_global(WorkspaceRoute::default());
    cx.set_global(WorkspaceApi::new());
    cx.set_global(AuthStateStore::default());
    cx.set_global(ActiveLocalRepoStore::default());
    cx.set_global(AppUpdateStore::default());
    cx.set_global(NotificationCountStore::default());

    let settings = ConfigStore::load_app_settings();
    cx.set_global(settings);
    cx.set_global(shortcuts::load_shortcut_overrides());
    cx.set_global(crate::command_usage::CommandUsageStore::load());
    crate::command_usage::install_palette_usage_recorder(cx);
    set_indent_rainbow_enabled(settings.indent_rainbow);
    Theme::global_mut(cx).font_size = px(settings.font_size);
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
    crate::install_app_key_bindings(cx);

    let git_page = cx.new(|cx| GitPage::new(window, cx));
    let git_config_page = cx.new(|cx| GitConfigPage::new(window, cx));
    let github_page = cx.new(|cx| GithubPage::new(window, cx));
    let github_profile_page = cx.new(|cx| GithubProfilePage::new(window, cx));
    let github_repo_page = cx.new(|cx| GithubRepoPage::new(window, cx));
    let github_pr_details_page = cx.new(|cx| GithubPrDetailsPage::new(window, cx));
    let github_commit_details_page = cx.new(|cx| GithubCommitDetailsPage::new(window, cx));
    let billing_page = cx.new(|cx| BillingPage::new(window, cx));
    let settings_page = cx.new(|cx| SettingsPage::new(window, cx, settings));
    let about_page = cx.new(|cx| AboutPage::new(window, cx));

    let view = Self {
      git_page,
      git_config_page,
      github_page,
      github_profile_page,
      github_repo_page,
      github_pr_details_page,
      github_commit_details_page,
      billing_page,
      settings_page,
      about_page,
      window_handle: window.window_handle(),
      last_page: None,
      _update_check_task: None,
      _periodic_update_check_task: None,
      _update_download_task: None,
      _notification_poll_task: None,
      #[cfg(any(target_os = "linux", target_os = "windows"))]
      _status_bar_event_task: None,
      _subscriptions: Vec::new(),
    };

    let mut view = view;
    let subscription = cx.observe_window_appearance(window, |this, window, cx| {
      this.on_window_appearance_changed(window, cx);
    });
    view._subscriptions.push(subscription);
    let subscription = cx.observe_global::<AuthStateStore>(|_, cx| {
      Self::sync_app_menus(cx);
      cx.notify();
    });
    view._subscriptions.push(subscription);
    let subscription = cx.observe_global::<shortcuts::ShortcutOverrides>(|_, cx| {
      cx.notify();
    });
    view._subscriptions.push(subscription);
    Self::sync_app_menus(cx);
    view.check_for_updates(cx);
    view.start_periodic_update_checks(cx);
    view.start_notification_polling(cx);
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    view.start_status_bar_event_polling(cx);
    if PersistedSettings::get(cx).menu_bar_icon {
      crate::status_bar::init_status_bar(STATUS_BAR_ICON_PNG);
    }

    view
  }

  fn on_window_appearance_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.settings_page.read(cx).auto_switch_theme_enabled() {
      return;
    }

    Theme::sync_system_appearance(Some(window), cx);
    let mut settings = PersistedSettings::get(cx);
    settings.dark_mode = cx.theme().mode.is_dark();
    cx.set_global(settings);
    ConfigStore::persist_app_settings(settings);
  }

  fn handle_pending_status_bar_notification(&self, cx: &mut App) {
    if crate::status_bar::take_open_reviu_request() {
      cx.activate(true);
    }

    let Some(notification) = crate::status_bar::take_pending_notification() else {
      return;
    };

    cx.activate(true);

    if notification.unread {
      let api = WorkspaceApi::global(cx).api.clone();
      let thread_id = notification.id.clone();

      let count = NotificationCountStore::get(cx).saturating_sub(1);
      NotificationCountStore::set(cx, count);
      set_dock_badge(count);

      cx.background_spawn(async move {
        let _ = unblock(move || api.mark_notification_read(&thread_id)).await;
      })
      .detach();
    }

    let full_name = &notification.repository.full_name;
    let (owner, repo) = full_name.split_once('/').unwrap_or((full_name, ""));

    match notification.subject.subject_type.as_str() {
      "PullRequest" => {
        if let Some(number) = notification
          .subject
          .url
          .as_deref()
          .and_then(|url| url.rsplit('/').next()?.parse::<u64>().ok())
        {
          crate::github_navigation::open_pr_target(
            owner.to_string(),
            repo.to_string(),
            number,
            false,
            None,
            cx,
          );
        }
      }
      "Issue" => {
        let issue_number = notification
          .subject
          .url
          .as_deref()
          .and_then(|url| url.rsplit('/').next()?.parse::<u64>().ok());
        crate::github_navigation::open_repo_target(
          owner.to_string(),
          repo.to_string(),
          Some(ui::CommandPaletteGithubRepoTab::Issues),
          issue_number,
          None,
          cx,
        );
      }
      _ => {
        let url = notification
          .subject
          .url
          .as_deref()
          .map(
            |_api_url| match notification.subject.subject_type.as_str() {
              "Release" => format!("https://github.com/{full_name}/releases"),
              "Discussion" => format!("https://github.com/{full_name}/discussions"),
              _ => format!("https://github.com/{full_name}"),
            },
          )
          .unwrap_or_else(|| format!("https://github.com/{full_name}"));
        cx.open_url(&url);
      }
    }
  }

  fn check_for_updates(&mut self, cx: &mut Context<Self>) {
    let api = WorkspaceApi::global(cx).api.clone();
    let current_version = resolved_build_version(env!("CARGO_PKG_VERSION"));
    let platform = current_platform().to_string();
    let arch = current_arch().to_string();
    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.check_desktop_update(&current_version, &platform, &arch)).await;
      let _ = this.update(cx, |this, cx| match result {
        Ok(payload) if payload.update_available => {
          let Some(artifact) = payload.artifact else {
            AppUpdateStore::set_error(cx, None, "Update artifact is missing for this platform.");
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

  fn start_periodic_update_checks(&mut self, cx: &mut Context<Self>) {
    let task = cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(UPDATE_CHECK_INTERVAL).await;

        let should_check = this
          .update(cx, |_, cx| {
            should_run_scheduled_update_check(AppUpdateStore::try_state(cx))
          })
          .unwrap_or(false);

        if !should_check {
          continue;
        }

        let _ = this.update(cx, |this, cx| {
          this.check_for_updates(cx);
        });
      }
    });

    self._periodic_update_check_task = Some(task);
  }

  fn start_notification_polling(&mut self, cx: &mut Context<Self>) {
    let api = WorkspaceApi::global(cx).api.clone();
    let task = cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_secs(60))
          .await;

        let has_access = this
          .update(cx, |_, cx| AuthStateStore::has_github_access(cx))
          .unwrap_or(false);

        if !has_access {
          continue;
        }

        let api = api.clone();
        let result = unblock(move || api.fetch_github_notifications()).await;

        let _ = this.update(cx, |_, cx| {
          if let Ok(notifications) = result {
            let unread = notifications.iter().filter(|n| n.unread).count();
            NotificationCountStore::set(cx, unread);
            set_dock_badge(unread);
            if PersistedSettings::get(cx).menu_bar_icon {
              crate::status_bar::update_status_bar(unread, &notifications);
            }
            cx.refresh_windows();
          }
        });
      }
    });

    self._notification_poll_task = Some(task);
  }

  #[cfg(any(target_os = "linux", target_os = "windows"))]
  fn start_status_bar_event_polling(&mut self, cx: &mut Context<Self>) {
    let task = cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(250))
          .await;

        if !crate::status_bar::has_pending_interaction() {
          continue;
        }

        let _ = this.update(cx, |this, cx| {
          this.handle_pending_status_bar_notification(cx);
        });
      }
    });

    self._status_bar_event_task = Some(task);
  }

  fn trigger_update_download(&mut self, cx: &mut Context<Self>) {
    if AppUpdateStore::is_downloading(cx) {
      return;
    }

    if let Some(ready) = AppUpdateStore::try_ready_to_install(cx) {
      #[cfg(target_os = "windows")]
      {
        match install_update_artifact(&ready) {
          Ok(()) => cx.quit(),
          Err(err) => AppUpdateStore::set_error(cx, Some(ready.update.clone()), err.to_string()),
        }
        return;
      }

      #[cfg(not(target_os = "windows"))]
      {
        if let Some(path) = ready.restart_binary_path {
          cx.set_restart_path(path);
        }
        cx.restart();
        return;
      }
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
          if !should_install_update_after_download() {
            let _ = this.update(cx, |_, cx| {
              AppUpdateStore::set_ready_to_install(cx, ready.clone());
            });
            return;
          }

          let install_ready = ready.clone();
          let install_result = unblock(move || install_update_artifact(&install_ready)).await;
          let _ = this.update(cx, |_, cx| match install_result {
            Ok(()) => {
              AppUpdateStore::set_ready_to_install(cx, ready.clone());
            }
            Err(err) => {
              AppUpdateStore::set_error(cx, Some(ready.update.clone()), err.to_string());
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
          .content(move |_, _, _cx| {
            let view = view.clone();
            div()
              .flex()
              .mt_3()
              .gap_2()
              .child(
                Button::new("workspace-update-changelog")
                  .ghost()
                  .compact()
                  .small()
                  .label("Changelog")
                  .on_click(move |_, _, cx| {
                    cx.open_url("https://reviu.dev/changelog");
                  }),
              )
              .child(
                Button::new("workspace-update-download")
                  .primary()
                  .compact()
                  .small()
                  .icon(UiIconName::Download)
                  .label("Download")
                  .on_click(move |_, window, cx| {
                    view.update(cx, |this, cx| this.trigger_update_download(cx));
                    window.on_next_frame(|window, cx| {
                      window.remove_notification::<AppUpdateNotificationId>(cx);
                    });
                  }),
              )
              .into_any_element()
          }),
        cx,
      );
    });
  }

  fn update_button_label(state: Option<AppUpdateState>) -> &'static str {
    match state {
      Some(AppUpdateState::Downloading(_)) => "Downloading...",
      Some(AppUpdateState::ReadyToInstall(_)) => ready_update_button_label(),
      _ => "New version available",
    }
  }

  fn global_update_download_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.trigger_update_download(cx);
    window.on_next_frame(|window, cx| {
      window.remove_notification::<AppUpdateNotificationId>(cx);
    });
  }

  fn open_github_home(cx: &mut App) {
    if AuthStateStore::has_github_access(cx) {
      GithubPageHandle::refresh(cx);
    }
    NavigationHistory::navigate("/github", cx);
  }

  fn refresh_current_page_action(
    &mut self,
    _: &crate::RefreshCurrentPage,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let page = WorkspaceRoute::global(cx).page;
    if refresh_in_progress_for_workspace_page(page, cx) {
      return;
    }

    match page {
      WorkspacePage::Git => GitPageHandle::refresh_page(cx),
      WorkspacePage::Github => GithubPageHandle::refresh(cx),
      WorkspacePage::GithubProfile => GithubProfilePageHandle::refresh(cx),
      WorkspacePage::GithubRepo => GithubRepoPageHandle::refresh(cx),
      WorkspacePage::GithubPrDetails => GithubPrDetailsPageHandle::refresh(cx),
      WorkspacePage::GithubCommitDetails => GithubCommitDetailsPageHandle::refresh(cx),
      WorkspacePage::Billing
      | WorkspacePage::GitConfig
      | WorkspacePage::Settings
      | WorkspacePage::About => {}
    }
  }

  fn navigate_back_action(
    &mut self,
    _: &crate::NavigateBack,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    NavigationHistory::navigate_back(cx);
  }

  fn render_linux_window_controls(window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let controls = window.window_controls();
    let button_layout = cx.button_layout();

    let render_button = |icon: IconName, action: WindowButton, theme: &Theme| -> gpui::AnyElement {
      let is_close = matches!(action, WindowButton::Close);
      div()
        .id(match action {
          WindowButton::Minimize => "linux-minimize",
          WindowButton::Maximize => "linux-maximize",
          WindowButton::Close => "linux-close",
        })
        .flex()
        .items_center()
        .justify_center()
        .w(px(28.0))
        .h(px(28.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|s| {
          if is_close {
            s.bg(gpui::red())
          } else {
            s.bg(theme.secondary_hover)
          }
        })
        .active(|s| {
          if is_close {
            s.bg(gpui::red())
          } else {
            s.bg(theme.secondary_active)
          }
        })
        .child(Icon::new(icon).size_4().text_color(theme.muted_foreground))
        .on_click(move |_, window, _cx| match action {
          WindowButton::Minimize => window.minimize_window(),
          WindowButton::Maximize => window.zoom_window(),
          WindowButton::Close => std::process::exit(0),
        })
        .into_any_element()
    };

    let mut buttons: Vec<gpui::AnyElement> = Vec::new();

    let ordered_buttons: Vec<WindowButton> = if let Some(layout) = button_layout {
      layout
        .right
        .iter()
        .chain(layout.left.iter())
        .filter_map(|b| *b)
        .collect()
    } else {
      vec![
        WindowButton::Minimize,
        WindowButton::Maximize,
        WindowButton::Close,
      ]
    };

    for button in ordered_buttons {
      let (icon, supported) = match button {
        WindowButton::Minimize => (IconName::WindowMinimize, controls.minimize),
        WindowButton::Maximize => (IconName::WindowMaximize, controls.maximize),
        WindowButton::Close => (IconName::WindowClose, true),
      };
      if supported {
        buttons.push(render_button(icon, button, &theme));
      }
    }

    h_flex().items_center().gap_1().ml_2().children(buttons)
  }

  fn render_global_bar(
    &self,
    window: &Window,
    page: WorkspacePage,
    pathname: &str,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let show_update_button = AppUpdateStore::try_available_update(cx).is_some();
    let update_download_in_progress = AppUpdateStore::is_downloading(cx);
    let update_button_label = Self::update_button_label(AppUpdateStore::try_state(cx));

    let current_page = user_menu_page_for_workspace_page(page);
    let auth_state = AuthStateStore::get(cx);
    let is_unauthenticated = matches!(auth_state, AuthState::Unauthenticated);
    let has_github_access = AuthStateStore::has_github_access(cx);
    let show_billing_entry = AuthStateStore::should_show_billing_entry(cx);

    let open_billing: Rc<dyn Fn(&mut Window, &mut App)> =
      Rc::new(|_window: &mut Window, cx: &mut App| {
        NavigationHistory::navigate("/billing", cx);
      });
    let open_git_config = Rc::new(|_window: &mut Window, cx: &mut App| {
      NavigationHistory::navigate("/git-config", cx);
    });
    let open_settings = Rc::new(|_window: &mut Window, cx: &mut App| {
      NavigationHistory::navigate("/settings", cx);
    });
    let open_about = Rc::new(|_window: &mut Window, cx: &mut App| {
      NavigationHistory::navigate("/about", cx);
    });
    let sign_in = Rc::new(|_window: &mut Window, cx: &mut App| {
      AuthCallbackTarget::start_sign_in(cx);
    });
    let sign_out = Rc::new(|_window: &mut Window, cx: &mut App| {
      AuthCallbackTarget::sign_out(cx);
      NotificationCountStore::set(cx, 0);
      set_dock_badge(0);
    });

    let auth_control = match auth_state {
      AuthState::Authenticated(user) => {
        let display_name = if user.name.trim().is_empty() {
          user.email.clone()
        } else {
          user.name.clone()
        };
        let on_open_billing = show_billing_entry.then_some(open_billing.clone());

        user_menu(UserMenuConfig {
          id: "workspace-auth-menu".into(),
          state: UserMenuState::Authenticated(UserMenuUser {
            name: display_name.into(),
            email: user.email.into(),
            image: user.image.map(Into::into),
          }),
          current_page,
          notification_count: NotificationCountStore::get(cx),
          on_open_git: None,
          on_open_github: None,
          on_open_billing,
          on_open_git_config: Some(open_git_config),
          on_open_settings: Some(open_settings),
          on_open_about: Some(open_about),
          on_sign_in: Some(sign_in),
          on_sign_out: Some(sign_out),
        })
      }
      AuthState::Unauthenticated => user_menu(UserMenuConfig {
        id: "workspace-auth-menu".into(),
        state: UserMenuState::Unauthenticated,
        current_page,
        notification_count: 0,
        on_open_git: None,
        on_open_github: None,
        on_open_billing: None,
        on_open_git_config: Some(open_git_config),
        on_open_settings: Some(open_settings),
        on_open_about: Some(open_about),
        on_sign_in: None,
        on_sign_out: None,
      }),
      _ => None,
    };

    let update_button = Button::new("workspace-global-update-download")
      .icon(UiIconName::Download)
      .label(update_button_label)
      .ghost()
      .compact()
      .small()
      .disabled(update_download_in_progress)
      .on_click(cx.listener(Self::global_update_download_action));

    let sign_in_button = Button::new("workspace-global-sign-in")
      .icon(IconName::Github)
      .label("Sign in with GitHub")
      .ghost()
      .gap_2()
      .small()
      .on_click(|_, _, cx| {
        AuthCallbackTarget::start_sign_in(cx);
      });

    let show_file_search_button = page_has_file_search(&pathname);
    let refresh_button = page_supports_refresh(page).then(|| {
      let label = refresh_label_for_workspace_page(page)
        .expect("refresh label should exist for refreshable workspace pages");
      let refresh_in_progress = refresh_in_progress_for_workspace_page(page, cx);
      Button::new("workspace-global-refresh")
        .icon(UiIconName::RefreshCw)
        .loading_icon(Icon::new(UiIconName::RefreshCw))
        .loading(refresh_in_progress)
        .ghost()
        .compact()
        .small()
        .disabled(refresh_in_progress)
        .tooltip(label)
        .child(Self::refresh_kbd(window, pathname, cx).ml_1())
        .on_click(|_, window, cx| {
          window.dispatch_action(Box::new(crate::RefreshCurrentPage), cx);
        })
    });

    let file_search_button = Button::new("workspace-global-file-search")
      .label("File search")
      .ghost()
      .compact()
      .small()
      .child(Self::file_search_kbd(window, pathname, cx).ml_1())
      .on_click(|_, window, cx| {
        window.dispatch_action(Box::new(ShowFileSearch), cx);
      });

    let command_palette_button = Button::new("workspace-global-command-palette")
      .label("Command palette")
      .ghost()
      .compact()
      .small()
      .child(Self::command_palette_kbd(window, pathname, cx).ml_1())
      .on_click(|_, window, cx| {
        window.dispatch_action(Box::new(ShowCommandPalette), cx);
      });

    let github_notification_count = NotificationCountStore::get(cx);
    let github_notification_label = has_github_access
      .then(|| github_primary_navigation_count_label(github_notification_count))
      .flatten();
    let primary_navigation = TabBar::new("workspace-primary-navigation")
      .segmented()
      .small()
      .on_click(|ix, _, cx| match ix {
        0 => NavigationHistory::navigate("/git", cx),
        1 => Self::open_github_home(cx),
        _ => {}
      })
      .child(Tab::new().label("Git"))
      .child(
        Tab::new().child(
          h_flex()
            .items_center()
            .gap_2()
            .child("GitHub")
            .when_some(github_notification_label, |this, label| {
              this.child(StatusTag::new(theme.status_red()).xsmall().child(label))
            }),
        ),
      );
    let primary_navigation = if let Some(selected_index) = primary_navigation_selected_index(page) {
      primary_navigation.selected_index(selected_index)
    } else {
      primary_navigation
    };

    let bar = div()
      .h(px(GLOBAL_BAR_HEIGHT))
      .max_h(px(GLOBAL_BAR_HEIGHT))
      .w_full()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border);
    let bar = if cfg!(target_os = "macos") {
      bar.pl(px(Self::GLOBAL_BAR_MACOS_LEFT_PADDING)).pr_3()
    } else {
      bar.px_3()
    };

    let mut right = div().flex().items_center().gap_2();
    if show_update_button {
      right = right.child(update_button);
    }
    if show_file_search_button {
      right = right.child(file_search_button);
    }
    right = right.child(command_palette_button);
    if is_unauthenticated {
      right = right.child(sign_in_button);
    }
    if let Some(auth_control) = auth_control {
      right = right.child(auth_control);
    }

    let use_client_decorations = cfg!(target_os = "linux")
      && matches!(window.window_decorations(), Decorations::Client { .. });

    if use_client_decorations {
      right = right.child(Self::render_linux_window_controls(window, cx));
    }

    let left = h_flex()
      .items_center()
      .gap_3()
      .when_some(AppProfile::current().header_tag_label(), |this, label| {
        this.child(Tag::secondary().small().rounded_full().child(label))
      })
      .child(primary_navigation)
      .when_some(refresh_button, |this, button| this.child(button));

    if use_client_decorations {
      let drag_area = div()
        .id("linux-titlebar-drag")
        .flex_1()
        .h_full()
        .on_mouse_down(gpui::MouseButton::Left, |ev, window, _cx| {
          if ev.click_count >= 2 {
            window.zoom_window();
          } else {
            window.start_window_move();
          }
        })
        .on_mouse_down(gpui::MouseButton::Right, |ev, window, _cx| {
          window.show_window_menu(ev.position);
        });
      bar.child(left).child(drag_area).child(right)
    } else {
      let zoom_area = div().id("titlebar-zoom").flex_1().h_full().on_mouse_down(
        gpui::MouseButton::Left,
        |ev, window, _cx| {
          if ev.click_count >= 2 {
            window.zoom_window();
          }
        },
      );
      bar.child(left).child(zoom_area).child(right)
    }
  }
}

impl Render for WorkspaceView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.handle_pending_status_bar_notification(cx);
    let auth_state = AuthStateStore::get(cx);

    // Show a loading screen while the initial auth check is in progress
    if matches!(auth_state, AuthState::Unknown) {
      let theme = cx.theme().clone();
      let version = format!(
        "Reviu v{}",
        resolved_build_version(env!("CARGO_PKG_VERSION"))
      );
      return div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .bg(theme.background)
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(version),
        )
        .child(Spinner::new().small())
        .into_any_element();
    }

    let pathname = NavigationHistory::current_pathname(cx);
    let page = workspace_page_from_pathname(&pathname);

    // Keep WorkspaceRoute in sync BEFORE focus delegation (focus_handle reads it)
    cx.global_mut::<WorkspaceRoute>().page = page;
    sentry_context::sync_workspace_route(&pathname, page);

    // Sync sentry context and focus on page change
    if self.last_page != Some(page) {
      let previous_page = self.last_page;
      self.last_page = Some(page);
      sentry_context::sync_workspace_page(previous_page, page);
      if previous_page == Some(WorkspacePage::Git) && page != WorkspacePage::Git {
        sentry_context::clear_git_context();
      }
      if previous_page == Some(WorkspacePage::GithubPrDetails)
        && page != WorkspacePage::GithubPrDetails
      {
        sentry_context::clear_github_pr_context();
      }
      let focus_handle = self.focus_handle(cx);
      window.focus(&focus_handle, cx);
    }

    let git_page = self.git_page.clone();
    let github_page = self.github_page.clone();
    let github_profile_page = self.github_profile_page.clone();
    let github_repo_page = self.github_repo_page.clone();
    let github_pr_details_page = self.github_pr_details_page.clone();
    let github_commit_details_page = self.github_commit_details_page.clone();
    let billing_page = self.billing_page.clone();
    let git_config_page = self.git_config_page.clone();
    let settings_page = self.settings_page.clone();
    let about_page = self.about_page.clone();

    let routes = Routes::new()
      .child(
        Route::new()
          .path("git")
          .element(move |_w, _cx| git_page.clone()),
      )
      .child(Route::new().path("github").element({
        let github_page = github_page.clone();
        move |_w, _cx| github_page.clone()
      }))
      .child(Route::new().path("github/{login}").element({
        let github_profile_page = github_profile_page.clone();
        move |_w, _cx| github_profile_page.clone()
      }))
      .child(Route::new().path("github/{owner}/{repo}").element({
        let github_repo_page = github_repo_page.clone();
        move |_w, _cx| github_repo_page.clone()
      }))
      .child(
        Route::new()
          .path("github/{owner}/{repo}/commit/{sha}")
          .element({
            let github_commit_details_page = github_commit_details_page.clone();
            move |_w, _cx| github_commit_details_page.clone()
          }),
      )
      .child(Route::new().path("github/{owner}/{repo}/{tab}").element({
        let github_repo_page = github_repo_page.clone();
        move |_w, _cx| github_repo_page.clone()
      }))
      .child(
        Route::new()
          .path("github/{owner}/{repo}/pull/{number}")
          .element({
            let github_pr_details_page = github_pr_details_page.clone();
            move |_w, _cx| github_pr_details_page.clone()
          }),
      )
      .child(
        Route::new()
          .path("github/{owner}/{repo}/pull/{number}/{tab}")
          .element(move |_w, _cx| github_pr_details_page.clone()),
      )
      .child(
        Route::new()
          .path("billing")
          .element(move |_w, _cx| billing_page.clone()),
      )
      .child(
        Route::new()
          .path("git-config")
          .element(move |_w, _cx| git_config_page.clone()),
      )
      .child(
        Route::new()
          .path("settings")
          .element(move |_w, _cx| settings_page.clone()),
      )
      .child(
        Route::new()
          .path("about")
          .element(move |_w, _cx| about_page.clone()),
      );

    let key_context = shortcuts::current_key_context_for_pathname(&pathname, cx);

    div()
      .size_full()
      .flex()
      .flex_col()
      .key_context(key_context.as_str())
      .on_action(cx.listener(|_, _: &crate::OpenGitPage, _window, cx| {
        NavigationHistory::navigate("/git", cx);
      }))
      .on_action(cx.listener(|_, _: &crate::OpenGithubPage, _window, cx| {
        Self::open_github_home(cx);
      }))
      .on_action(cx.listener(Self::navigate_back_action))
      .on_action(cx.listener(Self::refresh_current_page_action))
      .on_action(cx.listener(|_, _: &crate::OpenBillingPage, _window, cx| {
        if AuthStateStore::should_show_billing_entry(cx) {
          NavigationHistory::navigate("/billing", cx);
        }
      }))
      .on_action(cx.listener(|_, _: &crate::OpenGitConfigPage, _window, cx| {
        NavigationHistory::navigate("/git-config", cx);
      }))
      .on_action(cx.listener(|_, _: &crate::OpenSettingsPage, _window, cx| {
        NavigationHistory::navigate("/settings", cx);
      }))
      .on_action(cx.listener(|_, _: &crate::OpenAboutPage, _window, cx| {
        NavigationHistory::navigate("/about", cx);
      }))
      .child(self.render_global_bar(window, page, &pathname, cx))
      .child(div().flex_1().min_h_0().child(routes))
      .into_any_element()
  }
}

#[cfg(test)]
mod tests {
  use super::{
    WorkspacePage, WorkspaceView, build_app_menus, github_primary_navigation_count_label,
    page_has_file_search, page_supports_refresh, primary_navigation_selected_index,
    refresh_label_for_workspace_page, should_run_scheduled_update_check,
    user_menu_page_for_workspace_page, workspace_page_from_pathname,
  };
  use crate::app_update::{
    AppUpdateState, AvailableAppUpdate, ReadyToInstallAppUpdate, UpdateArtifact,
    ready_update_button_label,
  };
  use crate::shortcuts::{self, ShortcutId};
  use gpui::{Keystroke, Menu, MenuItem};
  use std::path::PathBuf;
  use ui::UserMenuPage;

  fn action_menu_item_names(menu: &Menu) -> Vec<String> {
    menu
      .items
      .iter()
      .filter_map(|item| match item {
        MenuItem::Action { name, .. } => Some(name.to_string()),
        _ => None,
      })
      .collect()
  }

  fn make_available_update() -> AvailableAppUpdate {
    AvailableAppUpdate {
      latest_version: "0.2.0".to_string(),
      minimum_supported_version: "0.1.0".to_string(),
      release_notes_url: "https://reviu.dev/changelog".to_string(),
      force_update: false,
      artifact: UpdateArtifact {
        url: "https://reviu.dev/downloads/reviu-0.2.0.dmg".to_string(),
        sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        size: 123,
      },
    }
  }

  #[test]
  fn workspace_page_from_pathname_maps_static_paths() {
    assert_eq!(workspace_page_from_pathname("/git"), WorkspacePage::Git);
    assert_eq!(
      workspace_page_from_pathname("/github"),
      WorkspacePage::Github
    );
    assert_eq!(
      workspace_page_from_pathname("/billing"),
      WorkspacePage::Billing
    );
    assert_eq!(
      workspace_page_from_pathname("/settings"),
      WorkspacePage::Settings
    );
    assert_eq!(
      workspace_page_from_pathname("/git-config"),
      WorkspacePage::GitConfig
    );
    assert_eq!(workspace_page_from_pathname("/about"), WorkspacePage::About);
  }

  #[test]
  fn build_app_menus_hides_billing_when_entry_is_unavailable() {
    let menus = build_app_menus(false);
    let navigate_menu = menus
      .iter()
      .find(|menu| menu.name == "Navigate")
      .expect("navigate menu");

    assert_eq!(
      action_menu_item_names(navigate_menu),
      vec!["Back", "Git", "GitHub", "Git Config"]
    );
  }

  #[test]
  fn build_app_menus_shows_billing_when_entry_is_available() {
    let menus = build_app_menus(true);
    let navigate_menu = menus
      .iter()
      .find(|menu| menu.name == "Navigate")
      .expect("navigate menu");

    assert_eq!(
      action_menu_item_names(navigate_menu),
      vec!["Back", "Git", "GitHub", "Git Config", "Billing"]
    );
  }

  #[test]
  fn scheduled_update_check_runs_without_known_update() {
    assert!(should_run_scheduled_update_check(None));
    assert!(should_run_scheduled_update_check(Some(
      AppUpdateState::Error {
        update: None,
        message: "network timeout".to_string(),
      }
    )));
  }

  #[test]
  fn scheduled_update_check_skips_when_update_is_already_known() {
    let update = make_available_update();
    let ready = ReadyToInstallAppUpdate {
      update: update.clone(),
      artifact_path: PathBuf::from("/tmp/reviu.dmg"),
      restart_binary_path: None,
    };

    assert!(!should_run_scheduled_update_check(Some(
      AppUpdateState::Available(update.clone())
    )));
    assert!(!should_run_scheduled_update_check(Some(
      AppUpdateState::Downloading(update.clone())
    )));
    assert!(!should_run_scheduled_update_check(Some(
      AppUpdateState::ReadyToInstall(ready)
    )));
    assert!(!should_run_scheduled_update_check(Some(
      AppUpdateState::Error {
        update: Some(update),
        message: "checksum mismatch".to_string(),
      }
    )));
  }

  #[test]
  fn workspace_page_from_pathname_maps_github_repo() {
    assert_eq!(
      workspace_page_from_pathname("/github/octocat"),
      WorkspacePage::GithubProfile
    );
    assert_eq!(
      workspace_page_from_pathname("/github/owner/repo"),
      WorkspacePage::GithubRepo
    );
    assert_eq!(
      workspace_page_from_pathname("/github/owner/repo/code"),
      WorkspacePage::GithubRepo
    );
    assert_eq!(
      workspace_page_from_pathname("/github/owner/repo/issues"),
      WorkspacePage::GithubRepo
    );
  }

  #[test]
  fn workspace_page_from_pathname_maps_github_pr_details() {
    assert_eq!(
      workspace_page_from_pathname("/github/owner/repo/pull/123"),
      WorkspacePage::GithubPrDetails
    );
    assert_eq!(
      workspace_page_from_pathname("/github/owner/repo/pull/123/changes"),
      WorkspacePage::GithubPrDetails
    );
    assert_eq!(
      workspace_page_from_pathname("/github/owner/repo/pull/123/checks"),
      WorkspacePage::GithubPrDetails
    );
  }

  #[test]
  fn workspace_page_from_pathname_maps_github_commit_details() {
    assert_eq!(
      workspace_page_from_pathname("/github/owner/repo/commit/abc123"),
      WorkspacePage::GithubCommitDetails
    );
  }

  #[test]
  fn page_has_file_search_matches_correct_paths() {
    assert!(page_has_file_search("/git"));
    assert!(page_has_file_search("/github/owner/repo/code"));
    assert!(page_has_file_search("/github/owner/repo/pull/123/changes"));
    assert!(!page_has_file_search("/github"));
    assert!(!page_has_file_search("/github/owner/repo"));
    assert!(!page_has_file_search("/github/owner/repo/commit/abc123"));
    assert!(!page_has_file_search("/github/owner/repo/pull/123"));
    assert!(!page_has_file_search("/github/owner/repo/pulls"));
    assert!(!page_has_file_search("/settings"));
  }

  #[test]
  fn workspace_page_from_pathname_unknown_falls_back_to_git() {
    assert_eq!(workspace_page_from_pathname("/unknown"), WorkspacePage::Git);
    assert_eq!(workspace_page_from_pathname("/"), WorkspacePage::Git);
  }

  #[test]
  fn user_menu_page_for_workspace_maps_repo_to_github() {
    assert_eq!(
      user_menu_page_for_workspace_page(WorkspacePage::GithubRepo),
      UserMenuPage::Github
    );
    assert_eq!(
      user_menu_page_for_workspace_page(WorkspacePage::GithubProfile),
      UserMenuPage::Github
    );
    assert_eq!(
      user_menu_page_for_workspace_page(WorkspacePage::GithubPrDetails),
      UserMenuPage::GithubPrDetails
    );
    assert_eq!(
      user_menu_page_for_workspace_page(WorkspacePage::GithubCommitDetails),
      UserMenuPage::Github
    );
  }

  #[test]
  fn primary_navigation_selected_index_matches_top_level_sections() {
    assert_eq!(
      primary_navigation_selected_index(WorkspacePage::Git),
      Some(0)
    );
    assert_eq!(
      primary_navigation_selected_index(WorkspacePage::Github),
      Some(1)
    );
    assert_eq!(
      primary_navigation_selected_index(WorkspacePage::GithubProfile),
      Some(1)
    );
    assert_eq!(
      primary_navigation_selected_index(WorkspacePage::GithubRepo),
      Some(1)
    );
    assert_eq!(
      primary_navigation_selected_index(WorkspacePage::GithubPrDetails),
      Some(1)
    );
    assert_eq!(
      primary_navigation_selected_index(WorkspacePage::GithubCommitDetails),
      Some(1)
    );
    assert_eq!(
      primary_navigation_selected_index(WorkspacePage::Billing),
      None
    );
    assert_eq!(
      primary_navigation_selected_index(WorkspacePage::GitConfig),
      None
    );
    assert_eq!(
      primary_navigation_selected_index(WorkspacePage::Settings),
      None
    );
    assert_eq!(
      primary_navigation_selected_index(WorkspacePage::About),
      None
    );
  }

  #[test]
  fn workspace_refresh_support_matches_git_and_github_surfaces() {
    assert!(page_supports_refresh(WorkspacePage::Git));
    assert!(page_supports_refresh(WorkspacePage::Github));
    assert!(page_supports_refresh(WorkspacePage::GithubProfile));
    assert!(page_supports_refresh(WorkspacePage::GithubRepo));
    assert!(page_supports_refresh(WorkspacePage::GithubPrDetails));
    assert!(page_supports_refresh(WorkspacePage::GithubCommitDetails));
    assert!(!page_supports_refresh(WorkspacePage::Billing));
    assert!(!page_supports_refresh(WorkspacePage::GitConfig));
    assert!(!page_supports_refresh(WorkspacePage::Settings));
    assert!(!page_supports_refresh(WorkspacePage::About));
  }

  #[test]
  fn refresh_shortcut_matches_default_cmd_r() {
    assert_eq!(
      shortcuts::shortcut_keystroke(ShortcutId::RefreshCurrentPage),
      Keystroke::parse("cmd-r").expect("cmd-r keystroke")
    );
  }

  #[test]
  fn refresh_label_for_workspace_page_matches_page_context() {
    assert_eq!(
      refresh_label_for_workspace_page(WorkspacePage::Git),
      Some("Refresh Git")
    );
    assert_eq!(
      refresh_label_for_workspace_page(WorkspacePage::Github),
      Some("Refresh GitHub")
    );
    assert_eq!(
      refresh_label_for_workspace_page(WorkspacePage::GithubProfile),
      Some("Refresh Profile")
    );
    assert_eq!(
      refresh_label_for_workspace_page(WorkspacePage::GithubRepo),
      Some("Refresh Repo")
    );
    assert_eq!(
      refresh_label_for_workspace_page(WorkspacePage::GithubPrDetails),
      Some("Refresh PR")
    );
    assert_eq!(
      refresh_label_for_workspace_page(WorkspacePage::GithubCommitDetails),
      Some("Refresh Commit")
    );
    assert_eq!(
      refresh_label_for_workspace_page(WorkspacePage::Billing),
      None
    );
  }

  #[test]
  fn github_primary_navigation_count_label_hides_zero_count() {
    assert_eq!(github_primary_navigation_count_label(0), None);
    assert_eq!(
      github_primary_navigation_count_label(7),
      Some("7".to_string())
    );
  }

  #[test]
  fn workspace_update_button_label_tracks_update_state() {
    let update = AvailableAppUpdate {
      latest_version: "0.2.0".to_string(),
      minimum_supported_version: "0.1.0".to_string(),
      release_notes_url: "https://reviu.dev/releases/0.2.0".to_string(),
      force_update: false,
      artifact: UpdateArtifact {
        url: "https://reviu.dev/downloads/latest".to_string(),
        sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        size: 1024,
      },
    };

    assert_eq!(
      WorkspaceView::update_button_label(None),
      "New version available"
    );
    assert_eq!(
      WorkspaceView::update_button_label(Some(AppUpdateState::Available(update.clone()))),
      "New version available"
    );
    assert_eq!(
      WorkspaceView::update_button_label(Some(AppUpdateState::Downloading(update.clone()))),
      "Downloading..."
    );
    assert_eq!(
      WorkspaceView::update_button_label(Some(AppUpdateState::ReadyToInstall(
        ReadyToInstallAppUpdate {
          update,
          artifact_path: PathBuf::from("/tmp/reviu-installer.dmg"),
          restart_binary_path: None,
        }
      ))),
      ready_update_button_label()
    );
  }

  #[test]
  fn workspace_command_palette_shortcut_matches_global_binding() {
    assert_eq!(
      WorkspaceView::command_palette_shortcut(),
      shortcuts::shortcut_keystroke(ShortcutId::ShowCommandPalette)
    );
  }
}

impl Focusable for WorkspaceView {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    match WorkspaceRoute::global(cx).page {
      WorkspacePage::Git => self.git_page.read(cx).focus_handle(cx),
      WorkspacePage::Github => self.github_page.read(cx).focus_handle(cx),
      WorkspacePage::GithubProfile => self.github_profile_page.read(cx).focus_handle(cx),
      WorkspacePage::GithubRepo => self.github_repo_page.read(cx).focus_handle(cx),
      WorkspacePage::GithubPrDetails => self.github_pr_details_page.read(cx).focus_handle(cx),
      WorkspacePage::GithubCommitDetails => {
        self.github_commit_details_page.read(cx).focus_handle(cx)
      }
      WorkspacePage::Billing => self.billing_page.read(cx).focus_handle(cx),
      WorkspacePage::GitConfig => self.git_config_page.read(cx).focus_handle(cx),
      WorkspacePage::Settings => self.settings_page.read(cx).focus_handle(cx),
      WorkspacePage::About => self.about_page.read(cx).focus_handle(cx),
    }
  }
}
