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
    let close_control_visible = self.center == CenterView::Diff && self.diff_chat_open;
    view.update(cx, |panel, cx| {
      panel.set_conversation_controls_visible(false);
      panel.set_close_control_visible(close_control_visible, cx);
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
        AgentChatPanelEvent::UndoTurnRequested { ref_name } => {
          this.undo_turn_files(ref_name.clone(), window, cx);
        }
        AgentChatPanelEvent::ConversationsChanged => {
          this.refresh_session_list(cx);
        }
        AgentChatPanelEvent::CloseRequested => {
          this.hide_diff_chat(window, cx);
        }
      },
    )
    .detach();
    self.agent_chat_view = Some(view);
    self.refresh_session_list(cx);
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
    let agent_id = self
      .agent_chat_view
      .as_ref()
      .map(|panel| panel.read(cx).backend_kind().clone())
      .unwrap_or_else(agent_chat_panel::default_agent_id);
    let icon = agent_chat_panel::backend_icon(&agent_id);
    let Ok(handle) = cx.open_window(AgentNotification::window_options(screen), |_, cx| {
      cx.new(|_| AgentNotification::new(icon, title, caption))
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

  /// Reverts one turn's file changes; the transcript keeps the turn and its
  /// summary card flips to "Undone".
  pub(super) fn undo_turn_files(
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
        Notification::info("Wait for the agent to finish before undoing"),
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
          // Safety net: snapshot the current state so the undo itself is undoable.
          git::create_checkpoint(&restore_repo_root, &session_id)?;
          git::restore_checkpoint(&restore_repo_root, &restore_ref)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            if let Some(panel) = this.agent_chat_view.clone() {
              panel.update(cx, |panel, cx| {
                panel.mark_turn_undone(&ref_name, cx);
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
              window.push_notification(Notification::error(format!("Undo failed: {error}")), cx);
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

      let cancel_handler: ReviewCommentCancelHandler = Arc::new({
        let view = view.clone();
        move |window, cx| {
          let _ = view.update(cx, |this, cx| this.focus_page_on_next_frame(window, cx));
        }
      });
      editor.set_review_comment_cancel_handler(Some(cancel_handler), cx);

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

      let send_handler: ReviewCommentSendHandler = Arc::new({
        let view = view.clone();
        move |comment_id, window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.send_agent_review_comment_to_agent(comment_id, window, cx);
            });
          });
        }
      });
      editor.set_review_comment_send_handler(Some(send_handler), cx);
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
      // The composer stays open on the error, so the focus stays in it.
      self.finish_agent_review_create(Some(error), cx);
      return;
    }

    self.sync_agent_review_comments_to_editor(cx);
    self.finish_agent_review_create(None, cx);
    self.focus_page_on_next_frame(window, cx);
    // An open dock follows what you are doing; a closed one keeps the tab you
    // left it on, and the rail badge says the batch grew.
    if self.dock_open {
      self
        .dock_panel
        .update(cx, |panel, cx| panel.select_tab(DockPanelTab::Review, cx));
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
    self.focus_page_on_next_frame(window, cx);
    cx.notify();
  }

  /// A composer that closes takes the focus with it, and the shortcuts stop
  /// answering until something takes it back.
  pub(super) fn focus_page_on_next_frame(&self, window: &mut Window, cx: &mut Context<Self>) {
    let view = cx.entity().downgrade();
    window.on_next_frame(move |window, cx| {
      let _ = view.update(cx, |this, cx| {
        let handle = this.focus_handle(cx);
        window.focus(&handle, cx);
      });
    });
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
    self.sync_review_panel(cx);
    self.persist_agent_review();
  }

  /// Every change goes through here, so the file on disk is never behind. The
  /// batch only moves on discrete gestures, which is why this needs no throttle.
  pub(super) fn persist_agent_review(&mut self) {
    if !self.agent_review.take_dirty() {
      return;
    }
    let Some(path) = self.review_store_path.clone() else {
      return;
    };
    write_review(&path, self.agent_review.all(), self.agent_review.next_id());
  }

  /// The panel shows the whole batch, including what the agent already addressed:
  /// the diff drops those, the review is where you see they were dealt with.
  pub(super) fn sync_review_panel(&mut self, cx: &mut Context<Self>) {
    let comments = review_panel_comments(self.agent_review.all());
    self.dock_panel.update(cx, |panel, cx| {
      panel
        .review_list
        .update(cx, |list, cx| list.set_comments(comments, cx));
    });
  }

  fn review_panel_selection(&self, cx: &App) -> HashSet<u64> {
    self
      .dock_panel
      .read(cx)
      .review_list
      .read(cx)
      .selected_ids()
      .clone()
  }

  /// Discarding is not undoable, and the batch is not persisted anywhere.
  pub(super) fn confirm_discard_agent_review(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let count = self.agent_review.all().len();
    if count == 0 {
      return;
    }
    let title: SharedString = "Discard this review?".into();
    let message: SharedString = if count == 1 {
      "Delete the comment of this review?".into()
    } else {
      format!("Delete the {count} comments of this review?").into()
    };
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Discard")
        .cancel_text("Cancel")
        .on_confirm(move |_, _, cx| {
          view.update(cx, |this, cx| this.discard_agent_review(cx));
          true
        })
        .build(alert)
    });
  }

  pub(super) fn discard_agent_review(&mut self, cx: &mut Context<Self>) {
    self.agent_review.clear();
    self.sync_agent_review_comments_to_editor(cx);
    cx.notify();
  }

  pub(super) fn copyable_review_comment_count(&self) -> usize {
    self.agent_review.copyable_count()
  }

  /// The panel's ticks decide what goes; nothing ticked sends the whole batch.
  pub(super) fn send_agent_review_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let send = ReviewSend::from_selection(self.review_panel_selection(cx));
    self.send_agent_review(send, window, cx);
  }

  pub(super) fn send_agent_review_comment_to_agent(
    &mut self,
    comment_id: u64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.send_agent_review(ReviewSend::one(comment_id), window, cx);
  }

  fn send_agent_review(&mut self, send: ReviewSend, window: &mut Window, cx: &mut Context<Self>) {
    self.sync_agent_review_comments_to_editor(cx);

    if self.agent_review.sendable_count(&send) == 0 {
      window.push_notification(Notification::info("No review comments to send"), cx);
      return;
    }

    let review = self.agent_review.export(&send);
    #[cfg(test)]
    {
      self.last_review_export = Some(review.clone());
    }
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

    self.agent_review.mark_as_copied(&send);
    // Only what went out loses its tick: sending one comment on its own leaves
    // the batch someone was building alone.
    if let ReviewSend::Only(ids) = &send {
      let ids = ids.clone();
      self.deselect_review_panel_comments(&ids, cx);
    }
    self.sync_agent_review_comments_to_editor(cx);
    // Back to the conversation to watch the agent address the comments.
    self.close_diff(window, cx);
    cx.notify();
  }

  fn deselect_review_panel_comments(&mut self, ids: &HashSet<u64>, cx: &mut Context<Self>) {
    self.dock_panel.update(cx, |panel, cx| {
      panel
        .review_list
        .update(cx, |list, cx| list.deselect(ids, cx));
    });
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

  pub(super) fn jump_to_latest_message_action(
    &mut self,
    _: &JumpToLatestMessage,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    panel.update(cx, |panel, cx| {
      panel.jump_to_tail();
      cx.notify();
    });
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
    panel.update(cx, |panel, cx| panel.new_conversation(window, cx));
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  pub(super) fn delete_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    panel.update(cx, |panel, cx| panel.delete_conversation(id, window, cx));
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
    panel.update(cx, |panel, cx| panel.load_conversation(id, window, cx));
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
  use crate::review_list::ReviewListEvent;
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

    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "extract helper"), window, cx);
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
  async fn the_session_composer_offers_a_single_destination(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-one-action");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    let mode = page.read_with(cx, |page, cx| {
      page
        .editor
        .as_ref()
        .expect("editor")
        .read(cx)
        .review_comment_display_mode()
    });

    assert_eq!(mode, ReviewCommentDisplayMode::LocalNote);
    // Comments go to the agent batch, so the GitHub review destinations stay out.
    let actions = editor::review_comment_create_actions(mode, false);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].mode, editor::ReviewCommentMode::SingleComment);
  }

  /// The card is painted over lines the diff set aside for it. Every sizing rule in
  /// the editor exists to keep those two in step; this is where they are compared.
  #[gpui::test]
  async fn a_review_comment_card_fits_the_lines_the_diff_reserved(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-card-fits");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    for body in [
      "short",
      "a comment long enough to wrap over several lines of the card,        with words of every width: iiii MMMM 0123456789 and a bit more prose",
      "first line\nsecond line\nthird line",
    ] {
      page.update_in(cx, |page, window, cx| {
        page.agent_review.clear();
        page.create_agent_review_comment(create_request(0, body), window, cx);
      });
      await_editor_diff(&page, cx).await;

      let block = cx
        .debug_bounds(editor::REVIEW_COMMENT_BLOCK_DEBUG_SELECTOR)
        .expect("the diff reserves a block for the comment");
      let card = cx
        .debug_bounds(editor::REVIEW_COMMENT_CARD_DEBUG_SELECTOR)
        .expect("the comment paints a card");

      assert!(
        card.bottom() <= block.bottom(),
        "the card runs {} past the lines reserved for {body:?}",
        card.bottom() - block.bottom()
      );
      assert!(
        card.top() >= block.top(),
        "the card starts above the lines reserved for {body:?}"
      );
    }
  }

  #[gpui::test]
  async fn a_new_comment_lands_in_the_review_panel(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-panel");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "extract helper"), window, cx);
    });

    page.read_with(cx, |page, cx| {
      let list = page.dock_panel.read(cx).review_list.read(cx);
      assert_eq!(list.comments().len(), 1);
      assert_eq!(list.comments()[0].excerpt, "extract helper");
      assert_eq!(list.comments()[0].path, PathBuf::from("README.md"));
    });
  }

  #[gpui::test]
  async fn a_closed_dock_keeps_the_tab_it_was_left_on(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-tab");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    // Closed: the batch grows behind the rail badge, the tab does not move.
    page.update_in(cx, |page, window, cx| page.close_dock(window, cx));
    cx.run_until_parked();
    page.read_with(cx, |page, _| assert!(!page.dock_open));
    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "first"), window, cx);
    });
    page.read_with(cx, |page, cx| {
      assert_eq!(page.dock_panel.read(cx).active_tab(), DockPanelTab::Changes);
    });

    // Open: the panel follows what the page is doing.
    page.update_in(cx, |page, window, cx| {
      page.open_changes_action(&crate::OpenGitChangesSidebar, window, cx)
    });
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(1, "second"), window, cx);
    });
    page.read_with(cx, |page, cx| {
      assert_eq!(page.dock_panel.read(cx).active_tab(), DockPanelTab::Review);
    });
  }

  #[gpui::test]
  async fn the_panel_rows_reach_the_batch_and_the_diff(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-rows");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "extract helper"), window, cx);
    });

    let (review_list, comment_id) = page.read_with(cx, |page, cx| {
      (
        page.dock_panel.read(cx).review_list.clone(),
        page.agent_review.all()[0].id,
      )
    });

    // A row opens the file it is about.
    page.update(cx, |page, _| page.selected_file = None);
    review_list.update(cx, |_, cx| {
      cx.emit(ReviewListEvent::OpenComment {
        path: PathBuf::from("README.md"),
        line: 0,
      });
    });
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_file, Some(PathBuf::from("README.md")));
    });

    // And its delete button takes the comment out of the batch.
    review_list.update(cx, |_, cx| {
      cx.emit(ReviewListEvent::DeleteComment { id: comment_id });
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert!(page.agent_review.all().is_empty());
      assert!(
        page
          .dock_panel
          .read(cx)
          .review_list
          .read(cx)
          .comments()
          .is_empty()
      );
    });
  }

  #[gpui::test]
  async fn discarding_a_review_empties_the_batch(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-discard");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "first"), window, cx);
      page.create_agent_review_comment(create_request(0, "second"), window, cx);
    });

    page.update(cx, |page, cx| page.discard_agent_review(cx));

    page.read_with(cx, |page, cx| {
      assert!(page.agent_review.all().is_empty());
      assert_eq!(page.copyable_review_comment_count(), 0);
      assert!(
        page
          .dock_panel
          .read(cx)
          .review_list
          .read(cx)
          .comments()
          .is_empty()
      );
    });
  }

  #[gpui::test]
  async fn cancelling_a_review_comment_hands_focus_back_to_the_diff(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-cancel-focus");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    let editor = page.read_with(cx, |page, _| page.editor.clone().expect("editor"));
    let editor_handle = editor.read_with(cx, |editor, cx| editor.focus_handle(cx));
    // Park the focus off the diff, the way the open composer does.
    let dock_handle = page.read_with(cx, |page, cx| page.dock_panel.read(cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&dock_handle, cx));
    cx.run_until_parked();
    assert_ne!(
      cx.update(|window, cx| window.focused(cx)).as_ref(),
      Some(&editor_handle)
    );

    editor.update_in(cx, |editor, window, cx| {
      assert!(editor.start_review_comment_for_active_hunk(window, cx));
      editor.cancel_review_comment_create_draft(window, cx);
    });
    // The handler restores the focus on the next frame, which tests must deliver.
    let ran = cx.update(|window, cx| window.simulate_next_frame(cx));
    assert!(ran > 0, "the cancel handler schedules the focus restore");
    cx.run_until_parked();

    assert_eq!(
      cx.update(|window, cx| window.focused(cx)).as_ref(),
      Some(&editor_handle),
      "the composer is gone, so the diff takes the focus back"
    );
  }

  /// Two drafts on one file, and the panel that lists them.
  async fn page_with_two_review_comments<'a>(
    name: &str,
    cx: &'a mut TestAppContext,
  ) -> (
    TempRepo,
    Entity<SessionPage>,
    &'a mut gpui::VisualTestContext,
    Entity<crate::review_list::ReviewList>,
    (u64, u64),
  ) {
    let repo = TempRepo::init(name);
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\nv3\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "first"), window, cx);
      page.create_agent_review_comment(create_request(1, "second"), window, cx);
    });

    let (review_list, ids) = page.read_with(cx, |page, cx| {
      let comments = page.agent_review.all();
      assert_eq!(comments.len(), 2);
      (
        page.dock_panel.read(cx).review_list.clone(),
        (comments[0].id, comments[1].id),
      )
    });
    (repo, page, cx, review_list, ids)
  }

  #[gpui::test]
  async fn a_send_only_carries_the_ticked_comments(cx: &mut TestAppContext) {
    let (_repo, page, cx, review_list, (first, second)) =
      page_with_two_review_comments("session-page-review-partial-send", cx).await;

    review_list.update(cx, |list, cx| list.toggle_comment(second, cx));
    page.update_in(cx, |page, window, cx| {
      page.send_agent_review_to_agent(window, cx)
    });

    page.read_with(cx, |page, cx| {
      let export = page.last_review_export.as_deref().expect("an export");
      assert!(export.contains("second"));
      assert!(!export.contains("first"));
      // No agent mounted: nothing was marked, and the tick is still there to
      // try again with.
      assert_eq!(
        page.agent_review.all()[1].state,
        LocalAgentReviewCommentState::Draft
      );
      assert_eq!(page.review_panel_selection(cx), HashSet::from([second]));
      let _ = first;
    });
  }

  #[gpui::test]
  async fn nothing_ticked_sends_the_whole_batch(cx: &mut TestAppContext) {
    let (_repo, page, cx, _review_list, _ids) =
      page_with_two_review_comments("session-page-review-send-all", cx).await;

    page.update_in(cx, |page, window, cx| {
      page.send_agent_review_to_agent(window, cx)
    });

    page.read_with(cx, |page, _| {
      let export = page.last_review_export.as_deref().expect("an export");
      assert!(export.contains("first"));
      assert!(export.contains("second"));
    });
  }

  #[gpui::test]
  async fn a_single_comment_send_ignores_the_ticks(cx: &mut TestAppContext) {
    let (_repo, page, cx, review_list, (first, second)) =
      page_with_two_review_comments("session-page-review-send-one", cx).await;

    // Ticked one, sent the other: the row and the diff card send themselves.
    review_list.update(cx, |list, cx| list.toggle_comment(second, cx));
    review_list.update(cx, |_, cx| {
      cx.emit(ReviewListEvent::SendComment { id: first });
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      let export = page.last_review_export.as_deref().expect("an export");
      assert!(export.contains("first"));
      assert!(!export.contains("second"));
    });
  }

  #[gpui::test]
  async fn saving_a_review_comment_hands_focus_back_to_the_diff(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-save-focus");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    let editor = page.read_with(cx, |page, _| page.editor.clone().expect("editor"));
    let editor_handle = editor.read_with(cx, |editor, cx| editor.focus_handle(cx));
    // Park the focus off the diff, the way the open composer does.
    let dock_handle = page.read_with(cx, |page, cx| page.dock_panel.read(cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&dock_handle, cx));
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "extract helper"), window, cx);
    });
    let ran = cx.update(|window, cx| window.simulate_next_frame(cx));
    assert!(ran > 0, "saving schedules the focus restore");
    cx.run_until_parked();

    assert_eq!(
      cx.update(|window, cx| window.focused(cx)).as_ref(),
      Some(&editor_handle),
      "the shortcuts only answer while something holds the focus"
    );

    // And an edit does the same.
    let comment_id = page.read_with(cx, |page, _| page.agent_review.all()[0].id);
    cx.update(|window, cx| window.focus(&dock_handle, cx));
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.update_agent_review_comment(comment_id, Arc::from("extract it twice"), window, cx);
    });
    cx.update(|window, cx| window.simulate_next_frame(cx));
    cx.run_until_parked();

    assert_eq!(
      cx.update(|window, cx| window.focused(cx)).as_ref(),
      Some(&editor_handle)
    );
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
      page.create_agent_review_comment(create_request(0, "still a draft"), window, cx);
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
