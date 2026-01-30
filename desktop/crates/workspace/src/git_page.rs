use std::{
  collections::HashMap,
  fs,
  ops::Range,
  path::{Path, PathBuf},
  sync::Arc,
  time::{Duration, Instant, SystemTime},
};

use crate::config::{ConfigStore, RecentRepository};
use crate::workspace::{WorkspacePage, WorkspaceRoute};
use editor::{
  ChangeDirection, DiffGutterKind, DiffLineKind, DiffViewMode, Document, Editor, LineDiffHunk,
  diff_line_hunks,
};
use git::{
  BranchKind, BranchRef, BranchStatus, DiffHunkInfo, FileStatusKind, HunkRange, RepositoryFile,
  apply_patch_to_workdir, branch_status, can_undo_last_commit, commit_repository, create_branch,
  create_branch_from, diff_buffers_for_path, has_head_commit, list_branches, open_repository,
  push_repository, restore_change, stage_all, stage_path, switch_branch,
  undo_last_commit as git_undo_last_commit, unstage_all, unstage_path, write_index_content,
};
use gpui::{
  App, ClickEvent, Context, Corner, Div, Entity, FocusHandle, Focusable, Hsla, Keystroke,
  PathPromptOptions, Pixels, Render, SharedString, Stateful, Subscription, Task, Window, actions,
  div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Theme as ComponentTheme,
  button::{ButtonGroup, ButtonVariant},
  kbd::Kbd,
  menu::{DropdownMenu as _, PopupMenuItem},
  tooltip::Tooltip,
};
use syntax::Theme as SyntaxTheme;
use ui::{
  Button, ButtonVariants, Collapsible, CommandPalette, CommandPaletteAction, CommandPaletteBranch,
  CommandPaletteBranchKind, CommandPaletteConfig, CommandPaletteHandler, ConfirmDialog,
  Disableable, IconName, Input, InputState, ResizableState, SearchableVec, Select, SelectEvent,
  SelectItem, SelectState, Sidebar, SidebarItem, Sizable, WindowExt, h_resizable, resizable_panel,
};

const SIDEBAR_DEFAULT_WIDTH: Pixels = px(260.0);
const SIDEBAR_MIN_WIDTH: Pixels = px(200.0);
const SIDEBAR_MAX_WIDTH: Pixels = px(600.0);
const APP_HEADER_HEIGHT: f32 = 42.0;
const FILE_POLL_INTERVAL_MS: u64 = 500;
const REPO_POLL_INTERVAL_MS: u64 = 1500;
const INCREMENTAL_DIFF_CONTEXT_LINES: usize = 80;

actions!(
  workspace,
  [OpenRepository, SaveFile, CommitChanges, ShowCommandPalette]
);

#[derive(Clone)]
struct FileEntry {
  path: PathBuf,
  display_name: String,
  status: FileStatusKind,
  head_content: Option<String>,
  index_content: Option<String>,
  workdir_content: Option<String>,
  saved_content: Option<String>,
  last_modified: Option<SystemTime>,
  stage_state: Option<StageState>,
}

fn entry_diff_base(entry: &FileEntry) -> Option<&str> {
  match entry.stage_state {
    Some(StageState::Staged) | Some(StageState::PartiallyStaged) => entry
      .head_content
      .as_deref()
      .or_else(|| entry.index_content.as_deref()),
    _ => entry.index_content.as_deref(),
  }
}

#[derive(Clone)]
struct SidebarFileEntry {
  list: FileListKind,
  list_index: usize,
  path: PathBuf,
  display_name: String,
  status: FileStatusKind,
  stage_state: Option<StageState>,
}

impl SidebarFileEntry {
  fn new(list: FileListKind, list_index: usize, entry: &FileEntry) -> Self {
    Self {
      list,
      list_index,
      path: entry.path.clone(),
      display_name: entry.display_name.clone(),
      status: entry.status,
      stage_state: entry.stage_state,
    }
  }
}

#[derive(Clone)]
struct FileDiffHunks {
  head_to_index: Vec<DiffHunkInfo>,
  index_to_workdir: Vec<DiffHunkInfo>,
}

#[derive(Clone)]
struct RepoSelectItem {
  label: SharedString,
  path: PathBuf,
}

impl SelectItem for RepoSelectItem {
  type Value = PathBuf;

  fn title(&self) -> SharedString {
    self.label.clone()
  }

  fn value(&self) -> &Self::Value {
    &self.path
  }

  fn matches(&self, query: &str) -> bool {
    let query = query.to_lowercase();
    self.label.as_ref().to_lowercase().contains(&query)
      || self.path.to_string_lossy().to_lowercase().contains(&query)
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileListKind {
  Tracked,
  Untracked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectedFile {
  Tracked(usize),
  Untracked(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StageState {
  Unstaged,
  PartiallyStaged,
  Staged,
}

#[derive(Clone)]
struct WorkspaceSidebarSection {
  view: Entity<GitPage>,
  entries: Vec<SidebarFileEntry>,
  selected_path: Option<PathBuf>,
  has_root: bool,
  collapsed: bool,
  dirty_path: Option<PathBuf>,
}

impl WorkspaceSidebarSection {
  fn new(
    view: Entity<GitPage>,
    entries: Vec<SidebarFileEntry>,
    selected_path: Option<PathBuf>,
    has_root: bool,
    dirty_path: Option<PathBuf>,
  ) -> Self {
    Self {
      view,
      entries,
      selected_path,
      has_root,
      collapsed: false,
      dirty_path,
    }
  }
}

impl Collapsible for WorkspaceSidebarSection {
  fn collapsed(mut self, collapsed: bool) -> Self {
    self.collapsed = collapsed;
    self
  }

  fn is_collapsed(&self) -> bool {
    self.collapsed
  }
}

impl SidebarItem for WorkspaceSidebarSection {
  fn render(
    self,
    id: impl Into<gpui::ElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> impl IntoElement {
    let id = id.into();
    let WorkspaceSidebarSection {
      view,
      entries,
      selected_path,
      has_root,
      dirty_path,
      ..
    } = self;

    render_sidebar_list(
      &view,
      &entries,
      selected_path.as_deref(),
      has_root,
      dirty_path.as_deref(),
      cx.theme(),
      window,
    )
    .id(id)
  }
}

pub struct GitPage {
  root_path: Option<PathBuf>,
  tracked: Vec<FileEntry>,
  untracked: Vec<FileEntry>,
  selected_file: Option<SelectedFile>,
  editor: Option<Entity<Editor>>,
  selected_diff_hunks: Option<FileDiffHunks>,
  error: Option<String>,
  current_dirty: bool,
  buffer_dirty_current_ranges: Vec<Range<usize>>,
  buffer_saved_to_current_hunks: Vec<LineDiffHunk>,
  saved_to_current_tracking: bool,
  auto_unstaging: bool,
  document_subscription: Option<Subscription>,
  document_edit_epoch: Option<usize>,
  poll_task: Option<Task<()>>,
  focus_handle: FocusHandle,
  recent_repositories: Vec<RecentRepository>,
  repo_select: Option<Entity<SelectState<SearchableVec<RepoSelectItem>>>>,
  sidebar_state: Entity<ResizableState>,
  theme: SyntaxTheme,
  diff_view_mode: DiffViewMode,
  commit_input: Entity<InputState>,
  has_head_commit: bool,
  can_undo_last_commit: bool,
  has_staged_changes: bool,
  can_push: bool,
  can_force_push: bool,
  branch_status: Option<BranchStatus>,
  last_repo_poll: Option<Instant>,
}

impl GitPage {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let recent_repositories = ConfigStore::load_recent_repositories();
    let theme = if cx.theme().is_dark() {
      SyntaxTheme::dark()
    } else {
      SyntaxTheme::light()
    };
    let commit_input = cx.new(|cx| {
      InputState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });
    let sidebar_state = cx.new(|_| ResizableState::default());
    let mut view = Self {
      root_path: None,
      tracked: Vec::new(),
      untracked: Vec::new(),
      selected_file: None,
      editor: None,
      selected_diff_hunks: None,
      error: None,
      current_dirty: false,
      buffer_dirty_current_ranges: Vec::new(),
      buffer_saved_to_current_hunks: Vec::new(),
      saved_to_current_tracking: false,
      auto_unstaging: false,
      document_subscription: None,
      document_edit_epoch: None,
      poll_task: None,
      focus_handle: cx.focus_handle(),
      recent_repositories,
      repo_select: None,
      sidebar_state,
      theme,
      diff_view_mode: DiffViewMode::Inline,
      commit_input,
      has_head_commit: false,
      can_undo_last_commit: false,
      has_staged_changes: false,
      can_push: false,
      can_force_push: false,
      branch_status: None,
      last_repo_poll: None,
    };
    if let Some(repo) = view.recent_repositories.first() {
      view.set_root_path(repo.path.clone(), cx);
    }
    view.start_file_polling(cx);
    view
  }

  fn sync_theme_from_app(&mut self, cx: &mut Context<Self>) {
    let is_dark = cx.theme().is_dark();
    if self.theme.is_dark == is_dark {
      return;
    }

    self.theme = if is_dark {
      SyntaxTheme::dark()
    } else {
      SyntaxTheme::light()
    };

    let theme = self.theme.clone();
    if let Some(editor) = self.editor.as_ref() {
      editor.update(cx, |editor, cx| {
        editor.set_theme(theme.clone(), cx);
      });
    }
  }

  fn toggle_diff_view(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    self.diff_view_mode.toggle();
    let mode = self.diff_view_mode;
    if let Some(editor) = self.editor.as_ref() {
      editor.update(cx, |editor, cx| {
        editor.set_diff_view_mode(mode, cx);
      });
    }
    cx.notify();
  }

  fn commit_changes(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    self.commit_changes_with_amend(false, window, cx);
  }

  fn commit_amend_changes(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    self.commit_changes_with_amend(true, window, cx);
  }

  fn undo_last_commit(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if !can_undo_last_commit(&root_path) {
      self.error =
        Some("Cannot undo last commit: already pushed or no upstream branch.".to_string());
      cx.notify();
      return;
    }
    if let Err(err) = git_undo_last_commit(&root_path) {
      self.error = Some(format!("Failed to undo last commit: {err}"));
      cx.notify();
      return;
    }

    self.error = None;
    self.refresh_repository_statuses(cx);
    self.rebuild_editor_for_selected(cx);
  }

  fn push_changes(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    self.push_changes_with_force(false, cx);
  }

  fn force_push_changes(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    self.push_changes_with_force(true, cx);
  }

  fn push_changes_with_force(&mut self, force: bool, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    let result = push_repository(&root_path, force);
    if let Err(err) = result {
      let action = if force { "force push" } else { "push" };
      self.error = Some(format!("Failed to {action}: {err}"));
      cx.notify();
      return;
    }

    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn commit_changes_with_amend(
    &mut self,
    amend: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let message = self.commit_input.read(cx).value().to_string();
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if !amend && message.trim().is_empty() {
      return;
    }
    if amend && !self.has_head_commit {
      self.error = Some("Nothing to amend.".to_string());
      cx.notify();
      return;
    }
    if !amend && !self.has_staged_changes {
      let has_changes = !(self.tracked.is_empty() && self.untracked.is_empty());
      if !has_changes {
        return;
      }
      if let Err(err) = stage_all(&root_path) {
        self.error = Some(format!("Failed to stage all files: {err}"));
        cx.notify();
        return;
      }
    }
    if let Err(err) = commit_repository(&root_path, &message, amend) {
      self.error = Some(format!("Failed to commit: {err}"));
      cx.notify();
      return;
    }

    self.error = None;
    self.commit_input.update(cx, |input, cx| {
      input.set_value("", window, cx);
    });
    self.refresh_repository_statuses(cx);
  }

  fn stage_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if let Err(err) = stage_path(&root_path, &path) {
      self.error = Some(format!("Failed to stage file: {err}"));
      cx.notify();
      return;
    }
    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn unstage_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if let Err(err) = unstage_path(&root_path, &path) {
      self.error = Some(format!("Failed to unstage file: {err}"));
      cx.notify();
      return;
    }
    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn restore_file_change(&mut self, path: PathBuf, status: FileStatusKind, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if let Err(err) = restore_change(&root_path, &path, status) {
      self.error = Some(format!("Failed to restore change: {err}"));
      cx.notify();
      return;
    }
    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn stage_all_files(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if let Err(err) = stage_all(&root_path) {
      self.error = Some(format!("Failed to stage all files: {err}"));
      cx.notify();
      return;
    }
    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn unstage_all_files(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if let Err(err) = unstage_all(&root_path) {
      self.error = Some(format!("Failed to unstage all files: {err}"));
      cx.notify();
      return;
    }
    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn stage_change_block(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
    if self.current_dirty {
      return;
    }
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    let Some(SelectedFile::Tracked(index)) = self.selected_file else {
      return;
    };
    let Some(entry) = self.tracked.get(index) else {
      return;
    };
    let Some(_editor) = self.editor.as_ref() else {
      return;
    };
    let Some(hunks_state) = self.selected_diff_hunks.as_ref() else {
      return;
    };
    let rel_path = entry
      .path
      .strip_prefix(&root_path)
      .unwrap_or(entry.path.as_path());
    let index_text = entry.index_content.as_deref().unwrap_or("");
    let workdir_text = entry.workdir_content.as_deref().unwrap_or("");
    let mut targets = self.unstaged_line_hunks_from_range(
      &range,
      index_text,
      workdir_text,
      &hunks_state.head_to_index,
      cx,
    );
    if targets.is_empty() {
      return;
    }
    targets.sort_by(|left, right| right.old_start.cmp(&left.old_start));
    let mut index_lines = split_text_lines(index_text);
    let workdir_lines = split_text_lines(workdir_text);
    for hunk in targets {
      let (old_start, old_end) =
        splice_range_for_hunk(hunk.old_start, hunk.old_lines, index_lines.len());
      let new_start = hunk.new_start.min(workdir_lines.len());
      let new_end = (hunk.new_start + hunk.new_lines).min(workdir_lines.len());
      if old_start > old_end || new_start > new_end {
        continue;
      }
      index_lines.splice(
        old_start..old_end,
        workdir_lines[new_start..new_end].iter().cloned(),
      );
    }
    let new_index_text = index_lines.join("\n");
    if new_index_text == index_text {
      return;
    }
    if let Err(err) = write_index_content(&root_path, rel_path, &new_index_text) {
      self.error = Some(format!("Failed to stage hunk: {err}"));
      cx.notify();
      return;
    }

    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn restore_change_block(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
    if self.current_dirty {
      return;
    }
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    let Some(SelectedFile::Tracked(index)) = self.selected_file else {
      return;
    };
    let Some(entry) = self.tracked.get(index) else {
      return;
    };
    let index_text = entry.index_content.as_deref().unwrap_or("");
    let workdir_text = entry.workdir_content.as_deref().unwrap_or("");
    let diff = match diff_buffers_for_path(&root_path, &entry.path, workdir_text, index_text, 0) {
      Ok(diff) => diff,
      Err(err) => {
        self.error = Some(format!("Failed to build diff: {err}"));
        cx.notify();
        return;
      }
    };
    let targets = self.collect_targets_workdir_to_index(&range, cx, &diff.hunks);
    if targets.is_empty() {
      return;
    }

    if let Err(err) = apply_patch_to_workdir(&root_path, &entry.path, &diff.patch, &targets) {
      self.error = Some(format!("Failed to restore hunk: {err}"));
      cx.notify();
      return;
    }

    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn unstage_change_block(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
    self.unstage_change_block_internal(range, false, cx);
  }

  fn unstage_change_block_internal(
    &mut self,
    range: Range<usize>,
    force: bool,
    cx: &mut Context<Self>,
  ) {
    if !force && self.current_dirty {
      return;
    }
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    let Some(SelectedFile::Tracked(index)) = self.selected_file else {
      return;
    };
    let Some(entry) = self.tracked.get(index) else {
      return;
    };
    let Some(_editor) = self.editor.as_ref() else {
      return;
    };
    if self.selected_diff_hunks.is_none() {
      return;
    }
    let index_text = entry.index_content.as_deref().unwrap_or("");
    let head_text = entry.head_content.as_deref().unwrap_or("");
    let workdir_text = entry.workdir_content.as_deref().unwrap_or("");
    let mut targets =
      self.staged_line_hunks_from_range(&range, head_text, index_text, workdir_text, cx);
    if targets.is_empty() {
      return;
    }
    let rel_path = entry
      .path
      .strip_prefix(&root_path)
      .unwrap_or(entry.path.as_path());
    targets.sort_by(|left, right| right.new_start.cmp(&left.new_start));
    let mut index_lines = split_text_lines(index_text);
    let head_lines = split_text_lines(head_text);
    for hunk in targets {
      let (old_start, old_end) =
        splice_range_for_hunk(hunk.new_start, hunk.new_lines, index_lines.len());
      let new_start = hunk.old_start.min(head_lines.len());
      let new_end = (hunk.old_start + hunk.old_lines).min(head_lines.len());
      if old_start > old_end || new_start > new_end {
        continue;
      }
      index_lines.splice(
        old_start..old_end,
        head_lines[new_start..new_end].iter().cloned(),
      );
    }
    let new_index_text = index_lines.join("\n");
    if new_index_text == index_text {
      return;
    }
    if let Err(err) = write_index_content(&root_path, rel_path, &new_index_text) {
      self.error = Some(format!("Failed to unstage hunk: {err}"));
      cx.notify();
      return;
    }

    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn update_saved_to_current_hunks_incremental(
    &self,
    saved_text: &str,
    current_text: &str,
    last_edit_view_range: &Range<usize>,
    doc: &Document,
  ) -> Option<Vec<LineDiffHunk>> {
    let current_edit_range = if doc.diff_enabled() {
      doc.map_view_line_range_to_current(last_edit_view_range)?
    } else {
      last_edit_view_range.clone()
    };

    let current_line_count = doc.buffer.len_lines();
    if current_line_count == 0 {
      return None;
    }
    let current_start = current_edit_range
      .start
      .saturating_sub(INCREMENTAL_DIFF_CONTEXT_LINES);
    let current_end =
      (current_edit_range.end + INCREMENTAL_DIFF_CONTEXT_LINES).min(current_line_count);
    if current_start >= current_end {
      return None;
    }
    let current_window = current_start..current_end;

    let mut existing = self.buffer_saved_to_current_hunks.clone();
    existing.sort_by_key(|hunk| hunk.old_start);
    let inverted = invert_line_hunks(&existing);
    let mut saved_window = map_line_range(&current_window, &inverted);
    let saved_line_count = line_count_from_text(saved_text);
    saved_window.start = saved_window.start.min(saved_line_count);
    saved_window.end = saved_window.end.min(saved_line_count);
    if saved_window.start >= saved_window.end {
      return None;
    }

    let saved_slice = text_for_line_range(saved_text, &saved_window);
    let current_slice = text_for_line_range(current_text, &current_window);
    let mut local_hunks = diff_line_hunks(&saved_slice, &current_slice);
    for hunk in &mut local_hunks {
      hunk.old_start += saved_window.start;
      hunk.new_start += current_window.start;
    }

    let old_mapped_end = map_line_index(saved_window.end, &existing);
    let delta_old_end = old_mapped_end as isize - saved_window.end as isize;
    let delta_new_end = current_window.end as isize - saved_window.end as isize;
    let delta_diff = delta_new_end - delta_old_end;

    let mut next_hunks = Vec::new();
    for mut hunk in existing {
      if line_hunk_overlaps_window(&hunk, &saved_window) {
        continue;
      }
      if hunk.old_start >= saved_window.end {
        hunk.new_start = ((hunk.new_start as isize) + delta_diff).max(0) as usize;
      }
      next_hunks.push(hunk);
    }
    next_hunks.extend(local_hunks);
    next_hunks.sort_by_key(|hunk| hunk.old_start);

    Some(next_hunks)
  }

  fn hunk_range_for_change(&self, range: &Range<usize>, cx: &App) -> Option<HunkRange> {
    let editor = self.editor.as_ref()?;
    let document = editor.read(cx).document().read(cx);

    let mut base_min: Option<usize> = None;
    let mut base_max: Option<usize> = None;
    let mut current_min: Option<usize> = None;
    let mut current_max: Option<usize> = None;

    for line_idx in range.clone() {
      if let Some(info) = document.diff_line_info(line_idx) {
        if let Some(base_line) = info.base_line {
          base_min = Some(base_min.map_or(base_line, |min| min.min(base_line)));
          base_max = Some(base_max.map_or(base_line, |max| max.max(base_line)));
        }
        if let Some(current_line) = info.current_line {
          current_min = Some(current_min.map_or(current_line, |min| min.min(current_line)));
          current_max = Some(current_max.map_or(current_line, |max| max.max(current_line)));
        }
      }
    }

    if base_min.is_none() && current_min.is_none() {
      return None;
    }

    let base = base_min.map(|min| min..(base_max.unwrap_or(min) + 1));
    let current = current_min.map(|min| min..(current_max.unwrap_or(min) + 1));

    Some(HunkRange { base, current })
  }

  fn staged_line_hunks_from_range(
    &self,
    range: &Range<usize>,
    head_text: &str,
    index_text: &str,
    workdir_text: &str,
    cx: &App,
  ) -> Vec<LineDiffHunk> {
    let Some(editor) = self.editor.as_ref() else {
      return Vec::new();
    };
    let editor_state = editor.read(cx);
    let document = editor_state.document().read(cx);
    let head_to_index = diff_line_hunks(head_text, index_text);
    let index_to_workdir = diff_line_hunks(index_text, workdir_text);
    let inverted = invert_line_hunks(&index_to_workdir);
    let mut targets = Vec::new();

    for line_idx in range.clone() {
      let Some(info) = document.diff_line_info(line_idx) else {
        continue;
      };
      if !editor_state.diff_line_is_staged(line_idx, cx) {
        continue;
      }
      if info.kind == DiffLineKind::Unchanged && info.gutter == DiffGutterKind::None {
        continue;
      }
      if let Some(base_line) = info.base_line {
        let base_range = base_line..(base_line + 1);
        if let Some(found) = find_line_hunk_by_old_range(&head_to_index, &base_range) {
          push_unique_line_hunk(&mut targets, found);
        }
      }
      if let Some(current_line) = info.current_line {
        let index_line = map_line_index(current_line, &inverted);
        let index_range = index_line..(index_line + 1);
        if let Some(found) = find_line_hunk_by_new_range(&head_to_index, &index_range) {
          push_unique_line_hunk(&mut targets, found);
        }
      }
    }

    targets
  }

  fn range_has_staged_lines(&self, range: &Range<usize>, cx: &App) -> bool {
    let Some(editor) = self.editor.as_ref() else {
      return false;
    };
    let editor_state = editor.read(cx);
    let document = editor_state.document().read(cx);
    for line_idx in range.clone() {
      if !editor_state.diff_line_is_staged(line_idx, cx) {
        continue;
      }
      if let Some(info) = document.diff_line_info(line_idx) {
        if info.kind != DiffLineKind::Unchanged || info.gutter != DiffGutterKind::None {
          return true;
        }
      }
    }
    false
  }

  fn unstaged_line_hunks_from_range(
    &self,
    range: &Range<usize>,
    base_text: &str,
    current_text: &str,
    head_to_index: &[DiffHunkInfo],
    cx: &App,
  ) -> Vec<LineDiffHunk> {
    let Some(editor) = self.editor.as_ref() else {
      return Vec::new();
    };
    let editor_state = editor.read(cx);
    let document = editor_state.document().read(cx);
    let hunks = diff_line_hunks(base_text, current_text);
    let mut targets = Vec::new();

    for line_idx in range.clone() {
      let Some(info) = document.diff_line_info(line_idx) else {
        continue;
      };
      if editor_state.diff_line_is_staged(line_idx, cx) {
        continue;
      }
      if info.kind == DiffLineKind::Unchanged && info.gutter == DiffGutterKind::None {
        continue;
      }
      if let Some(base_line) = info.base_line {
        let index_line = map_line_index(base_line, head_to_index);
        let base_range = index_line..(index_line + 1);
        if let Some(found) = find_line_hunk_by_old_range(&hunks, &base_range) {
          push_unique_line_hunk(&mut targets, found);
        }
      }
      if let Some(current_line) = info.current_line {
        let current_range = current_line..(current_line + 1);
        if let Some(found) = find_line_hunk_by_new_range(&hunks, &current_range) {
          push_unique_line_hunk(&mut targets, found);
        }
      }
    }

    targets
  }

  fn find_unstage_hunk(&self, hunk: &HunkRange) -> Option<HunkRange> {
    let hunks = self.selected_diff_hunks.as_ref()?;
    if let Some(base) = hunk.base.as_ref() {
      if let Some(found) = find_hunk_by_old_range(&hunks.head_to_index, base) {
        return Some(hunk_header_range(found));
      }
    }
    if let Some(current) = hunk.current.as_ref() {
      let inverted = invert_hunks(&hunks.index_to_workdir);
      let mapped = map_line_range(current, &inverted);
      if let Some(found) = find_hunk_by_new_range(&hunks.head_to_index, &mapped) {
        return Some(hunk_header_range(found));
      }
    }
    None
  }

  fn collect_targets_workdir_to_index(
    &self,
    range: &Range<usize>,
    cx: &App,
    hunks: &[DiffHunkInfo],
  ) -> Vec<HunkRange> {
    let Some(editor) = self.editor.as_ref() else {
      return Vec::new();
    };
    let document = editor.read(cx).document().read(cx);
    let mut targets: Vec<HunkRange> = Vec::new();

    for line_idx in range.clone() {
      let Some(info) = document.diff_line_info(line_idx) else {
        continue;
      };
      if let Some(current_line) = info.current_line {
        let old_range = current_line..(current_line + 1);
        if let Some(found) = find_hunk_by_old_range(hunks, &old_range) {
          let header = hunk_header_range(found);
          if !targets
            .iter()
            .any(|existing| existing.base == header.base && existing.current == header.current)
          {
            targets.push(header);
          }
        }
      }
      if let Some(base_line) = info.base_line {
        let new_range = base_line..(base_line + 1);
        if let Some(found) = find_hunk_by_new_range(hunks, &new_range) {
          let header = hunk_header_range(found);
          if !targets
            .iter()
            .any(|existing| existing.base == header.base && existing.current == header.current)
          {
            targets.push(header);
          }
        }
      }
    }

    targets
  }

  fn stage_state_for_range(&self, range: &Range<usize>, cx: &App) -> Option<StageState> {
    if self.selected_diff_hunks.is_none() {
      return None;
    }
    let editor = self.editor.as_ref()?;
    let editor_state = editor.read(cx);
    let mut has_staged = false;
    let mut has_unstaged = false;

    for line_idx in range.clone() {
      if editor_state.diff_line_is_staged(line_idx, cx) {
        has_staged = true;
      } else {
        has_unstaged = true;
      }
      if has_staged && has_unstaged {
        break;
      }
    }

    match (has_staged, has_unstaged) {
      (true, true) => Some(StageState::PartiallyStaged),
      (true, false) => Some(StageState::Staged),
      (false, true) => Some(StageState::Unstaged),
      (false, false) => None,
    }
  }

  fn rebuild_editor_for_selected(&mut self, cx: &mut Context<Self>) {
    let Some(selected) = self.selected_file else {
      return;
    };

    let (content, base_content, file_path) = match selected {
      SelectedFile::Tracked(index) => self
        .tracked
        .get(index)
        .map(|entry| {
          (
            entry.workdir_content.clone(),
            entry_diff_base(entry).map(|text| text.to_string()),
            entry.path.clone(),
          )
        })
        .unwrap_or((None, None, PathBuf::new())),
      SelectedFile::Untracked(index) => self
        .untracked
        .get(index)
        .map(|entry| {
          (
            entry.workdir_content.clone(),
            entry_diff_base(entry).map(|text| text.to_string()),
            entry.path.clone(),
          )
        })
        .unwrap_or((None, None, PathBuf::new())),
    };

    let Some(content) = content else {
      return;
    };

    let file_ext = file_path.extension().and_then(|ext| ext.to_str());
    let theme = self.theme.clone();
    let editor = cx.new(|cx| Editor::new(&content, base_content.as_deref(), file_ext, theme, cx));
    let mode = self.diff_view_mode;
    editor.update(cx, |editor, cx| {
      editor.set_diff_view_mode(mode, cx);
      editor.set_staged_ranges(Vec::new(), Vec::new(), cx);
    });
    self.editor = Some(editor.clone());
    self.attach_editor_observer(&editor, cx);
    self.update_selected_diff_hunks(cx);
    self.current_dirty = false;
    self.buffer_dirty_current_ranges.clear();
    self.buffer_saved_to_current_hunks.clear();
    self.saved_to_current_tracking = false;
    cx.notify();
  }

  fn diff_hunks_for_entry(&self, entry: &FileEntry) -> Option<FileDiffHunks> {
    let root_path = self.root_path.as_ref()?;
    let head_text = entry.head_content.as_deref().unwrap_or("");
    let index_text = entry.index_content.as_deref().unwrap_or("");
    let workdir_text = entry.workdir_content.as_deref().unwrap_or("");

    let mut head_to_index = diff_buffers_for_path(root_path, &entry.path, head_text, index_text, 0)
      .ok()?
      .hunks;
    let mut index_to_workdir =
      diff_buffers_for_path(root_path, &entry.path, index_text, workdir_text, 0)
        .ok()?
        .hunks;
    head_to_index.sort_by_key(|hunk| hunk.old_start);
    index_to_workdir.sort_by_key(|hunk| hunk.old_start);
    Some(FileDiffHunks {
      head_to_index,
      index_to_workdir,
    })
  }

  fn update_selected_diff_hunks(&mut self, cx: &mut Context<Self>) {
    let Some(selected) = self.selected_file else {
      self.selected_diff_hunks = None;
      return;
    };

    let (entry, tracked) = match selected {
      SelectedFile::Tracked(index) => (self.tracked.get(index), true),
      SelectedFile::Untracked(index) => (self.untracked.get(index), false),
    };

    let Some(entry) = entry else {
      self.selected_diff_hunks = None;
      return;
    };

    if !tracked {
      self.selected_diff_hunks = None;
      if let Some(editor) = self.editor.as_ref() {
        editor.update(cx, |editor, cx| {
          editor.set_staged_ranges(Vec::new(), Vec::new(), cx);
        });
      }
      return;
    }

    let Some(hunks) = self.diff_hunks_for_entry(entry) else {
      self.selected_diff_hunks = None;
      if let Some(editor) = self.editor.as_ref() {
        editor.update(cx, |editor, cx| {
          editor.set_staged_ranges(Vec::new(), Vec::new(), cx);
        });
      }
      return;
    };
    self.selected_diff_hunks = Some(hunks);
    self.apply_staged_ranges_for_selected(cx);
  }

  fn update_dirty_state_from_text(
    &mut self,
    selected_file: SelectedFile,
    saved_text: &str,
    current_text: &str,
    cx: &mut Context<Self>,
  ) -> (bool, bool) {
    let is_dirty = current_text != saved_text;
    let mut dirty_current_ranges = Vec::new();
    let mut saved_to_current_hunks = Vec::new();
    let has_staged_for_file = self
      .selected_diff_hunks
      .as_ref()
      .map(|hunks| !hunks.head_to_index.is_empty())
      .unwrap_or(false);
    if !has_staged_for_file {
      self.saved_to_current_tracking = false;
    }

    let (last_edit_range, document) = if let Some(editor) = self.editor.as_ref() {
      let editor_state = editor.read(cx);
      (
        editor_state.last_edit_view_range(),
        Some(editor_state.document().clone()),
      )
    } else {
      (None, None)
    };

    if is_dirty && matches!(selected_file, SelectedFile::Tracked(_)) && has_staged_for_file {
      let mut did_incremental = false;
      if self.saved_to_current_tracking {
        if let (Some(view_range), Some(document)) = (last_edit_range.as_ref(), document.as_ref()) {
          let doc = document.read(cx);
          if let Some(updated) = self.update_saved_to_current_hunks_incremental(
            saved_text,
            current_text,
            view_range,
            &doc,
          ) {
            saved_to_current_hunks = updated;
            did_incremental = true;
          }
        }
      }
      if !did_incremental {
        saved_to_current_hunks = diff_line_hunks(saved_text, current_text);
      }
      self.saved_to_current_tracking = true;
      for hunk in &saved_to_current_hunks {
        if hunk.new_lines > 0 {
          dirty_current_ranges.push(hunk.new_start..(hunk.new_start + hunk.new_lines));
        }
      }
    } else if !is_dirty {
      self.saved_to_current_tracking = false;
    }

    if let Some(document) = document.as_ref() {
      if let Some(view_range) = last_edit_range.as_ref() {
        let mut change_range = None;
        let mut edit_dirty_ranges = Vec::new();
        {
          let doc = document.read(cx);
          if is_dirty
            && matches!(selected_file, SelectedFile::Tracked(_))
            && has_staged_for_file
            && !self.auto_unstaging
          {
            let line_idx = view_range.start.min(doc.len_lines().saturating_sub(1));
            change_range = doc.diff_change_range_at_line(line_idx);
          }
          for line_idx in view_range.clone() {
            if doc.diff_enabled() {
              if let Some(info) = doc.diff_line_info(line_idx)
                && let Some(current_line) = info.current_line
              {
                edit_dirty_ranges.push(current_line..(current_line + 1));
              }
            } else {
              edit_dirty_ranges.push(line_idx..(line_idx + 1));
            }
          }
        }

        if is_dirty
          && matches!(selected_file, SelectedFile::Tracked(_))
          && has_staged_for_file
          && !self.auto_unstaging
        {
          if let Some(change_range) = change_range
            && self.range_has_staged_lines(&change_range, cx)
          {
            self.auto_unstaging = true;
            self.unstage_change_block_internal(change_range.clone(), true, cx);
            self.auto_unstaging = false;
          }
        }

        dirty_current_ranges.extend(edit_dirty_ranges);
      }

      if let (Some(view_range), Some(hunks_state)) =
        (last_edit_range.as_ref(), self.selected_diff_hunks.as_ref())
      {
        if let Some(edit_hunk) = self.hunk_range_for_change(view_range, cx) {
          if let Some(target_hunk) = self.find_unstage_hunk(&edit_hunk)
            && let Some(index_range) = target_hunk.current.as_ref()
          {
            let current_range = map_line_range(index_range, &hunks_state.index_to_workdir);
            let mapped_current_range = if !saved_to_current_hunks.is_empty() {
              map_line_range(&current_range, &saved_to_current_hunks)
            } else {
              current_range
            };
            dirty_current_ranges.push(mapped_current_range);
          }
        }
      }
    }
    dirty_current_ranges = merge_ranges(dirty_current_ranges);
    let dirty_ranges_changed = self.buffer_dirty_current_ranges != dirty_current_ranges;
    if dirty_ranges_changed {
      self.buffer_dirty_current_ranges = dirty_current_ranges;
    }
    let hunks_changed = self.buffer_saved_to_current_hunks != saved_to_current_hunks;
    if hunks_changed {
      self.buffer_saved_to_current_hunks = saved_to_current_hunks;
    }
    let dirty_state_changed = self.current_dirty != is_dirty;
    if dirty_state_changed {
      self.current_dirty = is_dirty;
    }
    if dirty_ranges_changed || dirty_state_changed || hunks_changed {
      self.apply_staged_ranges_for_selected(cx);
    }
    (
      is_dirty,
      dirty_ranges_changed || dirty_state_changed || hunks_changed,
    )
  }

  fn sync_dirty_state_from_editor(&mut self, cx: &mut Context<Self>) {
    let Some(selected_file) = self.selected_file else {
      return;
    };
    let Some(editor) = self.editor.as_ref() else {
      return;
    };
    let (saved_text, entry_available) = match selected_file {
      SelectedFile::Tracked(index) => self
        .tracked
        .get(index)
        .map(|entry| {
          (
            entry.saved_content.as_deref().unwrap_or("").to_string(),
            true,
          )
        })
        .unwrap_or_else(|| (String::new(), false)),
      SelectedFile::Untracked(index) => self
        .untracked
        .get(index)
        .map(|entry| {
          (
            entry.saved_content.as_deref().unwrap_or("").to_string(),
            true,
          )
        })
        .unwrap_or_else(|| (String::new(), false)),
    };
    if !entry_available {
      return;
    }
    let current_text = editor_buffer_text(editor, cx);
    let (_, needs_notify) =
      self.update_dirty_state_from_text(selected_file, &saved_text, &current_text, cx);
    if needs_notify {
      cx.notify();
    }
  }

  fn attach_editor_observer(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
    let document = editor.read(cx).document.clone();
    self.document_edit_epoch = Some(document.read(cx).edit_epoch());
    let subscription = cx.observe(&document, |this, document, cx| {
      let epoch = document.read(cx).edit_epoch();
      if this.document_edit_epoch == Some(epoch) {
        return;
      }
      this.document_edit_epoch = Some(epoch);
      this.sync_dirty_state_from_editor(cx);
    });
    self.document_subscription = Some(subscription);
  }

  fn clear_editor_observer(&mut self) {
    self.document_subscription = None;
    self.document_edit_epoch = None;
  }

  fn apply_staged_ranges_for_selected(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.as_ref() else {
      return;
    };
    let Some(hunks) = self.selected_diff_hunks.as_ref() else {
      editor.update(cx, |editor, cx| {
        editor.set_staged_ranges(Vec::new(), Vec::new(), cx);
      });
      return;
    };

    let dirty_ranges = if self.current_dirty {
      Some(self.buffer_dirty_current_ranges.as_slice())
    } else {
      None
    };
    let workdir_to_current = if self.current_dirty && !self.buffer_saved_to_current_hunks.is_empty()
    {
      Some(self.buffer_saved_to_current_hunks.as_slice())
    } else {
      None
    };

    let (staged_base, staged_current) = if let Some(selected) = self.selected_file {
      let entry = match selected {
        SelectedFile::Tracked(index) => self.tracked.get(index),
        SelectedFile::Untracked(index) => self.untracked.get(index),
      };
      if let Some(entry) = entry {
        let head_text = entry.head_content.as_deref().unwrap_or("");
        let index_text = entry.index_content.as_deref().unwrap_or("");
        let workdir_text = entry.workdir_content.as_deref().unwrap_or("");
        let head_lines = line_count_from_text(head_text);
        let index_lines = line_count_from_text(index_text);
        let workdir_lines = line_count_from_text(workdir_text);
        let max_lines = head_lines.max(index_lines).max(workdir_lines);
        if max_lines <= 20000 {
          let head_to_index = diff_line_hunks(head_text, index_text);
          let index_to_workdir = diff_line_hunks(index_text, workdir_text);
          staged_ranges_from_line_hunks(
            &head_to_index,
            &index_to_workdir,
            dirty_ranges,
            workdir_to_current,
          )
        } else {
          staged_ranges_from_hunks(hunks, dirty_ranges, workdir_to_current)
        }
      } else {
        staged_ranges_from_hunks(hunks, dirty_ranges, workdir_to_current)
      }
    } else {
      staged_ranges_from_hunks(hunks, dirty_ranges, workdir_to_current)
    };

    editor.update(cx, |editor, cx| {
      editor.set_staged_ranges(staged_base, staged_current, cx);
    });
  }

  fn sync_editor_diff_base(&mut self, previous_base: Option<&str>, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.as_ref() else {
      return;
    };
    let Some(selected) = self.selected_file else {
      return;
    };
    let new_base = match selected {
      SelectedFile::Tracked(index) => self.tracked.get(index).and_then(entry_diff_base),
      SelectedFile::Untracked(index) => self.untracked.get(index).and_then(entry_diff_base),
    };

    if previous_base == new_base {
      return;
    }

    editor.update(cx, |editor, cx| {
      editor.set_diff_base_text(new_base, cx);
    });
  }

  fn jump_to_next_change(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.as_ref() else {
      return;
    };
    editor.update(cx, |editor, cx| {
      editor.jump_to_change(ChangeDirection::Next, window, cx);
    });
  }

  fn jump_to_previous_change(
    &mut self,
    _: &ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.as_ref() else {
      return;
    };
    editor.update(cx, |editor, cx| {
      editor.jump_to_change(ChangeDirection::Previous, window, cx);
    });
  }

  fn open_repository_clicked(
    &mut self,
    _: &ClickEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.start_open_repository(cx);
  }

  fn open_repository_action(
    &mut self,
    _: &OpenRepository,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.start_open_repository(cx);
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      self.error = Some("Select a repository to use the command palette.".to_string());
      cx.notify();
      return;
    };

    let branches = match list_branches(&root_path) {
      Ok(branches) => branches,
      Err(err) => {
        self.error = Some(format!("Failed to list branches: {err}"));
        cx.notify();
        return;
      }
    };

    let palette_branches = branches
      .into_iter()
      .map(|branch| CommandPaletteBranch {
        name: branch.name.into(),
        kind: match branch.kind {
          BranchKind::Local => CommandPaletteBranchKind::Local,
          BranchKind::Remote => CommandPaletteBranchKind::Remote,
        },
      })
      .collect::<Vec<_>>();

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, _window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, cx)
      })
    });

    let palette = cx.new(|cx| {
      CommandPalette::new(
        window,
        cx,
        CommandPaletteConfig::new(palette_branches, handler),
      )
    });
    let palette_for_dialog = palette.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      dialog
        .p_0()
        .border_0()
        .min_h_0()
        .overlay_closable(true)
        .keyboard(true)
        .close_button(false)
        .keyboard(true)
        .child(palette_for_dialog.clone())
    });
  }

  fn start_open_repository(&mut self, cx: &mut Context<Self>) {
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
            let _ = this.update(cx, |view, cx| {
              view.set_root_path(path, cx);
            });
          }
        }
        Ok(None) => {}
        Err(err) => {
          let message = format!("Failed to open repository: {err}");
          let _ = this.update(cx, |view, cx| {
            view.error = Some(message);
            cx.notify();
          });
        }
      }
    })
    .detach();
  }

  fn repo_select_items(&self) -> SearchableVec<RepoSelectItem> {
    let items = self
      .recent_repositories
      .iter()
      .map(|repo| RepoSelectItem {
        label: SharedString::from(root_label(repo.path.as_path())),
        path: repo.path.clone(),
      })
      .collect::<Vec<_>>();

    SearchableVec::new(items)
  }

  fn on_repo_select_event(
    &mut self,
    _: &Entity<SelectState<SearchableVec<RepoSelectItem>>>,
    event: &SelectEvent<SearchableVec<RepoSelectItem>>,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let SelectEvent::Confirm(Some(path)) = event else {
      return;
    };
    self.set_root_path(path.clone(), cx);
  }

  fn start_file_polling(&mut self, cx: &mut Context<Self>) {
    if self.poll_task.is_some() {
      return;
    }

    let task = cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(FILE_POLL_INTERVAL_MS))
          .await;
        let _ = this.update(cx, |view, cx| {
          view.poll_current_file(cx);
          view.poll_repository_status(cx);
        });
      }
    });

    self.poll_task = Some(task);
  }

  fn poll_current_file(&mut self, cx: &mut Context<Self>) {
    let Some(selected_file) = self.selected_file else {
      if self.current_dirty {
        self.current_dirty = false;
        cx.notify();
      }
      return;
    };
    let Some(editor) = self.editor.clone() else {
      if self.current_dirty {
        self.current_dirty = false;
        cx.notify();
      }
      return;
    };
    let (is_tracked, entry_index) = match selected_file {
      SelectedFile::Tracked(selected_index) => (true, selected_index),
      SelectedFile::Untracked(selected_index) => (false, selected_index),
    };
    let current_text = editor_buffer_text(&editor, cx);
    let (saved_text, entry_path, entry_last_modified) = {
      let entry = if is_tracked {
        self.tracked.get(entry_index)
      } else {
        self.untracked.get(entry_index)
      };
      let Some(entry) = entry else {
        return;
      };
      (
        entry.saved_content.as_deref().unwrap_or("").to_string(),
        entry.path.clone(),
        entry.last_modified,
      )
    };
    let (is_dirty, mut needs_notify) =
      self.update_dirty_state_from_text(selected_file, &saved_text, &current_text, cx);

    if let Some(modified_time) = read_modified_time(&entry_path) {
      if entry_last_modified != Some(modified_time) && !is_dirty {
        if let Some(disk_text) = read_disk_text(&entry_path) {
          let entry = if is_tracked {
            self.tracked.get_mut(entry_index)
          } else {
            self.untracked.get_mut(entry_index)
          };
          if let Some(entry) = entry {
            entry.workdir_content = Some(disk_text.clone());
            entry.saved_content = Some(disk_text.clone());
            entry.last_modified = Some(modified_time);
          }
          reload_editor_content(&editor, &disk_text, cx);
          if self.current_dirty {
            self.current_dirty = false;
          }
          needs_notify = true;
        } else {
          let entry = if is_tracked {
            self.tracked.get_mut(entry_index)
          } else {
            self.untracked.get_mut(entry_index)
          };
          if let Some(entry) = entry {
            entry.last_modified = Some(modified_time);
          }
        }
      }
    }

    if needs_notify {
      cx.notify();
    }
  }

  fn poll_repository_status(&mut self, cx: &mut Context<Self>) {
    if self.root_path.is_none() {
      return;
    }

    let now = Instant::now();
    if let Some(last_poll) = self.last_repo_poll {
      if now.duration_since(last_poll) < Duration::from_millis(REPO_POLL_INTERVAL_MS) {
        return;
      }
    }

    self.last_repo_poll = Some(now);
    self.refresh_repository_statuses(cx);
  }

  fn refresh_repository_statuses(&mut self, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };

    let selected_entry = match self.selected_file {
      Some(SelectedFile::Tracked(idx)) => self.tracked.get(idx).cloned(),
      Some(SelectedFile::Untracked(idx)) => self.untracked.get(idx).cloned(),
      None => None,
    };
    let selected_path = selected_entry.as_ref().map(|entry| entry.path.clone());
    let selected_dirty = self.current_dirty;
    let previous_base = selected_entry
      .as_ref()
      .and_then(|entry| entry_diff_base(entry).map(|text| text.to_string()));

    let repository = match open_repository(&root_path) {
      Ok(repository) => repository,
      Err(err) => {
        self.error = Some(format!("Not a git repository: {err}"));
        return;
      }
    };

    let repo_root = repository.root;
    self.root_path = Some(repo_root.clone());
    self.has_head_commit = has_head_commit(&repo_root);
    self.can_undo_last_commit = can_undo_last_commit(&repo_root);
    match branch_status(&repo_root) {
      Ok(status) => {
        self.can_push = status.ahead > 0 && status.behind == 0;
        self.can_force_push = status.ahead > 0 && status.behind > 0;
        self.branch_status = Some(status);
      }
      Err(_) => {
        self.can_push = false;
        self.can_force_push = false;
        self.branch_status = None;
      }
    }
    let (mut tracked, mut untracked, has_staged_changes) =
      repository_entries_to_lists(&repo_root, repository.changes, repository.staged);
    self.has_staged_changes = has_staged_changes;

    if let Some(path) = selected_path.as_ref() {
      if let Some(index) = tracked.iter().position(|entry| entry.path == *path) {
        if let Some(existing) = selected_entry.as_ref() {
          let entry = &mut tracked[index];
          entry.workdir_content = existing.workdir_content.clone();
          entry.saved_content = existing.saved_content.clone();
          entry.last_modified = existing.last_modified;
        }
      } else if let Some(index) = untracked.iter().position(|entry| entry.path == *path) {
        if let Some(existing) = selected_entry.as_ref() {
          let entry = &mut untracked[index];
          entry.workdir_content = existing.workdir_content.clone();
          entry.saved_content = existing.saved_content.clone();
          entry.last_modified = existing.last_modified;
        }
      } else if selected_dirty {
        if let Some(mut entry) = selected_entry {
          entry.status = FileStatusKind::Modified;
          entry.stage_state = Some(StageState::Unstaged);
          tracked.push(entry);
        }
      } else {
        self.selected_file = None;
        self.editor = None;
        self.selected_diff_hunks = None;
        self.current_dirty = false;
        self.buffer_dirty_current_ranges.clear();
        self.buffer_saved_to_current_hunks.clear();
        self.saved_to_current_tracking = false;
        self.clear_editor_observer();
      }
    }

    tracked.sort_by(|a, b| a.path.cmp(&b.path));
    untracked.sort_by(|a, b| a.path.cmp(&b.path));
    let next_selected = selected_path.as_ref().and_then(|path| {
      tracked
        .iter()
        .position(|entry| entry.path == *path)
        .map(SelectedFile::Tracked)
        .or_else(|| {
          untracked
            .iter()
            .position(|entry| entry.path == *path)
            .map(SelectedFile::Untracked)
        })
    });

    self.tracked = tracked;
    self.untracked = untracked;
    self.selected_file = next_selected;
    self.error = None;
    self.update_selected_diff_hunks(cx);
    self.sync_editor_diff_base(previous_base.as_deref(), cx);
    cx.notify();
  }

  fn save_file_clicked(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    self.save_current_file(cx);
  }

  fn save_file_action(&mut self, _: &SaveFile, _window: &mut Window, cx: &mut Context<Self>) {
    self.save_current_file(cx);
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
    self.commit_changes_with_amend(false, window, cx);
  }

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(root_path) = self.root_path.clone() else {
      return Err("No repository selected.".into());
    };

    let result = match action {
      CommandPaletteAction::SwitchBranch(branch) => {
        let branch_ref = BranchRef {
          name: branch.name.to_string(),
          kind: match branch.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        switch_branch(&root_path, &branch_ref)
      }
      CommandPaletteAction::CreateBranch { name } => create_branch(&root_path, &name),
      CommandPaletteAction::CreateBranchFrom { name, base } => {
        let branch_ref = BranchRef {
          name: base.name.to_string(),
          kind: match base.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        create_branch_from(&root_path, &name, &branch_ref)
      }
    };

    if let Err(err) = result {
      let message: SharedString = format!("Failed to update branches: {err}").into();
      self.error = Some(message.to_string());
      cx.notify();
      return Err(message);
    }

    self.error = None;
    self.refresh_repository_statuses(cx);
    Ok(())
  }

  fn save_current_file(&mut self, cx: &mut Context<Self>) {
    let Some(selected_file) = self.selected_file else {
      return;
    };
    let Some(editor) = self.editor.as_ref() else {
      return;
    };
    let entry = match selected_file {
      SelectedFile::Tracked(selected_index) => self.tracked.get_mut(selected_index),
      SelectedFile::Untracked(selected_index) => self.untracked.get_mut(selected_index),
    };
    let Some(entry) = entry else {
      return;
    };

    let text = editor_buffer_text(editor, cx);
    if let Some(parent) = entry.path.parent() {
      if let Err(err) = fs::create_dir_all(parent) {
        eprintln!(
          "Failed to create directories for {}: {}",
          entry.display_name, err
        );
        return;
      }
    }
    if let Err(err) = fs::write(&entry.path, &text) {
      eprintln!("Failed to save {}: {}", entry.display_name, err);
      return;
    }

    entry.workdir_content = Some(text.clone());
    entry.saved_content = Some(text);
    entry.last_modified = read_modified_time(&entry.path);
    self.current_dirty = false;
    self.saved_to_current_tracking = false;
    self.refresh_repository_statuses(cx);
    cx.notify();
  }

  fn set_root_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    match open_repository(&path) {
      Ok(repository) => {
        let repo_root = repository.root;
        self.root_path = Some(repo_root.clone());
        ConfigStore::persist_recent_repository(&repo_root);
        bump_recent_repository(&mut self.recent_repositories, repo_root.clone());
        let (tracked, untracked, has_staged_changes) =
          repository_entries_to_lists(&repo_root, repository.changes, repository.staged);
        self.tracked = tracked;
        self.untracked = untracked;
        self.has_staged_changes = has_staged_changes;
        self.selected_file = None;
        self.editor = None;
        self.selected_diff_hunks = None;
        self.error = None;
        self.current_dirty = false;
        self.buffer_dirty_current_ranges.clear();
        self.buffer_saved_to_current_hunks.clear();
        self.saved_to_current_tracking = false;
        self.clear_editor_observer();
        self.last_repo_poll = Some(Instant::now());
        match branch_status(&repo_root) {
          Ok(status) => {
            self.can_push = status.ahead > 0 && status.behind == 0;
            self.can_force_push = status.ahead > 0 && status.behind > 0;
            self.branch_status = Some(status);
          }
          Err(_) => {
            self.can_push = false;
            self.can_force_push = false;
            self.branch_status = None;
          }
        }
      }
      Err(err) => {
        self.root_path = Some(path);
        self.tracked = Vec::new();
        self.untracked = Vec::new();
        self.selected_file = None;
        self.editor = None;
        self.selected_diff_hunks = None;
        self.error = Some(format!("Not a git repository: {err}"));
        self.current_dirty = false;
        self.buffer_dirty_current_ranges.clear();
        self.buffer_saved_to_current_hunks.clear();
        self.saved_to_current_tracking = false;
        self.clear_editor_observer();
        self.last_repo_poll = Some(Instant::now());
        self.has_staged_changes = false;
        self.can_push = false;
        self.can_force_push = false;
        self.branch_status = None;
      }
    }
    cx.notify();
  }

  fn select_file(
    &mut self,
    list: FileListKind,
    index: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let (content, base_content, file_path, display_name) = match list {
      FileListKind::Tracked => match self.tracked.get_mut(index) {
        Some(entry) => {
          refresh_entry_from_disk(entry);
          (
            entry.workdir_content.clone(),
            entry_diff_base(entry).map(|text| text.to_string()),
            entry.path.clone(),
            entry.display_name.clone(),
          )
        }
        None => return,
      },
      FileListKind::Untracked => match self.untracked.get_mut(index) {
        Some(entry) => {
          refresh_entry_from_disk(entry);
          (
            entry.workdir_content.clone(),
            entry_diff_base(entry).map(|text| text.to_string()),
            entry.path.clone(),
            entry.display_name.clone(),
          )
        }
        None => return,
      },
    };

    let Some(content) = content else {
      self.editor = None;
      self.clear_editor_observer();
      self.buffer_dirty_current_ranges.clear();
      self.buffer_saved_to_current_hunks.clear();
      self.saved_to_current_tracking = false;
      self.selected_file = Some(match list {
        FileListKind::Tracked => SelectedFile::Tracked(index),
        FileListKind::Untracked => SelectedFile::Untracked(index),
      });
      self.error = Some(format!("File content unavailable: {display_name}"));
      cx.notify();
      return;
    };

    let file_ext = file_path.extension().and_then(|ext| ext.to_str());
    let theme = self.theme.clone();
    let editor = cx.new(|cx| Editor::new(&content, base_content.as_deref(), file_ext, theme, cx));
    let mode = self.diff_view_mode;
    editor.update(cx, |editor, cx| {
      editor.set_diff_view_mode(mode, cx);
      editor.set_staged_ranges(Vec::new(), Vec::new(), cx);
    });
    let focus_handle = editor.read(cx).focus_handle(cx);

    match list {
      FileListKind::Tracked => {
        if let Some(entry) = self.tracked.get_mut(index) {
          if entry.saved_content.is_none() {
            entry.saved_content = Some(content.clone());
          }
          entry.last_modified = read_modified_time(&entry.path);
        }
      }
      FileListKind::Untracked => {
        if let Some(entry) = self.untracked.get_mut(index) {
          if entry.saved_content.is_none() {
            entry.saved_content = Some(content.clone());
          }
          entry.last_modified = read_modified_time(&entry.path);
        }
      }
    }

    self.editor = Some(editor.clone());
    self.attach_editor_observer(&editor, cx);
    self.selected_file = Some(match list {
      FileListKind::Tracked => SelectedFile::Tracked(index),
      FileListKind::Untracked => SelectedFile::Untracked(index),
    });
    self.update_selected_diff_hunks(cx);
    self.error = None;
    self.current_dirty = false;
    self.buffer_dirty_current_ranges.clear();
    self.buffer_saved_to_current_hunks.clear();
    self.saved_to_current_tracking = false;

    window.focus(&focus_handle, cx);
    cx.notify();
  }

  fn render_empty_state(&mut self, cx: &mut Context<Self>) -> Div {
    let theme = cx.theme().clone();
    let (message, color, show_hint) = if let Some(error) = &self.error {
      (error.clone(), theme.red, false)
    } else {
      (
        "Open a repository to get started.".to_string(),
        theme.foreground,
        true,
      )
    };

    div()
      .key_context("Workspace")
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(Self::open_repository_action))
      .on_action(cx.listener(Self::save_file_action))
      .size_full()
      .bg(theme.background)
      .text_color(color)
      .flex()
      .flex_col()
      .items_center()
      .justify_center()
      .gap_2()
      .child(message)
      .when(show_hint, |this| {
        this.child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Press Cmd+O to open a repository."),
        )
      })
      .child(
        Button::new("open-folder-empty")
          .label("Open Repository")
          .on_click(cx.listener(Self::open_repository_clicked)),
      )
  }

  fn render_app_header(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
    let theme = cx.theme().clone();
    let repo_select = self.render_repo_select(window, cx);

    let header_row = div()
      .h(px(APP_HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.title_bar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .child(div().text_sm().text_color(theme.foreground).child("Reviu"))
          .child(repo_select)
          .child(
            Button::new("open-repo")
              .icon(IconName::FolderOpen)
              .ghost()
              .compact()
              .tooltip("Open Repository")
              .on_click(cx.listener(Self::open_repository_clicked)),
          ),
      )
      .child(
        Button::new("open-settings")
          .icon(IconName::Settings)
          .ghost()
          .compact()
          .tooltip("Settings")
          .on_click(|_, _, cx| {
            WorkspaceRoute::global_mut(cx).page = WorkspacePage::Settings;
            cx.refresh_windows();
          }),
      );

    header_row
  }

  fn render_repo_select(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let items = self.repo_select_items();
    let sidebar_width = self
      .sidebar_state
      .read(cx)
      .sizes()
      .first()
      .copied()
      .unwrap_or(SIDEBAR_DEFAULT_WIDTH);
    let repo_select = match self.repo_select.clone() {
      Some(state) => state,
      None => {
        let state = cx.new(|cx| SelectState::new(items.clone(), None, window, cx).searchable(true));
        cx.subscribe_in(&state, window, Self::on_repo_select_event)
          .detach();
        self.repo_select = Some(state.clone());
        state
      }
    };

    repo_select.update(cx, |state, cx| {
      state.set_items(items, window, cx);
      match self.root_path.clone() {
        Some(root_path) => state.set_selected_value(&root_path, window, cx),
        None => state.set_selected_index(None, window, cx),
      }
    });

    Select::new(&repo_select)
      .placeholder("Select Repository")
      .search_placeholder("Search repositories")
      .icon(IconName::Folder)
      .menu_width(sidebar_width)
      .empty(
        div()
          .px_2()
          .py_2()
          .text_sm()
          .child("No recent repositories."),
      )
      .w(px(220.0))
  }

  fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let view = cx.entity();
    let dirty_path = self.selected_dirty_path();
    let mut entries = Vec::with_capacity(self.tracked.len() + self.untracked.len());
    entries.extend(
      self
        .tracked
        .iter()
        .enumerate()
        .map(|(idx, entry)| SidebarFileEntry::new(FileListKind::Tracked, idx, entry)),
    );
    entries.extend(
      self
        .untracked
        .iter()
        .enumerate()
        .map(|(idx, entry)| SidebarFileEntry::new(FileListKind::Untracked, idx, entry)),
    );
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let selected_path = match self.selected_file {
      Some(SelectedFile::Tracked(idx)) => self.tracked.get(idx).map(|entry| entry.path.clone()),
      Some(SelectedFile::Untracked(idx)) => self.untracked.get(idx).map(|entry| entry.path.clone()),
      None => None,
    };

    let list_section: WorkspaceSidebarSection = WorkspaceSidebarSection::new(
      view,
      entries,
      selected_path,
      self.root_path.is_some(),
      dirty_path,
    );
    let header = render_sidebar_header(self.root_path.is_none(), &theme, cx);
    let commit_bar = self.render_commit_bar(cx);

    let sidebar = Sidebar::new("workspace-sidebar")
      .w_full()
      .flex_1()
      .bg(theme.sidebar)
      .border_0()
      .text_color(theme.sidebar_foreground)
      .child(list_section);

    div()
      .w_full()
      .flex()
      .flex_col()
      .bg(theme.sidebar)
      .child(header)
      .child(sidebar)
      .child(commit_bar)
  }

  fn selected_dirty_path(&self) -> Option<PathBuf> {
    if !self.current_dirty {
      return None;
    }
    match self.selected_file {
      Some(SelectedFile::Tracked(index)) => self.tracked.get(index).map(|entry| entry.path.clone()),
      Some(SelectedFile::Untracked(index)) => {
        self.untracked.get(index).map(|entry| entry.path.clone())
      }
      None => None,
    }
  }

  fn render_commit_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let input = self.commit_input.clone();

    div()
      .flex()
      .p_2()
      .relative()
      .bg(theme.sidebar)
      .border_t_1()
      .border_color(theme.sidebar_border)
      .w_full()
      .child(
        div()
          .w_full()
          .flex()
          .flex_col()
          .gap_2()
          .child(div().w_full().child(Input::new(&input)))
          .child(self.render_commit_button(cx)),
      )
  }

  fn render_commit_button(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let commit_enabled = self.root_path.is_some();
    let amend_enabled = self.root_path.is_some() && self.has_head_commit;
    let undo_enabled = self.root_path.is_some() && self.can_undo_last_commit;
    let push_enabled = self.root_path.is_some() && self.can_push;
    let force_push_enabled = self.root_path.is_some() && self.can_force_push;
    let menu_enabled = amend_enabled || undo_enabled || push_enabled || force_push_enabled;
    let view = cx.entity();
    let amend_view = view.clone();
    let undo_view = view.clone();
    let push_view = view.clone();
    let force_push_view = view.clone();

    let main_button = Button::new("commit-button-main")
      .label("Commit")
      .with_variant(ButtonVariant::Secondary)
      .outline()
      .flex_1()
      .rounded_r_none()
      .child(Kbd::new(Keystroke::parse("cmd-enter").unwrap()).ml_1())
      .disabled(!commit_enabled)
      .on_click(cx.listener(Self::commit_changes));

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
                this.commit_amend_changes(event, window, cx);
              });
            }),
        );

        let menu = menu.item(
          PopupMenuItem::new("Undo last commit")
            .icon(IconName::Undo)
            .disabled(!undo_enabled)
            .on_click(move |event, window, cx| {
              undo_view.update(cx, |this, cx| {
                this.undo_last_commit(event, window, cx);
              });
            }),
        );

        let menu = menu.separator();

        let menu = menu.item(
          PopupMenuItem::new("Push")
            .icon(IconName::ArrowUp)
            .disabled(!push_enabled)
            .on_click(move |event, window, cx| {
              push_view.update(cx, |this, cx| {
                this.push_changes(event, window, cx);
              });
            }),
        );

        menu.item(
          PopupMenuItem::new("Force push (with lease)")
            .icon(IconName::TriangleAlert)
            .disabled(!force_push_enabled)
            .on_click(move |event, window, cx| {
              force_push_view.update(cx, |this, cx| {
                this.force_push_changes(event, window, cx);
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

  fn render_editor_header(&mut self, cx: &mut Context<Self>) -> Div {
    let theme = cx.theme().clone();
    let title = self
      .selected_file
      .and_then(|selected| match selected {
        SelectedFile::Tracked(idx) => self.tracked.get(idx),
        SelectedFile::Untracked(idx) => self.untracked.get(idx),
      })
      .map(|entry| entry.display_name.clone())
      .unwrap_or_else(|| "File".to_string());

    let mut title_row = div()
      .flex()
      .items_center()
      .gap_1()
      .child(div().text_sm().text_color(theme.foreground).child(title));

    if self.current_dirty {
      title_row = title_row.child(
        div()
          .flex_none()
          .size(px(6.0))
          .rounded_full()
          .ml_1()
          .bg(theme.primary),
      );
    }

    let diff_label = match self.diff_view_mode {
      DiffViewMode::Inline => "Split Diff",
      DiffViewMode::Split => "Inline Diff",
    };
    let change_position = self
      .editor
      .as_ref()
      .and_then(|editor| editor.read(cx).change_position(cx));

    let actions = div()
      .flex()
      .items_center()
      .gap_2()
      .when(self.current_dirty, |this| {
        this.child(
          Button::new("save-file")
            .label("Save")
            .small()
            .primary()
            .on_click(cx.listener(Self::save_file_clicked)),
        )
      })
      .child(
        Button::new("prev-change")
          .label("Prev Change")
          .small()
          .on_click(cx.listener(Self::jump_to_previous_change)),
      )
      .child(
        Button::new("next-change")
          .label("Next Change")
          .small()
          .on_click(cx.listener(Self::jump_to_next_change)),
      )
      .when_some(change_position, |this, (current, total)| {
        this.child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("{}/{}", current, total)),
        )
      })
      .child(
        Button::new("diff-toggle")
          .label(diff_label)
          .small()
          .on_click(cx.listener(Self::toggle_diff_view)),
      );

    div()
      .h_10()
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .border_b_1()
      .border_color(theme.border)
      .bg(theme.tab_bar)
      .child(title_row)
      .child(actions)
  }

  fn render_editor_with_overlay(
    &mut self,
    editor: Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Div {
    let overlay = self.render_change_block_actions(&editor, window, cx);
    div()
      .flex_1()
      .min_w(px(0.0))
      .min_h(px(0.0))
      .relative()
      .child(editor)
      .when_some(overlay, |this, overlay| this.child(overlay))
  }

  fn render_change_block_actions(
    &mut self,
    editor: &Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<Div> {
    let selected = self.selected_file?;
    if matches!(selected, SelectedFile::Untracked(_)) {
      return None;
    }
    let editor_state = editor.read(cx);
    let hovered_range = editor_state.hovered_change_range()?;
    let stage_state = self.stage_state_for_range(&hovered_range, cx)?;
    let line_idx = hovered_range.start;
    let row_idx = editor_state.row_for_line(line_idx, cx)?;
    let viewport_start = editor_state.scroll_offset_y.floor() as usize;
    if row_idx < viewport_start {
      return None;
    }

    let line_height = window.line_height();
    let top = line_height * (row_idx - viewport_start) as f32 + px(3.0);
    let file_dirty = self.current_dirty;
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
    } else {
      "Restore hunk"
    };

    let view = cx.entity();
    let stage_range = hovered_range.clone();
    let restore_range = hovered_range.clone();
    let unstage_range = hovered_range.clone();

    let actions = ButtonGroup::new("change-block-actions")
      .primary()
      .xsmall()
      .rounded_md()
      .disabled(file_dirty)
      .when(
        matches!(
          stage_state,
          StageState::Unstaged | StageState::PartiallyStaged
        ),
        |this| {
          this.child(
            Button::new("stage-hunk")
              .icon(IconName::Plus)
              .label("Stage")
              .tooltip(stage_tooltip)
              .disabled(file_dirty)
              .on_click(window.listener_for(
                &view,
                move |this: &mut GitPage, _: &ClickEvent, _window, cx| {
                  if file_dirty {
                    return;
                  }
                  this.stage_change_block(stage_range.clone(), cx);
                },
              )),
          )
        },
      )
      .when(
        matches!(
          stage_state,
          StageState::Staged | StageState::PartiallyStaged
        ),
        |this| {
          this.child(
            Button::new("unstage-hunk")
              .icon(IconName::Minus)
              .label("Unstage")
              .tooltip(unstage_tooltip)
              .disabled(file_dirty)
              .on_click(window.listener_for(
                &view,
                move |this: &mut GitPage, _: &ClickEvent, _window, cx| {
                  if file_dirty {
                    return;
                  }
                  this.unstage_change_block(unstage_range.clone(), cx);
                },
              )),
          )
        },
      )
      .when(
        matches!(
          stage_state,
          StageState::Unstaged | StageState::PartiallyStaged
        ),
        |this| {
          this.child(
            Button::new("restore-hunk")
              .icon(IconName::Undo)
              .label("Restore")
              .tooltip(restore_tooltip)
              .disabled(file_dirty)
              .on_click(window.listener_for(
                &view,
                move |this: &mut GitPage, _: &ClickEvent, _window, cx| {
                  if file_dirty {
                    return;
                  }
                  this.restore_change_block(restore_range.clone(), cx);
                },
              )),
          )
        },
      );

    Some(div().absolute().top(top).right(px(30.0)).child(actions))
  }

  fn render_footer(&self, cx: &mut Context<Self>) -> Div {
    if self.root_path.is_none() {
      return div();
    }

    let theme = cx.theme().clone();
    let (branch_name, ahead, behind) = match self.branch_status.as_ref() {
      Some(status) => (status.name.as_str(), status.ahead, status.behind),
      None => ("No branch", 0, 0),
    };

    let branch = div()
      .text_sm()
      .text_color(theme.foreground)
      .child(branch_name.to_string());

    let pull = div()
      .flex()
      .items_center()
      .child(IconName::ArrowDown)
      .child(format!("{behind}"));

    let push = div()
      .flex()
      .items_center()
      .child(IconName::ArrowUp)
      .child(format!("{ahead}"));

    div()
      .px_3()
      .py_1()
      .flex()
      .items_center()
      .gap_2()
      .border_t_1()
      .border_color(theme.border)
      .bg(theme.tab_bar)
      .text_xs()
      .text_color(theme.muted_foreground)
      .child(branch)
      .child(div().flex().items_center().gap_1().child(pull).child(push))
  }
}

impl Render for GitPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.sync_theme_from_app(cx);
    let theme = cx.theme().clone();
    let show_empty_only = self.root_path.is_none() && self.recent_repositories.is_empty();

    let content = if show_empty_only {
      div().flex_1().child(self.render_empty_state(cx))
    } else {
      let main = if self.root_path.is_none() {
        div()
          .flex_1()
          .flex()
          .flex_col()
          .min_w(px(0.0))
          .size_full()
          .bg(theme.background)
          .child(
            div()
              .flex()
              .items_center()
              .justify_center()
              .size_full()
              .text_color(theme.muted_foreground)
              .child("Select a repository to get started."),
          )
      } else {
        let mut main = div()
          .flex_1()
          .flex()
          .flex_col()
          .min_w(px(0.0))
          .min_h(px(0.0))
          .size_full()
          .bg(theme.background);

        if let Some(editor) = self.editor.clone() {
          let header = self.render_editor_header(cx);
          let editor_with_overlay = self.render_editor_with_overlay(editor, window, cx);
          main = main.child(header).child(editor_with_overlay);
        } else {
          let (message, color) = if let Some(error) = &self.error {
            (error.clone(), theme.red)
          } else if self.tracked.is_empty() && self.untracked.is_empty() {
            (
              "No changes found in this repository.".to_string(),
              theme.muted_foreground,
            )
          } else {
            (
              "Select a file to view it.".to_string(),
              theme.muted_foreground,
            )
          };

          main = main.child(
            div()
              .flex()
              .items_center()
              .justify_center()
              .size_full()
              .text_color(color)
              .child(message),
          );
        }

        main
      };

      div().flex_1().min_h(px(0.0)).bg(theme.background).child(
        h_resizable("workspace-layout")
          .with_state(&self.sidebar_state)
          .child(
            resizable_panel()
              .size(SIDEBAR_DEFAULT_WIDTH)
              .size_range(SIDEBAR_MIN_WIDTH..SIDEBAR_MAX_WIDTH)
              .child(self.render_sidebar(cx)),
          )
          .child(resizable_panel().child(main)),
      )
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .relative()
      .bg(theme.background)
      .child(self.render_app_header(window, cx))
      .child(content)
      .child(self.render_footer(cx))
      .key_context("Workspace")
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(Self::open_repository_action))
      .on_action(cx.listener(Self::save_file_action))
      .on_action(cx.listener(Self::commit_changes_action))
      .on_action(cx.listener(Self::show_command_palette_action))
  }
}

impl Focusable for GitPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

fn render_sidebar_list(
  view: &Entity<GitPage>,
  entries: &[SidebarFileEntry],
  selected_path: Option<&Path>,
  has_root: bool,
  dirty_path: Option<&Path>,
  theme: &ComponentTheme,
  window: &mut Window,
) -> Div {
  let entries_len = entries.len();
  let mut items = div().flex().flex_col();
  for (idx, entry) in entries.iter().enumerate() {
    let is_selected = selected_path.map_or(false, |path| path == entry.path);
    items = items.child(render_sidebar_row(
      view,
      idx,
      entry,
      is_selected,
      dirty_path,
      theme,
      window,
    ));
  }

  let empty_message = if has_root {
    "No changes."
  } else {
    "No repository selected."
  };

  let body = if entries_len == 0 {
    div()
      .flex_1()
      .flex()
      .items_center()
      .justify_center()
      .pb_2()
      .text_sm()
      .text_color(theme.muted_foreground)
      .child(empty_message)
  } else {
    div().flex().flex_col().child(items)
  };

  div().flex().flex_col().min_h(px(0.0)).child(body)
}

fn render_sidebar_header(disabled: bool, theme: &ComponentTheme, cx: &mut Context<GitPage>) -> Div {
  div()
    .h_10()
    .px_3()
    .flex()
    .items_center()
    .justify_between()
    .border_b_1()
    .border_color(theme.border)
    .bg(theme.tab_bar)
    .child(div())
    .child(
      div()
        .flex()
        .items_center()
        .gap_2()
        .child(
          Button::new("stage-all")
            .label("Stage All")
            .small()
            .compact()
            .disabled(disabled)
            .on_click(cx.listener(GitPage::stage_all_files)),
        )
        .child(
          Button::new("unstage-all")
            .label("Unstage All")
            .small()
            .compact()
            .disabled(disabled)
            .on_click(cx.listener(GitPage::unstage_all_files)),
        ),
    )
}

fn render_sidebar_row(
  view: &Entity<GitPage>,
  idx: usize,
  entry: &SidebarFileEntry,
  is_selected: bool,
  dirty_path: Option<&Path>,
  theme: &ComponentTheme,
  window: &mut Window,
) -> Stateful<Div> {
  let (tag, tag_color) = status_tag(entry.status, theme);
  let status_tooltip = status_tooltip(entry.status);
  let row_group = format!(
    "file-row-{}-{}",
    match entry.list {
      FileListKind::Tracked => "tracked",
      FileListKind::Untracked => "untracked",
    },
    entry.list_index
  );
  let list = entry.list;
  let list_index = entry.list_index;
  let status_id = format!("file-status-{}", row_group);
  let stage_id = format!("file-stage-{}", row_group);
  let is_dirty = dirty_path.map_or(false, |path| path == entry.path);
  let stage_state_display = match (entry.stage_state, entry.status) {
    (Some(stage_state), _) => Some(stage_state),
    (None, FileStatusKind::Untracked) => Some(StageState::Unstaged),
    (None, _) => None,
  };
  let (stage_icon, stage_color) = match stage_state_display {
    Some(StageState::Staged) => (Some(IconName::CircleCheck), theme.green),
    Some(StageState::PartiallyStaged) => (Some(IconName::CircleCheck), theme.yellow),
    Some(StageState::Unstaged) => (Some(IconName::Dash), theme.muted_foreground),
    None => (None, theme.muted_foreground),
  };
  let stage_tooltip = stage_state_display.map(stage_state_tooltip);

  let actions_bg = theme.sidebar_accent;

  let mut actions_wrap = ButtonGroup::new(format!("file-actions-{}", row_group))
    .ghost()
    .compact()
    .xsmall()
    .bg(actions_bg)
    .disabled(is_dirty)
    .rounded_md();

  let mut actions = div()
    .absolute()
    .right_1()
    .top_0()
    .bottom_0()
    .flex()
    .items_center()
    .opacity(0.0)
    .group_hover(row_group.clone(), |style| style.opacity(1.0));

  match entry.list {
    FileListKind::Tracked => {
      let path = entry.path.clone();
      let restore_path = entry.path.clone();
      let status = entry.status;
      let has_staged = matches!(
        entry.stage_state,
        Some(StageState::Staged | StageState::PartiallyStaged)
      );
      let has_unstaged = matches!(
        entry.stage_state,
        Some(StageState::Unstaged | StageState::PartiallyStaged)
      );
      let can_restore = matches!(entry.stage_state, Some(StageState::Unstaged));
      let stage_tooltip = if is_dirty {
        "File not saved"
      } else {
        "Stage file"
      };
      let unstage_tooltip = "Unstage file";
      let restore_tooltip = if is_dirty {
        "File not saved"
      } else {
        "Restore file"
      };
      if has_unstaged {
        actions_wrap = actions_wrap.child(
          Button::new(format!("stage-file-{}", &row_group))
            .icon(IconName::Plus)
            .tooltip(stage_tooltip)
            .disabled(is_dirty)
            .on_click(
              window.listener_for(view, move |this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                if is_dirty {
                  return;
                }
                this.stage_file(path.clone(), cx);
              }),
            ),
        );
      }
      if has_staged {
        let path = entry.path.clone();
        actions_wrap = actions_wrap.child(
          Button::new(format!("unstage-file-{}", &row_group))
            .icon(IconName::Minus)
            .tooltip(unstage_tooltip)
            .disabled(is_dirty)
            .on_click(
              window.listener_for(view, move |this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                if is_dirty {
                  return;
                }
                this.unstage_file(path.clone(), cx);
              }),
            ),
        );
      }
      if can_restore {
        let view_for_dialog = view.clone();
        let file_label = entry.display_name.clone();

        actions_wrap = actions_wrap.child(
          Button::new(format!("restore-file-{}", &row_group))
            .icon(IconName::Undo)
            .tooltip(restore_tooltip)
            .disabled(is_dirty)
            .on_click(
              window.listener_for(view, move |_, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                if is_dirty {
                  return;
                }
                let restore_path = restore_path.clone();
                let confirm_message: SharedString = format!(
                  "Restore changes to \"{}\"? This will discard all unstaged changes.",
                  file_label
                )
                .into();
                let view = view_for_dialog.clone();
                window.open_dialog(cx, move |dialog, _, _| {
                  let confirm_message = confirm_message.clone();
                  let view = view.clone();
                  let restore_path = restore_path.clone();
                  ConfirmDialog::new("Restore file?", confirm_message)
                    .confirm_text("Restore")
                    .destructive()
                    .on_confirm(move |_, _, cx| {
                      view.update(cx, |this, cx| {
                        this.restore_file_change(restore_path.clone(), status, cx);
                      });
                      true
                    })
                    .build(dialog)
                });
              }),
            ),
        );
      }
    }
    FileListKind::Untracked => {
      let path = entry.path.clone();
      let restore_path = entry.path.clone();
      let view_for_dialog = view.clone();
      let file_label = entry.display_name.clone();
      let stage_tooltip = if is_dirty {
        "File not saved"
      } else {
        "Stage file"
      };
      let restore_tooltip = if is_dirty {
        "File not saved"
      } else {
        "Delete file"
      };
      actions_wrap = actions_wrap
        .child(
          Button::new(format!("stage-file-{}", &row_group))
            .icon(IconName::Plus)
            .tooltip(stage_tooltip)
            .disabled(is_dirty)
            .on_click(
              window.listener_for(view, move |this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                if is_dirty {
                  return;
                }
                this.stage_file(path.clone(), cx);
              }),
            ),
        )
        .child(
          Button::new(format!("restore-file-{}", &row_group))
            .icon(IconName::Undo)
            .tooltip(restore_tooltip)
            .disabled(is_dirty)
            .on_click(
              window.listener_for(view, move |_, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                if is_dirty {
                  return;
                }

                let restore_path = restore_path.clone();
                let confirm_message: SharedString = format!(
                  "Delete untracked file \"{}\"? This will remove it from disk.",
                  file_label
                )
                .into();
                let view = view_for_dialog.clone();

                window.open_dialog(cx, move |dialog, _, _| {
                  let confirm_message = confirm_message.clone();
                  let view = view.clone();
                  let restore_path = restore_path.clone();
                  ConfirmDialog::new("Delete file?", confirm_message)
                    .confirm_text("Delete")
                    .destructive()
                    .on_confirm(move |_, _, cx| {
                      view.update(cx, |this, cx| {
                        this.restore_file_change(
                          restore_path.clone(),
                          FileStatusKind::Untracked,
                          cx,
                        );
                      });
                      true
                    })
                    .build(dialog)
                });
              }),
            ),
        );
    }
  }
  actions = actions.child(actions_wrap);

  div()
    .id(idx)
    .px_2()
    .py_1()
    .w_full()
    .text_sm()
    .text_color(theme.sidebar_foreground)
    .cursor_pointer()
    .on_click(
      window.listener_for(view, move |this, _: &ClickEvent, window, cx| {
        if is_selected {
          return;
        }
        this.select_file(list, list_index, window, cx);
      }),
    )
    .relative()
    .group(row_group.clone())
    .rounded_md()
    .flex()
    .items_center()
    .gap_2()
    .when_else(
      is_selected,
      |this| {
        this
          .bg(theme.list_active)
          .text_color(theme.sidebar_foreground)
      },
      |this| this.hover(|style| style.bg(theme.list_hover)),
    )
    .child(
      div()
        .id(status_id)
        .flex_none()
        .text_sm()
        .w_3()
        .text_color(tag_color)
        .child(tag)
        .tooltip(move |window, cx| Tooltip::new(status_tooltip).build(window, cx)),
    )
    .when_some(stage_icon, |this, icon| {
      this.child(
        div()
          .id(stage_id)
          .flex_none()
          .text_sm()
          .text_color(stage_color)
          .child(icon)
          .when_some(stage_tooltip, |this, tooltip| {
            this.tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
          }),
      )
    })
    .child(
      div()
        .flex_1()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis_start()
        .child(entry.display_name.clone()),
    )
    .child(actions)
}

fn editor_buffer_text(editor: &Entity<Editor>, cx: &App) -> String {
  let document = editor.read(cx).document().read(cx);
  let len = document.buffer.len();
  document.buffer.slice_to_string(0..len)
}

fn read_modified_time(path: &Path) -> Option<SystemTime> {
  fs::metadata(path)
    .ok()
    .and_then(|metadata| metadata.modified().ok())
}

fn read_disk_text(path: &Path) -> Option<String> {
  let bytes = fs::read(path).ok()?;
  Some(String::from_utf8_lossy(&bytes).to_string())
}

fn reload_editor_content(editor: &Entity<Editor>, text: &str, cx: &mut App) {
  editor.update(cx, |editor, cx| {
    editor.reload_from_disk(text, cx);
  });
}

fn refresh_entry_from_disk(entry: &mut FileEntry) {
  if let Some(modified_time) = read_modified_time(&entry.path) {
    if entry.last_modified != Some(modified_time) {
      if let Some(disk_text) = read_disk_text(&entry.path) {
        entry.workdir_content = Some(disk_text.clone());
        entry.saved_content = Some(disk_text);
        entry.last_modified = Some(modified_time);
      } else {
        entry.last_modified = Some(modified_time);
      }
    }
  }
}

fn root_label(path: &Path) -> String {
  path
    .file_name()
    .and_then(|name| name.to_str())
    .map(|name| name.to_string())
    .unwrap_or_else(|| path.display().to_string())
}

fn repository_entries_to_lists(
  repo_root: &Path,
  changes: Vec<RepositoryFile>,
  staged: Vec<RepositoryFile>,
) -> (Vec<FileEntry>, Vec<FileEntry>, bool) {
  let has_staged_changes = !staged.is_empty();
  let mut changes_map = HashMap::new();
  let mut staged_map = HashMap::new();

  for entry in changes {
    changes_map.insert(entry.path.clone(), entry);
  }
  for entry in staged {
    staged_map.insert(entry.path.clone(), entry);
  }

  let mut paths: Vec<PathBuf> = changes_map
    .keys()
    .chain(staged_map.keys())
    .cloned()
    .collect();
  paths.sort();
  paths.dedup();

  let mut tracked = Vec::new();
  let mut untracked = Vec::new();

  for path in paths {
    let change = changes_map.remove(&path);
    let staged_entry = staged_map.remove(&path);

    let is_untracked_only = change
      .as_ref()
      .map(|entry| entry.status == FileStatusKind::Untracked)
      .unwrap_or(false)
      && staged_entry.is_none();

    if is_untracked_only {
      let change = change.expect("untracked entry should exist");
      let abs_path = repo_root.join(&change.path);
      let workdir_content = change.workdir_content;
      let saved_content = workdir_content.clone();
      let last_modified = read_modified_time(&abs_path);
      untracked.push(FileEntry {
        path: abs_path,
        display_name: change.path.to_string_lossy().to_string(),
        status: change.status,
        head_content: change.head_content,
        index_content: change.index_content,
        workdir_content,
        saved_content,
        last_modified,
        stage_state: None,
      });
      continue;
    }

    let has_staged = staged_entry.is_some();
    let has_unstaged = change.is_some();
    let stage_state = match (has_staged, has_unstaged) {
      (true, true) => StageState::PartiallyStaged,
      (true, false) => StageState::Staged,
      (false, true) => StageState::Unstaged,
      (false, false) => StageState::Unstaged,
    };

    let status = match (change.as_ref(), staged_entry.as_ref()) {
      (Some(entry), _) if entry.status != FileStatusKind::Untracked => entry.status,
      (_, Some(entry)) => entry.status,
      (Some(entry), None) => entry.status,
      (None, None) => FileStatusKind::Modified,
    };

    let head_content = staged_entry
      .as_ref()
      .and_then(|entry| entry.head_content.clone())
      .or_else(|| change.as_ref().and_then(|entry| entry.head_content.clone()));
    let index_content = staged_entry
      .as_ref()
      .and_then(|entry| entry.index_content.clone())
      .or_else(|| {
        change
          .as_ref()
          .and_then(|entry| entry.index_content.clone())
      })
      .or_else(|| head_content.clone());
    let workdir_content = change
      .as_ref()
      .and_then(|entry| entry.workdir_content.clone())
      .or_else(|| {
        staged_entry
          .as_ref()
          .and_then(|entry| entry.workdir_content.clone())
      })
      .or_else(|| index_content.clone());

    let display_name = path.to_string_lossy().to_string();
    let abs_path = repo_root.join(&path);
    let last_modified = read_modified_time(&abs_path);

    tracked.push(FileEntry {
      path: abs_path,
      display_name,
      status,
      head_content,
      index_content,
      workdir_content: workdir_content.clone(),
      saved_content: workdir_content,
      last_modified,
      stage_state: Some(stage_state),
    });
  }

  tracked.sort_by(|a, b| a.path.cmp(&b.path));
  untracked.sort_by(|a, b| a.path.cmp(&b.path));

  (tracked, untracked, has_staged_changes)
}

fn bump_recent_repository(repositories: &mut Vec<RecentRepository>, path: PathBuf) {
  if let Some(index) = repositories.iter().position(|repo| repo.path == path) {
    let repo = repositories.remove(index);
    repositories.insert(0, repo);
  } else {
    repositories.insert(0, RecentRepository { path });
  }
}

fn status_tag(status: FileStatusKind, theme: &ComponentTheme) -> (&'static str, Hsla) {
  match status {
    FileStatusKind::Added => ("A", theme.green),
    FileStatusKind::Untracked => ("U", theme.green),
    FileStatusKind::Modified => ("M", theme.yellow),
    FileStatusKind::Deleted => ("D", theme.red),
    FileStatusKind::Renamed => ("R", theme.yellow),
    FileStatusKind::Typechange => ("T", theme.yellow),
    FileStatusKind::Conflicted => ("C", theme.red),
  }
}

fn status_tooltip(status: FileStatusKind) -> &'static str {
  match status {
    FileStatusKind::Added => "Added",
    FileStatusKind::Untracked => "Untracked",
    FileStatusKind::Modified => "Modified",
    FileStatusKind::Deleted => "Deleted",
    FileStatusKind::Renamed => "Renamed",
    FileStatusKind::Typechange => "Type changed",
    FileStatusKind::Conflicted => "Conflicted",
  }
}

fn stage_state_tooltip(stage_state: StageState) -> &'static str {
  match stage_state {
    StageState::Staged => "Staged",
    StageState::PartiallyStaged => "Partially staged",
    StageState::Unstaged => "Unstaged",
  }
}

fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
  left.start < right.end && right.start < left.end
}

fn range_overlaps_any(range: &Range<usize>, ranges: &[Range<usize>]) -> bool {
  ranges
    .iter()
    .any(|candidate| ranges_overlap(range, candidate))
}

fn find_hunk_by_old_range<'a>(
  hunks: &'a [DiffHunkInfo],
  range: &Range<usize>,
) -> Option<&'a DiffHunkInfo> {
  hunks.iter().find(|hunk| {
    let mut matches = false;
    if !hunk.old_changed.is_empty() {
      matches |= range_overlaps_any(range, &hunk.old_changed);
    }
    if hunk.old_lines > 0 {
      matches |= ranges_overlap(range, &(hunk.old_start..(hunk.old_start + hunk.old_lines)));
    }
    matches
  })
}

fn find_hunk_by_new_range<'a>(
  hunks: &'a [DiffHunkInfo],
  range: &Range<usize>,
) -> Option<&'a DiffHunkInfo> {
  hunks.iter().find(|hunk| {
    let mut matches = false;
    if !hunk.new_changed.is_empty() {
      matches |= range_overlaps_any(range, &hunk.new_changed);
    }
    if hunk.new_lines > 0 {
      matches |= ranges_overlap(range, &(hunk.new_start..(hunk.new_start + hunk.new_lines)));
    }
    matches
  })
}

fn find_line_hunk_by_old_range<'a>(
  hunks: &'a [LineDiffHunk],
  range: &Range<usize>,
) -> Option<&'a LineDiffHunk> {
  hunks.iter().find(|hunk| {
    if hunk.old_lines == 0 {
      return false;
    }
    ranges_overlap(range, &(hunk.old_start..(hunk.old_start + hunk.old_lines)))
  })
}

fn find_line_hunk_by_new_range<'a>(
  hunks: &'a [LineDiffHunk],
  range: &Range<usize>,
) -> Option<&'a LineDiffHunk> {
  hunks.iter().find(|hunk| {
    if hunk.new_lines == 0 {
      return false;
    }
    ranges_overlap(range, &(hunk.new_start..(hunk.new_start + hunk.new_lines)))
  })
}

fn push_unique_line_hunk(targets: &mut Vec<LineDiffHunk>, hunk: &LineDiffHunk) {
  if !targets.iter().any(|existing| {
    existing.old_start == hunk.old_start
      && existing.old_lines == hunk.old_lines
      && existing.new_start == hunk.new_start
      && existing.new_lines == hunk.new_lines
  }) {
    targets.push(hunk.clone());
  }
}

fn hunk_header_range(hunk: &DiffHunkInfo) -> HunkRange {
  let base = (hunk.old_lines > 0).then(|| hunk.old_start..(hunk.old_start + hunk.old_lines));
  let current = (hunk.new_lines > 0).then(|| hunk.new_start..(hunk.new_start + hunk.new_lines));
  HunkRange { base, current }
}

fn text_for_line_range(text: &str, range: &Range<usize>) -> String {
  if range.start >= range.end {
    return String::new();
  }

  let mut line_starts = Vec::new();
  line_starts.push(0);
  let mut char_index = 0usize;
  for ch in text.chars() {
    char_index += 1;
    if ch == '\n' {
      line_starts.push(char_index);
    }
  }
  let len_chars = char_index;
  let line_count = line_starts.len();
  let start_line = range.start.min(line_count);
  let end_line = range.end.min(line_count);
  let start_char = *line_starts.get(start_line).unwrap_or(&len_chars);
  let end_char = if end_line < line_count {
    line_starts[end_line]
  } else {
    len_chars
  };

  text
    .chars()
    .skip(start_char)
    .take(end_char.saturating_sub(start_char))
    .collect()
}

fn line_count_from_text(text: &str) -> usize {
  let mut count = 1usize;
  for ch in text.chars() {
    if ch == '\n' {
      count += 1;
    }
  }
  count
}

fn splice_range_for_hunk(start: usize, lines: usize, max_len: usize) -> (usize, usize) {
  let mut start = start;
  if start > max_len {
    start = max_len;
  }
  let mut end = start.saturating_add(lines);
  if end > max_len {
    end = max_len;
  }
  (start, end)
}

fn split_text_lines(text: &str) -> Vec<String> {
  let mut lines = text
    .split('\n')
    .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
    .collect::<Vec<_>>();
  if lines.is_empty() {
    lines.push(String::new());
  }
  lines
}

fn invert_line_hunks(hunks: &[LineDiffHunk]) -> Vec<LineDiffHunk> {
  let mut inverted = hunks
    .iter()
    .map(|hunk| LineDiffHunk {
      old_start: hunk.new_start,
      old_lines: hunk.new_lines,
      new_start: hunk.old_start,
      new_lines: hunk.old_lines,
    })
    .collect::<Vec<_>>();
  inverted.sort_by_key(|hunk| hunk.old_start);
  inverted
}

fn line_hunk_overlaps_window(hunk: &LineDiffHunk, window: &Range<usize>) -> bool {
  if hunk.old_lines == 0 {
    return hunk.old_start >= window.start && hunk.old_start < window.end;
  }
  let old_end = hunk.old_start + hunk.old_lines;
  old_end > window.start && hunk.old_start < window.end
}

fn invert_hunks(hunks: &[DiffHunkInfo]) -> Vec<DiffHunkInfo> {
  let mut inverted = hunks
    .iter()
    .map(|hunk| DiffHunkInfo {
      old_start: hunk.new_start,
      old_lines: hunk.new_lines,
      new_start: hunk.old_start,
      new_lines: hunk.old_lines,
      old_changed: hunk.new_changed.clone(),
      new_changed: hunk.old_changed.clone(),
    })
    .collect::<Vec<_>>();
  inverted.sort_by_key(|hunk| hunk.old_start);
  inverted
}

trait LineMapHunk {
  fn old_start(&self) -> usize;
  fn old_lines(&self) -> usize;
  fn new_start(&self) -> usize;
  fn new_lines(&self) -> usize;
}

impl LineMapHunk for DiffHunkInfo {
  fn old_start(&self) -> usize {
    self.old_start
  }

  fn old_lines(&self) -> usize {
    self.old_lines
  }

  fn new_start(&self) -> usize {
    self.new_start
  }

  fn new_lines(&self) -> usize {
    self.new_lines
  }
}

impl LineMapHunk for LineDiffHunk {
  fn old_start(&self) -> usize {
    self.old_start
  }

  fn old_lines(&self) -> usize {
    self.old_lines
  }

  fn new_start(&self) -> usize {
    self.new_start
  }

  fn new_lines(&self) -> usize {
    self.new_lines
  }
}

fn map_line_index<H: LineMapHunk>(line: usize, hunks: &[H]) -> usize {
  let mut delta: isize = 0;
  for hunk in hunks {
    let old_start = hunk.old_start();
    let old_end = old_start + hunk.old_lines();
    if line < old_start {
      break;
    }
    if line < old_end {
      if hunk.new_lines() == 0 {
        return hunk.new_start();
      }
      let offset = line - old_start;
      let mapped = hunk.new_start() + offset.min(hunk.new_lines().saturating_sub(1));
      return mapped;
    }
    if hunk.old_lines() == 0 && line == old_start && old_start > 0 {
      return (line as isize + delta).max(0) as usize;
    }
    delta += hunk.new_lines() as isize - hunk.old_lines() as isize;
  }
  (line as isize + delta).max(0) as usize
}

fn map_line_range<H: LineMapHunk>(range: &Range<usize>, hunks: &[H]) -> Range<usize> {
  if range.is_empty() {
    return range.clone();
  }
  let start = map_line_index(range.start, hunks);
  let end = map_line_index(range.end, hunks);
  if end < start {
    start..start
  } else {
    start..end
  }
}

fn map_index_range_to_current_ranges<H: LineMapHunk>(
  range: &Range<usize>,
  hunks: &[H],
) -> Vec<Range<usize>> {
  if range.is_empty() {
    return Vec::new();
  }
  let mut mapped = Vec::new();
  let mut current_start: Option<usize> = None;
  let mut last_mapped: usize = 0;

  for line in range.clone() {
    let mapped_line = map_line_index(line, hunks);
    if let Some(start) = current_start {
      if mapped_line == last_mapped + 1 {
        last_mapped = mapped_line;
      } else {
        mapped.push(start..(last_mapped + 1));
        current_start = Some(mapped_line);
        last_mapped = mapped_line;
      }
    } else {
      current_start = Some(mapped_line);
      last_mapped = mapped_line;
    }
  }

  if let Some(start) = current_start {
    mapped.push(start..(last_mapped + 1));
  }

  mapped
}

fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
  if ranges.is_empty() {
    return ranges;
  }
  ranges.sort_by_key(|range| range.start);
  let mut merged = Vec::new();
  let mut current = ranges.remove(0);
  for range in ranges {
    if range.start <= current.end {
      current.end = current.end.max(range.end);
    } else {
      merged.push(current);
      current = range;
    }
  }
  merged.push(current);
  merged
}

fn staged_ranges_from_hunks(
  hunks: &FileDiffHunks,
  dirty_current_ranges: Option<&[Range<usize>]>,
  workdir_to_current_hunks: Option<&[LineDiffHunk]>,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
  let mut staged_base = Vec::new();
  let mut staged_current = Vec::new();

  let mut unstaged_index = Vec::new();
  let mut unstaged_current = Vec::new();
  for hunk in &hunks.index_to_workdir {
    if hunk.old_lines > 0 {
      unstaged_index.push(hunk.old_start..(hunk.old_start + hunk.old_lines));
    }
    if hunk.new_lines > 0 {
      unstaged_current.push(hunk.new_start..(hunk.new_start + hunk.new_lines));
    }
  }
  let unstaged_index = merge_ranges(unstaged_index);
  let unstaged_current = merge_ranges(unstaged_current);

  let dirty_ranges = dirty_current_ranges.unwrap_or(&[]);

  for hunk in &hunks.head_to_index {
    let base_range =
      (hunk.old_lines > 0).then(|| hunk.old_start..(hunk.old_start + hunk.old_lines));
    let index_range =
      (hunk.new_lines > 0).then(|| hunk.new_start..(hunk.new_start + hunk.new_lines));
    let mut current_ranges = if let Some(range) = index_range.as_ref() {
      map_index_range_to_current_ranges(range, &hunks.index_to_workdir)
    } else {
      Vec::new()
    };
    if let Some(workdir_to_current_hunks) = workdir_to_current_hunks {
      current_ranges = current_ranges
        .into_iter()
        .map(|range| map_line_range(&range, workdir_to_current_hunks))
        .collect();
    }

    let overlaps_index = index_range
      .as_ref()
      .map(|range| range_overlaps_any(range, &unstaged_index))
      .unwrap_or(false);
    let overlaps_current = current_ranges
      .iter()
      .any(|range| range_overlaps_any(range, &unstaged_current));
    let overlaps_dirty = current_ranges
      .iter()
      .any(|range| range_overlaps_any(range, dirty_ranges));
    let should_stage = !overlaps_index && !overlaps_current && !overlaps_dirty;

    if !should_stage {
      continue;
    }

    if let Some(range) = base_range {
      staged_base.push(range);
    }
    for range in current_ranges {
      staged_current.push(range);
    }
  }

  let staged_base = merge_ranges(staged_base);
  let staged_current = merge_ranges(staged_current);
  (staged_base, staged_current)
}

fn staged_ranges_from_line_hunks(
  head_to_index: &[LineDiffHunk],
  index_to_workdir: &[LineDiffHunk],
  dirty_current_ranges: Option<&[Range<usize>]>,
  workdir_to_current_hunks: Option<&[LineDiffHunk]>,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
  let mut staged_base = Vec::new();
  let mut staged_current = Vec::new();

  let mut unstaged_index = Vec::new();
  let mut unstaged_current = Vec::new();
  for hunk in index_to_workdir {
    if hunk.old_lines > 0 {
      unstaged_index.push(hunk.old_start..(hunk.old_start + hunk.old_lines));
    }
    if hunk.new_lines > 0 {
      unstaged_current.push(hunk.new_start..(hunk.new_start + hunk.new_lines));
    }
  }
  let unstaged_index = merge_ranges(unstaged_index);
  let unstaged_current = merge_ranges(unstaged_current);

  let dirty_ranges = dirty_current_ranges.unwrap_or(&[]);

  for hunk in head_to_index {
    let base_range =
      (hunk.old_lines > 0).then(|| hunk.old_start..(hunk.old_start + hunk.old_lines));
    let index_range =
      (hunk.new_lines > 0).then(|| hunk.new_start..(hunk.new_start + hunk.new_lines));
    let mut current_ranges = if let Some(range) = index_range.as_ref() {
      map_index_range_to_current_ranges(range, index_to_workdir)
    } else {
      Vec::new()
    };
    if let Some(workdir_to_current_hunks) = workdir_to_current_hunks {
      current_ranges = current_ranges
        .into_iter()
        .map(|range| map_line_range(&range, workdir_to_current_hunks))
        .collect();
    }

    let overlaps_index = index_range
      .as_ref()
      .map(|range| range_overlaps_any(range, &unstaged_index))
      .unwrap_or(false);
    let overlaps_current = current_ranges
      .iter()
      .any(|range| range_overlaps_any(range, &unstaged_current));
    let overlaps_dirty = current_ranges
      .iter()
      .any(|range| range_overlaps_any(range, dirty_ranges));
    let should_stage = !overlaps_index && !overlaps_current && !overlaps_dirty;

    if !should_stage {
      continue;
    }

    if let Some(range) = base_range {
      staged_base.push(range);
    }
    for range in current_ranges {
      staged_current.push(range);
    }
  }

  let staged_base = merge_ranges(staged_base);
  let staged_current = merge_ranges(staged_current);
  (staged_base, staged_current)
}
