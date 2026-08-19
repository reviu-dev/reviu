//! The agent side of the shell: sessions, checkpoints and review comments.

use super::*;

impl SessionPage {
  pub(super) fn deliver_selection_context(
    &mut self,
    path: String,
    text: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.ensure_agent_chat_view(window, cx);
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    panel.update(cx, |panel, cx| {
      panel.add_selection_context(path, text, window, cx);
    });
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  pub(super) fn flush_pending_review_export(&mut self, cx: &mut Context<Self>) {
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    if self.pending_review_export.is_none() || !panel.read(cx).is_ready() {
      return;
    }
    if let Some(text) = self.pending_review_export.take() {
      panel.update(cx, |panel, cx| {
        panel.send_external_review(text, cx);
      });
    }
  }

  pub(super) fn ensure_agent_chat_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(view) = self.agent_chat_view.as_ref()
      && view.read(cx).needs_reconnect()
    {
      self.agent_chat_view = None;
    }
    if self.agent_chat_view.is_some() {
      return;
    }
    prune_agent_chat_state_once();
    let cwd = self
      .selected_repo
      .clone()
      .unwrap_or_else(|| PathBuf::from("."));
    let state_dir =
      agent_chat_state_dir().map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &cwd));
    let backend = AgentSettings::load();
    let view = cx.new(|cx| AgentChatPanel::new(backend, cwd, state_dir, window, cx));
    // The sessions sidebar owns the conversation list; hide the panel's own controls.
    view.update(cx, |panel, _| {
      panel.set_conversation_controls_visible(false)
    });
    // Sidebar reads conversation state from the panel; re-render when it changes.
    // Also the flush point for a review export queued while the agent was connecting.
    cx.observe(&view, |this, _, cx| {
      this.flush_pending_review_export(cx);
      this.sync_session_list(cx);
      cx.notify();
    })
    .detach();
    cx.subscribe_in(
      &view,
      window,
      |this, _panel, event: &AgentChatPanelEvent, window, cx| match event {
        AgentChatPanelEvent::OpenPath { path, line } => {
          let rel_path = agent_path_to_repo_relative(path.clone(), this.selected_repo.as_deref());
          this.open_diff(rel_path, *line, window, cx);
        }
        AgentChatPanelEvent::TurnStarted => {
          this.create_turn_checkpoint(cx);
        }
        AgentChatPanelEvent::PermissionRequested => {
          this.notify_agent_attention("Reviu agent needs a decision", window, cx);
        }
        AgentChatPanelEvent::TurnFinished => {
          // A queued prompt draining into a fresh turn is not a stopping point.
          if !_panel.read(cx).is_turn_in_flight() {
            this.notify_agent_attention("Reviu agent finished", window, cx);
          }
          this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
          if let Some(editor) = this.editor.clone() {
            editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
          }
          this.sync_agent_review_comments_to_editor(cx);
          this.refresh_branch(cx);
        }
        AgentChatPanelEvent::RollbackRequested { ref_name } => {
          this.rollback_to_checkpoint(ref_name.clone(), window, cx);
        }
      },
    )
    .detach();
    self.agent_chat_view = Some(view);
    self.sync_session_list(cx);
  }

  /// Popup on the primary display when the agent needs eyes and the main
  /// window is inactive; clicking it brings the app back.
  pub(super) fn notify_agent_attention(
    &mut self,
    title: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    use crate::agent_notification::{AgentNotification, AgentNotificationEvent};

    if window.is_window_active() {
      return;
    }
    if !crate::config::AppSettings::get(cx).agent_notifications {
      return;
    }
    // One popup at a time: the newest wins.
    if let Some(existing) = self.agent_notification.take() {
      let _ = existing.update(cx, |_, window, _| window.remove_window());
    }
    let Some(screen) = cx.primary_display() else {
      return;
    };
    let caption = self
      .agent_chat_view
      .as_ref()
      .map(|panel| panel.read(cx).current_conversation().title.clone())
      .filter(|title| !title.is_empty())
      .unwrap_or_else(|| "Agent session".to_string());
    let main_window = self.window_handle;
    let title = title.to_string();
    let Ok(handle) = cx.open_window(AgentNotification::window_options(screen), |_, cx| {
      cx.new(|_| AgentNotification::new(title, caption))
    }) else {
      return;
    };
    let this = cx.entity().downgrade();
    let _ = handle.update(cx, |_, _, cx| {
      cx.subscribe(
        &cx.entity(),
        move |_, _, event: &AgentNotificationEvent, cx| {
          if matches!(event, AgentNotificationEvent::Accepted) {
            let _ = cx.update_window(main_window, |_, window, _| window.activate_window());
          }
          cx.defer({
            let this = this.clone();
            move |cx| {
              let _ = this.update(cx, |this, cx| this.dismiss_agent_notification(cx));
            }
          });
        },
      )
      .detach();
    });
    self.agent_notification = Some(handle);
  }

  pub(super) fn dismiss_agent_notification(&mut self, cx: &mut Context<Self>) {
    if let Some(handle) = self.agent_notification.take() {
      let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
  }

  pub(super) fn create_turn_checkpoint(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    let session_id = panel.read(cx).current_conversation().id.clone();

    cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move { git::create_checkpoint(&repo_root, &session_id) })
        .await;
      let Ok(checkpoint) = result else {
        return;
      };
      let _ = this.update(cx, |this, cx| {
        if let Some(panel) = this.agent_chat_view.clone() {
          panel.update(cx, |panel, cx| {
            panel.record_checkpoint(checkpoint.ref_name, cx);
          });
        }
      });
    })
    .detach();
  }

  pub(super) fn rollback_to_checkpoint(
    &mut self,
    ref_name: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    if panel.read(cx).is_turn_in_flight() {
      window.push_notification(
        Notification::info("Wait for the agent to finish before rolling back"),
        cx,
      );
      return;
    }
    let session_id = panel.read(cx).current_conversation().id.clone();

    cx.spawn(async move |this, cx| {
      let restore_repo_root = repo_root.clone();
      let restore_ref = ref_name.clone();
      let result = cx
        .background_spawn(async move {
          // Safety net: snapshot the current state so the rollback itself is undoable.
          git::create_checkpoint(&restore_repo_root, &session_id)?;
          git::restore_checkpoint(&restore_repo_root, &restore_ref)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            if let Some(panel) = this.agent_chat_view.clone() {
              panel.update(cx, |panel, cx| {
                panel.truncate_at_checkpoint(&ref_name, cx);
              });
            }
            this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
            if let Some(editor) = this.editor.clone() {
              editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
            }
            this.sync_agent_review_comments_to_editor(cx);
          }
          Err(error) => {
            let _ = cx.update_window(this.window_handle, |_, window, cx| {
              window
                .push_notification(Notification::error(format!("Rollback failed: {error}")), cx);
            });
          }
        }
        cx.notify();
      });
    })
    .detach();
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
          window.on_next_frame(move |_window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.create_agent_review_comment(request, cx);
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
          window.on_next_frame(move |_window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.update_agent_review_comment(comment_id, body, cx);
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
    });
  }

  pub(super) fn create_agent_review_comment(
    &mut self,
    request: ReviewCommentCreateRequest,
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

  pub(super) fn copyable_review_comment_count(&self) -> usize {
    self.agent_review.copyable_count()
  }

  pub(super) fn send_agent_review_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.sync_agent_review_comments_to_editor(cx);

    if self.agent_review.copyable_count() == 0 {
      window.push_notification(Notification::info("No review comments to send"), cx);
      return;
    }

    let review = self.agent_review.export();
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };

    let dispatched = panel.update(cx, |panel, cx| {
      if !panel.is_ready() {
        return false;
      }
      panel.send_external_review(review, cx)
    });

    if !dispatched {
      window.push_notification(
        Notification::info("Agent not ready yet. Try again in a moment."),
        cx,
      );
      cx.notify();
      return;
    }

    self.agent_review.mark_copyable_as_copied();
    self.sync_agent_review_comments_to_editor(cx);
    // Back to the conversation to watch the agent address the comments.
    self.close_diff(window, cx);
    cx.notify();
  }

  pub(super) fn send_review_comments_to_agent_action(
    &mut self,
    _: &SendReviewCommentsToAgent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.send_agent_review_to_agent(window, cx);
    cx.stop_propagation();
  }

  pub(super) fn comment_hunk_action(
    &mut self,
    _: &CommentHunk,
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

  pub(super) fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    // Revives the panel when the previous backend connection errored out.
    self.ensure_agent_chat_view(window, cx);
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    panel.update(cx, |panel, cx| panel.new_conversation(cx));
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  pub(super) fn delete_session(&mut self, id: &str, cx: &mut Context<Self>) {
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    panel.update(cx, |panel, cx| panel.delete_conversation(id, cx));
    cx.notify();
  }

  pub(super) fn select_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    self.ensure_agent_chat_view(window, cx);
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    if panel.read(cx).current_conversation().id == id {
      return;
    }
    panel.update(cx, |panel, cx| panel.load_conversation(id, cx));
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  pub(super) fn focus_agent_input_on_next_frame(
    &self,
    window: &mut Window,
    _cx: &mut Context<Self>,
  ) {
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    window.on_next_frame(move |window, cx| {
      let focus_handle = panel.read(cx).input_focus_handle(cx);
      window.focus(&focus_handle, cx);
    });
  }

  pub(super) fn agent_turn_in_flight(&self, cx: &App) -> bool {
    #[cfg(test)]
    if self.pretend_agent_turn_in_flight {
      return true;
    }
    self
      .agent_chat_view
      .as_ref()
      .is_some_and(|panel| panel.read(cx).is_turn_in_flight())
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::*;
  use crate::agent_review::LocalAgentReviewCommentState;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;
  #[gpui::test]
  async fn review_comments_create_sync_and_delete(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-comments");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, _window, cx| {
      page.create_agent_review_comment(create_request(0, "extract helper"), cx);
    });

    let comment_id = page.read_with(cx, |page, _| {
      assert_eq!(page.agent_review.all().len(), 1);
      let comment = &page.agent_review.all()[0];
      assert_eq!(comment.state, LocalAgentReviewCommentState::Draft);
      assert_eq!(comment.path, PathBuf::from("README.md"));
      // The snapshot of the commented line is what later tells whether the
      // agent addressed the comment.
      assert_eq!(comment.original_start_line, Some(1));
      assert_eq!(comment.original_lines, vec!["v2".to_string()]);
      assert_eq!(page.copyable_review_comment_count(), 1);
      comment.id
    });

    page.update(cx, |page, cx| {
      page.delete_agent_review_comment(comment_id, cx);
    });
    page.read_with(cx, |page, _| {
      assert!(page.agent_review.is_empty());
      assert_eq!(page.copyable_review_comment_count(), 0);
    });
  }

  #[gpui::test]
  async fn send_without_agent_panel_keeps_drafts(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-send-no-agent");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "still a draft"), cx);
      // No agent chat view mounted: the send must not mark anything as sent.
      assert!(page.agent_chat_view.is_none());
      page.send_agent_review_to_agent(window, cx);
    });

    page.read_with(cx, |page, _| {
      assert_eq!(page.agent_review.all().len(), 1);
      assert_eq!(
        page.agent_review.all()[0].state,
        LocalAgentReviewCommentState::Draft
      );
      assert_eq!(page.center, CenterView::Diff);
    });
  }
}
