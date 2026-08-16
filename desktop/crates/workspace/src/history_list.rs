//! Commit history as a tree: commits, the files they touched, and what to open.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use git::{
  CommitChangedFile, CommitFileChangeKind, HistoryCommitNode, HistoryRevision, RepoStatusKind,
  current_history_revision, list_commit_changed_files, list_commit_history,
};
use gpui::{
  AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
  ParentElement, Render, SharedString, Styled, Task, Window, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, Sizable as _, h_flex,
  spinner::Spinner,
  tree::{TreeItem, TreeState, tree},
};
use smol::unblock;
use ui::{
  FILE_ICON_SIZE_PX, SelectableRowStyle, file_icon_path_for_path_with_theme, selectable_list_item,
};

pub(crate) const HISTORY_MAX_COMMITS: usize = 200;
pub(crate) const HISTORY_LIST_DEBUG_SELECTOR: &str = "history-list";
const AUTHOR_MAX_WIDTH: f32 = 180.0;

#[derive(Clone, Debug)]
pub(crate) struct HistoryCommitFileRow {
  pub(crate) path: PathBuf,
  pub(crate) kind: CommitFileChangeKind,
  pub(crate) label: SharedString,
}

impl HistoryCommitFileRow {
  pub(crate) fn from_commit_file(file: CommitChangedFile) -> Self {
    let path_label = file.path.to_string_lossy().replace(['\n', '\r'], "");
    let label = file
      .old_path
      .as_ref()
      .map(|old_path| {
        let old_label = old_path.to_string_lossy().replace(['\n', '\r'], "");
        format!("{old_label} -> {path_label}")
      })
      .unwrap_or(path_label);
    Self {
      path: file.path,
      kind: file.kind,
      label: label.into(),
    }
  }
}

#[derive(Clone, Debug)]
pub(crate) struct HistoryRenderRow {
  pub(crate) commit: HistoryCommitNode,
}

impl HistoryRenderRow {
  pub(crate) fn from_commit(commit: HistoryCommitNode) -> Self {
    Self { commit }
  }
}

#[derive(Clone, Debug)]
pub(crate) enum HistoryTreeNode {
  Commit {
    oid: String,
  },
  File {
    commit_oid: String,
    file: HistoryCommitFileRow,
  },
  LoadHint {
    oid: String,
  },
  Placeholder,
}

pub(crate) enum HistoryListEvent {
  OpenCommitFile { commit_oid: String, path: PathBuf },
}

pub(crate) struct HistoryList {
  repo_root: Option<PathBuf>,
  commits: Vec<HistoryCommitNode>,
  revision: Option<HistoryRevision>,
  rows: Vec<HistoryRenderRow>,
  files_by_commit: HashMap<String, Vec<HistoryCommitFileRow>>,
  loading_commits: HashSet<String>,
  pending_loads: HashSet<String>,
  expanded_commits: HashSet<String>,
  opened: Option<(String, PathBuf)>,
  loading: bool,
  tree: Entity<TreeState>,
  tree_nodes: HashMap<String, HistoryTreeNode>,
  focus_handle: FocusHandle,
  pub(crate) _history_task: Option<Task<()>>,
  pub(crate) _files_task: Option<Task<()>>,
}

impl EventEmitter<HistoryListEvent> for HistoryList {}

impl Focusable for HistoryList {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl HistoryList {
  pub(crate) fn new(cx: &mut Context<Self>) -> Self {
    Self {
      repo_root: None,
      commits: Vec::new(),
      revision: None,
      rows: Vec::new(),
      files_by_commit: HashMap::new(),
      loading_commits: HashSet::new(),
      pending_loads: HashSet::new(),
      expanded_commits: HashSet::new(),
      opened: None,
      loading: false,
      tree: cx.new(|cx| TreeState::new(cx)),
      tree_nodes: HashMap::new(),
      focus_handle: cx.focus_handle().tab_stop(true),
      _history_task: None,
      _files_task: None,
    }
  }

  pub(crate) fn set_repo_root(&mut self, repo_root: Option<PathBuf>, cx: &mut Context<Self>) {
    if self.repo_root == repo_root {
      return;
    }
    self.repo_root = repo_root;
    self.commits.clear();
    self.revision = None;
    self.files_by_commit.clear();
    self.loading_commits.clear();
    self.pending_loads.clear();
    self.expanded_commits.clear();
    self.opened = None;
    self.refresh(cx);
  }

  pub(crate) fn is_empty(&self) -> bool {
    self.commits.is_empty()
  }

  /// The file the consumer currently shows, so the row stays highlighted.
  pub(crate) fn set_opened(&mut self, opened: Option<(String, PathBuf)>, cx: &mut Context<Self>) {
    self.opened = opened;
    cx.notify();
  }

  pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.repo_root.clone() else {
      self.commits.clear();
      self.revision = None;
      self.loading = false;
      self.sync_rows(cx);
      return;
    };

    if self.commits.is_empty() {
      self.loading = true;
      cx.notify();
    }

    let task = cx.spawn(async move |this, cx| {
      let requested = repo_root.clone();
      let (history, revision) = unblock(move || {
        (
          list_commit_history(&repo_root, HISTORY_MAX_COMMITS),
          current_history_revision(&repo_root).ok(),
        )
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        if this.repo_root.as_ref() != Some(&requested) {
          return;
        }
        if let Ok(history) = history {
          this.commits = history;
          this.drop_cache_of_gone_commits();
          if let Some(revision) = revision {
            this.revision = Some(revision);
          }
        }
        this.loading = false;
        this.sync_rows(cx);
      });
    });
    self._history_task = Some(task);
  }

  fn drop_cache_of_gone_commits(&mut self) {
    let known = self
      .commits
      .iter()
      .map(|commit| commit.oid.clone())
      .collect::<HashSet<_>>();
    self.files_by_commit.retain(|oid, _| known.contains(oid));
    self.loading_commits.retain(|oid| known.contains(oid));
    self.pending_loads.retain(|oid| known.contains(oid));
    self.expanded_commits.retain(|oid| known.contains(oid));
    if let Some((commit_oid, _)) = self.opened.as_ref()
      && !known.contains(commit_oid)
    {
      self.opened = None;
    }
  }

  fn sync_rows(&mut self, cx: &mut Context<Self>) {
    self.rows = self
      .commits
      .iter()
      .cloned()
      .map(HistoryRenderRow::from_commit)
      .collect();
    self.sync_tree_state(cx);
  }

  fn sync_tree_state(&mut self, cx: &mut Context<Self>) {
    let selected_id = self
      .tree
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string());
    let (items, nodes) = build_history_tree_items(
      &self.rows,
      &self.files_by_commit,
      &self.loading_commits,
      &self.expanded_commits,
    );
    self.tree_nodes = nodes;
    self.tree.update(cx, |state, cx| {
      state.set_items(items, cx);
      if let Some(selected_id) = selected_id.as_ref() {
        let selected_item = TreeItem::new(selected_id.clone(), selected_id.clone());
        state.set_selected_item(Some(&selected_item), cx);
      }
    });
    cx.notify();
  }

  fn queue_commit_files_load(
    &mut self,
    commit_oid: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.files_by_commit.contains_key(commit_oid.as_str())
      || self.loading_commits.contains(commit_oid.as_str())
      || self.pending_loads.contains(commit_oid.as_str())
    {
      return;
    }

    self.pending_loads.insert(commit_oid.clone());
    cx.on_next_frame(window, move |this, _, cx| {
      this.pending_loads.remove(commit_oid.as_str());
      this.load_commit_files(commit_oid.clone(), cx);
    });
  }

  pub(crate) fn load_commit_files(&mut self, commit_oid: String, cx: &mut Context<Self>) {
    self.pending_loads.remove(commit_oid.as_str());
    let Some(repo_root) = self.repo_root.clone() else {
      return;
    };
    self.loading_commits.insert(commit_oid.clone());
    self.sync_rows(cx);

    let task = cx.spawn(async move |this, cx| {
      let load_repo_root = repo_root.clone();
      let load_commit_oid = commit_oid.clone();
      let files =
        unblock(move || list_commit_changed_files(&load_repo_root, &load_commit_oid)).await;
      let _ = this.update(cx, |this, cx| {
        if this.repo_root.as_ref() != Some(&repo_root) {
          return;
        }
        this.loading_commits.remove(commit_oid.as_str());
        match files {
          Ok(files) => {
            let rows = files
              .into_iter()
              .map(HistoryCommitFileRow::from_commit_file)
              .collect::<Vec<_>>();
            this.files_by_commit.insert(commit_oid.clone(), rows);
          }
          Err(_) => {
            this.files_by_commit.remove(commit_oid.as_str());
          }
        }
        this.sync_rows(cx);
      });
    });
    self._files_task = Some(task);
  }

  pub(crate) fn open_commit_file(
    &mut self,
    commit_oid: String,
    path: PathBuf,
    cx: &mut Context<Self>,
  ) {
    self.opened = Some((commit_oid.clone(), path.clone()));
    cx.emit(HistoryListEvent::OpenCommitFile { commit_oid, path });
    cx.notify();
  }

  fn render_tree(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let view = cx.entity();
    tree(&self.tree, move |ix, entry, selected, window, cx| {
      view.update(cx, |this, cx| {
        let theme = cx.theme().clone();
        let item = entry.item();
        let indent = px(12.) + px(16.) * entry.depth();
        let node = this.tree_nodes.get(item.id.as_ref()).cloned();

        match node {
          Some(HistoryTreeNode::Commit { oid }) => {
            let Some(row) = this.rows.iter().find(|row| row.commit.oid == oid).cloned() else {
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
                this.expanded_commits.insert(row.commit.oid.clone());
              } else {
                this.expanded_commits.remove(row.commit.oid.as_str());
              }
            }
            if is_expanded
              && !this.files_by_commit.contains_key(row.commit.oid.as_str())
              && !this.loading_commits.contains(row.commit.oid.as_str())
            {
              this.queue_commit_files_load(row.commit.oid.clone(), window, cx);
            }
            let chevron = if is_expanded {
              IconName::ChevronDown
            } else {
              IconName::ChevronRight
            };

            selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
              .w_full()
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
                      .max_w(px(AUTHOR_MAX_WIDTH))
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
            let status_color = crate::changes_list::status_color(status_kind, &theme);
            let file_icon = file_icon_path_for_path_with_theme(&file.path, &theme)
              .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
              .unwrap_or_else(|| {
                Icon::new(IconName::File)
                  .size_3()
                  .text_color(theme.sidebar_foreground)
                  .into_any_element()
              });
            let is_open = this
              .opened
              .as_ref()
              .is_some_and(|(oid, opened_path)| oid == &commit_oid && opened_path == &file.path);
            let path = file.path.clone();
            let open_commit_oid = commit_oid.clone();
            let row_index = ix;

            selectable_list_item(row_index, is_open, SelectableRowStyle::Inset, &theme)
              .w_full()
              .px_2()
              .pl(indent)
              .debug_selector(move || format!("history-file-{row_index}"))
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
                this.open_commit_file(open_commit_oid.clone(), path.clone(), cx);
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
                this.queue_commit_files_load(load_oid.clone(), window, cx);
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
    })
    .pb_1()
    .flex_1()
    .w_full()
    .into_any_element()
  }
}

impl Render for HistoryList {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    if self.loading {
      return div()
        .id("history-list-loading")
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading history..."),
        )
        .into_any_element();
    }

    if self.commits.is_empty() {
      return div()
        .id("history-list-empty")
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

    div()
      .id("history-list")
      .track_focus(&self.focus_handle)
      .debug_selector(|| HISTORY_LIST_DEBUG_SELECTOR.to_string())
      .flex()
      .flex_col()
      .size_full()
      .min_h_0()
      .child(self.render_tree(cx))
      .into_any_element()
  }
}

pub(crate) fn history_change_kind_to_repo_status(kind: CommitFileChangeKind) -> RepoStatusKind {
  match kind {
    CommitFileChangeKind::Added => RepoStatusKind::Added,
    CommitFileChangeKind::Deleted => RepoStatusKind::Deleted,
    CommitFileChangeKind::Modified => RepoStatusKind::Modified,
    CommitFileChangeKind::Renamed => RepoStatusKind::Renamed,
    // RepoStatusKind does not have "Copied", closest visual semantics is renamed.
    CommitFileChangeKind::Copied => RepoStatusKind::Renamed,
    CommitFileChangeKind::Typechange => RepoStatusKind::TypeChange,
    CommitFileChangeKind::Conflicted => RepoStatusKind::Conflicted,
  }
}

/// Polling reloads the history only when the repository moved.
pub(crate) fn should_refresh_history_for_poll(
  include_history: bool,
  history_empty: bool,
  cached_revision: Option<&HistoryRevision>,
  polled_revision: Option<&HistoryRevision>,
) -> bool {
  if !include_history {
    return false;
  }
  if history_empty {
    return true;
  }
  match polled_revision {
    Some(polled_revision) => Some(polled_revision) != cached_revision,
    None => false,
  }
}

pub(crate) fn build_history_tree_items(
  rows: &[HistoryRenderRow],
  files_by_commit: &HashMap<String, Vec<HistoryCommitFileRow>>,
  loading_commits: &HashSet<String>,
  expanded_commits: &HashSet<String>,
) -> (Vec<TreeItem>, HashMap<String, HistoryTreeNode>) {
  let mut items = Vec::with_capacity(rows.len());
  let mut nodes = HashMap::new();

  for row in rows {
    let commit_id = format!("history-commit:{}", row.commit.oid);
    nodes.insert(
      commit_id.clone(),
      HistoryTreeNode::Commit {
        oid: row.commit.oid.clone(),
      },
    );
    let mut children = Vec::new();
    if loading_commits.contains(row.commit.oid.as_str()) {
      let loading_id = format!("history-loading:{}", row.commit.oid);
      nodes.insert(loading_id.clone(), HistoryTreeNode::Placeholder);
      children.push(TreeItem::new(loading_id, "Loading files..."));
    } else if let Some(files) = files_by_commit.get(row.commit.oid.as_str()) {
      if files.is_empty() {
        let empty_id = format!("history-empty:{}", row.commit.oid);
        nodes.insert(empty_id.clone(), HistoryTreeNode::Placeholder);
        children.push(TreeItem::new(empty_id, "No files changed"));
      } else {
        for (file_index, file) in files.iter().enumerate() {
          let file_id = format!("history-file:{}:{}", row.commit.oid, file_index);
          nodes.insert(
            file_id.clone(),
            HistoryTreeNode::File {
              commit_oid: row.commit.oid.clone(),
              file: file.clone(),
            },
          );
          children.push(TreeItem::new(file_id, file.label.clone()));
        }
      }
    } else {
      let hint_id = format!("history-hint:{}", row.commit.oid);
      nodes.insert(
        hint_id.clone(),
        HistoryTreeNode::LoadHint {
          oid: row.commit.oid.clone(),
        },
      );
      children.push(TreeItem::new(hint_id, "Load files..."));
    }

    let is_expanded = expanded_commits.contains(row.commit.oid.as_str());
    items.push(
      TreeItem::new(commit_id, row.commit.summary.clone())
        .children(children)
        .expanded(is_expanded),
    );
  }

  (items, nodes)
}
