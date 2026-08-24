//! Where a link to a pull request lands. Reviu reviews the branch it has open,
//! so a link either names that branch's pull request, or asks to check it out.

use gpui::prelude::*;
use gpui::{App, Context, WeakEntity};

use crate::session_page::SessionPage;

/// Owner, repository and number: what identifies a pull request across the app.
pub(crate) type PullRequestIdentity = (String, String, u64);

#[derive(Clone, Default)]
pub struct PullRequestSurfaceHandle {
  page: Option<WeakEntity<SessionPage>>,
  /// What the panel is showing, kept here so a link can be judged without
  /// borrowing the shell that may already be busy answering the same click.
  branch_pull_request: Option<PullRequestIdentity>,
  /// A link waiting for the branch it asked for. It survives while nothing is
  /// resolved, and any other pull request answering drops it, so a checkout
  /// that never landed leaves nothing armed.
  pending: Option<PullRequestIdentity>,
  /// The review comment that link named, so the checkout it triggers still ends
  /// on the file the link was pointing at.
  awaited_comment: Option<(PullRequestIdentity, u64)>,
}

impl gpui::Global for PullRequestSurfaceHandle {}

fn same_pull_request(identity: &PullRequestIdentity, owner: &str, repo: &str, number: u64) -> bool {
  identity.2 == number
    && identity.0.eq_ignore_ascii_case(owner)
    && identity.1.eq_ignore_ascii_case(repo)
}

impl PullRequestSurfaceHandle {
  pub(crate) fn register(cx: &mut Context<SessionPage>) {
    let existing = cx.try_global::<Self>().cloned().unwrap_or_default();
    cx.set_global(Self {
      page: Some(cx.entity().downgrade()),
      ..existing
    });
  }

  pub(crate) fn publish(identity: Option<PullRequestIdentity>, cx: &mut App) {
    let Some(handle) = cx.try_global::<Self>() else {
      return;
    };
    let pending = handle.pending.clone();
    let awaited = matches!(
      (&pending, &identity),
      (Some(pending), Some(identity))
        if same_pull_request(identity, &pending.0, &pending.1, pending.2)
    );
    // A pull request answering for this branch settles the question, whether or
    // not it is the one the link asked for.
    let pending = if identity.is_some() { None } else { pending };
    let anchor = match (awaited, &identity, &handle.awaited_comment) {
      (true, Some(identity), Some((waiting, comment_id)))
        if same_pull_request(identity, &waiting.0, &waiting.1, waiting.2) =>
      {
        Some(*comment_id)
      }
      _ => None,
    };
    let awaited_comment = if identity.is_some() {
      None
    } else {
      handle.awaited_comment.clone()
    };
    if handle.branch_pull_request == identity
      && handle.pending == pending
      && handle.awaited_comment == awaited_comment
    {
      return;
    }
    let page = handle.page.clone();
    cx.set_global(Self {
      page,
      branch_pull_request: identity.clone(),
      pending,
      awaited_comment,
    });

    if let Some(identity) = identity
      && awaited
    {
      Self::show(&identity.0, &identity.1, identity.2, anchor, cx);
    }
  }

  /// Arms the panel for a pull request the branch does not carry yet: the
  /// checkout is on its way, and its refresh is what opens the panel.
  pub(crate) fn expect(identity: PullRequestIdentity, cx: &mut App) {
    let Some(handle) = cx.try_global::<Self>() else {
      return;
    };
    let page = handle.page.clone();
    let branch_pull_request = handle.branch_pull_request.clone();
    let awaited_comment = handle.awaited_comment.clone();
    cx.set_global(Self {
      page,
      branch_pull_request,
      pending: Some(identity),
      awaited_comment,
    });
  }

  /// The review comment a link named, held while its checkout is being offered.
  pub(crate) fn expect_comment(identity: PullRequestIdentity, comment_id: u64, cx: &mut App) {
    let Some(handle) = cx.try_global::<Self>() else {
      return;
    };
    let page = handle.page.clone();
    let branch_pull_request = handle.branch_pull_request.clone();
    let pending = handle.pending.clone();
    cx.set_global(Self {
      page,
      branch_pull_request,
      pending,
      awaited_comment: Some((identity, comment_id)),
    });
  }

  /// The checkout the link asked for did not happen: nothing is coming, so the
  /// panel must not open on a branch the user reaches by their own means later.
  pub(crate) fn forget(cx: &mut App) {
    let Some(handle) = cx.try_global::<Self>() else {
      return;
    };
    if handle.pending.is_none() && handle.awaited_comment.is_none() {
      return;
    }
    let page = handle.page.clone();
    let branch_pull_request = handle.branch_pull_request.clone();
    cx.set_global(Self {
      page,
      branch_pull_request,
      pending: None,
      awaited_comment: None,
    });
  }

  fn shows(owner: &str, repo: &str, number: u64, cx: &App) -> bool {
    cx.try_global::<Self>()
      .and_then(|handle| handle.branch_pull_request.as_ref())
      .is_some_and(|identity| same_pull_request(identity, owner, repo, number))
  }

  fn page(cx: &App) -> Option<WeakEntity<SessionPage>> {
    cx.try_global::<Self>()
      .and_then(|handle| handle.page.clone())
  }

  /// Shows the pull request panel when the link names the pull request of the
  /// branch in the open repository, and takes the diff to the review comment it
  /// named. Returns whether it did.
  pub(crate) fn show(
    owner: &str,
    repo: &str,
    number: u64,
    review_comment_id: Option<u64>,
    cx: &mut App,
  ) -> bool {
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
          if let Some(comment_id) = review_comment_id {
            page.reveal_pull_request_review_comment(comment_id, cx);
          }
        });
      });
    });
    true
  }

  /// For a link that asked Reviu to review the pull request: the shell offers to
  /// check out the branch that carries it, or says why it cannot.
  pub(crate) fn offer_checkout(
    owner: String,
    repo: String,
    number: u64,
    review_comment_id: Option<u64>,
    cx: &mut App,
  ) {
    let Some(page) = Self::page(cx) else {
      cx.open_url(&crate::github_navigation::github_pull_request_url(
        &owner,
        &repo,
        number,
        false,
        review_comment_id,
      ));
      return;
    };
    let Ok(window_handle) = page.read_with(cx, |page, _| page.window_handle()) else {
      cx.open_url(&crate::github_navigation::github_pull_request_url(
        &owner,
        &repo,
        number,
        false,
        review_comment_id,
      ));
      return;
    };
    if let Some(comment_id) = review_comment_id {
      Self::expect_comment((owner.clone(), repo.clone(), number), comment_id, cx);
    }
    let _ = cx.update_window(window_handle, move |_, window, cx| {
      let _ = page.update(cx, |page, cx| {
        page.offer_pull_request_checkout(owner, repo, number, review_comment_id, window, cx);
      });
    });
  }
}
