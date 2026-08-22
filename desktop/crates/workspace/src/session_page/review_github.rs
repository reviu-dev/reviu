//! Commenting on a pull request from the shell. The dock panel owns the pull
//! request and its comments; this only wires the open diff to it.

use super::*;

use editor::ReviewCommentResolveHandler;

use crate::pull_request_review_comments::{editable_comment_ids, editor_review_comments};
use crate::review_destination::GithubReviewHandlers;

impl SessionPage {
  /// A file of the pull request takes comments, and they go to GitHub. The
  /// working tree is the agent's business, a commit snapshot nobody's.
  pub(super) fn install_github_review_handlers_for_editor(
    &mut self,
    editor: &Entity<Editor>,
    cx: &mut Context<Self>,
  ) {
    let view = cx.entity().downgrade();

    let create: ReviewCommentCreateHandler = Arc::new({
      let view = view.clone();
      move |request, _window, cx| {
        let _ = view.update(cx, |this, cx| {
          let Some(path) = this.selected_file.clone() else {
            return;
          };
          this.dock_panel.update(cx, |panel, cx| {
            panel.create_pull_request_review_comment(request.clone(), path, cx);
          });
        });
      }
    });

    let edit: ReviewCommentEditHandler = Arc::new({
      let view = view.clone();
      move |comment_id, body, _window, cx| {
        let _ = view.update(cx, |this, cx| {
          this.dock_panel.update(cx, |panel, cx| {
            panel.edit_pull_request_review_comment(comment_id, body.clone(), cx);
          });
        });
      }
    });

    let delete: ReviewCommentDeleteHandler = Arc::new({
      let view = view.clone();
      move |comment_id, window, cx| {
        let _ = view.update(cx, |this, cx| {
          this.confirm_github_review_comment_delete(comment_id, window, cx);
        });
      }
    });

    let cancel: ReviewCommentCancelHandler = Arc::new({
      let view = view.clone();
      move |window, cx| {
        let _ = view.update(cx, |this, cx| this.focus_page_on_next_frame(window, cx));
      }
    });

    let resolve: ReviewCommentResolveHandler = Arc::new({
      let view = view.clone();
      move |thread_id: Arc<str>, _root_comment_id, currently_resolved, _window, cx| {
        let _ = view.update(cx, |this, cx| {
          this.dock_panel.update(cx, |panel, cx| {
            panel.toggle_pull_request_review_thread(thread_id.clone(), currently_resolved, cx);
          });
        });
      }
    });

    let api = WorkspaceApi::global(cx).api.clone();
    configure_review(
      editor,
      ReviewDestination::Github(Box::new(GithubReviewHandlers {
        create,
        edit,
        delete,
        cancel,
        resolve,
        asset_url_resolver: crate::github_shared::make_asset_url_resolver(&api),
        // Composer preview, dropped images, applied suggestions and links out of
        // a comment body are not offered here yet.
        preview_renderer: None,
        link: None,
        image_upload: None,
        suggestion_action_factory: None,
      })),
      cx,
    );
    self.sync_github_review_comments(cx);
  }

  /// A published comment is public and gone for good; a draft of your own review
  /// is not.
  fn confirm_github_review_comment_delete(
    &mut self,
    comment_id: u64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let is_pending = self
      .dock_panel
      .read(cx)
      .pull_request_review_comments()
      .iter()
      .any(|comment| comment.id == comment_id && comment.is_pending);
    if is_pending {
      self.delete_github_review_comment(comment_id, cx);
      return;
    }

    let view = cx.entity();
    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      ConfirmDialog::new(
        SharedString::from("Delete this comment?"),
        div().child("It is already on GitHub, and deleting it cannot be undone."),
      )
      .confirm_text("Delete")
      .cancel_text("Cancel")
      .on_confirm(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.delete_github_review_comment(comment_id, cx)
        });
        true
      })
      .build(alert)
    });
  }

  fn delete_github_review_comment(&mut self, comment_id: u64, cx: &mut Context<Self>) {
    self.dock_panel.update(cx, |panel, cx| {
      panel.delete_pending_review_comment(comment_id, cx);
    });
  }

  /// What GitHub holds on the open file, hung on the lines it talks about.
  pub(super) fn sync_github_review_comments(&mut self, cx: &mut Context<Self>) {
    let Some(OpenedSnapshot::PullRequestRange { .. }) = self.opened_snapshot.as_ref() else {
      return;
    };
    let (Some(editor), Some(path)) = (self.editor.clone(), self.selected_file.clone()) else {
      return;
    };

    let viewer_login = AuthStateStore::get(cx).github_login();
    let panel = self.dock_panel.read(cx);
    let comments = panel.pull_request_review_comments();
    let editor_comments = editor_review_comments(comments, path.as_path());
    let editable = editable_comment_ids(comments, viewer_login.as_deref());
    let pr_number = panel.pull_request_number();
    let has_pending_review = panel.has_pending_pull_request_review();

    editor.update(cx, move |editor, cx| {
      editor.set_review_comment_pr_number(pr_number, cx);
      editor.set_editable_review_comment_ids(editable.iter().copied(), cx);
      editor.set_review_comments(editor_comments, cx);
      editor.set_has_pending_review(has_pending_review, cx);
    });
  }

  /// The composer closes on success and stays open, with the reason, on failure.
  pub(super) fn finish_github_review_comment(
    &mut self,
    error: Option<Arc<str>>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let failed = error.is_some();
    editor.update(cx, |editor, cx| {
      editor.finish_review_comment_create_submission(error, cx);
    });
    if !failed {
      self.focus_page_on_next_frame(window, cx);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::session_page::test_support::{add_session_page_window, await_open_file};
  use editor::{ReviewCapabilities, ReviewCommentDisplayMode};
  use git::test_support::{TempRepo, commit_text_file};
  use std::path::Path;

  /// A pull request whose range is the two commits of a temp repository.
  async fn open_pull_request_file(
    cx: &mut gpui::TestAppContext,
  ) -> (TempRepo, Entity<SessionPage>, &mut gpui::VisualTestContext) {
    let repo = TempRepo::init("session-page-pr-file");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let base = git::current_head_sha(&repo.path)
      .expect("head sha")
      .expect("head sha");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\nv2\n", "second");
    let head = git::current_head_sha(&repo.path)
      .expect("head sha")
      .expect("head sha");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.open_pull_request_file(base, head, PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    (repo, page, cx)
  }

  #[gpui::test]
  async fn a_pull_request_file_takes_comments_and_they_go_to_github(cx: &mut gpui::TestAppContext) {
    let (_repo, page, cx) = open_pull_request_file(cx).await;

    let capabilities = page.read_with(cx, |page, cx| {
      page
        .editor
        .as_ref()
        .expect("editor")
        .read(cx)
        .review_capabilities()
    });

    assert_eq!(
      capabilities,
      ReviewCapabilities {
        display_mode: ReviewCommentDisplayMode::Conversation,
        replies_enabled: true,
        create: true,
        edit: true,
        delete: true,
        cancel: true,
        // Nothing to send to: these belong to the pull request.
        send: false,
        resolve: true,
        link: false,
        image_upload: false,
        asset_url_resolver: true,
        preview_renderer: false,
        suggestion_action_factory: false,
        pr_number: None,
      }
    );
  }

  #[gpui::test]
  async fn only_the_comments_of_the_open_file_hang_in_the_diff(cx: &mut gpui::TestAppContext) {
    let (_repo, page, cx) = open_pull_request_file(cx).await;

    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_pull_request_review_comments_for_test(
          vec![
            crate::pull_request_review_comments::pending_comment_fixture(
              1,
              "a.txt",
              Some(2),
              "here",
            ),
            crate::pull_request_review_comments::pending_comment_fixture(
              2,
              "other.txt",
              Some(1),
              "elsewhere",
            ),
          ],
          cx,
        );
      });
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      assert_eq!(editor.review_comment_ids(), vec![1]);
      // Written but not submitted: the composer must offer to join the review.
      assert!(editor.has_pending_review());
    });
  }
}
