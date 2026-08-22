//! Where a link to a pull request lands. Reviu reviews the branch it has open,
//! so a link only has a home here when it names that branch's pull request.

use gpui::prelude::*;
use gpui::{AnyWindowHandle, App, Context, WeakEntity};
use gpui_component::{Sizable as _, notification::Notification};
use ui::{Button, ButtonVariants as _, WindowExt as _};

use crate::github_navigation::github_pull_request_url;
use crate::session_page::SessionPage;

#[derive(Clone, Default)]
pub struct PullRequestSurfaceHandle {
  page: Option<WeakEntity<SessionPage>>,
  /// What the panel is showing, kept here so a link can be judged without
  /// borrowing the shell that may already be busy answering the same click.
  branch_pull_request: Option<(String, String, u64)>,
}

impl gpui::Global for PullRequestSurfaceHandle {}

impl PullRequestSurfaceHandle {
  pub(crate) fn register(cx: &mut Context<SessionPage>) {
    let branch_pull_request = cx
      .try_global::<Self>()
      .and_then(|handle| handle.branch_pull_request.clone());
    cx.set_global(Self {
      page: Some(cx.entity().downgrade()),
      branch_pull_request,
    });
  }

  pub(crate) fn publish(identity: Option<(String, String, u64)>, cx: &mut App) {
    let Some(handle) = cx.try_global::<Self>() else {
      return;
    };
    if handle.branch_pull_request == identity {
      return;
    }
    let page = handle.page.clone();
    cx.set_global(Self {
      page,
      branch_pull_request: identity,
    });
  }

  fn shows(owner: &str, repo: &str, number: u64, cx: &App) -> bool {
    cx.try_global::<Self>()
      .and_then(|handle| handle.branch_pull_request.as_ref())
      .is_some_and(|(open_owner, open_repo, open_number)| {
        *open_number == number
          && open_owner.eq_ignore_ascii_case(owner)
          && open_repo.eq_ignore_ascii_case(repo)
      })
  }

  fn page(cx: &App) -> Option<WeakEntity<SessionPage>> {
    cx.try_global::<Self>()
      .and_then(|handle| handle.page.clone())
  }

  /// Shows the pull request panel when the link names the pull request of the
  /// branch in the open repository. Returns whether it did.
  pub fn show(owner: &str, repo: &str, number: u64, cx: &mut App) -> bool {
    if !Self::shows(owner, repo, number, cx) {
      return false;
    }
    let Some(page) = Self::page(cx) else {
      return false;
    };
    // The click can come from inside the shell's own update, so the panel opens
    // once that one is done.
    cx.defer(move |cx| {
      let Ok(window_handle) = page.read_with(cx, |page, _| page.window_handle()) else {
        return;
      };
      let _ = cx.update_window(window_handle, move |_, window, cx| {
        let _ = page.update(cx, |page, cx| {
          page.show_dock_tab(crate::dock_panel::DockPanelTab::PullRequest, window, cx);
        });
      });
    });
    true
  }

  /// For a link that asked Reviu to review the pull request: it either lands, or
  /// it says why it could not.
  pub fn show_or_explain(owner: String, repo: String, number: u64, cx: &mut App) {
    if Self::show(&owner, &repo, number, cx) {
      return;
    }
    let window_handle =
      Self::page(cx).and_then(|page| page.read_with(cx, |page, _| page.window_handle()).ok());
    let Some(window_handle) = window_handle else {
      cx.open_url(&github_pull_request_url(&owner, &repo, number, false, None));
      return;
    };
    explain_not_open(window_handle, owner, repo, number, cx);
  }
}

/// Reviu reviews what is checked out: say that, and offer the only other place
/// the pull request can be read right now.
fn explain_not_open(
  window_handle: AnyWindowHandle,
  owner: String,
  repo: String,
  number: u64,
  cx: &mut App,
) {
  let _ = cx.update_window(window_handle, move |_, window, cx| {
    let owner = owner.clone();
    let repo = repo.clone();
    window.push_notification(
      Notification::info(format!("Check out its branch to review #{number} here."))
        .title("Not the pull request of the open branch")
        .content(move |_, _, _| {
          let owner = owner.clone();
          let repo = repo.clone();
          gpui::div()
            .flex()
            .mt_3()
            .child(
              Button::new("pull-request-link-open-on-github")
                .primary()
                .compact()
                .small()
                .label("Open on GitHub")
                .on_click(move |_, _, cx| {
                  cx.open_url(&github_pull_request_url(&owner, &repo, number, false, None));
                }),
            )
            .into_any_element()
        }),
      cx,
    );
  });
}
