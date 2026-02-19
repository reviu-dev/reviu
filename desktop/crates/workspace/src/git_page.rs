use std::{
  collections::{HashMap, HashSet},
  path::{Path, PathBuf},
  rc::Rc,
  sync::Arc,
  time::Duration,
};

use editor::{CloseFind, ConflictResolution, DiffViewMode, Editor, Find, HunkAction, HunkState};
use git::{
  BranchKind, BranchRef, BranchStatus, CommitChangedFile, CommitFileChangeKind, HeadCommitStatus,
  HistoryCommitNode, HistoryRevision, RepoStage, RepoStatusEntry, RepoStatusKind, abort_merge,
  abort_rebase, amend_commit, apply_stash, cherry_pick_commits, commit_changes, continue_rebase,
  create_branch, create_branch_from, create_stash, current_branch_status, current_history_revision,
  current_rebase_commit_message, default_stash_message, delete_untracked_file, diff_set_from_patch,
  drop_stash, fetch, head_commit_status, is_merge_in_progress, is_rebase_in_progress,
  list_branches, list_commit_changed_files, list_commit_history, list_repo_status, list_stashes,
  load_commit_file_diff, merge_branch, pop_stash, push, rebase_branch, restore_file, stage_all,
  stage_file, switch_branch, undo_last_commit, unstage_all, unstage_file,
};
use gpui::{
  AnyElement, AnyWindowHandle, App, Context, Corner, Entity, FocusHandle, Focusable, Global,
  InteractiveElement, Keystroke, ParentElement, PathPromptOptions, Render, RenderImage,
  SharedString, Styled, Task, WeakEntity, Window, actions, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, IndexPath, Selectable, Sizable,
  alert::Alert,
  button::{Button, ButtonVariant, ButtonVariants as _},
  h_flex,
  kbd::Kbd,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  menu::{DropdownMenu, PopupMenuItem},
  select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
  spinner::Spinner,
  tag::Tag,
  text::TextView,
  tooltip::Tooltip,
  tree::{TreeItem, TreeState, tree},
};
use smol::unblock;

use crate::{
  api::ApiClient,
  auth_state::{AuthState, AuthStateStore},
  config::{ConfigStore, RecentRepository},
  github_page::GithubPageHandle,
  github_pr_details_page::GithubPrDetailsPageHandle,
  workspace::{WorkspaceApi, WorkspacePage, WorkspaceRoute},
};
use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteBranch, CommandPaletteBranchKind,
  CommandPaletteCommand, CommandPaletteConfig, CommandPaletteHandler, CommandPalettePage,
  CommandPaletteRepository, CommandPaletteStash, ConfirmDialog, FILE_ICON_SIZE_PX, HEADER_HEIGHT,
  Input, InputState, SearchFileEntry, SearchFileHandler, SearchFilePalette,
  SearchFilePaletteConfig, StatusThemeExt, UiIconName, UserMenuConfig, UserMenuPage, UserMenuState,
  UserMenuUser, WindowExt, file_icon_path_for_path_with_theme, user_menu,
};

const SIDEBAR_DEFAULT_WIDTH: f32 = 400.0;
const SIDEBAR_MIN_WIDTH: f32 = 250.0;
const SIDEBAR_MAX_WIDTH: f32 = 1500.0;
const STATUS_POLL_INTERVAL_MS: u64 = 800;
const EDITOR_HEADER_HEIGHT: f32 = 40.0;
const HISTORY_MAX_COMMITS: usize = 200;
const HISTORY_AUTHOR_MAX_WIDTH: f32 = 180.0;

actions!(
  workspace,
  [
    OpenRepository,
    SaveFile,
    ShowCommandPalette,
    ShowFileSearch,
    CommitChanges
  ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitFileLabelFormat {
  FullPath,
  FileName,
}

fn format_git_file_label(path: &Path, format: GitFileLabelFormat) -> SharedString {
  match format {
    GitFileLabelFormat::FullPath => path.to_string_lossy().replace(['\n', '\r'], "").into(),
    GitFileLabelFormat::FileName => path
      .file_name()
      .and_then(|name| name.to_str())
      .unwrap_or("Untitled")
      .replace(['\n', '\r'], "")
      .into(),
  }
}

fn render_repo_status_label(
  theme: &gpui_component::Theme,
  status: Option<RepoStatusKind>,
  label: SharedString,
  old_label: Option<SharedString>,
) -> AnyElement {
  if status == Some(RepoStatusKind::Renamed)
    && let Some(old_label) = old_label
  {
    return h_flex()
      .min_w_0()
      .flex_1()
      .items_center()
      .gap_1()
      .child(
        div()
          .min_w_0()
          .overflow_hidden()
          .text_ellipsis_start()
          .text_color(theme.muted_foreground)
          .line_through()
          .child(old_label),
      )
      .child(
        Icon::new(IconName::ArrowRight)
          .size_3()
          .text_color(theme.muted_foreground),
      )
      .child(
        div()
          .min_w_0()
          .flex_1()
          .overflow_hidden()
          .text_ellipsis_start()
          .child(label),
      )
      .into_any_element();
  }

  div()
    .min_w_0()
    .flex_1()
    .overflow_hidden()
    .text_ellipsis_start()
    .when(status == Some(RepoStatusKind::Deleted), |this| {
      this.line_through()
    })
    .child(label)
    .into_any_element()
}

#[derive(Clone, Debug)]
struct GitFileRow {
  entry: RepoStatusEntry,
  label: SharedString,
  old_label: Option<SharedString>,
}

impl GitFileRow {
  fn new(entry: RepoStatusEntry) -> Self {
    let label = format_git_file_label(&entry.path, GitFileLabelFormat::FullPath);
    let old_label = entry
      .old_path
      .as_ref()
      .map(|path| format_git_file_label(path, GitFileLabelFormat::FullPath));
    Self {
      entry,
      label,
      old_label,
    }
  }
}

struct GitFileListDelegate {
  rows: Vec<Rc<GitFileRow>>,
  selected_index: Option<IndexPath>,
  opened_path: Option<PathBuf>,
}

impl GitFileListDelegate {
  fn new() -> Self {
    Self {
      rows: Vec::new(),
      selected_index: None,
      opened_path: None,
    }
  }

  fn set_rows(&mut self, entries: Vec<RepoStatusEntry>) {
    self.rows = entries
      .into_iter()
      .map(|entry| Rc::new(GitFileRow::new(entry)))
      .collect();
  }

  fn row_at(&self, ix: IndexPath) -> Option<Rc<GitFileRow>> {
    self.rows.get(ix.row).cloned()
  }

  fn set_opened_path(&mut self, path: Option<PathBuf>) {
    self.opened_path = path;
  }
}

fn file_list_base_item(ix: IndexPath, selected_index: Option<IndexPath>) -> ListItem {
  ListItem::new(ix).selected(
    selected_index
      .map(|selected| selected.eq_row(ix))
      .unwrap_or(false),
  )
}

impl ListDelegate for GitFileListDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.rows.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = cx.theme().clone();
    let mut base_item = file_list_base_item(ix, self.selected_index);
    let row = self.rows.get(ix.row)?;
    let is_opened = self
      .opened_path
      .as_ref()
      .map(|path| path == &row.entry.path)
      .unwrap_or(false);

    if is_opened {
      base_item = base_item.bg(theme.sidebar_accent.opacity(0.35));
    }

    let status_kind = row.entry.status;
    let status_letter = status_kind.short_code();
    let status_color = GitPage::status_color(status_kind, &theme);
    let status_tooltip = GitPage::status_tooltip(status_kind);
    let (stage_icon, stage_color, stage_tooltip) = GitPage::stage_style(row.entry.stage, &theme);
    let file_icon = file_icon_path_for_path_with_theme(&row.entry.path, &theme)
      .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
      .unwrap_or_else(|| {
        Icon::new(IconName::File)
          .size_3()
          .text_color(theme.sidebar_foreground)
          .into_any_element()
      });

    let stage_icon = Icon::new(stage_icon).size_3().text_color(stage_color);
    let stage_element: AnyElement = if let Some(tooltip) = stage_tooltip {
      let tooltip_id = format!("git-stage-icon-{}", ix.row);
      div()
        .id(tooltip_id)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(stage_icon)
        .into_any_element()
    } else {
      div().child(stage_icon).into_any_element()
    };

    let status_element = div()
      .id(format!("git-status-letter-{}", ix.row))
      .w(px(15.))
      .text_xs()
      .text_color(status_color)
      .tooltip(move |window, cx| Tooltip::new(status_tooltip.clone()).build(window, cx))
      .child(status_letter);

    let file_label = render_repo_status_label(
      &theme,
      Some(row.entry.status),
      row.label.clone(),
      row.old_label.clone(),
    );

    Some(
      base_item.px_2().py_1().child(
        h_flex()
          .items_center()
          .gap_2()
          .child(status_element)
          .child(stage_element)
          .child(file_icon)
          .child(file_label),
      ),
    )
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    div()
      .flex()
      .flex_col()
      .size_full()
      .items_center()
      .justify_center()
      .text_sm()
      .text_color(cx.theme().muted_foreground)
      .child("No changes")
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    self.selected_index = ix;
    cx.notify();
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitSidebarMode {
  Changes,
  History,
}

#[derive(Clone, Debug)]
struct HistoryCommitFileRow {
  path: PathBuf,
  kind: CommitFileChangeKind,
  label: SharedString,
}

impl HistoryCommitFileRow {
  fn from_commit_file(file: CommitChangedFile) -> Self {
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
struct HistoryRenderRow {
  commit: HistoryCommitNode,
}

impl HistoryRenderRow {
  fn from_commit(commit: HistoryCommitNode) -> Self {
    Self { commit }
  }
}

#[derive(Clone, Debug)]
enum HistoryTreeNode {
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

#[derive(Clone)]
struct RecentRepoItem {
  path: PathBuf,
  label: SharedString,
  is_selected: bool,
}

impl RecentRepoItem {
  fn new(repo: &RecentRepository, selected_repo: Option<&PathBuf>) -> Self {
    let label = repo.path.to_string_lossy().to_string();
    Self {
      path: repo.path.clone(),
      label: label.into(),
      is_selected: selected_repo == Some(&repo.path),
    }
  }
}

impl SelectItem for RecentRepoItem {
  type Value = PathBuf;

  fn title(&self) -> SharedString {
    self.label.clone()
  }

  fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
    let icon = Icon::new(IconName::Check)
      .size_3()
      .text_color(cx.theme().accent_foreground)
      .when(!self.is_selected, |this| this.opacity(0.0));

    h_flex()
      .items_center()
      .gap_2()
      .child(
        div()
          .flex_1()
          .overflow_hidden()
          .text_ellipsis_start()
          .child(self.label.clone()),
      )
      .child(icon)
  }

  fn value(&self) -> &Self::Value {
    &self.path
  }

  fn matches(&self, query: &str) -> bool {
    self.label.to_lowercase().contains(&query.to_lowercase())
  }
}

#[derive(Clone)]
struct BranchSelectItem {
  branch: BranchRef,
  label: SharedString,
  is_current: bool,
}

impl BranchSelectItem {
  fn new(branch: BranchRef, is_current: bool) -> Self {
    let label: SharedString = branch.name.clone().into();
    Self {
      branch,
      label,
      is_current,
    }
  }
}

impl SelectItem for BranchSelectItem {
  type Value = BranchRef;

  fn title(&self) -> SharedString {
    self.label.clone()
  }

  fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
    let icon = Icon::new(IconName::Check)
      .size_3()
      .text_color(cx.theme().accent_foreground)
      .when(!self.is_current, |this| this.opacity(0.0));

    h_flex()
      .items_center()
      .gap_2()
      .child(
        div()
          .flex_1()
          .overflow_hidden()
          .text_ellipsis()
          .child(self.label.clone()),
      )
      .child(icon)
  }

  fn value(&self) -> &Self::Value {
    &self.branch
  }

  fn matches(&self, query: &str) -> bool {
    self.label.to_lowercase().contains(&query.to_lowercase())
  }
}

#[derive(Clone, Default)]
pub struct AuthCallbackTarget {
  git_page: Option<WeakEntity<GitPage>>,
}

impl Global for AuthCallbackTarget {}

impl AuthCallbackTarget {
  pub fn register_git_page(cx: &mut Context<GitPage>) {
    cx.set_global(Self {
      git_page: Some(cx.entity().downgrade()),
    });
  }

  pub fn handle_auth_code(code: String, cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.handle_auth_code(code, cx));
  }

  pub fn start_sign_in(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.start_github_sign_in(cx));
  }

  pub fn sign_out(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.logout(cx));
  }
}

pub struct GitPage {
  focus_handle: FocusHandle,
  api: ApiClient,
  repo_select: Entity<SelectState<SearchableVec<RecentRepoItem>>>,
  branch_select: Entity<SelectState<SearchableVec<BranchSelectItem>>>,
  file_list: Entity<ListState<GitFileListDelegate>>,
  history_tree: Entity<TreeState>,
  window_handle: AnyWindowHandle,
  selected_repo: Option<PathBuf>,
  status_entries: Vec<RepoStatusEntry>,
  branch_status: Option<BranchStatus>,
  has_head_commit: bool,
  can_undo_last_commit: bool,
  can_push: bool,
  can_force_push: bool,
  force_push_after_rebase: bool,
  fetch_in_progress: bool,
  has_staged_changes: bool,
  merge_in_progress: bool,
  rebase_in_progress: bool,
  sidebar_mode: GitSidebarMode,
  history_commits: Vec<HistoryCommitNode>,
  history_revision: Option<HistoryRevision>,
  history_loading: bool,
  history_expanded_commit_oids: HashSet<String>,
  history_commit_files: HashMap<String, Vec<HistoryCommitFileRow>>,
  history_commit_files_loading: HashSet<String>,
  pending_history_file_loads: HashSet<String>,
  history_opened_commit_file: Option<(String, PathBuf)>,
  history_rows_cache: Vec<HistoryRenderRow>,
  history_tree_nodes: HashMap<String, HistoryTreeNode>,
  selected_file: Option<PathBuf>,
  select_first_file_after_restore: bool,
  force_list_selection: bool,
  editor: Option<Entity<Editor>>,
  diff_view: DiffViewMode,
  show_markdown_preview: bool,
  svg_preview: Option<Result<Arc<RenderImage>, SharedString>>,
  svg_preview_source: Option<SharedString>,
  svg_preview_task: Option<Task<()>>,
  auth_state: AuthState,
  auth_task: Option<Task<()>>,
  open_file_task: Option<Task<()>>,
  status_task: Option<Task<()>>,
  history_task: Option<Task<()>>,
  history_files_task: Option<Task<()>>,
  history_open_file_task: Option<Task<()>>,
  branch_task: Option<Task<()>>,
  open_file_generation: u64,
  branch_refresh_generation: u64,
  poll_task: Option<Task<()>>,
  commit_input: Entity<InputState>,
  operation_error: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SelectedFileUpdate {
  clear_selection: bool,
  sync_diff_view: bool,
}

impl GitPage {
  fn should_refresh_file_list(sidebar_mode: GitSidebarMode) -> bool {
    sidebar_mode == GitSidebarMode::Changes
  }

  fn should_refresh_history_for_poll(
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

  fn branch_name_changed(previous: Option<&BranchStatus>, next: Option<&BranchStatus>) -> bool {
    previous.map(|status| status.name.as_str()) != next.map(|status| status.name.as_str())
  }

  fn has_staged_changes(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| matches!(entry.stage, RepoStage::Staged | RepoStage::PartiallyStaged))
  }

  fn selected_file_update(
    selected_file: Option<&Path>,
    status_entries: &[RepoStatusEntry],
    has_history_file_selection: bool,
    sync_diff_when_selected_retained: bool,
  ) -> SelectedFileUpdate {
    if has_history_file_selection {
      return SelectedFileUpdate::default();
    }

    let Some(selected_file) = selected_file else {
      return SelectedFileUpdate::default();
    };

    let is_selected_file_present = status_entries
      .iter()
      .any(|entry| entry.path.as_path() == selected_file);
    if !is_selected_file_present {
      return SelectedFileUpdate {
        clear_selection: true,
        sync_diff_view: false,
      };
    }

    SelectedFileUpdate {
      clear_selection: false,
      sync_diff_view: sync_diff_when_selected_retained,
    }
  }

  fn selected_branch_from_status(current: Option<BranchStatus>) -> Option<BranchRef> {
    current.and_then(|status| {
      if status.name == "HEAD" {
        None
      } else {
        Some(BranchRef {
          name: status.name,
          kind: BranchKind::Local,
        })
      }
    })
  }

  fn branch_select_items(
    branches: Vec<BranchRef>,
    selected: Option<&BranchRef>,
  ) -> Vec<BranchSelectItem> {
    branches
      .into_iter()
      .map(|branch| {
        let is_current = selected == Some(&branch);
        BranchSelectItem::new(branch, is_current)
      })
      .collect()
  }

  fn should_apply_branch_refresh(
    selected_repo: Option<&Path>,
    requested_repo: &Path,
    current_generation: u64,
    refresh_generation: u64,
  ) -> bool {
    selected_repo == Some(requested_repo) && current_generation == refresh_generation
  }

  fn should_refresh_editor_for_path(selected_file: Option<&Path>, rel_path: &Path) -> bool {
    selected_file == Some(rel_path)
  }

  fn restore_uses_delete(status: RepoStatusKind) -> bool {
    status == RepoStatusKind::Untracked
  }

  fn stage_requires_confirmation(status: RepoStatusKind) -> bool {
    status == RepoStatusKind::Conflicted
  }

  fn should_confirm_stage_for_status(
    status: Option<RepoStatusKind>,
    has_unresolved_conflict_markers: bool,
  ) -> bool {
    status.is_some_and(Self::stage_requires_confirmation) && has_unresolved_conflict_markers
  }

  fn first_conflicted_path(repo_root: &Path) -> Option<PathBuf> {
    list_repo_status(repo_root)
      .ok()?
      .into_iter()
      .find(|entry| entry.status == RepoStatusKind::Conflicted)
      .map(|entry| entry.path)
  }

  fn all_entries_staged(entries: &[RepoStatusEntry]) -> bool {
    !entries.is_empty() && entries.iter().all(|entry| entry.stage == RepoStage::Staged)
  }

  #[allow(clippy::too_many_arguments)]
  fn apply_status_snapshot(
    &mut self,
    entries: Vec<RepoStatusEntry>,
    branch_status: Option<BranchStatus>,
    head_status: Option<HeadCommitStatus>,
    merge_in_progress: bool,
    rebase_in_progress: bool,
    rebase_commit_message: Option<String>,
    sync_diff_when_selected_retained: bool,
    cx: &mut Context<Self>,
  ) -> bool {
    let was_rebase_in_progress = self.rebase_in_progress;
    self.status_entries = entries;
    let branch_changed =
      Self::branch_name_changed(self.branch_status.as_ref(), branch_status.as_ref());
    self.branch_status = branch_status;
    if branch_changed {
      self.force_push_after_rebase = false;
    } else if self.force_push_after_rebase
      && self
        .branch_status
        .as_ref()
        .is_some_and(|status| status.ahead == 0)
    {
      self.force_push_after_rebase = false;
    }
    self.merge_in_progress = merge_in_progress;
    self.rebase_in_progress = rebase_in_progress;
    if !rebase_in_progress {
      self.operation_error = None;
    }
    self.sync_rebase_commit_input(
      was_rebase_in_progress,
      rebase_in_progress,
      rebase_commit_message,
      cx,
    );
    self.has_staged_changes = Self::has_staged_changes(&self.status_entries);
    let head_status = head_status.unwrap_or(HeadCommitStatus {
      has_head_commit: false,
      can_undo_last_commit: false,
    });
    self.has_head_commit = head_status.has_head_commit;
    self.can_undo_last_commit = head_status.can_undo_last_commit;
    let (can_push, can_force_push) = Self::push_flags(
      self.branch_status.as_ref(),
      self.has_head_commit,
      self.force_push_after_rebase,
    );
    self.can_push = can_push;
    self.can_force_push = can_force_push;

    let selected_file_update = Self::selected_file_update(
      self.selected_file.as_deref(),
      &self.status_entries,
      self.history_opened_commit_file.is_some(),
      sync_diff_when_selected_retained,
    );
    if selected_file_update.clear_selection {
      self.invalidate_open_file_task();
      self.selected_file = None;
      self.editor = None;
    } else if selected_file_update.sync_diff_view {
      self.sync_diff_view(cx);
    }

    if self.select_first_file_after_restore {
      self.select_first_file_after_restore = false;
      if let Some(first_path) = self.status_entries.first().map(|entry| entry.path.clone()) {
        self.open_file(first_path, cx);
      }
    }

    branch_changed
  }

  fn history_file_status_kind(&self, commit_oid: &str, rel_path: &Path) -> Option<RepoStatusKind> {
    self
      .history_commit_files
      .get(commit_oid)
      .and_then(|files| files.iter().find(|file| file.path == rel_path))
      .map(|file| Self::history_change_kind_to_repo_status(file.kind))
  }

  fn split_disabled_for_path(&self, rel_path: &Path) -> bool {
    if let Some((commit_oid, selected_path)) = self.history_opened_commit_file.as_ref()
      && selected_path == rel_path
      && let Some(status) = self.history_file_status_kind(commit_oid, rel_path)
    {
      return matches!(
        status,
        RepoStatusKind::Untracked | RepoStatusKind::Added | RepoStatusKind::Deleted
      );
    }

    self.status_entries.iter().any(|entry| {
      entry.path == rel_path
        && matches!(
          entry.status,
          RepoStatusKind::Untracked | RepoStatusKind::Added | RepoStatusKind::Deleted
        )
    })
  }

  fn is_markdown_path(path: &Path) -> bool {
    matches!(
      path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref(),
      Some("md" | "markdown" | "mdx")
    )
  }

  fn is_svg_path(path: &Path) -> bool {
    matches!(
      path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref(),
      Some("svg")
    )
  }

  fn selected_file_is_markdown(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|path| Self::is_markdown_path(path))
      .unwrap_or(false)
  }

  fn selected_file_is_svg(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|path| Self::is_svg_path(path))
      .unwrap_or(false)
  }

  fn effective_diff_view_for_path(&self, path: &Path) -> DiffViewMode {
    if self.show_markdown_preview && (Self::is_markdown_path(path) || Self::is_svg_path(path)) {
      return DiffViewMode::Inline;
    }

    if self.split_disabled_for_path(path) {
      return DiffViewMode::Inline;
    }

    self.diff_view
  }

  fn sync_diff_view(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let diff_view = if let Some(path) = self.selected_file.as_ref() {
      self.effective_diff_view_for_path(path)
    } else {
      self.diff_view
    };
    editor.update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
  }

  fn selected_file_index(&self) -> Option<IndexPath> {
    let selected = self.selected_file.as_ref()?;
    let index = self
      .status_entries
      .iter()
      .position(|entry| &entry.path == selected)?;
    Some(IndexPath::new(index))
  }

  fn set_file_list_selected_index(&self, index: Option<IndexPath>, cx: &mut Context<Self>) {
    let file_list = self.file_list.clone();
    let window_handle = self.window_handle;
    let _ = cx.update_window(window_handle, move |_, window, cx| {
      file_list.update(cx, |state, cx| {
        state.set_selected_index(index, window, cx);
      });
    });
  }

  fn refresh_file_list(&mut self, cx: &mut Context<Self>) {
    let rows = self.status_entries.clone();
    let current_index = self.file_list.read(cx).selected_index();
    let opened_path = self.selected_file.clone();
    self.file_list.update(cx, |state, cx| {
      state.delegate_mut().set_rows(rows.clone());
      state.delegate_mut().set_opened_path(opened_path);
      cx.notify();
    });

    let selected_index = if self.force_list_selection {
      self.force_list_selection = false;
      self.selected_file_index()
    } else {
      current_index
        .and_then(|ix| (ix.row < rows.len()).then_some(IndexPath::new(ix.row)))
        .or_else(|| self.selected_file_index())
    };
    self.set_file_list_selected_index(selected_index, cx);
  }

  fn refresh_history_list(&mut self, cx: &mut Context<Self>) {
    self.history_rows_cache = Self::build_history_rows(&self.history_commits);
    self.sync_history_tree_state(cx);
  }

  fn sync_history_tree_state(&mut self, cx: &mut Context<Self>) {
    let selected_id = self
      .history_tree
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string());
    let (items, nodes) = Self::build_history_tree_items(
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

  fn build_history_tree_items(
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

  fn sync_history_cache_with_commits(&mut self) {
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

  fn handle_auth_code(&mut self, code: String, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let service = self.api.keychain_service().to_string();
    let username = self.api.keychain_username().to_string();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.exchange_code_for_token(&code)).await;
      match result {
        Ok(token) => {
          let secret = token.clone().into_bytes();
          let write_task = cx.update(|cx| cx.write_credentials(&service, &username, &secret));
          let _ = write_task.await;
          let _ = this.update(cx, |this, cx| {
            this.api.set_bearer_token(token);
            this.refresh_auth_state(cx);
          });
        }
        Err(_) => {
          let _ = this.update(cx, |this, cx| {
            this.set_auth_state(AuthState::Unauthenticated, cx);
          });
        }
      }
    });

    self.auth_task = Some(task);
  }

  fn logout(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let service = self.api.keychain_service().to_string();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || api.sign_out()).await;
      let delete_task = cx.update(|cx| cx.delete_credentials(&service));
      let _ = delete_task.await;
      let _ = this.update(cx, |this, cx| {
        this.set_auth_state(AuthState::Unauthenticated, cx);
      });
    });

    self.auth_task = Some(task);
  }

  fn load_bearer_from_keychain(&mut self, cx: &mut Context<Self>) {
    let service = self.api.keychain_service().to_string();
    let task = cx.spawn(async move |this, cx| {
      let read_result = cx.update(|cx| cx.read_credentials(&service)).await;
      let _ = this.update(cx, |this, cx| {
        if let Ok(Some((_username, secret))) = read_result
          && let Ok(token) = String::from_utf8(secret)
        {
          this.api.set_bearer_token(token);
          this.refresh_auth_state(cx);
          return;
        }
        this.set_auth_state(AuthState::Unauthenticated, cx);
      });
    });

    self.auth_task = Some(task);
  }

  fn refresh_auth_state(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_me()).await;
      let _ = this.update(cx, |this, cx| {
        let state = match result {
          Ok(Some(user)) => AuthState::Authenticated(user),
          Ok(None) => AuthState::Unauthenticated,
          Err(_) => AuthState::Unauthenticated,
        };
        this.set_auth_state(state, cx);
      });
    });

    self.auth_task = Some(task);
  }

  fn start_github_sign_in(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let task = cx.spawn(async move |_, cx| {
      let result = unblock(move || api.sign_in_with_github()).await;
      if let Ok(Some(url)) = result {
        cx.update(|cx| cx.open_url(&url));
      }
    });

    self.auth_task = Some(task);
  }

  fn set_auth_state(&mut self, state: AuthState, cx: &mut Context<Self>) {
    self.auth_state = state.clone();
    AuthStateStore::set(cx, state);
    cx.refresh_windows();
    cx.notify();
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let recent = ConfigStore::load_recent_repositories();
    let selected_repo = recent.first().map(|repo| repo.path.clone());
    let items: Vec<RecentRepoItem> = recent
      .iter()
      .map(|repo| RecentRepoItem::new(repo, selected_repo.as_ref()))
      .collect();
    let repo_select =
      cx.new(|cx| SelectState::new(SearchableVec::new(items), None, window, cx).searchable(true));
    let branch_select = cx.new(|cx| {
      SelectState::new(
        SearchableVec::new(Vec::<BranchSelectItem>::new()),
        None,
        window,
        cx,
      )
      .searchable(true)
    });
    let file_list = cx.new(|cx| ListState::new(GitFileListDelegate::new(), window, cx));
    let history_tree = cx.new(|cx| TreeState::new(cx));

    if let Some(repo) = selected_repo.as_ref() {
      repo_select.update(cx, |state, cx| {
        state.set_selected_value(repo, window, cx);
      });
    }

    let commit_input = cx.new(|cx| {
      InputState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });

    let mut view = Self {
      focus_handle: cx.focus_handle(),
      api: WorkspaceApi::global(cx).api.clone(),
      repo_select,
      branch_select,
      file_list,
      history_tree,
      window_handle: window.window_handle(),
      selected_repo,
      status_entries: Vec::new(),
      branch_status: None,
      has_head_commit: false,
      can_undo_last_commit: false,
      can_push: false,
      can_force_push: false,
      force_push_after_rebase: false,
      fetch_in_progress: false,
      has_staged_changes: false,
      merge_in_progress: false,
      rebase_in_progress: false,
      sidebar_mode: GitSidebarMode::Changes,
      history_commits: Vec::new(),
      history_revision: None,
      history_loading: false,
      history_expanded_commit_oids: HashSet::new(),
      history_commit_files: HashMap::new(),
      history_commit_files_loading: HashSet::new(),
      pending_history_file_loads: HashSet::new(),
      history_opened_commit_file: None,
      history_rows_cache: Vec::new(),
      history_tree_nodes: HashMap::new(),
      selected_file: None,
      select_first_file_after_restore: false,
      force_list_selection: false,
      editor: None,
      diff_view: DiffViewMode::Inline,
      show_markdown_preview: false,
      svg_preview: None,
      svg_preview_source: None,
      svg_preview_task: None,
      auth_state: AuthState::Unknown,
      auth_task: None,
      open_file_task: None,
      status_task: None,
      history_task: None,
      history_files_task: None,
      history_open_file_task: None,
      branch_task: None,
      open_file_generation: 0,
      branch_refresh_generation: 0,
      poll_task: None,
      commit_input,
      operation_error: None,
    };

    view.subscribe_to_repo_select(cx);
    view.subscribe_to_branch_select(cx);
    view.subscribe_to_file_list(cx);
    view.reload_status(cx);
    view.refresh_branches(cx);
    view.start_polling(cx);
    view.load_bearer_from_keychain(cx);
    AuthCallbackTarget::register_git_page(cx);

    view
  }

  #[cfg(test)]
  fn new_for_test(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let repo_select = cx.new(|cx| {
      SelectState::new(
        SearchableVec::new(Vec::<RecentRepoItem>::new()),
        None,
        window,
        cx,
      )
    });
    let branch_select = cx.new(|cx| {
      SelectState::new(
        SearchableVec::new(Vec::<BranchSelectItem>::new()),
        None,
        window,
        cx,
      )
      .searchable(true)
    });
    let file_list = cx.new(|cx| ListState::new(GitFileListDelegate::new(), window, cx));
    let history_tree = cx.new(|cx| TreeState::new(cx));
    let commit_input = cx.new(|cx| {
      InputState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });

    let mut view = Self {
      focus_handle: cx.focus_handle(),
      api: ApiClient::new(),
      repo_select,
      branch_select,
      file_list,
      history_tree,
      window_handle: window.window_handle(),
      selected_repo: None,
      status_entries: Vec::new(),
      branch_status: None,
      has_head_commit: false,
      can_undo_last_commit: false,
      can_push: false,
      can_force_push: false,
      force_push_after_rebase: false,
      fetch_in_progress: false,
      has_staged_changes: false,
      merge_in_progress: false,
      rebase_in_progress: false,
      sidebar_mode: GitSidebarMode::Changes,
      history_commits: Vec::new(),
      history_revision: None,
      history_loading: false,
      history_expanded_commit_oids: HashSet::new(),
      history_commit_files: HashMap::new(),
      history_commit_files_loading: HashSet::new(),
      pending_history_file_loads: HashSet::new(),
      history_opened_commit_file: None,
      history_rows_cache: Vec::new(),
      history_tree_nodes: HashMap::new(),
      selected_file: None,
      select_first_file_after_restore: false,
      force_list_selection: false,
      editor: None,
      diff_view: DiffViewMode::Inline,
      show_markdown_preview: false,
      svg_preview: None,
      svg_preview_source: None,
      svg_preview_task: None,
      auth_state: AuthState::Unknown,
      auth_task: None,
      open_file_task: None,
      status_task: None,
      history_task: None,
      history_files_task: None,
      history_open_file_task: None,
      branch_task: None,
      open_file_generation: 0,
      branch_refresh_generation: 0,
      poll_task: None,
      commit_input,
      operation_error: None,
    };

    view.subscribe_to_repo_select(cx);
    view.subscribe_to_branch_select(cx);
    view.subscribe_to_file_list(cx);
    view
  }

  fn subscribe_to_repo_select(&mut self, cx: &mut Context<Self>) {
    cx.subscribe(
      &self.repo_select,
      move |this, _state, event: &SelectEvent<SearchableVec<RecentRepoItem>>, cx| {
        if let SelectEvent::Confirm(Some(repo)) = event {
          this.set_selected_repo(repo.clone(), cx);
        }
      },
    )
    .detach();
  }

  fn subscribe_to_branch_select(&mut self, cx: &mut Context<Self>) {
    cx.subscribe(
      &self.branch_select,
      move |this, _state, event: &SelectEvent<SearchableVec<BranchSelectItem>>, cx| {
        let SelectEvent::Confirm(Some(branch)) = event else {
          return;
        };
        let Some(repo_root) = this.selected_repo.clone() else {
          return;
        };
        let branch = branch.clone();
        let editor = this.editor.clone();

        let task = cx.spawn(async move |this, cx| {
          let result = unblock(move || switch_branch(&repo_root, &branch)).await;
          let _ = this.update(cx, |this, cx| {
            if result.is_ok() {
              this.reload_status(cx);
              this.refresh_branches(cx);
              if let Some(editor) = editor.clone() {
                editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
              }
            }
          });
        });

        this.branch_task = Some(task);
      },
    )
    .detach();
  }

  fn subscribe_to_file_list(&mut self, cx: &mut Context<Self>) {
    cx.subscribe(
      &self.file_list,
      move |this, state, event: &ListEvent, cx| match event {
        ListEvent::Select(ix) | ListEvent::Confirm(ix) => {
          let row = state.read(cx).delegate().row_at(*ix);
          if let Some(row) = row {
            this.open_file(row.entry.path.clone(), cx);
          }
        }
        ListEvent::Cancel => {}
      },
    )
    .detach();
  }

  fn invalidate_open_file_task(&mut self) {
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    self.open_file_task = None;
  }

  fn set_selected_repo(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    if self.selected_repo.as_ref() == Some(&repo_root) {
      return;
    }

    self.selected_repo = Some(repo_root.clone());
    self.invalidate_open_file_task();
    self.selected_file = None;
    self.select_first_file_after_restore = false;
    self.operation_error = None;
    self.editor = None;
    self.merge_in_progress = false;
    self.rebase_in_progress = false;
    self.force_push_after_rebase = false;
    self.history_commits.clear();
    self.history_revision = None;
    self.history_loading = self.sidebar_mode == GitSidebarMode::History;
    self.history_expanded_commit_oids.clear();
    self.history_commit_files.clear();
    self.history_commit_files_loading.clear();
    self.pending_history_file_loads.clear();
    self.history_opened_commit_file = None;
    ConfigStore::persist_recent_repository(&repo_root);

    self.reload_status(cx);
    self.refresh_branches(cx);
    self.refresh_repo_select(cx);
    self.sync_repo_select_with_path(&repo_root, cx);
    cx.notify();
  }

  fn refresh_repo_select(&mut self, cx: &mut Context<Self>) {
    let selected_repo = self.selected_repo.clone();
    let mut recent = ConfigStore::load_recent_repositories();
    if let Some(selected_repo_path) = selected_repo.as_ref()
      && !recent.iter().any(|repo| &repo.path == selected_repo_path)
    {
      recent.insert(
        0,
        RecentRepository {
          path: selected_repo_path.clone(),
        },
      );
    }

    let items: Vec<RecentRepoItem> = recent
      .iter()
      .map(|repo| RecentRepoItem::new(repo, selected_repo.as_ref()))
      .collect();
    let window_handle = self.window_handle;

    let select = cx.update_window(window_handle, |_, window, cx| {
      let select =
        cx.new(|cx| SelectState::new(SearchableVec::new(items), None, window, cx).searchable(true));
      if let Some(repo) = selected_repo.as_ref() {
        select.update(cx, |state, cx| {
          state.set_selected_value(repo, window, cx);
        });
      }
      select
    });

    if let Ok(select) = select {
      self.repo_select = select;
      self.subscribe_to_repo_select(cx);
    }
  }

  fn sync_repo_select_with_path(&self, repo_root: &Path, cx: &mut Context<Self>) {
    let repo_select = self.repo_select.clone();
    let repo_root = repo_root.to_path_buf();
    let mut recent = ConfigStore::load_recent_repositories();
    if !recent.iter().any(|repo| repo.path == repo_root) {
      recent.insert(
        0,
        RecentRepository {
          path: repo_root.clone(),
        },
      );
    }
    let items = recent
      .iter()
      .map(|repo| RecentRepoItem::new(repo, Some(&repo_root)))
      .collect::<Vec<_>>();
    let window_handle = self.window_handle;
    let _ = cx.update_window(window_handle, |_, window, cx| {
      repo_select.update(cx, |state, cx| {
        state.set_items(SearchableVec::new(items), window, cx);
        state.set_selected_value(&repo_root, window, cx);
      });
    });
  }

  fn clear_branch_select(&mut self, cx: &mut Context<Self>) {
    let branch_select = self.branch_select.clone();
    let window_handle = self.window_handle;
    let _ = cx.update_window(window_handle, |_, window, cx| {
      branch_select.update(cx, |state, cx| {
        state.set_items(
          SearchableVec::new(Vec::<BranchSelectItem>::new()),
          window,
          cx,
        );
        state.set_selected_index(None, window, cx);
      });
    });
  }

  fn refresh_branches(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.clear_branch_select(cx);
      return;
    };

    self.branch_refresh_generation = self.branch_refresh_generation.wrapping_add(1);
    let refresh_generation = self.branch_refresh_generation;
    let requested_repo = repo_root.clone();
    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        let branches = list_branches(&repo_root).ok()?;
        let current = current_branch_status(&repo_root).ok();
        Some((branches, current))
      })
      .await;
      let Some((branches, current)) = result else {
        return;
      };

      let selected = Self::selected_branch_from_status(current);
      let items = Self::branch_select_items(branches, selected.as_ref());

      let select = cx.update_window(window_handle, |_, window, cx| {
        let select = cx
          .new(|cx| SelectState::new(SearchableVec::new(items), None, window, cx).searchable(true));
        if let Some(selected) = selected.as_ref() {
          select.update(cx, |state, cx| {
            state.set_selected_value(selected, window, cx);
          });
        }
        select
      });

      let _ = this.update(cx, |this, cx| {
        if !Self::should_apply_branch_refresh(
          this.selected_repo.as_deref(),
          requested_repo.as_path(),
          this.branch_refresh_generation,
          refresh_generation,
        ) {
          return;
        }
        if let Ok(select) = select {
          this.branch_select = select;
          this.subscribe_to_branch_select(cx);
        }
      });
    });

    self.branch_task = Some(task);
  }

  fn refresh_history(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.history_commits.clear();
      self.history_revision = None;
      self.history_loading = false;
      self.history_expanded_commit_oids.clear();
      self.history_commit_files.clear();
      self.history_commit_files_loading.clear();
      self.pending_history_file_loads.clear();
      self.history_opened_commit_file = None;
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
      let (history, revision) = unblock(move || {
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

  fn reload_status(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.invalidate_open_file_task();
      self.status_entries.clear();
      self.select_first_file_after_restore = false;
      if Self::should_refresh_file_list(self.sidebar_mode) {
        self.refresh_file_list(cx);
      }
      self.branch_status = None;
      self.has_head_commit = false;
      self.can_undo_last_commit = false;
      self.can_push = false;
      self.can_force_push = false;
      self.force_push_after_rebase = false;
      self.has_staged_changes = false;
      self.merge_in_progress = false;
      self.rebase_in_progress = false;
      self.operation_error = None;
      self.history_commits.clear();
      self.history_revision = None;
      self.history_loading = false;
      self.history_expanded_commit_oids.clear();
      self.history_commit_files.clear();
      self.history_commit_files_loading.clear();
      self.pending_history_file_loads.clear();
      self.history_opened_commit_file = None;
      self.refresh_history_list(cx);
      cx.notify();
      return;
    };
    let include_history = self.sidebar_mode == GitSidebarMode::History;
    if include_history && self.history_commits.is_empty() {
      self.history_loading = true;
      cx.notify();
    }

    let task = cx.spawn(async move |this, cx| {
      let requested_repo = repo_root.clone();
      let status = unblock(move || {
        let entries = list_repo_status(&repo_root).ok()?;
        let branch = current_branch_status(&repo_root).ok();
        let head_status = head_commit_status(&repo_root).ok();
        let merge_in_progress = is_merge_in_progress(&repo_root).unwrap_or(false);
        let rebase_in_progress = is_rebase_in_progress(&repo_root).unwrap_or(false);
        let rebase_commit_message = if rebase_in_progress {
          current_rebase_commit_message(&repo_root).ok().flatten()
        } else {
          None
        };
        let history = if include_history {
          list_commit_history(&repo_root, HISTORY_MAX_COMMITS).ok()
        } else {
          None
        };
        let history_revision = if include_history {
          current_history_revision(&repo_root).ok()
        } else {
          None
        };
        Some((
          entries,
          branch,
          head_status,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          history,
          history_revision,
        ))
      })
      .await;
      let Some((
        entries,
        branch_status,
        head_status,
        merge_in_progress,
        rebase_in_progress,
        rebase_commit_message,
        history,
        history_revision,
      )) = status
      else {
        let _ = this.update(cx, |this, cx| {
          if this.selected_repo.as_ref() != Some(&requested_repo) {
            return;
          }
          this.invalidate_open_file_task();
          this.status_entries.clear();
          this.select_first_file_after_restore = false;
          this.branch_status = None;
          this.has_head_commit = false;
          this.can_undo_last_commit = false;
          this.can_push = false;
          this.can_force_push = false;
          this.force_push_after_rebase = false;
          this.has_staged_changes = false;
          this.merge_in_progress = false;
          this.rebase_in_progress = false;
          this.operation_error = None;
          this.selected_file = None;
          this.editor = None;
          this.history_opened_commit_file = None;
          this.clear_branch_select(cx);
          if include_history {
            this.history_commits.clear();
            this.history_revision = None;
            this.history_loading = false;
            this.history_expanded_commit_oids.clear();
            this.history_commit_files.clear();
            this.history_commit_files_loading.clear();
            this.pending_history_file_loads.clear();
            this.refresh_history_list(cx);
          } else if this.history_loading {
            this.history_loading = false;
          }
          if Self::should_refresh_file_list(this.sidebar_mode) {
            this.refresh_file_list(cx);
          }
          cx.notify();
        });
        return;
      };

      let _ = this.update(cx, |this, cx| {
        if this.selected_repo.as_ref() != Some(&requested_repo) {
          return;
        }
        let branch_changed = this.apply_status_snapshot(
          entries,
          branch_status,
          head_status,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          true,
          cx,
        );
        if include_history {
          if let Some(history) = history {
            this.history_commits = history;
            this.sync_history_cache_with_commits();
            if let Some(history_revision) = history_revision {
              this.history_revision = Some(history_revision);
            }
            this.refresh_history_list(cx);
          }
          this.history_loading = false;
        }
        if branch_changed {
          this.refresh_branches(cx);
        }
        if Self::should_refresh_file_list(this.sidebar_mode) {
          this.refresh_file_list(cx);
        }
        cx.notify();
      });
    });

    self.status_task = Some(task);
  }

  fn start_polling(&mut self, cx: &mut Context<Self>) {
    if self.poll_task.is_some() {
      return;
    }

    self.poll_task = Some(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(STATUS_POLL_INTERVAL_MS))
          .await;

        let poll_state = match this.update(cx, |this, _| {
          (
            this.selected_repo.clone(),
            this.sidebar_mode == GitSidebarMode::History,
            this.history_revision.clone(),
            this.history_commits.is_empty(),
          )
        }) {
          Ok(value) => value,
          Err(_) => return,
        };
        let (repo_root, include_history, cached_history_revision, history_empty) = poll_state;
        let Some(repo_root) = repo_root else {
          continue;
        };
        let requested_repo = repo_root.clone();

        let status = unblock(move || {
          let entries = list_repo_status(&repo_root).ok()?;
          let branch = current_branch_status(&repo_root).ok();
          let head_status = head_commit_status(&repo_root).ok();
          let merge_in_progress = is_merge_in_progress(&repo_root).unwrap_or(false);
          let rebase_in_progress = is_rebase_in_progress(&repo_root).unwrap_or(false);
          let rebase_commit_message = if rebase_in_progress {
            current_rebase_commit_message(&repo_root).ok().flatten()
          } else {
            None
          };
          let polled_history_revision = if include_history {
            current_history_revision(&repo_root).ok()
          } else {
            None
          };
          let should_refresh_history = Self::should_refresh_history_for_poll(
            include_history,
            history_empty,
            cached_history_revision.as_ref(),
            polled_history_revision.as_ref(),
          );
          let history = if should_refresh_history {
            list_commit_history(&repo_root, HISTORY_MAX_COMMITS).ok()
          } else {
            None
          };
          Some((
            entries,
            branch,
            head_status,
            merge_in_progress,
            rebase_in_progress,
            rebase_commit_message,
            polled_history_revision,
            should_refresh_history,
            history,
          ))
        })
        .await;
        let Some((
          entries,
          branch_status,
          head_status,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          polled_history_revision,
          should_refresh_history,
          history,
        )) = status
        else {
          continue;
        };

        let _ = this.update(cx, |this, cx| {
          if this.selected_repo.as_ref() != Some(&requested_repo) {
            return;
          }
          let branch_changed = this.apply_status_snapshot(
            entries,
            branch_status,
            head_status,
            merge_in_progress,
            rebase_in_progress,
            rebase_commit_message,
            false,
            cx,
          );
          if include_history {
            if let Some(history) = history {
              this.history_commits = history;
              this.sync_history_cache_with_commits();
              this.history_loading = false;
              if let Some(history_revision) = polled_history_revision {
                this.history_revision = Some(history_revision);
              }
              this.refresh_history_list(cx);
            } else if !should_refresh_history {
              if let Some(history_revision) = polled_history_revision {
                this.history_revision = Some(history_revision);
              }
            } else if this.history_loading {
              // Preserve last known history on transient failures.
              this.history_loading = false;
            }
          }
          if branch_changed {
            this.refresh_branches(cx);
          }
          if Self::should_refresh_file_list(this.sidebar_mode) {
            this.refresh_file_list(cx);
          }
          cx.notify();
        });
      }
    }));
  }

  #[cfg(test)]
  fn poll_once_for_test(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };

    let include_history = self.sidebar_mode == GitSidebarMode::History;
    let cached_history_revision = self.history_revision.clone();
    let history_empty = self.history_commits.is_empty();
    let requested_repo = repo_root.clone();
    let task = cx.spawn(async move |this, cx| {
      let status = unblock(move || {
        let entries = list_repo_status(&repo_root).ok()?;
        let branch = current_branch_status(&repo_root).ok();
        let head_status = head_commit_status(&repo_root).ok();
        let merge_in_progress = is_merge_in_progress(&repo_root).unwrap_or(false);
        let rebase_in_progress = is_rebase_in_progress(&repo_root).unwrap_or(false);
        let rebase_commit_message = if rebase_in_progress {
          current_rebase_commit_message(&repo_root).ok().flatten()
        } else {
          None
        };
        let polled_history_revision = if include_history {
          current_history_revision(&repo_root).ok()
        } else {
          None
        };
        let should_refresh_history = Self::should_refresh_history_for_poll(
          include_history,
          history_empty,
          cached_history_revision.as_ref(),
          polled_history_revision.as_ref(),
        );
        let history = if should_refresh_history {
          list_commit_history(&repo_root, HISTORY_MAX_COMMITS).ok()
        } else {
          None
        };
        Some((
          entries,
          branch,
          head_status,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          polled_history_revision,
          should_refresh_history,
          history,
        ))
      })
      .await;

      let Some((
        entries,
        branch_status,
        head_status,
        merge_in_progress,
        rebase_in_progress,
        rebase_commit_message,
        polled_history_revision,
        should_refresh_history,
        history,
      )) = status
      else {
        return;
      };

      let _ = this.update(cx, |this, cx| {
        if this.selected_repo.as_ref() != Some(&requested_repo) {
          return;
        }
        let branch_changed = this.apply_status_snapshot(
          entries,
          branch_status,
          head_status,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          false,
          cx,
        );
        if include_history {
          if let Some(history) = history {
            this.history_commits = history;
            this.sync_history_cache_with_commits();
            this.history_loading = false;
            if let Some(history_revision) = polled_history_revision {
              this.history_revision = Some(history_revision);
            }
            this.refresh_history_list(cx);
          } else if !should_refresh_history {
            if let Some(history_revision) = polled_history_revision {
              this.history_revision = Some(history_revision);
            }
          } else if this.history_loading {
            this.history_loading = false;
          }
        }
        if branch_changed {
          this.refresh_branches(cx);
        }
        if Self::should_refresh_file_list(this.sidebar_mode) {
          this.refresh_file_list(cx);
        }
        cx.notify();
      });
    });

    self.status_task = Some(task);
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn show_file_search_action(
    &mut self,
    _: &ShowFileSearch,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_file_search_palette(window, cx);
  }

  fn find_action(&mut self, action: &Find, window: &mut Window, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };

    editor.update(cx, |editor, cx| {
      editor::find(editor, action, window, cx);
    });
  }

  fn close_find_action(&mut self, action: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };

    editor.update(cx, |editor, cx| {
      editor::close_find(editor, action, window, cx);
    });
  }

  fn open_repository_action(
    &mut self,
    _: &OpenRepository,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.start_open_repository(window, cx);
  }

  fn start_open_repository(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
      files: false,
      directories: true,
      multiple: false,
      prompt: Some("Select a repository".into()),
    });

    cx.spawn(async move |this, cx| {
      let Ok(result) = receiver.await else {
        return;
      };

      match result {
        Ok(Some(paths)) => {
          if let Some(path) = paths.into_iter().next() {
            ConfigStore::persist_recent_repository(&path);
            let _ = this.update(cx, |view, cx| {
              view.set_selected_repo(path, cx);
            });
          }
        }
        Ok(None) => {}
        Err(_) => {}
      }
    })
    .detach();
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let mut palette_branches = Vec::new();
    let mut palette_stashes = Vec::new();
    let mut palette_default_stash_message: Option<SharedString> = None;
    let mut palette_repositories = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| CommandPaletteRepository {
        path: repo.path.to_string_lossy().replace(['\n', '\r'], "").into(),
      })
      .collect::<Vec<_>>();

    if let Some(selected_repo) = self.selected_repo.as_ref() {
      let selected_repo_path = selected_repo.to_string_lossy().replace(['\n', '\r'], "");
      if !palette_repositories
        .iter()
        .any(|repo| repo.path.as_ref() == selected_repo_path)
      {
        palette_repositories.insert(
          0,
          CommandPaletteRepository {
            path: selected_repo_path.into(),
          },
        );
      }
    }

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let include_github = matches!(self.auth_state, AuthState::Authenticated(_));
    let mut commands =
      CommandPaletteCommand::default_global_commands(CommandPalettePage::Git, include_github);

    if palette_repositories.len() > 1 {
      commands.push(CommandPaletteCommand::switch_repository());
    }

    commands.push(CommandPaletteCommand::open_repository());

    if let Some(root_path) = self.selected_repo.clone() {
      if Self::should_show_unstage_all_command(&self.status_entries) {
        commands.push(CommandPaletteCommand::unstage_all());
      } else if Self::should_show_stage_all_command(&self.status_entries) {
        commands.push(CommandPaletteCommand::stage_all());
      }
      commands.push(CommandPaletteCommand::fetch());
      commands.push(CommandPaletteCommand::cherry_pick());

      let (show_stash, show_stash_with_untracked) = Self::stash_command_flags(&self.status_entries);

      if show_stash {
        commands.push(CommandPaletteCommand::stash());
      }

      if show_stash_with_untracked {
        commands.push(CommandPaletteCommand::stash_with_untracked());
        palette_default_stash_message = default_stash_message(&root_path).ok().map(Into::into);
      }

      if let Ok(stashes) = list_stashes(&root_path) {
        palette_stashes = stashes
          .into_iter()
          .map(|stash| CommandPaletteStash {
            index: stash.index,
            name: stash.name.into(),
            oid: stash.oid.into(),
          })
          .collect();

        if !palette_stashes.is_empty() {
          commands.push(CommandPaletteCommand::apply_stash());
          commands.push(CommandPaletteCommand::drop_stash());
          commands.push(CommandPaletteCommand::pop_stash());
        }
      }
    }

    if let Some(root_path) = self.selected_repo.clone()
      && let Ok(branches) = list_branches(&root_path)
    {
      palette_branches = branches
        .into_iter()
        .map(|branch| CommandPaletteBranch {
          name: branch.name.into(),
          kind: match branch.kind {
            BranchKind::Local => CommandPaletteBranchKind::Local,
            BranchKind::Remote => CommandPaletteBranchKind::Remote,
          },
        })
        .collect::<Vec<_>>();
      commands.push(CommandPaletteCommand::switch_branch());
      commands.push(CommandPaletteCommand::merge_branch());
      if self.merge_in_progress {
        commands.push(CommandPaletteCommand::abort_merge());
      }
      commands.push(CommandPaletteCommand::rebase_branch());
      if self.rebase_in_progress {
        commands.push(CommandPaletteCommand::abort_rebase());
      }
    }

    let mut config = CommandPaletteConfig::new(palette_branches, commands, handler)
      .with_repositories(palette_repositories)
      .with_stashes(palette_stashes);
    if let Some(default_stash_message) = palette_default_stash_message {
      config = config.with_default_stash_message(default_stash_message);
    }

    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    let palette_for_dialog = palette.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      dialog
        .p_0()
        .border_0()
        .min_h_0()
        .overlay_closable(true)
        .keyboard(true)
        .close_button(false)
        .child(palette_for_dialog.clone())
    });
  }

  fn open_file_search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.selected_repo.is_none() || self.status_entries.is_empty() {
      return;
    }

    let entries = self
      .status_entries
      .iter()
      .map(|entry| {
        let file_label = entry.path.to_string_lossy();
        let file_label = file_label.replace(['\n', '\r'], "");
        SearchFileEntry::new(entry.path.clone(), file_label)
      })
      .collect::<Vec<_>>();

    let view = cx.entity();
    let handler: SearchFileHandler = Arc::new(move |path, window, cx| {
      view.update(cx, |view, cx| {
        view.open_file(path, cx);
      });

      let view_for_focus = view.clone();
      window.on_next_frame(move |window, cx| {
        if let Some(editor) = view_for_focus.read(cx).editor.clone() {
          let focus_handle: FocusHandle = editor.read(cx).focus_handle(cx);
          window.focus(&focus_handle, cx);
        } else {
          let focus_handle = view_for_focus.read(cx).focus_handle(cx);
          window.focus(&focus_handle, cx);
        }
      });
      Ok(())
    });

    let palette = cx
      .new(|cx| SearchFilePalette::new(window, cx, SearchFilePaletteConfig::new(entries, handler)));
    let palette_for_dialog = palette.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      dialog
        .p_0()
        .border_0()
        .min_h_0()
        .overlay_closable(true)
        .keyboard(true)
        .close_button(false)
        .child(palette_for_dialog.clone())
    });
  }

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let mut selected_branch: Option<BranchRef> = None;
    let mut should_post_action_refresh = true;
    let result = match action {
      CommandPaletteAction::OpenRepository => {
        self.start_open_repository(window, cx);
        Ok(())
      }
      CommandPaletteAction::OpenGitPage => {
        WorkspaceRoute::global_mut(cx).page = WorkspacePage::Git;
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGithubPage => {
        GithubPageHandle::refresh(cx);
        WorkspaceRoute::global_mut(cx).page = WorkspacePage::Github;
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
      } => {
        GithubPrDetailsPageHandle::show(owner.into(), repo.into(), number, cx);
        Ok(())
      }
      CommandPaletteAction::OpenSettingsPage => {
        WorkspaceRoute::open_settings(cx);
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGitConfigPage => {
        WorkspaceRoute::open_git_config(cx);
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGitHistorySidebar => {
        self.set_sidebar_mode(GitSidebarMode::History, window, cx);
        Ok(())
      }
      CommandPaletteAction::OpenGitChangesSidebar => {
        self.set_sidebar_mode(GitSidebarMode::Changes, window, cx);
        Ok(())
      }
      CommandPaletteAction::SwitchRepository(repository) => {
        let repo_root = PathBuf::from(repository.path.as_ref());
        if !repo_root.is_dir() {
          let message: SharedString =
            format!("Repository not found: {}", repo_root.display()).into();
          return Err(message);
        }
        self.set_selected_repo(repo_root.clone(), cx);
        let mut recent = ConfigStore::load_recent_repositories();
        if !recent.iter().any(|repo| repo.path == repo_root) {
          recent.insert(
            0,
            RecentRepository {
              path: repo_root.clone(),
            },
          );
        }
        let items = recent
          .iter()
          .map(|repo| RecentRepoItem::new(repo, Some(&repo_root)))
          .collect::<Vec<_>>();
        self.repo_select.update(cx, |state, cx| {
          state.set_items(SearchableVec::new(items), window, cx);
          state.set_selected_value(&repo_root, window, cx);
        });
        Ok(())
      }
      CommandPaletteAction::SwitchBranch(branch) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let branch_ref = BranchRef {
          name: branch.name.to_string(),
          kind: match branch.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        selected_branch = Some(branch_ref.clone());
        switch_branch(&root_path, &branch_ref)
      }
      CommandPaletteAction::CreateBranch { name } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let branch_ref = BranchRef {
          name: name.clone(),
          kind: BranchKind::Local,
        };
        selected_branch = Some(branch_ref.clone());
        create_branch(&root_path, &name).and_then(|_| switch_branch(&root_path, &branch_ref))
      }
      CommandPaletteAction::CreateBranchFrom { name, base } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let branch_ref = BranchRef {
          name: base.name.to_string(),
          kind: match base.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        let new_branch = BranchRef {
          name: name.clone(),
          kind: BranchKind::Local,
        };
        selected_branch = Some(new_branch.clone());
        create_branch_from(&root_path, &name, &branch_ref)
          .and_then(|_| switch_branch(&root_path, &new_branch))
      }
      CommandPaletteAction::MergeBranch { name } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let target_branch = self
          .branch_status
          .as_ref()
          .map(|status| status.name.clone())
          .or_else(|| {
            current_branch_status(&root_path)
              .ok()
              .map(|status| status.name)
          })
          .unwrap_or_else(|| "HEAD".to_string());
        let branch_ref = BranchRef {
          name: name.name.to_string(),
          kind: match name.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        match merge_branch(&root_path, &branch_ref) {
          Ok(()) => Ok(()),
          Err(err) => {
            if let Some(path) = Self::first_conflicted_path(&root_path) {
              let merge_message =
                Self::merge_commit_message(branch_ref.name.as_str(), target_branch.as_str());
              self
                .commit_input
                .update(cx, |input, cx| input.set_value(&merge_message, window, cx));
              self.open_file(path, cx);
              Ok(())
            } else {
              Err(err)
            }
          }
        }
      }
      CommandPaletteAction::AbortMerge => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = abort_merge(&root_path);
        if result.is_ok() {
          self
            .commit_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        }
        result
      }
      CommandPaletteAction::RebaseBranch { name } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let branch_ref = BranchRef {
          name: name.name.to_string(),
          kind: match name.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        match rebase_branch(&root_path, &branch_ref) {
          Ok(()) => {
            self.force_push_after_rebase = true;
            Ok(())
          }
          Err(err) => {
            if let Some(path) = Self::first_conflicted_path(&root_path) {
              if let Some(rebase_message) = current_rebase_commit_message(&root_path).ok().flatten()
              {
                self
                  .commit_input
                  .update(cx, |input, cx| input.set_value(&rebase_message, window, cx));
              }
              self.open_file(path, cx);
              Ok(())
            } else {
              Err(err)
            }
          }
        }
      }
      CommandPaletteAction::AbortRebase => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = abort_rebase(&root_path);
        if result.is_ok() {
          self.force_push_after_rebase = false;
          self
            .commit_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        }
        result
      }
      CommandPaletteAction::StageAll => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        should_post_action_refresh = false;
        if Self::should_confirm_stage_all(self.selected_repo.as_ref(), &self.status_entries) {
          self.confirm_stage_all_conflicted_action(window, cx);
        } else {
          self.stage_all_action(cx);
        }
        Ok(())
      }
      CommandPaletteAction::UnstageAll => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        should_post_action_refresh = false;
        self.unstage_all_action(cx);
        Ok(())
      }
      CommandPaletteAction::Fetch => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        should_post_action_refresh = false;
        self.fetch_repository(root_path, cx);
        Ok(())
      }
      CommandPaletteAction::Stash {
        include_untracked,
        message,
      } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        create_stash(&root_path, include_untracked, message.as_deref())
      }
      CommandPaletteAction::ApplyStash(stash) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        apply_stash(&root_path, stash.index)
      }
      CommandPaletteAction::DropStash(stash) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        drop_stash(&root_path, stash.index)
      }
      CommandPaletteAction::PopStash(stash) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        pop_stash(&root_path, stash.index)
      }
      CommandPaletteAction::CherryPick { commit_hashes } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        cherry_pick_commits(&root_path, &commit_hashes)
      }
    };

    if let Err(err) = result {
      let message: SharedString = format!("Action failed: {err}").into();
      return Err(message);
    }

    if let Some(selected_branch) = selected_branch {
      let branch_select = self.branch_select.clone();
      let window_handle = self.window_handle;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        branch_select.update(cx, |state, cx| {
          state.set_selected_value(&selected_branch, window, cx);
        });
      });
    }

    if should_post_action_refresh {
      self.reload_status(cx);
      self.refresh_branches(cx);
      if let Some(editor) = self.editor.clone() {
        editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
      }
    }
    Ok(())
  }

  fn commit_changes_action(
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

  fn commit_changes(&mut self, _: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    self.commit_changes_inner(window, cx);
  }

  fn commit_changes_inner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        if result.is_ok() {
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value("", window, cx));
          });
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  fn continue_rebase_inner(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.rebase_in_progress {
      return;
    }
    self.operation_error = None;
    if Self::has_conflicted_entries(&self.status_entries) {
      self.operation_error = Some("Resolve all conflicts before continuing the rebase.".into());
      cx.notify();
      return;
    }

    let commit_input = self.commit_input.clone();
    let window_handle = self.window_handle;
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_continue = repo_root.clone();
      let result = unblock(move || continue_rebase(&repo_root_for_continue)).await;
      let (success, conflicted_path, error_message) = match result {
        Ok(()) => (true, None, None),
        Err(err) => {
          let conflicted_path = Self::first_conflicted_path(&repo_root);
          let is_conflict_state =
            conflicted_path.is_some() || err.to_string().contains("rebase has conflicts");
          let error_message = if is_conflict_state {
            None
          } else {
            Some(format!("Continue rebase failed: {err}"))
          };
          (false, conflicted_path, error_message)
        }
      };
      let _ = this.update(cx, |this, cx| {
        if success {
          this.rebase_in_progress = false;
          this.force_push_after_rebase = true;
          this.operation_error = None;
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value("", window, cx));
          });
        }
        this.reload_status(cx);
        if let Some(path) = conflicted_path {
          this.open_file(path, cx);
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

  fn commit_amend_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        if result.is_ok() {
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value("", window, cx));
          });
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  fn undo_last_commit_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.can_undo_last_commit {
      return;
    }

    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || undo_last_commit(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  fn fetch_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.fetch_repository(repo_root, cx);
  }

  fn fetch_repository(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    if self.fetch_in_progress {
      return;
    }
    self.fetch_in_progress = true;
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || fetch(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.fetch_in_progress = false;
        this.reload_status(cx);
        this.refresh_branches(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  fn push_changes_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.can_push {
      return;
    }

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || push(&repo_root, false)).await;
      let _ = this.update(cx, |this, cx| {
        if result.is_ok() {
          this.force_push_after_rebase = false;
        }
        this.reload_status(cx);
      });
    });

    self.status_task = Some(task);
  }

  fn force_push_changes_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.can_force_push {
      return;
    }

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || push(&repo_root, true)).await;
      let _ = this.update(cx, |this, cx| {
        if result.is_ok() {
          this.force_push_after_rebase = false;
        }
        this.reload_status(cx);
      });
    });

    self.status_task = Some(task);
  }

  fn open_file(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if self.selected_file.as_ref() == Some(&rel_path) && self.history_opened_commit_file.is_none() {
      return;
    }
    let is_markdown = Self::is_markdown_path(&rel_path);
    if !is_markdown && !Self::is_svg_path(&rel_path) {
      self.show_markdown_preview = false;
    }

    self.invalidate_open_file_task();
    let generation = self.open_file_generation;
    let had_history_file_selection = self.history_opened_commit_file.is_some();
    self.history_opened_commit_file = None;
    self.selected_file = Some(rel_path.clone());
    self.editor = None;
    self.svg_preview = None;
    self.svg_preview_source = None;
    self.force_list_selection = true;
    let opened_path = self.selected_file.clone();
    self.file_list.update(cx, |state, cx| {
      state.delegate_mut().set_opened_path(opened_path);
      cx.notify();
    });
    let selected_index = self.selected_file_index();
    self.set_file_list_selected_index(selected_index, cx);
    if had_history_file_selection {
      self.refresh_history_list(cx);
    }

    let diff_view = self.effective_diff_view_for_path(&rel_path);
    let requested_repo = repo_root.clone();
    let requested_path = rel_path.clone();
    let file_path = requested_repo.join(&requested_path);
    let load_repo_root = requested_repo.clone();
    let load_file_path = file_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let loaded =
        unblock(move || Editor::load_file_for_editor(&load_repo_root, &load_file_path)).await;
      let _ = this.update(cx, move |this, cx| {
        if this.open_file_generation != generation {
          return;
        }
        if this.selected_repo.as_ref() != Some(&requested_repo) {
          return;
        }
        if this.selected_file.as_ref() != Some(&requested_path) {
          return;
        }
        if this.history_opened_commit_file.is_some() {
          return;
        }

        let editor_repo_root = requested_repo.clone();
        let editor_file_path = file_path.clone();
        let editor = cx.new(move |cx| {
          Editor::new_with_loaded_file(editor_repo_root, editor_file_path, loaded, cx)
        });
        editor.update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
        this.editor = Some(editor);
        cx.notify();
      });
    });
    self.open_file_task = Some(task);
    cx.notify();
  }

  fn queue_history_commit_files_load(
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

  fn load_history_commit_files(&mut self, commit_oid: String, cx: &mut Context<Self>) {
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
      let files =
        unblock(move || list_commit_changed_files(&load_repo_root, &load_commit_oid)).await;
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

  fn open_history_commit_file(
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
    self.refresh_history_list(cx);
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let load_repo_root = repo_root.clone();
      let load_commit_oid = commit_oid.clone();
      let load_rel_path = rel_path.clone();
      let commit_file =
        unblock(move || load_commit_file_diff(&load_repo_root, &load_commit_oid, &load_rel_path))
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

        editor.update(cx, |editor, cx| {
          editor.load_readonly_snapshot(commit_file.content, diff_set, cx);
          editor.set_diff_view_mode(diff_view, cx);
        });

        this.clear_markdown_preview_if_not_previewable(&rel_path);
        this.editor = Some(editor);
        this.selected_file = Some(rel_path.clone());
        this.history_opened_commit_file = Some((commit_oid.clone(), rel_path.clone()));
        this.svg_preview = None;
        this.svg_preview_source = None;
        this.refresh_history_list(cx);
        cx.notify();
      });
    });

    self.history_open_file_task = Some(task);
  }

  fn clear_markdown_preview_if_not_previewable(&mut self, rel_path: &Path) {
    if !Self::is_markdown_path(rel_path) && !Self::is_svg_path(rel_path) {
      self.show_markdown_preview = false;
    }
  }

  fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
    if let Some(selected) = self.selected_file.as_ref()
      && self.split_disabled_for_path(selected)
    {
      return;
    }
    if self.show_markdown_preview {
      self.show_markdown_preview = false;
    }
    self.diff_view = match self.diff_view {
      DiffViewMode::Inline => DiffViewMode::Split,
      DiffViewMode::Split => DiffViewMode::Inline,
    };
    self.sync_diff_view(cx);
    cx.notify();
  }

  fn toggle_markdown_preview(&mut self, cx: &mut Context<Self>) {
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      self.show_markdown_preview = false;
      self.sync_diff_view(cx);
      cx.notify();
      return;
    }

    self.show_markdown_preview = !self.show_markdown_preview;
    self.sync_diff_view(cx);
    cx.notify();
  }

  fn update_svg_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.show_markdown_preview || !self.selected_file_is_svg() {
      return;
    }

    let Some(editor) = self.editor.clone() else {
      return;
    };

    let document = editor.read(cx).document().read(cx);
    let svg_source = document.slice_to_string(0..document.len());
    let svg_source: SharedString = svg_source.into();

    if self.svg_preview_source.as_ref() == Some(&svg_source) {
      return;
    }

    self.svg_preview_source = Some(svg_source.clone());
    let renderer = cx.svg_renderer();
    let svg_bytes = svg_source.as_ref().as_bytes().to_vec();
    let background =
      cx.background_spawn(
        async move { renderer.render_single_frame(svg_bytes.as_slice(), 1.0, true) },
      );

    let task = cx.spawn_in(window, async move |this, cx| {
      let result = background.await;
      let _ = this.update_in(cx, |this, window, cx| {
        if let Some(Ok(image)) = this.svg_preview.take() {
          let _ = window.drop_image(image);
        }
        this.svg_preview = Some(result.map_err(|err| err.to_string().into()));
        cx.notify();
      });
    });

    self.svg_preview_task = Some(task);
  }

  fn toggle_sidebar_mode_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let next_mode = match self.sidebar_mode {
      GitSidebarMode::Changes => GitSidebarMode::History,
      GitSidebarMode::History => GitSidebarMode::Changes,
    };
    self.set_sidebar_mode(next_mode, window, cx);
  }

  fn focus_changes_sidebar_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.file_list.read(cx).selected_index().is_none() && !self.status_entries.is_empty() {
      self.file_list.update(cx, |state, cx| {
        state.set_selected_index(Some(IndexPath::new(0)), window, cx);
      });
    }

    self.file_list.update(cx, |state, cx| {
      state.focus(window, cx);
    });
  }

  fn focus_sidebar_on_next_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.sidebar_mode == GitSidebarMode::Changes {
      cx.on_next_frame(window, |this, window, cx| {
        if this.sidebar_mode == GitSidebarMode::Changes {
          this.focus_changes_sidebar_list(window, cx);
        }
      });
      return;
    }

    // Keep page-level shortcuts active in History mode without focusing the tree.
    window.focus(&self.focus_handle, cx);
  }

  fn set_sidebar_mode(
    &mut self,
    mode: GitSidebarMode,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.sidebar_mode = mode;

    if self.sidebar_mode == GitSidebarMode::History {
      self.refresh_history(cx);
    } else {
      self.refresh_file_list(cx);
      cx.notify();
    }

    self.focus_sidebar_on_next_frame(window, cx);
  }

  fn toggle_stage_all_action(
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

  fn restore_all_click_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.confirm_restore_all_action(window, cx);
  }

  fn abort_merge_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
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
        if result.is_ok() {
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value("", window, cx));
          });
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn abort_rebase_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
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
        if result.is_ok() {
          this.force_push_after_rebase = false;
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value("", window, cx));
          });
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn continue_rebase_action(
    &mut self,
    _: &gpui::ClickEvent,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.continue_rebase_inner(cx);
  }

  fn stage_all_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || stage_all(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn unstage_all_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || unstage_all(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn stage_file_action(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let rel_path_for_job = rel_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || stage_file(&repo_root, &rel_path_for_job)).await;
      let _ = this.update(cx, |this, cx| {
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

  fn unstage_file_action(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let rel_path_for_job = rel_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || unstage_file(&repo_root, &rel_path_for_job)).await;
      let _ = this.update(cx, |this, cx| {
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

  fn restore_file_action(
    &mut self,
    rel_path: PathBuf,
    status: RepoStatusKind,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let rel_path_for_job = rel_path.clone();
    let should_delete = Self::restore_uses_delete(status);
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        if should_delete {
          delete_untracked_file(&repo_root, &rel_path_for_job)
        } else {
          restore_file(&repo_root, &rel_path_for_job)
        }
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        if result.is_ok() {
          this.select_first_file_after_restore = true;
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

  fn restore_all_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if self.status_entries.is_empty() {
      return;
    }
    let entries = self.status_entries.clone();
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || {
        for entry in entries {
          if Self::restore_uses_delete(entry.status) {
            let _ = delete_untracked_file(&repo_root, &entry.path);
          } else {
            let _ = restore_file(&repo_root, &entry.path);
          }
        }
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        this.select_first_file_after_restore = true;
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn confirm_stage_conflicted_file_action(
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

    window.open_dialog(cx, move |dialog, _, _| {
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
        .build(dialog)
    });
  }

  fn confirm_stage_all_conflicted_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let title: SharedString = "Mark conflicts as resolved?".into();
    let message: SharedString = "Stage all files and mark merge conflicts as resolved?".into();
    let view = cx.entity();

    window.open_dialog(cx, move |dialog, _, _| {
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
        .build(dialog)
    });
  }

  fn confirm_restore_file_action(
    &mut self,
    window: &mut Window,
    rel_path: PathBuf,
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

    window.open_dialog(cx, move |dialog, _, _| {
      let view = view.clone();
      let rel_path_for_action = rel_path_for_action.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text(confirm_text.clone())
        .cancel_text("Cancel")
        .destructive()
        .on_confirm(move |_, _, cx| {
          let rel_path_for_action = rel_path_for_action.clone();
          view.update(cx, |view, cx| {
            view.restore_file_action(rel_path_for_action, status, cx);
          });
          true
        })
        .build(dialog)
    });
  }

  fn confirm_restore_all_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    window.open_dialog(cx, move |dialog, _, _| {
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
        .build(dialog)
    });
  }

  fn stage_style(
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

  fn status_color(kind: RepoStatusKind, theme: &gpui_component::Theme) -> gpui::Hsla {
    match kind {
      RepoStatusKind::Modified => theme.status_yellow(),
      RepoStatusKind::Added => theme.status_green(),
      RepoStatusKind::Deleted => theme.status_red(),
      RepoStatusKind::Renamed => theme.status_blue(),
      RepoStatusKind::TypeChange => theme.status_blue(),
      RepoStatusKind::Untracked => theme.status_green(),
      RepoStatusKind::Conflicted => theme.status_red(),
    }
  }

  fn status_tooltip(kind: RepoStatusKind) -> SharedString {
    match kind {
      RepoStatusKind::Modified => "Modified".into(),
      RepoStatusKind::Added => "Added".into(),
      RepoStatusKind::Deleted => "Deleted".into(),
      RepoStatusKind::Renamed => "Renamed".into(),
      RepoStatusKind::TypeChange => "Type changed".into(),
      RepoStatusKind::Untracked => "Untracked".into(),
      RepoStatusKind::Conflicted => "Conflicted".into(),
    }
  }

  fn should_publish_branch(branch_status: Option<&BranchStatus>, has_head_commit: bool) -> bool {
    has_head_commit
      && matches!(
        branch_status,
        Some(status) if !status.has_upstream && status.name != "HEAD"
      )
  }

  fn push_action_label(
    branch_status: Option<&BranchStatus>,
    has_head_commit: bool,
  ) -> &'static str {
    if Self::should_publish_branch(branch_status, has_head_commit) {
      "Push (Publish branch)"
    } else {
      "Push"
    }
  }

  fn push_flags(
    branch_status: Option<&BranchStatus>,
    has_head_commit: bool,
    force_push_after_rebase: bool,
  ) -> (bool, bool) {
    let Some(status) = branch_status else {
      return (false, false);
    };
    if Self::should_publish_branch(Some(status), has_head_commit) {
      return (true, false);
    }
    if !status.has_upstream {
      return (false, false);
    }
    if force_push_after_rebase && status.ahead > 0 {
      return (false, true);
    }
    let can_push = status.ahead > 0 && status.behind == 0;
    let can_force_push = status.ahead > 0 && status.behind > 0;
    (can_push, can_force_push)
  }

  fn all_changes_staged(&self) -> bool {
    Self::all_entries_staged(&self.status_entries)
  }

  fn changed_files_count(entries: &[RepoStatusEntry]) -> usize {
    entries.len()
  }

  fn has_conflicted_entries(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| entry.status == RepoStatusKind::Conflicted)
  }

  fn has_untracked_entries(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| entry.status == RepoStatusKind::Untracked)
  }

  fn has_tracked_entries(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| entry.status != RepoStatusKind::Untracked)
  }

  fn stash_command_flags(entries: &[RepoStatusEntry]) -> (bool, bool) {
    let show_stash = Self::has_tracked_entries(entries);
    let show_stash_with_untracked = show_stash || Self::has_untracked_entries(entries);
    (show_stash, show_stash_with_untracked)
  }

  fn should_show_stage_all_command(entries: &[RepoStatusEntry]) -> bool {
    Self::changed_files_count(entries) > 0 && !Self::all_entries_staged(entries)
  }

  fn should_show_unstage_all_command(entries: &[RepoStatusEntry]) -> bool {
    Self::all_entries_staged(entries)
  }

  fn should_confirm_stage_all(
    selected_repo: Option<&PathBuf>,
    status_entries: &[RepoStatusEntry],
  ) -> bool {
    selected_repo.is_some() && Self::has_conflicted_entries(status_entries)
  }

  fn merge_commit_message(source_branch: &str, target_branch: &str) -> String {
    format!("Merge branch '{source_branch}' into {target_branch}")
  }

  fn sync_rebase_commit_input(
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

  fn should_show_changed_files_tag(changed_files_count: usize) -> bool {
    changed_files_count > 0
  }

  fn build_history_rows(commits: &[HistoryCommitNode]) -> Vec<HistoryRenderRow> {
    commits
      .iter()
      .cloned()
      .map(HistoryRenderRow::from_commit)
      .collect()
  }

  fn history_change_kind_to_repo_status(kind: CommitFileChangeKind) -> RepoStatusKind {
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

  fn render_history_sidebar_content(
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
                return ListItem::new(ix)
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

              ListItem::new(ix)
                .w_full()
                .rounded(theme.radius)
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
              let status_kind = Self::history_change_kind_to_repo_status(file.kind);
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

              ListItem::new(ix)
                .selected(selected)
                .w_full()
                .rounded(theme.radius)
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
              ListItem::new(ix)
                .w_full()
                .rounded(theme.radius)
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
            _ => ListItem::new(ix)
              .w_full()
              .rounded(theme.radius)
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

    div()
      .id("git-history-scroll-container")
      .flex_1()
      .min_h_0()
      .child(tree_view.flex_1().w_full())
      .into_any_element()
  }

  fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let select = Select::new(&self.repo_select)
      .placeholder("Select repository...")
      .search_placeholder("Search repositories...")
      .menu_width(px(360.))
      .w(px(360.));

    let branch_select = Select::new(&self.branch_select)
      .placeholder("Select branch...")
      .search_placeholder("Search branches...")
      .menu_width(px(300.))
      .w(px(240.))
      .disabled(self.selected_repo.is_none());

    let branch_info = self.branch_status.as_ref().map(|status| {
      let ahead = status.ahead;
      let behind = status.behind;
      let ahead_color = if ahead > 0 {
        theme.status_green()
      } else {
        theme.muted_foreground
      };
      let behind_color = if behind > 0 {
        theme.status_red()
      } else {
        theme.muted_foreground
      };

      div()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .bg(theme.background)
        .border_1()
        .border_color(theme.title_bar_border)
        .child(
          div()
            .flex()
            .items_center()
            .gap_2()
            .child(
              div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                  Icon::new(IconName::ArrowUp)
                    .size_3()
                    .text_color(ahead_color),
                )
                .child(
                  div()
                    .text_xs()
                    .text_color(ahead_color)
                    .child(ahead.to_string()),
                ),
            )
            .child(
              div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                  Icon::new(IconName::ArrowDown)
                    .size_3()
                    .text_color(behind_color),
                )
                .child(
                  div()
                    .text_xs()
                    .text_color(behind_color)
                    .child(behind.to_string()),
                ),
            ),
        )
    });

    let fetch_button = Button::new("git-fetch-button")
      .label("Fetch")
      .icon(UiIconName::RefreshCcw)
      .loading_icon(Icon::new(UiIconName::RefreshCcw))
      .loading(self.fetch_in_progress)
      .with_variant(ButtonVariant::Secondary)
      .xsmall()
      .disabled(self.selected_repo.is_none() || self.fetch_in_progress)
      .tooltip("Fetch updates from remotes")
      .on_click(cx.listener(Self::fetch_action));

    let header_left = div()
      .flex()
      .items_center()
      .gap_3()
      .child(
        div()
          .text_sm()
          .text_color(theme.foreground)
          .child("Repository"),
      )
      .child(select)
      .child(div().text_sm().text_color(theme.foreground).child("Branch"))
      .child(branch_select)
      .when_some(branch_info, |this, info| this.child(info))
      .child(fetch_button);

    let menu_state = match &self.auth_state {
      AuthState::Unknown => UserMenuState::Unknown,
      AuthState::Unauthenticated => UserMenuState::Unauthenticated,
      AuthState::Authenticated(user) => {
        let display_name = if user.name.trim().is_empty() {
          user.email.clone()
        } else {
          user.name.clone()
        };
        UserMenuState::Authenticated(UserMenuUser {
          name: display_name.into(),
          email: user.email.clone().into(),
          image: user.image.clone().map(Into::into),
        })
      }
    };
    let is_unauthenticated = matches!(self.auth_state, AuthState::Unauthenticated);

    let sign_in_view = cx.entity();
    let open_git = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::global_mut(cx).page = WorkspacePage::Git;
      cx.refresh_windows();
    });
    let open_github = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      GithubPageHandle::refresh(cx);
      WorkspaceRoute::global_mut(cx).page = WorkspacePage::Github;
      cx.refresh_windows();
    });
    let open_settings = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::open_settings(cx);
      cx.refresh_windows();
    });
    let open_git_config = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::open_git_config(cx);
      cx.refresh_windows();
    });
    let sign_in = Rc::new(move |_window: &mut Window, cx: &mut App| {
      sign_in_view.update(cx, |this, cx| this.start_github_sign_in(cx));
    });
    let sign_out_view = cx.entity();
    let sign_out = Rc::new(move |_window: &mut Window, cx: &mut App| {
      sign_out_view.update(cx, |this, cx| this.logout(cx));
    });

    let auth_control = user_menu(UserMenuConfig {
      id: "auth-menu".into(),
      state: menu_state,
      current_page: UserMenuPage::Git,
      on_open_git: Some(open_git),
      on_open_github: Some(open_github),
      on_open_git_config: Some(open_git_config),
      on_open_settings: Some(open_settings),
      on_sign_in: Some(sign_in),
      on_sign_out: Some(sign_out),
    });

    let sign_in_button_view = cx.entity();
    let sign_in_button = Button::new("git-sign-in")
      .icon(IconName::GitHub)
      .label("Sign in with GitHub")
      .ghost()
      .gap_2()
      .small()
      .on_click(move |_, _, cx| {
        sign_in_button_view.update(cx, |this, cx| this.start_github_sign_in(cx));
      });

    let header_right = h_flex()
      .items_center()
      .gap_2()
      .when(is_unauthenticated, |this| this.child(sign_in_button))
      .when_some(auth_control, |this, control| this.child(control));

    div()
      .h(px(HEADER_HEIGHT))
      .min_h(px(HEADER_HEIGHT))
      .max_h(px(HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(header_left)
      .child(header_right)
  }

  fn render_empty_state(&self, message: &str, cx: &mut Context<Self>) -> AnyElement {
    let message = message.to_string();
    let theme = cx.theme().clone();
    div()
      .size_full()
      .flex()
      .px_2()
      .bg(theme.background)
      .items_center()
      .justify_center()
      .text_color(cx.theme().muted_foreground)
      .child(div().truncate().child(message))
      .into_any_element()
  }

  fn render_loading_state(&self, message: &str, cx: &mut Context<Self>) -> AnyElement {
    let message = message.to_string();
    let theme = cx.theme().clone();
    div()
      .size_full()
      .flex()
      .bg(theme.background)
      .items_center()
      .justify_center()
      .child(
        div()
          .id("git-editor-loading-state")
          .flex()
          .flex_col()
          .items_center()
          .gap_2()
          .child(Spinner::new().small())
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child(message),
          ),
      )
      .into_any_element()
  }

  fn should_show_editor_loading_state(selected_file: Option<&Path>, has_editor: bool) -> bool {
    selected_file.is_some() && !has_editor
  }

  fn render_editor_header(&self, editor: &Entity<Editor>, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let editor_state = editor.read(cx);
    let is_history_commit_file = self.history_opened_commit_file.is_some();
    let selected_entry = self
      .selected_file
      .as_ref()
      .and_then(|path| self.status_entries.iter().find(|entry| &entry.path == path))
      .cloned();
    let display_path = selected_entry
      .as_ref()
      .map(|entry| entry.path.as_path())
      .or(self.selected_file.as_deref())
      .unwrap_or(editor_state.workdir_path.as_path());
    let file_name = format_git_file_label(display_path, GitFileLabelFormat::FileName);
    let old_file_name = selected_entry
      .as_ref()
      .and_then(|entry| entry.old_path.as_ref())
      .map(|path| format_git_file_label(path, GitFileLabelFormat::FileName));
    let dir_path = display_path
      .parent()
      .and_then(|parent| parent.to_str())
      .unwrap_or("")
      .to_string();
    let file_dirty = editor_state.is_dirty;
    let editor_entity = editor.clone();
    let status_kind = selected_entry.as_ref().map(|entry| entry.status);
    let status_letter = status_kind.map(|status| status.short_code());
    let status_color = status_kind
      .map(|status| Self::status_color(status, &theme))
      .unwrap_or(theme.muted_foreground);

    let title = h_flex()
      .items_center()
      .gap_2()
      .min_w_0()
      .flex_1()
      .when_some(status_letter, |this, letter| {
        this.child(
          div()
            .w(px(15.))
            .text_xs()
            .text_color(status_color)
            .child(letter),
        )
      })
      .child(
        file_icon_path_for_path_with_theme(&editor_state.workdir_path, &theme)
          .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
          .unwrap_or_else(|| {
            Icon::new(IconName::File)
              .size_3()
              .text_color(theme.foreground)
              .into_any_element()
          }),
      )
      .child(
        h_flex()
          .min_w_0()
          .flex_1()
          .items_center()
          .gap_2()
          .child(
            h_flex()
              .min_w_0()
              .items_center()
              .gap_2()
              .child(render_repo_status_label(
                &theme,
                status_kind,
                file_name,
                old_file_name,
              ))
              .when(file_dirty, |this| {
                this.child(
                  div()
                    .size_2()
                    .rounded_full()
                    .bg(theme.foreground)
                    .flex_shrink_0(),
                )
              }),
          )
          .when(!dir_path.is_empty(), |this| {
            this.child(
              div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .text_ellipsis_start()
                .text_color(theme.muted_foreground)
                .child(format!("- {}", dir_path)),
            )
          }),
      );

    let save_button = Button::new("editor-save")
      .label("Save")
      .xsmall()
      .ghost()
      .disabled(!file_dirty || is_history_commit_file || editor_state.is_read_only)
      .on_click(move |_, _, cx| {
        editor_entity.update(cx, |editor, cx| editor.save(cx));
      });

    let is_markdown = self.selected_file_is_markdown();
    let is_svg = self.selected_file_is_svg();
    let preview_active = (is_markdown || is_svg) && self.show_markdown_preview;
    let split_disabled = self
      .selected_file
      .as_ref()
      .map(|path| self.split_disabled_for_path(path))
      .unwrap_or(false)
      || preview_active;
    let (toggle_label, toggle_icon) = if split_disabled {
      ("Split", IconName::PanelLeft)
    } else {
      match self.diff_view {
        DiffViewMode::Inline => ("Split", IconName::PanelLeft),
        DiffViewMode::Split => ("Inline", IconName::PanelLeftClose),
      }
    };
    let view = cx.entity();
    let toggle_button = Button::new("editor-diff-toggle")
      .label(toggle_label)
      .icon(toggle_icon)
      .xsmall()
      .ghost()
      .disabled(split_disabled)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_diff_view(cx);
        });
      });

    let view = cx.entity();
    let preview_button = Button::new("editor-markdown-preview")
      .label("Preview")
      .icon(if preview_active {
        IconName::EyeOff
      } else {
        IconName::Eye
      })
      .xsmall()
      .ghost()
      .selected(preview_active)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_markdown_preview(cx);
        });
      });

    let (can_stage, can_unstage, can_restore, file_path, file_status) = if is_history_commit_file {
      (false, false, false, None, None)
    } else if let Some(entry) = selected_entry {
      (
        matches!(
          entry.stage,
          RepoStage::Unstaged | RepoStage::PartiallyStaged
        ),
        matches!(entry.stage, RepoStage::Staged | RepoStage::PartiallyStaged),
        matches!(entry.stage, RepoStage::Unstaged),
        Some(entry.path.clone()),
        Some(entry.status),
      )
    } else {
      (false, false, false, None, None)
    };
    let show_accept_all_conflict_actions = matches!(file_status, Some(RepoStatusKind::Conflicted));
    let can_accept_all_conflicts = show_accept_all_conflict_actions
      && !editor_state.is_read_only
      && editor_state.has_unresolved_conflict_markers(cx);

    let editor_entity_accept_current = editor.clone();
    let accept_all_current_button = Button::new("editor-accept-all-current")
      .label("Accept All Current")
      .xsmall()
      .ghost()
      .disabled(!can_accept_all_conflicts)
      .on_click(move |_, _, cx| {
        editor_entity_accept_current.update(cx, |editor, cx| {
          editor.resolve_all_conflicts(ConflictResolution::Current, cx);
        });
      });

    let editor_entity_accept_incoming = editor.clone();
    let accept_all_incoming_button = Button::new("editor-accept-all-incoming")
      .label("Accept All Incoming")
      .xsmall()
      .ghost()
      .disabled(!can_accept_all_conflicts)
      .on_click(move |_, _, cx| {
        editor_entity_accept_incoming.update(cx, |editor, cx| {
          editor.resolve_all_conflicts(ConflictResolution::Incoming, cx);
        });
      });

    let file_path_stage = file_path.clone();
    let file_path_unstage = file_path.clone();
    let file_path_restore = file_path.clone();
    let file_status_for_stage = file_status;

    let view = cx.entity();
    let stage_button = Button::new("editor-stage-file")
      .label("Stage")
      .icon(IconName::Plus)
      .xsmall()
      .ghost()
      .disabled(!can_stage)
      .on_click(move |_, window, cx| {
        if let Some(path) = file_path_stage.clone() {
          view.update(cx, |this, cx| {
            let has_unresolved_conflict_markers = this.editor.as_ref().is_none_or(|editor| {
              editor.read_with(cx, |editor, cx| editor.has_unresolved_conflict_markers(cx))
            });
            if Self::should_confirm_stage_for_status(
              file_status_for_stage,
              has_unresolved_conflict_markers,
            ) {
              this.confirm_stage_conflicted_file_action(window, path.clone(), cx);
            } else {
              this.stage_file_action(path.clone(), cx);
            }
          });
        }
      });

    let view = cx.entity();
    let unstage_button = Button::new("editor-unstage-file")
      .label("Unstage")
      .icon(IconName::Minus)
      .xsmall()
      .ghost()
      .disabled(!can_unstage)
      .on_click(move |_, _, cx| {
        if let Some(path) = file_path_unstage.clone() {
          view.update(cx, |this, cx| {
            this.unstage_file_action(path.clone(), cx);
          });
        }
      });

    let view = cx.entity();
    let file_status_for_restore = file_status;
    let restore_button = Button::new("editor-restore-file")
      .label("Restore")
      .icon(IconName::Undo)
      .xsmall()
      .ghost()
      .disabled(!can_restore)
      .on_click(move |_, window, cx| {
        if let (Some(path), Some(status)) = (file_path_restore.clone(), file_status_for_restore) {
          view.update(cx, |this, cx| {
            this.confirm_restore_file_action(window, path.clone(), status, cx);
          });
        }
      });

    div()
      .min_h(px(EDITOR_HEADER_HEIGHT))
      .h(px(EDITOR_HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .gap_2()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(title)
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .flex_shrink_0()
          .when(show_accept_all_conflict_actions, |this| {
            this
              .child(accept_all_current_button)
              .child(accept_all_incoming_button)
          })
          .child(stage_button)
          .child(unstage_button)
          .child(restore_button)
          .child(save_button)
          .when(is_markdown || is_svg, |this| this.child(preview_button))
          .child(toggle_button),
      )
      .into_any_element()
  }

  fn render_editor_with_overlay(
    &mut self,
    editor: Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let overlay = self.render_change_block_actions(&editor, window, cx);
    let mut wrapper = div()
      .flex_1()
      .min_w(px(0.0))
      .min_h(px(0.0))
      .relative()
      .overflow_hidden()
      .child(editor);

    if let Some(overlay) = overlay {
      wrapper = wrapper.child(overlay);
    }

    wrapper.into_any_element()
  }

  fn render_change_block_actions(
    &mut self,
    editor: &Entity<Editor>,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    let theme = cx.theme().clone();
    let editor_state = editor.read(cx);
    if self.history_opened_commit_file.is_some() || editor_state.is_read_only {
      return None;
    }
    let selected_status = self
      .selected_file
      .as_ref()
      .and_then(|selected| {
        self
          .status_entries
          .iter()
          .find(|entry| &entry.path == selected)
      })
      .map(|entry| entry.status);

    if matches!(selected_status, Some(RepoStatusKind::Conflicted)) {
      let conflict_start_line = editor_state.hovered_conflict_start_line?;
      let anchor_display_line = editor_state
        .first_display_line_for_conflict(conflict_start_line)
        .unwrap_or(conflict_start_line);
      if editor_state.find_panel_occludes_display_line(anchor_display_line) {
        return None;
      }
      let mut top = Self::hunk_action_top(
        editor_state.measured_editor_line_height(),
        anchor_display_line,
        editor_state.scroll_offset_y,
      );
      if top >= editor_state.viewport_height {
        return None;
      }
      if top < px(0.0) {
        top = px(0.0);
      }

      let editor_entity = editor.clone();
      let mut actions = div().flex().items_center();

      let editor_entity_current = editor_entity.clone();
      actions = actions.child(
        Button::new("accept-current-conflict")
          .label("Accept Current")
          .small()
          .bg(theme.background)
          .rounded_t_none()
          .rounded_br_none()
          .on_click(move |_, _, cx| {
            editor_entity_current.update(cx, |editor, cx| {
              editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Current, cx);
            });
          }),
      );

      let editor_entity_incoming = editor_entity.clone();
      actions = actions.child(
        Button::new("accept-incoming-conflict")
          .label("Accept Incoming")
          .small()
          .bg(theme.background)
          .rounded_t_none()
          .on_click(move |_, _, cx| {
            editor_entity_incoming.update(cx, |editor, cx| {
              editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Incoming, cx);
            });
          }),
      );

      actions = actions.child(
        Button::new("accept-both-conflict")
          .label("Accept Both")
          .small()
          .bg(theme.background)
          .rounded_t_none()
          .rounded_bl_none()
          .on_click(move |_, _, cx| {
            editor_entity.update(cx, |editor, cx| {
              editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Both, cx);
            });
          }),
      );

      return Some(
        div()
          .absolute()
          .top(top)
          .right(px(30.0))
          .child(actions)
          .into_any_element(),
      );
    }

    let hovered_id = editor_state.hovered_group_id.as_ref()?;
    let overlay = editor_state
      .visible_groups
      .iter()
      .find(|overlay| overlay.id.as_ref() == hovered_id.as_ref())?;

    let anchor_display_line = editor_state
      .first_display_line_for_group(hovered_id)
      .unwrap_or(overlay.display_line);
    if editor_state.find_panel_occludes_display_line(anchor_display_line) {
      return None;
    }
    let mut top = Self::hunk_action_top(
      editor_state.measured_editor_line_height(),
      anchor_display_line,
      editor_state.scroll_offset_y,
    );
    if top >= editor_state.viewport_height {
      return None;
    }
    if top < px(0.0) {
      top = px(0.0);
    }
    let file_dirty = editor_state.is_dirty;

    if matches!(
      selected_status,
      Some(RepoStatusKind::Untracked | RepoStatusKind::Added)
    ) {
      return None;
    }

    let restore_disabled_by_status = matches!(
      selected_status,
      Some(RepoStatusKind::Untracked | RepoStatusKind::Added)
    );
    let restore_disabled = file_dirty || restore_disabled_by_status;

    let stage_tooltip = if file_dirty {
      "File not saved"
    } else {
      "Stage hunk"
    };
    let unstage_tooltip = if file_dirty {
      "File not saved"
    } else {
      "Unstage hunk"
    };
    let restore_tooltip = if file_dirty {
      "File not saved"
    } else if restore_disabled_by_status {
      "Restore unavailable for added/untracked files"
    } else {
      "Restore hunk"
    };

    let group_id = overlay.id.clone();
    let state = overlay.state;
    let editor_entity = editor.clone();

    let mut actions = div().flex().items_center();

    match state {
      HunkState::Unstaged => {
        let editor_entity = editor_entity.clone();
        let group_id = group_id.clone();
        actions = actions.child(
          Button::new("stage-hunk")
            .icon(IconName::Plus)
            .label("Stage")
            .small()
            .tooltip(stage_tooltip)
            .rounded_t_none()
            .rounded_br_none()
            .bg(theme.background)
            .disabled(file_dirty)
            .on_click(move |_, _, cx| {
              let group_id = group_id.clone();
              editor_entity.update(cx, |editor, cx| {
                editor.enqueue_group_action(group_id, HunkAction::Stage, cx);
              });
            }),
        );
      }
      HunkState::Staged => {
        let editor_entity = editor_entity.clone();
        let group_id = group_id.clone();
        actions = actions.child(
          Button::new("unstage-hunk")
            .icon(IconName::Minus)
            .label("Unstage")
            .tooltip(unstage_tooltip)
            .small()
            .disabled(file_dirty)
            .bg(theme.background)
            .rounded_t_none()
            .on_click(move |_, _, cx| {
              let group_id = group_id.clone();
              editor_entity.update(cx, |editor, cx| {
                editor.enqueue_group_action(group_id, HunkAction::Unstage, cx);
              });
            }),
        );
      }
    }

    if matches!(state, HunkState::Unstaged) {
      let editor_entity = editor_entity.clone();
      let group_id = group_id.clone();
      actions = actions.child(
        Button::new("restore-hunk")
          .icon(IconName::Undo)
          .label("Restore")
          .rounded_t_none()
          .rounded_bl_none()
          .small()
          .bg(theme.background)
          .tooltip(restore_tooltip)
          .disabled(restore_disabled)
          .on_click(move |_, _, cx| {
            let group_id = group_id.clone();
            editor_entity.update(cx, |editor, cx| {
              editor.enqueue_group_action(group_id, HunkAction::Restore, cx);
            });
          }),
      );
    }

    Some(
      div()
        .absolute()
        .top(top)
        .right(px(30.0))
        .child(actions)
        .into_any_element(),
    )
  }

  fn hunk_action_top(
    line_height: gpui::Pixels,
    display_line: usize,
    scroll_offset: f32,
  ) -> gpui::Pixels {
    line_height * (display_line as f32 - scroll_offset)
  }

  fn render_commit_button(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let repo_ready = self.selected_repo.is_some();
    let commit_message = self.commit_input.read(cx).value();
    let commit_message_ready = !commit_message.trim().is_empty();
    let has_changes = !self.status_entries.is_empty();
    let can_continue_rebase =
      repo_ready && self.rebase_in_progress && !Self::has_conflicted_entries(&self.status_entries);
    let commit_enabled = if self.rebase_in_progress {
      can_continue_rebase
    } else {
      repo_ready && commit_message_ready && has_changes
    };
    let amend_enabled = repo_ready && self.has_head_commit;
    let undo_enabled = repo_ready && self.can_undo_last_commit;
    let push_enabled = repo_ready && self.can_push;
    let force_push_enabled = repo_ready && self.can_force_push;
    let menu_enabled = !self.rebase_in_progress
      && (amend_enabled || undo_enabled || push_enabled || force_push_enabled);
    let view = cx.entity();
    let amend_view = view.clone();
    let undo_view = view.clone();
    let push_view = view.clone();
    let force_push_view = view.clone();
    let push_label = Self::push_action_label(self.branch_status.as_ref(), self.has_head_commit);

    let main_button = if self.rebase_in_progress {
      Button::new("commit-button-main")
        .label("Continue")
        .with_variant(ButtonVariant::Secondary)
        .outline()
        .flex_1()
        .rounded_r_none()
        .child(Kbd::new(Keystroke::parse("cmd-enter").unwrap()).ml_1())
        .disabled(!commit_enabled)
        .on_click(cx.listener(Self::continue_rebase_action))
    } else {
      Button::new("commit-button-main")
        .label("Commit")
        .with_variant(ButtonVariant::Secondary)
        .outline()
        .flex_1()
        .rounded_r_none()
        .child(Kbd::new(Keystroke::parse("cmd-enter").unwrap()).ml_1())
        .disabled(!commit_enabled)
        .on_click(cx.listener(Self::commit_changes))
    };

    let menu_button = Button::new("commit-button-menu")
      .icon(IconName::ChevronDown)
      .with_variant(ButtonVariant::Secondary)
      .outline()
      .rounded_l_none()
      .border_l_0()
      .disabled(!menu_enabled)
      .dropdown_menu_with_anchor(Corner::BottomRight, move |menu, _, _| {
        let amend_view = amend_view.clone();
        let undo_view = undo_view.clone();
        let push_view = push_view.clone();
        let force_push_view = force_push_view.clone();
        let menu = menu.item(
          PopupMenuItem::new("Amend")
            .icon(IconName::Replace)
            .disabled(!amend_enabled)
            .on_click(move |event, window, cx| {
              amend_view.update(cx, |this, cx| {
                let _ = event;
                this.commit_amend_changes(window, cx);
              });
            }),
        );

        let menu = menu.item(
          PopupMenuItem::new("Undo last commit")
            .icon(IconName::Undo)
            .disabled(!undo_enabled)
            .on_click(move |event, window, cx| {
              undo_view.update(cx, |this, cx| {
                let _ = event;
                let _ = window;
                this.undo_last_commit_action(cx);
              });
            }),
        );

        let menu = menu.separator();

        let menu = menu.item(
          PopupMenuItem::new(push_label)
            .icon(IconName::ArrowUp)
            .disabled(!push_enabled)
            .on_click(move |event, window, cx| {
              push_view.update(cx, |this, cx| {
                let _ = event;
                let _ = window;
                this.push_changes_action(cx);
              });
            }),
        );

        menu.item(
          PopupMenuItem::new("Force push (with lease)")
            .icon(IconName::TriangleAlert)
            .disabled(!force_push_enabled)
            .on_click(move |event, window, cx| {
              force_push_view.update(cx, |this, cx| {
                let _ = event;
                let _ = window;
                this.force_push_changes_action(cx);
              });
            }),
        )
      });

    div()
      .flex()
      .w_full()
      .overflow_hidden()
      .child(main_button)
      .child(menu_button)
  }

  fn render_commit_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let input = self.commit_input.clone();
    let has_conflicts = Self::has_conflicted_entries(&self.status_entries);
    let operation_error = self.operation_error.clone();

    div()
      .w_full()
      .flex()
      .flex_col()
      .p_2()
      .gap_2()
      .border_t_1()
      .border_color(theme.border)
      .when(has_conflicts, |this| {
        this.child(
          Alert::warning(
            "commit-conflicts-warning",
            "Resolve all conflicts before committing.",
          )
          .title("Conflicts detected"),
        )
      })
      .when_some(operation_error, |this, error| {
        this.child(Alert::error("commit-operation-error", error.clone()).title("Operation failed"))
      })
      .child(div().w_full().child(Input::new(&input)))
      .child(self.render_commit_button(cx))
  }

  fn render_sidebar_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme();
    let all_staged = self.all_changes_staged();
    let sidebar_enabled = self.selected_repo.is_some() && !self.status_entries.is_empty();
    let restore_all_enabled = sidebar_enabled;
    let merge_abort_enabled = self.selected_repo.is_some() && self.merge_in_progress;
    let rebase_abort_enabled = self.selected_repo.is_some() && self.rebase_in_progress;
    let changed_files_count = Self::changed_files_count(&self.status_entries);
    let (label, icon, tooltip) = if all_staged {
      ("Unstage all", IconName::Minus, "Unstage all files")
    } else {
      ("Stage all", IconName::Plus, "Stage all files")
    };
    let is_history_mode = self.sidebar_mode == GitSidebarMode::History;
    let (mode_label, mode_icon, mode_tooltip) = if is_history_mode {
      ("Changes", UiIconName::FileCode, "Show changes list")
    } else {
      ("History", UiIconName::History, "Show commit history")
    };

    let group_label = if is_history_mode {
      div()
        .text_sm()
        .text_color(theme.sidebar_foreground)
        .child("History")
        .into_any_element()
    } else {
      h_flex()
        .items_center()
        .gap_2()
        .child(
          div()
            .text_sm()
            .text_color(theme.sidebar_foreground)
            .child("Changes"),
        )
        .when(
          Self::should_show_changed_files_tag(changed_files_count),
          |this| {
            this.child(
              Tag::secondary()
                .small()
                .rounded_full()
                .child(changed_files_count.to_string()),
            )
          },
        )
        .into_any_element()
    };

    div()
      .w_full()
      .flex()
      .px_3()
      .min_h(px(EDITOR_HEADER_HEIGHT))
      .border_b_1()
      .border_color(cx.theme().border)
      .items_center()
      .justify_between()
      .child(group_label)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .when(self.merge_in_progress, |this| {
            this.child(
              Button::new("abort-merge-button")
                .label("Abort merge")
                .icon(IconName::Undo)
                .with_variant(ButtonVariant::Secondary)
                .xsmall()
                .disabled(!merge_abort_enabled)
                .tooltip("Abort current merge")
                .on_click(cx.listener(Self::abort_merge_action)),
            )
          })
          .when(self.rebase_in_progress, |this| {
            this.child(
              Button::new("abort-rebase-button")
                .label("Abort rebase")
                .icon(IconName::Undo)
                .with_variant(ButtonVariant::Secondary)
                .xsmall()
                .disabled(!rebase_abort_enabled)
                .tooltip("Abort current rebase")
                .on_click(cx.listener(Self::abort_rebase_action)),
            )
          })
          .when(!is_history_mode, |this| {
            this
              .child(
                Button::new("stage-all-button")
                  .label(label)
                  .icon(icon)
                  .with_variant(ButtonVariant::Secondary)
                  .xsmall()
                  .disabled(!sidebar_enabled)
                  .tooltip(tooltip)
                  .on_click(cx.listener(Self::toggle_stage_all_action)),
              )
              .child(
                Button::new("restore-all-button")
                  .label("Restore all")
                  .icon(IconName::Undo)
                  .with_variant(ButtonVariant::Secondary)
                  .xsmall()
                  .disabled(!restore_all_enabled)
                  .tooltip("Discard all changes")
                  .on_click(cx.listener(Self::restore_all_click_action)),
              )
          })
          .child(
            Button::new("sidebar-mode-toggle-button")
              .label(mode_label)
              .icon(mode_icon)
              .with_variant(ButtonVariant::Secondary)
              .xsmall()
              .selected(is_history_mode)
              .disabled(self.selected_repo.is_none())
              .tooltip(mode_tooltip)
              .on_click(cx.listener(Self::toggle_sidebar_mode_action)),
          ),
      )
  }

  fn render_sidebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let base_sidebar = div()
      .id("git-sidebar")
      .w_full()
      .h_full()
      .flex()
      .flex_col()
      .bg(theme.sidebar)
      .text_color(theme.sidebar_foreground);

    if self.selected_repo.is_none() {
      return base_sidebar
        .child(
          div()
            .p_4()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Select a repository"),
        )
        .into_any_element();
    }

    if self.sidebar_mode == GitSidebarMode::History {
      return base_sidebar
        .relative()
        .child(self.render_sidebar_header(cx))
        .child(self.render_history_sidebar_content(_window, cx))
        .into_any_element();
    }

    let list_container = div()
      .id("git-sidebar-file-list-container")
      .relative()
      .flex_1()
      .min_h_0()
      .overflow_hidden()
      .child(
        List::new(&self.file_list)
          .flex_1()
          .w_full()
          .min_h_0()
          .p(px(6.)),
      );

    base_sidebar
      .relative()
      .child(self.render_sidebar_header(cx))
      .child(
        div()
          .flex()
          .flex_col()
          .flex_1()
          .min_h_0()
          .child(list_container),
      )
      .child(self.render_commit_bar(cx))
      .into_any_element()
  }

  fn render_editor_area(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    if self.selected_repo.is_none() {
      return self.render_empty_state("Select a repository to view changes", cx);
    }

    let theme = cx.theme().clone();
    if let Some(editor) = self.editor.clone() {
      let editor_view = self.render_editor_with_overlay(editor.clone(), window, cx);
      if self.show_markdown_preview
        && (self.selected_file_is_markdown() || self.selected_file_is_svg())
      {
        let preview_content = if self.selected_file_is_svg() {
          self.update_svg_preview(window, cx);
          let preview = match self.svg_preview.clone() {
            Some(Ok(image)) => img(image).max_w_full().max_h_full().into_any_element(),
            Some(Err(error)) => div()
              .text_sm()
              .text_color(theme.status_red())
              .child(error)
              .into_any_element(),
            None => div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child("Rendering SVG preview...")
              .into_any_element(),
          };
          div()
            .flex_1()
            .min_h_0()
            .min_w(px(0.0))
            .bg(theme.background)
            .occlude()
            .child(
              div()
                .flex_1()
                .min_h_0()
                .min_w(px(0.0))
                .p_4()
                .items_center()
                .justify_center()
                .child(preview),
            )
            .into_any_element()
        } else {
          let markdown = editor.read(cx).document().read(cx);
          let markdown = markdown.slice_to_string(0..markdown.len());
          div()
            .flex_1()
            .min_h_0()
            .min_w(px(0.0))
            .bg(theme.background)
            .occlude()
            .child(
              div().size_full().pb_4().px_4().child(
                TextView::markdown("git-markdown-preview-text", markdown)
                  .size_full()
                  .selectable(true)
                  .scrollable(true),
              ),
            )
            .into_any_element()
        };

        return div()
          .size_full()
          .flex()
          .flex_col()
          .child(self.render_editor_header(&editor, cx))
          .child(
            ui::h_resizable("git-page-markdown-preview")
              .child(ui::resizable_panel().child(editor_view))
              .child(ui::resizable_panel().child(preview_content)),
          )
          .into_any_element();
      }

      return div()
        .size_full()
        .flex()
        .flex_col()
        .child(self.render_editor_header(&editor, cx))
        .child(editor_view)
        .into_any_element();
    }

    if Self::should_show_editor_loading_state(self.selected_file.as_deref(), self.editor.is_some())
    {
      return self.render_loading_state("Loading file...", cx);
    }

    self.render_empty_state("Select a file to view diff", cx)
  }
}

impl Render for GitPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(cx.theme().background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GitPage::show_command_palette_action))
      .on_action(cx.listener(GitPage::show_file_search_action))
      .on_action(cx.listener(GitPage::find_action))
      .on_action(cx.listener(GitPage::close_find_action))
      .on_action(cx.listener(GitPage::open_repository_action))
      .on_action(cx.listener(GitPage::commit_changes_action))
      .child(self.render_header(cx))
      .child(
        ui::h_resizable("git-page-split")
          .child(
            ui::resizable_panel()
              .size(px(SIDEBAR_DEFAULT_WIDTH))
              .size_range(px(SIDEBAR_MIN_WIDTH)..px(SIDEBAR_MAX_WIDTH))
              .child(self.render_sidebar(window, cx)),
          )
          .child(ui::resizable_panel().child(self.render_editor_area(window, cx))),
      )
  }
}

impl Focusable for GitPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use git2::build::CheckoutBuilder;
  use git2::{BranchType, Cred, PushOptions, RemoteCallbacks, Repository, Signature};
  use gpui::TestAppContext;
  use std::sync::atomic::{AtomicU64, Ordering};
  use std::time::{SystemTime, UNIX_EPOCH};

  #[test]
  fn format_git_file_label_supports_full_path_and_file_name() {
    let path = Path::new("src/features/renamed_file.rs");

    assert_eq!(
      format_git_file_label(path, GitFileLabelFormat::FullPath).as_ref(),
      "src/features/renamed_file.rs"
    );
    assert_eq!(
      format_git_file_label(path, GitFileLabelFormat::FileName).as_ref(),
      "renamed_file.rs"
    );
  }

  #[test]
  fn format_git_file_label_strips_newlines() {
    let path = Path::new("src/renamed\n_file.rs");

    assert_eq!(
      format_git_file_label(path, GitFileLabelFormat::FullPath).as_ref(),
      "src/renamed_file.rs"
    );
    assert_eq!(
      format_git_file_label(path, GitFileLabelFormat::FileName).as_ref(),
      "renamed_file.rs"
    );
  }

  struct TempRepo {
    path: PathBuf,
  }

  impl TempRepo {
    fn init(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!("reviu-{prefix}-{}-{nanos}", std::process::id()));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Repository::init(&path).expect("init git repository");
      Self { path }
    }
  }

  impl Drop for TempRepo {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  struct TempBareRepo {
    path: PathBuf,
  }

  impl TempBareRepo {
    fn init(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!(
        "reviu-{prefix}-bare-{}-{nanos}",
        std::process::id()
      ));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Repository::init_bare(&path).expect("init bare git repository");
      Self { path }
    }
  }

  impl Drop for TempBareRepo {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  struct TempDir {
    path: PathBuf,
  }

  impl TempDir {
    fn new(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!("reviu-{prefix}-dir-{}-{nanos}", std::process::id()));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Self { path }
    }
  }

  impl Drop for TempDir {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  fn commit_text_file(
    repo_root: &Path,
    rel_path: &Path,
    contents: &str,
    message: &str,
  ) -> git2::Oid {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::write(repo_root.join(rel_path), contents).expect("write worktree file");

    let mut index = repo.index().expect("open index");
    index.add_path(rel_path).expect("stage file");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("find tree");
    let signature = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());

    match parent {
      Some(parent) => repo
        .commit(
          Some("HEAD"),
          &signature,
          &signature,
          message,
          &tree,
          &[&parent],
        )
        .expect("commit with parent"),
      None => repo
        .commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
        .expect("initial commit"),
    }
  }

  fn push_branch_to_remote(repo_root: &Path, branch_name: &str, remote_name: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    let mut remote = repo.find_remote(remote_name).expect("find remote");
    let refspec = format!("refs/heads/{branch_name}:refs/heads/{branch_name}");
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_, _, _| Cred::default());
    let mut options = PushOptions::new();
    options.remote_callbacks(callbacks);
    remote
      .push(&[refspec], Some(&mut options))
      .expect("push branch");
  }

  fn set_upstream(repo_root: &Path, local_branch: &str, upstream_branch: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    let mut branch = repo
      .find_branch(local_branch, BranchType::Local)
      .expect("find local branch");
    branch
      .set_upstream(Some(upstream_branch))
      .expect("set upstream");
  }

  fn set_remote_head(remote_root: &Path, branch_name: &str) {
    let refname = format!("refs/heads/{branch_name}");
    Repository::open(remote_root)
      .expect("open remote")
      .set_head(&refname)
      .expect("set remote HEAD");
  }

  fn head_oid(repo_root: &Path) -> git2::Oid {
    Repository::open(repo_root)
      .expect("open repo")
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head")
      .id()
  }

  fn remote_branch_oid(remote_root: &Path, branch_name: &str) -> git2::Oid {
    let refname = format!("refs/heads/{branch_name}");
    Repository::open(remote_root)
      .expect("open remote")
      .refname_to_id(&refname)
      .expect("read remote branch oid")
  }

  fn force_checkout_head(repo_root: &Path) {
    let repo = Repository::open(repo_root).expect("open repo");
    let mut checkout = CheckoutBuilder::new();
    checkout.force();
    repo
      .checkout_head(Some(&mut checkout))
      .expect("force checkout HEAD");
  }

  fn make_commit(oid: &str, parents: &[&str]) -> HistoryCommitNode {
    HistoryCommitNode {
      oid: oid.to_string(),
      short_oid: oid.chars().take(7).collect(),
      summary: format!("commit-{oid}"),
      author: "author".to_string(),
      parent_oids: parents.iter().map(|parent| parent.to_string()).collect(),
      refs: Vec::new(),
    }
  }

  fn make_history_file(path: &str, kind: CommitFileChangeKind) -> HistoryCommitFileRow {
    HistoryCommitFileRow::from_commit_file(CommitChangedFile {
      path: PathBuf::from(path),
      old_path: None,
      kind,
    })
  }

  fn make_history_revision(tag: &str) -> HistoryRevision {
    HistoryRevision {
      head_oid: Some(format!("head-{tag}")),
      head_label: Some(format!("HEAD -> {tag}")),
      refs: vec![format!("{tag}@oid-{tag}")],
    }
  }

  fn make_branch_status(
    name: &str,
    ahead: usize,
    behind: usize,
    has_upstream: bool,
  ) -> BranchStatus {
    BranchStatus {
      name: name.to_string(),
      ahead,
      behind,
      has_upstream,
    }
  }

  fn isolate_config_store_for_test() {
    static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
      "reviu-git-page-test-config-{}-{id}.sqlite",
      std::process::id()
    ));
    let _ = std::fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));
  }

  fn init_gpui_test(cx: &mut TestAppContext) {
    isolate_config_store_for_test();
    cx.update(|cx| {
      gpui_component::init(cx);
    });
  }

  fn seed_repo_branch_state(this: &mut GitPage, repo_root: &Path, cx: &mut Context<GitPage>) {
    this.selected_repo = Some(repo_root.to_path_buf());
    let branch_status = current_branch_status(repo_root).expect("read initial branch status");
    let selected = GitPage::selected_branch_from_status(Some(branch_status.clone()));
    let items = GitPage::branch_select_items(
      list_branches(repo_root).expect("list branches"),
      selected.as_ref(),
    );
    this.branch_status = Some(branch_status);

    let branch_select = this.branch_select.clone();
    let window_handle = this.window_handle;
    let _ = cx.update_window(window_handle, move |_, window, cx| {
      branch_select.update(cx, |state, cx| {
        state.set_items(SearchableVec::new(items), window, cx);
        if let Some(selected) = selected.as_ref() {
          state.set_selected_value(selected, window, cx);
        } else {
          state.set_selected_index(None, window, cx);
        }
      });
    });
  }

  async fn await_git_page_background_tasks(
    git_page: Entity<GitPage>,
    cx: &mut gpui::VisualTestContext,
  ) {
    loop {
      let (
        open_file_task,
        status_task,
        branch_task,
        history_task,
        history_files_task,
        history_open_file_task,
      ) = git_page.update_in(cx, |this, _window, _| {
        (
          this.open_file_task.take(),
          this.status_task.take(),
          this.branch_task.take(),
          this.history_task.take(),
          this.history_files_task.take(),
          this.history_open_file_task.take(),
        )
      });

      let mut had_task = false;
      if let Some(task) = open_file_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = status_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = branch_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = history_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = history_files_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = history_open_file_task {
        had_task = true;
        task.await;
      }

      if !had_task {
        break;
      }
    }
  }

  #[test]
  fn build_history_tree_items_marks_selected_commit_expanded() {
    let commits = vec![
      make_commit("c3", &["c2"]),
      make_commit("c2", &["c1"]),
      make_commit("c1", &[]),
    ];
    let rows = GitPage::build_history_rows(&commits);

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
    let (items, _) =
      GitPage::build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);

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
    let rows = GitPage::build_history_rows(&commits);
    let files_by_commit = HashMap::new();
    let loading = HashSet::new();
    let expanded = HashSet::from(["c3".to_string(), "c1".to_string()]);

    let (items, _) =
      GitPage::build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);

    assert!(items[0].is_expanded());
    assert!(!items[1].is_expanded());
    assert!(items[2].is_expanded());
  }

  #[test]
  fn build_history_tree_items_includes_commit_and_file_nodes() {
    let commits = vec![make_commit("c2", &["c1"]), make_commit("c1", &[])];
    let rows = GitPage::build_history_rows(&commits);
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

    let (items, nodes) =
      GitPage::build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);
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
    let rows = GitPage::build_history_rows(&commits);
    let files_by_commit = HashMap::new();
    let loading = HashSet::from(["c1".to_string()]);
    let expanded = HashSet::from(["c1".to_string()]);

    let (items, nodes) =
      GitPage::build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);
    assert_eq!(items[0].children.len(), 1);
    assert_eq!(items[0].children[0].label.as_ref(), "Loading files...");
    assert!(matches!(
      nodes.get("history-loading:c1"),
      Some(HistoryTreeNode::Placeholder)
    ));
  }

  #[test]
  fn should_refresh_file_list_only_in_changes_mode() {
    assert!(GitPage::should_refresh_file_list(GitSidebarMode::Changes));
    assert!(!GitPage::should_refresh_file_list(GitSidebarMode::History));
  }

  #[test]
  fn should_refresh_history_for_poll_when_history_empty() {
    assert!(GitPage::should_refresh_history_for_poll(
      true,
      true,
      Some(&make_history_revision("a")),
      Some(&make_history_revision("a"))
    ));
  }

  #[test]
  fn should_not_refresh_history_for_poll_when_revision_unchanged() {
    let revision = make_history_revision("a");
    assert!(!GitPage::should_refresh_history_for_poll(
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
    assert!(GitPage::should_refresh_history_for_poll(
      true,
      false,
      Some(&cached),
      Some(&current)
    ));
  }

  #[test]
  fn should_not_refresh_history_for_poll_when_history_not_included() {
    assert!(!GitPage::should_refresh_history_for_poll(
      false,
      true,
      Some(&make_history_revision("a")),
      Some(&make_history_revision("b"))
    ));
  }

  #[test]
  fn should_not_refresh_history_for_poll_when_revision_unavailable() {
    assert!(!GitPage::should_refresh_history_for_poll(
      true,
      false,
      Some(&make_history_revision("a")),
      None
    ));
  }

  #[test]
  fn branch_name_changed_detects_name_transitions() {
    let main = make_branch_status("main", 0, 0, true);
    let feature = make_branch_status("feature", 0, 0, true);

    assert!(GitPage::branch_name_changed(None, Some(&main)));
    assert!(GitPage::branch_name_changed(Some(&main), Some(&feature)));
    assert!(GitPage::branch_name_changed(Some(&main), None));
    assert!(!GitPage::branch_name_changed(None, None));
    assert!(!GitPage::branch_name_changed(Some(&main), Some(&main)));
  }

  #[test]
  fn push_flags_respect_upstream_and_divergence() {
    let no_upstream = make_branch_status("main", 3, 0, false);
    assert_eq!(
      GitPage::push_flags(Some(&no_upstream), false, false),
      (false, false)
    );
    assert_eq!(
      GitPage::push_flags(Some(&no_upstream), true, false),
      (true, false)
    );

    let clean_ahead = make_branch_status("main", 2, 0, true);
    assert_eq!(
      GitPage::push_flags(Some(&clean_ahead), true, false),
      (true, false)
    );

    let diverged = make_branch_status("main", 1, 2, true);
    assert_eq!(
      GitPage::push_flags(Some(&diverged), true, false),
      (false, true)
    );

    let behind_only = make_branch_status("main", 0, 2, true);
    assert_eq!(
      GitPage::push_flags(Some(&behind_only), true, false),
      (false, false)
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
  fn push_action_label_mentions_publish_branch_without_upstream() {
    let no_upstream = make_branch_status("feature", 0, 0, false);
    assert_eq!(
      GitPage::push_action_label(Some(&no_upstream), true),
      "Push (Publish branch)"
    );
    assert_eq!(
      GitPage::push_action_label(Some(&no_upstream), false),
      "Push"
    );

    let tracked = make_branch_status("main", 1, 0, true);
    assert_eq!(GitPage::push_action_label(Some(&tracked), true), "Push");
    let detached = make_branch_status("HEAD", 0, 0, false);
    assert_eq!(GitPage::push_action_label(Some(&detached), true), "Push");
    assert_eq!(GitPage::push_action_label(None, true), "Push");
  }

  fn make_status_entry(path: &str, stage: RepoStage) -> RepoStatusEntry {
    RepoStatusEntry {
      path: PathBuf::from(path),
      old_path: None,
      status: RepoStatusKind::Modified,
      stage,
    }
  }

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
  fn changed_files_count_matches_status_entries_len() {
    assert_eq!(GitPage::changed_files_count(&[]), 0);

    let entries = vec![
      make_status_entry("src/a.rs", RepoStage::Unstaged),
      make_status_entry("src/b.rs", RepoStage::Staged),
      make_status_entry("src/c.rs", RepoStage::PartiallyStaged),
    ];
    assert_eq!(GitPage::changed_files_count(&entries), 3);
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

  #[test]
  fn has_tracked_entries_excludes_untracked_only_state() {
    let untracked_entries = vec![RepoStatusEntry {
      path: PathBuf::from("notes.txt"),
      old_path: None,
      status: RepoStatusKind::Untracked,
      stage: RepoStage::Unstaged,
    }];
    let tracked_entries = vec![
      RepoStatusEntry {
        path: PathBuf::from("notes.txt"),
        old_path: None,
        status: RepoStatusKind::Untracked,
        stage: RepoStage::Unstaged,
      },
      make_status_entry("src/main.rs", RepoStage::Unstaged),
    ];

    assert!(!GitPage::has_tracked_entries(&untracked_entries));
    assert!(GitPage::has_tracked_entries(&tracked_entries));
  }

  #[test]
  fn stash_command_flags_follow_untracked_only_rule() {
    let untracked_entries = vec![RepoStatusEntry {
      path: PathBuf::from("notes.txt"),
      old_path: None,
      status: RepoStatusKind::Untracked,
      stage: RepoStage::Unstaged,
    }];
    let tracked_entries = vec![make_status_entry("src/main.rs", RepoStage::Unstaged)];

    assert_eq!(GitPage::stash_command_flags(&[]), (false, false));
    assert_eq!(
      GitPage::stash_command_flags(&untracked_entries),
      (false, true)
    );
    assert_eq!(GitPage::stash_command_flags(&tracked_entries), (true, true));
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

  #[test]
  fn changed_files_tag_visibility_requires_positive_count() {
    assert!(!GitPage::should_show_changed_files_tag(0));
    assert!(GitPage::should_show_changed_files_tag(1));
    assert!(GitPage::should_show_changed_files_tag(42));
  }

  #[gpui::test]
  fn focus_changes_sidebar_list_selects_first_entry_when_unselected(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));

    git_page.update_in(cx, |this, window, cx| {
      this.status_entries = vec![
        make_status_entry("src/main.rs", RepoStage::Unstaged),
        make_status_entry("src/lib.rs", RepoStage::Unstaged),
      ];
      this.refresh_file_list(cx);
      assert_eq!(this.file_list.read(cx).selected_index(), None);

      this.focus_changes_sidebar_list(window, cx);
      assert_eq!(
        this.file_list.read(cx).selected_index(),
        Some(IndexPath::new(0))
      );
    });
  }

  #[gpui::test]
  fn set_sidebar_mode_history_keeps_page_focus_for_shortcuts(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));

    git_page.update_in(cx, |this, window, cx| {
      this.set_sidebar_mode(GitSidebarMode::History, window, cx);
      assert!(this.focus_handle.contains_focused(window, cx));
    });
  }

  #[test]
  fn selected_file_update_clears_missing_selection_without_history_file() {
    let update = GitPage::selected_file_update(
      Some(Path::new("src/missing.rs")),
      &[make_status_entry("src/exists.rs", RepoStage::Unstaged)],
      false,
      true,
    );
    assert_eq!(
      update,
      SelectedFileUpdate {
        clear_selection: true,
        sync_diff_view: false,
      }
    );
  }

  #[test]
  fn selected_file_update_keeps_selection_and_syncs_when_present() {
    let update = GitPage::selected_file_update(
      Some(Path::new("src/main.rs")),
      &[make_status_entry("src/main.rs", RepoStage::Unstaged)],
      false,
      true,
    );
    assert_eq!(
      update,
      SelectedFileUpdate {
        clear_selection: false,
        sync_diff_view: true,
      }
    );
  }

  #[test]
  fn selected_file_update_never_clears_when_history_file_is_open() {
    let update = GitPage::selected_file_update(
      Some(Path::new("src/main.rs")),
      &[make_status_entry("src/other.rs", RepoStage::Unstaged)],
      true,
      true,
    );
    assert_eq!(update, SelectedFileUpdate::default());
  }

  #[test]
  fn selected_file_update_is_noop_without_selection() {
    let update = GitPage::selected_file_update(
      None,
      &[make_status_entry("src/main.rs", RepoStage::Unstaged)],
      false,
      true,
    );
    assert_eq!(update, SelectedFileUpdate::default());
  }

  #[test]
  fn selected_branch_from_status_maps_detached_head_to_none() {
    let detached = make_branch_status("HEAD", 0, 0, false);
    assert_eq!(GitPage::selected_branch_from_status(Some(detached)), None);
  }

  #[test]
  fn selected_branch_from_status_maps_named_head_to_local_branch() {
    let main = make_branch_status("main", 0, 0, true);
    assert_eq!(
      GitPage::selected_branch_from_status(Some(main)),
      Some(BranchRef {
        name: "main".to_string(),
        kind: BranchKind::Local,
      })
    );
  }

  #[test]
  fn branch_select_items_marks_only_selected_branch() {
    let selected = BranchRef {
      name: "main".to_string(),
      kind: BranchKind::Local,
    };
    let items = GitPage::branch_select_items(
      vec![
        BranchRef {
          name: "main".to_string(),
          kind: BranchKind::Local,
        },
        BranchRef {
          name: "feature".to_string(),
          kind: BranchKind::Local,
        },
      ],
      Some(&selected),
    );

    assert_eq!(items.len(), 2);
    assert!(items[0].is_current);
    assert!(!items[1].is_current);
  }

  #[gpui::test]
  async fn command_palette_create_branch_creates_and_switches_to_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-branch");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranch {
          name: "feature".to_string(),
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&repo.path).expect("read status");
    assert_eq!(status.name, "feature");
    assert!(
      list_branches(&repo.path)
        .expect("list branches")
        .iter()
        .any(|branch| branch.kind == BranchKind::Local && branch.name == "feature")
    );

    let selected_branch = git_page.read_with(cx, |this, cx| {
      this.branch_select.read(cx).selected_value().cloned()
    });
    assert_eq!(
      selected_branch,
      Some(BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      })
    );
  }

  #[gpui::test]
  async fn command_palette_create_branch_returns_error_when_branch_exists(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-branch-existing");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create existing target branch");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranch {
          name: "feature".to_string(),
        },
        _window,
        cx,
      )
    });

    let error = result.expect_err("create branch should fail when branch already exists");
    assert!(error.as_ref().starts_with("Action failed:"));
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed create")
        .name,
      base_branch
    );
    let feature_count = list_branches(&repo.path)
      .expect("list branches after failed create")
      .iter()
      .filter(|branch| branch.kind == BranchKind::Local && branch.name == "feature")
      .count();
    assert_eq!(feature_count, 1);
  }

  #[gpui::test]
  async fn command_palette_switch_branch_switches_to_requested_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-switch-branch");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&repo.path).expect("read status");
    assert_eq!(status.name, "feature");
  }

  #[gpui::test]
  async fn command_palette_switch_repository_updates_selected_repo_and_header_select(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo_a = TempRepo::init("git-page-cmd-switch-repo-a");
    let repo_b = TempRepo::init("git-page-cmd-switch-repo-b");
    let _ = commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    let _ = commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    ConfigStore::persist_recent_repository(&repo_a.path);
    ConfigStore::persist_recent_repository(&repo_b.path);

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo_a.path.clone());
      this.refresh_repo_select(cx);
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchRepository(CommandPaletteRepository {
          path: repo_b.path.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, selected_in_header) = git_page.read_with(cx, |this, cx| {
      (
        this.selected_repo.clone(),
        this.repo_select.read(cx).selected_value().cloned(),
      )
    });
    assert_eq!(selected_repo, Some(repo_b.path.clone()));
    assert_eq!(selected_in_header, Some(repo_b.path.clone()));
  }

  #[gpui::test]
  async fn command_palette_switch_repository_returns_error_for_missing_repository(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-switch-repo-missing");
    let missing_repo = repo.path.join("does-not-exist");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchRepository(CommandPaletteRepository {
          path: missing_repo.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });

    let error = result.expect_err("switch repository should fail for a missing path");
    assert!(error.as_ref().starts_with("Repository not found:"));
    let selected_repo = git_page.read_with(cx, |this, _| this.selected_repo.clone());
    assert_eq!(selected_repo, Some(repo.path.clone()));
  }

  #[gpui::test]
  async fn command_palette_fetch_updates_remote_tracking_refs(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-fetch-origin");
    let source = TempRepo::init("git-page-cmd-fetch-source");
    let clone_dir = TempDir::new("git-page-cmd-fetch-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let tracking_ref = format!("refs/remotes/origin/{base_branch}");
    let before = Repository::open(&clone_dir.path)
      .expect("open clone")
      .refname_to_id(&tracking_ref)
      .expect("read remote tracking ref before fetch");

    let _ = commit_text_file(
      &source.path,
      Path::new("README.md"),
      "v2\n",
      "source update",
    );
    push_branch_to_remote(&source.path, &base_branch, "origin");
    let expected = remote_branch_oid(&remote.path, &base_branch);
    assert_ne!(
      before, expected,
      "expected remote branch to advance after push"
    );

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone");
    clone_repo
      .reference(
        &tracking_ref,
        before,
        true,
        "force stale remote tracking ref",
      )
      .expect("force stale remote tracking ref");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(CommandPaletteAction::Fetch, window, cx)
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let after = Repository::open(&clone_dir.path)
      .expect("open clone")
      .refname_to_id(&tracking_ref)
      .expect("read remote tracking ref after fetch");
    assert_eq!(after, expected);
  }

  #[gpui::test]
  async fn command_palette_fetch_toggles_loading_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-fetch-loading-origin");
    let source = TempRepo::init("git-page-cmd-fetch-loading-source");
    let clone_dir = TempDir::new("git-page-cmd-fetch-loading-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let _ = commit_text_file(
      &source.path,
      Path::new("README.md"),
      "v2\n",
      "source update",
    );
    push_branch_to_remote(&source.path, &base_branch, "origin");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(CommandPaletteAction::Fetch, window, cx)
    });
    assert!(result.is_ok());
    assert!(git_page.read_with(cx, |this, _| this.fetch_in_progress));

    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(!git_page.read_with(cx, |this, _| this.fetch_in_progress));
  }

  #[gpui::test]
  async fn command_palette_create_branch_from_local_creates_and_switches(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-from");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_head = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranchFrom {
          name: "feature-copy".to_string(),
          base: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&repo.path).expect("read status");
    assert_eq!(status.name, "feature-copy");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let created = repo_handle
      .find_branch("feature-copy", BranchType::Local)
      .expect("find feature-copy branch");
    assert_eq!(created.get().target(), Some(feature_head));
    assert!(created.upstream().is_err());
  }

  #[gpui::test]
  async fn command_palette_create_branch_from_returns_error_when_branch_exists(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-from-existing");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    create_branch(&repo.path, "feature-copy").expect("create existing target branch");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranchFrom {
          name: "feature-copy".to_string(),
          base: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });

    let error = result.expect_err("create branch from should fail when branch already exists");
    assert!(error.as_ref().starts_with("Action failed:"));
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed create from")
        .name,
      base_branch
    );
    let feature_copy_count = list_branches(&repo.path)
      .expect("list branches after failed create from")
      .iter()
      .filter(|branch| branch.kind == BranchKind::Local && branch.name == "feature-copy")
      .count();
    assert_eq!(feature_copy_count, 1);
  }

  #[gpui::test]
  async fn command_palette_switch_remote_branch_creates_local_branch_with_upstream(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-switch-remote-origin");
    let source = TempRepo::init("git-page-cmd-switch-remote-source");
    let clone_dir = TempDir::new("git-page-cmd-switch-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");

    let base_branch = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");
    set_remote_head(&remote.path, &base_branch);

    create_branch(&source.path, "feature").expect("create source feature branch");
    switch_branch(
      &source.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch source to feature");
    let _ = commit_text_file(
      &source.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
          name: "origin/feature".into(),
          kind: CommandPaletteBranchKind::Remote,
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&clone_dir.path).expect("status after remote switch");
    assert_eq!(status.name, "feature");
    assert!(status.has_upstream);

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone repo");
    let local_feature = clone_repo
      .find_branch("feature", BranchType::Local)
      .expect("find local feature branch");
    let upstream = local_feature
      .upstream()
      .expect("feature upstream")
      .name()
      .expect("upstream name")
      .expect("non-empty upstream")
      .to_string();
    assert_eq!(upstream, "origin/feature");
  }

  #[gpui::test]
  async fn command_palette_switch_remote_branch_returns_error_when_remote_branch_missing(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-switch-remote-missing");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
          name: "origin/missing".into(),
          kind: CommandPaletteBranchKind::Remote,
        }),
        _window,
        cx,
      )
    });

    let error = result.expect_err("switch remote should fail when remote branch is missing");
    assert!(error.as_ref().starts_with("Action failed:"));
    assert!(error.as_ref().contains("origin/missing"));
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed remote switch")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn command_palette_create_branch_from_remote_creates_branch_with_upstream(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-create-from-remote-origin");
    let source = TempRepo::init("git-page-cmd-create-from-remote-source");
    let clone_dir = TempDir::new("git-page-cmd-create-from-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");

    let base_branch = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");
    set_remote_head(&remote.path, &base_branch);

    create_branch(&source.path, "feature").expect("create source feature branch");
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranchFrom {
          name: "my-feature".to_string(),
          base: CommandPaletteBranch {
            name: "origin/feature".into(),
            kind: CommandPaletteBranchKind::Remote,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&clone_dir.path).expect("status after create from remote");
    assert_eq!(status.name, "my-feature");
    assert!(status.has_upstream);

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone repo");
    let created = clone_repo
      .find_branch("my-feature", BranchType::Local)
      .expect("find created branch");
    let upstream = created
      .upstream()
      .expect("created branch upstream")
      .name()
      .expect("upstream name")
      .expect("non-empty upstream")
      .to_string();
    assert_eq!(upstream, "origin/feature");
  }

  #[gpui::test]
  async fn command_palette_create_branch_from_remote_returns_error_when_base_missing(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-from-remote-missing");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranchFrom {
          name: "my-feature".to_string(),
          base: CommandPaletteBranch {
            name: "origin/missing".into(),
            kind: CommandPaletteBranchKind::Remote,
          },
        },
        _window,
        cx,
      )
    });

    let error = result.expect_err("create from remote should fail when base is missing");
    assert!(error.as_ref().starts_with("Action failed:"));
    assert!(error.as_ref().contains("origin/missing"));
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed create from remote")
        .name,
      base_branch
    );
    assert!(
      !list_branches(&repo.path)
        .expect("list branches")
        .iter()
        .any(|branch| branch.kind == BranchKind::Local && branch.name == "my-feature")
    );
  }

  #[gpui::test]
  async fn command_palette_merge_branch_fast_forwards_current_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-merge");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_head = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::MergeBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.force_push_after_rebase));

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.id(), feature_head);
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read merged file"),
      "v2-feature\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after merge")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn command_palette_rebase_branch_fast_forwards_current_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-rebase");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_head = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::RebaseBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.id(), feature_head);
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read rebased file"),
      "v2-feature\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after rebase")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn command_palette_cherry_pick_applies_multiple_commits(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-cherry-pick");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");

    let first = commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "feature 1");
    let second = commit_text_file(&repo.path, Path::new("extra.txt"), "extra\n", "feature 2");

    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CherryPick {
          commit_hashes: vec![first.to_string(), second.to_string()],
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.message().unwrap_or_default(), "feature 2");
    let parent = head.parent(0).expect("head parent");
    assert_eq!(parent.message().unwrap_or_default(), "feature 1");
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read cherry-picked README"),
      "v2\n"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("extra.txt")).expect("read cherry-picked extra file"),
      "extra\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after cherry-pick")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn command_palette_cherry_pick_returns_error_for_invalid_commit(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-cherry-pick-invalid");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CherryPick {
          commit_hashes: vec!["deadbeef".to_string()],
        },
        _window,
        cx,
      )
    });

    let error = result.expect_err("cherry-pick should fail for missing commit");
    assert!(error.as_ref().starts_with("Action failed:"));
    assert!(error.as_ref().contains("resolve commit"));
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed cherry-pick")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn command_palette_stash_and_apply_restore_tracked_changes(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-stash-apply");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write tracked change");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    let stash_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::Stash {
          include_untracked: false,
          message: None,
        },
        window,
        cx,
      )
    });
    assert!(stash_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after stash"),
      "v1\n"
    );
    let stash = list_stashes(&repo.path)
      .expect("list stashes after stash")
      .into_iter()
      .next()
      .expect("stash entry exists");

    let apply_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::ApplyStash(CommandPaletteStash {
          index: stash.index,
          name: stash.name.clone().into(),
          oid: stash.oid.clone().into(),
        }),
        window,
        cx,
      )
    });
    assert!(apply_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after apply stash"),
      "v2\n"
    );
    assert_eq!(
      list_stashes(&repo.path)
        .expect("list stashes after apply")
        .len(),
      1
    );
  }

  #[gpui::test]
  async fn command_palette_stash_with_untracked_and_pop_restores_untracked_file(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-stash-pop-untracked");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let rel_path = Path::new("notes.txt");
    std::fs::write(repo.path.join(rel_path), "notes\n").expect("write untracked file");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    let stash_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::Stash {
          include_untracked: true,
          message: None,
        },
        window,
        cx,
      )
    });
    assert!(stash_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !repo.path.join(rel_path).exists(),
      "untracked file should be removed after stash"
    );
    let stash = list_stashes(&repo.path)
      .expect("list stashes")
      .into_iter()
      .next()
      .expect("stash entry exists");

    let pop_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::PopStash(CommandPaletteStash {
          index: stash.index,
          name: stash.name.clone().into(),
          oid: stash.oid.clone().into(),
        }),
        window,
        cx,
      )
    });
    assert!(pop_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read restored untracked file"),
      "notes\n"
    );
    assert!(
      list_stashes(&repo.path)
        .expect("list stashes after pop")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn command_palette_drop_stash_removes_entry(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-stash-drop");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write tracked change");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    let stash_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::Stash {
          include_untracked: false,
          message: None,
        },
        window,
        cx,
      )
    });
    assert!(stash_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let stash = list_stashes(&repo.path)
      .expect("list stashes")
      .into_iter()
      .next()
      .expect("stash entry exists");

    let drop_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::DropStash(CommandPaletteStash {
          index: stash.index,
          name: stash.name.clone().into(),
          oid: stash.oid.clone().into(),
        }),
        window,
        cx,
      )
    });
    assert!(drop_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      list_stashes(&repo.path)
        .expect("list stashes after drop")
        .is_empty()
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after drop stash"),
      "v1\n"
    );
  }

  #[gpui::test]
  async fn command_palette_branch_actions_require_selected_repo(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));

    let actions = vec![
      CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
        name: "feature".into(),
        kind: CommandPaletteBranchKind::Local,
      }),
      CommandPaletteAction::CreateBranch {
        name: "feature".to_string(),
      },
      CommandPaletteAction::CreateBranchFrom {
        name: "feature-copy".to_string(),
        base: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::MergeBranch {
        name: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::AbortMerge,
      CommandPaletteAction::RebaseBranch {
        name: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::AbortRebase,
      CommandPaletteAction::StageAll,
      CommandPaletteAction::UnstageAll,
      CommandPaletteAction::Fetch,
      CommandPaletteAction::Stash {
        include_untracked: false,
        message: None,
      },
      CommandPaletteAction::ApplyStash(CommandPaletteStash {
        index: 0,
        name: "stash@{0}".into(),
        oid: "deadbeef".into(),
      }),
      CommandPaletteAction::DropStash(CommandPaletteStash {
        index: 0,
        name: "stash@{0}".into(),
        oid: "deadbeef".into(),
      }),
      CommandPaletteAction::PopStash(CommandPaletteStash {
        index: 0,
        name: "stash@{0}".into(),
        oid: "deadbeef".into(),
      }),
      CommandPaletteAction::CherryPick {
        commit_hashes: vec!["deadbeef".to_string()],
      },
    ];

    for action in actions {
      let result = git_page.update_in(cx, |this, _window, cx| {
        this.selected_repo = None;
        this.handle_command_palette_action(action.clone(), _window, cx)
      });
      let error = result.expect_err("action should fail without selected repo");
      assert_eq!(error.as_ref(), "No repository selected.");
    }
  }

  #[gpui::test]
  async fn command_palette_merge_branch_opens_first_conflicted_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-merge-conflict");
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::MergeBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });

    assert!(result.is_ok(), "merge conflict should be handled in-editor");
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let selected_file = git_page.read_with(cx, |this, _| this.selected_file.clone());
    assert_eq!(selected_file.as_deref(), Some(Path::new("README.md")));
    let commit_input_value = git_page.read_with(cx, |this, cx| {
      this.commit_input.read(cx).value().to_string()
    });
    assert_eq!(
      commit_input_value,
      format!("Merge branch 'feature' into {base_branch}")
    );
    let editor_text = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("editor opened").clone();
      editor.read_with(cx, |editor, cx| {
        let doc = editor.document().read(cx);
        doc.slice_to_string(0..doc.len())
      })
    });
    assert!(
      editor_text.contains("<<<<<<<"),
      "expected conflict markers in opened editor file: {editor_text}"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed merge")
        .name,
      base_branch
    );
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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
  async fn command_palette_abort_merge_clears_merge_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-abort-merge");
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(CommandPaletteAction::AbortMerge, window, cx)
    });
    assert!(
      result.is_ok(),
      "abort merge via command palette should succeed"
    );
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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
  async fn command_palette_abort_rebase_clears_rebase_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-abort-rebase");
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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
      this
        .commit_input
        .update(cx, |input, cx| input.set_value("main change", window, cx));
    });

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(CommandPaletteAction::AbortRebase, window, cx)
    });
    assert!(
      result.is_ok(),
      "abort rebase via command palette should succeed"
    );
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

  #[gpui::test]
  async fn command_palette_rebase_branch_opens_first_conflicted_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-rebase-conflict");
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::RebaseBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });

    assert!(
      result.is_ok(),
      "rebase conflict should be handled in-editor"
    );
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let selected_file = git_page.read_with(cx, |this, _| this.selected_file.clone());
    assert_eq!(selected_file.as_deref(), Some(Path::new("README.md")));
    let commit_input_value = git_page.read_with(cx, |this, cx| {
      this.commit_input.read(cx).value().to_string()
    });
    assert_eq!(commit_input_value, "main change");
    let editor_text = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("editor opened").clone();
      editor.read_with(cx, |editor, cx| {
        let doc = editor.document().read(cx);
        doc.slice_to_string(0..doc.len())
      })
    });
    assert!(
      editor_text.contains("<<<<<<<"),
      "expected conflict markers in opened editor file: {editor_text}"
    );
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );
  }

  #[gpui::test]
  async fn load_history_commit_files_populates_rows_for_commit(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-history-load-files");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    let commit_oid = commit_text_file(&repo.path, rel_path, "v2\n", "update").to_string();

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

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
  async fn open_file_loads_editor_asynchronously(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-open-file-async");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("update worktree file");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_file(rel_path.to_path_buf(), cx);
      assert!(
        this.open_file_task.is_some(),
        "open file should schedule an async load task"
      );
      assert_eq!(this.selected_file, Some(rel_path.to_path_buf()));
      assert!(
        this.editor.is_none(),
        "editor should be created after async load"
      );
    });

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_file, is_read_only, contents) = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("editor should exist").read(cx);
      let document = editor.document().read(cx);
      (
        this.selected_file.clone(),
        editor.is_read_only,
        document.slice_to_string(0..document.len()),
      )
    });

    assert_eq!(selected_file, Some(rel_path.to_path_buf()));
    assert!(!is_read_only);
    assert_eq!(contents, "v2\n");
  }

  #[gpui::test]
  async fn open_file_replaces_history_snapshot_when_same_path_is_selected(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-open-file-after-history");
    let rel_path = Path::new("README.md");
    let old_commit_oid = commit_text_file(&repo.path, rel_path, "v1\n", "initial").to_string();
    let _ = commit_text_file(&repo.path, rel_path, "v2\n", "update");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

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
    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));

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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

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

  #[test]
  fn external_branch_switch_updates_branch_status_and_branch_select_model() {
    let repo = TempRepo::init("git-page-external-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let initial_status = current_branch_status(&repo.path).expect("read initial branch");
    let initial_branch_name = initial_status.name.clone();
    create_branch(&repo.path, "feature").expect("create feature branch");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    repo_handle
      .set_head("refs/heads/feature")
      .expect("set HEAD to feature");

    let switched_status = current_branch_status(&repo.path).expect("read switched branch");
    assert_eq!(switched_status.name, "feature");
    assert!(GitPage::branch_name_changed(
      Some(&initial_status),
      Some(&switched_status)
    ));

    let branches = list_branches(&repo.path).expect("list branches");
    let selected = GitPage::selected_branch_from_status(Some(switched_status));
    let items = GitPage::branch_select_items(branches, selected.as_ref());

    assert_eq!(items.iter().filter(|item| item.is_current).count(), 1);
    assert!(
      items
        .iter()
        .any(|item| item.branch.kind == BranchKind::Local
          && item.branch.name == "feature"
          && item.is_current)
    );
    if initial_branch_name != "feature" {
      assert!(!items.iter().any(|item| {
        item.branch.kind == BranchKind::Local
          && item.branch.name == initial_branch_name
          && item.is_current
      }));
    }
  }

  #[test]
  fn external_detached_head_clears_selected_branch_in_branch_select_model() {
    let repo = TempRepo::init("git-page-external-detach");
    let oid = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let initial_status = current_branch_status(&repo.path).expect("read initial branch");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    repo_handle.set_head_detached(oid).expect("detach HEAD");

    let detached_status = current_branch_status(&repo.path).expect("read detached status");
    assert_eq!(detached_status.name, "HEAD");
    assert!(GitPage::branch_name_changed(
      Some(&initial_status),
      Some(&detached_status)
    ));

    let branches = list_branches(&repo.path).expect("list branches");
    let selected = GitPage::selected_branch_from_status(Some(detached_status));
    assert!(selected.is_none());
    let items = GitPage::branch_select_items(branches, selected.as_ref());
    assert!(items.iter().all(|item| !item.is_current));
  }

  #[gpui::test]
  async fn commit_changes_inner_requires_selected_repo(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));

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
    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));

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
  async fn push_changes_action_requires_selected_repo_and_push_capability(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-push-guards");
    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_push = false;
      this.push_changes_action(cx);
      assert!(this.status_task.is_none());

      this.selected_repo = None;
      this.can_push = true;
      this.push_changes_action(cx);
      assert!(this.status_task.is_none());
    });
  }

  #[gpui::test]
  async fn force_push_changes_action_requires_selected_repo_and_force_capability(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-force-push-guards");
    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_force_push = false;
      this.force_push_changes_action(cx);
      assert!(this.status_task.is_none());

      this.selected_repo = None;
      this.can_force_push = true;
      this.force_push_changes_action(cx);
      assert!(this.status_task.is_none());
    });
  }

  #[gpui::test]
  async fn undo_last_commit_action_requires_selected_repo_and_undo_capability(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-undo-guards");
    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));

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
  async fn push_changes_action_pushes_to_remote_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let source = TempRepo::init("git-page-push-success-source");
    let remote = TempBareRepo::init("git-page-push-success-remote");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");
    set_upstream(&source.path, &branch_name, &format!("origin/{branch_name}"));
    set_remote_head(&remote.path, &branch_name);

    let _ = commit_text_file(&source.path, rel_path, "v2-source\n", "source change");
    let expected_head = head_oid(&source.path);
    assert_ne!(
      remote_branch_oid(&remote.path, &branch_name),
      expected_head,
      "remote should be behind before push"
    );

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let push_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(source.path.clone());
      this.force_push_after_rebase = true;
      this.can_push = true;
      this.push_changes_action(cx);
      this.status_task.take().expect("push task")
    });
    push_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert_eq!(remote_branch_oid(&remote.path, &branch_name), expected_head);
    let status = current_branch_status(&source.path).expect("status after push");
    assert_eq!(status.ahead, 0);
    assert!(!git_page.read_with(cx, |this, _| this.force_push_after_rebase));
  }

  #[gpui::test]
  async fn force_push_changes_action_force_pushes_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let source = TempRepo::init("git-page-force-push-source");
    let remote = TempBareRepo::init("git-page-force-push-remote");
    let peer = TempDir::new("git-page-force-push-peer");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");
    set_upstream(&source.path, &branch_name, &format!("origin/{branch_name}"));
    set_remote_head(&remote.path, &branch_name);

    let _ = Repository::clone(remote.path.to_str().expect("remote path utf8"), &peer.path)
      .expect("clone remote into peer");

    let _ = commit_text_file(&source.path, rel_path, "v2-source\n", "source change");
    let expected_head = head_oid(&source.path);

    let _ = commit_text_file(&peer.path, rel_path, "v2-peer\n", "peer change");
    push_branch_to_remote(&peer.path, &branch_name, "origin");

    let non_force = push(&source.path, false).err();
    assert!(non_force.is_some(), "non-force push should fail");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let force_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(source.path.clone());
      this.force_push_after_rebase = true;
      this.can_force_push = true;
      this.force_push_changes_action(cx);
      this.status_task.take().expect("force push task")
    });
    force_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert_eq!(remote_branch_oid(&remote.path, &branch_name), expected_head);
    assert!(!git_page.read_with(cx, |this, _| this.force_push_after_rebase));
  }

  #[gpui::test]
  async fn stage_restore_actions_require_selected_repo(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));

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

      this.restore_file_action(PathBuf::from("README.md"), RepoStatusKind::Modified, cx);
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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
  async fn unstage_file_action_unstages_target_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-unstage-file-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");
    stage_file(&repo.path, rel_path).expect("stage file before ui action");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(rel_path.to_path_buf(), RepoStatusKind::Modified, cx);
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(PathBuf::from("missing.txt"), RepoStatusKind::Modified, cx);
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(rel_path.to_path_buf(), RepoStatusKind::Untracked, cx);
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(rel_path.to_path_buf(), RepoStatusKind::Deleted, cx);
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
  async fn restore_file_action_selects_first_remaining_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-select-first-remaining");
    let first_path = Path::new("a-first.txt");
    let second_path = Path::new("b-second.txt");
    let _ = commit_text_file(&repo.path, first_path, "v1\n", "initial first");
    let _ = commit_text_file(&repo.path, second_path, "v1\n", "initial second");
    std::fs::write(repo.path.join(first_path), "first change\n").expect("modify first file");
    std::fs::write(repo.path.join(second_path), "second change\n").expect("modify second file");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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
      this.restore_file_action(restore_path.clone(), RepoStatusKind::Modified, cx);
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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
  async fn commit_changes_inner_stages_and_commits_when_ready(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-commit-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("update file");
    let entries = list_repo_status(&repo.path).expect("list status after edit");
    assert!(!entries.is_empty());

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
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

  #[gpui::test]
  async fn poll_once_updates_branch_status_after_external_branch_switch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-poll-once-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      seed_repo_branch_state(this, &repo.path, cx);
    });

    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("read branch after switch")
        .name,
      "feature"
    );

    let status_task = git_page.update_in(cx, |this, _window, cx| {
      this.poll_once_for_test(cx);
      this.status_task.take().expect("poll status task")
    });
    status_task.await;
    let branch_name = git_page.read_with(cx, |this, _| {
      this
        .branch_status
        .as_ref()
        .map(|status| status.name.clone())
    });
    assert_eq!(branch_name.as_deref(), Some("feature"));
  }

  #[gpui::test]
  async fn poll_once_clears_branch_select_on_external_detached_head(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-poll-once-detached");
    let oid = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      seed_repo_branch_state(this, &repo.path, cx);
    });

    Repository::open(&repo.path)
      .expect("open repo")
      .set_head_detached(oid)
      .expect("detach HEAD");

    let status_task = git_page.update_in(cx, |this, _window, cx| {
      this.poll_once_for_test(cx);
      this.status_task.take().expect("poll status task")
    });
    status_task.await;
    if let Some(branch_task) = git_page.update_in(cx, |this, _window, _| this.branch_task.take()) {
      branch_task.await;
    }

    let (branch_name, selected_branch) = git_page.read_with(cx, |this, cx| {
      (
        this
          .branch_status
          .as_ref()
          .map(|status| status.name.clone()),
        this.branch_select.read(cx).selected_value().cloned(),
      )
    });
    assert_eq!(branch_name.as_deref(), Some("HEAD"));
    assert_eq!(selected_branch, None);
  }

  #[gpui::test]
  async fn refresh_branches_updates_branch_select_after_external_branch_switch(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-refresh-branches-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      seed_repo_branch_state(this, &repo.path, cx);
    });

    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("read branch after switch")
        .name,
      "feature"
    );

    let branch_task = git_page.update_in(cx, |this, _window, cx| {
      this.refresh_branches(cx);
      this.branch_task.take().expect("refresh branches task")
    });
    branch_task.await;

    let selected_branch = git_page.read_with(cx, |this, cx| {
      this.branch_select.read(cx).selected_value().cloned()
    });
    assert_eq!(
      selected_branch,
      Some(BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      })
    );
  }

  #[gpui::test]
  async fn reload_status_clears_git_state_when_selected_repo_becomes_unavailable(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-reload-missing-repo");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = cx.add_window_view(|window, cx| GitPage::new_for_test(window, cx));
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      seed_repo_branch_state(this, &repo.path, cx);
      this.status_entries = vec![make_status_entry("README.md", RepoStage::Unstaged)];
      this.branch_status = Some(current_branch_status(&repo.path).expect("read branch status"));
      this.has_head_commit = true;
      this.can_undo_last_commit = true;
      this.can_push = true;
      this.can_force_push = true;
      this.force_push_after_rebase = true;
      this.has_staged_changes = true;
      this.selected_file = Some(rel_path.to_path_buf());
    });

    std::fs::remove_dir_all(&repo.path).expect("remove repo root");

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (
      status_entries_len,
      branch_status,
      has_head_commit,
      can_undo_last_commit,
      can_push,
      can_force_push,
      force_push_after_rebase,
      has_staged_changes,
      selected_file,
      selected_branch,
    ) = git_page.read_with(cx, |this, cx| {
      (
        this.status_entries.len(),
        this.branch_status.clone(),
        this.has_head_commit,
        this.can_undo_last_commit,
        this.can_push,
        this.can_force_push,
        this.force_push_after_rebase,
        this.has_staged_changes,
        this.selected_file.clone(),
        this.branch_select.read(cx).selected_value().cloned(),
      )
    });

    assert_eq!(status_entries_len, 0);
    assert!(branch_status.is_none());
    assert!(!has_head_commit);
    assert!(!can_undo_last_commit);
    assert!(!can_push);
    assert!(!can_force_push);
    assert!(!force_push_after_rebase);
    assert!(!has_staged_changes);
    assert!(selected_file.is_none());
    assert!(selected_branch.is_none());
  }

  #[test]
  fn should_apply_branch_refresh_rejects_stale_generation_or_repo_mismatch() {
    let repo = Path::new("/tmp/repo");
    let other_repo = Path::new("/tmp/other");

    assert!(GitPage::should_apply_branch_refresh(Some(repo), repo, 3, 3));
    assert!(!GitPage::should_apply_branch_refresh(
      Some(repo),
      repo,
      4,
      3
    ));
    assert!(!GitPage::should_apply_branch_refresh(
      Some(other_repo),
      repo,
      3,
      3
    ));
    assert!(!GitPage::should_apply_branch_refresh(None, repo, 3, 3));
  }

  #[test]
  fn branch_refresh_guard_ignores_stale_result_after_repo_switch() {
    let repo_a = TempRepo::init("git-page-refresh-stale-a");
    commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    create_branch(&repo_a.path, "alpha").expect("create alpha branch");
    Repository::open(&repo_a.path)
      .expect("open repo a")
      .set_head("refs/heads/alpha")
      .expect("set HEAD to alpha");

    let repo_b = TempRepo::init("git-page-refresh-stale-b");
    commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    let repo_a_status = current_branch_status(&repo_a.path).expect("read repo a status");
    let repo_a_items = GitPage::branch_select_items(
      list_branches(&repo_a.path).expect("list repo a branches"),
      GitPage::selected_branch_from_status(Some(repo_a_status.clone())).as_ref(),
    );
    assert!(
      repo_a_items
        .iter()
        .any(|item| item.branch.name == "alpha" && item.is_current)
    );

    // Simulate two in-flight refreshes:
    // 1) old refresh requested for repo A at generation 1
    // 2) user switches repo, new refresh requested for repo B at generation 2
    let stale_request_generation = 1;
    let active_generation = 2;
    assert!(!GitPage::should_apply_branch_refresh(
      Some(repo_b.path.as_path()),
      repo_a.path.as_path(),
      active_generation,
      stale_request_generation
    ));
    assert!(GitPage::should_apply_branch_refresh(
      Some(repo_b.path.as_path()),
      repo_b.path.as_path(),
      active_generation,
      active_generation
    ));

    let repo_b_status = current_branch_status(&repo_b.path).expect("read repo b status");
    let repo_b_items = GitPage::branch_select_items(
      list_branches(&repo_b.path).expect("list repo b branches"),
      GitPage::selected_branch_from_status(Some(repo_b_status.clone())).as_ref(),
    );
    assert_eq!(
      repo_b_items.iter().filter(|item| item.is_current).count(),
      1
    );
    assert!(
      repo_b_items
        .iter()
        .any(|item| item.branch.name == repo_b_status.name && item.is_current)
    );
    assert!(
      !repo_b_items
        .iter()
        .any(|item| item.branch.name == "alpha" && item.is_current)
    );
  }

  #[test]
  fn should_refresh_editor_for_path_only_when_selected_matches() {
    let selected = Path::new("src/main.rs");
    let other = Path::new("src/lib.rs");

    assert!(GitPage::should_refresh_editor_for_path(
      Some(selected),
      selected
    ));
    assert!(!GitPage::should_refresh_editor_for_path(
      Some(selected),
      other
    ));
    assert!(!GitPage::should_refresh_editor_for_path(None, selected));
  }

  #[test]
  fn should_show_editor_loading_state_only_when_file_selected_without_editor() {
    let selected = Path::new("src/main.rs");
    assert!(GitPage::should_show_editor_loading_state(
      Some(selected),
      false
    ));
    assert!(!GitPage::should_show_editor_loading_state(
      Some(selected),
      true
    ));
    assert!(!GitPage::should_show_editor_loading_state(None, false));
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

  #[test]
  fn history_change_kind_mapping_covers_all_variants() {
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Added),
      RepoStatusKind::Added
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Deleted),
      RepoStatusKind::Deleted
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Modified),
      RepoStatusKind::Modified
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Renamed),
      RepoStatusKind::Renamed
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Copied),
      RepoStatusKind::Renamed
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Typechange),
      RepoStatusKind::TypeChange
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Conflicted),
      RepoStatusKind::Conflicted
    );
  }

  #[test]
  fn hunk_action_top_uses_local_display_line_position() {
    let top = GitPage::hunk_action_top(gpui::px(20.0), 110, 109.0);
    assert_eq!(top, gpui::px(20.0));
  }

  #[test]
  fn hunk_action_top_handles_fractional_scroll_offset() {
    let top = GitPage::hunk_action_top(gpui::px(18.0), 10, 9.5);
    assert_eq!(top, gpui::px(9.0));
  }
}
