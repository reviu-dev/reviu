//! Commit, amend and undo, plus the commit input wiring.

use super::*;

impl GitPage {
  pub(super) fn set_commit_input_value(
    &self,
    value: &str,
    window: Option<&mut Window>,
    cx: &mut Context<Self>,
  ) {
    let value = value.to_string();
    if let Some(window) = window {
      self
        .commit_input
        .update(cx, |input, cx| input.set_value(&value, window, cx));
      return;
    }

    let commit_input = self.commit_input.clone();
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      commit_input.update(cx, |input, cx| input.set_value(&value, window, cx));
    });
  }

  pub(super) fn subscribe_to_commit_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    cx.subscribe_in(
      &self.commit_input,
      window,
      |this, _state, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter {
          secondary: true, ..
        } = event
        {
          this.commit_changes_inner(window, cx);
        }
      },
    )
    .detach();
  }

  pub(super) fn commit_changes_action(
    &mut self,
    _: &CommitChanges,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let focus_handle = self.commit_input.read(cx).focus_handle(cx);
    if !focus_handle.contains_focused(window, cx) {
      return;
    }
    self.commit_changes_inner(window, cx);
  }

  pub(super) fn commit_changes(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.commit_changes_inner(window, cx);
  }

  pub(super) fn commit_changes_inner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.rebase_in_progress {
      let _ = window;
      self.continue_rebase_inner(cx);
      return;
    }

    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let message = self.commit_input.read(cx).value().to_string();
    if message.trim().is_empty() {
      return;
    }
    let has_changes = !self.status_entries.is_empty();
    if !has_changes {
      return;
    }
    let stage_all_needed = !self.has_staged_changes;
    let mut start_data = Map::new();
    start_data.insert("stage_all_needed".into(), stage_all_needed.into());
    self.add_git_breadcrumb("Commit started", start_data);
    crate::analytics::track(cx, "commit_made");

    let window_handle = window.window_handle();
    let commit_input = self.commit_input.clone();
    let editor = self.editor.clone();

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        if stage_all_needed {
          stage_all(&repo_root)?;
        }
        commit_changes(&repo_root, &message)
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            });
            let mut data = Map::new();
            data.insert("stage_all_needed".into(), stage_all_needed.into());
            this.add_git_breadcrumb("Commit succeeded", data);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("stage_all_needed".into(), stage_all_needed.into());
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Commit failed", data.clone());
            this.record_git_unexpected_error("git.commit", error_message.as_str(), data);
            this.push_git_action_error_notification("Commit failed", error_message.into(), cx);
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

  pub(super) fn commit_amend_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.has_head_commit {
      return;
    }

    let message = self.commit_input.read(cx).value().to_string();
    let message = message.trim().to_string();
    let message_opt = if message.is_empty() {
      None
    } else {
      Some(message)
    };

    let window_handle = window.window_handle();
    let commit_input = self.commit_input.clone();
    let editor = self.editor.clone();

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || amend_commit(&repo_root, message_opt.as_deref())).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            });
          }
          Err(error) => {
            this.push_git_action_error_notification("Amend failed", error.to_string().into(), cx);
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

  pub(super) fn undo_last_commit_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.can_undo_last_commit {
      return;
    }

    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || RepoCommand::UndoLastCommit.run(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(_) => {
            this.push_git_action_success_notification("Undid last commit".into(), cx);
          }
          Err(error) => {
            this.push_git_action_error_notification(
              "Undo last commit failed",
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

  pub(super) fn commit_primary_action_enabled(&self, commit_message: &str) -> bool {
    if self.rebase_in_progress {
      self.can_continue_rebase_command()
    } else {
      self.selected_repo.is_some()
        && !commit_message.trim().is_empty()
        && !self.status_entries.is_empty()
        && !crate::changes_list::has_conflicted_entries(&self.status_entries)
    }
  }

  pub(super) fn commit_primary_button_state(
    rebase_in_progress: bool,
    has_uncommitted_changes: bool,
    can_publish_branch: bool,
  ) -> GitCommitPrimaryButtonState {
    if rebase_in_progress {
      GitCommitPrimaryButtonState::ContinueRebase
    } else if can_publish_branch && !has_uncommitted_changes {
      GitCommitPrimaryButtonState::PublishBranch
    } else {
      GitCommitPrimaryButtonState::Commit
    }
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::*;
  use git::RepoStage;
  use git2::Repository;
  use gpui::TestAppContext;

  #[test]
  fn commit_primary_button_state_prefers_publish_branch_only_for_clean_publishable_branch() {
    assert_eq!(
      GitPage::commit_primary_button_state(false, false, true),
      GitCommitPrimaryButtonState::PublishBranch
    );
    assert_eq!(
      GitPage::commit_primary_button_state(false, true, true),
      GitCommitPrimaryButtonState::Commit
    );
    assert_eq!(
      GitPage::commit_primary_button_state(false, false, false),
      GitCommitPrimaryButtonState::Commit
    );
    assert_eq!(
      GitPage::commit_primary_button_state(true, false, true),
      GitCommitPrimaryButtonState::ContinueRebase
    );
  }

  #[gpui::test]
  async fn undo_last_commit_action_failure_shows_error_notification(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-undo-failure-notification");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    let head_before = head_oid(&repo.path);

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_undo_last_commit = true;
      this.undo_last_commit_action(cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
    assert_eq!(head_oid(&repo.path), head_before);
  }

  #[gpui::test]
  async fn commit_changes_inner_requires_selected_repo(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.status_entries = vec![make_status_entry("README.md", RepoStage::Unstaged)];
      this
        .commit_input
        .update(cx, |input, cx| input.set_value("feat: message", window, cx));

      this.commit_changes_inner(window, cx);
      assert!(this.status_task.is_none());
    });
  }

  #[gpui::test]
  async fn commit_changes_inner_requires_non_empty_message_and_changes(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-commit-guards");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = vec![make_status_entry("README.md", RepoStage::Unstaged)];

      this
        .commit_input
        .update(cx, |input, cx| input.set_value("   ", window, cx));
      this.commit_changes_inner(window, cx);
      assert!(this.status_task.is_none());

      this
        .commit_input
        .update(cx, |input, cx| input.set_value("feat: message", window, cx));
      this.status_entries.clear();
      this.commit_changes_inner(window, cx);
      assert!(this.status_task.is_none());
    });
  }

  #[gpui::test]
  async fn undo_last_commit_action_requires_selected_repo_and_undo_capability(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-undo-guards");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_undo_last_commit = false;
      this.undo_last_commit_action(cx);
      assert!(this.status_task.is_none());

      this.selected_repo = None;
      this.can_undo_last_commit = true;
      this.undo_last_commit_action(cx);
      assert!(this.status_task.is_none());
    });
  }

  #[gpui::test]
  async fn undo_last_commit_action_moves_head_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-undo-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "first");
    let _ = commit_text_file(&repo.path, rel_path, "v2\n", "second");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let expected_parent = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head before undo")
      .parent(0)
      .expect("parent")
      .id();

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let undo_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_undo_last_commit = true;
      this.undo_last_commit_action(cx);
      this.status_task.take().expect("undo task")
    });
    undo_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let head_after = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head after undo")
      .id();
    assert_eq!(head_after, expected_parent);
  }
}
