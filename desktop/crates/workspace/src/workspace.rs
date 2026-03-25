use std::rc::Rc;
use std::time::Duration;

use editor::set_indent_rainbow_enabled;
use gpui::{
  AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, Global, Keystroke, Render,
  Subscription, Task, Window, div, prelude::*, px,
};
use gpui_router::{Route, Routes};
use gpui_component::{
  ActiveTheme as _, Disableable, IconName, Sizable as _, Theme, ThemeMode, kbd::Kbd,
  notification::Notification, tag::Tag,
};
use smol::unblock;

use crate::AppProfile;
use crate::navigation::NavigationHistory;
use crate::AuthCallbackTarget;
use crate::about_page::AboutPage;
use crate::active_local_repo::ActiveLocalRepoStore;
use crate::api::ApiClient;
use crate::app_update::{
  AppUpdateNotificationId, AppUpdateState, AppUpdateStore, AvailableAppUpdate, UpdateArtifact,
  current_arch, current_platform, download_update_artifact, install_update_artifact,
  resolved_build_version,
};
use crate::auth_state::{AuthState, AuthStateStore};
use crate::billing_page::BillingPage;
use crate::config::{AppSettings as PersistedSettings, ConfigStore};
use crate::dock_badge::set_dock_badge;
use crate::git_config_page::GitConfigPage;
use crate::git_page::GitPage;
use crate::github_page::{GithubPage, GithubPageHandle};
use crate::github_pr_details_page::GithubPrDetailsPage;
use crate::github_repo_page::GithubRepoPage;
use crate::notification_count::NotificationCountStore;
use crate::sentry_context;
use crate::settings_page::SettingsPage;
use crate::{SHOW_COMMAND_PALETTE_SHORTCUT, ShowCommandPalette};
use ui::{
  Button, ButtonVariants as _, GLOBAL_BAR_HEIGHT, UiIconName, UserMenuConfig, UserMenuPage,
  UserMenuState, UserMenuUser, WindowExt, user_menu,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspacePage {
  Git,
  Github,
  GithubRepo,
  GithubPrDetails,
  Billing,
  GitConfig,
  Settings,
  About,
}

pub(crate) fn workspace_page_from_pathname(pathname: &str) -> WorkspacePage {
  if pathname.starts_with("/github/") {
    // Check if it's a PR details path: /github/{owner}/{repo}/pull/{number}
    let segments: Vec<&str> = pathname.trim_start_matches('/').split('/').collect();
    if segments.len() >= 5 && segments[3] == "pull" {
      return WorkspacePage::GithubPrDetails;
    }
    // Otherwise it's a repo page: /github/{owner}/{repo}
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

fn user_menu_page_for_workspace_page(page: WorkspacePage) -> UserMenuPage {
  match page {
    WorkspacePage::Git => UserMenuPage::Git,
    WorkspacePage::Github | WorkspacePage::GithubRepo => UserMenuPage::Github,
    WorkspacePage::GithubPrDetails => UserMenuPage::GithubPrDetails,
    WorkspacePage::Billing => UserMenuPage::Billing,
    WorkspacePage::GitConfig => UserMenuPage::GitConfig,
    WorkspacePage::Settings => UserMenuPage::Settings,
    WorkspacePage::About => UserMenuPage::About,
  }
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
  github_repo_page: Entity<GithubRepoPage>,
  github_pr_details_page: Entity<GithubPrDetailsPage>,
  billing_page: Entity<BillingPage>,
  settings_page: Entity<SettingsPage>,
  about_page: Entity<AboutPage>,
  window_handle: AnyWindowHandle,
  last_page: Option<WorkspacePage>,
  _update_check_task: Option<Task<()>>,
  _update_download_task: Option<Task<()>>,
  _notification_poll_task: Option<Task<()>>,
  _subscriptions: Vec<Subscription>,
}

impl WorkspaceView {
  const GLOBAL_BAR_MACOS_LEFT_PADDING: f32 = 85.0;

  fn command_palette_shortcut() -> Keystroke {
    Keystroke::parse(SHOW_COMMAND_PALETTE_SHORTCUT).expect("valid command palette shortcut")
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
    let github_repo_page = cx.new(|cx| GithubRepoPage::new(window, cx));
    let github_pr_details_page = cx.new(|cx| GithubPrDetailsPage::new(window, cx));
    let billing_page = cx.new(|cx| BillingPage::new(window, cx));
    let settings_page = cx.new(|cx| SettingsPage::new(window, cx, settings));
    let about_page = cx.new(|cx| AboutPage::new(window, cx));

    let view = Self {
      git_page,
      git_config_page,
      github_page,
      github_repo_page,
      github_pr_details_page,
      billing_page,
      settings_page,
      about_page,
      window_handle: window.window_handle(),
      last_page: None,
      _update_check_task: None,
      _update_download_task: None,
      _notification_poll_task: None,
      _subscriptions: Vec::new(),
    };

    let mut view = view;
    let subscription = cx.observe_window_appearance(window, |this, window, cx| {
      this.on_window_appearance_changed(window, cx);
    });
    view._subscriptions.push(subscription);
    view.check_for_updates(cx);
    view.start_notification_polling(cx);

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

  fn start_notification_polling(&mut self, cx: &mut Context<Self>) {
    let api = WorkspaceApi::global(cx).api.clone();
    let task = cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_secs(60))
          .await;

        let has_access = this
          .update(cx, |_, cx| AuthStateStore::has_pro_access(cx))
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
            cx.refresh_windows();
          }
        });
      }
    });

    self._notification_poll_task = Some(task);
  }

  fn trigger_update_download(&mut self, cx: &mut Context<Self>) {
    if AppUpdateStore::is_downloading(cx) {
      return;
    }

    if AppUpdateStore::try_ready_to_install(cx).is_some() {
      cx.restart();
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
          .action(move |_, _, _cx| {
            let view = view.clone();
            Button::new("workspace-update-download")
              .primary()
              .icon(UiIconName::Download)
              .label("Download")
              .on_click(move |_, window, cx| {
                view.update(cx, |this, cx| this.trigger_update_download(cx));
                window.on_next_frame(|window, cx| {
                  window.remove_notification::<AppUpdateNotificationId>(cx);
                });
              })
          }),
        cx,
      );
    });
  }

  fn update_button_label(state: Option<AppUpdateState>) -> &'static str {
    match state {
      Some(AppUpdateState::Downloading(_)) => "Downloading...",
      Some(AppUpdateState::ReadyToInstall(_)) => "Restart to update",
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
    GithubPageHandle::refresh(cx);
    NavigationHistory::navigate("/github", cx);
  }

  fn render_global_bar(&self, page: WorkspacePage, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let show_update_button = AppUpdateStore::try_available_update(cx).is_some();
    let update_download_in_progress = AppUpdateStore::is_downloading(cx);
    let update_button_label = Self::update_button_label(AppUpdateStore::try_state(cx));

    let current_page = user_menu_page_for_workspace_page(page);
    let auth_state = AuthStateStore::get(cx);
    let is_unauthenticated = matches!(auth_state, AuthState::Unauthenticated);

    let open_git = Rc::new(|_window: &mut Window, cx: &mut App| {
      NavigationHistory::navigate("/git", cx);
    });
    let open_github = Rc::new(|_window: &mut Window, cx: &mut App| {
      Self::open_github_home(cx);
    });
    let open_billing = Rc::new(|_window: &mut Window, cx: &mut App| {
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

        user_menu(UserMenuConfig {
          id: "workspace-auth-menu".into(),
          state: UserMenuState::Authenticated(UserMenuUser {
            name: display_name.into(),
            email: user.email.into(),
            image: user.image.map(Into::into),
          }),
          current_page,
          notification_count: NotificationCountStore::get(cx),
          on_open_git: Some(open_git),
          on_open_github: Some(open_github),
          on_open_billing: Some(open_billing),
          on_open_git_config: Some(open_git_config),
          on_open_settings: Some(open_settings),
          on_open_about: Some(open_about),
          on_sign_in: Some(sign_in),
          on_sign_out: Some(sign_out),
        })
      }
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

    let command_palette_button = Button::new("workspace-global-command-palette")
      .label("Command palette")
      .ghost()
      .compact()
      .small()
      .child(Kbd::new(Self::command_palette_shortcut()).ml_1())
      .on_click(|_, window, cx| {
        window.dispatch_action(Box::new(ShowCommandPalette), cx);
      });

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
    right = right.child(command_palette_button);
    if is_unauthenticated {
      right = right.child(sign_in_button);
    }
    if let Some(auth_control) = auth_control {
      right = right.child(auth_control);
    }

    let left = if let Some(label) = AppProfile::current().header_tag_label() {
      div().child(Tag::secondary().small().rounded_full().child(label))
    } else {
      div()
    };

    bar.child(left).child(right)
  }
}

impl Render for WorkspaceView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let pathname = NavigationHistory::current_pathname(cx);
    let page = workspace_page_from_pathname(&pathname);

    // Sync sentry context on page change
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

    // Keep WorkspaceRoute in sync for code that still reads it
    cx.global_mut::<WorkspaceRoute>().page = page;

    let git_page = self.git_page.clone();
    let github_page = self.github_page.clone();
    let github_repo_page = self.github_repo_page.clone();
    let github_pr_details_page = self.github_pr_details_page.clone();
    let billing_page = self.billing_page.clone();
    let git_config_page = self.git_config_page.clone();
    let settings_page = self.settings_page.clone();
    let about_page = self.about_page.clone();

    let routes = Routes::new()
      .child(Route::new().path("git").element(move |_w, _cx| git_page.clone()))
      .child(Route::new().path("github").element({
        let github_page = github_page.clone();
        move |_w, _cx| github_page.clone()
      }))
      .child(Route::new().path("github/{owner}/{repo}").element({
        let github_repo_page = github_repo_page.clone();
        move |_w, _cx| github_repo_page.clone()
      }))
      .child(
        Route::new()
          .path("github/{owner}/{repo}/pull/{number}")
          .element(move |_w, _cx| github_pr_details_page.clone()),
      )
      .child(Route::new().path("billing").element(move |_w, _cx| billing_page.clone()))
      .child(Route::new().path("git-config").element(move |_w, _cx| git_config_page.clone()))
      .child(Route::new().path("settings").element(move |_w, _cx| settings_page.clone()))
      .child(Route::new().path("about").element(move |_w, _cx| about_page.clone()));

    div()
      .size_full()
      .flex()
      .flex_col()
      .child(self.render_global_bar(page, cx))
      .child(div().flex_1().min_h_0().child(routes))
  }
}

#[cfg(test)]
mod tests {
  use super::{WorkspacePage, WorkspaceView, user_menu_page_for_workspace_page, workspace_page_from_pathname};
  use crate::SHOW_COMMAND_PALETTE_SHORTCUT;
  use crate::app_update::{
    AppUpdateState, AvailableAppUpdate, ReadyToInstallAppUpdate, UpdateArtifact,
  };
  use gpui::Keystroke;
  use std::path::PathBuf;
  use ui::UserMenuPage;

  #[test]
  fn workspace_page_from_pathname_maps_static_paths() {
    assert_eq!(workspace_page_from_pathname("/git"), WorkspacePage::Git);
    assert_eq!(workspace_page_from_pathname("/github"), WorkspacePage::Github);
    assert_eq!(workspace_page_from_pathname("/billing"), WorkspacePage::Billing);
    assert_eq!(workspace_page_from_pathname("/settings"), WorkspacePage::Settings);
    assert_eq!(workspace_page_from_pathname("/git-config"), WorkspacePage::GitConfig);
    assert_eq!(workspace_page_from_pathname("/about"), WorkspacePage::About);
  }

  #[test]
  fn workspace_page_from_pathname_maps_github_repo() {
    assert_eq!(
      workspace_page_from_pathname("/github/owner/repo"),
      WorkspacePage::GithubRepo
    );
  }

  #[test]
  fn workspace_page_from_pathname_maps_github_pr_details() {
    assert_eq!(
      workspace_page_from_pathname("/github/owner/repo/pull/123"),
      WorkspacePage::GithubPrDetails
    );
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
      user_menu_page_for_workspace_page(WorkspacePage::GithubPrDetails),
      UserMenuPage::GithubPrDetails
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
        }
      ))),
      "Restart to update"
    );
  }

  #[test]
  fn workspace_command_palette_shortcut_matches_global_binding() {
    assert_eq!(
      WorkspaceView::command_palette_shortcut(),
      Keystroke::parse(SHOW_COMMAND_PALETTE_SHORTCUT).expect("valid shortcut")
    );
  }
}

impl Focusable for WorkspaceView {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    match WorkspaceRoute::global(cx).page {
      WorkspacePage::Git => self.git_page.read(cx).focus_handle(cx),
      WorkspacePage::Github => self.github_page.read(cx).focus_handle(cx),
      WorkspacePage::GithubRepo => self.github_repo_page.read(cx).focus_handle(cx),
      WorkspacePage::GithubPrDetails => self.github_pr_details_page.read(cx).focus_handle(cx),
      WorkspacePage::Billing => self.billing_page.read(cx).focus_handle(cx),
      WorkspacePage::GitConfig => self.git_config_page.read(cx).focus_handle(cx),
      WorkspacePage::Settings => self.settings_page.read(cx).focus_handle(cx),
      WorkspacePage::About => self.about_page.read(cx).focus_handle(cx),
    }
  }
}
