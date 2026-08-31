use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use editor::{Copy, Cut, Paste, Quit, Redo, SelectAll, Undo, set_indent_rainbow_enabled};
#[cfg(test)]
use gpui::Keystroke;
use gpui::{
  AnyWindowHandle, App, Context, Decorations, Entity, FocusHandle, Focusable, Global, Menu,
  MenuItem, Render, Subscription, Task, Window, WindowButton, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, Sizable as _, Theme, ThemeMode, h_flex, kbd::Kbd,
  notification::Notification, spinner::Spinner, tag::Tag,
};

use crate::AppProfile;
use crate::about_dialog::open_about_dialog;
use crate::api::ApiClient;
use crate::app_update::{
  AppUpdateNotificationId, AppUpdateState, AppUpdateStore, AvailableAppUpdate, UpdateArtifact,
  current_arch, current_platform, resolved_build_version, start_update_download,
  update_action_label,
};
use crate::auth_state::{AuthState, AuthStateStore};
use crate::billing_dialog::open_billing_dialog;
use crate::config::{AppSettings as PersistedSettings, ConfigStore};
use crate::git_config_page::open_git_config_dialog;
use crate::github_notifications::{self, GithubNotificationsStore};
use crate::navigation::NavigationHistory;
use crate::sentry_context;
use crate::session_page::SessionPage;
use crate::settings_page::open_settings_dialog;
use crate::shortcuts::{self, ShortcutId};
use crate::workspace_window::WorkspaceWindow;
use crate::{ShowCommandPalette, ShowFileSearch};
use ui::{
  Button, ButtonVariants as _, GLOBAL_BAR_HEIGHT, REVIU_WORDMARK_WIDTH_PX, StatusThemeExt,
  UiIconName, UserMenuConfig, UserMenuPage, UserMenuState, UserMenuUser, WindowExt,
  reviu_logo_path, user_menu,
};

type NavigateFn = dyn Fn(&mut Window, &mut App);

const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);

pub const STATUS_BAR_ICON_PNG: &[u8] =
  include_bytes!("../../reviu/assets/reviu_status_bar_icon.png");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspacePage {
  Session,
}

pub(crate) fn workspace_page_from_pathname(_pathname: &str) -> WorkspacePage {
  WorkspacePage::Session
}

/// Returns true when the current path supports file search.
/// The shell is the only page with files to search.
fn page_has_file_search(pathname: &str) -> bool {
  workspace_page_from_pathname(pathname) == WorkspacePage::Session
}

fn user_menu_page_for_workspace_page(_page: WorkspacePage) -> UserMenuPage {
  UserMenuPage::Session
}

/// The shell connects its agent when the workspace routes to it, never while
/// painting: entering the page is the only signal.
fn should_activate_session_page(previous: Option<WorkspacePage>, next: WorkspacePage) -> bool {
  next == WorkspacePage::Session && previous != Some(WorkspacePage::Session)
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

fn update_button_tooltip(state: Option<&AppUpdateState>) -> String {
  match state {
    Some(AppUpdateState::Available(update)) => {
      format!("Download Reviu {}", update.latest_version)
    }
    Some(AppUpdateState::Downloading(_)) => "Downloading update...".to_string(),
    Some(AppUpdateState::ReadyToInstall(_)) => update_action_label(state.cloned()).to_string(),
    Some(AppUpdateState::Error {
      update: Some(_), ..
    }) => "Update failed. Try again.".to_string(),
    _ => "New version available".to_string(),
  }
}

pub fn build_app_menus() -> Vec<Menu> {
  build_app_menus_with_subscription(false)
}

pub(crate) fn build_app_menus_for_current_auth(cx: &App) -> Vec<Menu> {
  build_app_menus_with_subscription(AuthStateStore::has_subscription(cx))
}

fn build_app_menus_with_subscription(has_subscription: bool) -> Vec<Menu> {
  let billing_label = if has_subscription {
    "Billing"
  } else {
    "Reviu Pro"
  };

  vec![
    Menu {
      name: "Reviu".into(),
      disabled: false,
      items: vec![
        MenuItem::action("About Reviu", crate::OpenAboutPage),
        MenuItem::separator(),
        MenuItem::action("Settings...", crate::OpenSettingsPage),
        MenuItem::action("Git Config...", crate::OpenGitConfigPage),
        MenuItem::action("Browser Extension...", crate::OpenBrowserExtensions),
        MenuItem::separator(),
        MenuItem::action(billing_label, crate::OpenBillingPage),
        MenuItem::separator(),
        MenuItem::action("Quit Reviu", Quit),
      ],
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
  session_page: Entity<SessionPage>,
  window_handle: AnyWindowHandle,
  last_page: Option<WorkspacePage>,
  _update_check_task: Option<Task<()>>,
  _periodic_update_check_task: Option<Task<()>>,
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

  fn sync_app_menus(cx: &mut App) {
    cx.set_menus(build_app_menus_for_current_auth(cx));
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    if let Some(dir) = AppProfile::current().config_dir() {
      agent_chat_panel::set_settings_dir(dir);
    }

    let settings = ConfigStore::load_app_settings();
    if let Some(error) = crate::settings_file::take_startup_error() {
      window.push_notification(
        Notification::new()
          .title("Your settings could not be read")
          .message(format!("Reviu started with default settings. {error}"))
          .autohide(false),
        cx,
      );
    }

    gpui_router::init(cx);
    NavigationHistory::init(cx);
    NavigationHistory::navigate_replace("/session", cx);

    cx.set_global(WorkspaceApi::new());
    cx.set_global(AuthStateStore::default());
    cx.set_global(AppUpdateStore::default());
    cx.set_global(GithubNotificationsStore::default());

    cx.set_global(settings);
    cx.set_global(shortcuts::load_shortcut_overrides());
    if let Some(error) = crate::keybindings_file::take_startup_error() {
      window.push_notification(
        Notification::new()
          .title("Your keybindings could not be read")
          .message(format!("Reviu started with default shortcuts. {error}"))
          .autohide(false),
        cx,
      );
    }
    cx.set_global(crate::command_usage::CommandUsageStore::load());
    crate::command_usage::install_palette_usage_recorder(cx);
    crate::analytics::Analytics::init(cx);
    crate::analytics::track(cx, "app_started");
    // Signing in is app-wide, not something a page owns.
    crate::auth_flow::load_stored_token(cx);
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

    // Deep links and the pro promise land in the app with no window of their
    // own, and both have to open the billing dialog.
    WorkspaceWindow::register(window.window_handle(), cx);

    let session_page = cx.new(|cx| SessionPage::new(window, cx));

    let view = Self {
      session_page,
      window_handle: window.window_handle(),
      last_page: None,
      _update_check_task: None,
      _periodic_update_check_task: None,
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

  #[doc(hidden)]
  pub fn open_repository_for_driver(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), gpui::SharedString> {
    self.session_page.update(cx, |page, cx| {
      page.open_repository_for_driver(repo_root, window, cx)
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn open_file_for_driver(
    &mut self,
    rel_path: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), gpui::SharedString> {
    self.session_page.update(cx, |page, cx| {
      page.open_file_for_driver(rel_path, window, cx)
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn open_pull_request_file_for_driver(
    &mut self,
    rel_path: Option<PathBuf>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), gpui::SharedString> {
    self.session_page.update(cx, |page, cx| {
      page.open_pull_request_file_for_driver(rel_path, window, cx)
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn git_state_for_driver(&self, cx: &App) -> serde_json::Value {
    self
      .session_page
      .read_with(cx, |page, cx| page.git_state_for_driver(cx))
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn notification_log_for_driver(&self, cx: &App) -> serde_json::Value {
    self
      .session_page
      .read_with(cx, |page, _| page.notification_log_for_driver())
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn set_auth_token_for_driver(&self, token: String, cx: &mut App) {
    WorkspaceApi::global(cx).api.set_bearer_token(token);
    crate::auth_flow::refresh_me(cx);
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn auth_state_for_driver(&self, cx: &App) -> serde_json::Value {
    let state = AuthStateStore::get(cx);
    let github_access = AuthStateStore::github_access_state(cx);
    serde_json::json!({
      "status": match &state {
        AuthState::Unknown => "unknown",
        AuthState::Unauthenticated => "unauthenticated",
        AuthState::Authenticated(_) => "authenticated",
      },
      "github_access": match github_access {
        crate::auth_state::GithubAccessState::NeedsSignIn => "needs_sign_in",
        crate::auth_state::GithubAccessState::NeedsSubscription => "needs_subscription",
        crate::auth_state::GithubAccessState::Available => "available",
      },
      "signed_in": AuthStateStore::is_signed_in(cx),
      "has_subscription": AuthStateStore::has_subscription(cx),
      "has_github_access": AuthStateStore::has_github_access(cx),
      "github_login": state.github_login(),
      "app_profile": if AppProfile::current().is_dev() { "dev" } else { "prod" },
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn run_git_action_for_driver(
    &mut self,
    action: crate::DriverGitAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), gpui::SharedString> {
    self.session_page.update(cx, |page, cx| {
      page.run_git_action_for_driver(action, window, cx)
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn show_changes_for_driver(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self
      .session_page
      .update(cx, |page, cx| page.show_changes_for_driver(window, cx));
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn show_pull_request_for_driver(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self
      .session_page
      .update(cx, |page, cx| page.show_pull_request_for_driver(window, cx));
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn show_review_for_driver(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self
      .session_page
      .update(cx, |page, cx| page.show_review_for_driver(window, cx));
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn create_pull_request_review_comment_for_driver(
    &mut self,
    rel_path: PathBuf,
    line: usize,
    body: String,
    cx: &mut Context<Self>,
  ) -> Result<(), gpui::SharedString> {
    self.session_page.update(cx, |page, cx| {
      page.create_pull_request_review_comment_for_driver(rel_path, line, body, cx)
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn submit_pull_request_review_for_driver(
    &mut self,
    body: String,
    cx: &mut Context<Self>,
  ) -> Result<(), gpui::SharedString> {
    self.session_page.update(cx, |page, cx| {
      page.submit_pull_request_review_for_driver(body, cx)
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn discard_pull_request_review_for_driver(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.session_page.update(cx, |page, cx| {
      page.discard_pull_request_review_for_driver(window, cx)
    });
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn hide_dock_for_driver(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self
      .session_page
      .update(cx, |page, cx| page.hide_dock_for_driver(window, cx));
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn submit_agent_prompt_for_driver(
    &mut self,
    text: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), gpui::SharedString> {
    self.session_page.update(cx, |page, cx| {
      page.submit_agent_prompt_for_driver(text, window, cx)
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn agent_stats_for_driver(&self, cx: &App) -> serde_json::Value {
    self.session_page.read(cx).agent_stats_for_driver(cx)
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn editor_stats_for_driver(&self, cx: &App) -> serde_json::Value {
    self.session_page.read(cx).editor_stats_for_driver(cx)
  }

  fn on_window_appearance_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !PersistedSettings::get(cx).auto_switch_theme {
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
    github_notifications::open_notification(&notification, cx);
  }

  fn check_for_updates(&mut self, cx: &mut Context<Self>) {
    let api = WorkspaceApi::global(cx).api.clone();
    let current_version = resolved_build_version(env!("CARGO_PKG_VERSION"));
    let platform = current_platform().to_string();
    let arch = current_arch().to_string();
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(
          async move { api.check_desktop_update(&current_version, &platform, &arch) },
        )
        .await;
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
        let result = cx
          .background_spawn(async move { api.fetch_github_notifications() })
          .await;

        let _ = this.update(cx, |_, cx| {
          if let Ok(notifications) = result {
            GithubNotificationsStore::set(cx, notifications);
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
                    view.update(cx, |_, cx| start_update_download(cx));
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

  fn global_update_download_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    start_update_download(cx);
    window.on_next_frame(|window, cx| {
      window.remove_notification::<AppUpdateNotificationId>(cx);
    });
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
    let update_state = AppUpdateStore::try_state(cx);
    let show_update_button = AppUpdateStore::try_available_update(cx).is_some();
    let update_download_in_progress = AppUpdateStore::is_downloading(cx);
    let update_button_tooltip = update_button_tooltip(update_state.as_ref());

    let current_page = user_menu_page_for_workspace_page(page);
    let auth_state = AuthStateStore::get(cx);
    let is_unauthenticated = matches!(auth_state, AuthState::Unauthenticated);

    let open_billing: Rc<NavigateFn> = Rc::new(|window: &mut Window, cx: &mut App| {
      open_billing_dialog(window, cx);
    });
    let open_git_config = Rc::new(|window: &mut Window, cx: &mut App| {
      open_git_config_dialog(window, cx);
    });
    let open_settings = Rc::new(|window: &mut Window, cx: &mut App| {
      open_settings_dialog(window, cx);
    });
    let open_about = Rc::new(|window: &mut Window, cx: &mut App| {
      open_about_dialog(window, cx);
    });
    let open_browser_extensions = Rc::new(|window: &mut Window, cx: &mut App| {
      crate::browser_extensions_dialog::open_browser_extensions_dialog(window, cx);
    });
    let sign_in = Rc::new(|_window: &mut Window, cx: &mut App| {
      crate::auth_flow::start_github_sign_in(cx, "user_menu");
    });
    let sign_out = Rc::new(|_window: &mut Window, cx: &mut App| {
      crate::auth_flow::sign_out(cx);
      GithubNotificationsStore::clear(cx);
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
          on_open_billing: Some(open_billing.clone()),
          on_open_git_config: Some(open_git_config),
          on_open_settings: Some(open_settings),
          on_open_about: Some(open_about),
          on_open_browser_extensions: Some(open_browser_extensions.clone()),
          on_sign_in: Some(sign_in),
          on_sign_out: Some(sign_out),
        })
      }
      AuthState::Unauthenticated => user_menu(UserMenuConfig {
        id: "workspace-auth-menu".into(),
        state: UserMenuState::Unauthenticated,
        current_page,
        on_open_billing: Some(open_billing),
        on_open_git_config: Some(open_git_config),
        on_open_settings: Some(open_settings),
        on_open_about: Some(open_about),
        on_open_browser_extensions: Some(open_browser_extensions),
        on_sign_in: None,
        on_sign_out: None,
      }),
      _ => None,
    };

    let update_button = Button::new("workspace-global-update-download")
      .icon(UiIconName::Download)
      .ghost()
      .compact()
      .small()
      .text_color(theme.status_green())
      .tooltip(update_button_tooltip)
      .loading(update_download_in_progress)
      .disabled(update_download_in_progress)
      .on_click(cx.listener(Self::global_update_download_action));

    let sign_in_button = Button::new("workspace-global-sign-in")
      .icon(IconName::Github)
      .label("Sign in with GitHub")
      .ghost()
      .gap_2()
      .small()
      .on_click(|_, _, cx| {
        crate::auth_flow::start_github_sign_in(cx, "top_bar");
      });

    let show_file_search_button = page_has_file_search(pathname);
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
      });

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

    if matches!(auth_state, AuthState::Unknown) {
      let theme = cx.theme().clone();
      let version = format!("v{}", resolved_build_version(env!("CARGO_PKG_VERSION")));
      return div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .bg(theme.background)
        .child(
          img(reviu_logo_path(theme.mode.is_dark()))
            .w(px(REVIU_WORDMARK_WIDTH_PX))
            .h_auto(),
        )
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

    sentry_context::sync_workspace_route(&pathname, page);

    if self.last_page != Some(page) {
      let previous_page = self.last_page;
      self.last_page = Some(page);
      sentry_context::sync_workspace_page(previous_page, page);
      let focus_handle = self.focus_handle(cx);
      window.focus(&focus_handle, cx);
      if should_activate_session_page(previous_page, page) {
        self
          .session_page
          .update(cx, |session_page, cx| session_page.activate(window, cx));
      }
    }

    let session_page = self.session_page.clone();

    let key_context = shortcuts::current_key_context_for_pathname(&pathname, cx);

    div()
      .size_full()
      .flex()
      .flex_col()
      .key_context(key_context.as_str())
      .on_action(cx.listener(|_, _: &crate::OpenSessionPage, _window, cx| {
        NavigationHistory::navigate("/session", cx);
      }))
      .on_action(cx.listener(Self::navigate_back_action))
      .on_action(cx.listener(|_, _: &crate::OpenBillingPage, window, cx| {
        open_billing_dialog(window, cx);
      }))
      .on_action(cx.listener(|_, _: &crate::OpenGitConfigPage, window, cx| {
        open_git_config_dialog(window, cx);
      }))
      .on_action(cx.listener(|_, _: &crate::OpenSettingsPage, window, cx| {
        open_settings_dialog(window, cx);
      }))
      .on_action(cx.listener(|_, _: &crate::OpenAboutPage, window, cx| {
        open_about_dialog(window, cx);
      }))
      .on_action(
        cx.listener(|_, _: &crate::OpenBrowserExtensions, window, cx| {
          crate::browser_extensions_dialog::open_browser_extensions_dialog(window, cx);
        }),
      )
      .child(ui::scroll_dispatcher())
      .child(self.render_global_bar(window, page, &pathname, cx))
      .child(div().flex_1().min_h_0().child(session_page))
      .into_any_element()
  }
}

impl Focusable for WorkspaceView {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    self.session_page.read(cx).focus_handle(cx)
  }
}

#[cfg(test)]
mod tests {
  use super::{
    WorkspacePage, WorkspaceView, build_app_menus_with_subscription, page_has_file_search,
    should_activate_session_page, should_run_scheduled_update_check, update_button_tooltip,
    user_menu_page_for_workspace_page, workspace_page_from_pathname,
  };
  use crate::app_update::{
    AppUpdateState, AvailableAppUpdate, ReadyToInstallAppUpdate, UpdateArtifact,
    update_action_label,
  };
  use crate::shortcuts::{self, ShortcutId};
  use gpui::{Menu, MenuItem};
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
    assert_eq!(
      workspace_page_from_pathname("/session"),
      WorkspacePage::Session
    );
    assert_eq!(
      workspace_page_from_pathname("/git"),
      WorkspacePage::Session,
      "an old link to the deleted page lands in the shell"
    );
    assert_eq!(
      workspace_page_from_pathname("/settings"),
      WorkspacePage::Session,
      "settings is a dialog now, so old links land in the shell"
    );
    assert_eq!(
      workspace_page_from_pathname("/git-config"),
      WorkspacePage::Session,
      "Git config is a dialog now, so old links land in the shell"
    );
  }

  #[test]
  fn build_app_menus_keep_workspace_actions_out_of_the_app_menu() {
    let menus = build_app_menus_with_subscription(false);
    let app_menu = menus
      .iter()
      .find(|menu| menu.name == "Reviu")
      .expect("app menu");

    assert_eq!(
      action_menu_item_names(app_menu),
      vec![
        "About Reviu",
        "Settings...",
        "Git Config...",
        "Browser Extension...",
        "Reviu Pro",
        "Quit Reviu"
      ]
    );
    assert!(
      menus.iter().all(|menu| menu.name != "File"),
      "workspace actions live in the workspace UI and palette"
    );
  }

  #[test]
  fn build_app_menus_names_billing_for_subscribers() {
    let menus = build_app_menus_with_subscription(true);
    let app_menu = menus
      .iter()
      .find(|menu| menu.name == "Reviu")
      .expect("app menu");

    assert_eq!(
      action_menu_item_names(app_menu),
      vec![
        "About Reviu",
        "Settings...",
        "Git Config...",
        "Browser Extension...",
        "Billing",
        "Quit Reviu"
      ]
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
  fn update_button_tooltip_tracks_the_current_update_state() {
    let update = make_available_update();
    let ready = ReadyToInstallAppUpdate {
      update: update.clone(),
      artifact_path: PathBuf::from("/tmp/reviu.dmg"),
      restart_binary_path: None,
    };

    assert_eq!(
      update_button_tooltip(Some(&AppUpdateState::Available(update.clone()))),
      "Download Reviu 0.2.0"
    );
    assert_eq!(
      update_button_tooltip(Some(&AppUpdateState::Downloading(update.clone()))),
      "Downloading update..."
    );
    assert_eq!(
      update_button_tooltip(Some(&AppUpdateState::ReadyToInstall(ready))),
      update_action_label(Some(AppUpdateState::ReadyToInstall(
        ReadyToInstallAppUpdate {
          update: update.clone(),
          artifact_path: PathBuf::from("/tmp/reviu.dmg"),
          restart_binary_path: None,
        },
      )))
    );
    assert_eq!(
      update_button_tooltip(Some(&AppUpdateState::Error {
        update: Some(update),
        message: "checksum mismatch".to_string(),
      })),
      "Update failed. Try again."
    );
  }

  #[test]
  fn workspace_page_from_pathname_falls_back_for_removed_github_pages() {
    for pathname in [
      "/github/octocat",
      "/github/owner/repo",
      "/github/owner/repo/code",
      "/github/owner/repo/issues",
      "/github/owner/repo/commit/abc123",
    ] {
      assert_eq!(
        workspace_page_from_pathname(pathname),
        WorkspacePage::Session,
        "{pathname} should no longer resolve to a page"
      );
    }
  }

  #[test]
  fn page_has_file_search_matches_correct_paths() {
    assert!(page_has_file_search("/session"));
    // Removed page paths land on the shell now.
    assert!(page_has_file_search("/github/owner/repo/pull/123/changes"));
    assert!(page_has_file_search("/github"));
    assert!(page_has_file_search("/github/owner/repo"));
    assert!(page_has_file_search("/github/owner/repo/code"));
    assert!(page_has_file_search("/github/owner/repo/pull/123"));
    assert!(page_has_file_search("/github/owner/repo/pulls"));
    assert!(page_has_file_search("/settings"));
  }

  #[test]
  fn workspace_page_from_pathname_unknown_falls_back_to_session() {
    assert_eq!(
      workspace_page_from_pathname("/unknown"),
      WorkspacePage::Session
    );
    assert_eq!(workspace_page_from_pathname("/"), WorkspacePage::Session);
  }

  #[test]
  fn user_menu_page_for_workspace_maps_the_shell() {
    assert_eq!(
      user_menu_page_for_workspace_page(WorkspacePage::Session),
      UserMenuPage::Session
    );
  }

  #[test]
  fn the_shell_activates_when_the_workspace_routes_to_it() {
    // Startup on the shell, and every navigation back to it.
    assert!(should_activate_session_page(None, WorkspacePage::Session));
    // There is no secondary workspace page left to activate.
    assert!(!should_activate_session_page(
      Some(WorkspacePage::Session),
      WorkspacePage::Session
    ));
  }

  #[test]
  fn workspace_command_palette_shortcut_matches_global_binding() {
    assert_eq!(
      WorkspaceView::command_palette_shortcut(),
      shortcuts::shortcut_keystroke(ShortcutId::ShowCommandPalette)
    );
  }
}
