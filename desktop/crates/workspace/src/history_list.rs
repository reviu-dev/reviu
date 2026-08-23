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
  tree::{TreeEvent, TreeItem, TreeState, tree},
};
use ui::{
  FILE_ICON_SIZE_PX, SelectableRowStyle, file_icon_path_for_path_with_theme, selectable_list_item,
};

use crate::open_intent::OpenIntent;

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
  OpenCommitFile {
    commit_oid: String,
    path: PathBuf,
    intent: OpenIntent,
  },
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
  pub(crate) _poll_task: Option<Task<()>>,
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
    let tree = cx.new(|cx| TreeState::new(cx));
    cx.subscribe(&tree, |this, _tree, event: &TreeEvent, cx| {
      this.on_tree_event(event, cx)
    })
    .detach();

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
      tree,
      tree_nodes: HashMap::new(),
      focus_handle: cx.focus_handle().tab_stop(true),
      _history_task: None,
      _poll_task: None,
      _files_task: None,
    }
  }

  /// The commit tree carries the keyboard, not this container: its own handle is
  /// the one the arrow keys are bound to.
  pub(crate) fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.commits.is_empty() {
      // The empty state mounts no tree; its handle would drop the focus.
      window.focus(&self.focus_handle, cx);
      return;
    }
    self.tree.update(cx, |tree, cx| tree.focus(window, cx));
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
      let (history, revision) = cx
        .background_spawn(async move {
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

  /// Reads the cheap revision marker first: a poll that finds the same commits
  /// must not rebuild the list under the user's cursor.
  pub(crate) fn refresh_if_repository_moved(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.repo_root.clone() else {
      return;
    };

    let task = cx.spawn(async move |this, cx| {
      let requested = repo_root.clone();
      let polled = cx
        .background_spawn(async move { current_history_revision(&repo_root).ok() })
        .await;
      let _ = this.update(cx, |this, cx| {
        if this.repo_root.as_ref() != Some(&requested) {
          return;
        }
        if should_refresh_history_for_poll(
          true,
          this.commits.is_empty(),
          this.revision.as_ref(),
          polled.as_ref(),
        ) {
          this.refresh(cx);
        }
      });
    });
    self._poll_task = Some(task);
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
      let files = cx
        .background_spawn(
          async move { list_commit_changed_files(&load_repo_root, &load_commit_oid) },
        )
        .await;
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
    intent: OpenIntent,
    cx: &mut Context<Self>,
  ) {
    self.opened = Some((commit_oid.clone(), path.clone()));
    cx.emit(HistoryListEvent::OpenCommitFile {
      commit_oid,
      path,
      intent,
    });
    cx.notify();
  }

  /// The tree says where the user is and what they chose; the rows say nothing,
  /// so a click and a keystroke arrive here by the same road.
  fn on_tree_event(&mut self, event: &TreeEvent, cx: &mut Context<Self>) {
    let (id, intent) = match event {
      TreeEvent::Selected(id) => (id.clone(), OpenIntent::Browse),
      TreeEvent::Confirmed(id) => (id.clone(), OpenIntent::Open),
      TreeEvent::Expanded(_) | TreeEvent::Collapsed(_) => return,
    };
    match self.tree_nodes.get(id.as_ref()) {
      Some(HistoryTreeNode::File { commit_oid, file }) => {
        let (commit_oid, path) = (commit_oid.clone(), file.path.clone());
        self.open_commit_file(commit_oid, path, intent, cx);
      }
      // Loading more commits is a choice, not something to trip over in passing.
      Some(HistoryTreeNode::LoadHint { oid }) if intent.takes_focus() => {
        let oid = oid.clone();
        self.load_commit_files(oid, cx);
      }
      _ => {}
    }
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

#[cfg(test)]
pub(crate) mod test_support {
  use super::*;

  pub(crate) fn make_commit(oid: &str, parents: &[&str]) -> HistoryCommitNode {
    HistoryCommitNode {
      oid: oid.to_string(),
      short_oid: oid.chars().take(7).collect(),
      summary: format!("commit-{oid}"),
      author: "author".to_string(),
      parent_oids: parents.iter().map(|parent| parent.to_string()).collect(),
      refs: Vec::new(),
    }
  }

  pub(crate) fn make_history_file(path: &str, kind: CommitFileChangeKind) -> HistoryCommitFileRow {
    HistoryCommitFileRow::from_commit_file(CommitChangedFile {
      path: PathBuf::from(path),
      old_path: None,
      kind,
    })
  }

  pub(crate) fn make_history_revision(tag: &str) -> HistoryRevision {
    HistoryRevision {
      head_oid: Some(format!("head-{tag}")),
      head_label: Some(format!("HEAD -> {tag}")),
      refs: vec![format!("{tag}@oid-{tag}")],
    }
  }
}

#[cfg(test)]
mod tests {
  use super::test_support::*;
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;

  fn add_history_list_window(
    repo_root: Option<PathBuf>,
    cx: &mut TestAppContext,
  ) -> (Entity<HistoryList>, &mut gpui::VisualTestContext) {
    cx.update(|cx| gpui_component::init(cx));
    let mut mounted = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let list = cx.new(HistoryList::new);
      mounted = Some(list.clone());
      gpui_component::Root::new(list, window, cx)
    });
    let list = mounted.expect("history list");
    if let Some(repo_root) = repo_root {
      list.update(cx, |list, cx| list.set_repo_root(Some(repo_root), cx));
    }
    (list, cx)
  }

  #[gpui::test]
  async fn focusing_the_list_hands_the_keyboard_to_the_commit_tree(cx: &mut TestAppContext) {
    let repo = TempRepo::init("history-list-keyboard");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    let (list, cx) = add_history_list_window(Some(repo.path.clone()), cx);
    await_history(&list, cx).await;

    list.update_in(cx, |list, window, cx| list.focus(window, cx));
    cx.run_until_parked();

    let before = list.read_with(cx, |list, cx| list.tree.read(cx).selected_index());
    cx.simulate_keystrokes("down");
    let after = list.read_with(cx, |list, cx| list.tree.read(cx).selected_index());
    assert_ne!(
      before, after,
      "the arrow keys belong to the tree, not to the container around it"
    );
  }

  #[gpui::test]
  async fn walking_a_commit_shows_its_files_and_enter_opens_them(cx: &mut TestAppContext) {
    let repo = TempRepo::init("history-list-open-intent");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");

    let (list, cx) = add_history_list_window(Some(repo.path.clone()), cx);
    await_history(&list, cx).await;

    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(&list, move |_list, event: &HistoryListEvent, _cx| {
        let HistoryListEvent::OpenCommitFile { path, intent, .. } = event;
        seen.borrow_mut().push((path.clone(), *intent));
      })
      .detach();
    });

    list.update_in(cx, |list, window, cx| list.focus(window, cx));
    cx.run_until_parked();

    // Onto the commit and open it. The lazy load of its files is driven by the
    // render, which a test window does not run, so it is asked for here.
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    let oid = list.read_with(cx, |list, _| list.rows[0].commit.oid.clone());
    list.update(cx, |list, cx| list.load_commit_files(oid, cx));
    let files = list.update(cx, |list, _| list._files_task.take());
    if let Some(files) = files {
      files.await;
    }
    cx.run_until_parked();

    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(
      opened.borrow().as_slice(),
      &[(PathBuf::from("a.txt"), OpenIntent::Browse)],
      "walking onto a file of the commit shows it"
    );

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
      opened.borrow().last(),
      Some(&(PathBuf::from("a.txt"), OpenIntent::Open)),
      "Enter opens it, which it never did before"
    );
  }

  async fn await_history(list: &Entity<HistoryList>, cx: &mut gpui::VisualTestContext) {
    loop {
      let (history, files) = list.update(cx, |list, _| {
        (list._history_task.take(), list._files_task.take())
      });
      let mut had_task = false;
      if let Some(task) = history {
        had_task = true;
        task.await;
      }
      if let Some(task) = files {
        had_task = true;
        task.await;
      }
      cx.run_until_parked();
      if !had_task {
        return;
      }
    }
  }

  #[gpui::test]
  async fn refreshing_lists_the_commits_newest_first(cx: &mut TestAppContext) {
    let repo = TempRepo::init("history-list-refresh");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("a.txt"), "v2\n", "second");

    let (list, cx) = add_history_list_window(Some(repo.path.clone()), cx);
    await_history(&list, cx).await;

    list.read_with(cx, |list, _| {
      assert_eq!(list.commits.len(), 2);
      assert_eq!(list.commits[0].summary, "second");
      assert!(!list.is_empty());
      assert!(list.revision.is_some());
    });
  }

  #[gpui::test]
  async fn a_poll_reloads_the_list_only_when_the_repository_moved(cx: &mut TestAppContext) {
    let repo = TempRepo::init("history-list-poll");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");

    let (list, cx) = add_history_list_window(Some(repo.path.clone()), cx);
    await_history(&list, cx).await;

    // Nothing moved: the list must not be rebuilt under the cursor.
    let poll = list.update(cx, |list, cx| {
      list.refresh_if_repository_moved(cx);
      list._poll_task.take().expect("poll task")
    });
    poll.await;
    cx.run_until_parked();
    list.read_with(cx, |list, _| {
      assert!(
        list._history_task.is_none(),
        "an unchanged repository costs no reload"
      );
      assert_eq!(list.commits.len(), 1);
    });

    // A commit made outside Reviu moves the revision, so the poll reloads.
    commit_text_file(&repo.path, Path::new("a.txt"), "v2\n", "second");
    let poll = list.update(cx, |list, cx| {
      list.refresh_if_repository_moved(cx);
      list._poll_task.take().expect("poll task")
    });
    poll.await;
    await_history(&list, cx).await;

    list.read_with(cx, |list, _| {
      assert_eq!(list.commits.len(), 2);
      assert_eq!(list.commits[0].summary, "second");
    });
  }

  #[gpui::test]
  async fn a_commit_loads_its_files_once(cx: &mut TestAppContext) {
    let repo = TempRepo::init("history-list-files");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    let (list, cx) = add_history_list_window(Some(repo.path.clone()), cx);
    await_history(&list, cx).await;

    let head = list.read_with(cx, |list, _| list.commits[0].oid.clone());
    list.update(cx, |list, cx| list.load_commit_files(head.clone(), cx));
    await_history(&list, cx).await;

    list.read_with(cx, |list, _| {
      let files = list
        .files_by_commit
        .get(&head)
        .expect("files of the commit");
      assert_eq!(files.len(), 1);
      assert_eq!(files[0].path, PathBuf::from("b.txt"));
      assert!(list.loading_commits.is_empty());
    });

    // Already loaded: asking again starts nothing.
    list.update(cx, |list, cx| {
      list.load_commit_files(head.clone(), cx);
      assert!(list._files_task.is_some());
    });
    await_history(&list, cx).await;
    list.read_with(cx, |list, _| {
      assert_eq!(
        list.files_by_commit.get(&head).expect("files").len(),
        1,
        "reloading a commit does not duplicate its files"
      );
    });
  }

  #[gpui::test]
  async fn opening_a_file_reports_it_and_marks_the_row(cx: &mut TestAppContext) {
    let repo = TempRepo::init("history-list-open");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");

    let (list, cx) = add_history_list_window(Some(repo.path.clone()), cx);
    await_history(&list, cx).await;
    let head = list.read_with(cx, |list, _| list.commits[0].oid.clone());

    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(&list, move |_list, event: &HistoryListEvent, _cx| {
        let HistoryListEvent::OpenCommitFile {
          commit_oid, path, ..
        } = event;
        seen.borrow_mut().push((commit_oid.clone(), path.clone()));
      })
      .detach();
    });

    list.update(cx, |list, cx| {
      list.open_commit_file(head.clone(), PathBuf::from("a.txt"), OpenIntent::Open, cx)
    });
    cx.run_until_parked();

    assert_eq!(
      opened.borrow().as_slice(),
      &[(head.clone(), PathBuf::from("a.txt"))]
    );
    list.read_with(cx, |list, _| {
      assert_eq!(list.opened, Some((head, PathBuf::from("a.txt"))));
    });

    // The consumer moved on: the row is no longer the open one.
    list.update(cx, |list, cx| list.set_opened(None, cx));
    list.read_with(cx, |list, _| assert!(list.opened.is_none()));
  }

  #[gpui::test]
  async fn switching_repository_drops_the_previous_history(cx: &mut TestAppContext) {
    let repo = TempRepo::init("history-list-switch-from");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "from");
    let other = TempRepo::init("history-list-switch-to");
    commit_text_file(&other.path, Path::new("b.txt"), "v1\n", "to");

    let (list, cx) = add_history_list_window(Some(repo.path.clone()), cx);
    await_history(&list, cx).await;
    let head = list.read_with(cx, |list, _| list.commits[0].oid.clone());
    list.update(cx, |list, cx| list.load_commit_files(head.clone(), cx));
    await_history(&list, cx).await;
    list.update(cx, |list, cx| {
      list.set_opened(Some((head.clone(), PathBuf::from("a.txt"))), cx)
    });

    list.update(cx, |list, cx| {
      list.set_repo_root(Some(other.path.clone()), cx)
    });
    await_history(&list, cx).await;

    list.read_with(cx, |list, _| {
      assert_eq!(list.commits.len(), 1);
      assert_eq!(list.commits[0].summary, "to");
      assert!(list.files_by_commit.is_empty());
      assert!(list.opened.is_none());
    });
  }

  #[gpui::test]
  async fn a_commit_that_is_gone_takes_its_cache_with_it(cx: &mut TestAppContext) {
    let repo = TempRepo::init("history-list-amend");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    let (list, cx) = add_history_list_window(Some(repo.path.clone()), cx);
    await_history(&list, cx).await;
    let head = list.read_with(cx, |list, _| list.commits[0].oid.clone());
    list.update(cx, |list, cx| list.load_commit_files(head.clone(), cx));
    await_history(&list, cx).await;
    list.update(cx, |list, cx| {
      list.set_opened(Some((head.clone(), PathBuf::from("b.txt"))), cx)
    });

    // The commit the cache points at no longer exists after an undo.
    git::undo_last_commit(&repo.path).expect("undo the last commit");
    list.update(cx, |list, cx| list.refresh(cx));
    await_history(&list, cx).await;

    list.read_with(cx, |list, _| {
      assert_eq!(list.commits.len(), 1);
      assert!(!list.files_by_commit.contains_key(&head));
      assert!(list.opened.is_none(), "the open file left with its commit");
    });
  }

  #[test]
  fn build_history_tree_items_marks_selected_commit_expanded() {
    let commits = vec![
      make_commit("c3", &["c2"]),
      make_commit("c2", &["c1"]),
      make_commit("c1", &[]),
    ];
    let rows = commits
      .iter()
      .cloned()
      .map(HistoryRenderRow::from_commit)
      .collect::<Vec<_>>();

    let mut files_by_commit = HashMap::new();
    files_by_commit.insert(
      "c2".to_string(),
      vec![make_history_file(
        "src/c2.rs",
        CommitFileChangeKind::Modified,
      )],
    );
    files_by_commit.insert(
      "c1".to_string(),
      vec![make_history_file("src/c1.rs", CommitFileChangeKind::Added)],
    );

    let loading = HashSet::new();
    let expanded = HashSet::from(["c2".to_string()]);
    let (items, _) = build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);

    assert!(!items[0].is_expanded());
    assert!(items[1].is_expanded());
    assert!(!items[2].is_expanded());
  }

  #[test]
  fn build_history_tree_items_supports_multiple_expanded_commits() {
    let commits = vec![
      make_commit("c3", &["c2"]),
      make_commit("c2", &["c1"]),
      make_commit("c1", &[]),
    ];
    let rows = commits
      .iter()
      .cloned()
      .map(HistoryRenderRow::from_commit)
      .collect::<Vec<_>>();
    let files_by_commit = HashMap::new();
    let loading = HashSet::new();
    let expanded = HashSet::from(["c3".to_string(), "c1".to_string()]);

    let (items, _) = build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);

    assert!(items[0].is_expanded());
    assert!(!items[1].is_expanded());
    assert!(items[2].is_expanded());
  }

  #[test]
  fn build_history_tree_items_includes_commit_and_file_nodes() {
    let commits = vec![make_commit("c2", &["c1"]), make_commit("c1", &[])];
    let rows = commits
      .iter()
      .cloned()
      .map(HistoryRenderRow::from_commit)
      .collect::<Vec<_>>();
    let mut files_by_commit = HashMap::new();
    files_by_commit.insert(
      "c2".to_string(),
      vec![make_history_file(
        "src/main.rs",
        CommitFileChangeKind::Modified,
      )],
    );
    let loading = HashSet::new();
    let expanded = HashSet::from(["c2".to_string()]);

    let (items, nodes) = build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].children.len(), 1);
    assert_eq!(items[0].children[0].label.as_ref(), "src/main.rs");

    let commit_id = format!("history-commit:{}", rows[0].commit.oid);
    assert!(matches!(
      nodes.get(&commit_id),
      Some(HistoryTreeNode::Commit { oid }) if oid == "c2"
    ));

    let file_id = format!("history-file:{}:{}", rows[0].commit.oid, 0);
    assert!(matches!(
      nodes.get(&file_id),
      Some(HistoryTreeNode::File { commit_oid, .. }) if commit_oid == "c2"
    ));
  }

  #[test]
  fn build_history_tree_items_uses_loading_placeholder() {
    let commits = vec![make_commit("c1", &[])];
    let rows = commits
      .iter()
      .cloned()
      .map(HistoryRenderRow::from_commit)
      .collect::<Vec<_>>();
    let files_by_commit = HashMap::new();
    let loading = HashSet::from(["c1".to_string()]);
    let expanded = HashSet::from(["c1".to_string()]);

    let (items, nodes) = build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);
    assert_eq!(items[0].children.len(), 1);
    assert_eq!(items[0].children[0].label.as_ref(), "Loading files...");
    assert!(matches!(
      nodes.get("history-loading:c1"),
      Some(HistoryTreeNode::Placeholder)
    ));
  }

  #[test]
  fn should_refresh_history_for_poll_when_history_empty() {
    assert!(should_refresh_history_for_poll(
      true,
      true,
      Some(&make_history_revision("a")),
      Some(&make_history_revision("a"))
    ));
  }

  #[test]
  fn should_not_refresh_history_for_poll_when_revision_unchanged() {
    let revision = make_history_revision("a");
    assert!(!should_refresh_history_for_poll(
      true,
      false,
      Some(&revision),
      Some(&revision)
    ));
  }

  #[test]
  fn should_refresh_history_for_poll_when_revision_changed() {
    let cached = make_history_revision("a");
    let current = make_history_revision("b");
    assert!(should_refresh_history_for_poll(
      true,
      false,
      Some(&cached),
      Some(&current)
    ));
  }

  #[test]
  fn should_not_refresh_history_for_poll_when_history_not_included() {
    assert!(!should_refresh_history_for_poll(
      false,
      true,
      Some(&make_history_revision("a")),
      Some(&make_history_revision("b"))
    ));
  }

  #[test]
  fn should_not_refresh_history_for_poll_when_revision_unavailable() {
    assert!(!should_refresh_history_for_poll(
      true,
      false,
      Some(&make_history_revision("a")),
      None
    ));
  }

  #[test]
  fn history_change_kind_mapping_covers_all_variants() {
    assert_eq!(
      history_change_kind_to_repo_status(CommitFileChangeKind::Added),
      RepoStatusKind::Added
    );
    assert_eq!(
      history_change_kind_to_repo_status(CommitFileChangeKind::Deleted),
      RepoStatusKind::Deleted
    );
    assert_eq!(
      history_change_kind_to_repo_status(CommitFileChangeKind::Modified),
      RepoStatusKind::Modified
    );
    assert_eq!(
      history_change_kind_to_repo_status(CommitFileChangeKind::Renamed),
      RepoStatusKind::Renamed
    );
    assert_eq!(
      history_change_kind_to_repo_status(CommitFileChangeKind::Copied),
      RepoStatusKind::Renamed
    );
    assert_eq!(
      history_change_kind_to_repo_status(CommitFileChangeKind::Typechange),
      RepoStatusKind::TypeChange
    );
    assert_eq!(
      history_change_kind_to_repo_status(CommitFileChangeKind::Conflicted),
      RepoStatusKind::Conflicted
    );
  }
}
