//! Staging, unstaging and restoring: files, hunks and the whole worktree.

use super::*;

impl GitPage {
  pub(super) fn restore_uses_delete(status: RepoStatusKind) -> bool {
    status == RepoStatusKind::Untracked
  }

  pub(super) fn selected_file_can_stage(stage: RepoStage) -> bool {
    stage == RepoStage::Unstaged
  }

  pub(super) fn selected_file_can_unstage(stage: RepoStage) -> bool {
    matches!(stage, RepoStage::Staged | RepoStage::PartiallyStaged)
  }

  pub(super) fn can_restore_file_stage(stage: RepoStage) -> bool {
    matches!(stage, RepoStage::Unstaged | RepoStage::PartiallyStaged)
  }

  pub(super) fn sidebar_toggle_stage_action(
    stage: RepoStage,
    split_sections: bool,
    is_staged_section: bool,
  ) -> FileStageButtonAction {
    if split_sections {
      if is_staged_section {
        FileStageButtonAction::Unstage
      } else {
        FileStageButtonAction::Stage
      }
    } else if Self::selected_file_can_unstage(stage) {
      FileStageButtonAction::Unstage
    } else {
      FileStageButtonAction::Stage
    }
  }

  pub(super) fn stage_requires_confirmation(status: RepoStatusKind) -> bool {
    status == RepoStatusKind::Conflicted
  }

  pub(super) fn should_confirm_stage_for_status(
    status: Option<RepoStatusKind>,
    has_unresolved_conflict_markers: bool,
  ) -> bool {
    status.is_some_and(Self::stage_requires_confirmation) && has_unresolved_conflict_markers
  }

  pub(super) fn all_entries_staged(entries: &[RepoStatusEntry]) -> bool {
    !entries.is_empty() && entries.iter().all(|entry| entry.stage == RepoStage::Staged)
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

  pub(super) fn toggle_file_stage_action(
    &mut self,
    _: &crate::ToggleFileStage,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_repo.is_none() {
      return;
    }
    let Some(entry) = self.selected_file_entry().cloned() else {
      return;
    };
    if Self::selected_file_can_unstage(entry.stage) {
      self.unstage_file_action(entry.path, cx);
    } else {
      self.stage_file_click_action(window, entry.path, entry.status, cx);
    }
    cx.stop_propagation();
  }

  pub(super) fn restore_file_shortcut_action(
    &mut self,
    _: &crate::RestoreFile,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_repo.is_none() {
      return;
    }
    let Some(entry) = self.selected_file_entry().cloned() else {
      return;
    };
    if !Self::can_restore_file_stage(entry.stage) {
      return;
    }
    self.confirm_restore_file_action(window, entry.path, entry.old_path, entry.status, cx);
    cx.stop_propagation();
  }

  pub(super) fn toggle_stage_all_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.all_changes_staged() {
      self.unstage_all_action(cx);
    } else if Self::should_confirm_stage_all(self.selected_repo.as_ref(), &self.status_entries) {
      self.confirm_stage_all_conflicted_action(window, cx);
    } else {
      self.stage_all_action(cx);
    }
  }

  pub(super) fn restore_all_click_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.confirm_restore_all_action(window, cx);
  }

  pub(super) fn stage_all_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.add_git_breadcrumb("Stage all started", Map::new());
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || stage_all(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => this.add_git_breadcrumb("Stage all succeeded", Map::new()),
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Stage all failed", data.clone());
            this.record_git_unexpected_error("git.stage_all", error_message.as_str(), data);
            this.push_git_action_error_notification("Stage all failed", error_message.into(), cx);
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

  pub(super) fn unstage_all_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.add_git_breadcrumb("Unstage all started", Map::new());
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || unstage_all(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => this.add_git_breadcrumb("Unstage all succeeded", Map::new()),
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Unstage all failed", data.clone());
            this.record_git_unexpected_error("git.unstage_all", error_message.as_str(), data);
            this.push_git_action_error_notification("Unstage all failed", error_message.into(), cx);
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

  pub(super) fn stage_file_click_action(
    &mut self,
    window: &mut Window,
    rel_path: PathBuf,
    status: RepoStatusKind,
    cx: &mut Context<Self>,
  ) {
    if Self::should_confirm_stage_for_status(
      Some(status),
      self.open_editor_has_unresolved_conflict_markers(cx),
    ) {
      self.confirm_stage_conflicted_file_action(window, rel_path, cx);
    } else {
      self.stage_file_action(rel_path, cx);
    }
  }

  pub(super) fn stage_file_action(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let rel_path_label = rel_path.to_string_lossy().replace(['\n', '\r'], "");
    let mut start_data = Map::new();
    start_data.insert("file".into(), rel_path_label.clone().into());
    self.add_git_breadcrumb("Stage file started", start_data);
    let rel_path_for_job = rel_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || stage_file(&repo_root, &rel_path_for_job)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => this.add_git_breadcrumb("Stage file succeeded", Map::new()),
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            data.insert("file".into(), rel_path_label.clone().into());
            this.add_git_breadcrumb("Stage file failed", data.clone());
            this.record_git_unexpected_error("git.stage_file", error_message.as_str(), data);
            this.push_git_action_error_notification(
              format!("Failed to stage {rel_path_label}"),
              error_message.into(),
              cx,
            );
          }
        }
        this.reload_status(cx);
        if Self::should_refresh_editor_for_path(this.selected_file.as_deref(), &rel_path)
          && let Some(editor) = this.editor.clone()
        {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  pub(super) fn unstage_file_action(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let rel_path_label = rel_path.to_string_lossy().replace(['\n', '\r'], "");
    let mut start_data = Map::new();
    start_data.insert("file".into(), rel_path_label.clone().into());
    self.add_git_breadcrumb("Unstage file started", start_data);
    let rel_path_for_job = rel_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || unstage_file(&repo_root, &rel_path_for_job)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => this.add_git_breadcrumb("Unstage file succeeded", Map::new()),
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            data.insert("file".into(), rel_path_label.clone().into());
            this.add_git_breadcrumb("Unstage file failed", data.clone());
            this.record_git_unexpected_error("git.unstage_file", error_message.as_str(), data);
            this.push_git_action_error_notification(
              format!("Failed to unstage {rel_path_label}"),
              error_message.into(),
              cx,
            );
          }
        }
        this.reload_status(cx);
        if Self::should_refresh_editor_for_path(this.selected_file.as_deref(), &rel_path)
          && let Some(editor) = this.editor.clone()
        {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  pub(super) fn restore_file_click_action(
    &mut self,
    window: &mut Window,
    rel_path: PathBuf,
    old_path: Option<PathBuf>,
    status: RepoStatusKind,
    cx: &mut Context<Self>,
  ) {
    self.confirm_restore_file_action(window, rel_path, old_path, status, cx);
  }

  pub(super) fn restore_file_action(
    &mut self,
    rel_path: PathBuf,
    old_path: Option<PathBuf>,
    status: RepoStatusKind,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let rel_path_for_job = rel_path.clone();
    let old_path_for_job = old_path.clone();
    let should_delete = Self::restore_uses_delete(status);
    let is_rename_restore = status == RepoStatusKind::Renamed && old_path.is_some();
    let rel_path_label = rel_path.to_string_lossy().replace(['\n', '\r'], "");
    let mut start_data = Map::new();
    start_data.insert("file".into(), rel_path_label.clone().into());
    start_data.insert("delete".into(), should_delete.into());
    self.add_git_breadcrumb("Restore file started", start_data);
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        if should_delete {
          delete_untracked_file(&repo_root, &rel_path_for_job)
        } else if is_rename_restore {
          let old = old_path_for_job.as_deref().expect("rename has old_path");
          restore_renamed_file(&repo_root, old, &rel_path_for_job)
        } else {
          restore_file(&repo_root, &rel_path_for_job)
        }
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            this.select_first_file_after_restore = true;
            let mut data = Map::new();
            data.insert("delete".into(), should_delete.into());
            this.add_git_breadcrumb("Restore file succeeded", data);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("delete".into(), should_delete.into());
            data.insert("file".into(), rel_path_label.clone().into());
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Restore file failed", data.clone());
            this.record_git_unexpected_error("git.restore_file", error_message.as_str(), data);
            this.push_git_action_error_notification(
              format!("Failed to restore {rel_path_label}"),
              error_message.into(),
              cx,
            );
          }
        }
        this.reload_status(cx);
        if Self::should_refresh_editor_for_path(this.selected_file.as_deref(), &rel_path)
          && let Some(editor) = this.editor.clone()
        {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  pub(super) fn restore_all_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if self.status_entries.is_empty() {
      return;
    }
    self.add_git_breadcrumb("Restore all started", Map::new());
    let entries = self.status_entries.clone();
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let first_error = unblock(move || {
        let mut first_error = None;
        for entry in entries {
          let result = if Self::restore_uses_delete(entry.status) {
            delete_untracked_file(&repo_root, &entry.path)
          } else if entry.status == RepoStatusKind::Renamed
            && let Some(old_path) = entry.old_path.as_deref()
          {
            restore_renamed_file(&repo_root, old_path, &entry.path)
          } else {
            restore_file(&repo_root, &entry.path)
          };
          if let Err(error) = result
            && first_error.is_none()
          {
            first_error = Some(error.to_string());
          }
        }
        first_error
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        if let Some(error_message) = first_error {
          let mut data = Map::new();
          data.insert("error".into(), error_message.clone().into());
          this.add_git_breadcrumb("Restore all completed with errors", data.clone());
          this.record_git_unexpected_error("git.restore_all", error_message.as_str(), data);
          this.push_git_action_error_notification("Restore all failed", error_message.into(), cx);
        } else {
          this.add_git_breadcrumb("Restore all succeeded", Map::new());
        }
        this.select_first_file_after_restore = true;
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  pub(super) fn confirm_stage_conflicted_file_action(
    &mut self,
    window: &mut Window,
    rel_path: PathBuf,
    cx: &mut Context<Self>,
  ) {
    let file_label = rel_path.to_string_lossy().replace(['\n', '\r'], "");
    let title: SharedString = "Mark conflicts as resolved?".into();
    let message: SharedString = format!(
      "Stage {} and mark its merge conflicts as resolved?",
      file_label
    )
    .into();
    let view = cx.entity();
    let rel_path_for_action = rel_path.clone();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let rel_path_for_action = rel_path_for_action.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Stage")
        .cancel_text("Cancel")
        .on_confirm(move |_, _, cx| {
          let rel_path_for_action = rel_path_for_action.clone();
          view.update(cx, |view, cx| {
            view.stage_file_action(rel_path_for_action, cx);
          });
          true
        })
        .build(alert)
    });
  }

  pub(super) fn confirm_stage_all_conflicted_action(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let title: SharedString = "Mark conflicts as resolved?".into();
    let message: SharedString = "Stage all files and mark merge conflicts as resolved?".into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Stage all")
        .cancel_text("Cancel")
        .on_confirm(move |_, _, cx| {
          view.update(cx, |view, cx| {
            view.stage_all_action(cx);
          });
          true
        })
        .build(alert)
    });
  }

  pub(super) fn confirm_restore_file_action(
    &mut self,
    window: &mut Window,
    rel_path: PathBuf,
    old_path: Option<PathBuf>,
    status: RepoStatusKind,
    cx: &mut Context<Self>,
  ) {
    let file_label = rel_path.to_string_lossy().replace(['\n', '\r'], "");
    let (title, message, confirm_text) = if status == RepoStatusKind::Untracked {
      (
        "Delete file?",
        format!("Delete {} from disk?", file_label),
        "Delete",
      )
    } else {
      (
        "Restore file?",
        format!("Discard changes in {}?", file_label),
        "Restore",
      )
    };

    let title: SharedString = title.into();
    let message: SharedString = message.into();
    let confirm_text: SharedString = confirm_text.into();
    let view = cx.entity();
    let rel_path_for_action = rel_path.clone();
    let old_path_for_action = old_path.clone();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let rel_path_for_action = rel_path_for_action.clone();
      let old_path_for_action = old_path_for_action.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text(confirm_text.clone())
        .cancel_text("Cancel")
        .destructive()
        .on_confirm(move |_, _, cx| {
          let rel_path_for_action = rel_path_for_action.clone();
          let old_path_for_action = old_path_for_action.clone();
          view.update(cx, |view, cx| {
            view.restore_file_action(rel_path_for_action, old_path_for_action, status, cx);
          });
          true
        })
        .build(alert)
    });
  }

  pub(super) fn confirm_restore_all_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.selected_repo.is_none() || self.status_entries.is_empty() {
      return;
    }
    let has_untracked = Self::has_untracked_entries(&self.status_entries);
    let title: SharedString = "Restore all files?".into();
    let message: SharedString = if has_untracked {
      "Discard all tracked changes and delete all untracked files?".into()
    } else {
      "Discard all changes in the repository?".into()
    };
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Restore all")
        .cancel_text("Cancel")
        .destructive()
        .on_confirm(move |_, _, cx| {
          view.update(cx, |view, cx| {
            view.restore_all_action(cx);
          });
          true
        })
        .build(alert)
    });
  }

  pub(super) fn stage_style(
    stage: RepoStage,
    theme: &gpui_component::Theme,
  ) -> (IconName, gpui::Hsla, Option<SharedString>) {
    match stage {
      RepoStage::Staged => (
        IconName::CircleCheck,
        theme.status_green(),
        Some("Staged".into()),
      ),
      RepoStage::PartiallyStaged => (
        IconName::CircleCheck,
        theme.status_orange(),
        Some("Partially staged".into()),
      ),
      RepoStage::Unstaged => (IconName::Minus, theme.muted_foreground, None),
    }
  }

  pub(super) fn should_confirm_stage_all(
    selected_repo: Option<&PathBuf>,
    status_entries: &[RepoStatusEntry],
  ) -> bool {
    selected_repo.is_some() && Self::has_conflicted_entries(status_entries)
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

  #[test]
  fn should_confirm_stage_all_when_repo_selected_and_conflicts_present() {
    let repo_path = PathBuf::from("/tmp/reviu-stage-all");
    let conflicted_entries = vec![RepoStatusEntry {
      path: PathBuf::from("README.md"),
      old_path: None,
      status: RepoStatusKind::Conflicted,
      stage: RepoStage::Unstaged,
    }];
    let clean_entries = vec![make_status_entry("src/a.rs", RepoStage::Unstaged)];

    assert!(GitPage::should_confirm_stage_all(
      Some(&repo_path),
      &conflicted_entries
    ));
    assert!(!GitPage::should_confirm_stage_all(
      None,
      &conflicted_entries
    ));
    assert!(!GitPage::should_confirm_stage_all(
      Some(&repo_path),
      &clean_entries
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
  async fn stage_restore_actions_require_selected_repo(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = None;

      this.stage_all_action(cx);
      assert!(this.status_task.is_none());

      this.unstage_all_action(cx);
      assert!(this.status_task.is_none());

      this.stage_file_action(PathBuf::from("README.md"), cx);
      assert!(this.status_task.is_none());

      this.unstage_file_action(PathBuf::from("README.md"), cx);
      assert!(this.status_task.is_none());

      this.restore_file_action(
        PathBuf::from("README.md"),
        None,
        RepoStatusKind::Modified,
        cx,
      );
      assert!(this.status_task.is_none());
    });
  }

  #[gpui::test]
  async fn stage_all_action_stages_all_modified_entries(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-stage-all-success");
    let first = Path::new("a.txt");
    let second = Path::new("b.txt");
    let _ = commit_text_file(&repo.path, first, "a1\n", "first");
    let _ = commit_text_file(&repo.path, second, "b1\n", "second");
    std::fs::write(repo.path.join(first), "a2\n").expect("modify first");
    std::fs::write(repo.path.join(second), "b2\n").expect("modify second");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.stage_all_action(cx);
      this.status_task.take().expect("stage all task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after stage all");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.stage == RepoStage::Staged));
    let has_staged = git_page.read_with(cx, |this, _| this.has_staged_changes);
    assert!(has_staged);
  }

  #[gpui::test]
  async fn unstage_all_action_unstages_all_modified_entries(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-unstage-all-success");
    let first = Path::new("a.txt");
    let second = Path::new("b.txt");
    let _ = commit_text_file(&repo.path, first, "a1\n", "first");
    let _ = commit_text_file(&repo.path, second, "b1\n", "second");
    std::fs::write(repo.path.join(first), "a2\n").expect("modify first");
    std::fs::write(repo.path.join(second), "b2\n").expect("modify second");
    stage_all(&repo.path).expect("stage all before ui action");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.unstage_all_action(cx);
      this.status_task.take().expect("unstage all task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after unstage all");
    assert_eq!(entries.len(), 2);
    assert!(
      entries
        .iter()
        .all(|entry| entry.stage == RepoStage::Unstaged)
    );
    let has_staged = git_page.read_with(cx, |this, _| this.has_staged_changes);
    assert!(!has_staged);
  }

  #[gpui::test]
  async fn toggle_stage_all_action_unstages_when_all_entries_are_staged(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-toggle-stage-all-to-unstage");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");
    stage_all(&repo.path).expect("stage all before toggle action");
    let staged_entries = list_repo_status(&repo.path).expect("list staged status");
    assert!(
      staged_entries
        .iter()
        .all(|entry| entry.stage == RepoStage::Staged)
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = staged_entries.clone();
      this.toggle_stage_all_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("toggle stage-all task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let entries = list_repo_status(&repo.path).expect("list status after toggle unstage");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn toggle_stage_all_action_stages_when_any_entry_is_unstaged(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-toggle-stage-all-to-stage");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");
    let unstaged_entries = list_repo_status(&repo.path).expect("list unstaged status");
    assert!(
      unstaged_entries
        .iter()
        .all(|entry| entry.stage == RepoStage::Unstaged)
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = unstaged_entries.clone();
      this.toggle_stage_all_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("toggle stage-all task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let entries = list_repo_status(&repo.path).expect("list status after toggle stage");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Staged);
  }

  #[gpui::test]
  async fn stage_file_action_stages_only_target_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-stage-file-success");
    let first = Path::new("a.txt");
    let second = Path::new("b.txt");
    let _ = commit_text_file(&repo.path, first, "a1\n", "first");
    let _ = commit_text_file(&repo.path, second, "b1\n", "second");
    std::fs::write(repo.path.join(first), "a2\n").expect("modify first");
    std::fs::write(repo.path.join(second), "b2\n").expect("modify second");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.stage_file_action(first.to_path_buf(), cx);
      this.status_task.take().expect("stage file task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after stage file");
    let first_entry = entries
      .iter()
      .find(|entry| entry.path == first)
      .expect("first entry");
    let second_entry = entries
      .iter()
      .find(|entry| entry.path == second)
      .expect("second entry");
    assert_eq!(first_entry.stage, RepoStage::Staged);
    assert_eq!(second_entry.stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn stage_file_action_with_missing_path_keeps_existing_status(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-stage-file-missing");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify tracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.stage_file_action(PathBuf::from("missing.txt"), cx);
      this.status_task.take().expect("stage missing file task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let entries = list_repo_status(&repo.path).expect("status after stage missing file");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn stage_file_action_failure_shows_error_notification(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let missing_repo = TempDir::new("git-page-stage-file-failure-notification");
    let missing_repo_path = missing_repo.path.clone();
    std::fs::remove_dir_all(&missing_repo.path).expect("remove temp dir");

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
      this.selected_repo = Some(missing_repo_path.clone());
      this.stage_file_action(PathBuf::from("README.md"), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
  }

  #[gpui::test]
  async fn unstage_file_action_unstages_target_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-unstage-file-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");
    stage_file(&repo.path, rel_path).expect("stage file before ui action");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.unstage_file_action(rel_path.to_path_buf(), cx);
      this.status_task.take().expect("unstage file task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after unstage file");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn unstage_file_action_with_missing_path_keeps_existing_status(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-unstage-file-missing");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify tracked file");
    stage_file(&repo.path, rel_path).expect("stage tracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.unstage_file_action(PathBuf::from("missing.txt"), cx);
      this.status_task.take().expect("unstage missing file task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let entries = list_repo_status(&repo.path).expect("status after unstage missing file");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Staged);
  }

  #[gpui::test]
  async fn restore_file_action_reverts_modified_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-file-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(rel_path.to_path_buf(), None, RepoStatusKind::Modified, cx);
      this.status_task.take().expect("restore file task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let contents = std::fs::read_to_string(repo.path.join(rel_path)).expect("read restored file");
    assert_eq!(contents, "v1\n");
    assert!(
      list_repo_status(&repo.path)
        .expect("status after restore")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn restore_file_action_with_missing_path_keeps_existing_changes(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-file-missing");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify tracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(
        PathBuf::from("missing.txt"),
        None,
        RepoStatusKind::Modified,
        cx,
      );
      this.status_task.take().expect("restore missing file task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let contents = std::fs::read_to_string(repo.path.join(rel_path)).expect("read modified file");
    assert_eq!(contents, "v2\n");
    let entries = list_repo_status(&repo.path).expect("status after restore missing file");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn restore_file_action_deletes_untracked_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-delete-untracked-success");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let rel_path = Path::new("notes.txt");
    let absolute = repo.path.join(rel_path);
    std::fs::write(&absolute, "temporary\n").expect("write untracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(rel_path.to_path_buf(), None, RepoStatusKind::Untracked, cx);
      this.status_task.take().expect("delete untracked task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert!(!absolute.exists());
    assert!(
      list_repo_status(&repo.path)
        .expect("status after delete")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn restore_file_action_restores_deleted_tracked_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-deleted-file");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    let absolute = repo.path.join(rel_path);
    std::fs::remove_file(&absolute).expect("delete tracked file in worktree");

    let entries_before = list_repo_status(&repo.path).expect("list status before restore");
    assert_eq!(entries_before.len(), 1);
    assert_eq!(entries_before[0].path, rel_path);
    assert_eq!(entries_before[0].status, RepoStatusKind::Deleted);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(rel_path.to_path_buf(), None, RepoStatusKind::Deleted, cx);
      this.status_task.take().expect("restore deleted file task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(absolute.exists());
    let contents = std::fs::read_to_string(&absolute).expect("read restored tracked file");
    assert_eq!(contents, "v1\n");
    assert!(
      list_repo_status(&repo.path)
        .expect("status after deleted restore")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn restore_file_action_undoes_rename(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-rename");
    let old_path = Path::new("old.txt");
    let new_path = Path::new("new.txt");
    let _ = commit_text_file(&repo.path, old_path, "v1\n", "initial");
    std::fs::rename(repo.path.join(old_path), repo.path.join(new_path))
      .expect("rename file in worktree");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(
        new_path.to_path_buf(),
        Some(old_path.to_path_buf()),
        RepoStatusKind::Renamed,
        cx,
      );
      this.status_task.take().expect("restore rename task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(repo.path.join(old_path).exists());
    assert!(!repo.path.join(new_path).exists());
    let contents =
      std::fs::read_to_string(repo.path.join(old_path)).expect("read restored old file");
    assert_eq!(contents, "v1\n");
    assert!(
      list_repo_status(&repo.path)
        .expect("status after rename restore")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn restore_file_action_selects_first_remaining_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-select-first-remaining");
    let first_path = Path::new("a-first.txt");
    let second_path = Path::new("b-second.txt");
    let _ = commit_text_file(&repo.path, first_path, "v1\n", "initial first");
    let _ = commit_text_file(&repo.path, second_path, "v1\n", "initial second");
    std::fs::write(repo.path.join(first_path), "first change\n").expect("modify first file");
    std::fs::write(repo.path.join(second_path), "second change\n").expect("modify second file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (restore_path, expected_first_remaining_path) = git_page.read_with(cx, |this, _| {
      assert_eq!(
        this.status_entries.len(),
        2,
        "expected two modified files before restore"
      );
      (
        this.status_entries[1].path.clone(),
        this.status_entries[0].path.clone(),
      )
    });

    git_page.update_in(cx, |this, _window, cx| {
      this.open_file(restore_path.clone(), cx);
    });

    let restore_task = git_page.update_in(cx, |this, _window, cx| {
      this.restore_file_action(restore_path.clone(), None, RepoStatusKind::Modified, cx);
      this.status_task.take().expect("restore file task")
    });
    restore_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_file, entries_len, first_remaining_path) = git_page.read_with(cx, |this, _| {
      (
        this.selected_file.clone(),
        this.status_entries.len(),
        this.status_entries.first().map(|entry| entry.path.clone()),
      )
    });

    assert_eq!(entries_len, 1);
    assert_eq!(first_remaining_path, Some(expected_first_remaining_path));
    assert_eq!(selected_file, first_remaining_path);
  }

  #[gpui::test]
  async fn restore_all_action_restores_tracked_and_deletes_untracked(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-all");
    let tracked_path = Path::new("README.md");
    let untracked_path = Path::new("notes.txt");
    let _ = commit_text_file(&repo.path, tracked_path, "v1\n", "initial");
    std::fs::write(repo.path.join(tracked_path), "v2\n").expect("modify tracked file");
    std::fs::write(repo.path.join(untracked_path), "temporary\n").expect("write untracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let restore_all_task = git_page.update_in(cx, |this, _window, cx| {
      this.restore_all_action(cx);
      this.status_task.take().expect("restore all task")
    });
    restore_all_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      std::fs::read_to_string(repo.path.join(tracked_path)).expect("read tracked file"),
      "v1\n"
    );
    assert!(!repo.path.join(untracked_path).exists());
    assert!(
      list_repo_status(&repo.path)
        .expect("status after restore all")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn restore_all_action_undoes_renamed_files(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-all-rename");
    let old_path = Path::new("old.txt");
    let new_path = Path::new("new.txt");
    let _ = commit_text_file(&repo.path, old_path, "v1\n", "initial");
    // Stage the rename so libgit2 reports it as a single Renamed entry.
    std::fs::rename(repo.path.join(old_path), repo.path.join(new_path))
      .expect("rename file in worktree");
    stage_all(&repo.path).expect("stage rename");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let entries_before = git_page.read_with(cx, |this, _| this.status_entries.clone());
    assert_eq!(entries_before.len(), 1);
    assert_eq!(entries_before[0].status, RepoStatusKind::Renamed);
    assert_eq!(entries_before[0].path, new_path);
    assert_eq!(entries_before[0].old_path.as_deref(), Some(old_path));

    let restore_all_task = git_page.update_in(cx, |this, _window, cx| {
      this.restore_all_action(cx);
      this.status_task.take().expect("restore all task")
    });
    restore_all_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(repo.path.join(old_path).exists());
    assert!(!repo.path.join(new_path).exists());
    assert_eq!(
      std::fs::read_to_string(repo.path.join(old_path)).expect("read restored file"),
      "v1\n"
    );
    assert!(
      list_repo_status(&repo.path)
        .expect("status after restore all rename")
        .is_empty()
    );
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

  #[test]
  fn restore_uses_delete_only_for_untracked_entries() {
    assert!(GitPage::restore_uses_delete(RepoStatusKind::Untracked));
    assert!(!GitPage::restore_uses_delete(RepoStatusKind::Modified));
    assert!(!GitPage::restore_uses_delete(RepoStatusKind::Added));
    assert!(!GitPage::restore_uses_delete(RepoStatusKind::Deleted));
  }

  #[test]
  fn stage_requires_confirmation_only_for_conflicted_entries() {
    assert!(GitPage::stage_requires_confirmation(
      RepoStatusKind::Conflicted
    ));
    assert!(!GitPage::stage_requires_confirmation(
      RepoStatusKind::Modified
    ));
    assert!(!GitPage::stage_requires_confirmation(RepoStatusKind::Added));
  }

  #[test]
  fn should_confirm_stage_for_status_only_when_conflicts_are_unresolved() {
    assert!(GitPage::should_confirm_stage_for_status(
      Some(RepoStatusKind::Conflicted),
      true
    ));
    assert!(!GitPage::should_confirm_stage_for_status(
      Some(RepoStatusKind::Conflicted),
      false
    ));
    assert!(!GitPage::should_confirm_stage_for_status(
      Some(RepoStatusKind::Modified),
      true
    ));
    assert!(!GitPage::should_confirm_stage_for_status(None, true));
  }

  #[test]
  fn sidebar_toggle_stage_action_preserves_partial_split_behavior() {
    assert_eq!(
      GitPage::sidebar_toggle_stage_action(RepoStage::Unstaged, false, false),
      FileStageButtonAction::Stage
    );
    assert_eq!(
      GitPage::sidebar_toggle_stage_action(RepoStage::Staged, false, false),
      FileStageButtonAction::Unstage
    );
    assert_eq!(
      GitPage::sidebar_toggle_stage_action(RepoStage::PartiallyStaged, false, false),
      FileStageButtonAction::Unstage
    );
    assert_eq!(
      GitPage::sidebar_toggle_stage_action(RepoStage::PartiallyStaged, true, false),
      FileStageButtonAction::Stage
    );
    assert_eq!(
      GitPage::sidebar_toggle_stage_action(RepoStage::PartiallyStaged, true, true),
      FileStageButtonAction::Unstage
    );
  }

  #[test]
  fn all_changes_staged_requires_non_empty_and_only_staged_entries() {
    assert!(!GitPage::all_entries_staged(&[]));

    let all_staged = vec![
      make_status_entry("src/a.rs", RepoStage::Staged),
      make_status_entry("src/b.rs", RepoStage::Staged),
    ];
    assert!(GitPage::all_entries_staged(&all_staged));

    let mixed = vec![
      make_status_entry("src/a.rs", RepoStage::Staged),
      make_status_entry("src/b.rs", RepoStage::Unstaged),
    ];
    assert!(!GitPage::all_entries_staged(&mixed));

    let partial = vec![make_status_entry("src/a.rs", RepoStage::PartiallyStaged)];
    assert!(!GitPage::all_entries_staged(&partial));
  }
}
