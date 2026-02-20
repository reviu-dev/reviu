use std::sync::Arc;

use gpui::{
  AnyWindowHandle, App, Context, FocusHandle, Focusable, Render, SharedString, Task, Window, div,
  prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt,
  button::{Button, ButtonVariants as _},
  notification::Notification,
};
use smol::unblock;

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, HEADER_HEIGHT, StatusThemeExt, UiIconName, WindowExt,
};

use crate::{
  CloseWorkspacePage, ShowCommandPalette,
  app_update::{
    AppUpdateNotificationId, AppUpdateStore, AvailableAppUpdate, effective_current_version,
  },
  auth_state::{AuthState, AuthStateStore},
  config::ConfigStore,
  github_page::GithubPageHandle,
  github_pr_details_page::GithubPrDetailsPageHandle,
  workspace::{WorkspaceApi, WorkspaceRoute},
};

#[derive(Clone)]
enum UpdateCheckStatus {
  UpToDate,
  UpdateAvailable(String),
  Error(String),
}

pub struct AboutPage {
  focus_handle: FocusHandle,
  window_handle: AnyWindowHandle,
  check_in_progress: bool,
  update_check_status: Option<UpdateCheckStatus>,
  update_check_task: Option<Task<()>>,
}

impl AboutPage {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      check_in_progress: false,
      update_check_status: None,
      update_check_task: None,
    }
  }

  fn current_client_version() -> String {
    let simulated_version = ConfigStore::load_simulated_app_version();
    effective_current_version(env!("CARGO_PKG_VERSION"), simulated_version.as_deref())
  }

  fn close_workspace_page_action(
    &mut self,
    _: &CloseWorkspacePage,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    WorkspaceRoute::close_about(cx);
    cx.refresh_windows();
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let include_github = matches!(AuthStateStore::get(cx), AuthState::Authenticated(_));
    let commands =
      CommandPaletteCommand::default_global_commands(CommandPalettePage::About, include_github);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, _window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, cx)
      })
    });

    let config = CommandPaletteConfig::new(Vec::new(), commands, handler);
    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    let palette_for_dialog = palette.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      dialog
        .p_0()
        .border_0()
        .min_h_0()
        .overlay_closable(true)
        .keyboard(true)
        .close_button(false)
        .child(palette_for_dialog.clone())
    });
  }

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match action {
      CommandPaletteAction::OpenGitPage => {
        WorkspaceRoute::global_mut(cx).page = crate::workspace::WorkspacePage::Git;
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGithubPage => {
        if AuthStateStore::has_active_subscription(cx) {
          GithubPageHandle::refresh(cx);
          WorkspaceRoute::open_github(cx);
        } else {
          WorkspaceRoute::open_billing(cx);
        }
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
      } => {
        GithubPrDetailsPageHandle::show(owner.into(), repo.into(), number, cx);
        Ok(())
      }
      CommandPaletteAction::OpenSettingsPage => {
        WorkspaceRoute::open_settings(cx);
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenBillingPage => {
        WorkspaceRoute::open_billing(cx);
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenAboutPage => Ok(()),
      CommandPaletteAction::OpenGitConfigPage => {
        WorkspaceRoute::open_git_config(cx);
        cx.refresh_windows();
        Ok(())
      }
      _ => Err("Command not available.".into()),
    }
  }

  fn check_for_updates_action(
    &mut self,
    _: &gpui::ClickEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.check_for_updates(cx);
  }

  fn check_for_updates(&mut self, cx: &mut Context<Self>) {
    if self.check_in_progress {
      return;
    }

    self.check_in_progress = true;
    self.update_check_status = None;
    cx.notify();

    let api = WorkspaceApi::global(cx).api.clone();
    let current_version = Self::current_client_version();

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.check_desktop_update(&current_version)).await;
      let _ = this.update(cx, |this, cx| {
        this.check_in_progress = false;

        match result {
          Ok(payload) if payload.update_available => {
            let update = AvailableAppUpdate {
              latest_version: payload.latest_version,
              download_url: payload.download_url,
            };
            this.show_update_notification(update.clone(), cx);
            AppUpdateStore::set_available_update(cx, Some(update.clone()));
            this.update_check_status =
              Some(UpdateCheckStatus::UpdateAvailable(update.latest_version));
          }
          Ok(_) => {
            AppUpdateStore::clear_available_update(cx);
            this.dismiss_update_notification(cx);
            this.update_check_status = Some(UpdateCheckStatus::UpToDate);
          }
          Err(err) => {
            this.update_check_status = Some(UpdateCheckStatus::Error(err.to_string()));
          }
        }

        cx.notify();
      });
    });

    self.update_check_task = Some(task);
  }

  fn dismiss_update_notification(&self, cx: &mut Context<Self>) {
    let _ = cx.update_window(self.window_handle, |_, window, cx| {
      window.remove_notification::<AppUpdateNotificationId>(cx);
    });
  }

  fn show_update_notification(&self, update: AvailableAppUpdate, cx: &mut Context<Self>) {
    let latest_version = update.latest_version.clone();
    let download_url = update.download_url.clone();
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      let latest_version_for_action = latest_version.clone();
      let download_url_for_action = download_url.clone();
      window.push_notification(
        Notification::new()
          .id::<AppUpdateNotificationId>()
          .title("New version available")
          .message(format!(
            "Reviu {} is available. Download the latest version.",
            latest_version
          ))
          .autohide(false)
          .action(move |_, _, cx| {
            let latest_version = latest_version_for_action.clone();
            let download_url = download_url_for_action.clone();
            Button::new("about-update-download")
              .primary()
              .icon(UiIconName::Download)
              .label("Download")
              .on_click(cx.listener(move |_, _, window, cx| {
                AppUpdateStore::apply_download_action(&download_url, &latest_version, cx);
                window.on_next_frame(|window, cx| {
                  window.remove_notification::<AppUpdateNotificationId>(cx);
                });
              }))
          }),
        cx,
      );
    });
  }
}

impl Render for AboutPage {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let build_version = env!("CARGO_PKG_VERSION").to_string();
    let simulated_version = ConfigStore::load_simulated_app_version();
    let client_version = effective_current_version(&build_version, simulated_version.as_deref());

    let header = div()
      .h(px(HEADER_HEIGHT))
      .max_h(px(HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(div().text_sm().text_color(theme.foreground).child("About"))
      .child(
        Button::new("close-about")
          .icon(IconName::Close)
          .ghost()
          .compact()
          .tooltip("Close about")
          .on_click(|_, _, cx| {
            WorkspaceRoute::close_about(cx);
            cx.refresh_windows();
          }),
      );

    let check_status = match &self.update_check_status {
      Some(UpdateCheckStatus::UpToDate) => Some((
        "You're already on the latest version.".to_string(),
        theme.muted_foreground,
      )),
      Some(UpdateCheckStatus::UpdateAvailable(version)) => {
        Some((format!("Update available: {version}"), theme.status_green()))
      }
      Some(UpdateCheckStatus::Error(error)) => {
        Some((format!("Update check failed: {error}"), theme.status_red()))
      }
      None => None,
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(AboutPage::show_command_palette_action))
      .on_action(cx.listener(AboutPage::close_workspace_page_action))
      .child(header)
      .child(
        div()
          .w_full()
          .mx_auto()
          .h_full()
          .min_h_0()
          .py_4()
          .px_4()
          .child(
            div()
              .w_full()
              .max_w(px(700.))
              .mx_auto()
              .flex()
              .flex_col()
              .gap_3()
              .child(
                div()
                  .text_lg()
                  .font_semibold()
                  .text_color(theme.foreground)
                  .child("Reviu Desktop"),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .child("Version information and update controls."),
              )
              .child(
                div()
                  .flex()
                  .flex_col()
                  .gap_1()
                  .child(
                    div()
                      .text_sm()
                      .text_color(theme.foreground)
                      .child(format!("Client version: {client_version}")),
                  )
                  .child(
                    div()
                      .text_xs()
                      .text_color(theme.muted_foreground)
                      .child(format!("Build version: {build_version}")),
                  )
                  .when_some(simulated_version, |this, version| {
                    this.child(
                      div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("Simulated version (V1): {version}")),
                    )
                  }),
              )
              .child(
                Button::new("about-check-updates")
                  .small()
                  .icon(UiIconName::RefreshCcw)
                  .label(if self.check_in_progress {
                    "Checking..."
                  } else {
                    "Check for updates"
                  })
                  .disabled(self.check_in_progress)
                  .on_click(cx.listener(AboutPage::check_for_updates_action)),
              )
              .when_some(check_status, |this, (message, color)| {
                this.child(div().text_sm().text_color(color).child(message))
              }),
          ),
      )
  }
}

impl Focusable for AboutPage {
  fn focus_handle(&self, _: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}
