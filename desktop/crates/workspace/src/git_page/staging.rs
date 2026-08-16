//! Staging, unstaging and restoring: files, hunks and the whole worktree.

use super::*;

use crate::changes_list::{can_restore, can_unstage};

impl GitPage {
  pub(super) fn selected_file_can_unstage(stage: RepoStage) -> bool {
    can_unstage(stage)
  }

  #[allow(clippy::too_many_arguments)]
  pub(super) fn toggle_hunk_stage_action(
    &mut self,
    _: &crate::ToggleHunkStage,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    if self.resolve_active_conflict_in_editor(&editor, ConflictResolution::Current, cx) {
      cx.stop_propagation();
      return;
    }
    editor.update(cx, |editor, cx| {
      let Some(group_id) = editor.active_hunk_group_id(cx) else {
        return;
      };
      let Some(state) = editor
        .projection()
        .and_then(|p| p.groups.get(&group_id))
        .map(|g| g.state)
      else {
        return;
      };
      let action = match state {
        HunkState::Unstaged => HunkAction::Stage,
        HunkState::Staged => HunkAction::Unstage,
      };
      editor.enqueue_group_action(group_id, action, cx);
    });
    cx.stop_propagation();
  }

  pub(super) fn restore_hunk_action(
    &mut self,
    _: &crate::RestoreHunk,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    if self.resolve_active_conflict_in_editor(&editor, ConflictResolution::Incoming, cx) {
      cx.stop_propagation();
      return;
    }
    editor.update(cx, |editor, cx| {
      let Some(group_id) = editor.active_hunk_group_id(cx) else {
        return;
      };
      editor.enqueue_group_action(group_id, HunkAction::Restore, cx);
    });
    cx.stop_propagation();
  }

  /// The sidebar toggle: stage everything, or unstage it all when it already is.
  pub(super) fn toggle_stage_all_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self
      .changes_list
      .update(cx, |list, cx| list.toggle_stage_all(window, cx));
  }

  pub(super) fn restore_all_click_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self
      .changes_list
      .update(cx, |list, cx| list.confirm_restore_all(window, cx));
  }

  pub(super) fn stage_all_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self
      .changes_list
      .update(cx, |list, cx| list.stage_all_with_confirmation(window, cx));
  }

  pub(super) fn unstage_all_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self
      .changes_list
      .update(cx, |list, cx| list.unstage_all(window, cx));
  }

  pub(super) fn toggle_file_stage_action(
    &mut self,
    _: &crate::ToggleFileStage,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(entry) = self.selected_file_entry().cloned() else {
      return;
    };
    let has_markers = self.open_editor_has_unresolved_conflict_markers(cx);
    self.changes_list.update(cx, |list, cx| {
      list.set_open_file_has_conflict_markers(has_markers);
      if can_unstage(entry.stage) {
        list.unstage_file(entry.path, window, cx);
      } else {
        list.stage_file_with_confirmation(entry.path, entry.status, window, cx);
      }
    });
    cx.stop_propagation();
  }

  pub(super) fn restore_file_shortcut_action(
    &mut self,
    _: &crate::RestoreFile,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(entry) = self.selected_file_entry().cloned() else {
      return;
    };
    if !can_restore(entry.stage) {
      return;
    }
    self.changes_list.update(cx, |list, cx| {
      list.confirm_restore_file(entry.path, entry.status, window, cx)
    });
    cx.stop_propagation();
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::*;
  use git2::Repository;
  use gpui::TestAppContext;

  #[test]
  fn has_staged_changes_detects_staged_and_partial_entries() {
    assert!(!GitPage::has_staged_changes(&[
      make_status_entry("src/a.rs", RepoStage::Unstaged),
      make_status_entry("src/b.rs", RepoStage::Unstaged),
    ]));
    assert!(GitPage::has_staged_changes(&[
      make_status_entry("src/a.rs", RepoStage::Staged),
      make_status_entry("src/b.rs", RepoStage::Unstaged),
    ]));
    assert!(GitPage::has_staged_changes(&[make_status_entry(
      "src/a.rs",
      RepoStage::PartiallyStaged
    )]));
  }

  #[test]
  fn stage_all_command_visibility_requires_at_least_one_entry() {
    let entries = vec![make_status_entry("src/main.rs", RepoStage::Unstaged)];
    let all_staged_entries = vec![make_status_entry("src/lib.rs", RepoStage::Staged)];

    assert!(!GitPage::should_show_stage_all_command(&[]));
    assert!(GitPage::should_show_stage_all_command(&entries));
    assert!(!GitPage::should_show_stage_all_command(&all_staged_entries));
  }

  #[test]
  fn unstage_all_command_visibility_requires_all_entries_staged() {
    let mixed_entries = vec![
      make_status_entry("src/main.rs", RepoStage::Staged),
      make_status_entry("src/lib.rs", RepoStage::Unstaged),
    ];
    let all_staged_entries = vec![make_status_entry("src/editor.rs", RepoStage::Staged)];

    assert!(!GitPage::should_show_unstage_all_command(&[]));
    assert!(!GitPage::should_show_unstage_all_command(&mixed_entries));
    assert!(GitPage::should_show_unstage_all_command(
      &all_staged_entries
    ));
  }

  #[gpui::test]
  fn focus_page_restores_page_shortcut_focus(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      let external_focus = cx.focus_handle();
      window.focus(&external_focus, cx);
      assert!(!this.focus_handle.contains_focused(window, cx));

      this.focus_page(window, cx);
      assert!(this.focus_handle.contains_focused(window, cx));
    });
  }

  #[gpui::test]
  async fn commit_changes_inner_stages_and_commits_when_ready(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-commit-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("update file");
    let entries = list_repo_status(&repo.path).expect("list status after edit");
    assert!(!entries.is_empty());

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let commit_task = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = entries.clone();
      this.has_staged_changes = false;
      this.commit_input.update(cx, |input, cx| {
        input.set_value("  feat: update readme  ", window, cx)
      });

      this.commit_changes_inner(window, cx);
      this.status_task.take().expect("commit task")
    });
    commit_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.summary(), Some("feat: update readme"));
    assert!(
      list_repo_status(&repo.path)
        .expect("status after commit")
        .is_empty()
    );

    let input_value = git_page.read_with(cx, |this, cx| {
      this.commit_input.read(cx).value().to_string()
    });
    assert!(input_value.is_empty());
  }

  #[gpui::test]
  async fn commit_input_secondary_enter_stages_and_commits_when_ready(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-commit-secondary-enter");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("update file");
    let entries = list_repo_status(&repo.path).expect("list status after edit");
    assert!(!entries.is_empty());

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = entries.clone();
      this.has_staged_changes = false;
      this.commit_input.update(cx, |input, cx| {
        input.set_value("feat: secondary enter commit", window, cx)
      });

      this.commit_input.update(cx, |_input, cx| {
        cx.emit(InputEvent::PressEnter {
          secondary: true,
          shift: false,
        })
      });
    });

    cx.cx.run_until_parked();
    cx.run_until_parked();

    // The commit (and the status reload it triggers) may already be done by now on
    // fast machines; drain whatever is still scheduled instead of assuming a state.
    for _ in 0..4 {
      let task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
      let Some(task) = task else {
        break;
      };
      task.await;
      cx.run_until_parked();
    }

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.summary(), Some("feat: secondary enter commit"));
    assert!(
      list_repo_status(&repo.path)
        .expect("status after commit")
        .is_empty()
    );

    let input_value = git_page.read_with(cx, |this, cx| {
      this.commit_input.read(cx).value().to_string()
    });
    assert!(input_value.is_empty());
  }

  #[gpui::test]
  async fn discarding_the_open_file_falls_back_to_the_first_remaining_one(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-discard-open-file");
    commit_text_file(&repo.path, Path::new("a.txt"), "a1\n", "initial");
    std::fs::write(repo.path.join("a.txt"), "a2\n").expect("modify first file");
    std::fs::write(repo.path.join("b.txt"), "b\n").expect("write second file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = git::list_repo_status(&repo.path).expect("status");
      this.selected_file = Some(PathBuf::from("b.txt"));
      this.changes_list.update(cx, |list, cx| {
        list.set_repo_root(Some(repo.path.clone()), cx);
        list.set_entries(this.status_entries.clone(), cx);
      });
    });

    // Discard the file the editor is showing.
    let task = git_page.update_in(cx, |this, window, cx| {
      this.changes_list.update(cx, |list, cx| {
        list.restore_file(
          PathBuf::from("b.txt"),
          RepoStatusKind::Untracked,
          window,
          cx,
        );
        list._action_task.take().expect("discard task")
      })
    });
    task.await;
    cx.run_until_parked();
    await_git_page_background_tasks(git_page.clone(), cx).await;

    git_page.read_with(cx, |this, _| {
      assert_eq!(
        this.selected_file.as_deref(),
        Some(Path::new("a.txt")),
        "the editor should move to the file that is still there"
      );
    });
  }
}
