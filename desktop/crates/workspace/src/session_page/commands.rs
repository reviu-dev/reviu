//! Git commands the shell runs: one-shots, branch moves and interactive rebase.

use super::*;

impl SessionPage {
  /// Staging a conflicted file marks its conflict resolved, which deserves a
  /// question before it happens.
  pub(super) fn stage_all_with_confirmation(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let entries = self.dock_panel.read(cx).status_entries().to_vec();
    if !crate::changes_list::has_conflicted_entries(&entries) {
      return self.run_repo_command(RepoCommand::StageAll, window, cx);
    }

    let view = cx.entity();
    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      ConfirmDialog::new(
        SharedString::from("Mark conflicts as resolved?"),
        div().child("Stage all files and mark their merge conflicts as resolved?"),
      )
      .confirm_text("Stage all")
      .cancel_text("Cancel")
      .on_confirm(move |_, window, cx| {
        view.update(cx, |view, cx| {
          if let Err(error) = view.run_repo_command(RepoCommand::StageAll, window, cx) {
            window.push_notification(Notification::warning(error), cx);
          }
        });
        true
      })
      .build(alert)
    });
    Ok(())
  }

  /// Pushing to GitHub without Pro is the one moment the teaser is relevant.
  pub(super) fn maybe_show_pro_teaser(&mut self, cx: &mut Context<Self>) {
    if !pro_teaser::should_show_after_push(
      self.pro_teaser_shown,
      AuthStateStore::has_github_access(cx),
      true,
    ) {
      return;
    }
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };

    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let has_github_remote = cx
        .background_spawn(async move {
          git::current_github_remote_repo(&repo_root)
            .ok()
            .flatten()
            .is_some()
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        if !pro_teaser::should_show_after_push(
          this.pro_teaser_shown,
          AuthStateStore::has_github_access(cx),
          has_github_remote,
        ) {
          return;
        }
        this.pro_teaser_shown = true;
        pro_teaser::show_after_push(window_handle, cx);
      });
    });
    self._pro_teaser_task = Some(task);
  }

  /// A shortcut runs its command only when the palette would have offered it.
  pub(super) fn run_shortcut_command(
    &mut self,
    rule: PaletteCommand,
    command: RepoCommand,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    cx.stop_propagation();
    if !self.repo_state("", cx).allows(rule) {
      return;
    }
    if let Err(error) = self.run_repo_command(command, window, cx) {
      window.push_notification(Notification::warning(error), cx);
    }
  }

  /// Runs a repo command in the background, then refreshes the changes panel.
  /// The shell tracks no operation in progress: it never starts a merge or a rebase.
  pub(super) fn repo_state<'a>(&'a self, commit_message: &'a str, cx: &'a App) -> RepoState<'a> {
    let branch_status = self.branch_status.as_ref();
    let (can_push, can_force_push) = push_flags(branch_status, branch_status.is_some(), false);
    let panel = self.dock_panel.read(cx);
    let status_entries = panel.status_entries();
    RepoState {
      has_repo: self.selected_repo.is_some(),
      merge_in_progress: panel.merge_in_progress(),
      rebase_in_progress: panel.rebase_in_progress(),
      has_head_commit: panel.head_status().has_head_commit,
      can_push,
      can_force_push,
      can_undo_last_commit: panel.head_status().can_undo_last_commit,
      branch_status,
      status_entries,
      selected_entry: self
        .selected_file
        .as_deref()
        .and_then(|path| status_entries.iter().find(|entry| entry.path == path)),
      commit_message,
    }
  }

  pub(super) fn start_interactive_rebase(
    &mut self,
    target: InteractiveRebaseTarget,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(repo_root) = self.selected_repo.clone() else {
      return Err("No repository selected.".into());
    };
    if !self
      .repo_state("", cx)
      .allows(PaletteCommand::InteractiveRebase)
    {
      return Err("Interactive rebase is currently disabled.".into());
    }

    let preview = interactive_rebase::prepare_commits(&repo_root, &target)?;
    let commits = preview.commits;
    let Some(dropped) = interactive_rebase::dropped_merges_message(preview.dropped_merge_count)
    else {
      self.open_interactive_rebase_todo(target, commits, window, cx);
      return Ok(());
    };

    // Losing merge commits is the user's call, not ours.
    let view = cx.entity();
    window.on_next_frame(move |window, cx| {
      let view = view.clone();
      let target = target.clone();
      let commits = commits.clone();
      let dropped = dropped.clone();
      window.open_alert_dialog(cx, move |alert, _, _| {
        let view = view.clone();
        let target = target.clone();
        let commits = commits.clone();
        ConfirmDialog::new(
          SharedString::from("Drop merge commits?"),
          div().child(dropped.clone()),
        )
        .confirm_text("Continue")
        .cancel_text("Cancel")
        .on_confirm(move |_, window, cx| {
          let target = target.clone();
          let commits = commits.clone();
          view.update(cx, move |view, cx| {
            view.open_interactive_rebase_todo(target, commits, window, cx);
          });
          true
        })
        .build(alert)
      });
    });
    Ok(())
  }

  pub(super) fn open_interactive_rebase_todo(
    &mut self,
    target: InteractiveRebaseTarget,
    commits: Vec<git::InteractiveRebaseCommit>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let view_for_submit = cx.entity();
    let on_submit: InteractiveRebaseTodoViewHandler =
      Arc::new(move |target, todo_entries, window, cx| {
        view_for_submit.update(cx, |view, cx| {
          view.apply_interactive_rebase(target, todo_entries, window, cx)
        })
      });
    let view_for_cancel = cx.entity();
    let on_cancel: InteractiveRebaseTodoViewCancelHandler = Arc::new(move |window, cx| {
      view_for_cancel.update(cx, |view, cx| {
        view.close_interactive_rebase_todo(window, cx);
      });
    });

    let config = InteractiveRebaseTodoViewConfig::new(target, commits, on_submit, on_cancel);
    let todo_view = cx.new(|cx| InteractiveRebaseTodoView::new(window, cx, config));
    self.interactive_rebase_todo_view = Some(todo_view.clone());
    self.center = CenterView::InteractiveRebase;
    cx.on_next_frame(window, move |_, window, cx| {
      todo_view.update(cx, |view, cx| view.focus_rows_list(window, cx));
    });
    cx.notify();
  }

  pub(super) fn close_interactive_rebase_todo(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.interactive_rebase_todo_view = None;
    self.center = if self.editor.is_some() {
      CenterView::Diff
    } else {
      CenterView::Conversation
    };
    self.focus_editor_on_next_frame(window, cx);
    cx.notify();
  }

  pub(super) fn apply_interactive_rebase(
    &mut self,
    target: InteractiveRebaseTarget,
    todo_entries: Vec<git::InteractiveRebaseTodoEntry>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(repo_root) = self.selected_repo.clone() else {
      return Err("No repository selected.".into());
    };
    self.close_interactive_rebase_todo(window, cx);

    let success_message = interactive_rebase::success_message(&target);
    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let run_repo_root = repo_root.clone();
      let result = cx
        .background_spawn(async move {
          git::start_interactive_rebase(&run_repo_root, &target, &todo_entries)
        })
        .await;
      let stopped_on_conflict = git::is_rebase_in_progress(&repo_root).unwrap_or(false);
      let conflicted_path = crate::repo_command::first_conflicted_path(&repo_root);
      let rebase_message = stopped_on_conflict
        .then(|| {
          git::current_rebase_commit_message(&repo_root)
            .ok()
            .flatten()
        })
        .flatten();

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          match (&result, stopped_on_conflict) {
            (Ok(()), false) => {
              window.push_notification(Notification::success(success_message.clone()), cx);
              this
                .dock_panel
                .update(cx, |panel, cx| panel.set_commit_message("", window, cx));
            }
            // A rebase that stopped on a conflict is not a failure.
            (Err(error), false) => {
              window.push_notification(
                Notification::error(format!("Interactive rebase failed: {error}")),
                cx,
              );
            }
            _ => {}
          }
          if let Some(message) = rebase_message {
            this.dock_panel.update(cx, |panel, cx| {
              panel.set_commit_message(&message, window, cx)
            });
          }
          this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
          this.refresh_branch(cx);
          if let Some(path) = conflicted_path {
            this.open_diff(path, None, window, cx);
          }
          cx.notify();
        });
      });
    });
    self._interactive_rebase_task = Some(task);
    Ok(())
  }

  pub(super) fn run_commit_menu_command(
    &mut self,
    command: CommitMenuCommand,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match command {
      CommitMenuCommand::Amend => self.amend_last_commit(window, cx),
      CommitMenuCommand::UndoLastCommit => {
        self.run_repo_command(RepoCommand::UndoLastCommit, window, cx)
      }
      CommitMenuCommand::Push => self.run_repo_command(RepoCommand::Push, window, cx),
      CommitMenuCommand::ForcePush => self.run_repo_command(RepoCommand::ForcePush, window, cx),
    }
  }

  /// Amending takes the message in the commit box, or keeps the old one when
  /// the box is empty.
  pub(super) fn amend_last_commit(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let message = self.dock_panel.read(cx).commit_message(cx);
    let message = message.trim().to_string();
    let command = RepoCommand::Amend {
      message: (!message.is_empty()).then_some(message),
    };
    let started = self.run_repo_command(command, window, cx);
    if started.is_ok() {
      self
        .dock_panel
        .update(cx, |panel, cx| panel.set_commit_message("", window, cx));
    }
    started
  }

  /// Moving HEAD under a running agent breaks its turn: the branch waits.
  pub(super) fn run_branch_command(
    &mut self,
    command: RepoCommand,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    if self.agent_turn_in_flight(cx) {
      return Err("Wait for the agent to finish before switching branch.".into());
    }
    self.run_repo_command(command, window, cx)
  }

  pub(super) fn selected_status_entry(&self, cx: &App) -> Option<git::RepoStatusEntry> {
    let path = self.selected_file.as_deref()?;
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .find(|entry| entry.path == path)
      .cloned()
  }

  pub(super) fn stage_selected_file(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(entry) = self.selected_status_entry(cx) else {
      return Err("No file selected.".into());
    };
    // A conflicted file with markers left asks before being marked resolved.
    let has_markers = self.editor.as_ref().is_none_or(|editor| {
      editor.read_with(cx, |editor, cx| editor.has_unresolved_conflict_markers(cx))
    });
    let changes_list = self.dock_panel.read(cx).changes_list();
    changes_list.update(cx, |list, cx| {
      list.set_open_file_has_conflict_markers(has_markers);
      list.stage_file_with_confirmation(entry.path.clone(), entry.status, window, cx);
    });
    Ok(())
  }

  pub(super) fn unstage_selected_file(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(entry) = self.selected_status_entry(cx) else {
      return Err("No file selected.".into());
    };
    let changes_list = self.dock_panel.read(cx).changes_list();
    changes_list.update(cx, |list, cx| {
      list.unstage_file(entry.path.clone(), window, cx)
    });
    Ok(())
  }

  pub(super) fn start_merge_base_branch(
    &mut self,
    repo_root: PathBuf,
    base_branch_name: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_repo.as_deref() != Some(repo_root.as_path())
      && let Err(error) = self.set_selected_repo(repo_root.clone(), window, cx)
    {
      window.push_notification(Notification::warning(error), cx);
      return;
    }

    // A conflict is already waiting: resume it instead of starting another merge.
    if let Some(path) = crate::repo_command::first_conflicted_path(&repo_root) {
      self.open_diff(path, None, window, cx);
      return;
    }

    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let fetch_root = repo_root.clone();
      let branch_name = base_branch_name.clone();
      let resolved = cx
        .background_spawn(async move {
          git::fetch(&fetch_root)?;
          git::resolve_branch_ref(&fetch_root, &branch_name)?.ok_or_else(|| {
            anyhow::anyhow!("branch {branch_name:?} was not found locally or on any remote")
          })
        })
        .await;

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| match resolved {
          Ok(branch) => {
            if let Err(error) = this.run_repo_command(RepoCommand::MergeBranch(branch), window, cx)
            {
              window.push_notification(Notification::warning(error), cx);
            }
          }
          Err(error) => {
            window.push_notification(Notification::error(error.to_string()), cx);
          }
        });
      });
    });
    self._merge_base_task = Some(task);
  }

  pub(super) fn run_repo_command(
    &mut self,
    command: RepoCommand,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(repo_root) = self.selected_repo.clone() else {
      return Err("No repository selected.".into());
    };
    if self.repo_command_in_flight {
      return Err("Another git command is still running.".into());
    }

    self.repo_command_in_flight = true;
    let pushed = matches!(command, RepoCommand::Push | RepoCommand::ForcePush);
    let telemetry_key = command.telemetry_key();
    let analytics_event = command.analytics_event();
    self
      .git_telemetry(cx)
      .breadcrumb(command.label(), Map::new());
    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move { command.run(&repo_root) })
        .await;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.repo_command_in_flight = false;
          this
            .git_telemetry(cx)
            .report_outcome(telemetry_key, git_telemetry::outcome_report(&result));
          match result {
            Ok(outcome) => {
              match outcome {
                RepoCommandOutcome::Done { message } => {
                  window.push_notification(Notification::success(message), cx);
                }
                RepoCommandOutcome::UpToDate { message } => {
                  window.push_notification(Notification::info(message), cx);
                }
                RepoCommandOutcome::Conflicted {
                  path,
                  commit_message,
                  ..
                } => {
                  window.push_notification(
                    Notification::warning(format!("Resolve the conflicts in {}", path.display())),
                    cx,
                  );
                  if let Some(message) = commit_message {
                    this.dock_panel.update(cx, |panel, cx| {
                      panel.set_commit_message(&message, window, cx)
                    });
                  }
                  this.open_diff(path, None, window, cx);
                }
              }
              if let Some(event) = analytics_event {
                crate::analytics::track(cx, event);
              }
              this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
              this.refresh_branch(cx);
              this.open_pending_pull_request_dialog(window, cx);
              if pushed {
                this.maybe_show_pro_teaser(cx);
              }
            }
            Err(error) => {
              this.pending_pull_request = None;
              window.push_notification(Notification::error(error.to_string()), cx);
            }
          }
          cx.notify();
        });
      });
    });
    self._repo_command_task = Some(task);

    Ok(())
  }

  /// GitHub needs the branch on the remote before it can open a pull request,
  /// so the push comes first and the form only follows if it worked.
  pub(super) fn publish_branch_and_create_pull_request(
    &mut self,
    context: GithubBranchContext,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.pending_pull_request = Some(context);
    if let Err(error) = self.run_repo_command(RepoCommand::Push, window, cx) {
      self.pending_pull_request = None;
      window.push_notification(Notification::warning(error), cx);
    }
  }

  pub(super) fn open_pending_pull_request_dialog(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(context) = self.pending_pull_request.take() else {
      return;
    };
    let panel = self.dock_panel.clone();
    open_create_pull_request_dialog(
      WorkspaceApi::global(cx).api.clone(),
      self.window_handle,
      Rc::new(move |_context, _pull_request, cx| {
        panel.update(cx, |panel, cx| panel.refresh(cx));
      }),
      context,
      window,
      cx,
    );
  }
}
