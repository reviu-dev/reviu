//! Merge, rebase and interactive rebase.

use super::*;

pub(super) fn interactive_rebase_success_message(target: &InteractiveRebaseTarget) -> String {
  match target {
    InteractiveRebaseTarget::Branch(branch) => {
      format!("Rebased interactively onto {}", branch.name)
    }
    InteractiveRebaseTarget::BranchInPlace(branch) => {
      format!("Edited commits since {}", branch.name)
    }
    InteractiveRebaseTarget::HeadCount(count) => {
      format!("Rebased last {count} commits")
    }
  }
}

impl GitPage {
  pub(super) fn start_merge_base_branch_action(
    &mut self,
    repo_root: PathBuf,
    base_branch_name: String,
    cx: &mut Context<Self>,
  ) {
    if self.fetch_in_progress {
      return;
    }

    self.fetch_in_progress = true;
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_action = repo_root.clone();
      let branch_name_for_fetch = base_branch_name.clone();
      let result = unblock(move || {
        if let Some(conflict_resolution) =
          Self::active_conflict_resolution_snapshot(&repo_root_for_action)
        {
          return Ok::<_, anyhow::Error>(GitPageOpenActionResult::ResumeActiveConflict(
            conflict_resolution,
          ));
        }

        fetch(&repo_root_for_action)?;
        let branch_ref = resolve_branch_ref(&repo_root_for_action, &branch_name_for_fetch)?
          .ok_or_else(|| {
            anyhow::anyhow!(
              "branch {:?} was not found locally or on any remote",
              branch_name_for_fetch
            )
          })?;
        Ok::<_, anyhow::Error>(GitPageOpenActionResult::MergeBaseBranchReady(branch_ref))
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.fetch_in_progress = false;
        this.pending_open_action = None;

        match result {
          Ok(GitPageOpenActionResult::ResumeActiveConflict(conflict_resolution)) => {
            this.merge_in_progress = conflict_resolution.merge_in_progress;
            this.rebase_in_progress = conflict_resolution.rebase_in_progress;
            if let Some(path) = conflict_resolution.conflicted_path {
              this.open_file_revealing_first_conflict(path, cx);
            }
          }
          Ok(GitPageOpenActionResult::MergeBaseBranchReady(branch_ref)) => {
            if let Err(error) = this.merge_branch_action(branch_ref, None, true, cx) {
              this.push_git_action_error_notification(
                "Update branch failed",
                error.to_string().into(),
                cx,
              );
            }
          }
          Err(error) => {
            this.push_git_action_error_notification(
              "Update branch failed",
              error.to_string().into(),
              cx,
            );
          }
        }

        this.reload_status(cx);
        this.refresh_branches(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  pub(super) fn merge_branch_action(
    &mut self,
    branch_ref: BranchRef,
    window: Option<&mut Window>,
    reveal_first_conflict_on_open: bool,
    cx: &mut Context<Self>,
  ) -> Result<(), anyhow::Error> {
    let Some(root_path) = self.selected_repo.clone() else {
      anyhow::bail!("No repository selected.");
    };
    let mut start_data = Map::new();
    start_data.insert("target_branch".into(), branch_ref.name.clone().into());
    self.add_git_breadcrumb("Merge started", start_data);

    let mut data = Map::new();
    data.insert("target_branch".into(), branch_ref.name.clone().into());

    match RepoCommand::MergeBranch(branch_ref).run(&root_path) {
      Ok(outcome @ (RepoCommandOutcome::Done { .. } | RepoCommandOutcome::UpToDate { .. })) => {
        let breadcrumb = match outcome {
          RepoCommandOutcome::UpToDate { .. } => "Merge already up to date",
          _ => "Merge succeeded",
        };
        self.add_git_breadcrumb(breadcrumb, data);
        if let Some(window) = window {
          self.notify_repo_command_outcome(&outcome, window, cx);
        }
        Ok(())
      }
      Ok(RepoCommandOutcome::Conflicted {
        path,
        commit_message,
        error,
      }) => {
        self.merge_in_progress = true;
        self.rebase_in_progress = false;
        data.insert(
          "file".into(),
          path.to_string_lossy().replace(['\n', '\r'], "").into(),
        );
        data.insert("error".into(), error.into());
        self.record_git_expected_error("git.merge", "conflict", data.clone());
        self.add_git_breadcrumb("Merge has conflicts", data);
        self.open_conflicted_file(
          path,
          commit_message,
          window,
          reveal_first_conflict_on_open,
          cx,
        );
        Ok(())
      }
      Err(err) => {
        let err_text = err.to_string();
        data.insert("error".into(), err_text.clone().into());
        self.add_git_breadcrumb("Merge failed", data.clone());
        self.record_git_unexpected_error("git.merge", err_text.as_str(), data);
        Err(err)
      }
    }
  }

  pub(super) fn continue_rebase_inner(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.rebase_in_progress {
      return;
    }
    self.operation_error = None;
    if Self::has_conflicted_entries(&self.status_entries) {
      self.operation_error = Some("Resolve all conflicts before continuing the rebase.".into());
      let mut data = Map::new();
      data.insert("reason".into(), "conflicts_present".into());
      self.record_git_expected_error("git.continue_rebase", "conflict", data);
      cx.notify();
      return;
    }

    self.add_git_breadcrumb("Continue rebase started", Map::new());
    let commit_input = self.commit_input.clone();
    let window_handle = self.window_handle;
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_continue = repo_root.clone();
      let result = unblock(move || RepoCommand::ContinueRebase.run(&repo_root_for_continue)).await;
      let (success, conflicted_path, error_message, failure_message, expected_conflict) =
        match result {
          Ok(RepoCommandOutcome::Conflicted { path, error, .. }) => {
            (false, Some(path), None, Some(error), true)
          }
          Ok(_) => (true, None, None, None, false),
          Err(err) => {
            let err_text = err.to_string();
            // git can stop on conflicts without leaving a conflicted entry behind.
            let is_conflict_state = err_text.contains("rebase has conflicts");
            let error_message = if is_conflict_state {
              None
            } else {
              Some(format!("Continue rebase failed: {err}"))
            };
            (
              false,
              None,
              error_message,
              Some(err_text),
              is_conflict_state,
            )
          }
        };
      let _ = this.update(cx, |this, cx| {
        if success {
          this.rebase_in_progress = false;
          this.force_push_after_rebase = true;
          this.operation_error = None;
          this.add_git_breadcrumb("Continue rebase succeeded", Map::new());
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value("", window, cx));
          });
        } else if expected_conflict {
          let mut data = Map::new();
          data.insert("has_conflicts".into(), true.into());
          if let Some(message) = failure_message.clone() {
            data.insert("error".into(), message.into());
          }
          this.record_git_expected_error("git.continue_rebase", "conflict", data.clone());
          this.add_git_breadcrumb("Continue rebase blocked by conflicts", data);
        } else if let Some(message) = failure_message.as_deref() {
          let mut data = Map::new();
          data.insert("error".into(), message.to_string().into());
          this.add_git_breadcrumb("Continue rebase failed", data.clone());
          this.record_git_unexpected_error("git.continue_rebase", message, data);
        }
        this.reload_status(cx);
        if let Some(path) = conflicted_path {
          this.open_status_file(path, cx);
        }
        if let Some(error_message) = error_message {
          this.operation_error = Some(error_message.into());
        }
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
        cx.notify();
      });
    });
    self.status_task = Some(task);
  }

  pub(super) fn prepare_interactive_rebase_commits(
    &self,
    target: &InteractiveRebaseTarget,
  ) -> Result<git::InteractiveRebasePreview, SharedString> {
    let Some(repo_root) = self.selected_repo.clone() else {
      return Err("No repository selected.".into());
    };
    if !self.should_show_interactive_rebase_palette_command() {
      return Err("Interactive rebase is currently disabled.".into());
    }

    let preview = list_interactive_rebase_commits(&repo_root, target)
      .map_err(|err| -> SharedString { format!("Action failed: {err}").into() })?;
    if preview.commits.is_empty() {
      return Err("No commits available for interactive rebase.".into());
    }
    Ok(preview)
  }

  pub(super) fn dispatch_interactive_rebase_target(
    &mut self,
    target: InteractiveRebaseTarget,
    preview: git::InteractiveRebasePreview,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if preview.dropped_merge_count == 0 {
      let view = cx.entity();
      let commits = preview.commits;
      window.on_next_frame(move |window, cx| {
        let target = target.clone();
        let commits = commits.clone();
        view.update(cx, move |view, cx| {
          view.open_interactive_rebase_todo_view_with_commits(target, commits, window, cx);
        });
      });
      return;
    }

    let count = preview.dropped_merge_count;
    let title: SharedString = "Drop merge commits?".into();
    let message: SharedString = if count == 1 {
      "1 merge commit will be dropped from the rebase. Its changes will be re-applied through the picked commits.".into()
    } else {
      format!(
        "{count} merge commits will be dropped from the rebase. Their changes will be re-applied through the picked commits."
      )
      .into()
    };
    let view = cx.entity();
    let commits = preview.commits;

    window.on_next_frame(move |window, cx| {
      let view = view.clone();
      let target = target.clone();
      let commits = commits.clone();
      let title = title.clone();
      let message = message.clone();
      window.open_alert_dialog(cx, move |alert, _, _| {
        let view = view.clone();
        let target = target.clone();
        let commits = commits.clone();
        ConfirmDialog::new(title.clone(), div().child(message.clone()))
          .confirm_text("Continue")
          .cancel_text("Cancel")
          .on_confirm(move |_, window, cx| {
            let target = target.clone();
            let commits = commits.clone();
            view.update(cx, move |view, cx| {
              view.open_interactive_rebase_todo_view_with_commits(target, commits, window, cx);
            });
            true
          })
          .build(alert)
      });
    });
  }

  pub(super) fn close_interactive_rebase_todo_view(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.interactive_rebase_todo_view = None;
    self.focus_editor_or_page(window, cx);
    cx.on_next_frame(window, |this, window, cx| {
      this.focus_editor_or_page(window, cx);
    });
    cx.notify();
  }

  pub(super) fn open_interactive_rebase_todo_view_with_commits(
    &mut self,
    target: InteractiveRebaseTarget,
    commits: Vec<git::InteractiveRebaseCommit>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !self.should_show_interactive_rebase_palette_command() {
      self.operation_error = Some("Interactive rebase is currently disabled.".into());
      cx.notify();
      return;
    }

    let view_for_submit = cx.entity();
    let on_submit: InteractiveRebaseTodoViewHandler =
      Arc::new(move |target, todo_entries, window, cx| {
        view_for_submit.update(cx, |view, cx| {
          let result = view.start_interactive_rebase_action(target, todo_entries, window, cx);
          if result.is_ok() {
            view.close_interactive_rebase_todo_view(window, cx);
          }
          result
        })
      });

    let view_for_cancel = cx.entity();
    let on_cancel: InteractiveRebaseTodoViewCancelHandler = Arc::new(move |window, cx| {
      view_for_cancel.update(cx, |view, cx| {
        view.close_interactive_rebase_todo_view(window, cx);
      });
    });

    let todo_config = InteractiveRebaseTodoViewConfig::new(target, commits, on_submit, on_cancel);
    let todo_view = cx.new(|cx| InteractiveRebaseTodoView::new(window, cx, todo_config));
    self.interactive_rebase_todo_view = Some(todo_view.clone());
    cx.on_next_frame(window, move |_, window, cx| {
      todo_view.update(cx, |view, cx| {
        view.focus_rows_list(window, cx);
      });
    });
    cx.notify();
  }

  pub(super) fn start_interactive_rebase_action(
    &mut self,
    target: InteractiveRebaseTarget,
    todo_entries: Vec<InteractiveRebaseTodoEntry>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(repo_root) = self.selected_repo.clone() else {
      return Err("No repository selected.".into());
    };
    if !self.should_show_interactive_rebase_palette_command() {
      return Err("Interactive rebase is currently disabled.".into());
    }

    self.operation_error = None;
    let commit_input = self.commit_input.clone();
    let window_handle = window.window_handle();
    let editor = self.editor.clone();
    let success_message = interactive_rebase_success_message(&target);
    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_rebase = repo_root.clone();
      let result =
        unblock(move || start_interactive_rebase(&repo_root_for_rebase, &target, &todo_entries))
          .await;

      let rebase_in_progress = is_rebase_in_progress(&repo_root).unwrap_or(false);
      let conflicted_path = crate::repo_command::first_conflicted_path(&repo_root);
      let rebase_message = if rebase_in_progress {
        current_rebase_commit_message(&repo_root).ok().flatten()
      } else {
        None
      };
      let (success, error_message) = match result {
        Ok(()) => (!rebase_in_progress, None),
        Err(err) => {
          let is_conflict_state = conflicted_path.is_some() || rebase_in_progress;
          let error_message = if is_conflict_state {
            None
          } else {
            Some(format!("Interactive rebase failed: {err}"))
          };
          (false, error_message)
        }
      };

      let _ = this.update(cx, |this, cx| {
        if success {
          this.force_push_after_rebase = true;
          this.operation_error = None;
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            window.push_notification(Notification::success(success_message), cx);
          });
        }
        this.reload_status(cx);
        this.refresh_branches(cx);
        if let Some(path) = conflicted_path {
          this.open_status_file(path, cx);
        }
        if let Some(message) = rebase_message {
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value(&message, window, cx));
          });
        }
        if let Some(error_message) = error_message {
          this.operation_error = Some(error_message.into());
        }
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
        cx.notify();
      });
    });
    self.status_task = Some(task);
    Ok(())
  }

  pub(super) fn abort_merge_action(
    &mut self,
    _: &gpui::ClickEvent,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.merge_in_progress {
      return;
    }

    let editor = self.editor.clone();
    let commit_input = self.commit_input.clone();
    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || abort_merge(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            });
          }
          Err(error) => {
            this.push_git_action_error_notification(
              "Abort merge failed",
              error.to_string().into(),
              cx,
            );
          }
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  pub(super) fn abort_rebase_action(
    &mut self,
    _: &gpui::ClickEvent,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.rebase_in_progress {
      return;
    }

    let editor = self.editor.clone();
    let commit_input = self.commit_input.clone();
    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || abort_rebase(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            this.force_push_after_rebase = false;
            let _ = cx.update_window(window_handle, |_, window, cx| {
              commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            });
          }
          Err(error) => {
            this.push_git_action_error_notification(
              "Abort rebase failed",
              error.to_string().into(),
              cx,
            );
          }
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  pub(super) fn continue_rebase_action(
    &mut self,
    _: &gpui::ClickEvent,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.continue_rebase_inner(cx);
  }

  pub(super) fn can_continue_rebase_command(&self) -> bool {
    self.selected_repo.is_some()
      && self.rebase_in_progress
      && !Self::has_conflicted_entries(&self.status_entries)
  }

  pub(super) fn continue_rebase_disabled_reason(&self) -> Option<&'static str> {
    (self.selected_repo.is_some() && self.rebase_in_progress && !self.can_continue_rebase_command())
      .then_some("Resolve and stage conflicts first")
  }

  pub(super) fn operation_in_progress_disabled_reason(&self) -> Option<&'static str> {
    if self.rebase_in_progress {
      Some("Finish or abort the current rebase first")
    } else if self.merge_in_progress {
      Some("Finish or abort the current merge first")
    } else {
      None
    }
  }

  pub(super) fn interactive_rebase_disabled_reason(&self) -> Option<&'static str> {
    if self.selected_repo.is_none() || self.should_show_interactive_rebase_palette_command() {
      return None;
    }
    if let Some(reason) = self.operation_in_progress_disabled_reason() {
      return Some(reason);
    }
    if !self.has_head_commit {
      return Some("Create a commit first");
    }
    if !self.status_entries.is_empty() {
      return Some("Commit or stash worktree changes first");
    }
    if Self::is_detached_head(self.branch_status.as_ref()) {
      return Some("Checkout a branch first");
    }
    None
  }

  pub(super) fn sync_rebase_commit_input(
    &mut self,
    was_rebase_in_progress: bool,
    rebase_in_progress: bool,
    rebase_commit_message: Option<String>,
    cx: &mut Context<Self>,
  ) {
    if rebase_in_progress {
      let Some(message) = rebase_commit_message
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty())
      else {
        return;
      };
      if self.commit_input.read(cx).value() == message {
        return;
      }
      let commit_input = self.commit_input.clone();
      let window_handle = self.window_handle;
      let _ = cx.update_window(window_handle, move |_, window, cx| {
        commit_input.update(cx, |input, cx| input.set_value(&message, window, cx));
      });
      return;
    }

    if was_rebase_in_progress {
      let current_value = self.commit_input.read(cx).value();
      if current_value.trim().is_empty() {
        return;
      }
      let commit_input = self.commit_input.clone();
      let window_handle = self.window_handle;
      let _ = cx.update_window(window_handle, move |_, window, cx| {
        commit_input.update(cx, |input, cx| input.set_value("", window, cx));
      });
    }
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::*;
  use git::{create_branch, merge_branch, rebase_branch};
  use git2::Repository;
  use gpui::TestAppContext;

  #[gpui::test]
  async fn git_page_handle_selects_repo_navigates_and_starts_base_merge(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/github/acme/widget/pull/42", cx);
    });

    let repo = TempRepo::init("git-page-handle-merge-base");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    cx.update(|_, cx| {
      GitPageHandle::show_repository_and_merge_base(repo.path.clone(), base_branch.clone(), cx);
    });
    let (pending_open_action, selected_file, has_editor) = git_page.read_with(cx, |this, _cx| {
      (
        this.pending_open_action.clone(),
        this.selected_file.clone(),
        this.editor.is_some(),
      )
    });
    assert_eq!(
      pending_open_action,
      Some(GitPageOpenAction::MergeBaseBranch {
        base_branch_name: base_branch.clone(),
      })
    );
    assert!(GitPage::should_show_open_action_loading_state(
      pending_open_action.as_ref(),
      selected_file.as_deref(),
      has_editor,
    ));
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, merge_in_progress, selected_file) = git_page.read_with(cx, |this, _cx| {
      (
        this.selected_repo.clone(),
        this.merge_in_progress,
        this.selected_file.clone(),
      )
    });
    assert_eq!(selected_repo, Some(repo.path.clone()));
    assert!(
      merge_in_progress,
      "merge state should stay active on conflicts"
    );
    assert_eq!(selected_file, Some(rel_path.to_path_buf()));
    cx.update(|_, cx| {
      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/git");
    });
  }

  #[gpui::test]
  async fn git_page_handle_reopens_active_merge_conflicts_without_resetting_merge_mode(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/github/acme/widget/pull/42", cx);
    });

    let repo = TempRepo::init("git-page-handle-resume-merge-conflict");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    let merge_result = merge_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    );
    assert!(merge_result.is_err(), "merge should stop on conflicts");
    assert!(
      is_merge_in_progress(&repo.path).expect("read merge state"),
      "repo should already be in merge mode before reopening via handle"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);

    cx.update(|_, cx| {
      GitPageHandle::show_repository_and_merge_base(repo.path.clone(), base_branch.clone(), cx);
    });

    let (merge_in_progress, rebase_in_progress, pending_open_action, selected_file, has_editor) =
      git_page.read_with(cx, |this, _cx| {
        (
          this.merge_in_progress,
          this.rebase_in_progress,
          this.pending_open_action.clone(),
          this.selected_file.clone(),
          this.editor.is_some(),
        )
      });
    assert!(merge_in_progress);
    assert!(!rebase_in_progress);
    assert_eq!(
      pending_open_action,
      Some(GitPageOpenAction::MergeBaseBranch {
        base_branch_name: base_branch.clone(),
      })
    );
    assert!(GitPage::should_show_open_action_loading_state(
      pending_open_action.as_ref(),
      selected_file.as_deref(),
      has_editor,
    ));

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, merge_in_progress, selected_file) = git_page.read_with(cx, |this, _cx| {
      (
        this.selected_repo.clone(),
        this.merge_in_progress,
        this.selected_file.clone(),
      )
    });
    assert_eq!(selected_repo, Some(repo.path.clone()));
    assert!(merge_in_progress);
    assert_eq!(selected_file, Some(rel_path.to_path_buf()));
    cx.update(|_, cx| {
      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/git");
    });
  }

  #[gpui::test]
  async fn git_page_handle_reveals_first_conflict_after_opening_merge_resolution(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/github/acme/widget/pull/42", cx);
    });

    let repo = TempRepo::init("git-page-handle-reveal-first-conflict");
    let rel_path = Path::new("README.md");
    let build_contents = |replacement: &str| {
      let mut lines = (0..80)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>();
      lines[60] = replacement.to_string();
      format!("{}\n", lines.join("\n"))
    };
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      &build_contents("base line"),
      "initial",
    );
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      &build_contents("main change"),
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      &build_contents("feature change"),
      "feature change",
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);

    cx.update(|_, cx| {
      GitPageHandle::show_repository_and_merge_base(repo.path.clone(), base_branch.clone(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_file, conflict_navigation, conflict_top, viewport_height) =
      git_page.read_with(cx, |this, cx| {
        let editor = this.editor.as_ref().expect("editor should exist").read(cx);
        let conflict_navigation = this
          .editor_conflict_navigation_state(cx)
          .expect("conflict navigation state");
        let display_line = editor
          .first_display_line_for_conflict(conflict_navigation.active_start_line)
          .expect("display line for conflict");

        (
          this.selected_file.clone(),
          conflict_navigation,
          GitPage::hunk_action_top(
            editor.measured_editor_line_height(),
            display_line,
            editor.scroll_offset_y,
          ),
          editor.viewport_height,
        )
      });

    assert_eq!(selected_file, Some(rel_path.to_path_buf()));
    assert_eq!(conflict_navigation.active_index, 0);
    assert_eq!(conflict_navigation.total, 1);
    assert!(conflict_navigation.active_start_line >= 60);
    assert!(
      conflict_top < viewport_height,
      "expected first conflict to be visible after opening merge resolution"
    );
  }

  #[gpui::test]
  async fn git_page_handle_reveals_first_conflict_when_file_has_multiple_conflicts(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/github/acme/widget/pull/42", cx);
    });

    let repo = TempRepo::init("git-page-handle-reveal-multi-conflict");
    let rel_path = Path::new("README.md");
    let build_contents = |replacement_prefix: &str| {
      let mut lines = (0..160)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>();
      for target_line in [20usize, 55, 90, 125] {
        lines[target_line] = format!("{replacement_prefix} {target_line}");
      }
      format!("{}\n", lines.join("\n"))
    };
    let _ = commit_text_file(&repo.path, rel_path, &build_contents("base"), "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    let _ = commit_text_file(&repo.path, rel_path, &build_contents("main"), "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      &build_contents("feature"),
      "feature change",
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);

    cx.update(|_, cx| {
      GitPageHandle::show_repository_and_merge_base(repo.path.clone(), base_branch.clone(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_file, first_conflict_top, viewport_height, conflict_navigation) = git_page
      .read_with(cx, |this, cx| {
        let editor = this.editor.as_ref().expect("editor should exist").read(cx);
        let conflict_navigation = this
          .editor_conflict_navigation_state(cx)
          .expect("conflict navigation state");
        let first_display_line = editor
          .first_display_line_for_conflict(20)
          .expect("first conflict display line");

        (
          this.selected_file.clone(),
          GitPage::hunk_action_top(
            editor.measured_editor_line_height(),
            first_display_line,
            editor.scroll_offset_y,
          ),
          editor.viewport_height,
          conflict_navigation,
        )
      });

    assert_eq!(selected_file, Some(rel_path.to_path_buf()));
    assert_eq!(conflict_navigation.active_index, 0);
    assert_eq!(conflict_navigation.total, 4);
    assert_eq!(conflict_navigation.active_start_line, 20);
    assert!(
      first_conflict_top >= px(0.0) && first_conflict_top < viewport_height,
      "expected first conflict to remain visible after diff projection settles"
    );
  }

  #[test]
  fn push_flags_require_force_push_after_rebase_for_tracked_branch() {
    let clean_ahead = make_branch_status("main", 2, 0, true);
    assert_eq!(
      GitPage::push_flags(Some(&clean_ahead), true, true),
      (false, true)
    );
    assert_eq!(
      GitPage::push_flags(Some(&clean_ahead), true, false),
      (true, false)
    );

    let no_ahead = make_branch_status("main", 0, 0, true);
    assert_eq!(
      GitPage::push_flags(Some(&no_ahead), true, true),
      (false, false)
    );
  }

  #[test]
  fn accept_all_conflict_command_rules_match_editor_header_rules() {
    assert!(GitPage::can_accept_all_conflicts(
      Some(RepoStatusKind::Conflicted),
      false,
      true,
    ));
    assert!(!GitPage::can_accept_all_conflicts(
      Some(RepoStatusKind::Conflicted),
      true,
      true,
    ));
    assert!(!GitPage::can_accept_all_conflicts(
      Some(RepoStatusKind::Conflicted),
      false,
      false,
    ));
    assert!(!GitPage::can_accept_all_conflicts(
      Some(RepoStatusKind::Modified),
      false,
      true,
    ));
    assert!(!GitPage::can_accept_all_conflicts(None, false, true));
  }

  #[test]
  fn has_conflicted_entries_detects_conflict_status() {
    let clean_entries = vec![
      make_status_entry("src/a.rs", RepoStage::Unstaged),
      make_status_entry("src/b.rs", RepoStage::Staged),
    ];
    assert!(!GitPage::has_conflicted_entries(&clean_entries));

    let mut conflicted_entries = clean_entries;
    conflicted_entries.push(RepoStatusEntry {
      path: PathBuf::from("src/conflict.rs"),
      old_path: None,
      status: RepoStatusKind::Conflicted,
      stage: RepoStage::Unstaged,
    });
    assert!(GitPage::has_conflicted_entries(&conflicted_entries));
  }

  #[gpui::test]
  fn interactive_rebase_todo_view_open_and_cancel_returns_to_editor(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(PathBuf::from("/tmp/repo"));
      this.has_head_commit = true;
      this.branch_status = Some(make_branch_status("main", 0, 0, true));
      this.status_entries.clear();

      let commits = vec![git::InteractiveRebaseCommit {
        oid: "1111111111111111111111111111111111111111".to_string(),
        short_oid: "1111111".to_string(),
        summary: "sample commit".to_string(),
      }];
      this.open_interactive_rebase_todo_view_with_commits(
        InteractiveRebaseTarget::HeadCount(2),
        commits,
        window,
        cx,
      );
      assert!(this.interactive_rebase_todo_view.is_some());

      this.close_interactive_rebase_todo_view(window, cx);
      assert!(this.interactive_rebase_todo_view.is_none());
    });
  }

  #[gpui::test]
  async fn editor_conflict_navigation_moves_between_conflicts_in_selected_file(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-editor-conflict-navigation");
    let rel_path = Path::new("README.md");
    let conflict_text = "pre\n<<<<<<< HEAD\nours1\n=======\ntheirs1\n>>>>>>> branch\nmid\n<<<<<<< HEAD\nours2\n=======\ntheirs2\n>>>>>>> branch\npost\n";
    std::fs::write(repo.path.join(rel_path), conflict_text).expect("write conflict markers");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = vec![RepoStatusEntry {
        path: rel_path.to_path_buf(),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let initial_state = git_page.read_with(cx, |this, cx| {
      this
        .editor_conflict_navigation_state(cx)
        .expect("initial conflict navigation state")
    });
    assert_eq!(initial_state.active_index, 0);
    assert_eq!(initial_state.total, 2);

    git_page.update_in(cx, |this, _window, cx| {
      this.navigate_annotation_in_editor(AnnotationDirection::Next, cx);
    });

    let next_state = git_page.read_with(cx, |this, cx| {
      this
        .editor_conflict_navigation_state(cx)
        .expect("next conflict navigation state")
    });
    assert_eq!(next_state.active_index, 1);
    assert_eq!(next_state.total, 2);
    assert_eq!(next_state.active_start_line, 7);
  }

  #[gpui::test]
  async fn annotation_navigation_falls_back_to_hunks_when_no_conflicts(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-annotation-hunk-navigation");
    let rel_path = Path::new("README.md");
    let base_contents = (0..30)
      .map(|line| format!("line {line}"))
      .collect::<Vec<_>>()
      .join("\n");
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      &format!("{base_contents}\n"),
      "initial",
    );

    let mut modified_lines = (0..30)
      .map(|line| format!("line {line}"))
      .collect::<Vec<_>>();
    modified_lines[5] = "line 5 modified".to_string();
    modified_lines[20] = "line 20 modified".to_string();
    let modified_contents = format!("{}\n", modified_lines.join("\n"));
    std::fs::write(repo.path.join(rel_path), modified_contents).expect("write modified file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = vec![RepoStatusEntry {
        path: rel_path.to_path_buf(),
        old_path: None,
        status: RepoStatusKind::Modified,
        stage: RepoStage::Unstaged,
      }];
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let initial_state = git_page.read_with(cx, |this, cx| {
      this
        .editor_annotation_navigation_state(cx)
        .expect("initial annotation navigation state")
    });
    assert_eq!(initial_state.kind, AnnotationKind::Change);
    assert_eq!(initial_state.total, 2);

    git_page.update_in(cx, |this, _window, cx| {
      this.navigate_annotation_in_editor(AnnotationDirection::Next, cx);
    });

    let next_state = git_page.read_with(cx, |this, cx| {
      this
        .editor_annotation_navigation_state(cx)
        .expect("next annotation navigation state")
    });
    assert_eq!(next_state.kind, AnnotationKind::Change);
    assert_eq!(next_state.total, 2);
    assert_ne!(next_state.active_index, initial_state.active_index);
  }

  #[gpui::test]
  async fn abort_merge_action_clears_merge_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-abort-merge");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = merge_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("merge should fail with conflicts");
    assert!(
      is_merge_in_progress(&repo.path).expect("read merge state"),
      "merge state should be active after conflict"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.merge_in_progress));
    git_page.update_in(cx, |this, window, cx| {
      this.commit_input.update(cx, |input, cx| {
        input.set_value("Merge branch 'feature' into main", window, cx)
      });
    });

    let abort_task = git_page.update_in(cx, |this, window, cx| {
      this.abort_merge_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("abort merge task")
    });
    abort_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_merge_in_progress(&repo.path).expect("read merge state after abort"),
      "merge state should be cleaned after abort"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read README after abort"),
      "main change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after abort merge")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.merge_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[gpui::test]
  async fn commit_action_completes_merge_after_conflict_resolution(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-commit-merge-resolution");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = merge_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("merge should fail with conflicts");
    assert!(
      is_merge_in_progress(&repo.path).expect("read merge state"),
      "merge state should be active after conflict"
    );

    std::fs::write(repo.path.join(rel_path), "resolved\n").expect("write resolved contents");
    stage_file(&repo.path, rel_path).expect("stage resolved file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.merge_in_progress));
    assert!(
      !git_page.read_with(cx, |this, _| GitPage::has_conflicted_entries(
        &this.status_entries
      )),
      "conflicts should be resolved before commit"
    );

    let commit_task = git_page.update_in(cx, |this, window, cx| {
      this.commit_input.update(cx, |input, cx| {
        input.set_value("Merge branch 'feature' into main", window, cx)
      });
      this.commit_changes(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("commit task")
    });
    commit_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_merge_in_progress(&repo.path).expect("read merge state after commit"),
      "merge state should be cleaned after commit"
    );
    let repo_handle = Repository::open(&repo.path).expect("open repo after merge commit");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head after merge commit");
    assert_eq!(head.parent_count(), 2);
    assert_eq!(head.summary(), Some("Merge branch 'feature' into main"));
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after merge commit")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.merge_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[gpui::test]
  async fn abort_rebase_action_clears_rebase_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-abort-rebase");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.rebase_in_progress));
    git_page.update_in(cx, |this, window, cx| {
      this.commit_input.update(cx, |input, cx| {
        input.set_value("Rebase branch 'main' onto feature", window, cx)
      });
    });

    let abort_task = git_page.update_in(cx, |this, window, cx| {
      this.abort_rebase_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("abort rebase task")
    });
    abort_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after abort"),
      "rebase state should be cleaned after abort"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read README after abort"),
      "main change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after abort rebase")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.rebase_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[gpui::test]
  async fn continue_rebase_action_completes_rebase_after_conflict_resolution(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-continue-rebase");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );

    std::fs::write(repo.path.join(rel_path), "resolved\n").expect("write resolved contents");
    stage_file(&repo.path, rel_path).expect("stage resolved file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.rebase_in_progress));
    assert!(
      !git_page.read_with(cx, |this, _| {
        GitPage::has_conflicted_entries(&this.status_entries)
      }),
      "conflicts should be resolved before continue"
    );
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      "main change"
    );

    let continue_task = git_page.update_in(cx, |this, window, cx| {
      this.continue_rebase_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("continue rebase task")
    });
    continue_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after continue"),
      "rebase state should be cleaned after continue"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read README after continue"),
      "resolved\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after continue rebase")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.rebase_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[gpui::test]
  async fn continue_rebase_action_opens_first_conflicted_file_for_next_conflict(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-continue-rebase-next-conflict");
    let readme_path = Path::new("README.md");
    let notes_path = Path::new("NOTES.txt");
    let _ = commit_text_file(&repo.path, readme_path, "base\n", "initial readme");
    let _ = commit_text_file(&repo.path, notes_path, "base\n", "initial notes");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      readme_path,
      "main readme change\n",
      "main readme change",
    );
    let _ = commit_text_file(
      &repo.path,
      notes_path,
      "main notes change\n",
      "main notes change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      readme_path,
      "feature readme change\n",
      "feature readme change",
    );
    let _ = commit_text_file(
      &repo.path,
      notes_path,
      "feature notes change\n",
      "feature notes change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with first conflict");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after first conflict"
    );

    std::fs::write(repo.path.join(readme_path), "resolved readme\n")
      .expect("write resolved first conflict");
    stage_file(&repo.path, readme_path).expect("stage resolved first conflict");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let continue_task = git_page.update_in(cx, |this, window, cx| {
      this.continue_rebase_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("continue rebase task")
    });
    continue_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state after continue"),
      "rebase state should remain active due to next conflict"
    );
    assert!(git_page.read_with(cx, |this, _| this.rebase_in_progress));
    assert!(
      git_page.read_with(cx, |this, _| {
        GitPage::has_conflicted_entries(&this.status_entries)
      }),
      "expected conflicted entries after next conflict"
    );
    assert_eq!(
      git_page.read_with(cx, |this, _| this.selected_file.clone()),
      Some(notes_path.to_path_buf())
    );
  }
}
