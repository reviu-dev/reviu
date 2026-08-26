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
    let branch_status = self.repo_snapshot.read(cx).branch_status();
    let (can_push, can_force_push) = push_flags(branch_status, branch_status.is_some(), false);
    let panel = self.dock_panel.read(cx);
    let status_entries = panel.status_entries();
    RepoState {
      has_repo: self.fallback_repo.is_some(),
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
    let Some(repo_root) = self.checkout_root(cx) else {
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
    let Some(repo_root) = self.checkout_root(cx) else {
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
            this.open_diff(path, None, OpenIntent::Open, window, cx);
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
  /// Only a turn in THIS checkout counts; worktree sessions are isolated.
  pub(super) fn run_branch_command(
    &mut self,
    command: RepoCommand,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    if let Some(checkout) = self.checkout_root(cx)
      && self.agent_turn_in_flight_at(&checkout, cx)
    {
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

  pub(super) fn run_repo_command(
    &mut self,
    command: RepoCommand,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(repo_root) = self.checkout_root(cx) else {
      return Err("No repository selected.".into());
    };
    if self.repo_command_in_flight {
      return Err("Another git command is still running.".into());
    }

    self.repo_command_in_flight = true;
    let checked_out_for_link = matches!(command, RepoCommand::SwitchToBranchName { .. });
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
                  if let Some(message) = message {
                    window.push_notification(Notification::success(message), cx);
                  }
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
                  this.open_diff(path, None, OpenIntent::Open, window, cx);
                }
              }
              if let Some(event) = analytics_event {
                crate::analytics::track(cx, event);
              }
              this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
              this.refresh_branch(cx);
              this.open_pending_pull_request_dialog(window, cx);
            }
            Err(error) => {
              this.pending_pull_request = None;
              if checked_out_for_link {
                crate::pull_request_surface::PullRequestSurfaceHandle::forget(cx);
              }
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

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;
  use ui::CommandPaletteCommandId;

  #[gpui::test]
  async fn staging_says_nothing_while_a_stash_still_speaks(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-stage-quiet");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("dirty the worktree");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);

    run_command(&page, RepoCommand::StageAll, cx).await;
    assert!(
      notifications(cx).is_empty(),
      "the Changes panel already shows the file move"
    );
    let staged = git::list_repo_status(&repo.path).expect("status");
    assert!(
      staged
        .iter()
        .all(|entry| entry.stage == git::RepoStage::Staged)
    );

    run_command(&page, RepoCommand::UnstageAll, cx).await;
    assert!(notifications(cx).is_empty(), "and the move back");
    let unstaged = git::list_repo_status(&repo.path).expect("status");
    assert!(
      unstaged
        .iter()
        .all(|entry| entry.stage == git::RepoStage::Unstaged)
    );

    // A command whose result leaves no trace on screen still says so.
    run_command(
      &page,
      RepoCommand::Stash {
        include_untracked: false,
        message: None,
      },
      cx,
    )
    .await;
    assert!(
      !notifications(cx).is_empty(),
      "stashed work is gone from the panel with nothing to explain it"
    );
  }

  async fn run_command(
    page: &Entity<SessionPage>,
    command: RepoCommand,
    cx: &mut gpui::VisualTestContext,
  ) {
    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(command, window, cx)
        .expect("the command runs")
    });
    let task = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    task.await;
    cx.run_until_parked();
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
  async fn amending_and_undoing_reach_the_last_commit(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-amend");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      // The head status travels from the dock into the rules.
      assert!(page.dock_panel.read(cx).head_status().has_head_commit);
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::Amend));
      assert!(ids.contains(&CommandPaletteCommandId::UndoLastCommit));
      assert!(ids.contains(&CommandPaletteCommandId::CheckoutDetached));
    });

    // Amend takes the message in the box and rewords the last commit.
    page.update_in(cx, |page, window, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_commit_message("second, reworded", window, cx)
      });
      page
        .handle_command_palette_action(CommandPaletteAction::Amend, window, cx)
        .expect("amend runs")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    let history = git::list_commit_history(&repo.path, 10).expect("history");
    assert_eq!(history.len(), 2, "the commit was rewritten, not added to");
    assert_eq!(history[0].summary, "second, reworded");
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.dock_panel.read(cx).commit_message(cx),
        "",
        "the box is cleared once the message landed in the commit"
      );
    });

    // Undo puts the work back in the worktree.
    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::UndoLastCommit, window, cx)
        .expect("undo runs")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert_eq!(
      git::list_commit_history(&repo.path, 10)
        .expect("history")
        .len(),
      1
    );
    assert!(repo.path.join("b.txt").exists());
  }

  #[gpui::test]
  async fn the_branch_and_stash_commands_do_what_they_say(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-branch-commands");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let base = git::current_branch_status(&repo.path)
      .expect("branch status")
      .name;

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    // Git prepares a stash message from HEAD; the palette offers it.
    page.read_with(cx, |page, cx| {
      assert!(
        page
          .repo_snapshot
          .read(cx)
          .default_stash_message()
          .is_some(),
        "the stash screen starts with a message"
      );
    });

    let run = |page: &Entity<SessionPage>,
               cx: &mut gpui::VisualTestContext,
               action: CommandPaletteAction| {
      page.update_in(cx, |page, window, cx| {
        page
          .handle_command_palette_action(action, window, cx)
          .expect("the command runs")
      });
      page.update(cx, |page, _| page._repo_command_task.take())
    };

    // Creating a branch switches to it.
    let task = run(
      &page,
      cx,
      CommandPaletteAction::CreateBranch {
        name: "feature".to_string(),
      },
    );
    task.expect("command task").await;
    cx.run_until_parked();
    assert_eq!(
      git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      "feature"
    );

    // A commit only on this branch, so the base actually matters below.
    commit_text_file(
      &repo.path,
      Path::new("only-feature.txt"),
      "x\n",
      "feature only",
    );

    // Creating from a base starts there, whatever branch we are on.
    let task = run(
      &page,
      cx,
      CommandPaletteAction::CreateBranchFrom {
        name: "from-base".to_string(),
        base: ui::CommandPaletteBranch {
          name: base.clone().into(),
          kind: ui::CommandPaletteBranchKind::Local,
        },
      },
    );
    task.expect("command task").await;
    cx.run_until_parked();
    assert_eq!(
      git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      "from-base"
    );
    assert!(
      !repo.path.join("only-feature.txt").exists(),
      "the new branch starts at the base, not at the branch we were on"
    );

    // Deleting the branch we left behind.
    let task = run(
      &page,
      cx,
      CommandPaletteAction::DeleteBranch(ui::CommandPaletteBranch {
        name: "feature".into(),
        kind: ui::CommandPaletteBranchKind::Local,
      }),
    );
    task.expect("command task").await;
    cx.run_until_parked();
    assert!(
      !git::list_branches(&repo.path)
        .expect("branches")
        .iter()
        .any(|branch| branch.name == "feature")
    );

    // Stashing from the palette puts the change aside.
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");
    let task = run(
      &page,
      cx,
      CommandPaletteAction::Stash {
        include_untracked: false,
        message: Some("from the palette".to_string()),
      },
    );
    task.expect("command task").await;
    cx.run_until_parked();
    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v1\n"
    );
    assert_eq!(git::list_stashes(&repo.path).expect("stashes").len(), 1);

    // The command triggered a status refresh; let it finish before touching git
    // again, or the index lock and the test race.
    let refresh = page.update(cx, |page, cx| {
      page
        .dock_panel
        .update(cx, |panel, _| panel._refresh_task.take())
    });
    if let Some(refresh) = refresh {
      refresh.await;
    }
    cx.run_until_parked();

    // Cherry-picking a commit from the base branch.
    let base_ref = git::BranchRef {
      name: base.clone(),
      kind: git::BranchKind::Local,
    };
    git::switch_branch(&repo.path, &base_ref).expect("switch to base");
    commit_text_file(&repo.path, Path::new("b.txt"), "picked\n", "pick me");
    let picked = git::current_head_sha(&repo.path)
      .expect("head sha")
      .expect("head sha");
    git::switch_branch(
      &repo.path,
      &git::BranchRef {
        name: "from-base".to_string(),
        kind: git::BranchKind::Local,
      },
    )
    .expect("switch back");

    let task = run(
      &page,
      cx,
      CommandPaletteAction::CherryPick {
        commit_hashes: vec![picked],
      },
    );
    task.expect("command task").await;
    cx.run_until_parked();
    assert!(repo.path.join("b.txt").exists());
  }

  #[gpui::test]
  async fn a_branch_switch_waits_for_the_agent(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-branch-switch-guard");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let base = git::current_branch_status(&repo.path)
      .expect("branch status")
      .name;
    git::create_branch(&repo.path, "feature").expect("create branch");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update(cx, |page, _| page.pretend_agent_turn_in_flight = true);

    let refused = page.update_in(cx, |page, window, cx| {
      page.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(ui::CommandPaletteBranch {
          name: "feature".into(),
          kind: ui::CommandPaletteBranchKind::Local,
        }),
        window,
        cx,
      )
    });
    assert_eq!(
      refused.expect_err("refused mid-turn").as_ref(),
      "Wait for the agent to finish before switching branch."
    );

    // Creating a branch switches to it, so it waits too.
    let refused = page.update_in(cx, |page, window, cx| {
      page.handle_command_palette_action(
        CommandPaletteAction::CreateBranch {
          name: "another".to_string(),
        },
        window,
        cx,
      )
    });
    assert_eq!(
      refused.expect_err("refused mid-turn").as_ref(),
      "Wait for the agent to finish before switching branch."
    );
    assert!(
      !git::list_branches(&repo.path)
        .expect("branches")
        .iter()
        .any(|branch| branch.name == "another")
    );
    assert_eq!(
      git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      base,
      "the branch did not move under the agent"
    );

    // Turn over: the switch goes through.
    page.update(cx, |page, _| page.pretend_agent_turn_in_flight = false);
    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(
          CommandPaletteAction::SwitchBranch(ui::CommandPaletteBranch {
            name: "feature".into(),
            kind: ui::CommandPaletteBranchKind::Local,
          }),
          window,
          cx,
        )
        .expect("switch runs")
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
      "feature"
    );
  }

  #[gpui::test]
  async fn the_commit_menu_runs_what_it_names(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-commit-menu");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    // An empty box keeps the message of the commit being amended.
    page.update_in(cx, |page, window, cx| {
      page
        .run_commit_menu_command(CommitMenuCommand::Amend, window, cx)
        .expect("amend runs")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    let history = git::list_commit_history(&repo.path, 10).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(
      history[0].summary, "second",
      "an empty box keeps the old message"
    );

    // The Undo entry undoes, it does not push.
    page.update_in(cx, |page, window, cx| {
      page
        .run_commit_menu_command(CommitMenuCommand::UndoLastCommit, window, cx)
        .expect("undo runs")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert_eq!(
      git::list_commit_history(&repo.path, 10)
        .expect("history")
        .len(),
      1
    );
  }

  #[gpui::test]
  async fn the_selected_file_is_staged_and_unstaged_from_the_palette(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-selected-file-stage");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    // Nothing selected: the commands stay out of the palette.
    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(!ids.contains(&CommandPaletteCommandId::StageSelectedFile));
      assert!(!ids.contains(&CommandPaletteCommandId::UnstageSelectedFile));
    });

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, OpenIntent::Open, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::StageSelectedFile));
      assert!(!ids.contains(&CommandPaletteCommandId::UnstageSelectedFile));
    });

    let stage = page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::StageSelectedFile, window, cx)
        .expect("stage the selected file");
      page
        .dock_panel
        .read(cx)
        .changes_list()
        .update(cx, |list, _| list._action_task.take())
    });
    stage.expect("staging task").await;
    cx.run_until_parked();
    let entries = git::list_repo_status(&repo.path).expect("status");
    assert_eq!(entries[0].stage, git::RepoStage::Staged);

    let unstage = page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::UnstageSelectedFile, window, cx)
        .expect("unstage the selected file");
      page
        .dock_panel
        .read(cx)
        .changes_list()
        .update(cx, |list, _| list._action_task.take())
    });
    unstage.expect("unstaging task").await;
    cx.run_until_parked();
    let entries = git::list_repo_status(&repo.path).expect("status");
    assert_eq!(entries[0].stage, git::RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn an_interactive_rebase_runs_from_the_center(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-interactive-rebase");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");
    commit_text_file(&repo.path, Path::new("c.txt"), "v1\n", "third");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::InteractiveRebase));
    });

    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(
          CommandPaletteAction::InteractiveRebaseHeadCount { count: 2 },
          window,
          cx,
        )
        .expect("the todo opens")
    });
    cx.run_until_parked();

    let commits = page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::InteractiveRebase);
      assert!(page.interactive_rebase_todo_view.is_some());
      git::list_interactive_rebase_commits(&repo.path, &InteractiveRebaseTarget::HeadCount(2))
        .expect("preview")
        .commits
    });
    assert_eq!(commits.len(), 2);

    // Drop the last commit, keep the other.
    let todo = vec![
      git::InteractiveRebaseTodoEntry {
        oid: commits[0].oid.clone(),
        action: git::InteractiveRebaseAction::Pick,
      },
      git::InteractiveRebaseTodoEntry {
        oid: commits[1].oid.clone(),
        action: git::InteractiveRebaseAction::Drop,
      },
    ];
    page.update_in(cx, |page, window, cx| {
      page
        .apply_interactive_rebase(InteractiveRebaseTarget::HeadCount(2), todo, window, cx)
        .expect("the rebase starts")
    });
    let task = page.update(cx, |page, _| {
      page._interactive_rebase_task.take().expect("rebase task")
    });
    task.await;
    cx.run_until_parked();

    // The todo left the center, and the dropped commit is gone.
    page.read_with(cx, |page, _| {
      assert!(page.interactive_rebase_todo_view.is_none());
      assert_ne!(page.center, CenterView::InteractiveRebase);
    });
    let summaries = git::list_commit_history(&repo.path, 10)
      .expect("history")
      .into_iter()
      .map(|commit| commit.summary)
      .collect::<Vec<_>>();
    assert!(!summaries.contains(&"third".to_string()));
    assert!(summaries.contains(&"second".to_string()));
  }

  #[gpui::test]
  async fn rebasing_onto_a_branch_stops_on_the_conflict(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-interactive-rebase-branch");
    let base = start_conflicting_rebase_setup(&repo.path);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    // The palette offers the other branches, never the one we are on.
    page.read_with(cx, |page, cx| {
      let targets = page.rebase_branch_targets(cx);
      assert!(targets.iter().any(|branch| branch.name.as_ref() == base));
      assert!(
        !targets
          .iter()
          .any(|branch| branch.name.as_ref() == "feature")
      );
    });

    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(
          CommandPaletteAction::InteractiveRebaseBranch {
            name: ui::CommandPaletteBranch {
              name: base.clone().into(),
              kind: ui::CommandPaletteBranchKind::Local,
            },
          },
          window,
          cx,
        )
        .expect("the todo opens")
    });
    cx.run_until_parked();

    let commits = page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::InteractiveRebase);
      git::list_interactive_rebase_commits(
        &repo.path,
        &InteractiveRebaseTarget::Branch(git::BranchRef {
          name: base.clone(),
          kind: git::BranchKind::Local,
        }),
      )
      .expect("preview")
      .commits
    });

    let todo = commits
      .iter()
      .map(|commit| git::InteractiveRebaseTodoEntry {
        oid: commit.oid.clone(),
        action: git::InteractiveRebaseAction::Pick,
      })
      .collect::<Vec<_>>();
    page.update_in(cx, |page, window, cx| {
      page
        .apply_interactive_rebase(
          InteractiveRebaseTarget::Branch(git::BranchRef {
            name: base.clone(),
            kind: git::BranchKind::Local,
          }),
          todo,
          window,
          cx,
        )
        .expect("the rebase starts")
    });
    let task = page.update(cx, |page, _| {
      page._interactive_rebase_task.take().expect("rebase task")
    });
    task.await;
    cx.run_until_parked();

    // Stopped on the conflict: the file is on screen with the prepared message.
    assert!(git::is_rebase_in_progress(&repo.path).expect("rebase state"));
    page.read_with(cx, |page, cx| {
      assert_eq!(page.center, CenterView::Diff);
      assert_eq!(page.selected_file.as_deref(), Some(Path::new("a.txt")));
      assert!(page.interactive_rebase_todo_view.is_none());
      assert_eq!(page.dock_panel.read(cx).commit_message(cx), "feature work");
    });
  }

  #[gpui::test]
  async fn cancelling_the_todo_leaves_the_center_as_it_was(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-interactive-rebase-cancel");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("a.txt"), "v2\n", "second");
    commit_text_file(&repo.path, Path::new("a.txt"), "v3\n", "third");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page
        .start_interactive_rebase(InteractiveRebaseTarget::HeadCount(2), window, cx)
        .expect("the todo opens")
    });
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::InteractiveRebase)
    });

    page.update_in(cx, |page, window, cx| {
      page.close_interactive_rebase_todo(window, cx)
    });

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.interactive_rebase_todo_view.is_none());
    });
    // Nothing was rewritten.
    let summaries = git::list_commit_history(&repo.path, 10)
      .expect("history")
      .into_iter()
      .map(|commit| commit.summary)
      .collect::<Vec<_>>();
    assert_eq!(summaries, vec!["third", "second", "first"]);
  }

  #[gpui::test]
  async fn an_interactive_rebase_is_refused_with_uncommitted_changes(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-interactive-rebase-dirty");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("a.txt"), "v2\n", "second");
    std::fs::write(repo.path.join("a.txt"), "v3 working\n").expect("dirty the worktree");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(
        !ids.contains(&CommandPaletteCommandId::InteractiveRebase),
        "rewriting history under uncommitted changes is refused"
      );
    });

    let refused = page.update_in(cx, |page, window, cx| {
      page.start_interactive_rebase(InteractiveRebaseTarget::HeadCount(2), window, cx)
    });
    assert_eq!(
      refused.expect_err("refused").as_ref(),
      "Interactive rebase is currently disabled."
    );
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.interactive_rebase_todo_view.is_none());
    });
  }

  #[gpui::test(iterations = 10)]
  async fn publishing_a_branch_opens_the_pull_request_form_after_the_push(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-publish-and-create");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let remote = publish_to_new_remote(&repo.path, "session-page-publish-and-create");
    git::create_branch(&repo.path, "feature").expect("create branch");
    git::switch_branch(
      &repo.path,
      &git::BranchRef {
        name: "feature".to_string(),
        kind: git::BranchKind::Local,
      },
    )
    .expect("switch branch");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "feature work");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.read_with(cx, |page, cx| {
      assert!(
        !page
          .repo_snapshot
          .read(cx)
          .branch_status()
          .expect("branch status")
          .has_upstream,
        "the branch starts unpublished"
      );
    });

    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature".to_string(),
    };
    page.update_in(cx, |page, window, cx| {
      page.publish_branch_and_create_pull_request(context, window, cx);
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("push task")
    });
    command.await;
    cx.run_until_parked();

    let remote_repo = git2::Repository::open(&remote).expect("open remote");
    assert!(
      remote_repo.refname_to_id("refs/heads/feature").is_ok(),
      "the branch reached the remote"
    );
    assert!(
      cx.update(|window, cx| window.has_active_dialog(cx)),
      "the pull request form follows the push"
    );
    page.read_with(cx, |page, _| {
      assert!(page.pending_pull_request.is_none());
    });
  }

  #[gpui::test(iterations = 10)]
  async fn a_refused_publish_leaves_no_form_waiting_for_the_next_push(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-publish-refused");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let remote = publish_to_new_remote(&repo.path, "session-page-publish-refused");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // Another git command is already running: the publish is refused up front.
    page.update(cx, |page, _| page.repo_command_in_flight = true);
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature".to_string(),
    };
    page.update_in(cx, |page, window, cx| {
      page.publish_branch_and_create_pull_request(context, window, cx);
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert!(page._repo_command_task.is_none(), "nothing was launched");
      assert!(page.pending_pull_request.is_none());
    });
    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));

    // An unrelated push must not inherit the form that was never opened.
    page.update(cx, |page, _| page.repo_command_in_flight = false);
    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(RepoCommand::Push, window, cx)
        .expect("push");
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("push task")
    });
    command.await;
    cx.run_until_parked();

    let remote_repo = git2::Repository::open(&remote).expect("open remote");
    let head = remote_repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("remote head");
    assert_eq!(
      head.summary().expect("read summary"),
      Some("second"),
      "the push went through"
    );
    assert!(
      !cx.update(|window, cx| window.has_active_dialog(cx)),
      "a plain push opens no pull request form"
    );
  }

  #[gpui::test(iterations = 10)]
  async fn a_failed_publish_opens_no_pull_request_form(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-publish-failure");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // No remote at all: the push cannot go anywhere.
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature".to_string(),
    };
    page.update_in(cx, |page, window, cx| {
      page.publish_branch_and_create_pull_request(context, window, cx);
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("push task")
    });
    command.await;
    cx.run_until_parked();

    assert!(
      !cx.update(|window, cx| window.has_active_dialog(cx)),
      "a push that failed must not open the form"
    );
    page.read_with(cx, |page, _| {
      assert!(page.pending_pull_request.is_none());
    });
  }

  #[gpui::test(iterations = 10)]
  async fn a_second_git_command_is_refused_while_one_runs(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-command-in-flight");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    let error = page.update_in(cx, |page, window, cx| {
      page.repo_command_in_flight = true;
      page
        .run_repo_command(RepoCommand::Fetch, window, cx)
        .expect_err("refused while busy")
    });

    assert!(error.contains("still running"), "{error}");
  }

  /// Leaves `feature` checked out with a commit that conflicts with the base branch.
  fn start_conflicting_rebase_setup(repo_root: &Path) -> String {
    commit_text_file(repo_root, Path::new("a.txt"), "base\n", "initial");
    let base = git::current_branch_status(repo_root)
      .expect("branch status")
      .name;
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(repo_root, &feature.name).expect("create branch");
    git::switch_branch(repo_root, &feature).expect("switch to feature");
    commit_text_file(repo_root, Path::new("a.txt"), "feature\n", "feature work");
    let base_ref = git::BranchRef {
      name: base.clone(),
      kind: git::BranchKind::Local,
    };
    git::switch_branch(repo_root, &base_ref).expect("switch back");
    commit_text_file(repo_root, Path::new("a.txt"), "main\n", "main work");
    git::switch_branch(repo_root, &feature).expect("switch to feature");
    base
  }

  /// Leaves the repository mid-rebase, stopped on a conflicted file.
  fn start_conflicting_rebase(repo_root: &Path) -> String {
    commit_text_file(repo_root, Path::new("a.txt"), "base\n", "initial");
    let base = git::current_branch_status(repo_root)
      .expect("branch status")
      .name;
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(repo_root, &feature.name).expect("create branch");
    git::switch_branch(repo_root, &feature).expect("switch to feature");
    commit_text_file(repo_root, Path::new("a.txt"), "feature\n", "feature work");
    let base_ref = git::BranchRef {
      name: base.clone(),
      kind: git::BranchKind::Local,
    };
    git::switch_branch(repo_root, &base_ref).expect("switch back");
    commit_text_file(repo_root, Path::new("a.txt"), "main\n", "main work");
    git::switch_branch(repo_root, &feature).expect("switch to feature");
    let _ = git::rebase_branch(repo_root, &base_ref);
    assert!(
      git::is_rebase_in_progress(repo_root).expect("rebase state"),
      "the rebase must be waiting on the conflict"
    );
    base
  }

  #[gpui::test]
  async fn a_rebase_in_progress_turns_the_commit_button_into_continue(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-rebase-continue");
    start_conflicting_rebase(&repo.path);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let panel = page.dock_panel.read(cx);
      assert!(panel.rebase_in_progress());
      assert!(!panel.merge_in_progress());

      // The palette follows: no commit, but the rebase can be continued or dropped.
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(!ids.contains(&CommandPaletteCommandId::Commit));
      assert!(ids.contains(&CommandPaletteCommandId::SkipRebase));
      assert!(ids.contains(&CommandPaletteCommandId::AbortRebase));
      assert!(
        !ids.contains(&CommandPaletteCommandId::ContinueRebase),
        "the conflict is still there"
      );
    });

    // Resolve and stage: continuing becomes possible.
    std::fs::write(repo.path.join("a.txt"), "resolved\n").expect("resolve conflict");
    git::stage_all(&repo.path).expect("stage the resolution");
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::ContinueRebase));
    });

    // The dock button runs it, and the rebase lands.
    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(RepoCommand::ContinueRebase, window, cx)
        .expect("continue the rebase")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert!(!git::is_rebase_in_progress(&repo.path).expect("rebase state"));
  }

  #[gpui::test]
  async fn aborting_a_rebase_from_the_palette_puts_the_branch_back(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-rebase-abort");
    start_conflicting_rebase(&repo.path);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::AbortRebase, window, cx)
        .expect("abort the rebase")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert!(!git::is_rebase_in_progress(&repo.path).expect("rebase state"));
    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "feature\n",
      "the branch is back where it was"
    );
  }

  #[gpui::test]
  async fn skipping_from_the_palette_drops_the_conflicting_commit(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-rebase-skip");
    start_conflicting_rebase(&repo.path);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::SkipRebase, window, cx)
        .expect("skip the conflicting commit")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert!(!git::is_rebase_in_progress(&repo.path).expect("rebase state"));
    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "main\n",
      "the skipped commit left nothing behind"
    );
  }

  #[gpui::test]
  async fn a_command_that_conflicts_opens_the_file_to_resolve(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-command-conflict");
    commit_text_file(&repo.path, Path::new("a.txt"), "base\n", "initial");
    let base = git::BranchRef {
      name: git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      kind: git::BranchKind::Local,
    };
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(&repo.path, &feature.name).expect("create branch");
    git::switch_branch(&repo.path, &feature).expect("switch to feature");
    commit_text_file(&repo.path, Path::new("a.txt"), "feature\n", "feature work");
    git::switch_branch(&repo.path, &base).expect("switch back");
    commit_text_file(&repo.path, Path::new("a.txt"), "main\n", "main work");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(RepoCommand::MergeBranch(feature), window, cx)
        .expect("the merge starts");
    });
    let task = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    task.await;
    cx.run_until_parked();

    // The conflict is a stop to resolve, not an error: the file is on screen.
    page.read_with(cx, |page, cx| {
      assert_eq!(page.selected_file.as_deref(), Some(Path::new("a.txt")));
      assert_eq!(page.center, CenterView::Diff);
      // Git prepared the merge message; the box carries it.
      assert_eq!(
        page.dock_panel.read(cx).commit_message(cx),
        crate::repo_command::merge_commit_message("feature", &base.name)
      );
    });

    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    // Mid-merge: aborting is offered, committing waits for the conflict to go.
    page.read_with(cx, |page, cx| {
      assert!(page.dock_panel.read(cx).merge_in_progress());
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::AbortMerge));
      assert!(!ids.contains(&CommandPaletteCommandId::Commit));
      assert!(!ids.contains(&CommandPaletteCommandId::AbortRebase));
    });

    // Resolved and staged: a merge ends with a commit.
    std::fs::write(repo.path.join("a.txt"), "resolved\n").expect("resolve conflict");
    git::stage_all(&repo.path).expect("stage the resolution");
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::Commit));
      assert!(!ids.contains(&CommandPaletteCommandId::ContinueRebase));
    });
  }

  #[gpui::test]
  async fn a_failed_command_is_reported_under_its_own_key(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-telemetry");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    let sink = crate::git_telemetry::test_support::RecordingSink::install();

    // No remote: the push cannot succeed.
    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(RepoCommand::Push, window, cx)
        .expect("push starts");
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    use crate::git_telemetry::test_support::Report;
    let reports = sink.reports();
    assert!(
      reports.contains(&Report::Breadcrumb("Push".to_string())),
      "the command that ran leaves a trail, got {reports:?}"
    );
    assert!(
      reports.iter().any(|report| matches!(
        report,
        Report::Unexpected { operation, .. } if operation == RepoCommand::Push.telemetry_key()
      )),
      "the failure is filed under the command's own key, got {reports:?}"
    );

    // A command that works reports its run, and no error.
    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(RepoCommand::StageAll, window, cx)
        .expect("stage all starts");
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    let reports = sink.reports();
    assert!(
      !reports
        .iter()
        .any(|report| matches!(report, Report::Unexpected { .. })),
      "success is not a crash, got {reports:?}"
    );
    crate::git_telemetry::set_test_sink(None);
  }
}
