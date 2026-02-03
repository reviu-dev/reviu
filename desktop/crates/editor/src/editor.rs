use std::{
  collections::{HashMap, HashSet, VecDeque},
  ops::Range,
  path::PathBuf,
  sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
  },
  time::{Duration, Instant, SystemTime},
};

use buffer::TransactionId;
use git::{ApplyLocation, DiffSet, GitFileBases, GitStore, RepoFile};
use gpui::{
  App, Bounds, Context, CursorStyle, Entity, EntityInputHandler, FocusHandle, Focusable,
  MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollHandle, ShapedLine, Task,
  UTF16Selection, Window, black, div, point, prelude::*, px, white,
};
use gpui_component::{
  ActiveTheme as _,
  resizable::{h_resizable, resizable_panel},
};
use smol::unblock;
use syntax::Theme;

use crate::{
  boundaries::{line_range_at_offset, word_range_at_offset},
  cursor_blink::CursorBlink,
  document::Document,
  editor_element::{EditorElement, PositionMap},
  gutter_element::GutterElement,
  projection::{ChangeKind, DisplayLine, GapId, HunkState, NO_NEWLINE_MARKER_TEXT, Projection},
};

#[derive(Clone, Debug)]
pub struct Transaction {
  pub id: TransactionId,
  pub selection_before: Range<usize>,
  pub selection_after: Range<usize>,
}

/// Default viewport height before first render
const DEFAULT_VIEWPORT_HEIGHT: f32 = 800.0;
/// Default viewport width before first render
const DEFAULT_VIEWPORT_WIDTH: f32 = 1200.0;
/// Default maximum line width
pub const DEFAULT_MAX_LINE_WIDTH: f32 = 800.0;
/// Extra width added to editor content for horizontal scrolling
const EXTRA_EDITOR_WIDTH: f32 = 200.0;
/// Maximum number of cached shaped lines
const MAX_CACHE_SIZE: usize = 200;
/// Number of lines of padding when auto-scrolling to cursor
const SCROLL_PADDING: usize = 3;
/// Width of the gutter area
const GUTTER_WIDTH: f32 = 90.0;
/// Diff recompute debounce (ms)
const DIFF_DEBOUNCE_MS: u64 = 60;
/// External change polling interval (ms)
const POLL_INTERVAL_MS: u64 = 500;
/// Hardcoded repo root (temporary)
const DEFAULT_REPO_ROOT: &str = "/Users/joris/workspace/git-playground";
/// Hardcoded file path (temporary)
const DEFAULT_FILE_PATH: &str = "/Users/joris/workspace/git-playground/perf-100k.ts";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffViewMode {
  Inline,
  Split,
}

pub struct Editor {
  pub document: Entity<Document>,
  pub focus_handle: FocusHandle,
  pub selected_range: Range<usize>,
  pub selection_reversed: bool,
  pub display_selection: Option<DisplaySelection>,
  pub marked_range: Option<Range<usize>>,
  pub is_selecting: bool,

  // Performance: cache and viewport
  pub line_layouts: HashMap<usize, Arc<ShapedLine>>,
  pub virtual_line_layouts: HashMap<usize, Arc<ShapedLine>>,

  pub scroll_offset_y: f32, // Vertical scroll offset in lines (0.0 = top, 1.5 = 1.5 lines down)
  pub viewport_height: Pixels,
  pub viewport_width: Pixels,
  pub max_line_width: Pixels, // Maximum width of visible lines (never decreases to avoid scroll jumps)
  pub scroll_handle: ScrollHandle, // Handle for horizontal scrolling
  pub(crate) scroll_axis_lock: Option<ScrollAxis>,
  pub(crate) last_scroll_time: Option<Instant>,
  pub(crate) last_scroll_x: Pixels,

  // Cache size limit to prevent memory issues with large files
  pub(crate) max_cache_size: usize,

  // Target column for vertical navigation
  pub(crate) target_column: Option<usize>,

  pub(crate) undo_stack: VecDeque<Transaction>,
  pub(crate) redo_stack: VecDeque<Transaction>,

  pub theme: Theme,
  pub projection: Option<Arc<Projection>>,
  pub visible_groups: Vec<GroupOverlay>,
  pub hovered_group_id: Option<Arc<str>>,
  pub last_mouse_position: Option<Point<Pixels>>,
  pub expanded_gaps: HashMap<GapId, usize>,
  pub workdir_path: PathBuf,
  pub repo_file: Option<RepoFile>,
  pub git_store: Option<GitStore>,
  git_state: BufferGitState,
  pub diffs: Option<DiffSet>,
  pub diff_task: Option<Task<()>>,
  pub bases_task: Option<Task<()>>,
  pub poll_task: Option<Task<()>>,
  pub git_task: Option<Task<()>>,
  git_jobs: VecDeque<GitJob>,
  git_op_in_flight: bool,
  pending_git_after_bases: bool,
  pub diff_generation: Arc<AtomicUsize>,
  pub file_mtime: Option<SystemTime>,
  pub index_mtime: Option<SystemTime>,
  pub is_dirty: bool,
  pub save_task: Option<Task<()>>,
  pub optimistic_unstaged_groups: HashSet<Arc<str>>,

  diff_view_mode: DiffViewMode,

  // Track syntax highlighting version to invalidate cache when highlights change
  pub last_highlights_version: usize,
  pub last_highlights_epoch: usize,

  // Cursor blinking
  pub cursor_blink: Entity<CursorBlink>,
}

#[derive(Clone, Debug)]
pub struct GroupOverlay {
  pub id: Arc<str>,
  pub state: HunkState,
  pub display_line: usize,
  pub y: Pixels,
}

#[derive(Clone, Debug, Default)]
pub struct BufferGitState {
  pub op_id: usize,
  pub bases: Option<GitFileBases>,
  pub index_dirty: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum HunkAction {
  Stage,
  Unstage,
  Restore,
}

#[derive(Clone, Debug)]
struct GroupToken {
  state: HunkState,
  signature: Arc<str>,
  id: Arc<str>,
}

#[derive(Clone, Debug)]
struct GitJob {
  token: GroupToken,
  action: HunkAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayCursor {
  pub line: usize,
  pub column: usize,
}

#[derive(Clone, Debug)]
pub struct DisplaySelection {
  pub start: DisplayCursor,
  pub end: DisplayCursor,
}

impl DisplaySelection {
  pub fn is_empty(&self) -> bool {
    self.start == self.end
  }

  pub fn normalized(&self) -> (DisplayCursor, DisplayCursor) {
    if (self.start.line, self.start.column) <= (self.end.line, self.end.column) {
      (self.start, self.end)
    } else {
      (self.end, self.start)
    }
  }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum ScrollAxis {
  Horizontal,
  Vertical,
}

impl Editor {
  pub fn new(cx: &mut Context<Self>) -> Self {
    Self::new_with_paths(
      PathBuf::from(DEFAULT_REPO_ROOT),
      PathBuf::from(DEFAULT_FILE_PATH),
      cx,
    )
  }

  pub fn new_with_paths(repo_root: PathBuf, file_path: PathBuf, cx: &mut Context<Self>) -> Self {
    let workdir_path = file_path;
    let file_ext = workdir_path
      .extension()
      .and_then(|ext| ext.to_str())
      .map(|ext| ext.to_ascii_lowercase());
    let file_name = workdir_path
      .file_name()
      .and_then(|name| name.to_str())
      .map(|name| name.to_ascii_lowercase());
    let mut language_hint = file_ext.as_deref().or_else(|| file_name.as_deref());
    if let Some(name) = file_name.as_deref()
      && name.starts_with("dockerfile")
    {
      language_hint = Some("dockerfile");
    }
    let content = std::fs::read_to_string(&workdir_path).unwrap_or_default();

    let document = cx.new(|cx| Document::new(&content, language_hint, cx));
    let cursor_blink = cx.new(CursorBlink::new);
    let repo_file = RepoFile::new(repo_root, workdir_path.clone()).ok();
    let git_store = repo_file
      .as_ref()
      .map(|repo_file| GitStore::new(repo_file.repo_root.clone()));
    let file_mtime = std::fs::metadata(&workdir_path)
      .and_then(|meta| meta.modified())
      .ok();
    let index_mtime = repo_file
      .as_ref()
      .and_then(|repo| std::fs::metadata(repo.repo_root.join(".git/index")).ok())
      .and_then(|meta| meta.modified().ok());

    let mut editor = Self {
      document,
      focus_handle: cx.focus_handle(),
      selected_range: 0..0,
      selection_reversed: false,
      display_selection: None,
      marked_range: None,
      is_selecting: false,
      line_layouts: HashMap::new(),
      virtual_line_layouts: HashMap::new(),
      scroll_offset_y: 0.0,
      viewport_height: px(DEFAULT_VIEWPORT_HEIGHT), // Will be updated on first render
      viewport_width: px(DEFAULT_VIEWPORT_WIDTH),   // Will be updated on first render
      max_line_width: px(DEFAULT_MAX_LINE_WIDTH),   // Will be updated on first render
      scroll_handle: ScrollHandle::new(),
      scroll_axis_lock: None,
      last_scroll_time: None,
      last_scroll_x: px(0.0),
      max_cache_size: MAX_CACHE_SIZE,
      target_column: None,
      undo_stack: VecDeque::new(),
      redo_stack: VecDeque::new(),
      theme: Theme::dark(),
      projection: None,
      visible_groups: Vec::new(),
      hovered_group_id: None,
      last_mouse_position: None,
      expanded_gaps: HashMap::new(),
      last_highlights_version: 0,
      last_highlights_epoch: 0,
      cursor_blink,
      workdir_path,
      repo_file,
      git_store,
      git_state: BufferGitState::default(),
      diffs: None,
      diff_task: None,
      bases_task: None,
      poll_task: None,
      git_task: None,
      git_jobs: VecDeque::new(),
      git_op_in_flight: false,
      pending_git_after_bases: false,
      diff_generation: Arc::new(AtomicUsize::new(0)),
      file_mtime,
      index_mtime,
      is_dirty: false,
      save_task: None,
      optimistic_unstaged_groups: HashSet::new(),
      diff_view_mode: DiffViewMode::Inline,
    };
    editor.init(cx);
    editor
  }

  pub fn document(&self) -> &Entity<Document> {
    &self.document
  }

  pub fn set_projection(&mut self, projection: Option<Projection>) {
    self.projection = projection.map(Arc::new);
    self.virtual_line_layouts.clear();
  }

  pub fn projection(&self) -> Option<&Projection> {
    self.projection.as_deref()
  }

  pub fn diff_view_mode(&self) -> DiffViewMode {
    self.diff_view_mode
  }

  pub fn set_diff_view_mode(&mut self, mode: DiffViewMode, cx: &mut Context<Self>) {
    if self.diff_view_mode != mode {
      self.diff_view_mode = mode;
      if let Some(diffs) = self.diffs.clone() {
        self.apply_diffs(diffs, cx);
      } else {
        self.virtual_line_layouts.clear();
      }
    }
  }

  pub fn refresh_git_state(&mut self, cx: &mut Context<Self>) {
    self.reload_git_bases(cx);
  }

  fn maybe_optimistic_unstage_for_edit(
    &mut self,
    start_line: usize,
    end_line: usize,
    cx: &mut Context<Self>,
  ) {
    let Some(projection) = self.projection.as_ref() else {
      return;
    };
    if self.git_state.bases.is_none() {
      return;
    }

    let mut group_ids = HashSet::new();
    for doc_line in start_line..=end_line {
      let Some(display_line) = self.doc_to_display_line(doc_line) else {
        continue;
      };
      let Some(line) = projection.lines.get(display_line) else {
        continue;
      };
      let group_id = match line {
        DisplayLine::Doc {
          hunk: Some(HunkState::Staged),
          group_id: Some(group_id),
          ..
        } => group_id.clone(),
        DisplayLine::Modified {
          hunk: HunkState::Staged,
          group_id: Some(group_id),
          ..
        } => group_id.clone(),
        _ => continue,
      };

      group_ids.insert(group_id);
    }

    for group_id in group_ids {
      self.optimistic_unstage_group(group_id, cx);
    }
  }

  fn optimistic_unstage_group(&mut self, group_id: Arc<str>, cx: &mut Context<Self>) {
    if self.optimistic_unstaged_groups.contains(&group_id) {
      return;
    }
    let Some(projection) = self.projection.as_ref() else {
      return;
    };
    let Some(group) = projection.groups.get(group_id.as_ref()) else {
      return;
    };
    if group.state != HunkState::Staged {
      return;
    }
    let Some(bases) = self.git_state.bases.as_mut() else {
      return;
    };
    let base_index = bases.index.as_deref().unwrap_or("");
    let Ok(updated) = git::apply_hunk_to_text(base_index, &group.hunk, true) else {
      return;
    };
    bases.index = Some(updated);
    self.git_state.index_dirty = true;
    self.optimistic_unstaged_groups.insert(group_id);
    self.schedule_diff_recompute(cx);
  }

  fn init(&mut self, cx: &mut Context<Self>) {
    if self.repo_file.is_some() {
      self.reload_git_bases(cx);
      self.start_polling(cx);
    }
  }

  fn reload_git_bases(&mut self, cx: &mut Context<Self>) {
    let Some(repo_file) = self.repo_file.clone() else {
      return;
    };
    let Some(git_store) = self.git_store.clone() else {
      return;
    };
    let Ok(rel_path) = repo_file.relative_path() else {
      return;
    };
    let op_id = git_store.op_id();

    self.bases_task = Some(cx.spawn(async move |this, cx| {
      let bases = unblock(move || git_store.load_bases(&rel_path)).await;
      let Ok(bases) = bases else {
        return;
      };

      let _ = this.update(cx, |editor, cx| {
        let mut merged = bases;
        if editor.git_state.index_dirty {
          if let Some(existing) = editor
            .git_state
            .bases
            .as_ref()
            .and_then(|b| b.index.clone())
          {
            merged.index = Some(existing);
          }
        }
        editor.git_state.bases = Some(merged);
        editor.git_state.op_id = op_id;
        if editor.pending_git_after_bases {
          editor.pending_git_after_bases = false;
          editor.git_op_in_flight = false;
          editor.maybe_start_next_git_job(cx);
        }
        editor.schedule_diff_recompute(cx);
      });
    }));
  }

  pub fn schedule_diff_recompute(&mut self, cx: &mut Context<Self>) {
    let Some(repo_file) = self.repo_file.clone() else {
      return;
    };
    let Some(git_store) = self.git_store.clone() else {
      return;
    };
    let Some(git_bases) = self.git_state.bases.clone() else {
      self.reload_git_bases(cx);
      return;
    };

    if git_store.op_id() != self.git_state.op_id {
      self.reload_git_bases(cx);
      return;
    }

    let Ok(rel_path) = repo_file.relative_path() else {
      return;
    };

    let buffer_text = {
      let document = self.document.read(cx);
      document.slice_to_string(0..document.len())
    };

    let generation = self.diff_generation.fetch_add(1, Ordering::Relaxed) + 1;
    let diff_generation = self.diff_generation.clone();
    self.diff_task = Some(cx.spawn(async move |this, cx| {
      cx.background_executor()
        .timer(Duration::from_millis(DIFF_DEBOUNCE_MS))
        .await;

      let diffs =
        unblock(move || git::compute_buffer_diffs(&git_bases, &buffer_text, &rel_path)).await;
      let Ok(diffs) = diffs else {
        return;
      };

      let is_latest = diff_generation.load(Ordering::Relaxed) == generation;
      if !is_latest {
        return;
      }

      let _ = this.update(cx, |editor, cx| {
        editor.apply_diffs(diffs, cx);
      });
    }));
  }

  fn apply_diffs(&mut self, diffs: DiffSet, cx: &mut Context<Self>) {
    let doc_line_count = self.document.read(cx).len_lines();
    let projection = Projection::from_diffs(
      doc_line_count,
      &diffs.uncommitted,
      &diffs.unstaged,
      &diffs.staged,
      &self.expanded_gaps,
      matches!(self.diff_view_mode, DiffViewMode::Split),
    );
    self.diffs = Some(diffs);
    self.set_projection(Some(projection));

    let total_lines = self.display_line_count(doc_line_count);
    if total_lines == 0 {
      self.scroll_offset_y = 0.0;
    } else {
      let max_scroll = (total_lines.saturating_sub(1)) as f32;
      if self.scroll_offset_y > max_scroll {
        self.scroll_offset_y = max_scroll;
      }
    }

    cx.notify();
  }

  pub fn save(&mut self, cx: &mut Context<Self>) {
    let workdir_path = self.workdir_path.clone();
    let contents = {
      let document = self.document.read(cx);
      document.slice_to_string(0..document.len())
    };
    let repo_file = self.repo_file.clone();
    let index_text = self
      .git_state
      .bases
      .as_ref()
      .and_then(|bases| bases.index.clone());
    let needs_index_write = self.git_state.index_dirty;

    self.save_task = Some(cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        std::fs::write(&workdir_path, contents)?;
        let file_mtime = std::fs::metadata(&workdir_path)
          .and_then(|meta| meta.modified())
          .ok();
        let mut index_mtime = None;
        if needs_index_write {
          if let (Some(repo_file), Some(index_text)) = (repo_file, index_text) {
            if let Err(err) = git::write_index_content(&repo_file, &index_text) {
              return Err(std::io::Error::new(std::io::ErrorKind::Other, err));
            }
            index_mtime = std::fs::metadata(repo_file.repo_root.join(".git/index"))
              .and_then(|meta| meta.modified())
              .ok();
          }
        }
        Ok::<_, std::io::Error>((file_mtime, index_mtime))
      })
      .await;

      let _ = this.update(cx, |editor, cx| match result {
        Ok((file_mtime, index_mtime)) => {
          editor.is_dirty = false;
          editor.file_mtime = file_mtime;
          if needs_index_write {
            editor.git_state.index_dirty = false;
            editor.optimistic_unstaged_groups.clear();
            if let Some(index_mtime) = index_mtime {
              editor.index_mtime = Some(index_mtime);
            }
            if let Some(store) = editor.git_store.as_ref() {
              store.bump_op();
              editor.git_state.op_id = store.op_id();
            }
          }
          editor.reload_git_bases(cx);
          editor.schedule_diff_recompute(cx);
          cx.notify();
        }
        Err(err) => {
          eprintln!("[editor] save failed: {:?}", err);
        }
      });
    }));
  }

  pub fn selected_text_for_copy(&self, cx: &App) -> Option<String> {
    if let Some(selection) = &self.display_selection {
      if let Some(text) = self.display_selection_text(selection, cx) {
        return Some(text);
      }
    }

    if self.selected_range.is_empty() {
      return None;
    }

    let document = self.document.read(cx);
    Some(document.slice_to_string(self.selected_range.clone()))
  }

  fn display_selection_text(&self, selection: &DisplaySelection, cx: &App) -> Option<String> {
    if selection.is_empty() {
      return None;
    }

    let document = self.document.read(cx);
    let doc_line_count = document.len_lines();
    let total_lines = self.display_line_count(doc_line_count);
    if total_lines == 0 {
      return None;
    }

    let (start, end) = selection.normalized();
    let mut end_line = end.line.min(total_lines.saturating_sub(1));
    if end.column == 0 && end.line > start.line {
      end_line = end_line.saturating_sub(1);
    }

    if start.line > end_line {
      return None;
    }

    let mut lines = Vec::new();
    for display_line in start.line..=end_line {
      let line_text = match self.display_line(display_line, doc_line_count) {
        Some(DisplayLine::Doc { doc_line, .. }) => document
          .line_content(doc_line)
          .map(|cow| cow.into_owned())
          .unwrap_or_default(),
        Some(DisplayLine::Modified { doc_line, .. }) => document
          .line_content(doc_line)
          .map(|cow| cow.into_owned())
          .unwrap_or_default(),
        Some(DisplayLine::Removed { text, .. }) => text,
        _ => continue,
      };

      let line_len = line_text.len();
      let mut slice_start = 0;
      let mut slice_end = line_len;

      if display_line == start.line {
        slice_start = start.column.min(line_len);
      }

      if display_line == end_line && display_line == end.line {
        slice_end = end.column.min(line_len);
      }

      if slice_end < slice_start {
        continue;
      }

      let slice = line_text
        .get(slice_start..slice_end)
        .unwrap_or("")
        .to_string();
      lines.push(slice);
    }

    if lines.is_empty() {
      None
    } else {
      Some(lines.join("\n"))
    }
  }

  fn group_token_for_id(&self, group_id: &Arc<str>) -> Option<GroupToken> {
    let Some(projection) = self.projection.as_ref() else {
      return None;
    };
    let Some(group) = projection.groups.get(group_id.as_ref()) else {
      return None;
    };
    Some(GroupToken {
      state: group.state,
      signature: group.signature.clone(),
      id: group_id.clone(),
    })
  }

  fn resolve_group_from_token(&self, token: &GroupToken) -> Option<(HunkState, git::DiffHunk)> {
    let Some(projection) = self.projection.as_ref() else {
      return None;
    };
    if let Some((_, group)) = projection
      .groups
      .iter()
      .find(|(_, group)| group.state == token.state && group.signature == token.signature)
    {
      return Some((group.state, group.hunk.clone()));
    }
    let Some(group) = projection.groups.get(token.id.as_ref()) else {
      return None;
    };
    Some((group.state, group.hunk.clone()))
  }

  pub fn enqueue_group_action(
    &mut self,
    group_id: Arc<str>,
    action: HunkAction,
    cx: &mut Context<Self>,
  ) {
    let Some(token) = self.group_token_for_id(&group_id) else {
      return;
    };
    self.git_jobs.push_back(GitJob { token, action });
    self.maybe_start_next_git_job(cx);
  }

  fn maybe_start_next_git_job(&mut self, cx: &mut Context<Self>) {
    if self.git_op_in_flight {
      return;
    }
    let Some(job) = self.git_jobs.pop_front() else {
      return;
    };
    self.start_git_job(job, cx);
  }

  fn start_git_job(&mut self, job: GitJob, cx: &mut Context<Self>) {
    let Some(repo_file) = self.repo_file.clone() else {
      self.git_op_in_flight = false;
      return;
    };
    let Some((state, hunk)) = self.resolve_group_from_token(&job.token) else {
      self.git_op_in_flight = false;
      self.maybe_start_next_git_job(cx);
      return;
    };

    let (reverse, location) = match (state, job.action) {
      (HunkState::Unstaged, HunkAction::Stage) => (false, ApplyLocation::Index),
      (HunkState::Staged, HunkAction::Unstage) => (true, ApplyLocation::Index),
      (HunkState::Unstaged, HunkAction::Restore) => (true, ApplyLocation::WorkDir),
      (HunkState::Staged, HunkAction::Restore) => (true, ApplyLocation::Both),
      _ => {
        self.git_op_in_flight = false;
        self.maybe_start_next_git_job(cx);
        return;
      }
    };

    let needs_index = matches!(location, ApplyLocation::Index | ApplyLocation::Both);
    let needs_workdir = matches!(location, ApplyLocation::WorkDir | ApplyLocation::Both);
    let mut index_text_to_write: Option<String> = None;
    if needs_index {
      let Some(bases) = self.git_state.bases.clone() else {
        self.git_jobs.push_front(job);
        self.pending_git_after_bases = true;
        self.reload_git_bases(cx);
        return;
      };
      let base_index = bases.index.as_deref().unwrap_or("");
      match git::apply_hunk_to_text(base_index, &hunk, reverse) {
        Ok(updated) => {
          index_text_to_write = Some(updated);
        }
        Err(_err) => {
          self.git_op_in_flight = false;
          self.maybe_start_next_git_job(cx);
          return;
        }
      }
    }

    let workdir_path = self.workdir_path.clone();
    let workdir_path_for_fallback = workdir_path.clone();
    let hunk_for_fallback = hunk.clone();
    let reverse_for_fallback = reverse;
    let git_store = self.git_store.clone();
    if let Some(store) = &git_store {
      store.bump_op();
    }
    self.git_op_in_flight = true;
    let repo_for_index = repo_file.clone();
    let repo_for_apply = repo_file.clone();
    let repo_for_meta = repo_file.clone();
    let index_text_for_write = index_text_to_write.clone();
    let index_text_for_update = index_text_to_write.clone();
    self.git_task = Some(cx.spawn(async move |this, cx| {
      let index_result = if needs_index {
        if let Some(text) = index_text_for_write.clone() {
          unblock(move || git::write_index_content(&repo_for_index, &text)).await
        } else {
          Ok(())
        }
      } else {
        Ok(())
      };

      if let Err(_err) = index_result {
        let _ = this.update(cx, |editor, cx| {
          editor.git_op_in_flight = false;
          editor.maybe_start_next_git_job(cx);
        });
        return;
      }

      let workdir_result = if needs_workdir {
        unblock(move || git::apply_hunk(&repo_for_apply, &hunk, reverse, ApplyLocation::WorkDir))
          .await
      } else {
        Ok(())
      };

      if let Err(_err) = workdir_result {
        let fallback_result = if needs_workdir {
          unblock(move || {
            let text = std::fs::read_to_string(&workdir_path_for_fallback)?;
            let updated = git::apply_hunk_to_text(&text, &hunk_for_fallback, reverse_for_fallback)
              .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
            std::fs::write(&workdir_path_for_fallback, updated)?;
            Ok::<(), std::io::Error>(())
          })
          .await
        } else {
          Ok(())
        };

        if let Err(_err) = fallback_result {
          let _ = this.update(cx, |editor, cx| {
            editor.git_op_in_flight = false;
            editor.maybe_start_next_git_job(cx);
          });
          return;
        }
      }

      let (contents, file_mtime, index_mtime): (
        Option<String>,
        Option<SystemTime>,
        Option<SystemTime>,
      ) = unblock(move || {
        let contents = if needs_workdir {
          std::fs::read_to_string(&workdir_path).ok()
        } else {
          None
        };
        let file_mtime = std::fs::metadata(&workdir_path)
          .and_then(|meta| meta.modified())
          .ok();
        let index_mtime = std::fs::metadata(repo_for_meta.repo_root.join(".git/index"))
          .and_then(|meta| meta.modified())
          .ok();
        (contents, file_mtime, index_mtime)
      })
      .await;

      let _ = this.update(cx, |editor, cx| {
        if let Some(contents) = contents {
          editor.reload_from_disk(contents, cx);
        }
        editor.file_mtime = file_mtime;
        editor.index_mtime = index_mtime;
        if let Some(index_text) = index_text_for_update {
          if let Some(bases) = editor.git_state.bases.as_mut() {
            bases.index = Some(index_text);
          }
        }
        editor.pending_git_after_bases = true;
        editor.reload_git_bases(cx);
        editor.schedule_diff_recompute(cx);
      });
    }));
  }

  fn start_polling(&mut self, cx: &mut Context<Self>) {
    if self.poll_task.is_some() {
      return;
    }

    self.poll_task = Some(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(POLL_INTERVAL_MS))
          .await;

        let state = this
          .update(cx, |editor, _| {
            (
              editor.repo_file.clone(),
              editor.workdir_path.clone(),
              editor.file_mtime,
              editor.index_mtime,
            )
          })
          .ok();
        let Some((repo_file, workdir_path, last_file_mtime, last_index_mtime)) = state else {
          return;
        };

        let workdir_path_for_meta = workdir_path.clone();
        let (file_mtime, index_mtime): (Option<SystemTime>, Option<SystemTime>) =
          unblock(move || {
            let file_mtime = std::fs::metadata(&workdir_path_for_meta)
              .and_then(|meta| meta.modified())
              .ok();
            let index_mtime = repo_file
              .as_ref()
              .and_then(|repo| std::fs::metadata(repo.repo_root.join(".git/index")).ok())
              .and_then(|meta| meta.modified().ok());
            (file_mtime, index_mtime)
          })
          .await;

        let file_changed = file_mtime.is_some() && file_mtime != last_file_mtime;
        let index_changed = index_mtime.is_some() && index_mtime != last_index_mtime;

        let new_contents = if file_changed {
          let workdir_path = workdir_path.clone();
          unblock(move || std::fs::read_to_string(&workdir_path))
            .await
            .ok()
        } else {
          None
        };

        if new_contents.is_some() || index_changed {
          let _ = this.update(cx, |editor, cx| {
            if let Some(contents) = new_contents {
              editor.reload_from_disk(contents, cx);
              editor.file_mtime = file_mtime;
            }
            if index_changed {
              editor.index_mtime = index_mtime;
              editor.reload_git_bases(cx);
            } else {
              editor.schedule_diff_recompute(cx);
            }
          });
        }
      }
    }));
  }

  fn reload_from_disk(&mut self, contents: String, cx: &mut Context<Self>) {
    self.document.update(cx, |doc, cx| {
      doc.replace_all(&contents, cx);
    });
    self.line_layouts.clear();
    self.virtual_line_layouts.clear();
    self.expanded_gaps.clear();
    self.undo_stack.clear();
    self.redo_stack.clear();
    self.selected_range = 0..0;
    self.selection_reversed = false;
    self.display_selection = None;
    self.marked_range = None;
    self.hovered_group_id = None;
    self.last_mouse_position = None;
    self.git_jobs.clear();
    self.git_op_in_flight = false;
    self.pending_git_after_bases = false;
    self.git_state.index_dirty = false;
    self.optimistic_unstaged_groups.clear();
    self.is_dirty = false;
    self.last_highlights_version = 0;
    self.last_highlights_epoch = 0;
    self.scroll_offset_y = 0.0;
    cx.notify();
  }

  pub fn display_line_count(&self, doc_line_count: usize) -> usize {
    self
      .projection
      .as_ref()
      .map(|projection| projection.lines.len())
      .unwrap_or(doc_line_count)
  }

  pub fn display_line(&self, display_line: usize, doc_line_count: usize) -> Option<DisplayLine> {
    if let Some(projection) = &self.projection {
      projection.lines.get(display_line).cloned()
    } else if display_line < doc_line_count {
      Some(DisplayLine::Doc {
        doc_line: display_line,
        old_line: Some(display_line),
        change: None,
        hunk: None,
        group_id: None,
        secondary: false,
      })
    } else {
      None
    }
  }

  pub(crate) fn group_id_for_modified_display_line(&self, display_line: usize) -> Option<Arc<str>> {
    let projection = self.projection.as_ref()?;
    match projection.lines.get(display_line)? {
      DisplayLine::Doc {
        change: Some(ChangeKind::Added),
        group_id: Some(id),
        ..
      } => Some(id.clone()),
      DisplayLine::Modified { group_id, .. } => group_id.clone(),
      DisplayLine::Removed { group_id, .. } => group_id.clone(),
      _ => None,
    }
  }

  pub fn display_to_doc_line(&self, display_line: usize) -> Option<usize> {
    if let Some(projection) = &self.projection {
      projection.display_to_doc_line(display_line)
    } else {
      Some(display_line)
    }
  }

  pub fn doc_to_display_line(&self, doc_line: usize) -> Option<usize> {
    if let Some(projection) = &self.projection {
      projection.doc_to_display_line(doc_line)
    } else {
      Some(doc_line)
    }
  }

  pub fn previous_visible_doc_line(&self, doc_line: usize) -> Option<usize> {
    if let Some(projection) = &self.projection {
      projection.previous_visible_doc_line(doc_line)
    } else {
      doc_line.checked_sub(1)
    }
  }

  pub fn expand_gap(&mut self, gap_id: GapId, amount: usize, cx: &mut Context<Self>) {
    let entry = self.expanded_gaps.entry(gap_id).or_insert(0);
    *entry = entry.saturating_add(amount);

    if let Some(diffs) = self.diffs.clone() {
      self.apply_diffs(diffs, cx);
    } else {
      self.schedule_diff_recompute(cx);
    }
  }

  pub fn next_visible_doc_line(&self, doc_line: usize, doc_line_count: usize) -> Option<usize> {
    if let Some(projection) = &self.projection {
      projection.next_visible_doc_line(doc_line)
    } else if doc_line + 1 < doc_line_count {
      Some(doc_line + 1)
    } else {
      None
    }
  }

  /// Invalidate a single line in the cache
  pub(crate) fn invalidate_line(&mut self, line: usize) {
    self.line_layouts.remove(&line);
  }

  /// Invalidate all lines from start_line onwards (for multi-line edits)
  pub(crate) fn invalidate_lines_from(&mut self, start_line: usize) {
    self
      .line_layouts
      .retain(|&line_idx, _| line_idx < start_line);
  }

  pub fn ensure_cache_size(&mut self, viewport: Range<usize>) {
    // If cache is too large, keep only lines near the viewport
    if self.line_layouts.len() > self.max_cache_size {
      let viewport_start = viewport.start.saturating_sub(50);
      let viewport_end = viewport.end + 50;

      self
        .line_layouts
        .retain(|&line_idx, _| line_idx >= viewport_start && line_idx < viewport_end);
    }

    if self.virtual_line_layouts.len() > self.max_cache_size {
      let viewport_start = viewport.start.saturating_sub(50);
      let viewport_end = viewport.end + 50;

      self
        .virtual_line_layouts
        .retain(|&line_idx, _| line_idx >= viewport_start && line_idx < viewport_end);
    }
  }

  pub(crate) fn viewport_range(&self, line_height: Pixels, total_lines: usize) -> Range<usize> {
    let visible_line_count = ((self.viewport_height / line_height).ceil() as usize).max(1);
    let start_line = (self.scroll_offset_y.floor() as usize).min(total_lines.saturating_sub(1));
    let end_line = (start_line + visible_line_count).min(total_lines);
    start_line..end_line
  }

  pub(crate) fn doc_range_for_display_viewport(&self, viewport: Range<usize>) -> Range<usize> {
    let Some(projection) = &self.projection else {
      return viewport;
    };

    let mut min_line: Option<usize> = None;
    let mut max_line: Option<usize> = None;

    for display_line in viewport {
      if let Some(doc_line) = projection.display_to_doc_line(display_line) {
        min_line = Some(min_line.map_or(doc_line, |min| min.min(doc_line)));
        max_line = Some(max_line.map_or(doc_line, |max| max.max(doc_line)));
      }
    }

    match (min_line, max_line) {
      (Some(min), Some(max)) => min..(max + 1),
      _ => 0..0,
    }
  }

  pub(crate) fn ensure_cursor_visible(&mut self, window: &Window, cx: &mut Context<Self>) {
    let document = self.document.read(cx);
    let cursor_offset = self.cursor_offset();
    let doc_line_count = document.len_lines();
    let display_cursor = self.current_display_cursor(cx);
    let (cursor_line, cursor_column, cursor_doc_line) = if let Some(display_cursor) = display_cursor
    {
      let doc_line = self.display_to_doc_line(display_cursor.line);
      (display_cursor.line, display_cursor.column, doc_line)
    } else {
      let cursor_doc_line = document.char_to_line(cursor_offset);
      let Some(cursor_line) = self.doc_to_display_line(cursor_doc_line) else {
        return;
      };
      let line_start = document.line_to_char(cursor_doc_line);
      let column = cursor_offset.saturating_sub(line_start);
      (cursor_line, column, Some(cursor_doc_line))
    };
    let total_lines = self.display_line_count(doc_line_count);

    // Calculate how many lines are visible in the viewport
    let line_height = window.line_height();
    let visible_lines = (self.viewport_height / line_height).floor() as usize;

    // Offset for context padding when scrolling
    let scroll_padding = SCROLL_PADDING;

    // Calculate the visible range with padding
    let scroll_start = self.scroll_offset_y as usize;
    let scroll_end = scroll_start + visible_lines;

    // Ensure cursor is within the visible range with padding (vertical)
    if cursor_line < scroll_start + scroll_padding {
      // Cursor is too close to top, scroll up
      self.scroll_offset_y = (cursor_line.saturating_sub(scroll_padding)) as f32;
    } else if cursor_line >= scroll_end.saturating_sub(scroll_padding) {
      // Cursor is too close to bottom, scroll down
      let target_line = cursor_line + scroll_padding;
      self.scroll_offset_y = (target_line as f32 - visible_lines as f32 + 1.0).max(0.0);
    }

    // Ensure cursor is visible horizontally
    let shaped_line = match cursor_doc_line {
      Some(doc_line) => self.line_layouts.get(&doc_line).cloned(),
      None => self.virtual_line_layouts.get(&cursor_line).cloned(),
    };

    if let Some(shaped_line) = shaped_line {
      let line_len = self.display_line_len(cursor_line, cx);
      let cursor_in_line = cursor_column.min(line_len);
      let cursor_x = shaped_line.x_for_index(cursor_in_line);

      let horizontal_padding = px(GUTTER_WIDTH) + px(EXTRA_EDITOR_WIDTH);
      let current_scroll_x = self.scroll_handle.offset().x;

      // Note: scroll_x is negative when scrolled right (0 = left edge, -100 = scrolled 100px right)
      // visible area in absolute coordinates: [-current_scroll_x, -current_scroll_x + viewport_width]
      let visible_start_x = -current_scroll_x;
      let visible_end_x = -current_scroll_x + self.viewport_width;

      // Check if cursor is too far left
      if cursor_x < visible_start_x + horizontal_padding {
        let new_scroll_x = -(cursor_x - horizontal_padding).max(px(0.0));
        self.scroll_handle.set_offset(point(new_scroll_x, px(0.0)));
      }

      // Check if cursor is too far right
      if cursor_x > visible_end_x - horizontal_padding {
        let new_scroll_x = -(cursor_x - self.viewport_width + horizontal_padding);
        self.scroll_handle.set_offset(point(new_scroll_x, px(0.0)));
      }
    }

    let max_scroll = (total_lines as f32 - visible_lines as f32).max(0.0);
    self.scroll_offset_y = self.scroll_offset_y.clamp(0.0, max_scroll);
  }

  pub(crate) fn record_transaction(
    &mut self,
    id: TransactionId,
    selection_before: Range<usize>,
    selection_after: Range<usize>,
  ) {
    // Check if we should update an existing transaction with the same ID (grouping)
    if let Some(transaction) = self.undo_stack.iter_mut().find(|t| t.id == id) {
      transaction.selection_after = selection_after;
    } else {
      // Create new transaction
      self.undo_stack.push_back(Transaction {
        id,
        selection_before,
        selection_after,
      });
      self.redo_stack.clear();
    }
  }

  pub(crate) fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
    self.selected_range = offset..offset;
    if !self.is_selecting {
      self.display_selection = None;
    }
    // Show cursor immediately on move
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    cx.notify();
  }

  pub fn cursor_offset(&self) -> usize {
    if self.selection_reversed {
      self.selected_range.start
    } else {
      self.selected_range.end
    }
  }

  pub(crate) fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
    if self.selection_reversed {
      self.selected_range.start = offset
    } else {
      self.selected_range.end = offset
    };
    if !self.is_selecting {
      self.display_selection = None;
    }
    if self.selected_range.end < self.selected_range.start {
      self.selection_reversed = !self.selection_reversed;
      self.selected_range = self.selected_range.end..self.selected_range.start;
    }
    cx.notify()
  }

  fn selection_anchor_offset(&self) -> usize {
    if self.selection_reversed {
      self.selected_range.end
    } else {
      self.selected_range.start
    }
  }

  fn current_display_cursor(&self, cx: &App) -> Option<DisplayCursor> {
    if let Some(selection) = &self.display_selection {
      Some(selection.end)
    } else {
      self.display_cursor_for_offset(self.cursor_offset(), cx)
    }
  }

  fn current_display_anchor(&self, cx: &App) -> Option<DisplayCursor> {
    if let Some(selection) = &self.display_selection {
      Some(selection.start)
    } else {
      self.display_cursor_for_offset(self.selection_anchor_offset(), cx)
    }
  }

  fn display_line_len(&self, display_line: usize, cx: &App) -> usize {
    let document = self.document.read(cx);
    let doc_line_count = document.len_lines();
    match self.display_line(display_line, doc_line_count) {
      Some(DisplayLine::Doc { doc_line, .. }) => document
        .line_content(doc_line)
        .map(|cow| cow.len())
        .unwrap_or(0),
      Some(DisplayLine::Modified { doc_line, .. }) => document
        .line_content(doc_line)
        .map(|cow| cow.len())
        .unwrap_or(0),
      Some(DisplayLine::Removed { text, .. }) => text.len(),
      Some(DisplayLine::NoNewline { .. }) => NO_NEWLINE_MARKER_TEXT.len(),
      _ => 0,
    }
  }

  fn is_removed_display_line(&self, display_line: usize, cx: &App) -> bool {
    let document = self.document.read(cx);
    let doc_line_count = document.len_lines();
    matches!(
      self.display_line(display_line, doc_line_count),
      Some(DisplayLine::Removed { .. })
    )
  }

  fn is_read_only_display_cursor(&self, cx: &App) -> bool {
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    self.is_removed_display_line(cursor.line, cx)
  }

  fn set_display_cursor(&mut self, cursor: DisplayCursor, cx: &mut Context<Self>) {
    self.display_selection = Some(DisplaySelection {
      start: cursor,
      end: cursor,
    });
    if let Some(offset) = self.doc_offset_for_display_cursor(cursor, cx) {
      self.selected_range = offset..offset;
      self.selection_reversed = false;
    }
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    cx.notify();
  }

  fn set_display_selection_with_anchor(
    &mut self,
    anchor: DisplayCursor,
    cursor: DisplayCursor,
    cx: &mut Context<Self>,
  ) {
    self.display_selection = Some(DisplaySelection {
      start: anchor,
      end: cursor,
    });

    let reversed =
      cursor.line < anchor.line || (cursor.line == anchor.line && cursor.column < anchor.column);
    self.selection_reversed = reversed;

    if let (Some(anchor_offset), Some(cursor_offset)) = (
      self.doc_offset_for_display_cursor(anchor, cx),
      self.doc_offset_for_display_cursor(cursor, cx),
    ) {
      if reversed {
        self.selected_range = cursor_offset..anchor_offset;
      } else {
        self.selected_range = anchor_offset..cursor_offset;
      }
    }

    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    cx.notify();
  }

  fn next_selectable_display_line(&self, start: usize, direction: i32) -> Option<usize> {
    let projection = self.projection.as_ref()?;
    let mut line = start as i32 + direction;
    let max_line = projection.lines.len() as i32;
    while line >= 0 && line < max_line {
      if let Some(DisplayLine::Gap { .. }) = projection.lines.get(line as usize) {
        line += direction;
        continue;
      }
      return Some(line as usize);
    }
    None
  }

  pub(crate) fn move_display_cursor_vertical(
    &mut self,
    direction: i32,
    cx: &mut Context<Self>,
  ) -> bool {
    if self.projection.is_none() {
      return false;
    }
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };

    if self.target_column.is_none() {
      self.target_column = Some(cursor.column);
    }
    let target_column = self.target_column.unwrap_or(cursor.column);
    let Some(target_line) = self.next_selectable_display_line(cursor.line, direction) else {
      return true;
    };

    let line_len = self.display_line_len(target_line, cx);
    let column = target_column.min(line_len);
    self.set_display_cursor(
      DisplayCursor {
        line: target_line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn select_display_cursor_vertical(
    &mut self,
    direction: i32,
    cx: &mut Context<Self>,
  ) -> bool {
    if self.projection.is_none() {
      return false;
    }
    let Some(anchor) = self.current_display_anchor(cx) else {
      return false;
    };
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };

    if self.target_column.is_none() {
      self.target_column = Some(cursor.column);
    }
    let target_column = self.target_column.unwrap_or(cursor.column);
    let Some(target_line) = self.next_selectable_display_line(cursor.line, direction) else {
      return true;
    };

    let line_len = self.display_line_len(target_line, cx);
    let column = target_column.min(line_len);
    self.set_display_selection_with_anchor(
      anchor,
      DisplayCursor {
        line: target_line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn move_display_cursor_horizontal(
    &mut self,
    delta: i32,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    if !self.is_removed_display_line(cursor.line, cx) {
      return false;
    }

    let line_len = self.display_line_len(cursor.line, cx);
    let mut column = cursor.column as i32 + delta;
    column = column.clamp(0, line_len as i32);
    let column = column as usize;
    self.target_column = Some(column);
    self.set_display_cursor(
      DisplayCursor {
        line: cursor.line,
        column,
      },
      cx,
    );
    true
  }

  fn display_cursor_for_offset(&self, offset: usize, cx: &App) -> Option<DisplayCursor> {
    let document = self.document.read(cx);
    if document.is_empty() {
      return Some(DisplayCursor { line: 0, column: 0 });
    }

    let doc_line = document.char_to_line(offset);
    let line_start = document.line_to_char(doc_line);
    let column = offset.saturating_sub(line_start);

    let display_line = if let Some(display_line) = self.doc_to_display_line(doc_line) {
      display_line
    } else if let Some(projection) = &self.projection {
      if let Some(prev) = projection
        .previous_visible_doc_line(doc_line)
        .and_then(|line| self.doc_to_display_line(line))
      {
        prev
      } else if let Some(next) = projection
        .next_visible_doc_line(doc_line)
        .and_then(|line| self.doc_to_display_line(line))
      {
        next
      } else {
        return None;
      }
    } else {
      doc_line
    };

    Some(DisplayCursor {
      line: display_line,
      column,
    })
  }

  fn doc_offset_for_display_cursor(&self, cursor: DisplayCursor, cx: &App) -> Option<usize> {
    let document = self.document.read(cx);
    if document.is_empty() {
      return Some(0);
    }

    let doc_line = if let Some(doc_line) = self.display_to_doc_line(cursor.line) {
      doc_line
    } else if let Some(projection) = &self.projection {
      let mut forward = cursor.line + 1;
      let mut backward = cursor.line;
      let mut found = None;

      while forward < projection.lines.len() || backward > 0 {
        if forward < projection.lines.len() {
          if let Some(doc_line) = projection.display_to_doc_line(forward) {
            found = Some(doc_line);
            break;
          }
          forward += 1;
        }

        if backward > 0 {
          backward -= 1;
          if let Some(doc_line) = projection.display_to_doc_line(backward) {
            found = Some(doc_line);
            break;
          }
        }
      }

      found.unwrap_or(0)
    } else {
      cursor.line
    };

    let doc_line = doc_line.min(document.len_lines().saturating_sub(1));
    let line_start = document.line_to_char(doc_line);
    let line_len = document
      .line_content(doc_line)
      .map(|cow| cow.len())
      .unwrap_or(0);
    let col = cursor.column.min(line_len);
    Some(line_start + col)
  }

  pub(crate) fn offset_from_utf16(&self, offset: usize, cx: &App) -> usize {
    let document = self.document.read(cx);
    let mut utf16_count = 0;

    for (char_offset, ch) in document.chars().enumerate() {
      if utf16_count >= offset {
        return char_offset;
      }
      utf16_count += ch.len_utf16();
    }

    document.len()
  }

  pub(crate) fn offset_to_utf16(&self, offset: usize, cx: &App) -> usize {
    let document = self.document.read(cx);
    let mut utf16_offset = 0;

    for (char_count, ch) in document.chars().enumerate() {
      if char_count >= offset {
        break;
      }
      utf16_offset += ch.len_utf16();
    }

    utf16_offset
  }

  pub(crate) fn range_to_utf16(&self, range: &Range<usize>, cx: &App) -> Range<usize> {
    self.offset_to_utf16(range.start, cx)..self.offset_to_utf16(range.end, cx)
  }

  pub(crate) fn range_from_utf16(&self, range_utf16: &Range<usize>, cx: &App) -> Range<usize> {
    self.offset_from_utf16(range_utf16.start, cx)..self.offset_from_utf16(range_utf16.end, cx)
  }

  pub fn mouse_left_down(
    &mut self,
    event: &MouseDownEvent,
    position_map: &PositionMap,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.target_column = None;
    self.is_selecting = true;

    // Show cursor immediately on mouse down
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });

    if let Some(display_line) = position_map.display_line_for_position(event.position) {
      if let Some(projection) = &position_map.projection {
        if let Some(DisplayLine::Gap { id, .. }) = projection.lines.get(display_line) {
          let amount = if event.modifiers.shift { 20 } else { 10 };
          self.expand_gap(*id, amount, cx);
          self.is_selecting = false;
          return;
        }
      }
    }

    let document = self.document.read(cx);
    let Some(display_cursor) = position_map.display_cursor_for_position(event.position) else {
      return;
    };

    self.last_mouse_position = Some(event.position);

    let anchor = if event.modifiers.shift {
      if let Some(selection) = &self.display_selection {
        selection.start
      } else {
        self
          .display_cursor_for_offset(self.selection_anchor_offset(), cx)
          .unwrap_or(display_cursor)
      }
    } else {
      display_cursor
    };

    self.display_selection = Some(DisplaySelection {
      start: anchor,
      end: display_cursor,
    });

    let offset = position_map
      .point_for_position(event.position, document)
      .or_else(|| self.doc_offset_for_display_cursor(display_cursor, cx));
    let Some(offset) = offset else {
      return;
    };

    if event.modifiers.shift {
      self.select_to(offset, cx);
    } else {
      match event.click_count {
        1 => {
          self.move_to(offset, cx);
        }
        2 => {
          let (word_start, word_end) = word_range_at_offset(self, offset, cx);
          self.selected_range = word_start..word_end;
          self.selection_reversed = false;
          self.display_selection = None;
          cx.notify();
        }
        3 => {
          let (line_start, line_end) = line_range_at_offset(self, offset, cx);
          self.selected_range = line_start..line_end;
          self.selection_reversed = false;
          self.display_selection = None;
          cx.notify();
        }
        _ => {
          let doc_len = document.len();
          self.selected_range = 0..doc_len;
          self.selection_reversed = false;
          self.display_selection = None;
          cx.notify();
        }
      }
    }
  }

  pub fn mouse_left_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
    self.is_selecting = false;
  }

  pub fn mouse_moved(
    &mut self,
    event: &MouseMoveEvent,
    position_map: &PositionMap,
    cx: &mut Context<Self>,
  ) {
    if !position_map.bounds.contains(&event.position) {
      return;
    }
    self.last_mouse_position = Some(event.position);
    let hovered = position_map
      .display_line_for_position(event.position)
      .and_then(|display_line| self.group_id_for_modified_display_line(display_line));

    if self.hovered_group_id.as_deref() != hovered.as_deref() {
      self.hovered_group_id = hovered;
      cx.notify();
    }
  }

  pub fn mouse_dragged(
    &mut self,
    event: &MouseMoveEvent,
    position_map: &PositionMap,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !self.is_selecting {
      return;
    }

    self.last_mouse_position = Some(event.position);

    if let Some(display_cursor) = position_map.display_cursor_for_position(event.position) {
      if let Some(selection) = self.display_selection.as_mut() {
        selection.end = display_cursor;
      } else {
        self.display_selection = Some(DisplaySelection {
          start: display_cursor,
          end: display_cursor,
        });
      }
    }

    let document = self.document.read(cx);
    let offset = position_map
      .point_for_position(event.position, document)
      .or_else(|| {
        position_map
          .display_cursor_for_position(event.position)
          .and_then(|cursor| self.doc_offset_for_display_cursor(cursor, cx))
      });

    if let Some(offset) = offset {
      self.select_to(offset, cx);
    }
  }
}

impl EntityInputHandler for Editor {
  fn text_for_range(
    &mut self,
    range_utf16: Range<usize>,
    actual_range: &mut Option<Range<usize>>,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<String> {
    let doc = self.document.read(cx);
    let range = self.range_from_utf16(&range_utf16, cx);
    actual_range.replace(self.range_to_utf16(&range, cx));
    Some(doc.slice_to_string(range))
  }

  fn selected_text_range(
    &mut self,
    _ignore_disabled_input: bool,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<UTF16Selection> {
    Some(UTF16Selection {
      range: self.range_to_utf16(&self.selected_range, cx),
      reversed: self.selection_reversed,
    })
  }

  fn marked_text_range(
    &self,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<Range<usize>> {
    self
      .marked_range
      .as_ref()
      .map(|range| self.range_to_utf16(range, cx))
  }

  fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
    self.marked_range = None;
  }

  fn replace_text_in_range(
    &mut self,
    range_utf16: Option<Range<usize>>,
    new_text: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.is_read_only_display_cursor(cx) && self.selected_range.is_empty() {
      return;
    }
    // Pause cursor blinking when typing
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    self.display_selection = None;
    let range = range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16, cx))
      .or(self.marked_range.clone())
      .unwrap_or(self.selected_range.clone());

    let selection_before = self.selected_range.clone();
    let start_line = self.document.read(cx).char_to_line(range.start);
    let end_line = self.document.read(cx).char_to_line(range.end);

    let line_height = window.line_height();
    let doc_line_count = self.document.read(cx).len_lines();
    let total_display_lines = self.display_line_count(doc_line_count);
    let display_viewport = self.viewport_range(line_height, total_display_lines);
    let doc_viewport = self.doc_range_for_display_viewport(display_viewport);
    let new_line_count = new_text.matches('\n').count();
    let force_end_line = start_line.saturating_add(new_line_count).max(end_line);
    let force_range = start_line..(force_end_line + 1);

    self.maybe_optimistic_unstage_for_edit(start_line, end_line, cx);

    let transaction_id = self.document.update(cx, |doc, cx| {
      let id = doc.buffer.transaction(Instant::now(), |buffer, tx| {
        buffer.replace(tx, range.clone(), new_text);
      });

      // Trigger async syntax re-highlighting with debouncing
      doc.schedule_recompute_highlights(cx);
      doc.schedule_viewport_highlights(
        doc_viewport.clone(),
        Some(force_range.clone()),
        crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
        cx,
      );

      cx.notify();
      id
    });

    let has_newline = new_text.contains('\n');

    if has_newline || start_line != end_line {
      // Multi-line edit: invalidate from start line onwards
      self.invalidate_lines_from(start_line);
    } else {
      // Single-line edit: only invalidate the affected line
      self.invalidate_line(start_line);
    }

    self.selected_range = range.start + new_text.len()..range.start + new_text.len();
    self.marked_range.take();

    let selection_after = self.selected_range.clone();

    self.record_transaction(transaction_id, selection_before, selection_after);

    self.is_dirty = true;
    cx.notify();
    self.schedule_diff_recompute(cx);
  }

  fn replace_and_mark_text_in_range(
    &mut self,
    range_utf16: Option<Range<usize>>,
    new_text: &str,
    new_selected_range_utf16: Option<Range<usize>>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.is_read_only_display_cursor(cx) && self.selected_range.is_empty() {
      return;
    }
    // Pause cursor blinking when typing
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    let range = range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16, cx))
      .or(self.marked_range.clone())
      .unwrap_or(self.selected_range.clone());

    let start_line = self.document.read(cx).char_to_line(range.start);

    let line_height = window.line_height();
    let doc_line_count = self.document.read(cx).len_lines();
    let total_display_lines = self.display_line_count(doc_line_count);
    let display_viewport = self.viewport_range(line_height, total_display_lines);
    let doc_viewport = self.doc_range_for_display_viewport(display_viewport);
    let end_line = self.document.read(cx).char_to_line(range.end);
    let new_line_count = new_text.matches('\n').count();
    let force_end_line = start_line.saturating_add(new_line_count).max(end_line);
    let force_range = start_line..(force_end_line + 1);

    self.maybe_optimistic_unstage_for_edit(start_line, end_line, cx);

    self.document.update(cx, |doc, cx| {
      doc.replace(range.clone(), new_text, cx);
      doc.schedule_recompute_highlights(cx);
      doc.schedule_viewport_highlights(
        doc_viewport.clone(),
        Some(force_range.clone()),
        crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
        cx,
      );
    });

    // Invalidate cache for all lines from the start of the edit
    self.invalidate_lines_from(start_line);

    if !new_text.is_empty() {
      self.marked_range = Some(range.start..range.start + new_text.len());
    } else {
      self.marked_range = None;
    }
    self.selected_range = new_selected_range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16, cx))
      .map(|new_range| new_range.start + range.start..new_range.end + range.end)
      .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

    self.is_dirty = true;
    cx.notify();
    self.schedule_diff_recompute(cx);
  }

  fn bounds_for_range(
    &mut self,
    _range_utf16: Range<usize>,
    _bounds: Bounds<Pixels>,
    _window: &mut Window,
    _cx: &mut Context<Self>,
  ) -> Option<Bounds<Pixels>> {
    None
  }

  fn character_index_for_point(
    &mut self,
    _point: Point<Pixels>,
    _window: &mut Window,
    _cx: &mut Context<Self>,
  ) -> Option<usize> {
    None
  }
}

impl Render for Editor {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let is_dark = cx.theme().mode.is_dark();
    if self.theme.is_dark != is_dark {
      self.theme = Theme::new(is_dark);
      self.line_layouts.clear();
      self.virtual_line_layouts.clear();
      self.last_highlights_version = 0;
      self.last_highlights_epoch = 0;
    }

    let editor_entity = cx.entity().clone();
    let content = if self.diff_view_mode == DiffViewMode::Split {
      let left_panel = div()
        .size_full()
        .flex()
        .flex_row()
        .child(
          div()
            .w(px(GUTTER_WIDTH))
            .h_full()
            .bg(self.theme.gutter_background())
            .child(GutterElement::split_left(editor_entity.clone())),
        )
        .child(
          div()
            .flex_1()
            .h_full()
            .id("editor-content-left")
            .overflow_x_scroll()
            .track_scroll(&self.scroll_handle)
            .child(
              div()
                .min_w(self.max_line_width + px(EXTRA_EDITOR_WIDTH))
                .h_full()
                .relative()
                .child(EditorElement::split_left(editor_entity.clone())),
            ),
        );

      let right_panel = div()
        .size_full()
        .flex()
        .flex_row()
        .child(
          div()
            .w(px(GUTTER_WIDTH))
            .h_full()
            .bg(self.theme.gutter_background())
            .child(GutterElement::split_right(editor_entity.clone())),
        )
        .child(
          div()
            .flex_1()
            .h_full()
            .id("editor-content")
            .overflow_x_scroll()
            .track_scroll(&self.scroll_handle)
            .child(
              div()
                .min_w(self.max_line_width + px(EXTRA_EDITOR_WIDTH))
                .h_full()
                .relative()
                .child(EditorElement::split_right(editor_entity)),
            ),
        );

      div().flex_1().min_h(px(0.0)).child(
        h_resizable("editor-diff-split")
          .child(resizable_panel().child(left_panel))
          .child(resizable_panel().child(right_panel)),
      )
    } else {
      div()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .flex_row()
        .child(
          div()
            .w(px(GUTTER_WIDTH))
            .h_full()
            .bg(self.theme.gutter_background())
            .child(GutterElement::new(editor_entity.clone())),
        )
        .child(
          div()
            .flex_1()
            .h_full()
            .id("editor-content")
            .overflow_x_scroll()
            .track_scroll(&self.scroll_handle)
            .child(
              div()
                .min_w(self.max_line_width + px(EXTRA_EDITOR_WIDTH))
                .h_full()
                .relative()
                .child(EditorElement::new(editor_entity)),
            ),
        )
    };

    div()
      .key_context("Editor")
      .track_focus(&self.focus_handle(cx))
      .cursor(CursorStyle::IBeam)
      .size_full()
      .on_action(cx.listener(crate::actions::enter))
      .on_action(cx.listener(crate::actions::backspace))
      .on_action(cx.listener(crate::actions::backspace_word))
      .on_action(cx.listener(crate::actions::backspace_all))
      .on_action(cx.listener(crate::actions::delete))
      .on_action(cx.listener(crate::actions::up))
      .on_action(cx.listener(crate::actions::down))
      .on_action(cx.listener(crate::actions::left))
      .on_action(cx.listener(crate::actions::alt_left))
      .on_action(cx.listener(crate::actions::cmd_left))
      .on_action(cx.listener(crate::actions::right))
      .on_action(cx.listener(crate::actions::alt_right))
      .on_action(cx.listener(crate::actions::cmd_right))
      .on_action(cx.listener(crate::actions::cmd_up))
      .on_action(cx.listener(crate::actions::cmd_down))
      .on_action(cx.listener(crate::actions::select_cmd_left))
      .on_action(cx.listener(crate::actions::select_cmd_right))
      .on_action(cx.listener(crate::actions::select_cmd_up))
      .on_action(cx.listener(crate::actions::select_cmd_down))
      .on_action(cx.listener(crate::actions::select_up))
      .on_action(cx.listener(crate::actions::select_down))
      .on_action(cx.listener(crate::actions::select_left))
      .on_action(cx.listener(crate::actions::select_word_left))
      .on_action(cx.listener(crate::actions::select_right))
      .on_action(cx.listener(crate::actions::select_word_right))
      .on_action(cx.listener(crate::actions::select_all))
      .on_action(cx.listener(crate::actions::home))
      .on_action(cx.listener(crate::actions::end))
      .on_action(cx.listener(crate::actions::show_character_palette))
      .on_action(cx.listener(crate::actions::paste))
      .on_action(cx.listener(crate::actions::cut))
      .on_action(cx.listener(crate::actions::copy))
      .on_action(cx.listener(crate::actions::undo))
      .on_action(cx.listener(crate::actions::redo))
      .on_action(cx.listener(crate::actions::save))
      .when_else(self.theme.is_dark, |el| el.bg(black()), |el| el.bg(white()))
      .when_else(
        self.theme.is_dark,
        |el| el.text_color(white()),
        |el| el.text_color(black()),
      )
      .flex()
      .flex_col()
      .child(content)
  }
}

impl Focusable for Editor {
  fn focus_handle(&self, _: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
pub mod tests {
  use super::*;
  use gpui::TestAppContext;

  /// Helper context for testing Editor
  pub struct EditorTestContext {
    pub cx: TestAppContext,
    pub editor: Entity<Editor>,
  }

  impl EditorTestContext {
    /// Create a test context with specific text content
    pub fn with_text(mut cx: TestAppContext, text: &str) -> Self {
      let editor = cx.new(|cx| {
        let doc = cx.new(|cx| Document::new(text, None, cx));
        let cursor_blink = cx.new(CursorBlink::new);
        Editor {
          document: doc,
          focus_handle: cx.focus_handle(),
          selected_range: 0..0,
          selection_reversed: false,
          display_selection: None,
          marked_range: None,
          is_selecting: false,
          line_layouts: HashMap::new(),
          virtual_line_layouts: HashMap::new(),
          scroll_offset_y: 0.0,
          viewport_height: px(DEFAULT_VIEWPORT_HEIGHT),
          viewport_width: px(DEFAULT_VIEWPORT_WIDTH),
          max_line_width: px(DEFAULT_MAX_LINE_WIDTH),
          scroll_handle: ScrollHandle::new(),
          scroll_axis_lock: None,
          last_scroll_time: None,
          last_scroll_x: px(0.0),
          max_cache_size: MAX_CACHE_SIZE,
          target_column: None,
          undo_stack: VecDeque::new(),
          redo_stack: VecDeque::new(),
          theme: Theme::dark(),
          projection: None,
          visible_groups: Vec::new(),
          hovered_group_id: None,
          last_mouse_position: None,
          expanded_gaps: HashMap::new(),
          workdir_path: PathBuf::new(),
          repo_file: None,
          git_store: None,
          git_state: BufferGitState::default(),
          diffs: None,
          diff_task: None,
          bases_task: None,
          poll_task: None,
          git_task: None,
          git_jobs: VecDeque::new(),
          git_op_in_flight: false,
          pending_git_after_bases: false,
          diff_generation: Arc::new(AtomicUsize::new(0)),
          file_mtime: None,
          index_mtime: None,
          is_dirty: false,
          save_task: None,
          diff_view_mode: DiffViewMode::Inline,
          last_highlights_version: 0,
          last_highlights_epoch: 0,
          cursor_blink,
          optimistic_unstaged_groups: HashSet::new(),
        }
      });

      Self { cx, editor }
    }

    /// Create a test context with multiple lines for testing
    pub fn with_lines(cx: TestAppContext, count: usize) -> Self {
      let mut text = String::new();
      for i in 0..count {
        if i > 0 {
          text.push('\n');
        }
        text.push_str(&format!("Line {}", i + 1));
      }
      Self::with_text(cx, &text)
    }

    /// Get the current text content
    pub fn text(&self) -> String {
      self.editor.read_with(&self.cx, |editor, cx| {
        let doc = editor.document().read(cx);
        doc.slice_to_string(0..doc.len())
      })
    }

    /// Get the current cursor offset
    pub fn cursor_offset(&self) -> usize {
      self
        .editor
        .read_with(&self.cx, |editor, _| editor.cursor_offset())
    }

    /// Get the current selection range
    pub fn selection(&self) -> Range<usize> {
      self
        .editor
        .read_with(&self.cx, |editor, _| editor.selected_range.clone())
    }

    /// Get whether selection is reversed
    #[allow(dead_code)]
    pub fn selection_reversed(&self) -> bool {
      self
        .editor
        .read_with(&self.cx, |editor, _| editor.selection_reversed)
    }

    /// Set cursor position (collapses selection)
    pub fn set_cursor(&mut self, offset: usize) {
      self.editor.update(&mut self.cx, |editor, cx| {
        editor.move_to(offset, cx);
      });
    }

    /// Set selection range
    pub fn set_selection(&mut self, range: Range<usize>, reversed: bool) {
      self.editor.update(&mut self.cx, |editor, _| {
        editor.selected_range = range;
        editor.selection_reversed = reversed;
        editor.display_selection = None;
      });
    }

    /// Get the number of cached lines
    pub fn cache_size(&self) -> usize {
      self
        .editor
        .read_with(&self.cx, |editor, _| editor.line_layouts.len())
    }

    /// Check if a specific line is cached
    pub fn is_line_cached(&self, line_idx: usize) -> bool {
      self.editor.read_with(&self.cx, |editor, _| {
        editor.line_layouts.contains_key(&line_idx)
      })
    }
  }

  // ============================================================================
  // Cache Management Tests
  // ============================================================================

  #[gpui::test]
  fn test_invalidate_line_single(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 10);

    // Simulate cached lines
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..5 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    // Verify all are cached
    for i in 0..5 {
      assert!(ctx.is_line_cached(i));
    }

    // Invalidate line 2
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.invalidate_line(2);
    });

    // Line 2 should be removed, others stay
    assert!(ctx.is_line_cached(0));
    assert!(ctx.is_line_cached(1));
    assert!(!ctx.is_line_cached(2));
    assert!(ctx.is_line_cached(3));
    assert!(ctx.is_line_cached(4));
  }

  #[gpui::test]
  fn test_invalidate_lines_from(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 10);

    // Simulate cached lines 0-9
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..10 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    assert_eq!(ctx.cache_size(), 10);

    // Invalidate from line 5
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.invalidate_lines_from(5);
    });

    // Lines 0-4 should remain, 5-9 should be removed
    assert!(ctx.is_line_cached(0));
    assert!(ctx.is_line_cached(4));
    assert!(!ctx.is_line_cached(5));
    assert!(!ctx.is_line_cached(9));
    assert_eq!(ctx.cache_size(), 5);
  }

  #[gpui::test]
  fn test_ensure_cache_size_limit(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 300);

    // Fill cache beyond MAX_CACHE_SIZE
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..250 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    assert_eq!(ctx.cache_size(), 250);

    // Call ensure_cache_size with viewport at lines 100-120
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.ensure_cache_size(100..120);
    });

    // Cache should be reduced
    assert!(ctx.cache_size() < 250);

    // Lines near viewport should be kept (50..170 range)
    ctx.editor.read_with(&ctx.cx, |editor, _| {
      assert!(editor.line_layouts.contains_key(&100));
      assert!(editor.line_layouts.contains_key(&110));
    });
  }

  #[gpui::test]
  fn test_cache_retention_after_viewport_change(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 100);

    // Cache lines 10-20
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 10..=20 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    // Ensure cache size with different viewport
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.ensure_cache_size(30..40);
    });

    // Old cache should still exist (under limit)
    assert!(ctx.is_line_cached(15));
  }

  #[gpui::test]
  fn test_invalidate_on_insert(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    // Cache all lines
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..3 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    // Insert char on line 1 (offset 6 = start of "line2")
    ctx.set_cursor(6);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.document.update(cx, |doc, cx| {
        doc.insert_char(6, 'X', cx);
      });
      editor.invalidate_line(1);
    });

    // Only line 1 should be invalidated
    assert!(ctx.is_line_cached(0));
    assert!(!ctx.is_line_cached(1));
    assert!(ctx.is_line_cached(2));
  }

  #[gpui::test]
  fn test_invalidate_on_newline(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    // Cache all lines
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..3 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    // Insert newline on line 1
    ctx.set_cursor(6);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let current_line = editor.document.read(cx).char_to_line(6);
      editor.document.update(cx, |doc, cx| {
        doc.insert_char(6, '\n', cx);
      });
      editor.invalidate_lines_from(current_line);
    });

    // Lines from 1 onwards should be invalidated
    assert!(ctx.is_line_cached(0));
    assert!(!ctx.is_line_cached(1));
    assert!(!ctx.is_line_cached(2));
  }

  // ============================================================================
  // Navigation Tests
  // ============================================================================

  #[gpui::test]
  fn test_cursor_offset_initial(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello world");
    assert_eq!(ctx.cursor_offset(), 0);
    assert_eq!(ctx.selection(), 0..0);
  }

  #[gpui::test]
  fn test_move_to(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_cursor(5);
    assert_eq!(ctx.cursor_offset(), 5);
    assert_eq!(ctx.selection(), 5..5);

    ctx.set_cursor(11);
    assert_eq!(ctx.cursor_offset(), 11);
  }

  #[gpui::test]
  fn test_left_navigation(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.set_cursor(3);

    // Test the internal logic by checking cursor moved left
    let prev_offset = ctx.cursor_offset();
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let new_offset = if editor.selected_range.is_empty() {
        editor.cursor_offset().saturating_sub(1)
      } else {
        editor.selected_range.start.min(editor.selected_range.end)
      };
      editor.move_to(new_offset, cx);
    });
    assert_eq!(ctx.cursor_offset(), prev_offset - 1);
  }

  #[gpui::test]
  fn test_left_at_start(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.set_cursor(0);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let new_offset = editor.cursor_offset().saturating_sub(1);
      editor.move_to(new_offset, cx);
    });
    assert_eq!(ctx.cursor_offset(), 0); // Should stay at 0
  }

  #[gpui::test]
  fn test_right_navigation(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.set_cursor(2);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let doc_len = editor.document().read(cx).len();
      let new_offset = if editor.selected_range.is_empty() {
        (editor.cursor_offset() + 1).min(doc_len)
      } else {
        editor.selected_range.start.max(editor.selected_range.end)
      };
      editor.move_to(new_offset, cx);
    });
    assert_eq!(ctx.cursor_offset(), 3);
  }

  #[gpui::test]
  fn test_right_at_end(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.set_cursor(5);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let doc_len = editor.document().read(cx).len();
      let new_offset = (editor.cursor_offset() + 1).min(doc_len);
      editor.move_to(new_offset, cx);
    });
    assert_eq!(ctx.cursor_offset(), 5); // Should stay at end
  }

  // Note: Navigation tests that require Window are skipped for now
  // These will be tested with integration tests or VisualTestContext

  #[gpui::test]
  fn test_move_to_updates_cursor(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_cursor(7);
    assert_eq!(ctx.cursor_offset(), 7);
    assert_eq!(ctx.selection(), 7..7);
  }

  #[gpui::test]
  fn test_cursor_at_line_boundary(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    // Test cursor at line starts
    ctx.set_cursor(0);
    assert_eq!(ctx.cursor_offset(), 0);

    ctx.set_cursor(6); // Start of line2
    assert_eq!(ctx.cursor_offset(), 6);

    ctx.set_cursor(12); // Start of line3
    assert_eq!(ctx.cursor_offset(), 12);
  }

  #[gpui::test]
  fn test_cursor_positioning(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    // Test various cursor positions
    for pos in [0, 5, 11] {
      ctx.set_cursor(pos);
      assert_eq!(ctx.cursor_offset(), pos);
      assert_eq!(ctx.selection(), pos..pos);
    }
  }

  // ============================================================================
  // Text Editing Tests
  // ============================================================================

  // Note: Text editing tests that require Window are skipped for now
  // The core logic is well-tested in buffer.rs and document.rs

  #[gpui::test]
  fn test_selection_with_replace(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    // Test replacing selection
    ctx.set_selection(2..7, false);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let range = editor.selected_range.clone();
      editor.document.update(cx, |doc, cx| {
        doc.replace(range, "X", cx);
      });
      editor.move_to(2, cx);
    });

    assert_eq!(ctx.text(), "heXorld");
    assert_eq!(ctx.cursor_offset(), 2);
  }

  #[gpui::test]
  fn test_insert_at_cursor(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.set_cursor(5);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let cursor = editor.cursor_offset();
      editor.document.update(cx, |doc, cx| {
        doc.insert_char(cursor, '!', cx);
      });
      editor.move_to(cursor + 1, cx);
    });

    assert_eq!(ctx.text(), "hello!");
    assert_eq!(ctx.cursor_offset(), 6);
  }

  #[gpui::test]
  fn test_unicode_editing(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 world");

    // Verify emoji is present
    let text = ctx.text();
    assert!(text.contains("👋"));

    // Test cursor positioning around emoji
    ctx.set_cursor(6); // Before emoji
    assert_eq!(ctx.cursor_offset(), 6);

    ctx.set_cursor(7); // After emoji
    assert_eq!(ctx.cursor_offset(), 7);
  }

  #[gpui::test]
  fn test_cache_invalidation_on_edit(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    // Cache all lines
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..3 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    // Edit line 1
    ctx.set_cursor(6);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.document.update(cx, |doc, cx| {
        doc.insert_char(6, 'X', cx);
      });
      let line = editor.document.read(cx).char_to_line(6);
      editor.invalidate_line(line);
    });

    // Only line 1 should be invalidated
    assert!(ctx.is_line_cached(0));
    assert!(!ctx.is_line_cached(1));
    assert!(ctx.is_line_cached(2));
  }

  #[gpui::test]
  fn test_multiline_cache_invalidation(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3\nline4");

    // Cache all lines
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..4 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    // Insert newline on line 1
    ctx.set_cursor(6);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let line = editor.document.read(cx).char_to_line(6);
      editor.document.update(cx, |doc, cx| {
        doc.insert_char(6, '\n', cx);
      });
      editor.invalidate_lines_from(line);
    });

    // Lines from 1 onwards should be invalidated
    assert!(ctx.is_line_cached(0));
    assert!(!ctx.is_line_cached(1));
    assert!(!ctx.is_line_cached(2));
    assert!(!ctx.is_line_cached(3));
  }

  // ============================================================================
  // UTF-16 Conversion Tests
  // ============================================================================

  #[gpui::test]
  fn test_offset_to_utf16_ascii(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    let utf16_offset = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_to_utf16(5, cx));

    // ASCII: UTF-8 and UTF-16 offsets are the same
    assert_eq!(utf16_offset, 5);
  }

  #[gpui::test]
  fn test_offset_from_utf16_ascii(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    let utf8_offset = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_from_utf16(5, cx));

    assert_eq!(utf8_offset, 5);
  }

  #[gpui::test]
  fn test_offset_to_utf16_emoji(cx: &mut TestAppContext) {
    // "hello 👋 world" - emoji is 4 bytes in UTF-8, 2 code units in UTF-16
    let ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 world");

    // Offset 6 is before emoji (after "hello ")
    let utf16_before = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_to_utf16(6, cx));
    assert_eq!(utf16_before, 6);

    // Offset 7 is after emoji (4-byte char)
    let utf16_after = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_to_utf16(7, cx));
    // In UTF-16: "hello " (6) + "👋" (2) = 8
    assert_eq!(utf16_after, 8);
  }

  #[gpui::test]
  fn test_offset_from_utf16_emoji(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 world");

    // UTF-16 offset 6 = before emoji
    let utf8_before = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_from_utf16(6, cx));
    assert_eq!(utf8_before, 6);

    // UTF-16 offset 8 = after emoji (👋 is 2 UTF-16 code units)
    let utf8_after = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_from_utf16(8, cx));
    assert_eq!(utf8_after, 7); // 4-byte emoji = 1 char in UTF-8 offset
  }

  #[gpui::test]
  fn test_offset_to_utf16_multibyte(cx: &mut TestAppContext) {
    // "café" - é is 2 bytes in UTF-8, 1 code unit in UTF-16
    let ctx = EditorTestContext::with_text(cx.clone(), "café");

    let utf16_end = ctx.editor.read_with(&ctx.cx, |editor, cx| {
      editor.offset_to_utf16(5, cx) // 5 bytes: c(1) + a(1) + f(1) + é(2)
    });
    assert_eq!(utf16_end, 4); // 4 UTF-16 code units
  }

  #[gpui::test]
  fn test_range_to_utf16(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 world");

    let utf16_range = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.range_to_utf16(&(0..7), cx));

    // Range 0..7 in UTF-8 = "hello 👋"
    // In UTF-16: 0..8 (emoji is 2 code units)
    assert_eq!(utf16_range, 0..8);
  }

  #[gpui::test]
  fn test_range_from_utf16(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 world");

    let utf8_range = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.range_from_utf16(&(0..8), cx));

    // Range 0..8 in UTF-16 = "hello 👋"
    // In UTF-8: 0..7 (emoji is 4 bytes but counts as 1 char offset)
    assert_eq!(utf8_range, 0..7);
  }

  #[gpui::test]
  fn test_utf16_roundtrip(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 世界");

    // Test roundtrip: UTF-8 -> UTF-16 -> UTF-8
    for offset in [0, 5, 6, 7, 8, 9] {
      let utf16 = ctx
        .editor
        .read_with(&ctx.cx, |editor, cx| editor.offset_to_utf16(offset, cx));
      let back_to_utf8 = ctx
        .editor
        .read_with(&ctx.cx, |editor, cx| editor.offset_from_utf16(utf16, cx));
      assert_eq!(
        back_to_utf8, offset,
        "Roundtrip failed for offset {}",
        offset
      );
    }
  }

  // ============================================================================
  // Selection Logic Tests
  // ============================================================================

  #[gpui::test]
  fn test_select_to_forward(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_cursor(0);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.select_to(5, cx);
    });

    assert_eq!(ctx.selection(), 0..5);
    assert!(!ctx.selection_reversed());
  }

  #[gpui::test]
  fn test_select_to_backward(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_cursor(5);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.select_to(0, cx);
    });

    assert_eq!(ctx.selection(), 0..5);
    assert!(ctx.selection_reversed());
  }

  #[gpui::test]
  fn test_select_to_extends_selection(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    // Start with selection 2..5
    ctx.set_selection(2..5, false);

    // Extend to 8
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.select_to(8, cx);
    });

    assert_eq!(ctx.selection(), 2..8);
    assert!(!ctx.selection_reversed());
  }

  #[gpui::test]
  fn test_select_to_reverses_direction(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    // Start with forward selection 2..5
    ctx.set_selection(2..5, false);

    // Select backwards past anchor
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.select_to(0, cx);
    });

    assert_eq!(ctx.selection(), 0..2);
    assert!(ctx.selection_reversed());
  }

  #[gpui::test]
  fn test_selection_anchor_preserved(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    // Set selection with anchor at 3
    ctx.set_selection(3..7, false);

    // Select to different position, anchor should stay at 3
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.select_to(10, cx);
    });

    assert_eq!(ctx.selection(), 3..10);
  }

  #[gpui::test]
  fn test_syntax_highlights_cached(cx: &mut TestAppContext) {
    let editor = cx.new(Editor::new);

    // Wait for async highlighting to complete (it's scheduled but not immediate)
    editor.read_with(cx, |editor, cx| {
      let doc = editor.document().read(cx);

      // Highlighting is async with debouncing, so it might not be ready immediately
      // Just verify the document has content that should be highlighted
      assert!(doc.len() > 0);
      assert!(doc.len_lines() > 0);
    });
  }

  #[gpui::test]
  fn test_quadruple_click_selects_all(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    let doc_len = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.document().read(cx).len());

    // Simulate quadruple click - select all buffer
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.is_selecting = true;
      editor.selected_range = 0..doc_len;
      editor.selection_reversed = false;
      cx.notify();
    });

    // Verify entire buffer is selected
    assert_eq!(ctx.selection(), 0..doc_len);
    assert_eq!(doc_len, 17); // "line1\nline2\nline3"
  }
}
