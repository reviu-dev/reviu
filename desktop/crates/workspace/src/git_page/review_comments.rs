//! Local review comments: editor handlers, lifecycle and sending to the agent.

use super::*;

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

  pub(super) fn agent_review_original_lines_for_request(
    &self,
    request: &ReviewCommentCreateRequest,
    cx: &App,
  ) -> (Option<usize>, Vec<String>) {
    if request.side != ReviewCommentSide::Right {
      return (None, Vec::new());
    }

    let Some(editor) = self.editor.as_ref() else {
      return (None, Vec::new());
    };

    let start = request.start_line.unwrap_or(request.line).min(request.line);
    let end = request.start_line.unwrap_or(request.line).max(request.line);
    let document = editor.read(cx).document().clone();
    let document = document.read(cx);
    let mut lines = Vec::new();

    for line_ix in start..=end {
      let Some(line) = document.line_content(line_ix) else {
        continue;
      };
      lines.push(line.trim_end_matches(['\r', '\n']).to_string());
    }

    if lines.is_empty() {
      (None, Vec::new())
    } else {
      (Some(start.saturating_add(1)), lines)
    }
  }

  pub(super) fn root_agent_review_comment_id(&self, comment_id: u64) -> u64 {
    let mut root_id = comment_id;
    let mut current_id = Some(comment_id);
    for _ in 0..32 {
      let Some(id) = current_id else {
        break;
      };
      let Some(comment) = self
        .agent_review_comments
        .iter()
        .find(|comment| comment.id == id)
      else {
        break;
      };
      let Some(parent_id) = comment.in_reply_to_id else {
        root_id = comment.id;
        break;
      };
      root_id = parent_id;
      current_id = Some(parent_id);
    }
    root_id
  }

  pub(super) fn create_agent_review_comment(
    &mut self,
    request: ReviewCommentCreateRequest,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let parent = request.in_reply_to_id.and_then(|parent_id| {
      self
        .agent_review_comments
        .iter()
        .find(|comment| comment.id == parent_id)
        .cloned()
    });
    let Some(path) = parent
      .as_ref()
      .map(|comment| comment.path.clone())
      .or_else(|| self.selected_file.clone())
    else {
      self.finish_agent_review_create(Some(Arc::from("No selected file")), cx);
      return;
    };

    let (original_start_line, original_lines) = if request.in_reply_to_id.is_some() {
      (None, Vec::new())
    } else {
      self.agent_review_original_lines_for_request(&request, cx)
    };

    let id = self.next_agent_review_comment_id;
    self.next_agent_review_comment_id = self.next_agent_review_comment_id.saturating_add(1);
    self.agent_review_comments.push(LocalAgentReviewComment {
      id,
      in_reply_to_id: request.in_reply_to_id,
      path,
      line: request.line,
      side: request.side,
      start_line: request.start_line,
      start_side: request.start_side,
      body: request.body.clone(),
      original_start_line,
      original_lines,
      state: LocalAgentReviewCommentState::Draft,
    });
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
    if let Some(comment) = self
      .agent_review_comments
      .iter_mut()
      .find(|comment| comment.id == comment_id)
    {
      comment.body = body;
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
  }

  pub(super) fn delete_agent_review_comment(&mut self, comment_id: u64, cx: &mut Context<Self>) {
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.start_review_comment_delete_submission(comment_id, cx);
      });
    }

    let removed_root = self.root_agent_review_comment_id(comment_id);
    let removed_ids = self
      .agent_review_comments
      .iter()
      .filter(|comment| {
        comment.id == comment_id
          || comment.in_reply_to_id == Some(comment_id)
          || (comment_id == removed_root
            && self.root_agent_review_comment_id(comment.id) == removed_root)
      })
      .map(|comment| comment.id)
      .collect::<HashSet<_>>();
    self
      .agent_review_comments
      .retain(|comment| !removed_ids.contains(&comment.id));
    self.sync_agent_review_comments_to_editor(cx);
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.finish_review_comment_delete_submission(comment_id, cx);
      });
    }
    cx.notify();
  }

  pub(super) fn local_agent_review_comment_to_editor_comment(
    &self,
    comment: &LocalAgentReviewComment,
  ) -> ReviewComment {
    let line_label = Some(Arc::<str>::from(agent_review_line_label(comment)));
    let suggestion_context = if comment.original_lines.is_empty() {
      None
    } else {
      Some(SuggestionContext {
        original_start_line: comment.original_start_line,
        suggested_start_line: comment.original_start_line,
        original_lines: comment.original_lines.clone(),
        path: Arc::from(comment.path.to_string_lossy().as_ref()),
      })
    };

    ReviewComment {
      id: comment.id,
      in_reply_to_id: comment.in_reply_to_id,
      line: comment.line,
      side: comment.side,
      author: Arc::from(""),
      avatar_url: None,
      line_label,
      body: comment.body.clone(),
      suggestion_context,
      created_at: Arc::from(""),
      thread_id: None,
      is_resolved: false,
      is_outdated: matches!(comment.state, LocalAgentReviewCommentState::Outdated),
      viewer_can_resolve: false,
      viewer_can_unresolve: false,
      is_pending: false,
    }
  }

  pub(super) fn sync_agent_review_comments_to_editor(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let Some(selected_file) = self.selected_file.clone() else {
      editor.update(cx, |editor, cx| {
        editor.set_review_comments(Vec::new(), cx);
        editor.set_editable_review_comment_ids(std::iter::empty::<u64>(), cx);
      });
      return;
    };

    self.refresh_agent_review_comment_states_for_selected_file(cx);

    let comments = self
      .agent_review_comments
      .iter()
      .filter(|comment| comment.path == selected_file)
      .filter(|comment| {
        matches!(
          comment.state,
          LocalAgentReviewCommentState::Draft | LocalAgentReviewCommentState::Copied
        )
      })
      .map(|comment| self.local_agent_review_comment_to_editor_comment(comment))
      .collect::<Vec<_>>();
    let editable_ids = comments
      .iter()
      .map(|comment| comment.id)
      .collect::<Vec<_>>();

    editor.update(cx, |editor, cx| {
      editor.set_editable_review_comment_ids(editable_ids, cx);
      editor.set_review_comments(comments, cx);
    });
  }

  pub(super) fn refresh_agent_review_comment_states_for_selected_file(&mut self, cx: &App) -> bool {
    let Some(editor) = self.editor.clone() else {
      return false;
    };
    let Some(selected_file) = self.selected_file.clone() else {
      return false;
    };

    let current_file_lines = {
      let document = editor.read(cx).document().clone();
      let document = document.read(cx);
      (0..document.len_lines())
        .filter_map(|line_ix| {
          document
            .line_content(line_ix)
            .map(|line| line.trim_end_matches(['\r', '\n']).to_string())
        })
        .collect::<Vec<_>>()
    };

    let mut changed = false;
    for comment in &mut self.agent_review_comments {
      if comment.path != selected_file {
        continue;
      }
      let next_state = next_agent_review_comment_state(comment, &current_file_lines);
      if comment.state != next_state {
        comment.state = next_state;
        changed = true;
      }
    }

    changed
  }

  pub(super) fn send_agent_review_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.sync_agent_review_comments_to_editor(cx);
    let copyable_ids = self
      .agent_review_comments
      .iter()
      .filter(|comment| agent_review_comment_is_copyable(comment))
      .map(|comment| comment.id)
      .collect::<HashSet<_>>();

    if copyable_ids.is_empty() {
      window.push_notification(Notification::info("No local review comments to send"), cx);
      return;
    }

    let review = format_agent_review_export(&self.agent_review_comments);
    let _ = window;

    for comment in &mut self.agent_review_comments {
      if copyable_ids.contains(&comment.id) {
        comment.state = LocalAgentReviewCommentState::Copied;
      }
    }
    self.sync_agent_review_comments_to_editor(cx);
    cx.notify();

    // The agent lives in the sessions shell; route the batch there.
    crate::session_page::SessionPageHandle::send_review(review, cx);
  }
}
