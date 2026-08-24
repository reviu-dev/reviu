use gpui::{App, Context, Render, Task, Window, div, prelude::*};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Sizable as _,
  button::{Button, ButtonVariants as _},
  dialog::DialogButtonProps,
  h_flex, v_flex,
};
use ui::{StatusThemeExt, UiIconName, WindowExt};

use crate::{
  app_update::{
    AppUpdateState, AppUpdateStore, AvailableAppUpdate, UpdateArtifact, current_arch,
    current_platform, ready_update_status_message, resolved_build_version, start_update_download,
    update_action_label,
  },
  workspace::WorkspaceApi,
};

pub fn open_about_dialog(window: &mut Window, _cx: &mut App) {
  // Defer to next frame so the command palette dialog closes first
  window.on_next_frame(|window, cx| {
    open_about_dialog_inner(window, cx);
  });
}

fn open_about_dialog_inner(window: &mut Window, cx: &mut App) {
  let about = cx.new(AboutContent::new);
  window.open_alert_dialog(cx, move |alert, _, _| {
    alert
      .title("Reviu Desktop")
      .description("Version information and update controls.")
      .child(about.clone())
      .show_cancel(false)
      .button_props(DialogButtonProps::default().ok_text("Close"))
  });
}

/// The notice reads the same whatever the palette: the tone picks the colour at
/// render time, so what to say stays testable without a theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateNoticeTone {
  Neutral,
  Good,
  Bad,
}

#[derive(Clone)]
enum UpdateCheckStatus {
  UpToDate,
  UpdateAvailable(String),
  Error(String),
}

struct AboutContent {
  check_in_progress: bool,
  check_status: Option<UpdateCheckStatus>,
  check_task: Option<Task<()>>,
}

impl AboutContent {
  fn new(_: &mut Context<Self>) -> Self {
    Self {
      check_in_progress: false,
      check_status: None,
      check_task: None,
    }
  }

  fn current_client_version() -> String {
    resolved_build_version(env!("CARGO_PKG_VERSION"))
  }

  fn check_for_updates_action(
    &mut self,
    _: &gpui::ClickEvent,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.check_in_progress {
      return;
    }

    self.check_in_progress = true;
    self.check_status = None;
    cx.notify();

    let api = WorkspaceApi::global(cx).api.clone();
    let current_version = Self::current_client_version();
    let platform = current_platform().to_string();
    let arch = current_arch().to_string();

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(
          async move { api.check_desktop_update(&current_version, &platform, &arch) },
        )
        .await;
      let _ = this.update(cx, |this, cx| {
        this.check_in_progress = false;

        match result {
          Ok(payload) if payload.update_available => {
            let Some(artifact) = payload.artifact else {
              let message = "Update artifact is missing for this platform.";
              this.check_status = Some(UpdateCheckStatus::Error(message.to_string()));
              AppUpdateStore::set_error(cx, None, message);
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
            AppUpdateStore::set_available_update(cx, Some(update.clone()));
            this.check_status = Some(UpdateCheckStatus::UpdateAvailable(update.latest_version));
          }
          Ok(_) => {
            AppUpdateStore::clear_available_update(cx);
            this.check_status = Some(UpdateCheckStatus::UpToDate);
          }
          Err(err) => {
            this.check_status = Some(UpdateCheckStatus::Error(err.to_string()));
          }
        }

        cx.notify();
      });
    });

    self.check_task = Some(task);
  }

  fn download_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    self.check_status = None;
    start_update_download(cx);
    cx.notify();
  }

  fn update_notice(&self, cx: &App) -> Option<(String, UpdateNoticeTone)> {
    match &self.check_status {
      Some(UpdateCheckStatus::UpToDate) => Some((
        "You're already on the latest version.".to_string(),
        UpdateNoticeTone::Neutral,
      )),
      Some(UpdateCheckStatus::UpdateAvailable(version)) => Some((
        format!("Update available: {version}"),
        UpdateNoticeTone::Good,
      )),
      Some(UpdateCheckStatus::Error(error)) => {
        Some((format!("Update failed: {error}"), UpdateNoticeTone::Bad))
      }
      None => match AppUpdateStore::try_state(cx) {
        Some(AppUpdateState::Downloading(_)) => Some((
          "Downloading update artifact...".to_string(),
          UpdateNoticeTone::Neutral,
        )),
        Some(AppUpdateState::ReadyToInstall(_)) => Some((
          ready_update_status_message().to_string(),
          UpdateNoticeTone::Good,
        )),
        Some(AppUpdateState::Error { message, .. }) => {
          Some((format!("Update failed: {message}"), UpdateNoticeTone::Bad))
        }
        _ => None,
      },
    }
  }
}

impl Render for AboutContent {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let build_version = env!("CARGO_PKG_VERSION").to_string();
    let client_version = Self::current_client_version();
    let notice = self.update_notice(cx);

    let update_state = AppUpdateStore::try_state(cx);
    let has_update = AppUpdateStore::try_available_update(cx).is_some();
    let download_in_progress = AppUpdateStore::is_downloading(cx);

    let check_button = Button::new("about-check-updates")
      .small()
      .icon(UiIconName::RefreshCw)
      .label(if self.check_in_progress {
        "Checking..."
      } else {
        "Check for updates"
      })
      .disabled(self.check_in_progress)
      .on_click(cx.listener(Self::check_for_updates_action));

    let download_button = Button::new("about-download-update")
      .small()
      .primary()
      .icon(UiIconName::Download)
      .label(update_action_label(update_state))
      .disabled(download_in_progress)
      .on_click(cx.listener(Self::download_action));

    v_flex()
      .w_full()
      .gap_3()
      .pt_2()
      .child(
        v_flex()
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
        h_flex()
          .gap_2()
          .justify_start()
          .child(check_button)
          .when(has_update, |this| this.child(download_button)),
      )
      .when_some(notice, |this, (message, tone)| {
        let color = match tone {
          UpdateNoticeTone::Neutral => theme.muted_foreground,
          UpdateNoticeTone::Good => theme.status_green(),
          UpdateNoticeTone::Bad => theme.status_red(),
        };
        this.child(div().text_sm().text_color(color).child(message))
      })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app_update::ReadyToInstallAppUpdate;
  use gpui::TestAppContext;
  use std::path::PathBuf;

  fn available_update() -> AvailableAppUpdate {
    AvailableAppUpdate {
      latest_version: "0.19.0".to_string(),
      minimum_supported_version: "0.1.0".to_string(),
      release_notes_url: "https://reviu.dev/changelog".to_string(),
      force_update: false,
      artifact: UpdateArtifact {
        url: "https://reviu.dev/downloads/latest".to_string(),
        sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        size: 1024,
      },
    }
  }

  #[test]
  fn the_version_shown_is_the_resolved_build_version() {
    assert_eq!(
      AboutContent::current_client_version(),
      resolved_build_version(env!("CARGO_PKG_VERSION"))
    );
  }

  struct Page;

  impl Render for Page {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
      div()
    }
  }

  #[gpui::test]
  fn the_dialog_opens_over_whatever_page_is_up(cx: &mut TestAppContext) {
    cx.update(|cx| {
      gpui_component::init(cx);
      cx.set_global(AppUpdateStore::default());
      cx.set_global(WorkspaceApi::new());
    });

    let (_root, cx) = cx.add_window_view(|window, cx| {
      let page = cx.new(|_| Page);
      gpui_component::Root::new(page, window, cx)
    });

    cx.update(|window, cx| {
      assert!(!window.has_active_dialog(cx));
      open_about_dialog_inner(window, cx);
    });
    cx.run_until_parked();

    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
  }

  #[gpui::test]
  fn nothing_is_said_about_updates_until_something_happens(cx: &mut TestAppContext) {
    cx.update(|cx| {
      cx.set_global(AppUpdateStore::default());
      let content = cx.new(AboutContent::new);
      content.read_with(cx, |content, cx| {
        assert!(content.update_notice(cx).is_none());
      });
    });
  }

  #[gpui::test]
  fn a_failed_check_is_reported_in_the_dialog(cx: &mut TestAppContext) {
    cx.update(|cx| {
      cx.set_global(AppUpdateStore::default());
      let content = cx.new(AboutContent::new);
      content.update(cx, |content, _| {
        content.check_status = Some(UpdateCheckStatus::Error("network is down".to_string()));
      });
      content.read_with(cx, |content, cx| {
        let (message, _) = content.update_notice(cx).expect("a notice");
        assert_eq!(message, "Update failed: network is down");
      });
    });
  }

  #[gpui::test]
  fn a_ready_update_tells_the_user_what_is_left_to_do(cx: &mut TestAppContext) {
    cx.update(|cx| {
      cx.set_global(AppUpdateStore::default());
      AppUpdateStore::set_ready_to_install(
        cx,
        ReadyToInstallAppUpdate {
          update: available_update(),
          artifact_path: PathBuf::from("/tmp/reviu-installer.dmg"),
          restart_binary_path: None,
        },
      );

      let content = cx.new(AboutContent::new);
      content.read_with(cx, |content, cx| {
        let (message, _) = content.update_notice(cx).expect("a notice");
        assert_eq!(message, ready_update_status_message());
      });
    });
  }

  #[gpui::test]
  fn the_store_error_surfaces_when_no_check_ran_in_the_dialog(cx: &mut TestAppContext) {
    cx.update(|cx| {
      cx.set_global(AppUpdateStore::default());
      AppUpdateStore::set_error(cx, Some(available_update()), "artifact is corrupt");

      let content = cx.new(AboutContent::new);
      content.read_with(cx, |content, cx| {
        let (message, _) = content.update_notice(cx).expect("a notice");
        assert_eq!(message, "Update failed: artifact is corrupt");
      });
    });
  }
}
