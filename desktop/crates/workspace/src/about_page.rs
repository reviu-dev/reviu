use std::sync::Arc;

use gpui::{
  AnyWindowHandle, App, Context, FocusHandle, Focusable, Render, SharedString, Task, Window, div,
  prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt,
  button::{Button, ButtonVariants as _},
  h_flex,
  notification::Notification,
  v_flex,
};
use smol::unblock;

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, DETAILS_PAGE_CONTAINER_MAX_WIDTH, PAGE_HEADER_HEIGHT,
  StatusThemeExt, UiIconName, WindowExt,
};

use crate::{
  CloseWorkspacePage, ShowCommandPalette,
  app_update::{
    AppUpdateNotificationId, AppUpdateState, AppUpdateStore, AvailableAppUpdate, UpdateArtifact,
    current_arch, current_platform, download_update_artifact, install_update_artifact,
    ready_update_status_message, resolved_build_version, should_install_update_after_download,
  },
  auth_state::AuthStateStore,
  github_navigation::{open_pr_target, open_repo_target},
  github_page::GithubPageHandle,
  navigation::NavigationHistory,
  workspace::WorkspaceApi,
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
  update_action_task: Option<Task<()>>,
}

impl AboutPage {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      check_in_progress: false,
      update_check_status: None,
      update_check_task: None,
      update_action_task: None,
    }
  }

  fn current_client_version() -> String {
    resolved_build_version(env!("CARGO_PKG_VERSION"))
  }

  fn close_workspace_page_action(
    &mut self,
    _: &CloseWorkspacePage,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    NavigationHistory::navigate_back(cx);
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
    let include_github = AuthStateStore::has_github_access(cx);
    let commands =
      CommandPaletteCommand::default_global_commands(CommandPalettePage::About, include_github);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let config = CommandPaletteConfig::new(Vec::new(), commands, handler);
    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    let palette_for_dialog = palette.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      dialog
        .on_ok(|_, _, _| false)
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
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match action {
      CommandPaletteAction::OpenGitPage => {
        NavigationHistory::navigate("/git", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPage => {
        GithubPageHandle::refresh(cx);
        NavigationHistory::navigate("/github", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
        open_changes_tab,
        review_comment_id,
      } => {
        open_pr_target(
          owner,
          repo,
          number,
          open_changes_tab,
          review_comment_id,
          None,
          cx,
        );
        Ok(())
      }
      CommandPaletteAction::OpenGithubRepoDetails {
        owner,
        repo,
        tab,
        issue_number,
        issue_comment_id,
      } => {
        open_repo_target(owner, repo, tab, issue_number, issue_comment_id, cx);
        Ok(())
      }
      CommandPaletteAction::OpenSettingsPage => {
        NavigationHistory::navigate("/settings", cx);
        Ok(())
      }
      CommandPaletteAction::OpenBillingPage => {
        NavigationHistory::navigate("/billing", cx);
        Ok(())
      }
      CommandPaletteAction::OpenAboutPage => Ok(()),
      CommandPaletteAction::OpenGitConfigPage => {
        NavigationHistory::navigate("/git-config", cx);
        Ok(())
      }
      CommandPaletteAction::SendFeedback => {
        crate::feedback_dialog::open_feedback_dialog(window, cx);
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
    let platform = current_platform().to_string();
    let arch = current_arch().to_string();

    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.check_desktop_update(&current_version, &platform, &arch)).await;
      let _ = this.update(cx, |this, cx| {
        this.check_in_progress = false;

        match result {
          Ok(payload) if payload.update_available => {
            let Some(artifact) = payload.artifact else {
              this.update_check_status = Some(UpdateCheckStatus::Error(
                "Update artifact is missing for this platform.".to_string(),
              ));
              AppUpdateStore::set_error(cx, None, "Update artifact is missing for this platform.");
              cx.notify();
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

  fn trigger_update_download(&mut self, cx: &mut Context<Self>) {
    if AppUpdateStore::is_downloading(cx) {
      return;
    }

    if let Some(ready) = AppUpdateStore::try_ready_to_install(cx) {
      #[cfg(target_os = "windows")]
      {
        match install_update_artifact(&ready) {
          Ok(()) => cx.quit(),
          Err(err) => {
            AppUpdateStore::set_error(cx, Some(ready.update.clone()), err.to_string());
            self.update_check_status = Some(UpdateCheckStatus::Error(err.to_string()));
            cx.notify();
          }
        }
        return;
      }

      #[cfg(not(target_os = "windows"))]
      {
        if let Some(path) = ready.restart_binary_path {
          cx.set_restart_path(path);
        }
        cx.restart();
        cx.notify();
        return;
      }
    }

    let Some(update) = AppUpdateStore::try_available_update(cx) else {
      return;
    };

    AppUpdateStore::set_downloading(cx, update.clone());
    self.update_check_status = None;
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let download_result = unblock({
        let update = update.clone();
        move || download_update_artifact(&update)
      })
      .await;

      match download_result {
        Ok(ready) => {
          if !should_install_update_after_download() {
            let _ = this.update(cx, |this, cx| {
              AppUpdateStore::set_ready_to_install(cx, ready.clone());
              this.update_check_status = None;
              cx.notify();
            });
            return;
          }

          let install_ready = ready.clone();
          let install_result = unblock(move || install_update_artifact(&install_ready)).await;
          let _ = this.update(cx, |this, cx| {
            match install_result {
              Ok(()) => {
                AppUpdateStore::set_ready_to_install(cx, ready.clone());
                this.update_check_status = None;
              }
              Err(err) => {
                AppUpdateStore::set_error(cx, Some(ready.update.clone()), err.to_string());
                this.update_check_status = Some(UpdateCheckStatus::Error(err.to_string()));
              }
            }
            cx.notify();
          });
        }
        Err(err) => {
          let _ = this.update(cx, |this, cx| {
            AppUpdateStore::set_error(cx, Some(update.clone()), err.to_string());
            this.update_check_status = Some(UpdateCheckStatus::Error(err.to_string()));
            cx.notify();
          });
        }
      }
    });

    self.update_action_task = Some(task);
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
          .title("New version available")
          .message(format!(
            "Reviu {} is available. Download the latest version.",
            latest_version
          ))
          .autohide(false)
          .action(move |_, _, _cx| {
            let view = view.clone();
            Button::new("about-update-download")
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
}

impl Render for AboutPage {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let build_version = env!("CARGO_PKG_VERSION").to_string();
    let client_version = Self::current_client_version();

    let header = div()
      .h(px(PAGE_HEADER_HEIGHT))
      .max_h(px(PAGE_HEADER_HEIGHT))
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
            NavigationHistory::navigate_back(cx);
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
        Some((format!("Update failed: {error}"), theme.status_red()))
      }
      None => match AppUpdateStore::try_state(cx) {
        Some(AppUpdateState::Downloading(_)) => Some((
          "Downloading update artifact...".to_string(),
          theme.muted_foreground,
        )),
        Some(AppUpdateState::ReadyToInstall(_)) => Some((
          ready_update_status_message().to_string(),
          theme.status_green(),
        )),
        Some(AppUpdateState::Error { message, .. }) => {
          Some((format!("Update failed: {message}"), theme.status_red()))
        }
        _ => None,
      },
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
        div().w_full().h_full().min_h_0().py_4().px_4().child(
          v_flex()
            .w_full()
            .max_w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH))
            .mx_auto()
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
                ),
            )
            .child(
              h_flex().justify_start().child(
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
              ),
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
