//! Local review comments: editor handlers and the page-side glue around
//! [`AgentReviewComments`].

use super::*;

use crate::agent_review::{editor_file_lines, original_lines_for_request, sync_comments_to_editor};

impl GitPage {
  pub(super) fn send_review_comments_to_agent_action(
    &mut self,
    _: &crate::SendReviewCommentsToAgent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.send_agent_review_to_agent(window, cx);
    cx.stop_propagation();
  }

  pub(super) fn add_selection_to_agent_action(
    &mut self,
    _: &crate::AddSelectionToAgent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.add_selection_to_agent(window, cx);
    cx.stop_propagation();
  }

  pub(super) fn add_selection_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      window.push_notification(Notification::info("Open a file diff first"), cx);
      return;
    };
    let Some(text) = editor.read(cx).selected_text_for_copy(cx) else {
      window.push_notification(Notification::info("Select code in the diff first"), cx);
      return;
    };
    let path = self
      .selected_file
      .as_ref()
      .map(|p| p.to_string_lossy().to_string())
      .unwrap_or_else(|| "selection".to_string());

    // The agent lives in the sessions shell; attach the selection there.
    crate::session_page::SessionPageHandle::add_selection(path, text, cx);
  }

  pub(super) fn comment_hunk_action(
    &mut self,
    _: &crate::CommentHunk,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    if editor.update(cx, |editor, cx| {
      editor.start_review_comment_for_active_hunk(window, cx)
    }) {
      cx.stop_propagation();
    }
  }

  pub(super) fn install_agent_review_handlers_for_editor(
    &mut self,
    editor: &Entity<Editor>,
    cx: &mut Context<Self>,
  ) {
    let view = cx.entity().downgrade();
    editor.update(cx, |editor, cx| {
      let create_handler: ReviewCommentCreateHandler = Arc::new({
        let view = view.clone();
        move |request, window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.create_agent_review_comment(request, window, cx);
            });
          });
        }
      });
      editor.set_review_comment_create_handler(Some(create_handler), cx);
      editor.set_review_comment_replies_enabled(false, cx);
      editor.set_review_comment_display_mode(ReviewCommentDisplayMode::LocalNote, cx);

      let edit_handler: ReviewCommentEditHandler = Arc::new({
        let view = view.clone();
        move |comment_id, body, window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.update_agent_review_comment(comment_id, body, window, cx);
            });
          });
        }
      });
      editor.set_review_comment_edit_handler(Some(edit_handler), cx);

      let delete_handler: ReviewCommentDeleteHandler = Arc::new({
        let view = view.clone();
        move |comment_id, window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |_window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.delete_agent_review_comment(comment_id, cx);
            });
          });
        }
      });
      editor.set_review_comment_delete_handler(Some(delete_handler), cx);

      let cancel_handler: ReviewCommentCancelHandler = Arc::new({
        let view = view.clone();
        move |window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
              if this.sidebar_mode == GitSidebarMode::Changes {
                this.focus_changes_sidebar_list(window, cx);
              }
            });
          });
        }
      });
      editor.set_review_comment_cancel_handler(Some(cancel_handler), cx);
      editor.set_review_comment_card_width(px(600.0), cx);
      editor.set_review_comment_textarea_height(px(90.0), cx);
    });
  }

  pub(super) fn create_agent_review_comment(
    &mut self,
    request: ReviewCommentCreateRequest,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let original = original_lines_for_request(self.editor.as_ref(), &request, cx);
    let created = self
      .agent_review
      .create(&request, self.selected_file.as_deref(), original);

    if let Err(error) = created {
      self.finish_agent_review_create(Some(error), cx);
      return;
    }

    self.sync_agent_review_comments_to_editor(cx);
    self.finish_agent_review_create(None, cx);
    if self.sidebar_mode == GitSidebarMode::Changes {
      self.focus_changes_sidebar_list(window, cx);
    }
    cx.notify();
  }

  pub(super) fn finish_agent_review_create(
    &mut self,
    error: Option<Arc<str>>,
    cx: &mut Context<Self>,
  ) {
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.finish_review_comment_create_submission(error, cx);
      });
    }
  }

  pub(super) fn update_agent_review_comment(
    &mut self,
    comment_id: u64,
    body: Arc<str>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !self.agent_review.update(comment_id, body) {
      return;
    }

    self.sync_agent_review_comments_to_editor(cx);
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.finish_review_comment_edit_submission(comment_id, None, cx);
      });
    }
    if self.sidebar_mode == GitSidebarMode::Changes {
      self.focus_changes_sidebar_list(window, cx);
    }
    cx.notify();
  }

  pub(super) fn delete_agent_review_comment(&mut self, comment_id: u64, cx: &mut Context<Self>) {
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.start_review_comment_delete_submission(comment_id, cx);
      });
    }

    self.agent_review.delete(comment_id);
    self.sync_agent_review_comments_to_editor(cx);

    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.finish_review_comment_delete_submission(comment_id, cx);
      });
    }
    cx.notify();
  }

  pub(super) fn sync_agent_review_comments_to_editor(&mut self, cx: &mut Context<Self>) {
    let editor = self.editor.clone();
    sync_comments_to_editor(
      &mut self.agent_review,
      editor.as_ref(),
      self.selected_file.as_deref(),
      cx,
    );
  }

  pub(super) fn refresh_agent_review_comment_states_for_selected_file(&mut self, cx: &App) -> bool {
    let Some(editor) = self.editor.clone() else {
      return false;
    };
    let Some(selected_file) = self.selected_file.clone() else {
      return false;
    };

    let file_lines = editor_file_lines(&editor, cx);
    self
      .agent_review
      .refresh_states(&selected_file, &file_lines)
  }

  pub(super) fn send_agent_review_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.sync_agent_review_comments_to_editor(cx);

    if self.agent_review.copyable_count() == 0 {
      window.push_notification(Notification::info("No local review comments to send"), cx);
      return;
    }

    let review = self.agent_review.export();
    self.agent_review.mark_copyable_as_copied();
    self.sync_agent_review_comments_to_editor(cx);
    cx.notify();

    // The agent lives in the sessions shell; route the batch there.
    crate::session_page::SessionPageHandle::send_review(review, cx);
  }
}
