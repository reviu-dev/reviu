use std::path::PathBuf;
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
  ActiveTheme as _, Disableable, Icon, IconName, Sizable as _, Theme, ThemeMode, h_flex, kbd::Kbd,
  notification::Notification, spinner::Spinner, tag::Tag,
};
use gpui_router::{Route, Routes};

use crate::AppProfile;
use crate::about_page::AboutPage;
use crate::api::ApiClient;
use crate::app_update::{
  AppUpdateNotificationId, AppUpdateState, AppUpdateStore, AvailableAppUpdate, UpdateArtifact,
  current_arch, current_platform, download_update_artifact, install_update_artifact,
  ready_update_button_label, resolved_build_version, should_install_update_after_download,
};
use crate::auth_state::{AuthState, AuthStateStore};
use crate::billing_page::BillingPage;
use crate::config::{AppSettings as PersistedSettings, ConfigStore};
use crate::git_config_page::GitConfigPage;
use crate::github_notifications::{self, GithubNotificationsStore};
use crate::navigation::NavigationHistory;
use crate::sentry_context;
use crate::session_page::SessionPage;
use crate::settings_page::SettingsPage;
use crate::shortcuts::{self, ShortcutId};
use crate::{ShowCommandPalette, ShowFileSearch};
use ui::{
  Button, ButtonVariants as _, GLOBAL_BAR_HEIGHT, UiIconName, UserMenuConfig, UserMenuPage,
  UserMenuState, UserMenuUser, WindowExt, user_menu,
};

const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);

pub const STATUS_BAR_ICON_PNG: &[u8] =
  include_bytes!("../../reviu/assets/reviu_status_bar_icon.png");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspacePage {
  Session,
  Billing,
  GitConfig,
  Settings,
  About,
}

pub(crate) fn workspace_page_from_pathname(pathname: &str) -> WorkspacePage {
  match pathname {
    "/session" => WorkspacePage::Session,
    "/billing" => WorkspacePage::Billing,
    "/settings" => WorkspacePage::Settings,
    "/git-config" => WorkspacePage::GitConfig,
    "/about" => WorkspacePage::About,
    _ => WorkspacePage::Session,
  }
}

/// Returns true when the current path supports file search.
/// The shell is the only page with files to search.
fn page_has_file_search(pathname: &str) -> bool {
  pathname == "/session"
}

fn user_menu_page_for_workspace_page(page: WorkspacePage) -> UserMenuPage {
  match page {
    WorkspacePage::Session => UserMenuPage::Session,
    WorkspacePage::Billing => UserMenuPage::Billing,
    WorkspacePage::GitConfig => UserMenuPage::GitConfig,
    WorkspacePage::Settings => UserMenuPage::Settings,
    WorkspacePage::About => UserMenuPage::About,
  }
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

pub fn build_app_menus(show_billing_entry: bool) -> Vec<Menu> {
  let mut navigate_items = vec![
    MenuItem::action("Back", crate::NavigateBack),
    MenuItem::separator(),
    MenuItem::action("Sessions", crate::OpenSessionPage),
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
      page: WorkspacePage::Session,
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
  session_page: Entity<SessionPage>,
  git_config_page: Entity<GitConfigPage>,
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

  fn sync_app_menus(cx: &mut App) {
    cx.set_menus(build_app_menus(AuthStateStore::should_show_billing_entry(
      cx,
    )));
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

    cx.set_global(WorkspaceRoute::default());
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

    let session_page = cx.new(|cx| SessionPage::new(window, cx));
    let git_config_page = cx.new(|cx| GitConfigPage::new(window, cx));
    let billing_page = cx.new(|cx| BillingPage::new(window, cx));
    let settings_page = cx.new(|cx| SettingsPage::new(window, cx, settings));
    let about_page = cx.new(|cx| AboutPage::new(window, cx));

    let view = Self {
      session_page,
      git_config_page,
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
            let unread = notifications.iter().filter(|n| n.unread).count();
            if PersistedSettings::get(cx).menu_bar_icon {
              crate::status_bar::update_status_bar(unread, &notifications);
            }
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
      let download_result = cx
        .background_spawn({
          let update = update.clone();
          async move { download_update_artifact(&update) }
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
          let install_result = cx
            .background_spawn(async move { install_update_artifact(&install_ready) })
            .await;
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
        let on_open_billing = show_billing_entry.then_some(open_billing.clone());

        user_menu(UserMenuConfig {
          id: "workspace-auth-menu".into(),
          state: UserMenuState::Authenticated(UserMenuUser {
            name: display_name.into(),
            email: user.email.into(),
            image: user.image.map(Into::into),
          }),
          current_page,
          on_open_billing,
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
        on_open_billing: None,
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
      let focus_handle = self.focus_handle(cx);
      window.focus(&focus_handle, cx);
      if should_activate_session_page(previous_page, page) {
        self
          .session_page
          .update(cx, |session_page, cx| session_page.activate(window, cx));
      }
    }

    let session_page = self.session_page.clone();
    let billing_page = self.billing_page.clone();
    let git_config_page = self.git_config_page.clone();
    let settings_page = self.settings_page.clone();
    let about_page = self.about_page.clone();

    let routes = Routes::new()
      .child(
        Route::new()
          .path("session")
          .element(move |_w, _cx| session_page.clone()),
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
      .on_action(cx.listener(|_, _: &crate::OpenSessionPage, _window, cx| {
        NavigationHistory::navigate("/session", cx);
      }))
      .on_action(cx.listener(Self::navigate_back_action))
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
      .child(ui::scroll_dispatcher())
      .child(self.render_global_bar(window, page, &pathname, cx))
      .child(div().flex_1().min_h_0().child(routes))
      .into_any_element()
  }
}

#[cfg(test)]
mod tests {
  use super::{
    WorkspacePage, WorkspaceView, build_app_menus, page_has_file_search,
    should_activate_session_page, should_run_scheduled_update_check,
    user_menu_page_for_workspace_page, workspace_page_from_pathname,
  };
  use crate::app_update::{
    AppUpdateState, AvailableAppUpdate, ReadyToInstallAppUpdate, UpdateArtifact,
    ready_update_button_label,
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
      vec!["Back", "Sessions", "Git Config"]
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
      vec!["Back", "Sessions", "Git Config", "Billing"]
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
    // A pull request has no page of its own any more: the shell is the surface.
    assert!(!page_has_file_search("/github/owner/repo/pull/123/changes"));
    assert!(!page_has_file_search("/github"));
    assert!(!page_has_file_search("/github/owner/repo"));
    assert!(!page_has_file_search("/github/owner/repo/code"));
    assert!(!page_has_file_search("/github/owner/repo/pull/123"));
    assert!(!page_has_file_search("/github/owner/repo/pulls"));
    assert!(!page_has_file_search("/settings"));
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
  fn user_menu_page_for_workspace_maps_github_surfaces() {
    assert_eq!(
      user_menu_page_for_workspace_page(WorkspacePage::Session),
      UserMenuPage::Session
    );
    assert_eq!(
      user_menu_page_for_workspace_page(WorkspacePage::Billing),
      UserMenuPage::Billing
    );
  }

  #[test]
  fn the_shell_activates_when_the_workspace_routes_to_it() {
    // Startup on the shell, and every navigation back to it.
    assert!(should_activate_session_page(None, WorkspacePage::Session));
    assert!(should_activate_session_page(
      Some(WorkspacePage::Billing),
      WorkspacePage::Session
    ));

    // Never for another page, and never twice for the same one.
    assert!(!should_activate_session_page(None, WorkspacePage::Billing));
    assert!(!should_activate_session_page(
      Some(WorkspacePage::Session),
      WorkspacePage::Billing
    ));
    assert!(!should_activate_session_page(
      Some(WorkspacePage::Session),
      WorkspacePage::Session
    ));
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
      WorkspacePage::Session => self.session_page.read(cx).focus_handle(cx),
      WorkspacePage::Billing => self.billing_page.read(cx).focus_handle(cx),
      WorkspacePage::GitConfig => self.git_config_page.read(cx).focus_handle(cx),
      WorkspacePage::Settings => self.settings_page.read(cx).focus_handle(cx),
      WorkspacePage::About => self.about_page.read(cx).focus_handle(cx),
    }
  }
}
