//! Commit history: tree rows, loading and the history sidebar.

use super::*;

impl GitPage {
  pub(super) fn history_file_status_kind(
    &self,
    commit_oid: &str,
    rel_path: &Path,
  ) -> Option<RepoStatusKind> {
    self
      .history_commit_files
      .get(commit_oid)
      .and_then(|files| files.iter().find(|file| file.path == rel_path))
      .map(|file| history_change_kind_to_repo_status(file.kind))
  }

  pub(super) fn refresh_history_list(&mut self, cx: &mut Context<Self>) {
    self.history_rows_cache = Self::build_history_rows(&self.history_commits);
    self.sync_history_tree_state(cx);
  }

  pub(super) fn sync_history_tree_state(&mut self, cx: &mut Context<Self>) {
    let selected_id = self
      .history_tree
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string());
    let (items, nodes) = build_history_tree_items(
      &self.history_rows_cache,
      &self.history_commit_files,
      &self.history_commit_files_loading,
      &self.history_expanded_commit_oids,
    );
    self.history_tree_nodes = nodes;
    self.history_tree.update(cx, |state, cx| {
      state.set_items(items, cx);
      if let Some(selected_id) = selected_id.as_ref() {
        let selected_item = TreeItem::new(selected_id.clone(), selected_id.clone());
        state.set_selected_item(Some(&selected_item), cx);
      }
    });
    cx.notify();
  }

  pub(super) fn sync_history_cache_with_commits(&mut self) {
    let known_oids = self
      .history_commits
      .iter()
      .map(|commit| commit.oid.clone())
      .collect::<HashSet<_>>();
    self
      .history_commit_files
      .retain(|oid, _| known_oids.contains(oid));
    self
      .history_commit_files_loading
      .retain(|oid| known_oids.contains(oid));
    self
      .pending_history_file_loads
      .retain(|oid| known_oids.contains(oid));
    self
      .history_expanded_commit_oids
      .retain(|oid| known_oids.contains(oid));
    if let Some((commit_oid, _)) = self.history_opened_commit_file.as_ref()
      && !known_oids.contains(commit_oid)
    {
      self.history_opened_commit_file = None;
    }
  }

  pub(super) fn refresh_history(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.history_commits.clear();
      self.history_revision = None;
      self.history_loading = false;
      self.history_expanded_commit_oids.clear();
      self.history_commit_files.clear();
      self.history_commit_files_loading.clear();
      self.pending_history_file_loads.clear();
      self.history_opened_commit_file = None;
      self.interactive_rebase_todo_view = None;
      self.refresh_history_list(cx);
      cx.notify();
      return;
    };

    if self.history_commits.is_empty() {
      self.history_loading = true;
      cx.notify();
    }

    let task = cx.spawn(async move |this, cx| {
      let requested_repo = repo_root.clone();
      let (history, revision) = cx
        .background_spawn(async move {
          (
            list_commit_history(&repo_root, HISTORY_MAX_COMMITS),
            current_history_revision(&repo_root).ok(),
          )
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        if this.selected_repo.as_ref() != Some(&requested_repo) {
          return;
        }
        if let Ok(history) = history {
          this.history_commits = history;
          this.sync_history_cache_with_commits();
          if let Some(revision) = revision {
            this.history_revision = Some(revision);
          }
          this.refresh_history_list(cx);
        }
        this.history_loading = false;
        cx.notify();
      });
    });

    self.history_task = Some(task);
  }

  pub(super) fn queue_history_commit_files_load(
    &mut self,
    commit_oid: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.history_commit_files.contains_key(commit_oid.as_str())
      || self
        .history_commit_files_loading
        .contains(commit_oid.as_str())
      || self
        .pending_history_file_loads
        .contains(commit_oid.as_str())
    {
      return;
    }

    self.pending_history_file_loads.insert(commit_oid.clone());
    cx.on_next_frame(window, move |this, _, cx| {
      this.pending_history_file_loads.remove(commit_oid.as_str());
      this.load_history_commit_files(commit_oid.clone(), cx);
    });
  }

  pub(super) fn load_history_commit_files(&mut self, commit_oid: String, cx: &mut Context<Self>) {
    self.pending_history_file_loads.remove(commit_oid.as_str());
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.history_commit_files_loading.insert(commit_oid.clone());
    self.refresh_history_list(cx);
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let load_repo_root = repo_root.clone();
      let load_commit_oid = commit_oid.clone();
      let files = cx
        .background_spawn(
          async move { list_commit_changed_files(&load_repo_root, &load_commit_oid) },
        )
        .await;
      let _ = this.update(cx, |this, cx| {
        if this.selected_repo.as_ref() != Some(&repo_root) {
          return;
        }
        this
          .history_commit_files_loading
          .remove(commit_oid.as_str());
        if let Ok(files) = files {
          let rows = files
            .into_iter()
            .map(HistoryCommitFileRow::from_commit_file)
            .collect::<Vec<_>>();
          this.history_commit_files.insert(commit_oid.clone(), rows);
        } else {
          this.history_commit_files.remove(commit_oid.as_str());
        }
        this.refresh_history_list(cx);
        cx.notify();
      });
    });

    self.history_files_task = Some(task);
  }

  pub(super) fn open_history_commit_file(
    &mut self,
    commit_oid: String,
    rel_path: PathBuf,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.invalidate_open_file_task();
    self.history_opened_commit_file = Some((commit_oid.clone(), rel_path.clone()));
    self.selected_file = Some(rel_path.clone());
    self.selected_file_source = None;
    self.sync_sentry_git_context();
    let mut data = Map::new();
    data.insert(
      "file".into(),
      rel_path.to_string_lossy().replace(['\n', '\r'], "").into(),
    );
    data.insert("history_commit".into(), commit_oid.clone().into());
    self.add_git_breadcrumb("Opened history file in git page", data);
    self.refresh_history_list(cx);
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let load_repo_root = repo_root.clone();
      let load_commit_oid = commit_oid.clone();
      let load_rel_path = rel_path.clone();
      let commit_file = cx
        .background_spawn(async move {
          load_commit_file_diff(&load_repo_root, &load_commit_oid, &load_rel_path)
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        if this.selected_repo.as_ref() != Some(&repo_root) {
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
          diff_set_from_patch(&commit_file.patch).ok()
        };
        let diff_view = this.effective_diff_view_for_path(&rel_path);

        let hide_ws = this.hide_whitespace;
        editor.update(cx, |editor, cx| {
          editor.load_readonly_snapshot(commit_file.content, diff_set, cx);
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_ws, cx);
        });

        this.clear_markdown_preview_if_not_previewable(&rel_path);
        this.binary_preview =
          build_binary_preview(rel_path.as_path(), commit_file.binary_bytes.clone());
        this.editor = Some(editor);
        this.selected_file = Some(rel_path.clone());
        this.selected_file_source = None;
        this.history_opened_commit_file = Some((commit_oid.clone(), rel_path.clone()));
        this.sync_sentry_git_context();
        this.svg_preview.update(cx, |preview, _| preview.clear());
        this.refresh_history_list(cx);
        cx.notify();
      });
    });

    self.history_open_file_task = Some(task);
  }

  pub(super) fn render_history_sidebar_content(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    if self.history_loading {
      return div()
        .id("git-history-loading-container")
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .child(
          div()
            .id("git-history-loading-content")
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(Spinner::new().small())
            .child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Loading history..."),
            ),
        )
        .into_any_element();
    }

    if self.history_commits.is_empty() {
      return div()
        .id("git-history-empty-container")
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("No commits to display"),
        )
        .into_any_element();
    }

    if let Some(selected_id) = self
      .history_tree
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string())
      && let Some(HistoryTreeNode::File { commit_oid, file }) =
        self.history_tree_nodes.get(selected_id.as_str()).cloned()
    {
      let already_opened = self
        .history_opened_commit_file
        .as_ref()
        .map(|(opened_oid, opened_path)| opened_oid == &commit_oid && opened_path == &file.path)
        .unwrap_or(false);
      if !already_opened {
        let open_commit_oid = commit_oid.clone();
        let open_path = file.path.clone();
        cx.on_next_frame(window, move |this, _, cx| {
          this.open_history_commit_file(open_commit_oid.clone(), open_path.clone(), cx);
        });
      }
    }

    let view = cx.entity();
    let tree_view = tree(
      &self.history_tree,
      move |ix, entry, selected, window, cx| {
        view.update(cx, |this, cx| {
          let theme = cx.theme().clone();
          let item = entry.item();
          let indent = px(12.) + px(16.) * entry.depth();
          let node = this.history_tree_nodes.get(item.id.as_ref()).cloned();

          match node {
            Some(HistoryTreeNode::Commit { oid }) => {
              let row = this
                .history_rows_cache
                .iter()
                .find(|row| row.commit.oid == oid)
                .cloned();

              let Some(row) = row else {
                return selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
                  .w_full()
                  .px_2()
                  .pl(indent)
                  .child(item.label.clone());
              };

              let summary: SharedString = if row.commit.summary.trim().is_empty() {
                "No commit message".into()
              } else {
                row.commit.summary.clone().into()
              };

              let is_expanded = entry.is_expanded();
              if selected {
                if is_expanded {
                  this
                    .history_expanded_commit_oids
                    .insert(row.commit.oid.clone());
                } else {
                  this
                    .history_expanded_commit_oids
                    .remove(row.commit.oid.as_str());
                }
              }
              if is_expanded
                && !this
                  .history_commit_files
                  .contains_key(row.commit.oid.as_str())
                && !this
                  .history_commit_files_loading
                  .contains(row.commit.oid.as_str())
              {
                this.queue_history_commit_files_load(row.commit.oid.clone(), window, cx);
              }
              let chevron = if is_expanded {
                IconName::ChevronDown
              } else {
                IconName::ChevronRight
              };

              selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
                .w_full()
                .pl_2()
                .pr_3()
                .pl(indent)
                .child(
                  h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                      h_flex()
                        .min_w_0()
                        .flex_1()
                        .items_center()
                        .gap_2()
                        .child(
                          Icon::new(chevron)
                            .size_3()
                            .text_color(theme.muted_foreground),
                        )
                        .child(
                          div()
                            .min_w_0()
                            .flex_1()
                            .overflow_hidden()
                            .text_sm()
                            .text_ellipsis()
                            .child(summary),
                        ),
                    )
                    .child(
                      div()
                        .max_w(px(HISTORY_AUTHOR_MAX_WIDTH))
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(row.commit.author.clone()),
                    ),
                )
            }
            Some(HistoryTreeNode::File { commit_oid, file }) => {
              let status_kind = history_change_kind_to_repo_status(file.kind);
              let status_color = Self::status_color(status_kind, &theme);
              let file_icon = file_icon_path_for_path_with_theme(&file.path, &theme)
                .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
                .unwrap_or_else(|| {
                  Icon::new(IconName::File)
                    .size_3()
                    .text_color(theme.sidebar_foreground)
                    .into_any_element()
                });
              let selected = this
                .history_opened_commit_file
                .as_ref()
                .map(|(selected_oid, selected_path)| {
                  selected_oid == &commit_oid && selected_path == &file.path
                })
                .unwrap_or(false);
              let path = file.path.clone();
              let open_commit_oid = commit_oid.clone();

              selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
                .w_full()
                .px_2()
                .pl(indent)
                .child(
                  h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                      div()
                        .w(px(15.))
                        .text_xs()
                        .text_color(status_color)
                        .child(status_kind.short_code()),
                    )
                    .child(file_icon)
                    .child(
                      div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .text_ellipsis_start()
                        .text_xs()
                        .child(file.label.clone()),
                    ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                  this.open_history_commit_file(open_commit_oid.clone(), path.clone(), cx);
                }))
            }
            Some(HistoryTreeNode::LoadHint { oid }) => {
              let load_oid = oid.clone();
              selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
                .w_full()
                .px_2()
                .pl(indent)
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Load files..."),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                  this.queue_history_commit_files_load(load_oid.clone(), window, cx);
                }))
            }
            _ => selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
              .w_full()
              .px_2()
              .pl(indent)
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(item.label.clone()),
              ),
          }
        })
      },
    );

    let tree_focused = self.history_tree_wrapper_focus.contains_focused(window, cx);
    div()
      .id("git-history-scroll-container")
      .track_focus(&self.history_tree_wrapper_focus)
      .relative()
      .flex_1()
      .min_h_0()
      .key_context(crate::shortcuts::GIT_HISTORY_TREE_CONTEXT)
      .child(tree_view.pb_1().flex_1().w_full())
      .when(tree_focused, |this| {
        this.child(
          div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .border_2()
            .border_color(theme.ring.alpha(0.1)),
        )
      })
      .into_any_element()
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::*;
  use git::CommitFileChangeKind;

  use gpui::TestAppContext;

  #[gpui::test]
  fn focus_history_sidebar_tree_selects_first_commit_and_takes_focus(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.history_commits = vec![make_commit("c1", &[]), make_commit("c2", &["c1"])];
      this.refresh_history_list(cx);

      let external_focus = cx.focus_handle();
      let page_focus = this.focus_handle.clone();
      window.focus(&external_focus, cx);

      this.focus_history_sidebar_tree(window, cx);

      let focused = window.focused(cx).expect("history tree should take focus");
      assert_ne!(focused, external_focus);
      assert_ne!(focused, page_focus);
      assert_eq!(
        this
          .history_tree
          .read(cx)
          .selected_entry()
          .map(|entry| entry.item().id.to_string())
          .as_deref(),
        Some("history-commit:c1")
      );
    });
  }

  #[gpui::test]
  async fn set_sidebar_mode_history_focuses_history_tree(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-history-sidebar-focus");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.history_commits = vec![make_commit("c1", &[])];
      this.refresh_history_list(cx);

      let external_focus = cx.focus_handle();
      let page_focus = this.focus_handle.clone();
      window.focus(&external_focus, cx);

      this.set_sidebar_mode(GitSidebarMode::History, window, cx);

      let focused = window.focused(cx).expect("history tree should take focus");
      assert_ne!(focused, external_focus);
      assert_ne!(focused, page_focus);
    });

    let history_task = git_page.update_in(cx, |this, _window, _cx| this.history_task.take());
    if let Some(task) = history_task {
      task.await;
    }
  }

  #[gpui::test]
  async fn load_history_commit_files_populates_rows_for_commit(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-history-load-files");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    let commit_oid = commit_text_file(&repo.path, rel_path, "v2\n", "update").to_string();

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.load_history_commit_files(commit_oid.clone(), cx);
      this
        .history_files_task
        .take()
        .expect("history files task should exist")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (rows, still_loading) = git_page.read_with(cx, |this, _| {
      (
        this.history_commit_files.get(commit_oid.as_str()).cloned(),
        this
          .history_commit_files_loading
          .contains(commit_oid.as_str()),
      )
    });
    let rows = rows.expect("loaded history rows for commit");
    assert!(!still_loading);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path, rel_path);
    assert_eq!(rows[0].kind, CommitFileChangeKind::Modified);
  }

  #[gpui::test]
  async fn open_history_commit_file_loads_readonly_snapshot_content(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-history-open-file");
    let rel_path = Path::new("README.md");
    let old_commit_oid = commit_text_file(&repo.path, rel_path, "v1\n", "initial").to_string();
    let _ = commit_text_file(&repo.path, rel_path, "v2\n", "update");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_history_commit_file(old_commit_oid.clone(), rel_path.to_path_buf(), cx);
      this
        .history_open_file_task
        .take()
        .expect("history open file task should exist")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (opened, selected, is_read_only, contents) = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("history editor should exist");
      let editor = editor.read(cx);
      let document = editor.document().read(cx);
      (
        this.history_opened_commit_file.clone(),
        this.selected_file.clone(),
        editor.is_read_only,
        document.slice_to_string(0..document.len()),
      )
    });

    assert_eq!(
      opened,
      Some((old_commit_oid.clone(), rel_path.to_path_buf()))
    );
    assert_eq!(selected, Some(rel_path.to_path_buf()));
    assert!(is_read_only);
    assert_eq!(contents, "v1\n");
  }

  #[gpui::test]
  async fn open_history_commit_file_readonly_editor_save_does_not_overwrite_worktree(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-history-readonly-save");
    let rel_path = Path::new("README.md");
    let old_commit_oid = commit_text_file(&repo.path, rel_path, "v1\n", "initial").to_string();
    let _ = commit_text_file(&repo.path, rel_path, "v2\n", "update");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let open_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_history_commit_file(old_commit_oid, rel_path.to_path_buf(), cx);
      this
        .history_open_file_task
        .take()
        .expect("history open file task should exist")
    });
    open_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let save_task = git_page.update_in(cx, |this, _window, cx| {
      let editor = this.editor.as_ref().expect("history editor").clone();
      editor.update(cx, |editor, cx| {
        assert!(editor.is_read_only, "history editor must stay readonly");
        editor.save(cx);
        editor.save_task.take()
      })
    });

    assert!(
      save_task.is_none(),
      "readonly editor should not schedule save task"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read worktree file"),
      "v2\n"
    );
  }

  #[gpui::test]
  async fn open_file_replaces_history_snapshot_when_same_path_is_selected(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-open-file-after-history");
    let rel_path = Path::new("README.md");
    let old_commit_oid = commit_text_file(&repo.path, rel_path, "v1\n", "initial").to_string();
    let _ = commit_text_file(&repo.path, rel_path, "v2\n", "update");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let history_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_history_commit_file(old_commit_oid.clone(), rel_path.to_path_buf(), cx);
      this
        .history_open_file_task
        .take()
        .expect("history open file task should exist")
    });
    history_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (before_opened, before_read_only, before_contents) = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("history editor should exist");
      let editor = editor.read(cx);
      let document = editor.document().read(cx);
      (
        this.history_opened_commit_file.clone(),
        editor.is_read_only,
        document.slice_to_string(0..document.len()),
      )
    });
    assert_eq!(
      before_opened,
      Some((old_commit_oid.clone(), rel_path.to_path_buf()))
    );
    assert!(before_read_only);
    assert_eq!(before_contents, "v1\n");

    git_page.update_in(cx, |this, _window, cx| {
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (opened, is_read_only, contents) = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("editor should exist");
      let editor = editor.read(cx);
      let document = editor.document().read(cx);
      (
        this.history_opened_commit_file.clone(),
        editor.is_read_only,
        document.slice_to_string(0..document.len()),
      )
    });

    assert_eq!(opened, None);
    assert!(!is_read_only);
    assert_eq!(contents, "v2\n");
  }

  #[gpui::test]
  async fn queue_history_commit_files_load_skips_cached_loading_and_pending_commits(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      let cached_oid = "cached-oid".to_string();
      this.history_commit_files.insert(
        cached_oid.clone(),
        vec![make_history_file(
          "README.md",
          CommitFileChangeKind::Modified,
        )],
      );
      this.queue_history_commit_files_load(cached_oid.clone(), window, cx);
      assert!(
        !this
          .pending_history_file_loads
          .contains(cached_oid.as_str())
      );

      let loading_oid = "loading-oid".to_string();
      this
        .history_commit_files_loading
        .insert(loading_oid.clone());
      this.queue_history_commit_files_load(loading_oid.clone(), window, cx);
      assert!(
        !this
          .pending_history_file_loads
          .contains(loading_oid.as_str())
      );

      let pending_oid = "pending-oid".to_string();
      this.pending_history_file_loads.insert(pending_oid.clone());
      this.queue_history_commit_files_load(pending_oid.clone(), window, cx);
      assert!(
        this
          .pending_history_file_loads
          .contains(pending_oid.as_str())
      );
      assert_eq!(this.pending_history_file_loads.len(), 1);
    });
  }

  #[gpui::test]
  async fn load_history_commit_files_with_invalid_oid_clears_loading_and_stale_rows(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-history-load-invalid");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let invalid_oid = "0123456789012345678901234567890123456789".to_string();

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.history_commit_files.insert(
        invalid_oid.clone(),
        vec![make_history_file(
          "README.md",
          CommitFileChangeKind::Modified,
        )],
      );
      this.load_history_commit_files(invalid_oid.clone(), cx);
      this
        .history_files_task
        .take()
        .expect("history files task should exist")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (rows, loading) = git_page.read_with(cx, |this, _| {
      (
        this.history_commit_files.get(invalid_oid.as_str()).cloned(),
        this
          .history_commit_files_loading
          .contains(invalid_oid.as_str()),
      )
    });
    assert!(rows.is_none());
    assert!(!loading);
  }
}
