//! The open file at the centre: diff, commit snapshot, previews and the
//! editor-side actions. The candidate surface for the PR-page merge (#542).

use super::*;

/// Where a read-only snapshot in the centre comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OpenedSnapshot {
  /// One commit against its parent.
  Commit(String),
  /// What a pull request proposes: its merge base against its head.
  PullRequestRange { base: String, head: String },
  /// The exact old/new text an agent reported for a tool call.
  AgentTool {
    old_text: Option<String>,
    new_text: String,
    whole_file_change: bool,
  },
}

#[derive(Clone)]
pub(super) enum UnsavedEditorAction {
  CloseDiff,
  SelectSession {
    id: String,
  },
  NewSessionIn {
    repo_root: PathBuf,
  },
  NewWorktreeSessionIn {
    repo_root: PathBuf,
    base: Option<String>,
  },
  SetFallbackRepo {
    repo_root: PathBuf,
  },
  PinCheckout {
    path: PathBuf,
  },
  FollowSessionCheckout,
  ForgetRepository {
    repo_root: PathBuf,
  },
  RunBranchCommand {
    command: RepoCommand,
  },
  OpenDiff {
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    reveal_column: Option<u32>,
    intent: OpenIntent,
  },
  OpenFile {
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    reveal_column: Option<u32>,
    intent: OpenIntent,
  },
  AgentDiffSnapshot {
    rel_path: PathBuf,
    old_text: Option<String>,
    new_text: String,
    reveal_line: Option<u32>,
    intent: OpenIntent,
  },
  CommitFile {
    commit_oid: String,
    rel_path: PathBuf,
    intent: OpenIntent,
  },
  PullRequestFile {
    base_oid: String,
    head_oid: String,
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    intent: OpenIntent,
  },
  CloseCenterTab {
    tab: CenterTab,
  },
}

fn worktree_file_modified(path: &Path) -> Option<SystemTime> {
  std::fs::metadata(path)
    .and_then(|metadata| metadata.modified())
    .ok()
}

fn agent_snapshot_diff_set(
  old_text: Option<&str>,
  new_text: &str,
  rel_path: &Path,
  ignore_whitespace: bool,
) -> Option<git::DiffSet> {
  let unstaged = git::compute_buffer_diff(
    git::DiffKind::Unstaged,
    old_text,
    new_text,
    rel_path,
    ignore_whitespace,
  )
  .ok()?;
  Some(git::DiffSet {
    uncommitted: unstaged.clone_with_kind(git::DiffKind::Uncommitted),
    unstaged,
    staged: git::FileDiff::empty(git::DiffKind::Staged),
  })
}

impl SessionPage {
  pub(super) fn open_diff(
    &mut self,
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_diff_at_position(rel_path, reveal_line, None, intent, window, cx);
  }

  pub(super) fn open_diff_at_position(
    &mut self,
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    reveal_column: Option<u32>,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.should_prompt_before_opening(&CenterTab::diff(rel_path.clone()), cx) {
      self.open_unsaved_editor_dialog(
        UnsavedEditorAction::OpenDiff {
          rel_path,
          reveal_line,
          reveal_column,
          intent,
        },
        window,
        cx,
      );
      return;
    }
    self.open_diff_without_unsaved_prompt(rel_path, reveal_line, reveal_column, intent, window, cx);
  }

  pub(super) fn open_file(
    &mut self,
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    reveal_column: Option<u32>,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.should_prompt_before_opening(&CenterTab::file(rel_path.clone()), cx) {
      self.open_unsaved_editor_dialog(
        UnsavedEditorAction::OpenFile {
          rel_path,
          reveal_line,
          reveal_column,
          intent,
        },
        window,
        cx,
      );
      return;
    }
    self.open_file_without_unsaved_prompt(rel_path, reveal_line, reveal_column, intent, window, cx);
  }

  fn open_diff_without_unsaved_prompt(
    &mut self,
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    reveal_column: Option<u32>,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_worktree_file_without_unsaved_prompt(
      CenterTab::diff(rel_path.clone()),
      rel_path,
      reveal_line,
      reveal_column,
      intent,
      true,
      window,
      cx,
    );
  }

  fn open_file_without_unsaved_prompt(
    &mut self,
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    reveal_column: Option<u32>,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_worktree_file_without_unsaved_prompt(
      CenterTab::file(rel_path.clone()),
      rel_path,
      reveal_line,
      reveal_column,
      intent,
      false,
      window,
      cx,
    );
  }

  fn open_worktree_file_without_unsaved_prompt(
    &mut self,
    tab: CenterTab,
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    reveal_column: Option<u32>,
    intent: OpenIntent,
    show_git_diff: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.cache_warm_editor();
    let Some(repo_root) = self.checkout_root(cx) else {
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
    // Reading preference for the session, seeded from the settings once.
    if self.warm_selected_file().is_none() {
      self.hide_whitespace = app_settings.hide_whitespace;
    }
    let hide_whitespace = self.hide_whitespace;
    // External file positions are 1-based; the editor reveals by 0-based position.
    let reveal_doc_position = reveal_line.map(|line| {
      (
        line.saturating_sub(1) as usize,
        reveal_column.map_or(0, |column| column.saturating_sub(1) as usize),
      )
    });

    self.center = CenterView::Diff;
    self.sync_agent_chat_close_control(cx);
    // Same path, but the snapshot of a commit is not the working-tree file.
    if !left_commit_file
      && self.editor_tab.as_ref() == Some(&tab)
      && let Some(editor) = self.warm_editor()
    {
      if let Some((doc_line, doc_column)) = reveal_doc_position {
        editor.update(cx, |editor, cx| {
          editor.reveal_source_position(doc_line, doc_column, cx)
        });
      }
      self.active_center_tab = Some(tab.clone());
      self.editor_tab = Some(tab.clone());
      self.record_recent_file(&repo_root, &rel_path);
      self.focus_editor_if_asked(intent, window, cx);
      cx.notify();
      return;
    }

    if self.restore_center_editor(&tab, &rel_path, reveal_doc_position, intent, window, cx) {
      return;
    }

    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    let generation = self.open_file_generation;
    self.record_recent_file(&repo_root, &rel_path);
    self.remember_center_tab(tab.clone());
    self.set_editor_tab_loading(tab.clone(), rel_path.clone(), None);
    let diff_view = self.effective_diff_view(&rel_path, cx);
    // Wherever the open came from (chat recap, palette, review row), the
    // Changes list highlights the file it is now showing.
    self
      .dock_panel
      .read(cx)
      .changes_list()
      .update(cx, |list, cx| {
        list.select_path(Some(rel_path.as_path()), cx);
      });

    let file_path = repo_root.join(&rel_path);
    let load_repo_root = repo_root.clone();
    let load_file_path = file_path.clone();
    let load_tab = tab.clone();
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
        if this.warm_selected_file() != Some(rel_path.as_path()) {
          return;
        }
        let binary_preview = build_binary_preview(rel_path.as_path(), loaded.binary_bytes.clone());
        let file_modified = worktree_file_modified(&file_path);
        let editor =
          cx.new(move |cx| Editor::new_with_loaded_file(repo_root, file_path, loaded, cx));
        editor.update(cx, |editor, cx| {
          editor.set_git_diff_enabled(show_git_diff, cx);
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_whitespace, cx);
          if let Some((doc_line, doc_column)) = reveal_doc_position {
            editor.reveal_source_position(doc_line, doc_column, cx);
          }
        });
        this.set_editor_tab_state(
          load_tab,
          CenterEditorState {
            selected_file: rel_path.clone(),
            file_modified,
            editor: Some(editor.clone()),
            binary_preview,
            opened_snapshot: None,
          },
        );
        this.sync_editor_unmerged_state(cx);
        this.sync_git_telemetry(cx);
        // Focus once loaded: the requester (file tree, list, search) may still hold
        // focus, and there was no editor to focus when the open was requested.
        // A browse leaves it where it is, or the arrow keys would land here.
        if this.center == CenterView::Diff && intent.takes_focus() {
          let _ = cx.update_window(this.window_handle, |_, window, cx| {
            let focus_handle = editor.read(cx).focus_handle(cx);
            window.focus(&focus_handle, cx);
          });
        }
        if show_git_diff {
          this.install_agent_review_handlers_for_editor(&editor, cx);
          this.sync_agent_review_comments_to_editor(cx);
        } else {
          configure_review(&editor, ReviewDestination::None, cx);
        }
        cx.subscribe(
          &editor,
          |this, _editor, event: &EditorEvent, cx| match event {
            EditorEvent::Saved => {
              let file_modified = this.warm_worktree_file_modified(cx);
              if let Some(state) = this.editor_tab_state_mut() {
                state.file_modified = file_modified;
              }
              this.cache_warm_editor();
              this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
            }
            EditorEvent::HunkStagingChanged => {
              this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
            }
          },
        )
        .detach();
        cx.notify();
      });
    });
    self.open_file_task = Some(task);
    self.focus_editor_if_asked(intent, window, cx);
    cx.notify();
  }

  pub(super) fn warm_editor_tab(&self) -> Option<&CenterTab> {
    self.editor_tab.as_ref()
  }

  pub(super) fn shown_editor_tab(&self) -> Option<&CenterTab> {
    if self.center != CenterView::Diff {
      return None;
    }
    self
      .active_center_tab
      .as_ref()
      .filter(|tab| matches!(tab.kind, CenterTabKind::File | CenterTabKind::Diff))
      .or_else(|| {
        self
          .warm_editor_tab()
          .filter(|tab| matches!(tab.kind, CenterTabKind::File | CenterTabKind::Diff))
      })
  }

  pub(super) fn editor_tab_state(&self) -> Option<&CenterEditorState> {
    self
      .warm_editor_tab()
      .and_then(|tab| self.editor_states.get(tab))
  }

  fn editor_tab_state_mut(&mut self) -> Option<&mut CenterEditorState> {
    let tab = self.warm_editor_tab()?.clone();
    self.editor_states.get_mut(&tab)
  }

  pub(super) fn warm_editor(&self) -> Option<Entity<Editor>> {
    self.editor_tab_state()?.editor.clone()
  }

  pub(super) fn warm_selected_file(&self) -> Option<&Path> {
    Some(self.editor_tab_state()?.selected_file.as_path())
  }

  pub(super) fn warm_binary_preview(&self) -> Option<&BinaryPreview> {
    self.editor_tab_state()?.binary_preview.as_ref()
  }

  pub(super) fn warm_opened_snapshot(&self) -> Option<&OpenedSnapshot> {
    self.editor_tab_state()?.opened_snapshot.as_ref()
  }

  fn shown_editor_state(&self) -> Option<&CenterEditorState> {
    self
      .shown_editor_tab()
      .and_then(|tab| self.editor_states.get(tab))
  }

  pub(super) fn shown_editor(&self) -> Option<Entity<Editor>> {
    self.shown_editor_state()?.editor.clone()
  }

  pub(super) fn shown_selected_file(&self) -> Option<&Path> {
    Some(self.shown_editor_state()?.selected_file.as_path())
  }

  pub(super) fn shown_binary_preview(&self) -> Option<&BinaryPreview> {
    self.shown_editor_state()?.binary_preview.as_ref()
  }

  pub(super) fn shown_opened_snapshot(&self) -> Option<&OpenedSnapshot> {
    self.shown_editor_state()?.opened_snapshot.as_ref()
  }

  fn set_editor_tab_state(&mut self, tab: CenterTab, state: CenterEditorState) {
    self.editor_states.insert(tab, state);
  }

  fn set_editor_tab_loading(
    &mut self,
    tab: CenterTab,
    selected_file: PathBuf,
    opened_snapshot: Option<OpenedSnapshot>,
  ) {
    self.editor_tab = Some(tab.clone());
    self.set_editor_tab_state(
      tab,
      CenterEditorState {
        selected_file,
        file_modified: None,
        editor: None,
        binary_preview: None,
        opened_snapshot,
      },
    );
  }

  fn clear_editor_tab(&mut self, tab: &CenterTab) {
    self.editor_states.remove(tab);
    if self.editor_tab.as_ref() == Some(tab) {
      self.clear_editor_tab_selection();
    }
  }

  fn clear_editor_tab_selection(&mut self) {
    self.editor_tab = None;
  }

  fn warm_worktree_file_modified(&self, cx: &App) -> Option<SystemTime> {
    let repo_root = self.checkout_root(cx)?;
    let selected_file = self.warm_selected_file()?;
    worktree_file_modified(&repo_root.join(selected_file))
  }

  fn cache_warm_editor(&mut self) {
    let Some(tab) = self.editor_tab.clone() else {
      return;
    };
    if !matches!(tab.kind, CenterTabKind::File | CenterTabKind::Diff) {
      return;
    }
    let Some(state) = self.editor_tab_state().cloned() else {
      return;
    };
    self.set_editor_tab_state(tab, state);
  }

  fn restore_center_editor(
    &mut self,
    tab: &CenterTab,
    rel_path: &Path,
    reveal_doc_position: Option<(usize, usize)>,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(state) = self.editor_states.get(tab).cloned() else {
      return false;
    };
    let Some(repo_root) = self.checkout_root(cx) else {
      return false;
    };
    if state.opened_snapshot.is_none()
      && worktree_file_modified(&repo_root.join(rel_path)) != state.file_modified
    {
      self.clear_editor_tab(tab);
      return false;
    }
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    self.open_file_task = None;
    self.remember_center_tab(tab.clone());
    self.editor_tab = Some(tab.clone());
    self.set_editor_tab_state(tab.clone(), state);
    self.svg_preview.update(cx, |preview, _| preview.clear());
    self
      .dock_panel
      .read(cx)
      .changes_list()
      .update(cx, |list, cx| {
        list.select_path(Some(rel_path), cx);
      });
    let active_editor = self.warm_editor();
    if let (Some((doc_line, doc_column)), Some(editor)) =
      (reveal_doc_position, active_editor.clone())
    {
      editor.update(cx, |editor, cx| {
        editor.reveal_source_position(doc_line, doc_column, cx)
      });
    }
    self.sync_editor_unmerged_state(cx);
    self.sync_git_telemetry(cx);
    match self.warm_opened_snapshot() {
      Some(OpenedSnapshot::PullRequestRange { .. }) => {
        if let Some(editor) = active_editor {
          self.install_github_review_handlers_for_editor(&editor, cx);
        }
      }
      Some(_) => {
        if let Some(editor) = active_editor.as_ref() {
          configure_review(editor, ReviewDestination::None, cx);
        }
      }
      None if tab.kind == CenterTabKind::Diff => self.sync_agent_review_comments_to_editor(cx),
      None => {}
    }
    self.focus_editor_if_asked(intent, window, cx);
    cx.notify();
    true
  }

  /// Split needs two sides to compare: a whole-file change or a binary preview
  /// falls back to inline.
  pub(super) fn open_agent_diff_snapshot(
    &mut self,
    rel_path: PathBuf,
    old_text: Option<String>,
    new_text: String,
    reveal_line: Option<u32>,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.should_prompt_before_replacing_editor(cx) {
      self.open_unsaved_editor_dialog(
        UnsavedEditorAction::AgentDiffSnapshot {
          rel_path,
          old_text,
          new_text,
          reveal_line,
          intent,
        },
        window,
        cx,
      );
      return;
    }
    self.open_agent_diff_snapshot_without_unsaved_prompt(
      rel_path,
      old_text,
      new_text,
      reveal_line,
      intent,
      window,
      cx,
    );
  }

  fn open_agent_diff_snapshot_without_unsaved_prompt(
    &mut self,
    rel_path: PathBuf,
    old_text: Option<String>,
    new_text: String,
    reveal_line: Option<u32>,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.cache_warm_editor();
    let Some(repo_root) = self.checkout_root(cx) else {
      return;
    };
    self.show_preview = false;
    self.center = CenterView::Diff;
    self.sync_agent_chat_close_control(cx);
    let app_settings = crate::config::AppSettings::get(cx);
    self.diff_view = if app_settings.split_diff_view {
      DiffViewMode::Split
    } else {
      DiffViewMode::Inline
    };
    if self.warm_selected_file().is_none() {
      self.hide_whitespace = app_settings.hide_whitespace;
    }
    let tab = CenterTab::agent_snapshot(rel_path.clone(), old_text.clone(), new_text.clone());
    let reveal_doc_position = reveal_line.map(|line| (line.saturating_sub(1) as usize, 0));
    if self.restore_center_editor(&tab, &rel_path, reveal_doc_position, intent, window, cx) {
      return;
    }
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    let generation = self.open_file_generation;
    self.remember_center_tab(tab.clone());
    let agent_whole_file_change = old_text.is_none() || new_text.is_empty();
    let opened_snapshot = OpenedSnapshot::AgentTool {
      old_text: old_text.clone(),
      new_text: new_text.clone(),
      whole_file_change: agent_whole_file_change,
    };
    self.set_editor_tab_loading(tab.clone(), rel_path.clone(), Some(opened_snapshot.clone()));
    self.svg_preview.update(cx, |preview, _| preview.clear());
    self
      .dock_panel
      .read(cx)
      .changes_list()
      .update(cx, |list, cx| {
        list.select_path(Some(rel_path.as_path()), cx);
      });

    let file_path = repo_root.join(&rel_path);
    let diff_view = if agent_whole_file_change {
      DiffViewMode::Inline
    } else {
      self.effective_diff_view(&rel_path, cx)
    };
    let hide_whitespace = self.hide_whitespace;
    let reveal_doc_line = reveal_line.map(|line| line.saturating_sub(1) as usize);
    let load_tab = tab.clone();
    let task = cx.spawn(async move |this, cx| {
      let diff_rel_path = rel_path.clone();
      let diff_old_text = old_text.clone();
      let diff_new_text = new_text.clone();
      let diff_set = cx
        .background_spawn(async move {
          agent_snapshot_diff_set(
            diff_old_text.as_deref(),
            &diff_new_text,
            &diff_rel_path,
            hide_whitespace,
          )
        })
        .await;
      let _ = this.update(cx, move |this, cx| {
        if this.open_file_generation != generation {
          return;
        }
        if this.warm_selected_file() != Some(rel_path.as_path()) {
          return;
        }
        let editor = cx.new(|cx| Editor::new_with_paths(repo_root.clone(), file_path, cx));
        editor.update(cx, |editor, cx| {
          editor.load_readonly_snapshot(new_text, diff_set, cx);
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_whitespace, cx);
          if let Some(doc_line) = reveal_doc_line {
            editor.reveal_source_line(doc_line, cx);
          }
        });
        configure_review(&editor, ReviewDestination::None, cx);
        this.set_editor_tab_state(
          load_tab,
          CenterEditorState {
            selected_file: rel_path.clone(),
            file_modified: None,
            editor: Some(editor.clone()),
            binary_preview: None,
            opened_snapshot: Some(opened_snapshot),
          },
        );
        this.sync_git_telemetry(cx);
        if this.center == CenterView::Diff && intent.takes_focus() {
          let _ = cx.update_window(this.window_handle, |_, window, cx| {
            let focus_handle = editor.read(cx).focus_handle(cx);
            window.focus(&focus_handle, cx);
          });
        }
        cx.notify();
      });
    });
    self.open_file_task = Some(task);
    self.focus_editor_if_asked(intent, window, cx);
    cx.notify();
  }

  /// A file as it was in a commit: a read-only snapshot with its own patch.
  pub(super) fn open_commit_file(
    &mut self,
    commit_oid: String,
    rel_path: PathBuf,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.should_prompt_before_replacing_editor(cx) {
      self.open_unsaved_editor_dialog(
        UnsavedEditorAction::CommitFile {
          commit_oid,
          rel_path,
          intent,
        },
        window,
        cx,
      );
      return;
    }
    self.open_commit_file_without_unsaved_prompt(commit_oid, rel_path, intent, window, cx);
  }

  fn open_commit_file_without_unsaved_prompt(
    &mut self,
    commit_oid: String,
    rel_path: PathBuf,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.cache_warm_editor();
    let Some(repo_root) = self.checkout_root(cx) else {
      return;
    };
    self.show_preview = false;
    self.center = CenterView::Diff;
    self.sync_agent_chat_close_control(cx);
    let tab = CenterTab::commit_snapshot(rel_path.clone(), commit_oid.clone());
    if self.restore_center_editor(&tab, &rel_path, None, intent, window, cx) {
      return;
    }
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    let generation = self.open_file_generation;
    self.remember_center_tab(tab.clone());
    let opened_snapshot = OpenedSnapshot::Commit(commit_oid.clone());
    self.set_editor_tab_loading(tab.clone(), rel_path.clone(), Some(opened_snapshot.clone()));
    let hide_whitespace = self.hide_whitespace;
    let diff_view = self.effective_diff_view(&rel_path, cx);
    let load_tab = tab.clone();

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
        // A commit is history: a comment on it would have nowhere to go.
        configure_review(&editor, ReviewDestination::None, cx);
        let binary_preview =
          build_binary_preview(rel_path.as_path(), commit_file.binary_bytes.clone());
        this.set_editor_tab_state(
          load_tab,
          CenterEditorState {
            selected_file: rel_path.clone(),
            file_modified: None,
            editor: Some(editor),
            binary_preview,
            opened_snapshot: Some(opened_snapshot),
          },
        );
        this.svg_preview.update(cx, |preview, _| preview.clear());
        cx.notify();
      });
    });
    self.open_file_task = Some(task);
    self.focus_editor_if_asked(intent, window, cx);
    cx.notify();
  }

  /// A file as the pull request proposes it: the merge base against the head,
  /// read-only. Comments on it go to GitHub, never to the agent, because the
  /// agent edits the working tree and that is a different content.
  pub(super) fn open_pull_request_file(
    &mut self,
    base_oid: String,
    head_oid: String,
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.should_prompt_before_replacing_editor(cx) {
      self.open_unsaved_editor_dialog(
        UnsavedEditorAction::PullRequestFile {
          base_oid,
          head_oid,
          rel_path,
          reveal_line,
          intent,
        },
        window,
        cx,
      );
      return;
    }
    self.open_pull_request_file_without_unsaved_prompt(
      base_oid,
      head_oid,
      rel_path,
      reveal_line,
      intent,
      window,
      cx,
    );
  }

  fn open_pull_request_file_without_unsaved_prompt(
    &mut self,
    base_oid: String,
    head_oid: String,
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    intent: OpenIntent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.cache_warm_editor();
    let Some(repo_root) = self.checkout_root(cx) else {
      return;
    };
    let reveal_doc_line = reveal_line.map(|line| line.saturating_sub(1) as usize);
    self.show_preview = false;
    self.center = CenterView::Diff;
    self.sync_agent_chat_close_control(cx);
    // Another comment on the file already open: reveal, do not reload.
    if self.warm_opened_snapshot()
      == Some(&OpenedSnapshot::PullRequestRange {
        base: base_oid.clone(),
        head: head_oid.clone(),
      })
      && self.warm_selected_file() == Some(rel_path.as_path())
      && let Some(editor) = self.warm_editor()
    {
      if let Some(doc_line) = reveal_doc_line {
        editor.update(cx, |editor, cx| editor.reveal_source_line(doc_line, cx));
      }
      self.focus_editor_if_asked(intent, window, cx);
      cx.notify();
      return;
    }
    self.leave_commit_file(cx);
    let tab =
      CenterTab::pull_request_snapshot(rel_path.clone(), base_oid.clone(), head_oid.clone());
    let reveal_doc_position = reveal_line.map(|line| (line.saturating_sub(1) as usize, 0));
    if self.restore_center_editor(&tab, &rel_path, reveal_doc_position, intent, window, cx) {
      return;
    }
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    let generation = self.open_file_generation;
    self.remember_center_tab(tab.clone());
    let opened_snapshot = OpenedSnapshot::PullRequestRange {
      base: base_oid.clone(),
      head: head_oid.clone(),
    };
    self.set_editor_tab_loading(tab.clone(), rel_path.clone(), Some(opened_snapshot.clone()));
    let hide_whitespace = self.hide_whitespace;
    let diff_view = self.effective_diff_view(&rel_path, cx);
    let load_tab = tab.clone();

    let task = cx.spawn(async move |this, cx| {
      let load_repo_root = repo_root.clone();
      let load_base = base_oid.clone();
      let load_head = head_oid.clone();
      let load_rel_path = rel_path.clone();
      let range_file = cx
        .background_spawn(async move {
          git::load_range_file_diff(&load_repo_root, &load_base, &load_head, &load_rel_path)
        })
        .await;
      let _ = this.update(cx, move |this, cx| {
        if this.open_file_generation != generation {
          return;
        }
        let Ok(range_file) = range_file else {
          return;
        };

        let file_path = repo_root.join(&rel_path);
        let editor = cx.new(|cx| Editor::new_with_paths(repo_root.clone(), file_path, cx));
        let diff_set = if range_file.patch.trim().is_empty() {
          None
        } else {
          git::diff_set_from_patch(&range_file.patch).ok()
        };
        editor.update(cx, |editor, cx| {
          editor.load_readonly_snapshot(range_file.content, diff_set, cx);
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_whitespace, cx);
          if let Some(doc_line) = reveal_doc_line {
            editor.reveal_source_line(doc_line, cx);
          }
        });
        let binary_preview =
          build_binary_preview(rel_path.as_path(), range_file.binary_bytes.clone());
        this.set_editor_tab_state(
          load_tab,
          CenterEditorState {
            selected_file: rel_path.clone(),
            file_modified: None,
            editor: Some(editor.clone()),
            binary_preview,
            opened_snapshot: Some(opened_snapshot),
          },
        );
        // The comments sync through the page's editor: install after it lands,
        // or they hang on the one this replaces.
        this.install_github_review_handlers_for_editor(&editor, cx);
        this.svg_preview.update(cx, |preview, _| preview.clear());
        cx.notify();
      });
    });
    self.open_file_task = Some(task);
    self.focus_editor_if_asked(intent, window, cx);
    cx.notify();
  }

  /// Leaving a snapshot for a working-tree file. Returns whether there was one.
  pub(super) fn leave_commit_file(&mut self, cx: &mut Context<Self>) -> bool {
    let Some(snapshot) = self.warm_opened_snapshot() else {
      return false;
    };
    if matches!(snapshot, OpenedSnapshot::Commit(_)) {
      let history = self.dock_panel.read(cx).history_list.clone();
      history.update(cx, |list, cx| list.set_opened(None, cx));
    }
    true
  }

  pub(super) fn effective_diff_view(&self, path: &Path, cx: &App) -> DiffViewMode {
    // A clean file has no other side: the split preference must not follow it.
    if !self.path_has_changes(path, self.warm_opened_snapshot().is_some(), cx) {
      return DiffViewMode::Inline;
    }
    if self.path_is_conflicted(path, cx) {
      return DiffViewMode::Inline;
    }
    effective_diff_view(DiffViewInputs {
      preferred: self.diff_view,
      binary_preview: self.warm_binary_preview().is_some(),
      previewing: self.show_preview && self.previewable(),
      whole_file_change: self.whole_file_change(path, cx),
    })
  }

  /// A file opened from the Files tab with no pending change has nothing to
  /// compare: the toggle would show the same content twice.
  #[cfg(test)]
  pub(super) fn selected_file_has_changes(&self, cx: &App) -> bool {
    let Some(path) = self.warm_selected_file() else {
      return false;
    };
    self.path_has_changes(path, self.warm_opened_snapshot().is_some(), cx)
  }

  pub(super) fn shown_file_has_changes(&self, cx: &App) -> bool {
    let Some(path) = self.shown_selected_file() else {
      return false;
    };
    self.path_has_changes(path, self.shown_opened_snapshot().is_some(), cx)
  }

  fn path_has_changes(&self, path: &Path, snapshot_open: bool, cx: &App) -> bool {
    // A snapshot always carries its own patch.
    if snapshot_open {
      return true;
    }
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .any(|entry| entry.path == path)
  }

  fn path_is_conflicted(&self, path: &Path, cx: &App) -> bool {
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .any(|entry| entry.path == path && entry.status == RepoStatusKind::Conflicted)
  }

  pub(super) fn selected_file_is_markdown(&self) -> bool {
    self
      .warm_selected_file()
      .is_some_and(crate::file_preview::is_markdown_path)
  }

  pub(super) fn selected_file_is_svg(&self) -> bool {
    self
      .warm_selected_file()
      .is_some_and(crate::file_preview::is_svg_path)
  }

  pub(super) fn shown_file_is_markdown(&self) -> bool {
    self
      .shown_selected_file()
      .is_some_and(crate::file_preview::is_markdown_path)
  }

  pub(super) fn shown_file_is_svg(&self) -> bool {
    self
      .shown_selected_file()
      .is_some_and(crate::file_preview::is_svg_path)
  }

  pub(super) fn previewable(&self) -> bool {
    self.selected_file_is_markdown() || self.selected_file_is_svg()
  }

  pub(super) fn shown_previewable(&self) -> bool {
    self.shown_file_is_markdown() || self.shown_file_is_svg()
  }

  pub(super) fn toggle_preview(&mut self, cx: &mut Context<Self>) {
    if !self.shown_previewable() {
      self.show_preview = false;
      return;
    }
    self.show_preview = !self.show_preview;
    self.sync_diff_view(cx);
    self.sync_git_telemetry(cx);
    cx.notify();
  }

  pub(super) fn split_disabled(&self, cx: &App) -> bool {
    let Some(path) = self.shown_selected_file() else {
      return true;
    };
    // The preview is not a reason to refuse: asking for split closes it.
    self.shown_binary_preview().is_some()
      || self.shown_whole_file_change(path, cx)
      || self.path_is_conflicted(path, cx)
  }

  pub(super) fn whole_file_change(&self, path: &Path, cx: &App) -> bool {
    self.whole_file_change_for(path, self.warm_opened_snapshot(), cx)
  }

  fn shown_whole_file_change(&self, path: &Path, cx: &App) -> bool {
    self.whole_file_change_for(path, self.shown_opened_snapshot(), cx)
  }

  fn whole_file_change_for(
    &self,
    path: &Path,
    opened_snapshot: Option<&OpenedSnapshot>,
    cx: &App,
  ) -> bool {
    if let Some(OpenedSnapshot::AgentTool {
      whole_file_change, ..
    }) = opened_snapshot
    {
      return *whole_file_change;
    }

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
    let Some(editor) = self.diff_editor() else {
      return;
    };
    let file_status = self.shown_file_status(cx);
    editor.update(cx, |editor, cx| {
      navigate_annotation(editor, file_status, direction, cx)
    });
    cx.notify();
    cx.stop_propagation();
  }

  /// The status of the warm editor file, unless it is a snapshot: those have none.
  pub(super) fn selected_file_status(&self, cx: &App) -> Option<RepoStatusKind> {
    if self.warm_opened_snapshot().is_some() {
      return None;
    }
    self.status_for_path(self.warm_selected_file()?, cx)
  }

  /// The status of the shown editor file, unless it is a snapshot: those have none.
  pub(super) fn shown_file_status(&self, cx: &App) -> Option<RepoStatusKind> {
    if self.shown_opened_snapshot().is_some() {
      return None;
    }
    self.status_for_path(self.shown_selected_file()?, cx)
  }

  fn status_for_path(&self, path: &Path, cx: &App) -> Option<RepoStatusKind> {
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
    let Some(editor) = self.warm_editor() else {
      return;
    };
    let is_unmerged = matches!(
      self.selected_file_status(cx),
      Some(RepoStatusKind::Conflicted)
    );
    editor.update(cx, |editor, cx| editor.set_is_unmerged(is_unmerged, cx));
  }

  /// The path a renamed file came from, so the diff header can name both sides.
  pub(super) fn shown_selected_file_old_path(&self, cx: &App) -> Option<PathBuf> {
    if self.shown_opened_snapshot().is_some() {
      return None;
    }
    self.old_path_for(self.shown_selected_file()?, cx)
  }

  fn old_path_for(&self, path: &Path, cx: &App) -> Option<PathBuf> {
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .find(|entry| entry.path == path)
      .and_then(|entry| entry.old_path.clone())
  }

  pub(super) fn annotation_navigation(&self, cx: &App) -> Option<AnnotationNavigationState> {
    let editor = self.diff_editor()?;
    let file_status = self.shown_file_status(cx);
    editor.read_with(cx, |editor, cx| {
      annotation_navigation_state_for(file_status, editor, cx)
    })
  }

  /// Accepting every conflict at once needs a conflicted file still holding markers.
  pub(super) fn can_accept_all_conflicts(&self, cx: &App) -> bool {
    let file_status = self.shown_file_status(cx);
    self.diff_editor().is_some_and(|editor| {
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
    let Some(editor) = self.diff_editor() else {
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
    let file_status = self.shown_file_status(cx);
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
    let file_status = self.shown_file_status(cx);
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
    let file_status = self.shown_file_status(cx);
    resolve_active_conflict(&editor, file_status, ConflictResolution::Both, cx);
    cx.stop_propagation();
  }

  /// The editor of the open file, unless the center shows something else or a
  /// rendered file hides the diff.
  pub(super) fn diff_editor(&self) -> Option<Entity<Editor>> {
    if self.show_preview && self.shown_previewable() {
      return None;
    }
    self.shown_editor()
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
      || (self.show_preview && self.shown_previewable())
      || !self.shown_file_has_changes(cx)
    {
      return;
    }
    self.hide_whitespace = !self.hide_whitespace;
    let value = self.hide_whitespace;
    if let Some(OpenedSnapshot::AgentTool {
      old_text, new_text, ..
    }) = self.shown_opened_snapshot().cloned()
      && let (Some(rel_path), Some(editor)) = (
        self.shown_selected_file().map(Path::to_path_buf),
        self.diff_editor(),
      )
    {
      self.open_file_generation = self.open_file_generation.wrapping_add(1);
      let generation = self.open_file_generation;
      let task = cx.spawn(async move |this, cx| {
        let diff_rel_path = rel_path.clone();
        let diff_old_text = old_text.clone();
        let diff_new_text = new_text.clone();
        let diff_set = cx
          .background_spawn(async move {
            agent_snapshot_diff_set(
              diff_old_text.as_deref(),
              &diff_new_text,
              &diff_rel_path,
              value,
            )
          })
          .await;
        let _ = this.update(cx, move |this, cx| {
          if this.open_file_generation != generation
            || this.shown_selected_file() != Some(rel_path.as_path())
          {
            return;
          }
          editor.update(cx, |editor, cx| {
            editor.set_diffs(diff_set, cx);
            editor.set_ignore_whitespace(value, cx);
          });
        });
      });
      self.open_file_task = Some(task);
    } else if let Some(editor) = self.diff_editor() {
      editor.update(cx, |editor, cx| editor.set_ignore_whitespace(value, cx));
    }
    cx.notify();
  }

  pub(super) fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
    // While the rendered file holds the pane there is no diff to switch, and a
    // clean file must not flip the shared preference from a dead toggle.
    if self.center != CenterView::Diff
      || (self.show_preview && self.shown_previewable())
      || self.split_disabled(cx)
      || !self.shown_file_has_changes(cx)
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
    let Some(editor) = self.warm_editor() else {
      return;
    };
    let Some(path) = self.warm_selected_file().map(Path::to_path_buf) else {
      return;
    };
    let diff_view = self.effective_diff_view(&path, cx);
    editor.update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
  }

  pub(super) fn close_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.center != CenterView::Diff {
      return;
    }
    if self.editor_is_dirty(cx) {
      self.open_unsaved_editor_dialog(UnsavedEditorAction::CloseDiff, window, cx);
      return;
    }
    self.close_diff_without_unsaved_prompt(window, cx);
  }

  fn close_diff_without_unsaved_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.center != CenterView::Diff {
      return;
    }
    self.center = CenterView::Conversation;
    self.remember_active_chat_tab(cx);
    self.diff_chat_open = true;
    self.sync_agent_chat_close_control(cx);
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  pub(super) fn close_active_center_tab_action(
    &mut self,
    _: &crate::CloseCenterTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let tab = match self.center {
      CenterView::Conversation => self.active_chat_tab(cx),
      CenterView::Diff => self
        .shown_editor_tab()
        .cloned()
        .unwrap_or_else(CenterTab::chat),
      CenterView::InteractiveRebase => CenterTab::interactive_rebase(),
    };
    if !tab.is_closeable() {
      cx.propagate();
      return;
    }
    self.close_center_tab(tab, window, cx);
    cx.stop_propagation();
  }

  pub(super) fn close_center_tab(
    &mut self,
    tab: CenterTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match tab.kind {
      CenterTabKind::Chat => {
        self.close_center_chat_tab(tab, window, cx);
        return;
      }
      CenterTabKind::InteractiveRebase => {
        self.close_interactive_rebase_todo(window, cx);
        return;
      }
      CenterTabKind::File | CenterTabKind::Diff => {}
    }
    if self.editor_is_dirty(cx) && self.editor_tab.as_ref() == Some(&tab) {
      self.open_unsaved_editor_dialog(UnsavedEditorAction::CloseCenterTab { tab }, window, cx);
      return;
    }
    self.close_center_tab_without_unsaved_prompt(tab, window, cx);
  }

  fn close_center_chat_tab(&mut self, tab: CenterTab, window: &mut Window, cx: &mut Context<Self>) {
    let Some(conversation_id) = tab.conversation_id().map(ToOwned::to_owned) else {
      self.activate_conversation_tab(window, cx);
      return;
    };
    let selected_closed = self.active_center_tab.as_ref() == Some(&tab);
    let next_tab = selected_closed
      .then(|| self.next_center_tab_after_closing(&tab))
      .flatten();
    self.center_tabs.retain(|candidate| candidate != &tab);
    self
      .center_tab_history
      .retain(|candidate| candidate != &tab);
    if !selected_closed {
      cx.notify();
      return;
    }

    if self
      .agent_chat_view
      .as_ref()
      .is_some_and(|panel| panel.read(cx).current_conversation().id == conversation_id)
    {
      self.park_active_chat_panel(cx);
    }
    if let Some(next_tab) = next_tab {
      self.activate_center_tab(next_tab, OpenIntent::Open, window, cx);
      return;
    }
    self.new_session(window, cx);
  }

  fn next_center_tab_after_closing(&self, closing_tab: &CenterTab) -> Option<CenterTab> {
    let tabs = self.center_tabs_for_navigation();
    if let Some(tab) = self
      .center_tab_history
      .iter()
      .rev()
      .find(|candidate| *candidate != closing_tab && tabs.iter().any(|tab| tab == *candidate))
    {
      return Some(tab.clone());
    }

    let closing_index = tabs.iter().position(|tab| tab == closing_tab)?;
    let remaining_tabs = tabs
      .into_iter()
      .filter(|tab| tab != closing_tab)
      .collect::<Vec<_>>();
    remaining_tabs
      .get(closing_index)
      .or_else(|| remaining_tabs.last())
      .cloned()
  }

  fn close_center_tab_without_unsaved_prompt(
    &mut self,
    tab: CenterTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let selected_closed = self.active_center_tab.as_ref() == Some(&tab);
    let next_tab = selected_closed
      .then(|| self.next_center_tab_after_closing(&tab))
      .flatten();
    self.center_tabs.retain(|candidate| candidate != &tab);
    self
      .center_tab_history
      .retain(|candidate| candidate != &tab);
    let editor_closed = self.editor_tab.as_ref() == Some(&tab);
    self.clear_editor_tab(&tab);
    if editor_closed {
      self.open_file_task = None;
      self.open_file_generation = self.open_file_generation.wrapping_add(1);
      self.svg_preview.update(cx, |preview, _| preview.clear());
    }
    if !selected_closed {
      cx.notify();
      return;
    }

    self.active_center_tab = None;

    if let Some(next_tab) = next_tab {
      self.activate_center_tab(next_tab, OpenIntent::Open, window, cx);
      return;
    }

    self.center = CenterView::Conversation;
    self.diff_chat_open = true;
    self.sync_agent_chat_close_control(cx);
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  pub(super) fn editor_is_dirty(&self, cx: &App) -> bool {
    self
      .warm_editor()
      .as_ref()
      .is_some_and(|editor| editor.read(cx).is_dirty)
  }

  fn should_prompt_before_opening(&self, tab: &CenterTab, cx: &App) -> bool {
    self.editor_is_dirty(cx) && self.editor_tab.as_ref() != Some(tab)
  }

  fn should_prompt_before_replacing_editor(&self, cx: &App) -> bool {
    self.editor_is_dirty(cx)
  }

  fn discard_warm_editor(&mut self) {
    if let Some(tab) = self.editor_tab.clone() {
      self.clear_editor_tab(&tab);
    } else {
      self.clear_editor_tab_selection();
    }
    self.open_file_task = None;
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
  }

  #[cfg(test)]
  pub(super) fn discard_unsaved_editor_for_test(
    &mut self,
    action: UnsavedEditorAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    window.close_dialog(cx);
    self.discard_warm_editor();
    self.perform_unsaved_editor_action(action, window, cx);
  }

  fn perform_unsaved_editor_action(
    &mut self,
    action: UnsavedEditorAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match action {
      UnsavedEditorAction::CloseDiff => self.close_diff_without_unsaved_prompt(window, cx),
      UnsavedEditorAction::SelectSession { id } => {
        self.select_session_without_unsaved_prompt(&id, window, cx)
      }
      UnsavedEditorAction::NewSessionIn { repo_root } => {
        self.new_session_in_without_unsaved_prompt(repo_root, window, cx)
      }
      UnsavedEditorAction::NewWorktreeSessionIn { repo_root, base } => {
        self.new_worktree_session_in_without_unsaved_prompt(repo_root, base, window, cx)
      }
      UnsavedEditorAction::SetFallbackRepo { repo_root } => {
        if let Err(error) = self.set_fallback_repo_without_unsaved_prompt(repo_root, window, cx) {
          window.push_notification(Notification::warning(error), cx);
        }
      }
      UnsavedEditorAction::PinCheckout { path } => {
        self.pin_checkout_without_unsaved_prompt(path, window, cx)
      }
      UnsavedEditorAction::FollowSessionCheckout => {
        self.follow_session_checkout_without_unsaved_prompt(window, cx)
      }
      UnsavedEditorAction::ForgetRepository { repo_root } => {
        if let Err(error) = self.forget_repository_without_unsaved_prompt(repo_root, window, cx) {
          window.push_notification(Notification::warning(error), cx);
        }
      }
      UnsavedEditorAction::RunBranchCommand { command } => {
        if let Err(error) = self.run_branch_command_without_unsaved_prompt(command, window, cx) {
          window.push_notification(Notification::warning(error), cx);
        }
      }
      UnsavedEditorAction::OpenDiff {
        rel_path,
        reveal_line,
        reveal_column,
        intent,
      } => self.open_diff_without_unsaved_prompt(
        rel_path,
        reveal_line,
        reveal_column,
        intent,
        window,
        cx,
      ),
      UnsavedEditorAction::OpenFile {
        rel_path,
        reveal_line,
        reveal_column,
        intent,
      } => self.open_file_without_unsaved_prompt(
        rel_path,
        reveal_line,
        reveal_column,
        intent,
        window,
        cx,
      ),
      UnsavedEditorAction::AgentDiffSnapshot {
        rel_path,
        old_text,
        new_text,
        reveal_line,
        intent,
      } => self.open_agent_diff_snapshot_without_unsaved_prompt(
        rel_path,
        old_text,
        new_text,
        reveal_line,
        intent,
        window,
        cx,
      ),
      UnsavedEditorAction::CommitFile {
        commit_oid,
        rel_path,
        intent,
      } => self.open_commit_file_without_unsaved_prompt(commit_oid, rel_path, intent, window, cx),
      UnsavedEditorAction::PullRequestFile {
        base_oid,
        head_oid,
        rel_path,
        reveal_line,
        intent,
      } => self.open_pull_request_file_without_unsaved_prompt(
        base_oid,
        head_oid,
        rel_path,
        reveal_line,
        intent,
        window,
        cx,
      ),
      UnsavedEditorAction::CloseCenterTab { tab } => {
        self.close_center_tab_without_unsaved_prompt(tab, window, cx)
      }
    }
  }

  pub(super) fn open_unsaved_editor_dialog(
    &mut self,
    action: UnsavedEditorAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let view = cx.entity();
    let editor = self.warm_editor();
    let window_handle = window.window_handle();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let save_view = view.clone();
      let discard_view = view.clone();
      let save_editor = editor.clone();
      let save_action = action.clone();
      let discard_action = action.clone();

      alert
        .title("Save file changes?")
        .description(div().child("Save your edits before closing, or discard them permanently."))
        .close_button(true)
        .footer(
          DialogFooter::new()
            .child(
              Button::new(UNSAVED_EDITOR_CANCEL_DEBUG_SELECTOR)
                .debug_selector(|| UNSAVED_EDITOR_CANCEL_DEBUG_SELECTOR.to_string())
                .label("Cancel")
                .ghost()
                .on_click(|_, window, cx| {
                  window.close_dialog(cx);
                }),
            )
            .child(
              Button::new(UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR)
                .debug_selector(|| UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR.to_string())
                .label("Discard")
                .danger()
                .on_click(move |_, window, cx| {
                  window.close_dialog(cx);
                  let discard_action = discard_action.clone();
                  discard_view.update(cx, move |view, cx| {
                    view.discard_warm_editor();
                    view.perform_unsaved_editor_action(discard_action, window, cx);
                  });
                }),
            )
            .child(
              Button::new(UNSAVED_EDITOR_SAVE_DEBUG_SELECTOR)
                .debug_selector(|| UNSAVED_EDITOR_SAVE_DEBUG_SELECTOR.to_string())
                .label("Save")
                .primary()
                .on_click(move |_, window, cx| {
                  window.close_dialog(cx);
                  if let Some(editor) = save_editor.clone() {
                    let save_view = save_view.clone();
                    let save_action = save_action.clone();
                    editor.update(cx, |editor, cx| {
                      editor.save_with_completion(
                        cx,
                        Some(Box::new(move |cx| {
                          let save_view = save_view.clone();
                          let save_action = save_action.clone();
                          let _ = cx.update_window(window_handle, move |_, window, _cx| {
                            window.on_next_frame(move |window, cx| {
                              save_view.update(cx, move |view, cx| {
                                view.perform_unsaved_editor_action(save_action, window, cx);
                              });
                            });
                          });
                        })),
                      );
                    });
                  }
                }),
            ),
        )
    });
  }

  /// The centre follows a browse, the keyboard does not: only a file the user
  /// asked for takes the focus.
  fn focus_editor_if_asked(&self, intent: OpenIntent, window: &mut Window, cx: &mut Context<Self>) {
    if intent.takes_focus() {
      self.focus_editor_on_next_frame(window, cx);
    }
  }

  pub(super) fn focus_editor_on_next_frame(&self, window: &mut Window, cx: &mut Context<Self>) {
    let view = cx.entity().downgrade();
    window.on_next_frame(move |window, cx| {
      let _ = view.update(cx, |this, cx| {
        if let Some(editor) = this.warm_editor() {
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
      .warm_selected_file()
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
  use super::agent_snapshot_diff_set;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;

  #[test]
  fn agent_snapshot_diff_set_uses_reported_old_and_new_text() {
    let diff_set = agent_snapshot_diff_set(Some("old\n"), "new\n", Path::new("src/main.rs"), false)
      .expect("diff set");

    assert_eq!(diff_set.uncommitted.hunks.len(), 1);
    assert_eq!(diff_set.unstaged.hunks.len(), 1);
    assert!(diff_set.staged.hunks.is_empty());
    assert!(
      diff_set.unstaged.hunks[0]
        .lines
        .iter()
        .any(|line| line.kind == git::DiffLineKind::Remove && line.content.as_ref() == "old")
    );
    assert!(
      diff_set.unstaged.hunks[0]
        .lines
        .iter()
        .any(|line| line.kind == git::DiffLineKind::Add && line.content.as_ref() == "new")
    );
  }

  fn dirty_warm_editor(page: &Entity<SessionPage>, cx: &mut gpui::VisualTestContext, text: &str) {
    let editor = page
      .read_with(cx, |page, _| page.warm_editor())
      .expect("editor");
    editor.update(cx, |editor, cx| {
      editor.document.update(cx, |document, cx| {
        document.replace_all(text, cx);
      });
      editor.is_dirty = true;
    });
  }

  fn snapshot_changed_line_count(
    page: &Entity<SessionPage>,
    cx: &mut gpui::VisualTestContext,
  ) -> usize {
    page.read_with(cx, |page, cx| {
      page
        .warm_editor()
        .as_ref()
        .expect("editor")
        .read(cx)
        .projection
        .as_ref()
        .map(|projection| {
          projection
            .lines
            .iter()
            .filter(|line| {
              matches!(
                line,
                editor::DisplayLine::Modified { .. }
                  | editor::DisplayLine::Removed { .. }
                  | editor::DisplayLine::Doc {
                    change: Some(editor::ChangeKind::Added),
                    ..
                  }
              )
            })
            .count()
        })
        .unwrap_or_default()
    })
  }

  #[gpui::test]
  async fn agent_snapshot_hide_whitespace_toggle_rebuilds_the_snapshot_diff(
    cx: &mut TestAppContext,
  ) {
    let repo = TempRepo::init("session-agent-snapshot-whitespace");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);

    page.update_in(cx, |page, window, cx| {
      page.open_agent_diff_snapshot(
        PathBuf::from("main.rs"),
        Some("fn main() {\n  value();\n}\n".to_string()),
        "fn main() {\n    value();\n}\n".to_string(),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    assert!(snapshot_changed_line_count(&page, cx) > 0);

    page.update(cx, |page, cx| page.toggle_hide_whitespace(cx));
    await_open_file(&page, cx).await;

    assert_eq!(snapshot_changed_line_count(&page, cx), 0);
  }

  #[gpui::test]
  async fn agent_snapshot_for_a_new_file_stays_inline(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-agent-snapshot-new-file-inline");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |_, cx| {
      crate::config::AppSettings::update(cx, |settings| settings.split_diff_view = true);
    });

    page.update_in(cx, |page, window, cx| {
      page.open_agent_diff_snapshot(
        PathBuf::from("new.txt"),
        None,
        "created\n".to_string(),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert!(page.split_disabled(cx));
      assert_eq!(
        page
          .warm_editor()
          .as_ref()
          .expect("editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Inline
      );
    });
  }

  #[gpui::test]
  async fn opening_a_file_position_reveals_its_line_and_column(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-file-position");
    commit_text_file(
      &repo.path,
      Path::new("position.txt"),
      "first\nsecond\n",
      "initial",
    );
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);

    page.update_in(cx, |page, window, cx| {
      page.open_diff_at_position(
        PathBuf::from("position.txt"),
        Some(2),
        Some(4),
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .warm_editor()
          .as_ref()
          .expect("editor")
          .read(cx)
          .cursor_offset(),
        9
      );
    });
  }

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
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
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

    // Swapping to another file remounts the split; the conversation pane
    // must ride along.
    commit_text_file(&repo.path, Path::new("other.md"), "one\n", "second file");
    std::fs::write(repo.path.join("other.md"), "two\n").expect("update other");
    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("other.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert!(
      cx.debug_bounds("session-conversation-pane").is_some(),
      "the conversation survives a file swap"
    );
    assert!(cx.debug_bounds("session-diff-editor").is_some());

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
  async fn opened_files_are_kept_as_center_tabs(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-center-tabs");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("other.md"), "one\n", "second");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    std::fs::write(repo.path.join("other.md"), "two\n").expect("update other");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
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
      page.open_diff(
        PathBuf::from("other.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert_eq!(
        page.center_tabs,
        vec![
          CenterTab::chat(),
          CenterTab::diff(PathBuf::from("README.md")),
          CenterTab::diff(PathBuf::from("other.md"))
        ]
      );
      assert_eq!(page.warm_selected_file(), Some(Path::new("other.md")));
    });

    page.update_in(cx, |page, window, cx| {
      page.activate_center_tab(
        CenterTab::diff(PathBuf::from("README.md")),
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert_eq!(page.warm_selected_file(), Some(Path::new("README.md")));
      assert_eq!(
        page.center_tabs,
        vec![
          CenterTab::chat(),
          CenterTab::diff(PathBuf::from("README.md")),
          CenterTab::diff(PathBuf::from("other.md"))
        ]
      );
    });
  }

  #[gpui::test]
  async fn center_tab_shortcuts_walk_the_open_tab_order(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-center-tab-navigation");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("other.md"), "one\n", "second");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    std::fs::write(repo.path.join("other.md"), "two\n").expect("update other");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
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
      page.open_diff(
        PathBuf::from("other.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page.activate_next_center_tab_action(&crate::NextCenterTab, window, cx);
    });
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert_eq!(page.active_center_tab, Some(CenterTab::chat()));
    });

    page.update_in(cx, |page, window, cx| {
      page.activate_next_center_tab_action(&crate::NextCenterTab, window, cx);
    });
    await_open_file(&page, cx).await;
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert_eq!(page.warm_selected_file(), Some(Path::new("README.md")));
      assert_eq!(
        page.center_tabs,
        vec![
          CenterTab::chat(),
          CenterTab::diff(PathBuf::from("README.md")),
          CenterTab::diff(PathBuf::from("other.md"))
        ]
      );
    });

    page.update_in(cx, |page, window, cx| {
      page.activate_previous_center_tab_action(&crate::PreviousCenterTab, window, cx);
    });
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert_eq!(page.active_center_tab, Some(CenterTab::chat()));
    });
  }

  #[gpui::test]
  async fn snapshot_and_worktree_diff_tabs_for_same_path_stay_separate(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-snapshot-tab-identity");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
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
    let worktree_editor = page.read_with(cx, |page, _| {
      page
        .warm_editor()
        .as_ref()
        .expect("worktree editor")
        .entity_id()
    });

    let snapshot_tab = CenterTab::agent_snapshot(
      PathBuf::from("README.md"),
      Some("agent before\n".to_string()),
      "agent after\n".to_string(),
    );
    page.update_in(cx, |page, window, cx| {
      page.open_agent_diff_snapshot(
        PathBuf::from("README.md"),
        Some("agent before\n".to_string()),
        "agent after\n".to_string(),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    let snapshot_editor = page.read_with(cx, |page, _| {
      page
        .warm_editor()
        .as_ref()
        .expect("snapshot editor")
        .entity_id()
    });

    page.read_with(cx, |page, _| {
      assert_eq!(
        page.center_tabs,
        vec![
          CenterTab::chat(),
          CenterTab::diff(PathBuf::from("README.md")),
          snapshot_tab.clone()
        ]
      );
      assert_eq!(page.active_center_tab, Some(snapshot_tab.clone()));
      assert_eq!(page.warm_selected_file(), Some(Path::new("README.md")));
      assert!(matches!(
        page.warm_opened_snapshot(),
        Some(OpenedSnapshot::AgentTool { .. })
      ));
    });

    page.update_in(cx, |page, window, cx| {
      page.activate_center_tab(
        CenterTab::diff(PathBuf::from("README.md")),
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert!(page.warm_opened_snapshot().is_none());
      assert_eq!(page.warm_selected_file(), Some(Path::new("README.md")));
      assert_eq!(
        page.warm_editor().expect("worktree editor").entity_id(),
        worktree_editor
      );
      assert!(
        page
          .warm_editor()
          .as_ref()
          .is_some_and(|editor| editor.read(cx).projection().is_some()),
        "the worktree diff tab keeps the live diff"
      );
    });

    page.update_in(cx, |page, window, cx| {
      page.activate_center_tab(snapshot_tab.clone(), OpenIntent::Open, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert_eq!(page.active_center_tab, Some(snapshot_tab.clone()));
      assert_eq!(
        page.warm_editor().expect("snapshot editor").entity_id(),
        snapshot_editor
      );
      assert!(matches!(
        page.warm_opened_snapshot(),
        Some(OpenedSnapshot::AgentTool { .. })
      ));
    });

    page.update_in(cx, |page, window, cx| {
      page.close_active_center_tab_action(&crate::CloseCenterTab, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert_eq!(
        page.active_center_tab,
        Some(CenterTab::diff(PathBuf::from("README.md")))
      );
      assert!(!page.center_tabs.contains(&snapshot_tab));
      assert!(!page.editor_states.contains_key(&snapshot_tab));
      assert!(page.warm_opened_snapshot().is_none());
      assert_eq!(
        page.warm_editor().expect("worktree editor").entity_id(),
        worktree_editor
      );
    });
  }

  #[gpui::test]
  async fn opening_a_clean_file_after_a_snapshot_uses_the_file_tab_state(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-snapshot-to-clean-file");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("clean.txt"), "clean\n", "clean");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |_, cx| {
      crate::config::AppSettings::update(cx, |settings| settings.split_diff_view = true);
    });

    page.update_in(cx, |page, window, cx| {
      page.open_agent_diff_snapshot(
        PathBuf::from("README.md"),
        Some("agent before\n".to_string()),
        "agent after\n".to_string(),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    page.read_with(cx, |page, _| {
      assert_eq!(page.warm_selected_file(), Some(Path::new("README.md")));
      assert!(page.warm_opened_snapshot().is_some());
    });
    await_open_file(&page, cx).await;
    page.read_with(cx, |page, cx| {
      assert!(page.warm_opened_snapshot().is_some());
      assert!(page.selected_file_has_changes(cx));
    });

    page.update_in(cx, |page, window, cx| {
      page.open_file(
        PathBuf::from("clean.txt"),
        None,
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    page.read_with(cx, |page, _| {
      assert_eq!(page.warm_selected_file(), Some(Path::new("clean.txt")));
      assert!(page.warm_opened_snapshot().is_none());
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert_eq!(page.warm_selected_file(), Some(Path::new("clean.txt")));
      assert!(page.warm_opened_snapshot().is_none());
      assert!(page.selected_file_status(cx).is_none());
      assert!(!page.selected_file_has_changes(cx));
      assert_eq!(
        page
          .warm_editor()
          .expect("clean editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Inline
      );
    });
  }

  #[gpui::test]
  async fn file_and_diff_tabs_keep_their_editor_entities(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-center-tab-editor-cache");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("other.md"), "one\n", "second");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    std::fs::write(repo.path.join("other.md"), "two\n").expect("update other");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
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
    let first_editor = page.read_with(cx, |page, _| {
      page
        .warm_editor()
        .as_ref()
        .expect("first editor")
        .entity_id()
    });

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("other.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    let second_editor = page.read_with(cx, |page, _| {
      page
        .warm_editor()
        .as_ref()
        .expect("second editor")
        .entity_id()
    });
    assert_ne!(first_editor, second_editor);

    page.update_in(cx, |page, window, cx| {
      page.activate_center_tab(
        CenterTab::diff(PathBuf::from("README.md")),
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert_eq!(page.warm_selected_file(), Some(Path::new("README.md")));
      assert_eq!(
        page
          .warm_editor()
          .as_ref()
          .expect("restored editor")
          .entity_id(),
        first_editor
      );
    });
  }

  #[gpui::test]
  async fn closing_the_active_center_tab_returns_to_the_previous_tab(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-center-tab-close-previous");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("other.md"), "one\n", "second");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    std::fs::write(repo.path.join("other.md"), "two\n").expect("update other");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
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
      page.open_diff(
        PathBuf::from("other.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, window, cx| {
      page.activate_center_tab(
        CenterTab::diff(PathBuf::from("README.md")),
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page.close_active_center_tab_action(&crate::CloseCenterTab, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert_eq!(page.warm_selected_file(), Some(Path::new("other.md")));
      assert_eq!(
        page.center_tabs,
        vec![
          CenterTab::chat(),
          CenterTab::diff(PathBuf::from("other.md"))
        ]
      );
    });
  }

  #[gpui::test]
  async fn file_and_diff_tabs_for_same_path_keep_separate_diff_modes(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-file-and-diff-tabs");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update_in(cx, |page, window, cx| {
      page.open_file(
        PathBuf::from("README.md"),
        None,
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.center_tabs,
        vec![
          CenterTab::chat(),
          CenterTab::file(PathBuf::from("README.md"))
        ]
      );
      assert_eq!(
        page.active_center_tab,
        Some(CenterTab::file(PathBuf::from("README.md")))
      );
      assert!(
        page
          .warm_editor()
          .as_ref()
          .is_some_and(|editor| editor.read(cx).projection().is_none()),
        "plain file tabs do not show git diffs"
      );
    });

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

    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.center_tabs,
        vec![
          CenterTab::chat(),
          CenterTab::file(PathBuf::from("README.md")),
          CenterTab::diff(PathBuf::from("README.md"))
        ]
      );
      assert_eq!(
        page.active_center_tab,
        Some(CenterTab::diff(PathBuf::from("README.md")))
      );
      assert!(
        page
          .warm_editor()
          .as_ref()
          .is_some_and(|editor| editor.read(cx).projection().is_some()),
        "diff tabs show git diffs"
      );
    });

    page.update_in(cx, |page, window, cx| {
      page.activate_center_tab(
        CenterTab::file(PathBuf::from("README.md")),
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.active_center_tab,
        Some(CenterTab::file(PathBuf::from("README.md")))
      );
      assert!(
        page
          .warm_editor()
          .as_ref()
          .is_some_and(|editor| editor.read(cx).projection().is_none()),
        "returning to the file tab hides git diffs again"
      );
    });
  }

  #[gpui::test]
  async fn closing_a_center_file_tab_activates_the_previous_file(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-center-tab-close");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("other.md"), "one\n", "second");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    std::fs::write(repo.path.join("other.md"), "two\n").expect("update other");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
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
      page.open_diff(
        PathBuf::from("other.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page.close_center_tab(CenterTab::diff(PathBuf::from("other.md")), window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert_eq!(page.warm_selected_file(), Some(Path::new("README.md")));
      assert_eq!(
        page.center_tabs,
        vec![
          CenterTab::chat(),
          CenterTab::diff(PathBuf::from("README.md"))
        ]
      );
    });

    page.update_in(cx, |page, window, cx| {
      page.close_center_tab(CenterTab::diff(PathBuf::from("README.md")), window, cx);
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert_eq!(page.center_tabs, vec![CenterTab::chat()]);
      assert!(page.warm_selected_file().is_none());
    });
  }

  #[gpui::test]
  async fn opening_a_file_highlights_it_in_the_changes_list(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-open-selects-row");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("other.md"), "v1\n", "second file");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    std::fs::write(repo.path.join("other.md"), "v2\n").expect("update other");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    // An open that does NOT come from the Changes list (chat recap, palette):
    // the list must highlight the file it now shows.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("other.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let selected = page
        .selected_status_entry(cx)
        .expect("the Changes list selected a row");
      assert_eq!(selected.path, PathBuf::from("other.md"));
    });
  }

  #[gpui::test]
  async fn open_diff_switches_center_and_escape_returns(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-open-diff");
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
      assert_eq!(page.center, CenterView::Diff);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert!(page.warm_editor().is_some());
      assert_eq!(page.warm_selected_file(), Some(Path::new("README.md")));
    });

    page.update_in(cx, |page, window, cx| {
      page.close_workspace_page_action(&CloseWorkspacePage, window, cx);
    });
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      // Editor kept for instant reopen of the same file.
      assert!(page.warm_editor().is_some());
    });
  }

  #[gpui::test]
  async fn closing_a_dirty_file_asks_before_discarding(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-dirty-close");
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
    dirty_warm_editor(&page, cx, "unsaved\n");

    page.update_in(cx, |page, window, cx| {
      page.close_workspace_page_action(&CloseWorkspacePage, window, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    assert!(
      cx.debug_bounds(UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR)
        .is_some()
    );
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert!(page.warm_editor().is_some());
    });

    page.update_in(cx, |page, window, cx| {
      page.discard_unsaved_editor_for_test(UnsavedEditorAction::CloseDiff, window, cx);
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.warm_editor().is_none());
    });
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read file"),
      "v2\n"
    );
  }

  #[gpui::test]
  async fn opening_another_file_asks_before_discarding_dirty_edits(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-dirty-open-other");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("other.md"), "a\n", "other");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    std::fs::write(repo.path.join("other.md"), "b\n").expect("update other");

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
    dirty_warm_editor(&page, cx, "unsaved\n");

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("other.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    assert!(
      cx.debug_bounds(UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR)
        .is_some()
    );
    page.read_with(cx, |page, _| {
      assert_eq!(page.warm_selected_file(), Some(Path::new("README.md")));
    });

    page.update_in(cx, |page, window, cx| {
      page.discard_unsaved_editor_for_test(
        UnsavedEditorAction::OpenDiff {
          rel_path: PathBuf::from("other.md"),
          reveal_line: None,
          reveal_column: None,
          intent: OpenIntent::Open,
        },
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert_eq!(page.warm_selected_file(), Some(Path::new("other.md")));
      assert!(page.warm_editor().is_some());
    });
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read first file"),
      "v2\n"
    );
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
      list.open_commit_file(first.clone(), PathBuf::from("a.txt"), OpenIntent::Open, cx)
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert_eq!(page.center, CenterView::Diff);
      assert_eq!(
        page.warm_opened_snapshot(),
        Some(&OpenedSnapshot::Commit(first.clone()))
      );
      let editor = page.warm_editor().as_ref().expect("editor").read(cx);
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
      page.open_diff(PathBuf::from("a.txt"), None, OpenIntent::Open, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert!(page.warm_opened_snapshot().is_none());
      let editor = page.warm_editor().as_ref().expect("editor").read(cx);
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
      crate::config::AppSettings::update(cx, |settings| settings.split_diff_view = true);
      page.open_diff(PathBuf::from("a.txt"), None, OpenIntent::Open, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    // Whole file: every document line is visible, nothing is folded away.
    let visible_and_total = |page: &SessionPage, cx: &App| {
      let editor = page.warm_editor().as_ref().expect("editor").read(cx);
      let projection = editor.projection().expect("projection");
      (
        projection.visible_doc_lines.len(),
        projection.doc_to_display.len(),
      )
    };

    page.read_with(cx, |page, cx| {
      let editor = page.warm_editor().as_ref().expect("editor").read(cx);
      assert_eq!(editor.diff_view_mode(), DiffViewMode::Inline);
      assert!(page.split_disabled(cx));
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
      let editor = page.warm_editor().as_ref().expect("editor").read(cx);
      assert_eq!(editor.diff_view_mode(), DiffViewMode::Split);
      assert!(!page.split_disabled(cx));
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
      page.open_diff(PathBuf::from("a.txt"), None, OpenIntent::Open, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    let visible_and_total = |page: &SessionPage, cx: &App| {
      let editor = page.warm_editor().as_ref().expect("editor").read(cx);
      let projection = editor.projection().expect("projection");
      (
        projection.visible_doc_lines.len(),
        projection.doc_to_display.len(),
      )
    };
    page.read_with(cx, |page, cx| {
      let editor = page.warm_editor().as_ref().expect("editor").read(cx);
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
      let editor = page.warm_editor().as_ref().expect("editor").read(cx);
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
      page.open_diff(
        PathBuf::from("dirty.txt"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    page.update(cx, |page, cx| page.toggle_diff_view(cx));
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .warm_editor()
          .as_ref()
          .expect("editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Split
      );
    });

    // A clean file from the Files tab must land inline anyway.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("clean.txt"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .warm_editor()
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
          .warm_editor()
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
      page.open_diff(PathBuf::from("a.txt"), None, OpenIntent::Open, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |page, cx| page.toggle_diff_view(cx));
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .warm_editor()
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
          .warm_editor()
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
          .warm_editor()
          .as_ref()
          .expect("editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Split
      );
    });
  }
}
