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
    // A panel whose backend connection errored out is rebuilt on the same
    // conversation instead of revived in place.
    let reconnect_resume = match self.agent_chat_view.as_ref() {
      Some(view) if view.read(cx).needs_reconnect() => {
        let id = view.read(cx).current_conversation().id.clone();
        self.agent_chat_view = None;
        Some(self.conversation_meta(&id, cx))
      }
      Some(_) => return,
      None => None,
    };
    prune_agent_chat_state_once();
    if let Some(evicted_repo) = self.ensure_chat_store(cx) {
      self.push_repo_hidden_notification(&evicted_repo, window, cx);
    }
    let resume = match reconnect_resume {
      Some(meta) => meta,
      None => self
        .chat_store
        .as_ref()
        .and_then(|store| store.read(cx).active_meta()),
    };
    let view = self.build_fallback_chat_panel(resume, window, cx);
    view.update(cx, |panel, _| panel.set_active_conversation(true));
    self.agent_chat_view = Some(view);
    self.refresh_session_list(cx);
    self.sync_active_checkout(window, cx);
  }

  /// Points `chat_store` at the FALLBACK repo's store; sessions of other
  /// repos reach their own store through their panel or the hub.
  pub(super) fn ensure_chat_store(&mut self, cx: &mut Context<Self>) -> Option<PathBuf> {
    let repo = self
      .fallback_repo
      .clone()
      .unwrap_or_else(|| PathBuf::from("."));
    let access = self.chat_store_for_repo(&repo, cx)?;
    let evicted_repo = access.evicted_repo.clone();
    self.chat_store = Some(access.store.clone());
    if self.fallback_repo.is_some() && self.swept_repos.insert(repo.clone()) {
      self.sweep_orphan_worktrees(repo, access.store, cx);
    }
    evicted_repo
  }

  /// Boot-time housekeeping for the checkouts we created: a crash between the
  /// worktree and its binding, a failed removal, or a pruned conversation all
  /// leave a `reviu-` worktree nothing references any more. Comet and waku
  /// both leak these forever; we don't.
  fn sweep_orphan_worktrees(
    &mut self,
    repo_root: PathBuf,
    store: Entity<ConversationStore>,
    cx: &mut Context<Self>,
  ) {
    use app_log::ResultExt as _;

    let list_root = repo_root.clone();
    let listing = cx.background_spawn(async move { git::list_worktrees(&list_root) });
    cx.spawn(async move |this, cx| {
      let Some(worktrees) = listing.await.log_err_context("listing worktrees") else {
        return;
      };
      // The doomed set is decided back on the foreground, against the
      // bindings AS OF NOW: a worktree session created while the listing ran
      // has its binding in by the time this continuation runs (the foreground
      // queue is ordered), so it can never be mistaken for an orphan.
      let doomed = this
        .update(cx, |this, cx| {
          this.doomed_worktrees(&repo_root, &store, worktrees, cx)
        })
        .unwrap_or_default();
      if doomed.is_empty() {
        return;
      }
      cx.background_spawn(async move {
        for path in doomed {
          git::remove_worktree(&repo_root, &path).log_err_context("removing an orphaned worktree");
        }
      })
      .await;
    })
    .detach();
  }

  /// Which of `worktrees` nothing references: ours (our directory, still on a
  /// `reviu-` branch; a renamed branch means the user took over) and bound to
  /// no surviving conversation. Bindings whose conversation is gone (pruned,
  /// or a delete that lost its removal) are dropped on the way.
  fn doomed_worktrees(
    &mut self,
    repo_root: &Path,
    store: &Entity<ConversationStore>,
    worktrees: Vec<git::LinkedWorktree>,
    cx: &mut Context<Self>,
  ) -> Vec<PathBuf> {
    use app_log::ResultExt as _;

    let store = store.clone();
    let canonical =
      |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // A conversation proves it is alive through the index, or through a live
    // panel: a just-created worktree session is still blank, so it has no
    // index row yet, only its panel.
    let mut known_ids: std::collections::HashSet<String> = store
      .read(cx)
      .list()
      .into_iter()
      .map(|meta| meta.id)
      .collect();
    known_ids.extend(
      self
        .agent_chat_view
        .iter()
        .map(|panel| panel.read(cx).current_conversation().id.clone()),
    );
    known_ids.extend(self.background_chat_panels.iter().map(|(id, _)| id.clone()));
    let mut live_paths = std::collections::HashSet::new();
    for (conversation_id, binding) in store.read(cx).worktree_bindings() {
      if known_ids.contains(&conversation_id) {
        live_paths.insert(canonical(&binding.path));
      } else {
        store.update(cx, |store, cx| {
          store.set_worktree(&conversation_id, None, cx)
        });
      }
    }
    let Some(sweep_root) =
      git::worktrees_root_for(repo_root).log_err_context("resolving the worktrees root")
    else {
      return Vec::new();
    };
    let sweep_root = canonical(&sweep_root);
    worktrees
      .into_iter()
      .filter(|worktree| {
        let path = canonical(&worktree.path);
        path.starts_with(&sweep_root)
          && !live_paths.contains(&path)
          && worktree
            .branch
            .as_deref()
            .is_some_and(|branch| branch.starts_with(git::WORKTREE_BRANCH_PREFIX))
      })
      .map(|worktree| worktree.path)
      .collect()
  }

  fn conversation_meta(&self, id: &str, cx: &App) -> Option<agent_chat_panel::ConversationMeta> {
    self
      .conversation_hub
      .find_conversation(id, cx)
      .map(|(_, _, meta)| meta)
  }

  /// The checkout a conversation's agent works in: its bound worktree when it
  /// has one and the directory still exists, the main checkout otherwise. A
  /// stale binding (worktree deleted outside Reviu) is dropped on the way.
  fn session_cwd_for(
    &mut self,
    repo_root: &Path,
    store: Option<&Entity<ConversationStore>>,
    conversation_id: Option<&str>,
    cx: &mut Context<Self>,
  ) -> PathBuf {
    let main = repo_root.to_path_buf();
    let Some((conversation_id, store)) = conversation_id.zip(store.cloned()) else {
      return main;
    };
    let Some(binding) = store.read(cx).worktree(conversation_id) else {
      return main;
    };
    if binding.path.is_dir() {
      binding.path
    } else {
      store.update(cx, |store, cx| {
        store.set_worktree(conversation_id, None, cx)
      });
      main
    }
  }

  /// A panel for a conversation of the FALLBACK repo, when nothing on screen
  /// names one.
  pub(super) fn build_fallback_chat_panel(
    &mut self,
    resume: Option<agent_chat_panel::ConversationMeta>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<AgentChatPanel> {
    let repo = self
      .fallback_repo
      .clone()
      .unwrap_or_else(|| PathBuf::from("."));
    let store = self.chat_store.clone();
    self.build_chat_panel(repo, store, resume, window, cx)
  }

  /// One panel per conversation: process, transcript and composer live and die
  /// with it. The shell only decides which one is on screen.
  fn build_chat_panel(
    &mut self,
    repo_root: PathBuf,
    store: Option<Entity<ConversationStore>>,
    resume: Option<agent_chat_panel::ConversationMeta>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<AgentChatPanel> {
    let cwd = self.session_cwd_for(
      &repo_root,
      store.as_ref(),
      resume.as_ref().map(|meta| meta.id.as_str()),
      cx,
    );
    self.build_chat_panel_at(repo_root, cwd, store, resume, window, cx)
  }

  #[allow(clippy::too_many_arguments)]
  fn build_chat_panel_at(
    &mut self,
    repo_root: PathBuf,
    cwd: PathBuf,
    store: Option<Entity<ConversationStore>>,
    resume: Option<agent_chat_panel::ConversationMeta>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<AgentChatPanel> {
    let backend = resume
      .as_ref()
      .and_then(|meta| agent_chat_panel::resolve_agent(&agent_registry::global(), &meta.agent_id))
      .unwrap_or_else(AgentSettings::load);
    let turn_gate = self.turn_gate.clone();
    let view = cx.new(|cx| {
      AgentChatPanel::new(
        backend, repo_root, cwd, store, resume, turn_gate, window, cx,
      )
    });
    let close_control_visible = self.center == CenterView::Diff && self.diff_chat_open;
    view.update(cx, |panel, cx| {
      panel.set_close_control_visible(close_control_visible, cx);
    });
    // Sidebar reads conversation state from the panel. The panel already
    // notifies itself, so avoid repainting the whole page on stream chunks.
    // Also the flush point for a review export queued while the agent was connecting.
    cx.observe(&view, |this, _, cx| {
      this.flush_pending_review_export(cx);
      this.sync_session_list(cx);
    })
    .detach();
    cx.subscribe_in(
      &view,
      window,
      |this, panel, event: &AgentChatPanelEvent, window, cx| match event {
        AgentChatPanelEvent::OpenPath { path, line } => {
          let checkout = panel.read(cx).cwd().to_path_buf();
          let rel_path = agent_path_to_repo_relative(path.clone(), Some(checkout.as_path()));
          this.open_diff(rel_path, *line, OpenIntent::Open, window, cx);
        }
        AgentChatPanelEvent::OpenDiffSnapshot {
          path,
          old_text,
          new_text,
          line,
        } => {
          let checkout = panel.read(cx).cwd().to_path_buf();
          let rel_path = agent_path_to_repo_relative(path.clone(), Some(checkout.as_path()));
          this.open_agent_diff_snapshot(
            rel_path,
            old_text.clone(),
            new_text.clone(),
            *line,
            OpenIntent::Open,
            window,
            cx,
          );
        }
        AgentChatPanelEvent::TurnStarted => {
          this.create_turn_checkpoint(panel.clone(), cx);
        }
        AgentChatPanelEvent::PermissionRequested => {
          this.notify_agent_attention("Reviu agent needs a decision", Some(panel), window, cx);
        }
        AgentChatPanelEvent::TurnFinished { completed } => {
          // A queued prompt draining into a fresh turn is not a stopping point.
          if !panel.read(cx).is_turn_in_flight() {
            this.notify_agent_attention("Reviu agent finished", Some(panel), window, cx);
          }
          this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
          if let Some(editor) = this.editor.clone() {
            editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
          }
          this.consume_sent_review_comments(*completed);
          this.sync_agent_review_comments_to_editor(cx);
          this.refresh_branch(cx);
          // A background session's row (timestamp, preview) moved too.
          this.refresh_session_list(cx);
        }
        AgentChatPanelEvent::RollbackRequested { ref_name } => {
          this.rollback_to_checkpoint(ref_name.clone(), window, cx);
        }
        AgentChatPanelEvent::UndoTurnRequested { ref_name } => {
          this.undo_turn_files(ref_name.clone(), window, cx);
        }
        AgentChatPanelEvent::TitleSettled { title } => {
          this.rename_session_worktree_branch(panel.clone(), title.clone(), cx);
        }
        AgentChatPanelEvent::TurnFailed { .. } => {
          // No toast: the red card in the transcript and the Failed row in
          // the sidebar already carry it. The popup covers an inactive window.
          this.notify_agent_attention("Reviu agent failed", Some(panel), window, cx);
          this.refresh_session_list(cx);
        }
        AgentChatPanelEvent::CloseRequested => {
          this.hide_diff_chat(window, cx);
        }
      },
    )
    .detach();
    view
  }

  /// `reviu-swift-otter` becomes `reviu-fix-the-scroll` once the conversation
  /// has a title. Best-effort polish the user never asked for: a refusal
  /// (branch moved, name taken) keeps the generated name, silently.
  fn rename_session_worktree_branch(
    &mut self,
    panel: Entity<AgentChatPanel>,
    title: String,
    cx: &mut Context<Self>,
  ) {
    use app_log::ResultExt as _;

    let repo_root = panel.read(cx).repo_root().to_path_buf();
    let Some(store) = panel.read(cx).store() else {
      return;
    };
    let conversation_id = panel.read(cx).current_conversation().id.clone();
    let Some(binding) = store.read(cx).worktree(&conversation_id) else {
      return;
    };
    cx.spawn(async move |this, cx| {
      let rename_binding = binding.clone();
      let renamed = cx
        .background_spawn(async move {
          git::rename_worktree_branch(
            &repo_root,
            &rename_binding.path,
            &rename_binding.branch,
            &title,
          )
        })
        .await;
      let Some(branch) = renamed.log_err_context("renaming the worktree branch") else {
        return;
      };
      if branch == binding.branch {
        return;
      }
      let _ = this.update(cx, |this, cx| {
        // The session may have been deleted while git ran: never resurrect
        // its binding.
        let still_bound = store
          .read(cx)
          .worktree(&conversation_id)
          .is_some_and(|current| current.path == binding.path);
        if !still_bound {
          return;
        }
        store.update(cx, |store, cx| {
          store.set_worktree(
            &conversation_id,
            Some(agent_chat_panel::WorktreeBinding {
              path: binding.path.clone(),
              branch: branch.clone(),
            }),
            cx,
          )
        });
        this.refresh_session_list(cx);
        this.refresh_branch(cx);
      });
    })
    .detach();
  }

  /// Moves the shown panel to the background without stopping its agent.
  fn park_active_chat_panel(&mut self, cx: &mut Context<Self>) {
    let Some(panel) = self.agent_chat_view.take() else {
      return;
    };
    // A blank idle conversation has no row to come back through; drop it,
    // along with the worktree it never used.
    let keep = {
      let panel = panel.read(cx);
      panel.has_persistable_content() || !panel.is_parked()
    };
    if !keep {
      let id = panel.read(cx).current_conversation().id.clone();
      let repo_root = panel.read(cx).repo_root().to_path_buf();
      let store = panel.read(cx).store();
      drop(panel);
      if let Some(store) = store {
        self.cleanup_session_worktree(repo_root, store, &id, cx);
      }
      return;
    }
    let id = panel.read(cx).current_conversation().id.clone();
    panel.update(cx, |panel, cx| {
      panel.set_active_conversation(false);
      panel.persist_now(cx);
    });
    self.background_chat_panels.insert(0, (id, panel));
  }

  /// Keeps the most recent parked panels alive; the rest are dropped, which
  /// stops their agent process. Running sessions are never evicted.
  fn evict_parked_chat_panels(&mut self, cx: &mut Context<Self>) {
    const MAX_PARKED_CHAT_PANELS: usize = 5;
    let mut kept = 0;
    let mut index = 0;
    while index < self.background_chat_panels.len() {
      if !self.background_chat_panels[index].1.read(cx).is_parked() {
        index += 1;
        continue;
      }
      kept += 1;
      if kept > MAX_PARKED_CHAT_PANELS {
        let (_, panel) = self.background_chat_panels.remove(index);
        panel.update(cx, |panel, cx| panel.persist_now(cx));
      } else {
        index += 1;
      }
    }
  }

  /// Popup on the primary display when the agent needs eyes and the main
  /// window is inactive; clicking it brings the app back. `panel` names the
  /// session that asked, background ones included.
  pub(super) fn notify_agent_attention(
    &mut self,
    title: &str,
    panel: Option<&Entity<AgentChatPanel>>,
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
    let panel = panel.or(self.agent_chat_view.as_ref());
    let caption = panel
      .map(|panel| panel.read(cx).current_conversation().title.clone())
      .filter(|title| !title.is_empty())
      .unwrap_or_else(|| "Agent session".to_string());
    let main_window = self.window_handle;
    let title = title.to_string();
    let agent_id = panel
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

  pub(super) fn create_turn_checkpoint(
    &mut self,
    panel: Entity<AgentChatPanel>,
    cx: &mut Context<Self>,
  ) {
    if self.fallback_repo.is_none() {
      return;
    }
    // The snapshot covers the checkout the agent actually edits: the
    // session's worktree, or the main one. The refs land in the shared .git.
    let repo_root = panel.read(cx).cwd().to_path_buf();
    let session_id = panel.read(cx).current_conversation().id.clone();

    // The marker must land on the session that started the turn, shown or not.
    let panel = panel.downgrade();
    cx.spawn(async move |_this, cx| {
      let result = cx
        .background_spawn(async move { git::create_checkpoint(&repo_root, &session_id) })
        .await;
      let Ok(checkpoint) = result else {
        return;
      };
      let _ = panel.update(cx, |panel, cx| {
        panel.record_checkpoint(checkpoint.ref_name, cx);
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
    if self.fallback_repo.is_none() {
      return;
    }
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
    // Restore rewinds the checkout the turn ran in: the session's worktree
    // for a worktree session, the main checkout otherwise.
    let repo_root = panel.read(cx).cwd().to_path_buf();
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
    if self.fallback_repo.is_none() {
      return;
    }
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
    // Undo reverts files in the checkout the turn ran in.
    let repo_root = panel.read(cx).cwd().to_path_buf();
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
    let create: ReviewCommentCreateHandler = Arc::new({
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

    // The composer is gone on the next frame; the page resolves focus to the diff.
    let cancel: ReviewCommentCancelHandler = Arc::new({
      let view = view.clone();
      move |window, cx| {
        let _ = view.update(cx, |this, cx| this.focus_page_on_next_frame(window, cx));
      }
    });

    let edit: ReviewCommentEditHandler = Arc::new({
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

    let delete: ReviewCommentDeleteHandler = Arc::new({
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

    let send: ReviewCommentSendHandler = Arc::new({
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

    configure_review(
      editor,
      ReviewDestination::Agent(Box::new(AgentReviewHandlers {
        create,
        edit,
        delete,
        cancel,
        send,
      })),
      cx,
    );
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
    let editor = self
      .opened_snapshot
      .is_none()
      .then(|| self.editor.clone())
      .flatten();
    sync_comments_to_editor(
      &self.agent_review,
      editor.as_ref(),
      self.selected_file.as_deref(),
      cx,
    );
    self.sync_review_panel(cx);
    self.persist_agent_review();
  }

  /// A completed turn consumes the comments it carried: they were instructions,
  /// and the work is over. One the user stopped, or one that failed, did nothing
  /// with them, so they stay and can go again.
  pub(super) fn consume_sent_review_comments(&mut self, completed: bool) -> usize {
    if !completed {
      return 0;
    }
    self.agent_review.clear_sent()
  }

  /// Reads the batch of the shown session's repository and hands it to the
  /// panel. Nothing else fills the panel until a file is opened.
  pub(super) fn reload_review_for_repo(&mut self, cx: &mut Context<Self>) {
    self.review_store_path = review_store_path_for(
      self.session_repo(cx).as_deref(),
      self.review_state_dir.as_deref(),
    );
    self.agent_review = load_agent_review(self.review_store_path.as_deref());
    self.sync_review_panel(cx);
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
      panel.review_list.update(cx, |list, cx| {
        list.set_comments(ReviewSection::Agent, comments, cx)
      });
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
    let count = self.draft_review_comment_count();
    if count == 0 {
      return;
    }
    let title: SharedString = "Discard this review?".into();
    let message: SharedString = if count == 1 {
      "Delete the comment you have not sent yet?".into()
    } else {
      format!("Delete the {count} comments you have not sent yet?").into()
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
    if self.agent_review.clear_drafts() == 0 {
      return;
    }
    self.sync_agent_review_comments_to_editor(cx);
    cx.notify();
  }

  pub(super) fn draft_review_comment_count(&self) -> usize {
    self.agent_review.draft_count()
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

    self.agent_review.mark_as_sent(&send);
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

  /// Creation lands where you are: the shown session's repo, else the fallback.
  fn creation_repo(&self, cx: &App) -> PathBuf {
    self.session_repo(cx).unwrap_or_else(|| PathBuf::from("."))
  }

  pub(super) fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let repo_root = self.creation_repo(cx);
    self.new_session_in(repo_root, window, cx);
  }

  pub(super) fn new_agent_session_action(
    &mut self,
    _: &crate::NewAgentSession,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.session_repo(cx).is_none() {
      window.push_notification(
        Notification::warning("Open a repository before starting a session."),
        cx,
      );
      return;
    }
    self.new_session(window, cx);
  }

  pub(super) fn new_agent_worktree_session_action(
    &mut self,
    _: &crate::NewAgentWorktreeSession,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.session_repo(cx) else {
      window.push_notification(
        Notification::warning("Open a repository before starting a worktree session."),
        cx,
      );
      return;
    };
    self.new_worktree_session_in(repo_root, None, window, cx);
  }

  pub(super) fn new_session_in(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.editor_is_dirty(cx) && self.target_checkout_differs_from_editor(&repo_root, cx) {
      self.open_unsaved_editor_dialog(UnsavedEditorAction::NewSessionIn { repo_root }, window, cx);
      return;
    }
    self.new_session_in_without_unsaved_prompt(repo_root, window, cx);
  }

  pub(super) fn new_session_in_without_unsaved_prompt(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    prune_agent_chat_state_once();
    if let Some(evicted_repo) = self.ensure_chat_store(cx) {
      self.push_repo_hidden_notification(&evicted_repo, window, cx);
    }
    let access = self.chat_store_for_repo(&repo_root, cx);
    if let Some(evicted_repo) = access
      .as_ref()
      .and_then(|access| access.evicted_repo.as_ref())
    {
      self.push_repo_hidden_notification(evicted_repo, window, cx);
    }
    let store = access.map(|access| access.store);
    if let Some(panel) = self.agent_chat_view.as_ref() {
      // The shown conversation is still blank: it already is the new session.
      // Not while hydrating (its transcript may be about to land) and not when
      // its connection died (a fresh panel is the revival).
      let panel = panel.read(cx);
      if !panel.has_persistable_content()
        && panel.loading_conversation_id().is_none()
        && !panel.needs_reconnect()
        && panel.repo_root() == repo_root.as_path()
      {
        self.reveal_active_session_chat(window, cx);
        return;
      }
    }
    self.park_active_chat_panel(cx);
    let view = self.build_chat_panel(repo_root, store, None, window, cx);
    view.update(cx, |panel, _| panel.set_active_conversation(true));
    self.agent_chat_view = Some(view);
    self.evict_parked_chat_panels(cx);
    self.sync_agent_chat_close_control(cx);
    self.refresh_session_list(cx);
    self.sync_active_checkout(window, cx);
    self.reveal_active_session_chat(window, cx);
    cx.notify();
  }

  /// A session in its own worktree that never went anywhere: no reason to
  /// keep the checkout. Unbinds first so a failure leaves no dangling pointer.
  fn cleanup_session_worktree(
    &mut self,
    repo_root: PathBuf,
    store: Entity<ConversationStore>,
    conversation_id: &str,
    cx: &mut Context<Self>,
  ) {
    let Some(binding) = store.read(cx).worktree(conversation_id) else {
      return;
    };
    store.update(cx, |store, cx| {
      store.set_worktree(conversation_id, None, cx)
    });
    let window_handle = self.window_handle;
    cx.spawn(async move |_this, cx| {
      let removed = cx
        .background_spawn(async move { git::remove_worktree(&repo_root, &binding.path) })
        .await;
      if let Err(error) = removed {
        let _ = cx.update_window(window_handle, |_, window, cx| {
          window.push_notification(
            Notification::error(format!("Removing the session worktree failed: {error}")),
            cx,
          );
        });
      }
    })
    .detach();
  }

  /// Checkpoint refs pin up to 50 snapshots per conversation in the object
  /// database; a deleted conversation releases them.
  fn cleanup_session_checkpoints(
    &mut self,
    repo_root: PathBuf,
    conversation_id: &str,
    cx: &mut Context<Self>,
  ) {
    let conversation_id = conversation_id.to_string();
    cx.background_spawn(async move {
      // Best-effort: stale refs cost disk, not correctness.
      let _ = git::delete_session_checkpoints(&repo_root, &conversation_id);
    })
    .detach();
  }

  /// The section buttons name their repo explicitly: a worktree session is
  /// always created in a repo the user pointed at.
  pub(super) fn new_worktree_session_in(
    &mut self,
    repo_root: PathBuf,
    base: Option<String>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.editor_is_dirty(cx) {
      self.open_unsaved_editor_dialog(
        UnsavedEditorAction::NewWorktreeSessionIn { repo_root, base },
        window,
        cx,
      );
      return;
    }
    self.new_worktree_session_in_without_unsaved_prompt(repo_root, base, window, cx);
  }

  pub(super) fn new_worktree_session_in_without_unsaved_prompt(
    &mut self,
    repo_root: PathBuf,
    base: Option<String>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    prune_agent_chat_state_once();
    if let Some(evicted_repo) = self.ensure_chat_store(cx) {
      self.push_repo_hidden_notification(&evicted_repo, window, cx);
    }
    let access = self.chat_store_for_repo(&repo_root, cx);
    if let Some(evicted_repo) = access
      .as_ref()
      .and_then(|access| access.evicted_repo.as_ref())
    {
      self.push_repo_hidden_notification(evicted_repo, window, cx);
    }
    let target_store = access.map(|access| access.store);
    cx.spawn_in(window, async move |this, cx| {
      let create_root = repo_root.clone();
      let created = cx
        .background_spawn(async move { git::create_worktree(&create_root, base.as_deref()) })
        .await;
      let _ = this.update_in(cx, |this, window, cx| {
        match created {
          Ok(worktree) => {
            this.park_active_chat_panel(cx);
            let store = target_store.clone();
            let view = this.build_chat_panel_at(
              repo_root.clone(),
              worktree.path.clone(),
              store,
              None,
              window,
              cx,
            );
            view.update(cx, |panel, _| panel.set_active_conversation(true));
            let conversation_id = view.read(cx).current_conversation().id.clone();
            this.agent_chat_view = Some(view);
            if let Some(store) = target_store.clone() {
              store.update(cx, |store, cx| {
                store.set_worktree(
                  &conversation_id,
                  Some(agent_chat_panel::WorktreeBinding {
                    path: worktree.path,
                    branch: worktree.branch,
                  }),
                  cx,
                )
              });
            }
            this.evict_parked_chat_panels(cx);
            this.sync_agent_chat_close_control(cx);
            this.refresh_session_list(cx);
            this.sync_active_checkout(window, cx);
            this.reveal_active_session_chat(window, cx);
          }
          Err(error) => {
            window.push_notification(
              Notification::error(format!("Creating the worktree failed: {error}")),
              cx,
            );
          }
        }
        cx.notify();
      });
    })
    .detach();
  }

  pub(super) fn delete_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    // The session's own repo and store, which may not be the fallback's: a row
    // of another repo can be deleted straight from the aggregated sidebar.
    let found = self.conversation_hub.find_conversation(id, cx);
    let from_panel = || {
      self
        .agent_chat_view
        .iter()
        .chain(self.background_chat_panels.iter().map(|(_, panel)| panel))
        .find(|panel| panel.read(cx).current_conversation().id == id)
        .and_then(|panel| {
          let panel = panel.read(cx);
          panel
            .store()
            .map(|store| (panel.repo_root().to_path_buf(), store))
        })
    };
    let Some((repo_root, store)) = found
      .map(|(repo, store, _)| (repo, store))
      .or_else(from_panel)
    else {
      return;
    };
    let deleted_repo = repo_root.clone();
    // Bindings are read before the delete scrubs them.
    self.cleanup_session_worktree(repo_root.clone(), store.clone(), id, cx);
    self.cleanup_session_checkpoints(repo_root, id, cx);
    store.update(cx, |store, cx| store.delete(id, cx));
    // Dropping the panel stops its agent process.
    self
      .background_chat_panels
      .retain(|(panel_id, _)| panel_id != id);
    let deleting_active = self
      .agent_chat_view
      .as_ref()
      .is_some_and(|panel| panel.read(cx).current_conversation().id == id);
    if deleting_active {
      store.update(cx, |store, cx| store.set_active(None, cx));
      self.agent_chat_view = None;
      // You were working in that repo: the fresh session stays there.
      let access = self.chat_store_for_repo(&deleted_repo, cx);
      if let Some(evicted_repo) = access
        .as_ref()
        .and_then(|access| access.evicted_repo.as_ref())
      {
        self.push_repo_hidden_notification(evicted_repo, window, cx);
      }
      let repo = access.map(|access| (deleted_repo.clone(), Some(access.store)));
      let view = match repo {
        Some((repo_root, store)) => self.build_chat_panel(repo_root, store, None, window, cx),
        None => self.build_fallback_chat_panel(None, window, cx),
      };
      view.update(cx, |panel, _| panel.set_active_conversation(true));
      self.agent_chat_view = Some(view);
    }
    self.refresh_session_list(cx);
    self.sync_active_checkout(window, cx);
    cx.notify();
  }

  fn reveal_active_session_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.center == CenterView::Diff && !self.diff_chat_open {
      self.diff_chat_open = true;
      self.sync_agent_chat_close_control(cx);
      cx.notify();
    }
    self.focus_agent_input_on_next_frame(window, cx);
  }

  fn session_checkout_for_id(&self, id: &str, cx: &App) -> Option<PathBuf> {
    self
      .agent_chat_view
      .iter()
      .chain(self.background_chat_panels.iter().map(|(_, panel)| panel))
      .find_map(|panel| {
        let panel = panel.read(cx);
        (panel.current_conversation().id == id).then(|| panel.cwd().to_path_buf())
      })
      .or_else(|| {
        let (repo_root, store, _) = self.conversation_hub.find_conversation(id, cx)?;
        store
          .read(cx)
          .worktree(id)
          .map(|binding| binding.path)
          .or(Some(repo_root))
      })
  }

  pub(super) fn select_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    let target_checkout = self.session_checkout_for_id(id, cx);
    if self.editor_is_dirty(cx)
      && target_checkout
        .as_deref()
        .is_none_or(|target| self.target_checkout_differs_from_editor(target, cx))
    {
      self.open_unsaved_editor_dialog(
        UnsavedEditorAction::SelectSession { id: id.to_string() },
        window,
        cx,
      );
      return;
    }
    self.select_session_without_unsaved_prompt(id, window, cx);
  }

  pub(super) fn select_session_without_unsaved_prompt(
    &mut self,
    id: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    prune_agent_chat_state_once();
    if let Some(evicted_repo) = self.ensure_chat_store(cx) {
      self.push_repo_hidden_notification(&evicted_repo, window, cx);
    }
    if let Some(active) = self.agent_chat_view.as_ref()
      && active.read(cx).current_conversation().id == id
    {
      self.reveal_active_session_chat(window, cx);
      return;
    }
    self.park_active_chat_panel(cx);
    let panel = match self
      .background_chat_panels
      .iter()
      .position(|(panel_id, _)| panel_id == id)
    {
      // The session was live in the background: back on screen as it is,
      // reviving its connection if it died while parked.
      Some(position) => {
        let panel = self.background_chat_panels.remove(position).1;
        if panel.read(cx).needs_reconnect() {
          panel.update(cx, |panel, cx| panel.reconnect(cx));
        }
        panel
      }
      None => match self.conversation_hub.find_conversation(id, cx) {
        Some((repo_root, store, meta)) => {
          self.build_chat_panel(repo_root, Some(store), Some(meta), window, cx)
        }
        // Unknown id (stale click): a fresh fallback session is the least wrong.
        None => self.build_fallback_chat_panel(None, window, cx),
      },
    };
    panel.update(cx, |panel, _| panel.set_active_conversation(true));
    if let Some(store) = panel.read(cx).store() {
      store.update(cx, |store, cx| store.set_active(Some(id.to_string()), cx));
    }
    self.agent_chat_view = Some(panel);
    self.evict_parked_chat_panels(cx);
    self.sync_agent_chat_close_control(cx);
    self.refresh_session_list(cx);
    self.sync_active_checkout(window, cx);
    self.reveal_active_session_chat(window, cx);
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

  /// A turn running in THIS checkout; turns isolated in other worktrees
  /// don't block work here.
  pub(super) fn agent_turn_in_flight_at(&self, checkout: &Path, cx: &App) -> bool {
    #[cfg(test)]
    if self.pretend_agent_turn_in_flight {
      return true;
    }
    let busy_here = |panel: &Entity<AgentChatPanel>| {
      let panel = panel.read(cx);
      panel.is_turn_in_flight() && panel.cwd() == checkout
    };
    self.agent_chat_view.iter().any(&busy_here)
      || self
        .background_chat_panels
        .iter()
        .any(|(_, panel)| busy_here(panel))
  }

  /// A turn in any session OF THIS REPO, whatever checkout it works in.
  pub(super) fn agent_turn_in_flight_for_repo(&self, repo_root: &Path, cx: &App) -> bool {
    #[cfg(test)]
    if self.pretend_agent_turn_in_flight {
      return true;
    }
    let busy = |panel: &Entity<AgentChatPanel>| {
      let panel = panel.read(cx);
      panel.is_turn_in_flight() && panel.repo_root() == repo_root
    };
    self.agent_chat_view.iter().any(&busy)
      || self
        .background_chat_panels
        .iter()
        .any(|(_, panel)| busy(panel))
  }

  /// A turn in ANY session, whatever repo it belongs to.
  #[cfg(test)]
  pub(super) fn agent_turn_in_flight(&self, cx: &App) -> bool {
    #[cfg(test)]
    if self.pretend_agent_turn_in_flight {
      return true;
    }
    self
      .agent_chat_view
      .as_ref()
      .is_some_and(|panel| panel.read(cx).is_turn_in_flight())
      || self
        .background_chat_panels
        .iter()
        .any(|(_, panel)| panel.read(cx).is_turn_in_flight())
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::*;
  use crate::agent_review::LocalAgentReviewCommentState;
  use crate::review_list::{ReviewListEvent, ReviewSection};
  use crate::test_support::{TempRepo, commit_text_file};
  use editor::ReviewCommentDisplayMode;
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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
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
      assert_eq!(page.draft_review_comment_count(), 1);
      comment.id
    });

    page.update(cx, |page, cx| {
      page.delete_agent_review_comment(comment_id, cx);
    });
    page.read_with(cx, |page, _| {
      assert!(page.agent_review.is_empty());
      assert_eq!(page.draft_review_comment_count(), 0);
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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    for body in [
      "short",
      "a comment long enough to wrap over several lines of the card,        with words of every width: iiii MMMM 0123456789 and a bit more prose",
      "first line\nsecond line\nthird line",
    ] {
      page.update_in(cx, |page, window, cx| {
        page.agent_review.clear_drafts();
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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "extract helper"), window, cx);
    });

    page.read_with(cx, |page, cx| {
      let list = page.dock_panel.read(cx).review_list.read(cx);
      assert_eq!(list.comments(ReviewSection::Agent).len(), 1);
      assert_eq!(
        list.comments(ReviewSection::Agent)[0].excerpt,
        "extract helper"
      );
      assert_eq!(
        list.comments(ReviewSection::Agent)[0].path,
        PathBuf::from("README.md")
      );
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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
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
        section: ReviewSection::Agent,
        path: PathBuf::from("README.md"),
        line: 0,
        intent: OpenIntent::Open,
      });
    });
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_file, Some(PathBuf::from("README.md")));
    });

    // And its delete button takes the comment out of the batch.
    review_list.update(cx, |_, cx| {
      cx.emit(ReviewListEvent::DeleteComment {
        section: ReviewSection::Agent,
        id: comment_id,
      });
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
          .comments(ReviewSection::Agent)
          .is_empty()
      );
    });
  }

  /// A comment the agent already has is part of the conversation: discarding
  /// takes back the drafts, and nothing else.
  #[gpui::test]
  async fn discarding_a_review_keeps_what_already_went_out(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-discard-sent");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    let sent = page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "sent"), window, cx);
      let sent = page
        .agent_review
        .all()
        .first()
        .expect("the comment was created")
        .id;
      page
        .agent_review
        .mark_as_sent(&ReviewSend::Only(HashSet::from([sent])));
      page.create_agent_review_comment(create_request(0, "draft"), window, cx);
      sent
    });

    page.update(cx, |page, cx| page.discard_agent_review(cx));

    page.read_with(cx, |page, _| {
      let ids = page
        .agent_review
        .all()
        .iter()
        .map(|comment| comment.id)
        .collect::<Vec<_>>();
      assert_eq!(ids, vec![sent]);
      assert_eq!(page.draft_review_comment_count(), 0);
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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "first"), window, cx);
      page.create_agent_review_comment(create_request(0, "second"), window, cx);
    });

    page.update(cx, |page, cx| page.discard_agent_review(cx));

    page.read_with(cx, |page, cx| {
      assert!(page.agent_review.all().is_empty());
      assert_eq!(page.draft_review_comment_count(), 0);
      assert!(
        page
          .dock_panel
          .read(cx)
          .review_list
          .read(cx)
          .comments(ReviewSection::Agent)
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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
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
  async fn a_completed_turn_takes_the_sent_comments_away(cx: &mut TestAppContext) {
    let (_repo, page, cx, _review_list, (first, _second)) =
      page_with_two_review_comments("session-page-review-turn", cx).await;

    // A review of the pull request is waiting in the other section the whole
    // time: an agent turn has no business touching it.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_pull_request_review_comments_for_test(
          vec![
            crate::pull_request_review_comments::pending_comment_fixture(
              9,
              "src/a.rs",
              Some(3),
              "waiting for GitHub",
            ),
          ],
          cx,
        );
      });
    });

    // What a successful send leaves behind: one gone, one still a draft.
    page.update(cx, |page, cx| {
      page.agent_review.mark_as_sent(&ReviewSend::one(first));
      page.sync_agent_review_comments_to_editor(cx);
    });

    // A turn the user stopped did nothing with it: nothing is dropped.
    page.update(cx, |page, cx| {
      assert_eq!(page.consume_sent_review_comments(false), 0);
      page.sync_agent_review_comments_to_editor(cx);
    });
    page.read_with(cx, |page, _| assert_eq!(page.agent_review.all().len(), 2));

    page.update(cx, |page, cx| {
      assert_eq!(page.consume_sent_review_comments(true), 1);
      page.sync_agent_review_comments_to_editor(cx);
    });
    page.read_with(cx, |page, cx| {
      let comments = page.agent_review.all();
      assert_eq!(comments.len(), 1);
      assert_eq!(comments[0].body.as_ref(), "second");
      assert_eq!(page.draft_review_comment_count(), 1);
      // The panel follows, without waiting for a file to be opened.
      let rows = page
        .dock_panel
        .read(cx)
        .review_list
        .read(cx)
        .comments(ReviewSection::Agent);
      assert_eq!(rows.len(), 1);
      assert_eq!(rows[0].excerpt, "second");

      // The pull request section is untouched: its comments live on GitHub and
      // leave only when the review is submitted.
      let pull_request_rows = page
        .dock_panel
        .read(cx)
        .review_list
        .read(cx)
        .comments(ReviewSection::PullRequest);
      assert_eq!(pull_request_rows.len(), 1);
      assert_eq!(pull_request_rows[0].excerpt, "waiting for GitHub");
    });
  }

  /// A page with the agent panel mounted against a nonexistent binary: the
  /// full multi-panel plumbing runs, no process ever spawns. The override is
  /// process-wide and never cleared: tests run in parallel, and clearing it
  /// mid-run would let another test spawn a real agent.
  async fn page_with_agent_panel<'a>(
    name: &str,
    cx: &'a mut TestAppContext,
  ) -> (
    TempRepo,
    Entity<SessionPage>,
    &'a mut gpui::VisualTestContext,
  ) {
    agent_chat_panel::set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
    let repo = TempRepo::init(name);
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let state_dir = agent_chat_state_dir()
      .map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &repo.path))
      .expect("agent chat state dir");
    let _ = std::fs::remove_dir_all(&state_dir);
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| page.activate(window, cx));
    cx.run_until_parked();
    (repo, page, cx)
  }

  fn active_panel(page: &Entity<SessionPage>, cx: &TestAppContext) -> Entity<AgentChatPanel> {
    page.read_with(cx, |page, _| {
      page.agent_chat_view.clone().expect("active panel")
    })
  }

  #[gpui::test]
  async fn stream_chunks_do_not_notify_the_whole_page(cx: &mut TestAppContext) {
    let (_repo, page, cx) = page_with_agent_panel("session-page-stream-no-page-notify", cx).await;
    let panel = active_panel(&page, cx);
    let page_notifies = std::rc::Rc::new(std::cell::Cell::new(0_usize));
    cx.update(|_, cx| {
      let page_notifies = page_notifies.clone();
      cx.observe(&page, move |_, _| {
        page_notifies.set(page_notifies.get() + 1);
      })
      .detach();
    });

    panel.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("stream", cx);
    });
    cx.run_until_parked();

    assert_eq!(page_notifies.get(), 0);
  }

  #[gpui::test]
  async fn switching_agent_updates_an_existing_blank_sidebar_row(cx: &mut TestAppContext) {
    let (_repo, page, cx) = page_with_agent_panel("session-page-blank-agent-icon", cx).await;
    let panel = active_panel(&page, cx);
    let current_id = panel.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update(cx, |page, cx| {
      let row = {
        let panel = page
          .agent_chat_view
          .as_ref()
          .expect("active panel")
          .read(cx);
        crate::session_list::SessionRow {
          repo_root: panel.repo_root().to_path_buf(),
          meta: panel.current_conversation().clone(),
        }
      };
      page.session_list.update(cx, |list, cx| {
        list.set_conversations(vec![row], current_id.clone(), cx);
      });
    });

    panel.update(cx, |panel, cx| {
      panel.switch_backend(agent_registry::AgentId::new("pi-acp"), cx)
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .session_list
          .read(cx)
          .agent_id_of(&current_id)
          .as_deref(),
        Some("pi-acp")
      );
    });
  }

  #[gpui::test]
  async fn switching_sessions_parks_the_panel_and_brings_it_back(cx: &mut TestAppContext) {
    let (_repo, page, cx) = page_with_agent_panel("session-page-park-revive", cx).await;

    let first = active_panel(&page, cx);
    first.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("first", cx)
    });
    cx.run_until_parked();
    let first_id = first.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update_in(cx, |page, window, cx| page.new_session(window, cx));
    cx.run_until_parked();
    let second = active_panel(&page, cx);
    assert_ne!(
      second.entity_id(),
      first.entity_id(),
      "a fresh panel took over"
    );
    second.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("second", cx)
    });
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert_eq!(page.background_chat_panels.len(), 1);
      assert_eq!(page.background_chat_panels[0].0, first_id);
    });

    // Coming back revives the very same panel entity: nothing was reloaded.
    page.update_in(cx, |page, window, cx| {
      page.select_session(&first_id, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(active_panel(&page, cx).entity_id(), first.entity_id());
    page.read_with(cx, |page, cx| {
      assert_eq!(page.background_chat_panels.len(), 1);
      let store = page.chat_store.as_ref().expect("store").read(cx);
      assert_eq!(
        store.active_id(),
        Some(first_id.as_str()),
        "a relaunch would reopen the switched-to conversation"
      );
    });
  }

  #[gpui::test]
  async fn selecting_another_repo_session_waits_for_dirty_file_choice(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-dirty-switch", cx).await;
    let (other, other_state) = seed_second_repo("session-page-dirty-switch-b", "other-session");

    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("track the other repo");
      page
        .set_fallback_repo(repo.path.clone(), window, cx)
        .expect("switch back to the first repo");
    });
    cx.run_until_parked();
    let current_id =
      active_panel(&page, cx).read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    let editor = page
      .read_with(cx, |page, _| page.editor.clone())
      .expect("editor");
    editor.update(cx, |editor, cx| {
      editor.document.update(cx, |document, cx| {
        document.replace_all("unsaved\n", cx);
      });
      editor.is_dirty = true;
    });

    page.update_in(cx, |page, window, cx| {
      page.select_session("other-session", window, cx)
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    assert!(
      cx.debug_bounds(UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR)
        .is_some()
    );
    assert_eq!(
      active_panel(&page, cx).read_with(cx, |panel, _| panel.current_conversation().id.clone()),
      current_id,
      "the session should not switch before the modal choice"
    );

    page.update_in(cx, |page, window, cx| {
      page.discard_unsaved_editor_for_test(
        UnsavedEditorAction::SelectSession {
          id: "other-session".to_string(),
        },
        window,
        cx,
      );
    });
    cx.run_until_parked();

    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
    assert_eq!(
      active_panel(&page, cx).read_with(cx, |panel, _| panel.current_conversation().id.clone()),
      "other-session"
    );
    let _ = std::fs::remove_dir_all(&other_state);
  }

  #[gpui::test]
  async fn selecting_a_same_checkout_session_keeps_dirty_file_without_prompt(
    cx: &mut TestAppContext,
  ) {
    let (_repo, page, cx) = page_with_agent_panel("session-page-dirty-same-checkout", cx).await;

    let first = active_panel(&page, cx);
    first.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("first", cx)
    });
    cx.run_until_parked();
    let first_id = first.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update_in(cx, |page, window, cx| page.new_session(window, cx));
    cx.run_until_parked();
    let second_id =
      active_panel(&page, cx).read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    let editor = page
      .read_with(cx, |page, _| page.editor.clone())
      .expect("editor");
    editor.update(cx, |editor, cx| {
      editor.document.update(cx, |document, cx| {
        document.replace_all("unsaved\n", cx);
      });
      editor.is_dirty = true;
    });

    page.update_in(cx, |page, window, cx| {
      page.select_session(&first_id, window, cx)
    });
    cx.run_until_parked();

    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
    assert_eq!(
      active_panel(&page, cx).read_with(cx, |panel, _| panel.current_conversation().id.clone()),
      first_id
    );
    assert_ne!(first_id, second_id);
    page.read_with(cx, |page, cx| {
      assert!(page.editor.as_ref().expect("editor").read(cx).is_dirty);
    });
  }

  #[gpui::test]
  async fn starting_a_worktree_session_waits_for_dirty_file_choice(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-dirty-worktree", cx).await;
    let first_id =
      active_panel(&page, cx).read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    let editor = page
      .read_with(cx, |page, _| page.editor.clone())
      .expect("editor");
    editor.update(cx, |editor, cx| {
      editor.document.update(cx, |document, cx| {
        document.replace_all("unsaved\n", cx);
      });
      editor.is_dirty = true;
    });

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    assert!(
      cx.debug_bounds(UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR)
        .is_some()
    );
    assert_eq!(
      active_panel(&page, cx).read_with(cx, |panel, _| panel.current_conversation().id.clone()),
      first_id,
      "the worktree session should not start before the modal choice"
    );

    page.update_in(cx, |page, window, cx| {
      page.discard_unsaved_editor_for_test(
        UnsavedEditorAction::NewWorktreeSessionIn {
          repo_root: repo.path.clone(),
          base: None,
        },
        window,
        cx,
      );
    });
    cx.run_until_parked();

    let panel = active_panel(&page, cx);
    panel.read_with(cx, |panel, _| {
      assert_ne!(panel.current_conversation().id, first_id);
      assert_ne!(panel.cwd(), repo.path.as_path());
    });
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn deleting_the_active_session_starts_a_fresh_one(cx: &mut TestAppContext) {
    let (_repo, page, cx) = page_with_agent_panel("session-page-delete-active", cx).await;

    let first = active_panel(&page, cx);
    first.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("doomed", cx)
    });
    cx.run_until_parked();
    let first_id = first.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update_in(cx, |page, window, cx| {
      page.delete_session(&first_id, window, cx)
    });
    cx.run_until_parked();

    let fresh = active_panel(&page, cx);
    assert_ne!(fresh.entity_id(), first.entity_id());
    page.read_with(cx, |page, cx| {
      let store = page.chat_store.as_ref().expect("store").read(cx);
      assert!(store.list().is_empty(), "the conversation left the index");
      assert_eq!(store.active_id(), None);
      assert!(page.background_chat_panels.is_empty());
    });
  }

  #[gpui::test]
  async fn deleting_a_background_session_drops_its_panel(cx: &mut TestAppContext) {
    let (_repo, page, cx) = page_with_agent_panel("session-page-delete-background", cx).await;

    let first = active_panel(&page, cx);
    first.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("first", cx)
    });
    cx.run_until_parked();
    let first_id = first.read_with(cx, |panel, _| panel.current_conversation().id.clone());
    page.update_in(cx, |page, window, cx| page.new_session(window, cx));
    let second = active_panel(&page, cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.delete_session(&first_id, window, cx)
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert!(page.background_chat_panels.is_empty());
      let store = page.chat_store.as_ref().expect("store").read(cx);
      assert!(store.list().is_empty());
    });
    // The shown session was untouched.
    assert_eq!(active_panel(&page, cx).entity_id(), second.entity_id());
  }

  #[gpui::test]
  async fn parked_panels_beyond_the_cap_are_dropped_oldest_first(cx: &mut TestAppContext) {
    let (_repo, page, cx) = page_with_agent_panel("session-page-eviction", cx).await;

    for i in 0..8 {
      let panel = active_panel(&page, cx);
      panel.update(cx, |panel, cx| {
        panel.seed_user_message_for_test(format!("session {i}"), cx)
      });
      cx.run_until_parked();
      page.update_in(cx, |page, window, cx| page.new_session(window, cx));
      cx.run_until_parked();
    }

    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.background_chat_panels.len(),
        5,
        "idle panels beyond the cap are dropped"
      );
      let store = page.chat_store.as_ref().expect("store").read(cx);
      assert_eq!(
        store.list().len(),
        8,
        "evicting a panel never deletes its conversation"
      );
    });
  }

  #[gpui::test]
  async fn a_running_background_session_is_never_evicted_and_blocks_repo_moves(
    cx: &mut TestAppContext,
  ) {
    let (_repo, page, cx) = page_with_agent_panel("session-page-running-background", cx).await;

    let first = active_panel(&page, cx);
    first.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("busy one", cx);
      panel.pretend_turn_in_flight_for_test(cx);
    });
    cx.run_until_parked();

    // Park it behind six more sessions: the cap must not touch it.
    for i in 0..6 {
      page.update_in(cx, |page, window, cx| page.new_session(window, cx));
      let panel = active_panel(&page, cx);
      panel.update(cx, |panel, cx| {
        panel.seed_user_message_for_test(format!("filler {i}"), cx)
      });
      cx.run_until_parked();
    }

    page.read_with(cx, |page, cx| {
      assert!(
        page
          .background_chat_panels
          .iter()
          .any(|(_, panel)| panel.entity_id() == first.entity_id()),
        "the running panel survived the eviction pass"
      );
      assert!(page.agent_turn_in_flight(cx), "a background turn counts");
    });
  }

  #[gpui::test]
  async fn activating_the_shell_resumes_the_conversation_left_active(cx: &mut TestAppContext) {
    agent_chat_panel::set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
    let repo = TempRepo::init("session-page-resume-active");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let state_dir = agent_chat_state_dir()
      .map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &repo.path))
      .expect("agent chat state dir");
    let _ = std::fs::remove_dir_all(&state_dir);
    std::fs::create_dir_all(&state_dir).expect("create state dir");

    // What a previous run left behind: two conversations, one marked active.
    let meta = serde_json::json!({
      "id": "resumed-conversation",
      "started_at_secs": 1,
      "updated_at_secs": 2,
      "title": "Left open last time",
      "message_count": 1,
      "agent_id": "pi-acp",
      "session_id": null,
      "preview": "hello from disk"
    });
    let other = serde_json::json!({
      "id": "some-other-conversation",
      "started_at_secs": 1,
      "updated_at_secs": 3,
      "title": "Not this one",
      "message_count": 1,
      "agent_id": "codex-acp",
      "session_id": null,
      "preview": ""
    });
    let index = serde_json::json!({ "version": 1, "conversations": [other, meta.clone()] });
    std::fs::write(state_dir.join("index.json"), index.to_string()).expect("write index");
    let transcript = serde_json::json!({
      "version": 1,
      "meta": meta,
      "items": [{ "type": "Message", "role": "User", "text": "hello from disk", "images": 0 }],
      "group_pins": {},
      "auto_approve": false
    });
    std::fs::write(
      state_dir.join("resumed-conversation.json"),
      transcript.to_string(),
    )
    .expect("write transcript");
    std::fs::write(state_dir.join("active.txt"), "resumed-conversation").expect("write active");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| page.activate(window, cx));
    cx.run_until_parked();

    let panel = active_panel(&page, cx);
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.current_conversation().id, "resumed-conversation");
      assert_eq!(
        panel.backend_kind(),
        &agent_registry::AgentId::new("pi-acp")
      );
      assert_eq!(
        panel.transcript_texts(),
        vec!["hello from disk".to_string()],
        "the transcript hydrated from disk"
      );
    });

    let _ = std::fs::remove_dir_all(&state_dir);
  }

  fn cleanup_worktrees_root(repo_root: &Path) {
    if let Ok(root) = git::worktrees_root_for(repo_root) {
      let _ = std::fs::remove_dir_all(root);
    }
  }

  #[gpui::test]
  async fn a_worktree_session_binds_its_checkout_and_survives_a_revisit(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-worktree-bind", cx).await;

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();

    let panel = active_panel(&page, cx);
    let (conversation_id, cwd) = panel.read_with(cx, |panel, _| {
      (
        panel.current_conversation().id.clone(),
        panel.cwd().to_path_buf(),
      )
    });
    assert_ne!(cwd, repo.path, "the agent works in its own checkout");
    assert!(cwd.is_dir(), "the worktree exists on disk");
    assert!(
      cwd.join(".git").is_file(),
      "and it is a linked worktree of the repository"
    );
    let binding = page.read_with(cx, |page, cx| {
      page
        .chat_store
        .as_ref()
        .expect("store")
        .read(cx)
        .worktree(&conversation_id)
        .expect("the binding was persisted")
    });
    assert_eq!(binding.path, cwd);
    assert!(binding.branch.starts_with("reviu-"));

    // Give it content, park it behind a fresh session, come back: same
    // panel, same checkout.
    panel.update(cx, |panel, cx| panel.seed_user_message_for_test("work", cx));
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| page.new_session(window, cx));
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.select_session(&conversation_id, window, cx)
    });
    cx.run_until_parked();
    let revived = active_panel(&page, cx);
    assert_eq!(revived.entity_id(), panel.entity_id());
    revived.read_with(cx, |panel, _| assert_eq!(panel.cwd(), cwd.as_path()));

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn the_new_session_action_lands_where_you_are(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-new-session-action", cx).await;
    let (other, other_state) = seed_second_repo("session-page-new-session-action-b", "action-b");

    // The other repo's session on screen, fallback on the first repo.
    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("track the other repo");
      page
        .set_fallback_repo(repo.path.clone(), window, cx)
        .expect("switch back");
      page.select_session("action-b", window, cx);
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.new_agent_session_action(&crate::NewAgentSession, window, cx)
    });
    cx.run_until_parked();

    let panel = active_panel(&page, cx);
    panel.read_with(cx, |panel, _| {
      assert_eq!(
        panel.repo_root(),
        other.path.as_path(),
        "creation lands in the SHOWN session's repo, not the fallback"
      );
      assert!(!panel.has_persistable_content(), "a fresh blank session");
    });

    let _ = std::fs::remove_dir_all(&other_state);
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn the_worktree_session_action_lands_where_you_are(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-worktree-action", cx).await;

    page.update_in(cx, |page, window, cx| {
      page.new_agent_worktree_session_action(&crate::NewAgentWorktreeSession, window, cx)
    });
    cx.run_until_parked();

    let panel = active_panel(&page, cx);
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.repo_root(), repo.path.as_path());
      assert_ne!(panel.cwd(), repo.path.as_path(), "its own checkout");
      assert!(panel.cwd().join(".git").is_file());
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn session_creation_actions_refuse_without_a_repository(cx: &mut TestAppContext) {
    let (page, cx) = add_session_page_window_without_repo(cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.new_agent_session_action(&crate::NewAgentSession, window, cx);
      page.new_agent_worktree_session_action(&crate::NewAgentWorktreeSession, window, cx);
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert!(page.agent_chat_view.is_none(), "nothing to create in");
    });
  }

  #[gpui::test]
  async fn a_worktree_session_can_start_in_a_repo_other_than_the_fallback(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-worktree-cross", cx).await;
    let other = TempRepo::init("session-page-worktree-cross-b");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");
    let other_state = agent_chat_state_dir()
      .map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &other.path))
      .expect("agent chat state dir");
    let _ = std::fs::remove_dir_all(&other_state);

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(other.path.clone(), None, window, cx)
    });
    cx.run_until_parked();

    let panel = active_panel(&page, cx);
    panel.read_with(cx, |panel, _| {
      assert_eq!(
        panel.repo_root(),
        other.path.as_path(),
        "the session belongs to the section's repo, not the fallback"
      );
      assert_ne!(panel.cwd(), other.path.as_path());
      assert!(
        panel.cwd().join(".git").is_file(),
        "a linked worktree of the other repo"
      );
    });
    page.read_with(cx, |page, _| {
      assert_eq!(
        page.fallback_repo.as_deref(),
        Some(repo.path.as_path()),
        "the fallback did not move"
      );
    });

    let _ = std::fs::remove_dir_all(&other_state);
    cleanup_worktrees_root(&other.path);
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn deleting_a_worktree_session_removes_its_checkout_and_checkpoints(
    cx: &mut TestAppContext,
  ) {
    let (repo, page, cx) = page_with_agent_panel("session-page-worktree-delete", cx).await;

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    let panel = active_panel(&page, cx);
    panel.update(cx, |panel, cx| panel.seed_user_message_for_test("work", cx));
    cx.run_until_parked();
    let (conversation_id, cwd) = panel.read_with(cx, |panel, _| {
      (
        panel.current_conversation().id.clone(),
        panel.cwd().to_path_buf(),
      )
    });
    // A turn left a checkpoint ref behind, in the shared .git.
    git::create_checkpoint(&cwd, &conversation_id).expect("checkpoint from the worktree");
    assert_eq!(
      git::list_checkpoints(&repo.path, &conversation_id)
        .expect("list refs")
        .len(),
      1
    );

    page.update_in(cx, |page, window, cx| {
      page.delete_session(&conversation_id, window, cx)
    });
    cx.run_until_parked();

    assert!(!cwd.exists(), "the worktree went with the session");
    assert!(
      git::list_checkpoints(&repo.path, &conversation_id)
        .expect("list refs after delete")
        .is_empty(),
      "the checkpoint refs went with it"
    );
    page.read_with(cx, |page, cx| {
      let store = page.chat_store.as_ref().expect("store").read(cx);
      assert_eq!(store.worktree(&conversation_id), None);
      assert!(store.list().is_empty());
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn a_stale_worktree_binding_falls_back_to_the_main_checkout(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-worktree-stale", cx).await;

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    let panel = active_panel(&page, cx);
    panel.update(cx, |panel, cx| panel.seed_user_message_for_test("work", cx));
    cx.run_until_parked();
    let (conversation_id, cwd) = panel.read_with(cx, |panel, _| {
      (
        panel.current_conversation().id.clone(),
        panel.cwd().to_path_buf(),
      )
    });

    // The worktree vanishes outside Reviu while the session is parked and
    // its panel evicted (simulated by dropping it from the map).
    page.update_in(cx, |page, window, cx| page.new_session(window, cx));
    cx.run_until_parked();
    page.update(cx, |page, _| page.background_chat_panels.clear());
    std::fs::remove_dir_all(&cwd).expect("delete the worktree externally");

    page.update_in(cx, |page, window, cx| {
      page.select_session(&conversation_id, window, cx)
    });
    cx.run_until_parked();

    let rebuilt = active_panel(&page, cx);
    rebuilt.read_with(cx, |panel, _| {
      assert_eq!(
        panel.cwd(),
        repo.path.as_path(),
        "a gone worktree falls back to the main checkout"
      );
    });
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .chat_store
          .as_ref()
          .expect("store")
          .read(cx)
          .worktree(&conversation_id),
        None,
        "the stale binding was dropped"
      );
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn a_turn_checkpoint_snapshots_the_sessions_worktree_not_the_main_checkout(
    cx: &mut TestAppContext,
  ) {
    let (repo, page, cx) = page_with_agent_panel("session-page-worktree-checkpoint", cx).await;

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    let panel = active_panel(&page, cx);
    let (conversation_id, cwd) = panel.read_with(cx, |panel, _| {
      (
        panel.current_conversation().id.clone(),
        panel.cwd().to_path_buf(),
      )
    });

    // The agent edited in its worktree; the turn snapshot must capture THAT.
    std::fs::write(cwd.join("README.md"), "agent v1\n").expect("edit in the worktree");
    page.update(cx, |page, cx| {
      page.create_turn_checkpoint(panel.clone(), cx);
    });
    cx.run_until_parked();
    let checkpoints =
      git::list_checkpoints(&repo.path, &conversation_id).expect("list checkpoint refs");
    assert_eq!(checkpoints.len(), 1, "the turn snapshot landed");

    // Rolling back restores the worktree and leaves the main checkout alone.
    std::fs::write(cwd.join("README.md"), "agent v2\n").expect("edit again");
    page.update_in(cx, |page, window, cx| {
      page.rollback_to_checkpoint(checkpoints[0].ref_name.clone(), window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
      std::fs::read_to_string(cwd.join("README.md")).expect("read worktree file"),
      "agent v1\n",
      "the rollback rewound the session's worktree"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read main file"),
      "v1\n",
      "the main checkout never moved"
    );

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn a_worktree_session_in_an_empty_repository_fails_softly(cx: &mut TestAppContext) {
    agent_chat_panel::set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
    // No commit: `git worktree add` has no base to start from.
    let repo = TempRepo::init("session-page-worktree-empty");
    let state_dir = agent_chat_state_dir()
      .map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &repo.path))
      .expect("agent chat state dir");
    let _ = std::fs::remove_dir_all(&state_dir);
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| page.activate(window, cx));
    cx.run_until_parked();
    let before = active_panel(&page, cx);

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();

    assert_eq!(
      active_panel(&page, cx).entity_id(),
      before.entity_id(),
      "a failed worktree creation leaves the shown session in place"
    );
    page.read_with(cx, |page, cx| {
      assert!(page.background_chat_panels.is_empty());
      let conversation_id = before.read(cx).current_conversation().id.clone();
      assert_eq!(
        page
          .chat_store
          .as_ref()
          .expect("store")
          .read(cx)
          .worktree(&conversation_id),
        None,
        "nothing was bound"
      );
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn activating_resumes_a_worktree_session_and_points_the_dock_at_it(
    cx: &mut TestAppContext,
  ) {
    agent_chat_panel::set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
    let repo = TempRepo::init("session-page-resume-worktree");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let worktree = git::create_worktree(&repo.path, None).expect("create worktree");

    // What a previous run left behind: a worktree session marked active.
    let state_dir = agent_chat_state_dir()
      .map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &repo.path))
      .expect("agent chat state dir");
    let _ = std::fs::remove_dir_all(&state_dir);
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    let meta = serde_json::json!({
      "id": "worktree-conversation",
      "started_at_secs": 1,
      "updated_at_secs": 2,
      "title": "In a worktree",
      "message_count": 1,
      "session_id": null,
      "preview": "hello"
    });
    std::fs::write(
      state_dir.join("index.json"),
      serde_json::json!({ "version": 1, "conversations": [meta.clone()] }).to_string(),
    )
    .expect("write index");
    std::fs::write(
      state_dir.join("worktree-conversation.json"),
      serde_json::json!({
        "version": 1,
        "meta": meta,
        "items": [{ "type": "Message", "role": "User", "text": "hello", "images": 0 }],
        "group_pins": {},
        "auto_approve": false
      })
      .to_string(),
    )
    .expect("write transcript");
    std::fs::write(
      state_dir.join("worktrees.json"),
      serde_json::json!({
        "worktree-conversation": { "path": worktree.path, "branch": worktree.branch }
      })
      .to_string(),
    )
    .expect("write bindings");
    std::fs::write(state_dir.join("active.txt"), "worktree-conversation").expect("write active");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| page.activate(window, cx));
    cx.run_until_parked();

    let panel = active_panel(&page, cx);
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.current_conversation().id, "worktree-conversation");
      assert_eq!(
        panel.cwd(),
        worktree.path.as_path(),
        "the resumed session works in its worktree again"
      );
    });
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.dock_panel.read(cx).repo_root(),
        Some(worktree.path.as_path()),
        "the dock followed the resumed checkout, not the main one"
      );
    });

    let _ = std::fs::remove_dir_all(&state_dir);
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn the_git_surfaces_follow_the_active_sessions_checkout(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-checkout-follow", cx).await;

    let main_session = active_panel(&page, cx);
    main_session.update(cx, |panel, cx| panel.seed_user_message_for_test("main", cx));
    cx.run_until_parked();
    let main_id = main_session.read_with(cx, |panel, _| panel.current_conversation().id.clone());
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.dock_panel.read(cx).repo_root(),
        Some(repo.path.as_path())
      );
    });

    // A worktree session points the dock and the branch header at its checkout.
    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    let worktree_panel = active_panel(&page, cx);
    let cwd = worktree_panel.read_with(cx, |panel, _| panel.cwd().to_path_buf());
    let branch = page.read_with(cx, |page, cx| {
      assert_eq!(page.dock_panel.read(cx).repo_root(), Some(cwd.as_path()));
      page
        .chat_store
        .as_ref()
        .expect("store")
        .read(cx)
        .worktree(&worktree_panel.read(cx).current_conversation().id)
        .expect("binding")
        .branch
    });
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.repo_snapshot.read(cx).current_branch_name(),
        Some(branch.as_str()),
        "the branch header names the worktree's branch"
      );
    });

    // Back on the main session, everything points home again.
    worktree_panel.update(cx, |panel, cx| panel.seed_user_message_for_test("keep", cx));
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.select_session(&main_id, window, cx)
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.dock_panel.read(cx).repo_root(),
        Some(repo.path.as_path())
      );
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn an_open_diff_survives_a_same_checkout_switch_and_closes_on_a_checkout_change(
    cx: &mut TestAppContext,
  ) {
    let (repo, page, cx) = page_with_agent_panel("session-page-checkout-diff", cx).await;
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("dirty the main checkout");

    let first = active_panel(&page, cx);
    first.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("first", cx)
    });
    cx.run_until_parked();
    let first_id = first.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    // Another MAIN-checkout session: same checkout, the diff stays.
    page.update_in(cx, |page, window, cx| page.new_session(window, cx));
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert_eq!(
        page.center,
        CenterView::Diff,
        "same checkout keeps the diff"
      );
    });

    // A worktree session changes the checkout: the diff belongs to the one left.
    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.editor.is_none());
      assert!(page.selected_file.is_none());
    });

    let _ = first_id;
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn starting_a_same_checkout_session_reopens_the_chat_without_closing_the_editor(
    cx: &mut TestAppContext,
  ) {
    let (repo, page, cx) = page_with_agent_panel("session-page-new-session-chat", cx).await;
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("dirty the main checkout");

    let first = active_panel(&page, cx);
    first.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("first", cx)
    });
    cx.run_until_parked();
    let first_id = first.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, window, cx| page.hide_diff_chat(window, cx));
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| page.new_session(window, cx));
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert_eq!(page.center, CenterView::Diff);
      assert!(page.diff_chat_open);
      assert_eq!(page.selected_file.as_deref(), Some(Path::new("README.md")));
      assert!(page.editor.is_some());
      assert_ne!(
        page
          .agent_chat_view
          .as_ref()
          .expect("active chat")
          .read(cx)
          .current_conversation()
          .id
          .as_str(),
        first_id.as_str()
      );
    });
    assert!(cx.debug_bounds("session-conversation-pane").is_some());
    assert!(cx.debug_bounds("session-diff-editor").is_some());

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn starting_a_worktree_session_from_a_hidden_chat_shows_the_conversation(
    cx: &mut TestAppContext,
  ) {
    let (repo, page, cx) = page_with_agent_panel("session-page-worktree-chat", cx).await;
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("dirty the main checkout");

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, window, cx| page.hide_diff_chat(window, cx));
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();

    let cwd = active_panel(&page, cx).read_with(cx, |panel, _| panel.cwd().to_path_buf());
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.diff_chat_open);
      assert!(page.editor.is_none());
      assert!(page.selected_file.is_none());
      assert_ne!(cwd, repo.path);
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn selecting_a_same_checkout_session_reopens_the_chat_without_closing_the_editor(
    cx: &mut TestAppContext,
  ) {
    let (repo, page, cx) = page_with_agent_panel("session-page-same-checkout-chat", cx).await;
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("dirty the main checkout");

    let first = active_panel(&page, cx);
    first.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("first", cx)
    });
    cx.run_until_parked();
    let first_id = first.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| page.new_session(window, cx));
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| page.hide_diff_chat(window, cx));
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert!(!page.diff_chat_open);
      assert!(page.editor.is_some());
    });

    page.update_in(cx, |page, window, cx| {
      page.select_session(&first_id, window, cx)
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert_eq!(page.center, CenterView::Diff);
      assert!(page.diff_chat_open);
      assert_eq!(page.selected_file.as_deref(), Some(Path::new("README.md")));
      assert!(page.editor.is_some());
      assert_eq!(
        page
          .agent_chat_view
          .as_ref()
          .expect("active chat")
          .read(cx)
          .current_conversation()
          .id
          .as_str(),
        first_id.as_str()
      );
    });
    assert!(cx.debug_bounds("session-conversation-pane").is_some());
    assert!(cx.debug_bounds("session-diff-editor").is_some());

    page.update_in(cx, |page, window, cx| page.hide_diff_chat(window, cx));
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.select_session(&first_id, window, cx)
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert!(page.diff_chat_open);
      assert!(page.editor.is_some());
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn a_turn_in_a_worktree_does_not_block_work_in_the_main_checkout(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-checkout-guard", cx).await;

    let main_session = active_panel(&page, cx);
    main_session.update(cx, |panel, cx| panel.seed_user_message_for_test("main", cx));
    cx.run_until_parked();
    let main_id = main_session.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    let worktree_panel = active_panel(&page, cx);
    let worktree_cwd = worktree_panel.read_with(cx, |panel, _| panel.cwd().to_path_buf());
    worktree_panel.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("busy", cx);
      panel.pretend_turn_in_flight_for_test(cx);
    });
    cx.run_until_parked();

    // Back on the main session while the worktree agent runs.
    page.update_in(cx, |page, window, cx| {
      page.select_session(&main_id, window, cx)
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert!(
        page.agent_turn_in_flight(cx),
        "the worktree turn is real work"
      );
      assert!(
        !page.agent_turn_in_flight_at(&repo.path, cx),
        "but it does not occupy the main checkout"
      );
      assert!(page.agent_turn_in_flight_at(&worktree_cwd, cx));
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn the_worktree_branch_takes_the_conversations_title(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-worktree-rename", cx).await;

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    let panel = active_panel(&page, cx);
    let conversation_id = panel.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    // The first user message titles the conversation; the branch follows.
    panel.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("Fix the scroll jump", cx)
    });
    cx.run_until_parked();

    let binding = page.read_with(cx, |page, cx| {
      page
        .chat_store
        .as_ref()
        .expect("store")
        .read(cx)
        .worktree(&conversation_id)
        .expect("binding")
    });
    assert_eq!(binding.branch, "reviu-fix-the-scroll-jump");
    assert_eq!(
      git::list_worktrees(&repo.path).expect("list")[0]
        .branch
        .as_deref(),
      Some("reviu-fix-the-scroll-jump"),
      "the checkout rode along with its renamed branch"
    );
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .session_list
          .read(cx)
          .worktree_branch_of(&conversation_id),
        Some("reviu-fix-the-scroll-jump"),
        "the sidebar row shows the new name"
      );
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn a_session_deleted_during_the_rename_never_gets_a_zombie_binding(
    cx: &mut TestAppContext,
  ) {
    let (repo, page, cx) = page_with_agent_panel("session-page-rename-deleted", cx).await;

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    let panel = active_panel(&page, cx);
    let conversation_id = panel.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    // The title lands (rename task spawned) and the session dies right after,
    // before anything parked: whatever the rename outcome, the deleted
    // conversation must not come back with a binding.
    panel.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("Fix the scroll jump", cx)
    });
    page.update_in(cx, |page, window, cx| {
      page.delete_session(&conversation_id, window, cx)
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .chat_store
          .as_ref()
          .expect("store")
          .read(cx)
          .worktree(&conversation_id),
        None,
        "a finished rename must not resurrect the deleted session's binding"
      );
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn a_rename_lost_to_a_crash_heals_on_the_next_run(cx: &mut TestAppContext) {
    agent_chat_panel::set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
    let repo = TempRepo::init("session-page-rename-heal");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    // What a crash between the title and the rename leaves behind: a titled
    // conversation still bound to its generated branch.
    let worktree = git::create_worktree(&repo.path, None).expect("create worktree");
    let state_dir = agent_chat_state_dir()
      .map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &repo.path))
      .expect("agent chat state dir");
    let _ = std::fs::remove_dir_all(&state_dir);
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    let meta = serde_json::json!({
      "id": "titled-conversation",
      "started_at_secs": 1,
      "updated_at_secs": 2,
      "title": "Fix the scroll jump",
      "message_count": 1,
      "session_id": null,
      "preview": "hello"
    });
    std::fs::write(
      state_dir.join("index.json"),
      serde_json::json!({ "version": 1, "conversations": [meta.clone()] }).to_string(),
    )
    .expect("write index");
    std::fs::write(
      state_dir.join("titled-conversation.json"),
      serde_json::json!({
        "version": 1,
        "meta": meta,
        "items": [{ "type": "Message", "role": "User", "text": "Fix the scroll jump", "images": 0 }],
        "group_pins": {},
        "auto_approve": false
      })
      .to_string(),
    )
    .expect("write transcript");
    std::fs::write(
      state_dir.join("worktrees.json"),
      serde_json::json!({
        "titled-conversation": { "path": worktree.path, "branch": worktree.branch }
      })
      .to_string(),
    )
    .expect("write bindings");
    std::fs::write(state_dir.join("active.txt"), "titled-conversation").expect("write active");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| page.activate(window, cx));
    cx.run_until_parked();

    // The next persist re-announces the title and the rename catches up.
    let panel = active_panel(&page, cx);
    panel.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("more work", cx)
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .chat_store
          .as_ref()
          .expect("store")
          .read(cx)
          .worktree("titled-conversation")
          .expect("binding")
          .branch,
        "reviu-fix-the-scroll-jump",
        "the crashed rename healed on the next run"
      );
    });

    let _ = std::fs::remove_dir_all(&state_dir);
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn a_worktree_session_starts_from_the_picked_base(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-worktree-base-pick", cx).await;
    let main_oid = crate::test_support::head_oid(&repo.path).to_string();
    std::process::Command::new("git")
      .current_dir(&repo.path)
      .args(["switch", "-c", "feature"])
      .output()
      .expect("create feature branch");
    commit_text_file(&repo.path, Path::new("extra.txt"), "x\n", "on feature");

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), Some("feature".to_string()), window, cx)
    });
    cx.run_until_parked();
    let feature_oid = crate::test_support::head_oid(&repo.path).to_string();
    let cwd = active_panel(&page, cx).read_with(cx, |panel, _| panel.cwd().to_path_buf());
    assert_eq!(
      crate::test_support::head_oid(&cwd).to_string(),
      feature_oid,
      "the worktree starts from the picked base"
    );
    assert_ne!(feature_oid, main_oid);

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn an_abandoned_blank_worktree_session_takes_its_checkout_with_it(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-worktree-blank", cx).await;

    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    let cwd = active_panel(&page, cx).read_with(cx, |panel, _| panel.cwd().to_path_buf());
    assert!(cwd.is_dir());

    // Never used: switching away drops the blank panel and its worktree.
    page.update_in(cx, |page, window, cx| page.new_session(window, cx));
    cx.run_until_parked();

    assert!(!cwd.exists(), "the unused worktree was removed");
    page.read_with(cx, |page, _| {
      assert!(page.background_chat_panels.is_empty());
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn the_sidebar_shows_each_sessions_live_status_and_worktree_branch(
    cx: &mut TestAppContext,
  ) {
    let (repo, page, cx) = page_with_agent_panel("session-page-sidebar-status", cx).await;

    // A worktree session left running in the background.
    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    let working = active_panel(&page, cx);
    working.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("busy", cx);
      panel.pretend_turn_in_flight_for_test(cx);
    });
    cx.run_until_parked();
    let working_id = working.read_with(cx, |panel, _| panel.current_conversation().id.clone());
    let branch = page.read_with(cx, |page, cx| {
      page
        .chat_store
        .as_ref()
        .expect("store")
        .read(cx)
        .worktree(&working_id)
        .expect("binding")
        .branch
    });

    // A second session, idle in the foreground (its connection is dead under
    // this fixture, which is exactly what Failed reports).
    page.update_in(cx, |page, window, cx| page.new_session(window, cx));
    cx.run_until_parked();
    let failed = active_panel(&page, cx);
    failed.update(cx, |panel, cx| panel.seed_user_message_for_test("idle", cx));
    cx.run_until_parked();
    let failed_id = failed.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update(cx, |page, cx| page.refresh_session_list(cx));
    page.read_with(cx, |page, cx| {
      let list = page.session_list.read(cx);
      assert_eq!(
        list.status_of(&working_id),
        SessionStatus::Working,
        "a background turn shows as Working on its row"
      );
      let _ = list;
    });

    // The working session hits a permission wall: Waiting outranks Working.
    working.update(cx, |panel, cx| {
      panel.seed_unresolved_permission_for_test(cx);
    });
    page.update(cx, |page, cx| page.refresh_session_list(cx));
    page.read_with(cx, |page, cx| {
      let list = page.session_list.read(cx);
      assert_eq!(
        list.status_of(&working_id),
        SessionStatus::Waiting,
        "a turn parked on a permission shows as Waiting, not Working"
      );
      assert_eq!(
        list.status_of(&failed_id),
        SessionStatus::Failed,
        "a dead connection shows as Failed"
      );
      assert_eq!(
        list.worktree_branch_of(&working_id),
        Some(branch.as_str()),
        "the worktree row names its branch"
      );
      assert_eq!(list.worktree_branch_of(&failed_id), None);
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn the_boot_sweep_removes_orphans_and_spares_bound_and_user_worktrees(
    cx: &mut TestAppContext,
  ) {
    agent_chat_panel::set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
    let repo = TempRepo::init("session-page-sweep");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    // Three worktrees before the app boots: one bound to a conversation, one
    // orphaned, one the user claimed by renaming its branch.
    let bound = git::create_worktree(&repo.path, None).expect("bound worktree");
    let orphan = git::create_worktree(&repo.path, None).expect("orphan worktree");
    let claimed = git::create_worktree(&repo.path, None).expect("claimed worktree");
    std::process::Command::new("git")
      .current_dir(&claimed.path)
      .args(["switch", "-c", "my-own-work"])
      .output()
      .expect("rename the claimed branch");

    let state_dir = agent_chat_state_dir()
      .map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &repo.path))
      .expect("agent chat state dir");
    let _ = std::fs::remove_dir_all(&state_dir);
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    let meta = serde_json::json!({
      "id": "bound-conversation",
      "started_at_secs": 1,
      "updated_at_secs": 2,
      "title": "Bound",
      "message_count": 1,
      "session_id": null,
      "preview": "hello"
    });
    std::fs::write(
      state_dir.join("index.json"),
      serde_json::json!({ "version": 1, "conversations": [meta] }).to_string(),
    )
    .expect("write index");
    std::fs::write(
      state_dir.join("worktrees.json"),
      serde_json::json!({
        "bound-conversation": { "path": bound.path, "branch": bound.branch }
      })
      .to_string(),
    )
    .expect("write bindings");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| page.activate(window, cx));
    cx.run_until_parked();

    assert!(!orphan.path.exists(), "the orphan was swept at boot");
    assert!(bound.path.exists(), "a bound worktree is not an orphan");
    assert!(
      claimed.path.exists(),
      "a worktree whose branch the user renamed is theirs now"
    );

    let _ = std::fs::remove_dir_all(&state_dir);
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn the_sweep_spares_a_blank_worktree_session_that_is_still_alive(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-sweep-blank-live", cx).await;

    // A fresh worktree session: bound, but blank, so absent from the index.
    page.update_in(cx, |page, window, cx| {
      page.new_worktree_session_in(repo.path.clone(), None, window, cx)
    });
    cx.run_until_parked();
    let panel = active_panel(&page, cx);
    let (conversation_id, cwd) = panel.read_with(cx, |panel, _| {
      (
        panel.current_conversation().id.clone(),
        panel.cwd().to_path_buf(),
      )
    });

    // The sweep runs again, as it would if it raced the creation at boot.
    page.update(cx, |page, cx| {
      let repo_root = page.fallback_repo.clone().expect("fallback repo");
      let store = page.chat_store.clone().expect("fallback store");
      page.sweep_orphan_worktrees(repo_root, store, cx);
    });
    cx.run_until_parked();

    assert!(
      cwd.exists(),
      "a live session's checkout is never an orphan, blank or not"
    );
    page.read_with(cx, |page, cx| {
      assert!(
        page
          .chat_store
          .as_ref()
          .expect("store")
          .read(cx)
          .worktree(&conversation_id)
          .is_some(),
        "its binding survived too"
      );
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn a_binding_whose_conversation_is_gone_is_dropped_with_its_worktree(
    cx: &mut TestAppContext,
  ) {
    agent_chat_panel::set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
    let repo = TempRepo::init("session-page-sweep-stale-binding");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let stale = git::create_worktree(&repo.path, None).expect("stale worktree");

    // The binding survived a prune that took its conversation away.
    let state_dir = agent_chat_state_dir()
      .map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &repo.path))
      .expect("agent chat state dir");
    let _ = std::fs::remove_dir_all(&state_dir);
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    std::fs::write(
      state_dir.join("worktrees.json"),
      serde_json::json!({
        "pruned-conversation": { "path": stale.path, "branch": stale.branch }
      })
      .to_string(),
    )
    .expect("write bindings");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| page.activate(window, cx));
    cx.run_until_parked();

    assert!(
      !stale.path.exists(),
      "the unreferenced checkout was removed"
    );
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .chat_store
          .as_ref()
          .expect("store")
          .read(cx)
          .worktree("pruned-conversation"),
        None,
        "the dangling binding was dropped"
      );
    });

    let _ = std::fs::remove_dir_all(&state_dir);
    cleanup_worktrees_root(&repo.path);
  }

  /// A second repo with one conversation already on disk, tracked by the hub
  /// through a fallback visit.
  fn seed_second_repo(name: &str, conversation_id: &str) -> (TempRepo, PathBuf) {
    let repo = TempRepo::init(name);
    commit_text_file(&repo.path, Path::new("README.md"), "other\n", "initial");
    let state_dir = agent_chat_state_dir()
      .map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &repo.path))
      .expect("agent chat state dir");
    let _ = std::fs::remove_dir_all(&state_dir);
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    let meta = serde_json::json!({
      "id": conversation_id,
      "started_at_secs": 1,
      "updated_at_secs": 2,
      "title": "In the other repo",
      "message_count": 1,
      "session_id": null,
      "preview": "hello"
    });
    std::fs::write(
      state_dir.join("index.json"),
      serde_json::json!({ "version": 1, "conversations": [meta.clone()] }).to_string(),
    )
    .expect("write index");
    std::fs::write(
      state_dir.join(format!("{conversation_id}.json")),
      serde_json::json!({
        "version": 1,
        "meta": meta,
        "items": [{ "type": "Message", "role": "User", "text": "hello", "images": 0 }],
        "group_pins": {},
        "auto_approve": false
      })
      .to_string(),
    )
    .expect("write transcript");
    (repo, state_dir)
  }

  #[gpui::test]
  async fn switching_repository_parks_a_running_session_without_stopping_it(
    cx: &mut TestAppContext,
  ) {
    let (repo, page, cx) = page_with_agent_panel("session-page-fallback-running", cx).await;
    let (other, other_state) =
      seed_second_repo("session-page-fallback-running-b", "b-conversation");

    let running = active_panel(&page, cx);
    running.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("busy", cx);
      panel.pretend_turn_in_flight_for_test(cx);
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("switch repository");
    });
    cx.run_until_parked();

    // You went to the other repo; the running session keeps working behind.
    active_panel(&page, cx).read_with(cx, |panel, _| {
      assert_eq!(panel.repo_root(), other.path.as_path());
    });
    page.read_with(cx, |page, cx| {
      assert!(
        page
          .background_chat_panels
          .iter()
          .any(|(_, panel)| panel.entity_id() == running.entity_id()),
        "the running session parked instead of stopping"
      );
      assert_eq!(
        page.dock_panel.read(cx).repo_root(),
        Some(other.path.as_path()),
        "the git surfaces follow where you went"
      );
    });
    running.read_with(cx, |panel, _| {
      assert!(panel.is_turn_in_flight(), "its turn never stopped");
      assert_eq!(panel.repo_root(), repo.path.as_path());
    });

    let _ = std::fs::remove_dir_all(&other_state);
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn the_sidebar_always_lists_every_tracked_repos_sessions(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-fallback-all", cx).await;
    let (other, other_state) =
      seed_second_repo("session-page-fallback-all-b", "all-b-conversation");

    let local = active_panel(&page, cx);
    local.update(cx, |panel, cx| panel.seed_user_message_for_test("mine", cx));
    cx.run_until_parked();
    let local_id = local.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    // Tracking the other repo is enough: no mode, no filter, one list.
    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("track the other repo");
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      let ids = page.session_list.read(cx).conversation_ids();
      assert!(ids.contains(&"all-b-conversation".to_string()));
      assert!(
        ids.contains(&local_id),
        "both repos' sessions share the one list"
      );
    });

    let _ = std::fs::remove_dir_all(&other_state);
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn selecting_another_repos_session_builds_it_on_its_own_repo(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-cross-select", cx).await;
    let (other, other_state) =
      seed_second_repo("session-page-cross-select-b", "cross-b-conversation");

    // Track the other repo, then come back: fallback = the first repo.
    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("track the other repo");
      page
        .set_fallback_repo(repo.path.clone(), window, cx)
        .expect("switch back to the first repo");
    });

    page.update_in(cx, |page, window, cx| {
      page.select_session("cross-b-conversation", window, cx)
    });
    cx.run_until_parked();

    let panel = active_panel(&page, cx);
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.current_conversation().id, "cross-b-conversation");
      assert_eq!(
        panel.repo_root(),
        other.path.as_path(),
        "the session runs in ITS repo, wherever the fallback points"
      );
      assert_eq!(panel.cwd(), other.path.as_path());
      assert_eq!(
        panel.transcript_texts(),
        vec!["hello".to_string()],
        "its transcript hydrated from its own store"
      );
    });
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.fallback_repo.as_deref(),
        Some(repo.path.as_path()),
        "the fallback did not move"
      );
      assert_eq!(
        page.dock_panel.read(cx).repo_root(),
        Some(other.path.as_path()),
        "the git surfaces follow the selected session"
      );
      assert_eq!(
        page.reviewed_repo.as_deref(),
        Some(other.path.as_path()),
        "the review batch follows the session's repo"
      );
    });

    let _ = std::fs::remove_dir_all(&other_state);
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn selecting_another_repos_session_loads_its_review_batch(cx: &mut TestAppContext) {
    use crate::agent_review::LocalAgentReviewComment;
    use editor::ReviewCommentSide;

    let (repo, page, cx) = page_with_agent_panel("session-page-cross-review", cx).await;
    let (other, other_state) = seed_second_repo("session-page-cross-review-b", "cross-review-b");
    std::fs::write(other.path.join("README.md"), "changed\n").expect("dirty the other repo");

    let state_dir = std::env::temp_dir().join(format!(
      "reviu-cross-review-{}-{:?}",
      std::process::id(),
      std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&state_dir);
    // A batch the other repo left behind in a previous run.
    write_review(
      &review_path_for_repo(&state_dir, &other.path),
      &[LocalAgentReviewComment {
        id: 4,
        in_reply_to_id: None,
        path: PathBuf::from("README.md"),
        line: 0,
        side: ReviewCommentSide::Right,
        start_line: None,
        start_side: None,
        body: std::sync::Arc::from("from the other repo"),
        original_start_line: Some(1),
        original_lines: vec!["other".to_string()],
        state: LocalAgentReviewCommentState::Draft,
      }],
      5,
    );
    page.update(cx, |page, _| {
      page.review_state_dir = Some(state_dir.clone());
      page.review_store_path = review_store_path_for(Some(&repo.path), Some(&state_dir));
    });

    // Track the other repo, then come back: fallback = the first repo.
    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("track the other repo");
      page
        .set_fallback_repo(repo.path.clone(), window, cx)
        .expect("switch back to the first repo");
    });

    page.update_in(cx, |page, window, cx| {
      page.select_session("cross-review-b", window, cx)
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert_eq!(
        page.review_store_path.as_deref(),
        Some(review_path_for_repo(&state_dir, &other.path).as_path()),
        "the batch loads and persists in the SESSION's repo, not the fallback"
      );
      let comments = page.agent_review.all();
      assert_eq!(comments.len(), 1);
      assert_eq!(comments[0].body.as_ref(), "from the other repo");
    });

    // A comment written here lands in the other repo's file, not the fallback's.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "on the other repo"), window, cx);
    });

    let stored =
      crate::agent_review_store::read_review(&review_path_for_repo(&state_dir, &other.path))
        .expect("the other repo's batch is on disk");
    assert_eq!(stored.comments.len(), 2);
    assert!(
      !review_path_for_repo(&state_dir, &repo.path).exists(),
      "nothing leaked into the fallback repo's batch"
    );

    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&other_state);
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn forgetting_a_repo_takes_only_its_sessions_and_replaces_the_shown_one(
    cx: &mut TestAppContext,
  ) {
    let (repo, page, cx) = page_with_agent_panel("session-page-forget-scoped", cx).await;
    let (other, other_state) = seed_second_repo("session-page-forget-scoped-b", "doomed-b");

    // A session of the first repo, parked in the background.
    let survivor = active_panel(&page, cx);
    survivor.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("keep me", cx)
    });
    cx.run_until_parked();
    let survivor_id = survivor.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    // The other repo's session on screen, while the fallback stays on repo A.
    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("track the other repo");
      page
        .set_fallback_repo(repo.path.clone(), window, cx)
        .expect("switch back");
      page.select_session("doomed-b", window, cx);
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository(other.path.clone(), window, cx)
        .expect("forget the other repo");
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      // The shown session died with its repo: a fresh fallback session took over
      // and the git surfaces left the dead checkout.
      let panel = page.agent_chat_view.as_ref().expect("a panel is shown");
      assert_eq!(panel.read(cx).repo_root(), repo.path.as_path());
      assert_eq!(
        page.dock_panel.read(cx).repo_root(),
        Some(repo.path.as_path())
      );
      // The first repo's session survived untouched.
      assert!(
        page
          .background_chat_panels
          .iter()
          .any(|(id, _)| id == &survivor_id),
        "the other repo's sessions were never touched"
      );
      let ids = page.session_list.read(cx).conversation_ids();
      assert!(!ids.contains(&"doomed-b".to_string()));
    });

    let _ = std::fs::remove_dir_all(&other_state);
    cleanup_worktrees_root(&repo.path);
  }

  fn notifications(cx: &mut gpui::VisualTestContext) -> Vec<gpui::Entity<Notification>> {
    cx.update(|window, cx| {
      gpui_component::Root::read(window, cx)
        .notification
        .read(cx)
        .notifications()
        .to_vec()
    })
  }

  #[gpui::test]
  async fn the_hub_evicts_an_old_store_instead_of_refusing_the_ninth_repo(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-hub-cap", cx).await;
    ConfigStore::persist_recent_repository(&repo.path);

    let mut extra_repos = Vec::new();
    for index in 0..crate::conversation_hub::MAX_TRACKED_REPOS - 1 {
      let extra = TempRepo::init(&format!("session-page-hub-cap-{index}"));
      commit_text_file(&extra.path, Path::new("README.md"), "v1\n", "initial");
      page.update_in(cx, |page, window, cx| {
        page
          .set_fallback_repo(extra.path.clone(), window, cx)
          .expect("switch to the extra repo");
      });
      page.read_with(cx, |page, _| {
        assert!(
          page.chat_store.is_some(),
          "repo {index}: the fallback always gets a store, cap or not"
        );
      });
      extra_repos.push(extra);
    }
    assert!(notifications(cx).is_empty());

    let overflow = TempRepo::init("session-page-hub-cap-overflow");
    commit_text_file(&overflow.path, Path::new("README.md"), "v1\n", "initial");
    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(overflow.path.clone(), window, cx)
        .expect("switch to the overflow repo");
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert!(
        page.chat_store.is_some(),
        "the overflow repo still gets a store"
      );
    });
    assert_eq!(notifications(cx).len(), 1);
    assert!(
      ConfigStore::load_recent_repositories()
        .into_iter()
        .any(|recent| recent.path == repo.path),
      "the hidden repo remains in Recent Repositories"
    );
    extra_repos.push(overflow);
  }

  #[gpui::test]
  async fn a_cross_repo_worktree_session_resolves_its_binding_from_its_own_store(
    cx: &mut TestAppContext,
  ) {
    let (repo, page, cx) = page_with_agent_panel("session-page-cross-worktree", cx).await;
    let (other, other_state) =
      seed_second_repo("session-page-cross-worktree-b", "worktree-b-conversation");
    // The other repo's conversation is bound to a real worktree of ITS repo.
    let worktree = git::create_worktree(&other.path, None).expect("worktree of the other repo");
    std::fs::write(
      other_state.join("worktrees.json"),
      serde_json::json!({
        "worktree-b-conversation": { "path": worktree.path, "branch": worktree.branch }
      })
      .to_string(),
    )
    .expect("write bindings");

    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("track the other repo");
      page
        .set_fallback_repo(repo.path.clone(), window, cx)
        .expect("switch back");
      page.select_session("worktree-b-conversation", window, cx);
    });
    cx.run_until_parked();

    active_panel(&page, cx).read_with(cx, |panel, _| {
      assert_eq!(
        panel.repo_root(),
        other.path.as_path(),
        "the session belongs to its own repo"
      );
      assert_eq!(
        panel.cwd(),
        worktree.path.as_path(),
        "and works in ITS worktree, resolved from ITS store"
      );
    });
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.dock_panel.read(cx).repo_root(),
        Some(worktree.path.as_path()),
        "the git surfaces follow the cross-repo worktree"
      );
    });

    let _ = std::fs::remove_dir_all(&other_state);
    cleanup_worktrees_root(&other.path);
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn an_emptied_repo_keeps_its_section_and_its_compose_button(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-empty-section", cx).await;

    let panel = active_panel(&page, cx);
    panel.update(cx, |panel, cx| {
      panel.seed_user_message_for_test("only one", cx)
    });
    cx.run_until_parked();
    let id = panel.read_with(cx, |panel, _| panel.current_conversation().id.clone());

    page.update_in(cx, |page, window, cx| page.delete_session(&id, window, cx));
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let list = page.session_list.read(cx);
      assert!(list.conversation_ids().is_empty(), "no rows left");
      assert!(
        list
          .section_order_for_test()
          .iter()
          .any(|section| section == &repo.path),
        "the emptied repo keeps its section header"
      );
    });

    // And the section's compose button still works: creating there is
    // possible without any surviving row.
    page.update_in(cx, |page, window, cx| {
      page.new_session_in(repo.path.clone(), window, cx)
    });
    cx.run_until_parked();
    active_panel(&page, cx).read_with(cx, |panel, _| {
      assert_eq!(panel.repo_root(), repo.path.as_path());
    });

    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn a_new_session_lands_where_you_are(cx: &mut TestAppContext) {
    let (repo, page, cx) = page_with_agent_panel("session-page-new-in-context", cx).await;
    let (other, other_state) = seed_second_repo("session-page-new-in-context-b", "seed-b");

    // Content first, so New Session forks instead of reusing the blank.
    active_panel(&page, cx).update(cx, |panel, cx| panel.seed_user_message_for_test("mine", cx));
    cx.run_until_parked();

    // The fallback moves to the other repo, but the SHOWN session stays in the
    // first one: the plain New Session follows what you are looking at.
    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("fallback to the other repo");
      page.select_session(
        &page
          .conversation_hub
          .sections(cx)
          .iter()
          .find(|(section_repo, _)| section_repo == &repo.path)
          .and_then(|(_, metas)| metas.first().cloned())
          .expect("the first repo's conversation")
          .id
          .clone(),
        window,
        cx,
      );
      page.new_session(window, cx);
    });
    cx.run_until_parked();
    active_panel(&page, cx).read_with(cx, |panel, _| {
      assert_eq!(
        panel.repo_root(),
        repo.path.as_path(),
        "a new session lands in the shown session's repo"
      );
    });

    // The section header's compose button targets ITS repo explicitly.
    page.update_in(cx, |page, window, cx| {
      page.new_session_in(other.path.clone(), window, cx);
    });
    cx.run_until_parked();
    active_panel(&page, cx).read_with(cx, |panel, _| {
      assert_eq!(
        panel.repo_root(),
        other.path.as_path(),
        "the per-section compose creates in that section's repo"
      );
    });

    let _ = std::fs::remove_dir_all(&other_state);
    cleanup_worktrees_root(&repo.path);
  }

  #[gpui::test]
  async fn a_blank_session_never_stacks_parked_panels(cx: &mut TestAppContext) {
    let (_repo, page, cx) = page_with_agent_panel("session-page-blank-reuse", cx).await;

    // Repeated New Session on a blank conversation piles nothing up: no
    // parked panel, no row on disk. (The entity itself may be rebuilt when
    // its connection is dead, as it always is under this fixture.)
    for _ in 0..3 {
      page.update_in(cx, |page, window, cx| page.new_session(window, cx));
      cx.run_until_parked();
    }
    page.read_with(cx, |page, cx| {
      assert!(page.background_chat_panels.is_empty());
      let store = page.chat_store.as_ref().expect("store").read(cx);
      assert!(store.list().is_empty(), "nothing blank was persisted");
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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
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
