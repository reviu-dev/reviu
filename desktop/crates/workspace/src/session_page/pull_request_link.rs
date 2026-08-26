//! A link asking Reviu to review a pull request the open branch does not carry:
//! the shell offers to check out the branch that does.

use super::*;

use gpui::AnyWindowHandle;

use crate::github_navigation::github_pull_request_url;
use crate::pull_request_surface::{PullRequestIdentity, PullRequestSurfaceHandle};
use crate::workspace::WorkspaceApi;

/// Why a link cannot be answered here. Reviu reviews what is checked out, so
/// every case ends with the only other place the pull request can be read.
enum CheckoutRefusal {
  NoRepository,
  OtherRepository { open: Option<String> },
  Unreachable,
}

impl CheckoutRefusal {
  fn title(&self) -> SharedString {
    match self {
      Self::NoRepository => "No repository open".into(),
      Self::OtherRepository { .. } => "Another repository is open".into(),
      Self::Unreachable => "GitHub did not answer".into(),
    }
  }

  fn message(&self, owner: &str, repo: &str, number: u64) -> String {
    match self {
      Self::NoRepository => {
        format!("Open {owner}/{repo} to review #{number} here.")
      }
      Self::OtherRepository { open: Some(open) } => {
        format!("Reviu has {open} open, and #{number} belongs to {owner}/{repo}.")
      }
      Self::OtherRepository { open: None } => {
        format!("The open repository is not {owner}/{repo}.")
      }
      Self::Unreachable => format!("Reviu could not read #{number} on GitHub."),
    }
  }
}

impl SessionPage {
  /// A link named a review comment of the pull request the panel is showing:
  /// the diff opens on the lines it is about.
  pub(crate) fn reveal_pull_request_review_comment(
    &mut self,
    comment_id: u64,
    cx: &mut Context<Self>,
  ) {
    self.dock_panel.update(cx, |panel, cx| {
      panel.reveal_review_comment(comment_id, cx);
    });
  }

  /// The link named a pull request of the open repository: find its branch and
  /// offer the checkout. Anything else belongs to github.com.
  pub(crate) fn offer_pull_request_checkout(
    &mut self,
    owner: String,
    repo: String,
    number: u64,
    review_comment_id: Option<u64>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.session_repo(cx) else {
      let _ = window;
      explain_not_open(
        CheckoutRefusal::NoRepository,
        owner,
        repo,
        number,
        review_comment_id,
        self.window_handle,
        cx,
      );
      return;
    };

    let api = WorkspaceApi::global(cx).api.clone();
    let window_handle = self.window_handle;
    let lookup_owner = owner.clone();
    let lookup_repo = repo.clone();
    let task = cx.spawn(async move |this, cx| {
      let found = cx
        .background_spawn(async move {
          let remote = git::current_github_remote_repo(&repo_root).ok().flatten();
          let opens_it = remote.as_ref().is_some_and(|remote| {
            remote.owner.eq_ignore_ascii_case(&lookup_owner)
              && remote.repo.eq_ignore_ascii_case(&lookup_repo)
          });
          if !opens_it {
            return Err(CheckoutRefusal::OtherRepository {
              open: remote.map(|remote| format!("{}/{}", remote.owner, remote.repo)),
            });
          }
          let details = api
            .fetch_pull_request_details(&lookup_owner, &lookup_repo, number)
            .map_err(|_| CheckoutRefusal::Unreachable)?;
          let current = git::current_branch_status(&repo_root)
            .ok()
            .map(|status| status.name);
          Ok((details.head_ref_name, current))
        })
        .await;

      let _ = cx.update_window(window_handle, move |_, window, cx| {
        let _ = this.update(cx, |this, cx| match found {
          Ok((branch, current)) => this.ask_to_check_out_pull_request_branch(
            (owner, repo, number),
            branch,
            current,
            window,
            cx,
          ),
          Err(refusal) => explain_not_open(
            refusal,
            owner,
            repo,
            number,
            review_comment_id,
            window_handle,
            cx,
          ),
        });
      });
    });
    self._pull_request_link_task = Some(task);
  }

  /// Moving HEAD is the user's call, so the branch is named before anything
  /// happens. A branch already checked out only waits for its panel.
  fn ask_to_check_out_pull_request_branch(
    &mut self,
    identity: PullRequestIdentity,
    branch: String,
    current_branch: Option<String>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if current_branch.as_deref() == Some(branch.as_str()) {
      PullRequestSurfaceHandle::expect(identity, cx);
      self.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
      return;
    }

    let number = identity.2;
    let view = cx.entity();
    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let identity = identity.clone();
      let branch = branch.clone();
      ConfirmDialog::new(
        SharedString::from(format!("Check out {branch}?")),
        div().child(format!(
          "Reviu reviews the branch it has open. Check out {branch} to review #{number} here?"
        )),
      )
      .confirm_text("Check out")
      .cancel_text("Cancel")
      .on_confirm(move |_, window, cx| {
        view.update(cx, |view, cx| {
          view.check_out_pull_request_branch(identity.clone(), branch.clone(), window, cx);
        });
        true
      })
      .build(alert)
    });
  }

  fn check_out_pull_request_branch(
    &mut self,
    identity: PullRequestIdentity,
    branch: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let command = RepoCommand::SwitchToBranchName {
      name: branch.clone(),
    };
    match self.run_branch_command(command, window, cx) {
      // The panel opens on the refresh that follows the checkout, not here: the
      // pull request of the branch is only known once git has moved.
      Ok(()) => PullRequestSurfaceHandle::expect(identity, cx),
      Err(error) => window.push_notification(Notification::warning(error), cx),
    }
  }
}

/// The link asked to review it here, so bouncing to the browser without a word
/// would be no answer at all: say why, and offer the only place left.
fn explain_not_open(
  refusal: CheckoutRefusal,
  owner: String,
  repo: String,
  number: u64,
  review_comment_id: Option<u64>,
  window_handle: AnyWindowHandle,
  cx: &mut App,
) {
  let title = refusal.title();
  let message = refusal.message(&owner, &repo, number);
  let _ = cx.update_window(window_handle, move |_, window, cx| {
    let owner = owner.clone();
    let repo = repo.clone();
    let message = message.clone();
    window.push_notification(
      Notification::info(message).title(title.clone()).content({
        move |_, _, _| {
          let owner = owner.clone();
          let repo = repo.clone();
          div()
            .flex()
            .mt_3()
            .child(
              Button::new("pull-request-link-open-on-github")
                .primary()
                .compact()
                .small()
                .label("Open on GitHub")
                .on_click(move |_, _, cx| {
                  cx.open_url(&github_pull_request_url(
                    &owner,
                    &repo,
                    number,
                    false,
                    review_comment_id,
                  ));
                }),
            )
            .into_any_element()
        }
      }),
      cx,
    );
  });
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::super::*;
  use crate::pull_request_surface::PullRequestSurfaceHandle;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;

  fn identity(number: u64) -> (String, String, u64) {
    ("acme".to_string(), "widget".to_string(), number)
  }

  #[gpui::test]
  async fn the_panel_opens_on_the_pull_request_the_link_was_waiting_for(cx: &mut TestAppContext) {
    let repo = TempRepo::init("pull-request-link-lands");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    cx.update(|_, cx| PullRequestSurfaceHandle::expect(identity(42), cx));

    // The checkout landed: the branch now carries the pull request that was asked for.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(surface_pull_request(42), cx);
      });
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert!(page.dock_open);
      assert_eq!(
        page.dock_panel.read(cx).active_tab(),
        DockPanelTab::PullRequest
      );
    });
  }

  #[gpui::test]
  async fn the_comment_a_link_named_survives_the_checkout(cx: &mut TestAppContext) {
    let repo = TempRepo::init("pull-request-link-comment");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    cx.update(|_, cx| {
      PullRequestSurfaceHandle::expect(identity(42), cx);
      PullRequestSurfaceHandle::expect_comment(identity(42), 9, cx);
    });

    let panel = page.read_with(cx, |page, _| page.dock_panel.clone());
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(
        &panel,
        move |_panel, event: &crate::dock_panel::DockPanelEvent, _cx| {
          if let crate::dock_panel::DockPanelEvent::OpenPullRequestFile { path, line, .. } = event {
            seen.borrow_mut().push((path.clone(), *line));
          }
        },
      )
      .detach();
    });

    // The checkout landed: the branch carries the pull request the link asked for.
    panel.update(cx, |panel, cx| {
      panel.set_pull_request_range_for_test(crate::dock_panel::PullRequestRange {
        base: "b".repeat(40),
        head: "h".repeat(40),
        base_ref: "main".to_string(),
        head_ref: "feature".to_string(),
      });
      panel.set_branch_pull_request_state(surface_pull_request(42), cx);
    });
    cx.run_until_parked();

    panel.update(cx, |panel, cx| {
      panel.set_pull_request_review_comments_for_test(
        vec![
          crate::pull_request_review_comments::pending_comment_fixture(
            9,
            "README.md",
            Some(3),
            "here",
          ),
        ],
        cx,
      );
    });
    cx.run_until_parked();

    assert_eq!(
      opened.borrow().as_slice(),
      &[(std::path::PathBuf::from("README.md"), Some(3))],
      "the comment the link named outlives the checkout it triggered"
    );
  }

  #[gpui::test]
  async fn a_link_survives_the_refreshes_that_answer_nothing(cx: &mut TestAppContext) {
    let repo = TempRepo::init("pull-request-link-waits");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    cx.update(|_, cx| PullRequestSurfaceHandle::expect(identity(42), cx));

    // A branch with no pull request to show says nothing about the link.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(crate::dock_panel::BranchPrState::NoRemote, cx);
      });
    });
    cx.run_until_parked();

    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(surface_pull_request(42), cx);
      });
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.dock_panel.read(cx).active_tab(),
        DockPanelTab::PullRequest
      );
    });
  }

  #[gpui::test]
  async fn a_checkout_that_lands_elsewhere_leaves_nothing_armed(cx: &mut TestAppContext) {
    let repo = TempRepo::init("pull-request-link-misses");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    cx.update(|_, cx| PullRequestSurfaceHandle::expect(identity(42), cx));

    // Another pull request answers first: the link it was waiting for is dropped.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(surface_pull_request(7), cx);
      });
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_ne!(
        page.dock_panel.read(cx).active_tab(),
        DockPanelTab::PullRequest,
        "the panel belongs to the link, not to any pull request"
      );
    });

    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(surface_pull_request(42), cx);
      });
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_ne!(
        page.dock_panel.read(cx).active_tab(),
        DockPanelTab::PullRequest,
        "a link is answered once, not the next time that branch comes back"
      );
    });
  }

  #[gpui::test]
  async fn a_checkout_that_fails_leaves_nothing_armed(cx: &mut TestAppContext) {
    let repo = TempRepo::init("pull-request-link-checkout-fails");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let branch_before = git::current_branch_status(&repo.path)
      .expect("branch status")
      .name;

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // No such branch, here or on any remote: the checkout fails.
    page.update_in(cx, |page, window, cx| {
      page.check_out_pull_request_branch(identity(42), "nowhere".to_string(), window, cx);
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert_eq!(
      git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      branch_before
    );

    // Checking that branch out by hand later is not the link answering late.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(surface_pull_request(42), cx);
      });
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_ne!(
        page.dock_panel.read(cx).active_tab(),
        DockPanelTab::PullRequest
      );
    });
  }

  #[gpui::test]
  async fn a_link_does_not_move_the_branch_under_a_running_agent(cx: &mut TestAppContext) {
    let repo = TempRepo::init("pull-request-link-agent-busy");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let branch_before = git::current_branch_status(&repo.path)
      .expect("branch status")
      .name;

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update(cx, |page, _| page.pretend_agent_turn_in_flight = true);

    page.update_in(cx, |page, window, cx| {
      page.check_out_pull_request_branch(identity(42), "feature".to_string(), window, cx);
    });
    cx.run_until_parked();

    assert_eq!(
      git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      branch_before
    );
    page.read_with(cx, |page, _| {
      assert!(page._repo_command_task.is_none(), "nothing was launched");
    });

    // Nothing is armed either: the pull request that lands next is not this link.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(surface_pull_request(42), cx);
      });
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_ne!(
        page.dock_panel.read(cx).active_tab(),
        DockPanelTab::PullRequest
      );
    });
  }

  #[gpui::test]
  async fn the_branch_already_checked_out_only_waits_for_its_panel(cx: &mut TestAppContext) {
    let repo = TempRepo::init("pull-request-link-same-branch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let branch = git::current_branch_status(&repo.path)
      .expect("branch status")
      .name;

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.ask_to_check_out_pull_request_branch(
        identity(42),
        branch.clone(),
        Some(branch.clone()),
        window,
        cx,
      );
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert!(
        page._repo_command_task.is_none(),
        "the branch is already the one the link wants"
      );
    });

    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(surface_pull_request(42), cx);
      });
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.dock_panel.read(cx).active_tab(),
        DockPanelTab::PullRequest
      );
    });
  }
}
