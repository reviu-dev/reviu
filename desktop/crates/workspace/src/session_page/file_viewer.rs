//! The open file at the centre: diff, commit snapshot, previews and the
//! editor-side actions. The candidate surface for the PR-page merge (#542).

use super::*;

impl SessionPage {
  pub(super) fn open_diff(
    &mut self,
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    // Previewing is a detour, not a mode: opening a file always shows its code.
    self.show_preview = false;
    let left_commit_file = self.leave_commit_file(cx);
    let app_settings = crate::config::AppSettings::get(cx);
    self.diff_view = if app_settings.split_diff_view {
      DiffViewMode::Split
    } else {
      DiffViewMode::Inline
    };
    let diff_view = self.effective_diff_view(&rel_path, cx);
    // Reading preference for the session, seeded from the settings once.
    if self.selected_file.is_none() {
      self.hide_whitespace = app_settings.hide_whitespace;
    }
    let hide_whitespace = self.hide_whitespace;
    // Agent line numbers are 1-based; the editor reveals by 0-based doc line.
    let reveal_doc_line = reveal_line.map(|line| line.saturating_sub(1) as usize);

    self.center = CenterView::Diff;
    // Same path, but the snapshot of a commit is not the working-tree file.
    if !left_commit_file && self.selected_file.as_ref() == Some(&rel_path) && self.editor.is_some()
    {
      if let (Some(doc_line), Some(editor)) = (reveal_doc_line, self.editor.clone()) {
        editor.update(cx, |editor, cx| editor.reveal_source_line(doc_line, cx));
      }
      self.focus_editor_on_next_frame(window, cx);
      cx.notify();
      return;
    }

    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    let generation = self.open_file_generation;
    self.selected_file = Some(rel_path.clone());
    self.editor = None;
    self.binary_preview = None;

    let file_path = repo_root.join(&rel_path);
    let load_repo_root = repo_root.clone();
    let load_file_path = file_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let loaded = cx
        .background_spawn(
          async move { Editor::load_file_for_editor(&load_repo_root, &load_file_path) },
        )
        .await;
      let _ = this.update(cx, move |this, cx| {
        if this.open_file_generation != generation {
          return;
        }
        if this.selected_file.as_ref() != Some(&rel_path) {
          return;
        }
        let binary_preview = build_binary_preview(rel_path.as_path(), loaded.binary_bytes.clone());
        let editor =
          cx.new(move |cx| Editor::new_with_loaded_file(repo_root, file_path, loaded, cx));
        editor.update(cx, |editor, cx| {
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_whitespace, cx);
          if let Some(doc_line) = reveal_doc_line {
            editor.reveal_source_line(doc_line, cx);
          }
        });
        this.binary_preview = binary_preview;
        this.editor = Some(editor.clone());
        this.sync_editor_unmerged_state(cx);
        this.sync_git_telemetry(cx);
        // Focus once loaded: the requester (file tree, list, search) may still hold
        // focus, and there was no editor to focus when the open was requested.
        if this.center == CenterView::Diff {
          let _ = cx.update_window(this.window_handle, |_, window, cx| {
            let focus_handle = editor.read(cx).focus_handle(cx);
            window.focus(&focus_handle, cx);
          });
        }
        this.install_agent_review_handlers_for_editor(&editor, cx);
        this.sync_agent_review_comments_to_editor(cx);
        cx.subscribe(
          &editor,
          |this, _editor, event: &EditorEvent, cx| match event {
            EditorEvent::Saved | EditorEvent::HunkStagingChanged => {
              this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
            }
          },
        )
        .detach();
        cx.notify();
      });
    });
    self.open_file_task = Some(task);
    self.focus_editor_on_next_frame(window, cx);
    cx.notify();
  }

  /// Split needs two sides to compare: a whole-file change or a binary preview
  /// falls back to inline.
  /// A file as it was in a commit: a read-only snapshot with its own patch.
  pub(super) fn open_commit_file(
    &mut self,
    commit_oid: String,
    rel_path: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.show_preview = false;
    self.center = CenterView::Diff;
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    let generation = self.open_file_generation;
    self.selected_file = Some(rel_path.clone());
    self.opened_commit = Some(commit_oid.clone());
    self.editor = None;
    self.binary_preview = None;
    let hide_whitespace = self.hide_whitespace;
    let diff_view = self.effective_diff_view(&rel_path, cx);

    let task = cx.spawn(async move |this, cx| {
      let load_repo_root = repo_root.clone();
      let load_commit_oid = commit_oid.clone();
      let load_rel_path = rel_path.clone();
      let commit_file = cx
        .background_spawn(async move {
          git::load_commit_file_diff(&load_repo_root, &load_commit_oid, &load_rel_path)
        })
        .await;
      let _ = this.update(cx, move |this, cx| {
        if this.open_file_generation != generation {
          return;
        }
        let Ok(commit_file) = commit_file else {
          return;
        };

        let file_path = repo_root.join(&rel_path);
        let editor = cx.new(|cx| Editor::new_with_paths(repo_root.clone(), file_path, cx));
        let diff_set = if commit_file.patch.trim().is_empty() {
          None
        } else {
          git::diff_set_from_patch(&commit_file.patch).ok()
        };
        editor.update(cx, |editor, cx| {
          editor.load_readonly_snapshot(commit_file.content, diff_set, cx);
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_whitespace, cx);
        });
        this.binary_preview =
          build_binary_preview(rel_path.as_path(), commit_file.binary_bytes.clone());
        this.editor = Some(editor);
        this.svg_preview.update(cx, |preview, _| preview.clear());
        cx.notify();
      });
    });
    self.open_file_task = Some(task);
    self.focus_editor_on_next_frame(window, cx);
    cx.notify();
  }

  /// Back to the working tree: the history row stops being the open one.
  pub(super) fn leave_commit_file(&mut self, cx: &mut Context<Self>) -> bool {
    if self.opened_commit.take().is_none() {
      return false;
    }
    let history = self.dock_panel.read(cx).history_list.clone();
    history.update(cx, |list, cx| list.set_opened(None, cx));
    true
  }

  pub(super) fn effective_diff_view(&self, path: &Path, cx: &App) -> DiffViewMode {
    // A clean file has no other side: the split preference must not follow it.
    if !self.path_has_changes(path, cx) {
      return DiffViewMode::Inline;
    }
    effective_diff_view(DiffViewInputs {
      preferred: self.diff_view,
      binary_preview: self.binary_preview.is_some(),
      previewing: self.show_preview && self.previewable(),
      whole_file_change: self.whole_file_change(path, cx),
    })
  }

  /// A file opened from the Files tab with no pending change has nothing to
  /// compare: the toggle would show the same content twice.
  pub(super) fn selected_file_has_changes(&self, cx: &App) -> bool {
    let Some(path) = self.selected_file.as_deref() else {
      return false;
    };
    self.path_has_changes(path, cx)
  }

  fn path_has_changes(&self, path: &Path, cx: &App) -> bool {
    // A commit snapshot always carries its own patch.
    if self.opened_commit.is_some() {
      return true;
    }
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .any(|entry| entry.path == path)
  }

  pub(super) fn selected_file_is_markdown(&self) -> bool {
    self
      .selected_file
      .as_deref()
      .is_some_and(crate::file_preview::is_markdown_path)
  }

  pub(super) fn selected_file_is_svg(&self) -> bool {
    self
      .selected_file
      .as_deref()
      .is_some_and(crate::file_preview::is_svg_path)
  }

  pub(super) fn previewable(&self) -> bool {
    self.selected_file_is_markdown() || self.selected_file_is_svg()
  }

  pub(super) fn toggle_preview(&mut self, cx: &mut Context<Self>) {
    if !self.previewable() {
      self.show_preview = false;
      return;
    }
    self.show_preview = !self.show_preview;
    self.sync_diff_view(cx);
    self.sync_git_telemetry(cx);
    cx.notify();
  }

  pub(super) fn split_disabled(&self, cx: &App) -> bool {
    let Some(path) = self.selected_file.as_deref() else {
      return true;
    };
    // The preview is not a reason to refuse: asking for split closes it.
    self.binary_preview.is_some() || self.whole_file_change(path, cx)
  }

  pub(super) fn whole_file_change(&self, path: &Path, cx: &App) -> bool {
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .any(|entry| {
        entry.path == path
          && matches!(
            entry.status,
            git::RepoStatusKind::Untracked
              | git::RepoStatusKind::Added
              | git::RepoStatusKind::Deleted
          )
      })
  }

  pub(super) fn toggle_diff_view_action(
    &mut self,
    _: &crate::ToggleDiffView,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_diff_view(cx);
    cx.stop_propagation();
  }

  pub(super) fn previous_annotation_action(
    &mut self,
    _: &crate::PreviousAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.navigate_change(AnnotationDirection::Previous, cx);
  }

  pub(super) fn next_annotation_action(
    &mut self,
    _: &crate::NextAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.navigate_change(AnnotationDirection::Next, cx);
  }

  pub(super) fn navigate_change(&mut self, direction: AnnotationDirection, cx: &mut Context<Self>) {
    // A rendered file has nothing to walk.
    if self.center != CenterView::Diff || (self.show_preview && self.previewable()) {
      return;
    }
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let file_status = self.selected_file_status(cx);
    editor.update(cx, |editor, cx| {
      navigate_annotation(editor, file_status, direction, cx)
    });
    cx.stop_propagation();
  }

  /// The status of the open file, unless it comes from a commit: a snapshot has none.
  pub(super) fn selected_file_status(&self, cx: &App) -> Option<RepoStatusKind> {
    if self.opened_commit.is_some() {
      return None;
    }
    let path = self.selected_file.as_deref()?;
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .find(|entry| entry.path == path)
      .map(|entry| entry.status)
  }

  /// A conflicted file is shown whole: once its markers are resolved there is no
  /// diff left to read, only the file.
  pub(super) fn sync_editor_unmerged_state(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let is_unmerged = matches!(
      self.selected_file_status(cx),
      Some(RepoStatusKind::Conflicted)
    );
    editor.update(cx, |editor, cx| editor.set_is_unmerged(is_unmerged, cx));
  }

  /// The path a renamed file came from, so the diff header can name both sides.
  pub(super) fn selected_file_old_path(&self, cx: &App) -> Option<PathBuf> {
    if self.opened_commit.is_some() {
      return None;
    }
    let path = self.selected_file.as_deref()?;
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .find(|entry| entry.path == path)
      .and_then(|entry| entry.old_path.clone())
  }

  pub(super) fn annotation_navigation(&self, cx: &App) -> Option<AnnotationNavigationState> {
    let editor = self.editor.as_ref()?;
    let file_status = self.selected_file_status(cx);
    editor.read_with(cx, |editor, cx| {
      annotation_navigation_state_for(file_status, editor, cx)
    })
  }

  /// Accepting every conflict at once needs a conflicted file still holding markers.
  pub(super) fn can_accept_all_conflicts(&self, cx: &App) -> bool {
    let file_status = self.selected_file_status(cx);
    self.editor.as_ref().is_some_and(|editor| {
      editor.read_with(cx, |editor, cx| {
        can_accept_all_conflicts(
          file_status,
          editor.is_read_only,
          editor.has_unresolved_conflict_markers(cx),
        )
      })
    })
  }

  pub(super) fn resolve_all_conflicts(
    &mut self,
    resolution: ConflictResolution,
    cx: &mut Context<Self>,
  ) {
    if !self.can_accept_all_conflicts(cx) {
      return;
    }
    let Some(editor) = self.editor.clone() else {
      return;
    };
    editor.update(cx, |editor, cx| {
      editor.resolve_all_conflicts(resolution, cx)
    });
    self.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
  }

  pub(super) fn toggle_hunk_stage_action(
    &mut self,
    _: &crate::ToggleHunkStage,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.diff_editor() else {
      return;
    };
    let file_status = self.selected_file_status(cx);
    toggle_hunk_stage(&editor, file_status, cx);
    cx.stop_propagation();
  }

  pub(super) fn restore_hunk_action(
    &mut self,
    _: &crate::RestoreHunk,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.diff_editor() else {
      return;
    };
    let file_status = self.selected_file_status(cx);
    restore_hunk(&editor, file_status, cx);
    cx.stop_propagation();
  }

  pub(super) fn accept_both_conflict_action(
    &mut self,
    _: &crate::AcceptBothConflict,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.diff_editor() else {
      return;
    };
    let file_status = self.selected_file_status(cx);
    resolve_active_conflict(&editor, file_status, ConflictResolution::Both, cx);
    cx.stop_propagation();
  }

  /// The editor of the open file, unless the center shows something else or a
  /// rendered file hides the diff.
  pub(super) fn diff_editor(&self) -> Option<Entity<Editor>> {
    if self.center != CenterView::Diff || (self.show_preview && self.previewable()) {
      return None;
    }
    self.editor.clone()
  }

  pub(super) fn toggle_hide_whitespace_action(
    &mut self,
    _: &crate::ToggleHideWhitespace,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_hide_whitespace(cx);
    cx.stop_propagation();
  }

  pub(super) fn toggle_hide_whitespace(&mut self, cx: &mut Context<Self>) {
    // No diff on screen, nothing to hide: rendered file, or a file with no change.
    if self.center != CenterView::Diff
      || (self.show_preview && self.previewable())
      || !self.selected_file_has_changes(cx)
    {
      return;
    }
    self.hide_whitespace = !self.hide_whitespace;
    if let Some(editor) = self.editor.as_ref() {
      let value = self.hide_whitespace;
      editor.update(cx, |editor, cx| editor.set_ignore_whitespace(value, cx));
    }
    cx.notify();
  }

  pub(super) fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
    // While the rendered file holds the pane there is no diff to switch, and a
    // clean file must not flip the shared preference from a dead toggle.
    if self.center != CenterView::Diff
      || (self.show_preview && self.previewable())
      || self.split_disabled(cx)
      || !self.selected_file_has_changes(cx)
    {
      return;
    }

    self.diff_view = match self.diff_view {
      DiffViewMode::Inline => DiffViewMode::Split,
      DiffViewMode::Split => DiffViewMode::Inline,
    };
    // One preference for every diff surface, the shell and PR Changes alike.
    crate::config::AppSettings::update(cx, |settings| {
      settings.split_diff_view = self.diff_view == DiffViewMode::Split
    });
    self.sync_diff_view(cx);
    self.sync_git_telemetry(cx);
    cx.notify();
  }

  pub(super) fn sync_diff_view(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let Some(path) = self.selected_file.clone() else {
      return;
    };
    let diff_view = self.effective_diff_view(&path, cx);
    editor.update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
  }

  pub(super) fn close_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.center != CenterView::Diff {
      return;
    }
    self.center = CenterView::Conversation;
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  pub(super) fn focus_editor_on_next_frame(&self, window: &mut Window, cx: &mut Context<Self>) {
    let view = cx.entity().downgrade();
    window.on_next_frame(move |window, cx| {
      let _ = view.update(cx, |this, cx| {
        if let Some(editor) = this.editor.as_ref() {
          let focus_handle = editor.read(cx).focus_handle(cx);
          window.focus(&focus_handle, cx);
        }
      });
    });
  }

  /// The editor handles `cmd-f` when it has focus; this catches it when the
  /// focus sits in the dock instead.
  pub(super) fn find_action(
    &mut self,
    action: &editor::Find,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.diff_editor() else {
      return;
    };
    editor.update(cx, |editor, cx| editor::find(editor, action, window, cx));
    cx.stop_propagation();
  }

  /// The selection of the open diff becomes context for the next prompt.
  pub(super) fn add_selection_to_agent_action(
    &mut self,
    _: &crate::AddSelectionToAgent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    cx.stop_propagation();
    match self.selection_context(cx) {
      Ok((path, text)) => self.deliver_selection_context(path, text, window, cx),
      Err(reason) => window.push_notification(Notification::info(reason), cx),
    }
  }

  /// What `cmd-shift-l` would send, or why it cannot.
  pub(super) fn selection_context(&self, cx: &App) -> Result<(String, String), &'static str> {
    let Some(editor) = self.diff_editor() else {
      return Err("Open a file diff first");
    };
    let Some(text) = editor.read(cx).selected_text_for_copy(cx) else {
      return Err("Select code in the diff first");
    };
    let path = self
      .selected_file
      .as_ref()
      .map(|path| path.to_string_lossy().to_string())
      .unwrap_or_else(|| "selection".to_string());
    Ok((path, text))
  }

  /// Escape is bound to the editor's CloseFind; it bubbles up here when there was
  /// no find panel to close, which is our cue to close the file view.
  pub(super) fn close_file_view_action(
    &mut self,
    _: &editor::CloseFind,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.center != CenterView::Diff {
      return;
    }
    self.close_diff(window, cx);
    cx.stop_propagation();
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;

  #[gpui::test]
  async fn the_conversation_stays_visible_next_to_an_open_diff(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-split-view");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    assert!(
      cx.debug_bounds("session-conversation-pane").is_some(),
      "the conversation owns the center while no file is open"
    );

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    let conversation = cx
      .debug_bounds("session-conversation-pane")
      .expect("conversation still painted next to the diff");
    let editor = cx
      .debug_bounds("session-diff-editor")
      .expect("diff editor painted");
    assert!(
      conversation.right() <= editor.left() + gpui::px(1.),
      "conversation sits left of the diff: {conversation:?} vs {editor:?}"
    );

    page.update_in(cx, |page, window, cx| {
      page.close_workspace_page_action(&CloseWorkspacePage, window, cx);
    });
    cx.run_until_parked();
    assert!(
      cx.debug_bounds("session-conversation-pane").is_some(),
      "closing the file gives the conversation the full center back"
    );
  }

  #[gpui::test]
  async fn open_diff_switches_center_and_escape_returns(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-open-diff");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
      assert_eq!(page.center, CenterView::Diff);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert!(page.editor.is_some());
      assert_eq!(page.selected_file, Some(PathBuf::from("README.md")));
    });

    page.update_in(cx, |page, window, cx| {
      page.close_workspace_page_action(&CloseWorkspacePage, window, cx);
    });
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      // Editor kept for instant reopen of the same file.
      assert!(page.editor.is_some());
    });
  }

  #[gpui::test]
  async fn a_file_from_the_history_opens_read_only_in_the_center(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-history-file");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let first = git::current_head_sha(&repo.path)
      .expect("head sha")
      .expect("head sha");
    commit_text_file(&repo.path, Path::new("a.txt"), "v2\n", "second");
    std::fs::write(repo.path.join("a.txt"), "v3 working\n").expect("update worktree");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    let history = page.read_with(cx, |page, cx| page.dock_panel.read(cx).history_list.clone());
    history.update(cx, |list, cx| {
      list.open_commit_file(first.clone(), PathBuf::from("a.txt"), cx)
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert_eq!(page.center, CenterView::Diff);
      assert_eq!(page.opened_commit.as_deref(), Some(first.as_str()));
      let editor = page.editor.as_ref().expect("editor").read(cx);
      // A snapshot has no working-tree status, so it is walked change by change.
      assert!(page.selected_file_status(cx).is_none());
      // The commit content, not what the worktree holds now.
      let first_line = editor
        .document()
        .read(cx)
        .line_content(0)
        .expect("first line")
        .to_string();
      assert_eq!(first_line.trim_end(), "v1");
      assert!(editor.is_read_only, "a commit snapshot cannot be edited");
    });

    // Back to the working tree: the history row stops being the open one.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert!(page.opened_commit.is_none());
      let editor = page.editor.as_ref().expect("editor").read(cx);
      let first_line = editor
        .document()
        .read(cx)
        .line_content(0)
        .expect("first line")
        .to_string();
      assert_eq!(first_line.trim_end(), "v3 working");
      assert!(!editor.is_read_only);
    });
  }

  #[gpui::test]
  async fn a_conflicted_file_is_shown_whole_until_it_is_resolved(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-unmerged");
    // Long enough that a diff folds most of it away, unlike a whole-file view.
    let long_file = |mid: &str| {
      let mut lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
      lines[19] = mid.to_string();
      format!("{}\n", lines.join("\n"))
    };
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("base"),
      "initial",
    );
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
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("feature"),
      "feature work",
    );
    git::switch_branch(&repo.path, &base).expect("switch back");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("main"),
      "main work",
    );
    let _ = git::merge_branch(&repo.path, &feature);
    // Markers resolved by hand, but git still calls the file conflicted.
    std::fs::write(repo.path.join("a.txt"), long_file("resolved")).expect("resolve conflict");

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
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    // Whole file: every document line is visible, nothing is folded away.
    let visible_and_total = |page: &SessionPage, cx: &App| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      let projection = editor.projection().expect("projection");
      (
        projection.visible_doc_lines.len(),
        projection.doc_to_display.len(),
      )
    };

    page.read_with(cx, |page, cx| {
      let (visible, total) = visible_and_total(page, cx);
      assert_eq!(
        visible, total,
        "a conflicted file is read whole, there is no diff left in it"
      );
    });

    // Staging the resolution ends the conflict: the file goes back to a diff.
    git::stage_all(&repo.path).expect("stage the resolution");
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_editor_diff(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let (visible, total) = visible_and_total(page, cx);
      assert!(
        visible < total,
        "a resolved file is read as a diff again, got {visible} of {total} lines"
      );
    });
  }

  #[gpui::test]
  async fn a_merge_that_conflicts_switches_the_open_file_to_the_whole_view(
    cx: &mut TestAppContext,
  ) {
    let repo = TempRepo::init("session-page-unmerged-later");
    let long_file = |mid: &str| {
      let mut lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
      lines[19] = mid.to_string();
      format!("{}\n", lines.join("\n"))
    };
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("base"),
      "initial",
    );
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
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("feature"),
      "feature work",
    );
    git::switch_branch(&repo.path, &base).expect("switch back");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("main"),
      "main work",
    );

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // The file is open and clean when the merge starts.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    let visible_and_total = |page: &SessionPage, cx: &App| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      let projection = editor.projection().expect("projection");
      (
        projection.visible_doc_lines.len(),
        projection.doc_to_display.len(),
      )
    };
    page.read_with(cx, |page, cx| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      assert!(!editor.is_unmerged(), "a clean file is read as a diff");
      let (visible, total) = visible_and_total(page, cx);
      assert_eq!(visible, total, "and a file without changes shows in full");
    });

    let _ = git::merge_branch(&repo.path, &feature);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_editor_diff(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      assert!(
        editor.is_unmerged(),
        "the file the merge just broke is read whole, without reopening it"
      );
    });
  }

  #[gpui::test]
  async fn a_clean_file_opens_inline_even_with_the_split_preference_on(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-clean-file-split");
    commit_text_file(&repo.path, Path::new("clean.txt"), "same\n", "initial");
    commit_text_file(&repo.path, Path::new("dirty.txt"), "v1\n", "second");
    std::fs::write(repo.path.join("dirty.txt"), "v2\n").expect("modify file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    // Split on the modified file.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("dirty.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |page, cx| page.toggle_diff_view(cx));
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Split
      );
    });

    // A clean file from the Files tab must land inline anyway.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("clean.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Inline,
        "a clean file has no other side to show"
      );
    });

    // And the shortcut toggle is dead on it: the preference must not flip.
    page.update(cx, |page, cx| page.toggle_diff_view(cx));
    page.read_with(cx, |page, cx| {
      assert!(crate::config::AppSettings::get(cx).split_diff_view);
      assert_eq!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Inline
      );
    });
  }

  #[gpui::test]
  async fn the_view_leaves_split_when_a_save_empties_the_diff(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-split-follows-status");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("modify file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |page, cx| page.toggle_diff_view(cx));
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Split
      );
    });

    // The change goes away on disk, as an editor save reverting it would do.
    std::fs::write(repo.path.join("a.txt"), "v1\n").expect("revert file");
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Inline,
        "a clean file has nothing left to split"
      );
    });

    // And a new change brings the split preference back.
    std::fs::write(repo.path.join("a.txt"), "v3\n").expect("modify again");
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Split
      );
    });
  }
}
