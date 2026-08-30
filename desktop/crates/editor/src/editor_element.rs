use gpui::{
  App, Bounds, ContentMask, CursorStyle, DispatchPhase, ElementId, ElementInputHandler, Entity,
  GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, LayoutId, MouseButton,
  MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Path, PathBuilder, Pixels, Point,
  ScrollDelta, ScrollWheelEvent, ShapedLine, Style, TextAlign, TextRun, TextStyle, Window, fill,
  point, prelude::*, px, relative, rems, size,
};
use std::{
  collections::{HashMap, HashSet},
  ops::Range,
  rc::Rc,
  sync::Arc,
  time::{Duration, Instant},
};

use diff_core::{merge_ranges, word_diff_ranges};
use git::DiffLineKind;

use crate::{
  document::Document,
  editor::{
    ConflictLineKind, DEFAULT_MAX_LINE_WIDTH, DisplayCursor, Editor, GroupOverlay,
    REVIEW_COMMENT_COMPOSER_LINE_HEIGHT_REMS, REVIEW_COMMENT_UI_FONT_FAMILY, ScrollAxis,
  },
  projection::{
    ChangeKind, DisplayLine, HunkState, NO_NEWLINE_MARKER_TEXT, Projection, ProjectionBlock,
    ProjectionBlockMap, ReviewCommentBackground, ReviewCommentSide,
  },
  settings::indent_rainbow_enabled,
  text_offsets::{byte_offset_to_char_offset, char_offset_to_byte_offset},
};
use gpui_component::ActiveTheme as _;
use syntax::HighlightSpan;
use ui::Theme;

const NEWLINE_SELECTION_WIDTH: f32 = 4.0;
const PIXEL_SCROLL_DIVISOR: f32 = 20.0;
const LINE_SCROLL_MULTIPLIER: f32 = 3.0;
const SCROLL_AXIS_RATIO: f32 = 1.1;
const SCROLL_AXIS_SWITCH_RATIO: f32 = 1.4;
const SCROLL_AXIS_TIMEOUT_MS: u64 = 150;
const DIAGONAL_STRIPE_SPACING: f32 = 6.0;
const DIAGONAL_STRIPE_WIDTH: f32 = 1.0;
const INDENT_GUIDE_BORDER_WIDTH: f32 = 1.0;
const INDENT_RAINBOW_BLOCK_COLUMNS: usize = 2;
const CONFLICT_MARKER_ALPHA_MULTIPLIER: f32 = 1.35;
const EDITOR_CHAR_WIDTH_SAMPLE: &str =
  "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_";
const REVIEW_COMMENT_CHAR_WIDTH_SAMPLE: &str =
  "the quick brown fox jumps over the lazy dog, with spaces and punctuation. ";

fn indent_guide_byte_ranges(text: &str, tab_spaces: usize) -> Vec<Range<usize>> {
  if tab_spaces == 0 {
    return Vec::new();
  }

  let mut columns = 0usize;
  let mut ranges = Vec::new();
  let mut previous_boundary = 0usize;
  for (idx, ch) in text.char_indices() {
    match ch {
      ' ' => {
        columns += 1;
        if columns.is_multiple_of(tab_spaces) {
          let boundary = idx + 1;
          if boundary > previous_boundary {
            ranges.push(previous_boundary..boundary);
            previous_boundary = boundary;
          }
        }
      }
      '\t' => {
        let next_tab_stop = ((columns / tab_spaces) + 1) * tab_spaces;
        while columns < next_tab_stop {
          columns += 1;
          if columns.is_multiple_of(tab_spaces) {
            let boundary = idx + 1;
            if boundary > previous_boundary {
              ranges.push(previous_boundary..boundary);
              previous_boundary = boundary;
            }
          }
        }
      }
      _ => break,
    }
  }

  ranges
}

fn indent_guide_border_color(fill_color: gpui::Hsla) -> gpui::Hsla {
  let mut color = fill_color;
  color.a = (fill_color.a + 0.06).min(0.28);
  color
}

fn line_y(
  bounds_top: Pixels,
  line_height: Pixels,
  display_line: usize,
  scroll_offset: f32,
) -> Pixels {
  bounds_top + line_height * (display_line as f32 - scroll_offset)
}

fn conflict_doc_line(display_line: &DisplayLine) -> Option<usize> {
  match display_line {
    DisplayLine::Doc { doc_line, .. } | DisplayLine::Modified { doc_line, .. } => Some(*doc_line),
    _ => None,
  }
}

fn conflict_background(theme: &Theme, kind: ConflictLineKind) -> Option<gpui::Hsla> {
  match kind {
    ConflictLineKind::Current => Some(theme.current_conflict_background()),
    ConflictLineKind::CurrentMarker => {
      let mut color = theme.current_conflict_background();
      color.a = (color.a * CONFLICT_MARKER_ALPHA_MULTIPLIER).min(1.0);
      Some(color)
    }
    ConflictLineKind::Divider => None,
    ConflictLineKind::Incoming => Some(theme.incoming_conflict_background()),
    ConflictLineKind::IncomingMarker => {
      let mut color = theme.incoming_conflict_background();
      color.a = (color.a * CONFLICT_MARKER_ALPHA_MULTIPLIER).min(1.0);
      Some(color)
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConflictBlockKind {
  Current,
  Incoming,
}

fn conflict_block_kind(kind: ConflictLineKind) -> Option<ConflictBlockKind> {
  match kind {
    ConflictLineKind::Current | ConflictLineKind::CurrentMarker => Some(ConflictBlockKind::Current),
    ConflictLineKind::Incoming | ConflictLineKind::IncomingMarker => {
      Some(ConflictBlockKind::Incoming)
    }
    ConflictLineKind::Divider => None,
  }
}

fn conflict_border_color(theme: &Theme, kind: ConflictLineKind) -> Option<gpui::Hsla> {
  match conflict_block_kind(kind)? {
    ConflictBlockKind::Current => Some(theme.current_conflict_stripe()),
    ConflictBlockKind::Incoming => Some(theme.incoming_conflict_stripe()),
  }
}

fn conflict_border_edges(
  previous: Option<ConflictLineKind>,
  current: ConflictLineKind,
  next: Option<ConflictLineKind>,
) -> Option<(bool, bool)> {
  let current_block = conflict_block_kind(current)?;
  Some((
    previous.and_then(conflict_block_kind) != Some(current_block),
    next.and_then(conflict_block_kind) != Some(current_block),
  ))
}

fn conflict_doc_line_for_display_line(
  display_line: usize,
  projection: Option<&Projection>,
  doc_line_count: usize,
) -> Option<usize> {
  if let Some(projection) = projection {
    projection
      .lines
      .get(display_line)
      .and_then(conflict_doc_line)
  } else if display_line < doc_line_count {
    Some(display_line)
  } else {
    None
  }
}

fn conflict_kind_for_display_line(
  display_line: usize,
  projection: Option<&Projection>,
  doc_line_count: usize,
  conflict_line_kinds: &HashMap<usize, ConflictLineKind>,
) -> Option<ConflictLineKind> {
  let doc_line = conflict_doc_line_for_display_line(display_line, projection, doc_line_count)?;
  conflict_line_kinds.get(&doc_line).copied()
}

/// Encapsulates layout information for mouse position -> text offset conversion
#[derive(Clone)]
pub struct PositionMap {
  pub shaped_lines: Vec<(usize, Arc<ShapedLine>)>,
  pub line_texts: HashMap<usize, String>,
  pub bounds: Bounds<Pixels>,
  pub line_height: Pixels,
  pub viewport: Range<usize>,
  pub scroll_offset: f32,
  pub projection: Option<Arc<Projection>>,
  pub block_map: ProjectionBlockMap,
}

impl PositionMap {
  pub fn display_line_for_position(&self, position: Point<Pixels>) -> Option<usize> {
    if !self.bounds.contains(&position) {
      return None;
    }

    let y_offset = position.y - self.bounds.top();
    let line_float = self.scroll_offset + (y_offset / self.line_height);
    if line_float.is_sign_negative() {
      return None;
    }
    let mut display_line = line_float.floor() as usize;
    if display_line >= self.viewport.end {
      display_line = self.viewport.end.saturating_sub(1);
    }
    Some(display_line)
  }

  pub fn display_cursor_for_position(&self, position: Point<Pixels>) -> Option<DisplayCursor> {
    if !self.bounds.contains(&position) {
      return None;
    }

    let y_offset = position.y - self.bounds.top();
    let line_float = self.scroll_offset + (y_offset / self.line_height);
    if line_float.is_sign_negative() {
      return None;
    }
    let mut actual_row = line_float.floor() as usize;
    if actual_row >= self.viewport.end {
      actual_row = self.viewport.end.saturating_sub(1);
    }

    let x_offset = position.x - self.bounds.left();
    let byte_column = self
      .shaped_lines
      .iter()
      .find(|(idx, _)| *idx == actual_row)
      .map(|(_, shaped)| shaped.closest_index_for_x(x_offset))
      .unwrap_or(0);
    let column = self
      .line_texts
      .get(&actual_row)
      .map(|text| byte_offset_to_char_offset(text, byte_column))
      .unwrap_or(byte_column);

    Some(DisplayCursor {
      line: actual_row,
      column,
    })
  }

  pub fn point_for_position(&self, position: Point<Pixels>, document: &Document) -> Option<usize> {
    if !self.bounds.contains(&position) {
      return None;
    }

    if document.is_empty() {
      return Some(0);
    }

    let y_offset = position.y - self.bounds.top();
    let line_float = self.scroll_offset + (y_offset / self.line_height);
    if line_float.is_sign_negative() {
      return None;
    }
    let mut actual_row = line_float.floor() as usize;
    if actual_row >= self.viewport.end {
      actual_row = self.viewport.end.saturating_sub(1);
    }

    let doc_line = if let Some(projection) = &self.projection {
      projection.display_to_doc_line(actual_row)?
    } else {
      actual_row
    };

    if doc_line >= document.len_lines() {
      return Some(document.len());
    }

    let shaped = self
      .shaped_lines
      .iter()
      .find(|(idx, _)| *idx == actual_row)
      .map(|(_, s)| s)?;

    let x_offset = position.x - self.bounds.left();
    let byte_column = shaped.closest_index_for_x(x_offset);
    let column = document
      .line_content(doc_line)
      .map(|line| byte_offset_to_char_offset(line.as_ref(), byte_column))
      .unwrap_or(byte_column);

    let line_start = document.line_to_char(doc_line);
    Some(line_start + column)
  }
}

fn position_hits_review_comment_line(position_map: &PositionMap, position: Point<Pixels>) -> bool {
  let Some(display_line) = position_map.display_line_for_position(position) else {
    return false;
  };
  position_map
    .block_map
    .is_review_comment_display_line(display_line)
}

pub(crate) fn highlights_to_text_runs(
  highlights: &[HighlightSpan],
  line_text: &str,
  theme: &Theme,
  base_style: &TextStyle,
) -> Vec<TextRun> {
  syntax::highlights_to_text_runs(
    highlights,
    line_text,
    base_style.color,
    base_style.font(),
    &theme.syntax(),
  )
}

#[derive(Clone, Debug, Default)]
pub(crate) struct WordDiffCache {
  entries: HashMap<usize, WordDiffCacheEntry>,
}

impl WordDiffCache {
  fn insert_removed(&mut self, display_idx: usize, ranges: Vec<Range<usize>>) {
    if !ranges.is_empty() {
      self.entries.entry(display_idx).or_default().removed_ranges = Arc::from(ranges);
    }
  }

  fn insert_added(&mut self, display_idx: usize, ranges: Vec<Range<usize>>) {
    if !ranges.is_empty() {
      self.entries.entry(display_idx).or_default().added_ranges = Arc::from(ranges);
    }
  }

  pub(crate) fn len(&self) -> usize {
    self.entries.len()
  }

  fn get(&self, display_idx: usize) -> Option<&WordDiffCacheEntry> {
    self.entries.get(&display_idx)
  }
}

#[derive(Clone, Debug, Default)]
struct WordDiffCacheEntry {
  removed_ranges: Arc<[Range<usize>]>,
  added_ranges: Arc<[Range<usize>]>,
}

#[derive(Clone, Debug)]
struct WordDiffStyle {
  ranges: Arc<[Range<usize>]>,
  background: gpui::Hsla,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineDiffKind {
  Added,
  Removed,
}

fn clean_line_text(text: &str) -> String {
  if text.contains('\n') || text.contains('\r') {
    text.replace(['\n', '\r'], "")
  } else {
    text.to_string()
  }
}

#[doc(hidden)]
pub fn benchmark_word_diff_ranges(
  old_text: &str,
  new_text: &str,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
  word_diff_ranges(old_text, new_text)
}

fn apply_background_ranges(
  runs: Vec<TextRun>,
  ranges: &[Range<usize>],
  background: gpui::Hsla,
) -> Vec<TextRun> {
  if ranges.is_empty() {
    return runs;
  }

  let ranges = merge_ranges(ranges.to_vec());
  let mut result = Vec::new();
  let mut range_index = 0;
  let mut current_range = ranges.get(range_index);
  let mut pos = 0;

  for run in runs {
    let run_start = pos;
    let run_end = pos + run.len;
    let mut cursor = run_start;

    while let Some(range) = current_range {
      if range.end <= run_start {
        range_index += 1;
        current_range = ranges.get(range_index);
        continue;
      }
      if range.start >= run_end {
        break;
      }

      let overlap_start = range.start.max(run_start);
      let overlap_end = range.end.min(run_end);

      if overlap_start > cursor {
        result.push(TextRun {
          len: overlap_start - cursor,
          font: run.font.clone(),
          color: run.color,
          background_color: run.background_color,
          underline: run.underline,
          strikethrough: run.strikethrough,
        });
      }

      if overlap_end > overlap_start {
        result.push(TextRun {
          len: overlap_end - overlap_start,
          font: run.font.clone(),
          color: run.color,
          background_color: Some(background),
          underline: run.underline,
          strikethrough: run.strikethrough,
        });
      }

      cursor = overlap_end;

      if range.end <= run_end {
        range_index += 1;
        current_range = ranges.get(range_index);
      } else {
        break;
      }
    }

    if cursor < run_end {
      result.push(TextRun {
        len: run_end - cursor,
        font: run.font.clone(),
        color: run.color,
        background_color: run.background_color,
        underline: run.underline,
        strikethrough: run.strikethrough,
      });
    }

    pos = run_end;
  }

  result
}

fn inline_diff_key(line: &DisplayLine) -> Option<HunkState> {
  match line {
    DisplayLine::Removed { hunk, .. } => Some(*hunk),
    DisplayLine::Doc {
      change: Some(ChangeKind::Added),
      hunk: Some(hunk),
      ..
    } => Some(*hunk),
    _ => None,
  }
}

fn inline_diff_kind(line: &DisplayLine) -> Option<InlineDiffKind> {
  match line {
    DisplayLine::Removed { .. } => Some(InlineDiffKind::Removed),
    DisplayLine::Doc {
      change: Some(ChangeKind::Added),
      ..
    } => Some(InlineDiffKind::Added),
    _ => None,
  }
}

pub(crate) fn build_word_diff_cache(projection: &Projection, document: &Document) -> WordDiffCache {
  let mut cache = WordDiffCache::default();

  for (display_idx, display_line) in projection.lines.iter().enumerate() {
    if let DisplayLine::Modified {
      old_text, doc_line, ..
    } = display_line
    {
      let old_text = clean_line_text(old_text);
      let new_text = document
        .line_content(*doc_line)
        .map(|cow| clean_line_text(&cow))
        .unwrap_or_default();
      let (removed_ranges, added_ranges) = word_diff_ranges(&old_text, &new_text);
      cache.insert_removed(display_idx, removed_ranges);
      cache.insert_added(display_idx, added_ranges);
    }
  }

  let mut idx = 0;
  while idx < projection.lines.len() {
    let Some(key) = projection.lines.get(idx).and_then(inline_diff_key) else {
      idx += 1;
      continue;
    };
    if projection
      .lines
      .get(idx)
      .and_then(inline_diff_kind)
      .is_none()
    {
      idx += 1;
      continue;
    }
    let start = idx;
    while idx < projection.lines.len()
      && projection.lines.get(idx).and_then(inline_diff_key) == Some(key)
      && projection
        .lines
        .get(idx)
        .and_then(inline_diff_kind)
        .is_some()
    {
      idx += 1;
    }
    fill_inline_word_diff_cache(start..idx, projection, document, &mut cache);
  }

  cache
}

fn fill_inline_word_diff_cache(
  display_range: Range<usize>,
  projection: &Projection,
  document: &Document,
  cache: &mut WordDiffCache,
) {
  let mut removed_indices = Vec::new();
  let mut added_indices = Vec::new();
  for display_idx in display_range {
    match projection.lines.get(display_idx).and_then(inline_diff_kind) {
      Some(InlineDiffKind::Removed) => removed_indices.push(display_idx),
      Some(InlineDiffKind::Added) => added_indices.push(display_idx),
      None => {}
    }
  }

  for (removed_idx, added_idx) in removed_indices.into_iter().zip(added_indices) {
    let removed_text = match projection.lines.get(removed_idx) {
      Some(DisplayLine::Removed { text, .. }) => clean_line_text(text),
      _ => continue,
    };
    let added_text = match projection.lines.get(added_idx) {
      Some(DisplayLine::Doc { doc_line, .. }) => document
        .line_content(*doc_line)
        .map(|cow| clean_line_text(&cow))
        .unwrap_or_default(),
      _ => continue,
    };
    let (removed_ranges, added_ranges) = word_diff_ranges(&removed_text, &added_text);
    cache.insert_removed(removed_idx, removed_ranges);
    cache.insert_added(added_idx, added_ranges);
  }
}

fn word_diff_for_line(
  display_idx: usize,
  display_line: &DisplayLine,
  diff_view: DiffElementView,
  cache: &WordDiffCache,
  theme: &Theme,
) -> Option<WordDiffStyle> {
  let entry = cache.get(display_idx)?;
  match display_line {
    DisplayLine::Modified { .. } => match diff_view {
      DiffElementView::SplitLeft if !entry.removed_ranges.is_empty() => Some(WordDiffStyle {
        ranges: Arc::clone(&entry.removed_ranges),
        background: theme.diff_word_removed_background(),
      }),
      DiffElementView::SplitRight if !entry.added_ranges.is_empty() => Some(WordDiffStyle {
        ranges: Arc::clone(&entry.added_ranges),
        background: theme.diff_word_added_background(),
      }),
      _ => None,
    },
    DisplayLine::Removed { .. } if matches!(diff_view, DiffElementView::Inline) => {
      (!entry.removed_ranges.is_empty()).then(|| WordDiffStyle {
        ranges: Arc::clone(&entry.removed_ranges),
        background: theme.diff_word_removed_background(),
      })
    }
    DisplayLine::Doc {
      change: Some(ChangeKind::Added),
      ..
    } if matches!(diff_view, DiffElementView::Inline) => {
      (!entry.added_ranges.is_empty()).then(|| WordDiffStyle {
        ranges: Arc::clone(&entry.added_ranges),
        background: theme.diff_word_added_background(),
      })
    }
    _ => None,
  }
}

fn collect_word_diffs_for_viewport(
  viewport_lines: &[(usize, DisplayLine)],
  diff_view: DiffElementView,
  cache: &WordDiffCache,
  theme: &Theme,
) -> HashMap<usize, WordDiffStyle> {
  viewport_lines
    .iter()
    .filter_map(|(display_idx, display_line)| {
      word_diff_for_line(*display_idx, display_line, diff_view, cache, theme)
        .map(|word_diff| (*display_idx, word_diff))
    })
    .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorElementRole {
  Primary,
  Secondary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiffElementView {
  Inline,
  SplitLeft,
  SplitRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineVisibility {
  Text,
  Blank,
}

fn line_visibility_for_view(
  diff_view: DiffElementView,
  display_line: &DisplayLine,
  block: Option<&ProjectionBlock>,
) -> LineVisibility {
  let review_comment_side = block.and_then(ProjectionBlock::review_comment_side);
  match diff_view {
    DiffElementView::Inline => LineVisibility::Text,
    DiffElementView::SplitLeft => match display_line {
      DisplayLine::Doc {
        change: Some(ChangeKind::Added),
        ..
      } => LineVisibility::Blank,
      _ if review_comment_side == Some(ReviewCommentSide::Right) => LineVisibility::Blank,
      _ => LineVisibility::Text,
    },
    DiffElementView::SplitRight => match display_line {
      DisplayLine::Removed { .. } => LineVisibility::Blank,
      _ if review_comment_side == Some(ReviewCommentSide::Left) => LineVisibility::Blank,
      _ => LineVisibility::Text,
    },
  }
}

fn position_hits_blank_line(
  position_map: &PositionMap,
  position: Point<Pixels>,
  diff_view: DiffElementView,
) -> bool {
  let Some(display_line) = position_map.display_line_for_position(position) else {
    return false;
  };
  let Some(line) = position_map
    .projection
    .as_ref()
    .and_then(|projection| projection.lines.get(display_line))
  else {
    return false;
  };
  let block = position_map.block_map.block_at_display_line(display_line);
  line_visibility_for_view(diff_view, line, block) == LineVisibility::Blank
}

fn group_id_for_display_line<'a>(
  display_idx: usize,
  projection: Option<&'a Projection>,
  block_map: &'a ProjectionBlockMap,
) -> Option<&'a Arc<str>> {
  block_map
    .block_at_display_line(display_idx)
    .and_then(|block| block.group_id.as_ref())
    .or_else(|| {
      projection
        .and_then(|projection| projection.lines.get(display_idx))
        .and_then(|line| match line {
          DisplayLine::Doc { group_id, .. } => group_id.as_ref(),
          DisplayLine::Modified { group_id, .. } => group_id.as_ref(),
          DisplayLine::Removed { group_id, .. } => group_id.as_ref(),
          DisplayLine::NoNewline { group_id, .. } => group_id.as_ref(),
          _ => None,
        })
    })
}

fn hunk_border_colors_for_kinds(
  theme: &Theme,
  diff_view: DiffElementView,
  kinds: impl IntoIterator<Item = DiffLineKind>,
) -> Option<(gpui::Hsla, gpui::Hsla)> {
  let mut has_add = false;
  let mut has_remove = false;
  let mut first_kind = None;
  let mut last_kind = None;

  for kind in kinds {
    match kind {
      DiffLineKind::Add => {
        has_add = true;
        first_kind.get_or_insert(kind);
        last_kind = Some(kind);
      }
      DiffLineKind::Remove => {
        has_remove = true;
        first_kind.get_or_insert(kind);
        last_kind = Some(kind);
      }
      DiffLineKind::Context => {}
    }
  }

  if has_add && has_remove {
    let removed = theme.diff_gutter_removed();
    let added = theme.diff_gutter_added();
    return Some(match diff_view {
      DiffElementView::SplitLeft => (removed, removed),
      DiffElementView::SplitRight => (added, added),
      DiffElementView::Inline => (removed, added),
    });
  }

  let color_for_kind = |kind| match kind {
    DiffLineKind::Add => theme.diff_gutter_added(),
    DiffLineKind::Remove => theme.diff_gutter_removed(),
    DiffLineKind::Context => theme.diff_gutter_modified(),
  };

  Some((color_for_kind(first_kind?), color_for_kind(last_kind?)))
}

fn display_line_text_for_view(
  display_line: &DisplayLine,
  diff_view: DiffElementView,
  document: &Document,
) -> String {
  match display_line {
    DisplayLine::Doc { doc_line, .. } => document
      .line_content(*doc_line)
      .map(|cow| clean_line_text(&cow))
      .unwrap_or_default(),
    DisplayLine::Modified {
      old_text, doc_line, ..
    } => match diff_view {
      DiffElementView::SplitLeft => clean_line_text(old_text),
      DiffElementView::SplitRight | DiffElementView::Inline => document
        .line_content(*doc_line)
        .map(|cow| clean_line_text(&cow))
        .unwrap_or_default(),
    },
    DisplayLine::Removed { text, .. } => clean_line_text(text),
    DisplayLine::NoNewline { .. } => NO_NEWLINE_MARKER_TEXT.to_string(),
    _ => String::new(),
  }
}

pub struct EditorElement {
  editor: Entity<Editor>,
  diff_view: DiffElementView,
  role: EditorElementRole,
}

pub struct PrepaintState {
  shaped_lines: Vec<(usize, Arc<ShapedLine>)>,
  line_texts: HashMap<usize, String>,
  indent_guides: Vec<PaintQuad>,
  line_backgrounds: Vec<PaintQuad>,
  gap_separators: Vec<PaintQuad>,
  word_diff_quads: Vec<PaintQuad>,
  conflict_borders: Vec<PaintQuad>,
  group_borders: Vec<PaintQuad>,
  diag_paths: Vec<Path<Pixels>>,
  cursor_quad: Option<PaintQuad>,
  selection_quads: Vec<PaintQuad>,
  viewport: Range<usize>,
  bounds: Bounds<Pixels>,
  line_height: Pixels,
  scroll_offset: f32,
  scroll_hitbox: Hitbox,
  projection: Option<Arc<Projection>>,
  block_map: ProjectionBlockMap,
}

impl EditorElement {
  pub fn new(editor: Entity<Editor>) -> Self {
    Self {
      editor,
      diff_view: DiffElementView::Inline,
      role: EditorElementRole::Primary,
    }
  }

  fn diff_view(mut self, diff_view: DiffElementView) -> Self {
    self.diff_view = diff_view;
    self
  }

  fn role(mut self, role: EditorElementRole) -> Self {
    self.role = role;
    self
  }

  pub fn split_left(editor: Entity<Editor>) -> Self {
    Self::new(editor)
      .diff_view(DiffElementView::SplitLeft)
      .role(EditorElementRole::Secondary)
  }

  pub fn split_right(editor: Entity<Editor>) -> Self {
    Self::new(editor).diff_view(DiffElementView::SplitRight)
  }

  fn calculate_viewport(
    &self,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    scroll_offset: f32,
    total_lines: usize,
  ) -> Range<usize> {
    Editor::viewport_range_for_height(scroll_offset, bounds.size.height, line_height, total_lines)
  }

  fn is_primary(&self) -> bool {
    self.role == EditorElementRole::Primary
  }

  fn line_visibility(
    &self,
    display_line: &DisplayLine,
    block: Option<&ProjectionBlock>,
  ) -> LineVisibility {
    line_visibility_for_view(self.diff_view, display_line, block)
  }
}

impl IntoElement for EditorElement {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for EditorElement {
  type RequestLayoutState = ();
  type PrepaintState = PrepaintState;

  fn id(&self) -> Option<ElementId> {
    None
  }

  fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
    None
  }

  fn request_layout(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    let mut style = Style::default();
    style.size.width = relative(1.).into();
    style.size.height = relative(1.).into();

    (window.request_layout(style, [], cx), ())
  }

  fn prepaint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    window: &mut Window,
    cx: &mut App,
  ) -> Self::PrepaintState {
    let measured_line_height = window.line_height();
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let scroll_hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
    let is_primary = self.is_primary();

    let (highlights_epoch, highlights_version, dirty_highlight_lines) = {
      let document = self.editor.read(cx).document().read(cx);
      let epoch = *document.highlights_epoch.read();
      let version = *document.highlights_version.read();
      let dirty = document.drain_dirty_highlight_lines();
      (epoch, version, dirty)
    };
    self.editor.update(cx, |editor, cx| {
      editor.editor_line_height = measured_line_height;
      editor.invalidate_layout_cache_if_font_size_changed(font_size);
      if is_primary {
        editor.viewport_height = bounds.size.height;
        if editor.viewport_width != bounds.size.width {
          editor.viewport_width = bounds.size.width;
          // Review comment cards follow the content width; re-render to resize them.
          cx.notify();
        }
      }

      if editor.scroll_axis_lock == Some(ScrollAxis::Vertical)
        && editor.scroll_handle.offset().x != editor.clamp_horizontal_scroll_x(editor.last_scroll_x)
      {
        editor.last_scroll_x = editor.clamp_horizontal_scroll_x(editor.last_scroll_x);
        editor
          .scroll_handle
          .set_offset(point(editor.last_scroll_x, px(0.0)));
      }

      if highlights_epoch > editor.last_highlights_epoch {
        editor.line_layouts.clear();
        editor.last_highlights_epoch = highlights_epoch;
        editor.last_highlights_version = highlights_version;
      } else if highlights_version > editor.last_highlights_version {
        for line_idx in &dirty_highlight_lines {
          editor.line_layouts.remove(line_idx);
        }
        editor.last_highlights_version = highlights_version;
      }
    });

    let (
      viewport,
      selected_range,
      display_selection,
      cursor_offset,
      scroll_offset,
      mut shaped_lines,
      lines_to_shape,
      viewport_lines,
      projection,
      block_map,
    ) = {
      let editor = self.editor.read(cx);
      let document = editor.document().read(cx);
      let line_height = measured_line_height;
      let scroll_offset = editor.scroll_offset_y;
      let doc_line_count = document.len_lines();
      let total_lines = editor.display_line_count(doc_line_count);

      let viewport = self.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

      let mut lines_to_shape = Vec::new();
      let mut shaped_lines = Vec::new();
      let mut viewport_lines = Vec::new();
      let projection = editor.projection.clone();
      let block_map = editor.block_map.clone();

      for display_idx in viewport.clone() {
        let Some(display_line) = editor.display_line(display_idx, doc_line_count) else {
          continue;
        };
        viewport_lines.push((display_idx, display_line.clone()));

        let doc_line_for_layout = match &display_line {
          DisplayLine::Doc { doc_line, .. } => Some(*doc_line),
          DisplayLine::Modified { doc_line, .. }
            if matches!(
              self.diff_view,
              DiffElementView::SplitRight | DiffElementView::Inline
            ) =>
          {
            Some(*doc_line)
          }
          _ => None,
        };

        if let Some(doc_line) = doc_line_for_layout {
          match editor.line_layouts.get(&doc_line) {
            Some(shaped) => {
              shaped_lines.push((display_idx, Arc::clone(shaped)));
            }
            None => {
              lines_to_shape.push((display_idx, display_line));
            }
          }
        } else {
          match editor.virtual_line_layouts.get(&display_idx) {
            Some(shaped) => {
              shaped_lines.push((display_idx, Arc::clone(shaped)));
            }
            None => {
              lines_to_shape.push((display_idx, display_line));
            }
          }
        }
      }

      (
        viewport,
        editor.selected_range.clone(),
        editor.display_selection.clone(),
        editor.cursor_offset(),
        scroll_offset,
        shaped_lines,
        lines_to_shape,
        viewport_lines,
        projection,
        block_map,
      )
    };

    let measure_char_width = |style: &TextStyle, sample: &'static str| {
      let font_size = style.font_size.to_pixels(window.rem_size());
      let sample_runs = vec![TextRun {
        len: sample.len(),
        font: style.font(),
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
      }];
      let shaped = window
        .text_system()
        .shape_line(sample.into(), font_size, &sample_runs, None);
      let sample_chars = sample.chars().count().max(1) as f32;
      (shaped.x_for_index(sample.len()) / sample_chars).max(px(1.0))
    };
    let measured_char_width = measure_char_width(&style, EDITOR_CHAR_WIDTH_SAMPLE);
    let mut review_comment_style = style.clone();
    review_comment_style.font_family = REVIEW_COMMENT_UI_FONT_FAMILY.into();
    review_comment_style.font_size = rems(0.875).into();
    let measured_review_comment_char_width =
      measure_char_width(&review_comment_style, REVIEW_COMMENT_CHAR_WIDTH_SAMPLE);
    let measured_review_comment_line_height = review_comment_style
      .line_height_in_pixels(window.rem_size())
      .max(px(1.0));
    self.editor.update(cx, |editor, cx| {
      editor.editor_char_width = measured_char_width;
      editor.review_comment_char_width = measured_review_comment_char_width;
      editor.review_comment_font_size = review_comment_style.font_size.to_pixels(window.rem_size());
      editor.review_comment_composer_line_height_px =
        (window.rem_size() * REVIEW_COMMENT_COMPOSER_LINE_HEIGHT_REMS) / px(1.0);
      editor.set_review_comment_line_height_px(
        (measured_review_comment_line_height / px(1.0)).max(1.0),
        cx,
      );
    });
    let line_height = measured_line_height;

    let theme = self.editor.read(cx).theme.clone();

    let document_entity = self.editor.read(cx).document().clone();
    let mut newly_shaped = Vec::new();
    let word_diffs_by_display = {
      let editor = self.editor.read(cx);
      collect_word_diffs_for_viewport(
        &viewport_lines,
        self.diff_view,
        &editor.word_diff_cache,
        &theme,
      )
    };
    {
      let document = document_entity.read(cx);
      for (display_idx, display_line) in lines_to_shape {
        let (line_text, doc_line, base_color, allow_highlights) =
          if block_map.is_gap_display_line(display_idx) {
            (String::new(), None, cx.theme().muted_foreground, false)
          } else {
            match &display_line {
              DisplayLine::Doc { doc_line, .. } => {
                let content = document
                  .line_content(*doc_line)
                  .map(|cow| clean_line_text(&cow))
                  .unwrap_or_default();
                let base_color = style.color;
                (content, Some(*doc_line), base_color, true)
              }
              DisplayLine::Modified {
                old_text, doc_line, ..
              } => match self.diff_view {
                DiffElementView::SplitLeft => {
                  let old_text = clean_line_text(old_text);
                  (old_text, None, theme.diff_removed_text(), false)
                }
                DiffElementView::SplitRight => {
                  let content = document
                    .line_content(*doc_line)
                    .map(|cow| clean_line_text(&cow))
                    .unwrap_or_default();
                  (content, Some(*doc_line), style.color, true)
                }
                DiffElementView::Inline => {
                  let content = document
                    .line_content(*doc_line)
                    .map(|cow| clean_line_text(&cow))
                    .unwrap_or_default();
                  (content, Some(*doc_line), style.color, true)
                }
              },
              DisplayLine::Removed { text, .. } => {
                let color = theme.diff_removed_text();
                (clean_line_text(text), None, color, false)
              }
              DisplayLine::NoNewline { .. } => (
                NO_NEWLINE_MARKER_TEXT.to_string(),
                None,
                cx.theme().muted_foreground,
                false,
              ),
              DisplayLine::ReviewComment { .. } => {
                (String::new(), None, cx.theme().muted_foreground, false)
              }
              _ => (String::new(), None, cx.theme().muted_foreground, false),
            }
          };

        let runs = if allow_highlights {
          if let Some(doc_line) = doc_line
            && let Some(highlights) = document.get_highlights_for_line(doc_line)
          {
            highlights_to_text_runs(highlights.as_ref(), &line_text, &theme, &style)
          } else {
            vec![TextRun {
              len: line_text.len(),
              font: style.font(),
              color: base_color,
              background_color: None,
              underline: None,
              strikethrough: None,
            }]
          }
        } else {
          vec![TextRun {
            len: line_text.len(),
            font: style.font(),
            color: base_color,
            background_color: None,
            underline: None,
            strikethrough: None,
          }]
        };

        let runs = if let Some(word_diff) = word_diffs_by_display.get(&display_idx) {
          apply_background_ranges(runs, &word_diff.ranges, word_diff.background)
        } else {
          runs
        };

        let shaped = window
          .text_system()
          .shape_line(line_text.into(), font_size, &runs, None);
        newly_shaped.push((display_idx, doc_line, shaped));
      }
    }

    if !newly_shaped.is_empty() {
      self.editor.update(cx, |editor, _| {
        for (display_idx, doc_line, shaped) in newly_shaped {
          let shaped_arc = Arc::new(shaped);
          if let Some(doc_line) = doc_line {
            editor.line_layouts.insert(doc_line, shaped_arc.clone());
          } else {
            editor
              .virtual_line_layouts
              .insert(display_idx, shaped_arc.clone());
          }
          shaped_lines.push((display_idx, shaped_arc));
        }
        editor.ensure_cache_size(viewport.clone());
      });
    }

    let line_texts = {
      let document = document_entity.read(cx);
      let mut line_texts = HashMap::new();
      for (display_idx, display_line) in &viewport_lines {
        let text = display_line_text_for_view(display_line, self.diff_view, document);
        line_texts.insert(*display_idx, text);
      }
      line_texts
    };

    let mut indent_guides = Vec::new();
    if indent_rainbow_enabled() {
      let rainbow = theme.indent_rainbow_colors();
      let border_width = px(INDENT_GUIDE_BORDER_WIDTH);
      for (display_idx, display_line) in &viewport_lines {
        let block = block_map.block_at_display_line(*display_idx);
        if self.line_visibility(display_line, block) != LineVisibility::Text {
          continue;
        }
        let Some(text) = line_texts.get(display_idx) else {
          continue;
        };
        let Some((_, shaped)) = shaped_lines.iter().find(|(idx, _)| idx == display_idx) else {
          continue;
        };
        let y = line_y(bounds.top(), line_height, *display_idx, scroll_offset);
        for (depth, byte_range) in indent_guide_byte_ranges(text, INDENT_RAINBOW_BLOCK_COLUMNS)
          .into_iter()
          .enumerate()
        {
          let x_start = shaped.x_for_index(byte_range.start);
          let x_end = shaped.x_for_index(byte_range.end);
          if x_end <= x_start {
            continue;
          }
          let color = rainbow[depth % rainbow.len()];
          let border_color = indent_guide_border_color(color);
          indent_guides.push(fill(
            Bounds::from_corners(
              point(bounds.left() + x_start, y),
              point(bounds.left() + x_end, y + line_height),
            ),
            color,
          ));
          indent_guides.push(fill(
            Bounds::from_corners(
              point(bounds.left() + x_start, y),
              point(bounds.left() + x_start + border_width, y + line_height),
            ),
            border_color,
          ));
        }
      }
    }

    let max_width = shaped_lines
      .iter()
      .map(|(_, shaped)| shaped.width)
      .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
      .unwrap_or(px(DEFAULT_MAX_LINE_WIDTH));

    if is_primary {
      self.editor.update(cx, |editor, _| {
        editor.max_line_width = editor.max_line_width.max(max_width);
      });
    }

    let document = document_entity.read(cx);
    let doc_line_count = document.len_lines();
    let mut line_backgrounds = Vec::new();
    let mut gap_separators = Vec::new();
    let mut word_diff_quads = Vec::new();
    let mut conflict_borders = Vec::new();
    let mut group_borders = Vec::new();
    let mut diag_paths = Vec::new();
    let added_bg = theme.diff_added_background();
    let added_staged_bg = theme.diff_added_staged_background();
    let removed_bg = theme.diff_removed_background();
    let removed_staged_bg = theme.diff_removed_staged_background();
    let review_comment_background_for_block = |block: Option<&ProjectionBlock>| {
      let block = block?;
      match block.background? {
        ReviewCommentBackground::Added if !matches!(self.diff_view, DiffElementView::SplitLeft) => {
          Some(if block.secondary {
            added_staged_bg
          } else {
            added_bg
          })
        }
        ReviewCommentBackground::Removed
          if !matches!(self.diff_view, DiffElementView::SplitRight) =>
        {
          Some(if block.secondary {
            removed_staged_bg
          } else {
            removed_bg
          })
        }
        _ => None,
      }
    };
    let conflict_line_kinds = self.editor.read(cx).conflict_line_kinds(cx);
    let active_hunk_group_id = self.editor.read(cx).highlighted_hunk_group_id(cx);
    let active_hunk_focus_color = theme.hunk_focused_border();
    let active_conflict_doc_range = self.editor.read(cx).highlighted_conflict_doc_range(cx);
    let mut group_border_colors = HashMap::new();
    if let Some(projection) = projection.as_ref() {
      for (group_id, group) in &projection.groups {
        if group.state != HunkState::Staged {
          continue;
        }
        if let Some(colors) = hunk_border_colors_for_kinds(
          &theme,
          self.diff_view,
          group.hunk.lines.iter().map(|line| line.kind),
        ) {
          group_border_colors.insert(group_id.clone(), colors);
        }
      }
    }

    let mut blank_line_set = HashSet::new();
    if !matches!(self.diff_view, DiffElementView::Inline) {
      for (display_idx, display_line) in &viewport_lines {
        let block = block_map.block_at_display_line(*display_idx);
        if self.line_visibility(display_line, block) == LineVisibility::Blank {
          blank_line_set.insert(*display_idx);
        }
      }
    }

    let group_id_for_visible_display_line = |display_idx: usize| {
      let line = projection.as_ref()?.lines.get(display_idx)?;
      let block = block_map.block_at_display_line(display_idx);
      if self.line_visibility(line, block) == LineVisibility::Blank {
        None
      } else {
        group_id_for_display_line(display_idx, projection.as_deref(), &block_map).cloned()
      }
    };

    for (display_idx, display_line) in &viewport_lines {
      let block = block_map.block_at_display_line(*display_idx);
      if block_map
        .interior_gap_id_for_display_line(*display_idx)
        .is_some()
      {
        let y = line_y(bounds.top(), line_height, *display_idx, scroll_offset) + line_height * 0.5;
        gap_separators.push(fill(
          Bounds::new(point(bounds.left(), y), size(bounds.size.width, px(1.0))),
          cx.theme().muted_foreground.opacity(0.35),
        ));
      }

      let conflict_kind = conflict_kind_for_display_line(
        *display_idx,
        projection.as_deref(),
        doc_line_count,
        &conflict_line_kinds,
      );
      let background = if let Some(conflict_kind) = conflict_kind {
        if self.line_visibility(display_line, block) == LineVisibility::Blank {
          None
        } else {
          conflict_background(&theme, conflict_kind)
        }
      } else if let Some(background) = review_comment_background_for_block(block) {
        Some(background)
      } else {
        match display_line {
          DisplayLine::Doc {
            change: Some(ChangeKind::Added),
            secondary,
            ..
          } if !matches!(self.diff_view, DiffElementView::SplitLeft) => Some(if *secondary {
            added_staged_bg
          } else {
            added_bg
          }),
          DisplayLine::Removed { secondary, .. }
            if !matches!(self.diff_view, DiffElementView::SplitRight) =>
          {
            Some(if *secondary {
              removed_staged_bg
            } else {
              removed_bg
            })
          }
          DisplayLine::Modified { secondary, .. } => match self.diff_view {
            DiffElementView::SplitLeft => Some(if *secondary {
              removed_staged_bg
            } else {
              removed_bg
            }),
            DiffElementView::SplitRight => Some(if *secondary {
              added_staged_bg
            } else {
              added_bg
            }),
            DiffElementView::Inline => None,
          },
          _ => None,
        }
      };

      if let Some(color) = background {
        let y = line_y(bounds.top(), line_height, *display_idx, scroll_offset);
        line_backgrounds.push(fill(
          Bounds::new(
            point(bounds.left(), y),
            size(bounds.size.width, line_height),
          ),
          color,
        ));
      }

      if let Some(conflict_kind) = conflict_kind
        && let Some(default_color) = conflict_border_color(&theme, conflict_kind)
      {
        let doc_line =
          conflict_doc_line_for_display_line(*display_idx, projection.as_deref(), doc_line_count);
        let is_active_conflict = doc_line
          .zip(active_conflict_doc_range.as_ref())
          .map(|(line, range)| range.contains(&line))
          .unwrap_or(false);
        let color = if is_active_conflict {
          active_hunk_focus_color
        } else {
          default_color
        };
        let previous_conflict_kind = display_idx.checked_sub(1).and_then(|idx| {
          conflict_kind_for_display_line(
            idx,
            projection.as_deref(),
            doc_line_count,
            &conflict_line_kinds,
          )
        });
        let next_conflict_kind = conflict_kind_for_display_line(
          display_idx + 1,
          projection.as_deref(),
          doc_line_count,
          &conflict_line_kinds,
        );
        let (is_top, is_bottom) =
          conflict_border_edges(previous_conflict_kind, conflict_kind, next_conflict_kind)
            .unwrap_or((false, false));
        let border_thickness = px(1.0);
        let y = line_y(bounds.top(), line_height, *display_idx, scroll_offset);

        if is_top {
          conflict_borders.push(fill(
            Bounds::new(
              point(bounds.left(), y),
              size(bounds.size.width, border_thickness),
            ),
            color,
          ));
        }

        if is_bottom {
          conflict_borders.push(fill(
            Bounds::new(
              point(bounds.left(), y + line_height - border_thickness),
              size(bounds.size.width, border_thickness),
            ),
            color,
          ));
        }
      }

      if let Some(word_diff) = word_diffs_by_display.get(display_idx)
        && let Some((_, shaped)) = shaped_lines.iter().find(|(idx, _)| *idx == *display_idx)
      {
        let y = line_y(bounds.top(), line_height, *display_idx, scroll_offset);
        for range in word_diff.ranges.iter() {
          if range.start >= range.end {
            continue;
          }
          let x_start = shaped.x_for_index(range.start);
          let x_end = shaped.x_for_index(range.end);
          if x_end <= x_start {
            continue;
          }
          word_diff_quads.push(fill(
            Bounds::from_corners(
              point(bounds.left() + x_start, y),
              point(bounds.left() + x_end, y + line_height),
            ),
            word_diff.background,
          ));
        }
      }

      let group_id = group_id_for_visible_display_line(*display_idx);
      if conflict_kind.is_none()
        && let Some(group_id) = group_id
      {
        let is_active_hunk = active_hunk_group_id.as_deref() == Some(group_id.as_ref());
        let border_colors = if is_active_hunk {
          Some((active_hunk_focus_color, active_hunk_focus_color))
        } else {
          group_border_colors.get(group_id.as_ref()).copied()
        };

        if let Some((top_color, bottom_color)) = border_colors {
          let previous_group = display_idx
            .checked_sub(1)
            .and_then(group_id_for_visible_display_line);
          let next_group = group_id_for_visible_display_line(display_idx + 1);
          let is_top = previous_group.as_deref() != Some(group_id.as_ref());
          let is_bottom = next_group.as_deref() != Some(group_id.as_ref());
          let border_thickness = px(1.0);
          let y = line_y(bounds.top(), line_height, *display_idx, scroll_offset);

          if is_top {
            group_borders.push(fill(
              Bounds::new(
                point(bounds.left(), y),
                size(bounds.size.width, border_thickness),
              ),
              top_color,
            ));
          }

          if is_bottom {
            group_borders.push(fill(
              Bounds::new(
                point(bounds.left(), y + line_height - border_thickness),
                size(bounds.size.width, border_thickness),
              ),
              bottom_color,
            ));
          }
        }
      }
    }

    if !blank_line_set.is_empty() {
      let mut blank_ranges = Vec::new();
      let mut current_start: Option<usize> = None;
      for (display_idx, _) in &viewport_lines {
        if blank_line_set.contains(display_idx) {
          if current_start.is_none() {
            current_start = Some(*display_idx);
          }
        } else if let Some(start) = current_start.take() {
          blank_ranges.push((start, display_idx.saturating_sub(1)));
        }
      }
      if let Some(start) = current_start.take()
        && let Some((last_idx, _)) = viewport_lines.last()
      {
        blank_ranges.push((start, *last_idx));
      }

      let stripe_spacing = px(DIAGONAL_STRIPE_SPACING);
      let stripe_width = px(DIAGONAL_STRIPE_WIDTH);
      for (start, end) in blank_ranges {
        let y = line_y(bounds.top(), line_height, start, scroll_offset);
        let height = line_height * (end - start + 1) as f32;
        let top = y;
        let bottom = y + height;
        let left = bounds.left();
        let right = bounds.right();
        let mut builder = PathBuilder::stroke(stripe_width);
        let start_n = ((left - height + bottom) / stripe_spacing).floor();
        let mut x = start_n * stripe_spacing - bottom;
        let end_x = right + height;
        while x < end_x {
          builder.move_to(point(x, bottom));
          builder.line_to(point(x + height, top));
          x += stripe_spacing;
        }
        if let Ok(path) = builder.build() {
          diag_paths.push(path);
        }
      }
    }

    if !blank_line_set.is_empty() {
      shaped_lines.retain(|(idx, _)| !blank_line_set.contains(idx));
    }

    if is_primary {
      let mut overlays = Vec::new();
      let mut seen = HashSet::new();
      for (display_idx, display_line) in &viewport_lines {
        let (group_id, state) = match display_line {
          DisplayLine::Doc {
            change: Some(ChangeKind::Added),
            hunk: Some(state),
            group_id: Some(id),
            ..
          } => (id.clone(), *state),
          DisplayLine::Modified {
            hunk,
            group_id: Some(id),
            ..
          } => (id.clone(), *hunk),
          DisplayLine::Removed {
            hunk,
            group_id: Some(id),
            ..
          } => (id.clone(), *hunk),
          _ => continue,
        };

        if !seen.insert(group_id.clone()) {
          continue;
        }

        overlays.push(GroupOverlay {
          id: group_id,
          state,
          display_line: *display_idx,
        });
      }

      self.editor.update(cx, |editor, _cx| {
        editor.visible_groups = overlays;
      });
    }

    let allow_hover = is_primary || matches!(self.diff_view, DiffElementView::SplitLeft);
    if allow_hover {
      self.editor.update(cx, |editor, cx| {
        if editor.is_selecting {
          return;
        }
        let Some(position) = editor.last_mouse_position else {
          return;
        };
        if !bounds.contains(&position) {
          return;
        }
        let y_offset = position.y - bounds.top();
        let line_float = editor.scroll_offset_y + (y_offset / line_height);
        if line_float.is_sign_negative() {
          return;
        }
        let mut display_line = line_float.floor() as usize;
        if display_line >= viewport.end {
          display_line = viewport.end.saturating_sub(1);
        }
        let review_comment_side = match self.diff_view {
          DiffElementView::SplitLeft => Some(ReviewCommentSide::Left),
          DiffElementView::SplitRight => Some(ReviewCommentSide::Right),
          DiffElementView::Inline => None,
        };
        let hovered =
          editor.group_id_for_hunk_action_display_line(display_line, review_comment_side);
        let hovered_conflict = editor.conflict_start_line_for_display_line(display_line, cx);
        let mut did_change = false;
        if editor.hovered_group_id.as_deref() != hovered.as_deref() {
          editor.hovered_group_id = hovered;
          did_change = true;
        }
        if editor.hovered_conflict_start_line != hovered_conflict {
          editor.hovered_conflict_start_line = hovered_conflict;
          did_change = true;
        }
        if did_change {
          cx.notify();
        }
      });
    }

    let document = self.editor.read(cx).document().read(cx);
    let display_selection = display_selection.clone();
    let display_cursor = display_selection.as_ref().map(|selection| selection.end);

    let cursor_quad = if let Some(display_cursor) = display_cursor
      && viewport.contains(&display_cursor.line)
    {
      let shaped_opt = shaped_lines
        .iter()
        .find(|(idx, _)| *idx == display_cursor.line)
        .map(|(_, shaped)| shaped);
      let line_text = line_texts
        .get(&display_cursor.line)
        .cloned()
        .unwrap_or_default();
      if let Some(shaped) = shaped_opt {
        let line_len = line_text.chars().count();
        let cursor_in_line = display_cursor.column.min(line_len);
        let cursor_byte = char_offset_to_byte_offset(&line_text, cursor_in_line);
        let cursor_x = shaped.x_for_index(cursor_byte);
        let y = line_y(
          bounds.top(),
          line_height,
          display_cursor.line,
          scroll_offset,
        );
        Some(fill(
          Bounds::new(
            point(bounds.left() + cursor_x, y),
            size(px(2.), line_height),
          ),
          theme.cursor(),
        ))
      } else {
        None
      }
    } else {
      let cursor_doc_line = document.char_to_line(cursor_offset);
      let cursor_display_line = self.editor.read(cx).doc_to_display_line(cursor_doc_line);
      if let Some(cursor_line) = cursor_display_line
        && viewport.contains(&cursor_line)
      {
        let shaped_opt = shaped_lines
          .iter()
          .find(|(idx, _)| *idx == cursor_line)
          .map(|(_, shaped)| shaped);
        if let Some(shaped) = shaped_opt {
          let line_start = document.line_to_char(cursor_doc_line);
          let cursor_in_line = cursor_offset - line_start;
          let line_text = line_texts.get(&cursor_line).cloned().unwrap_or_default();
          let cursor_byte = char_offset_to_byte_offset(&line_text, cursor_in_line);
          let cursor_x = shaped.x_for_index(cursor_byte);
          let y = line_y(bounds.top(), line_height, cursor_line, scroll_offset);
          Some(fill(
            Bounds::new(
              point(bounds.left() + cursor_x, y),
              size(px(2.), line_height),
            ),
            theme.cursor(),
          ))
        } else {
          None
        }
      } else {
        None
      }
    };

    let mut selection_quads = Vec::new();
    let display_selection = display_selection.filter(|selection| !selection.is_empty());
    if let Some(selection) = display_selection {
      let (start, end) = selection.normalized();
      let mut end_line = end.line;
      if end.column == 0 && end.line > start.line {
        end_line = end_line.saturating_sub(1);
      }

      for display_line in start.line..=end_line {
        if !viewport.contains(&display_line) {
          continue;
        }

        let Some(line_text) = line_texts.get(&display_line).cloned() else {
          continue;
        };

        let shaped_opt = shaped_lines
          .iter()
          .find(|(idx, _)| *idx == display_line)
          .map(|(_, shaped)| shaped);

        if let Some(shaped) = shaped_opt {
          let line_len = line_text.chars().count();
          let line_start = if display_line == start.line {
            start.column.min(line_len)
          } else {
            0
          };
          let line_end = if display_line == end_line && display_line == end.line {
            end.column.min(line_len)
          } else {
            line_len
          };

          let x_start = shaped.x_for_index(char_offset_to_byte_offset(&line_text, line_start));
          let x_end = shaped.x_for_index(char_offset_to_byte_offset(&line_text, line_end));
          let y = line_y(bounds.top(), line_height, display_line, scroll_offset);

          let is_selecting_newline = display_line < end_line && x_start == x_end;
          let visual_x_end = if is_selecting_newline {
            x_end + px(NEWLINE_SELECTION_WIDTH)
          } else {
            x_end
          };

          selection_quads.push(fill(
            Bounds::from_corners(
              point(bounds.left() + x_start, y),
              point(bounds.left() + visual_x_end, y + line_height),
            ),
            theme.selection(),
          ));
        }
      }
    } else if !selected_range.is_empty() {
      let sel_start = selected_range.start;
      let sel_end = selected_range.end;
      let sel_start_line = document.char_to_line(sel_start);
      let sel_end_line = document.char_to_line(sel_end);

      for doc_line in sel_start_line..=sel_end_line {
        let display_line = self.editor.read(cx).doc_to_display_line(doc_line);
        let Some(display_line) = display_line else {
          continue;
        };
        if !viewport.contains(&display_line) {
          continue;
        }
        let line_range = document.line_range(doc_line).unwrap();
        let shaped_opt = shaped_lines
          .iter()
          .find(|(idx, _)| *idx == display_line)
          .map(|(_, shaped)| shaped);

        if let Some(shaped) = shaped_opt {
          let line_start = line_range.start;
          let line_end = line_range.end;
          let sel_line_start = sel_start.max(line_start) - line_start;
          let sel_line_end = sel_end.min(line_end) - line_start;
          let line_text = line_texts.get(&display_line).cloned().unwrap_or_default();
          let x_start = shaped.x_for_index(char_offset_to_byte_offset(&line_text, sel_line_start));
          let x_end = shaped.x_for_index(char_offset_to_byte_offset(&line_text, sel_line_end));
          let y = line_y(bounds.top(), line_height, display_line, scroll_offset);

          // A selection covering only the newline measures zero wide, so it needs a sliver.
          let is_selecting_newline = sel_line_end > sel_line_start && x_start == x_end;
          let visual_x_end = if is_selecting_newline {
            x_end + px(NEWLINE_SELECTION_WIDTH) // Small width to show newline selection
          } else {
            x_end
          };

          selection_quads.push(fill(
            Bounds::from_corners(
              point(bounds.left() + x_start, y),
              point(bounds.left() + visual_x_end, y + line_height),
            ),
            theme.selection(),
          ));
        }
      }
    }

    let cursor_quad = if is_primary { cursor_quad } else { None };
    let selection_quads = if is_primary {
      selection_quads
    } else {
      Vec::new()
    };

    PrepaintState {
      shaped_lines,
      line_texts,
      indent_guides,
      line_backgrounds,
      gap_separators,
      word_diff_quads,
      conflict_borders,
      group_borders,
      diag_paths,
      cursor_quad,
      selection_quads,
      viewport,
      bounds,
      line_height,
      scroll_offset,
      scroll_hitbox,
      projection,
      block_map,
    }
  }

  fn paint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    prepaint: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
    let is_primary = self.is_primary();
    let (focus_handle, is_focused) = {
      let editor = self.editor.read(cx);
      (
        editor.focus_handle.clone(),
        editor.focus_handle.is_focused(window),
      )
    };

    // Use Rc to avoid cloning PositionMap in closures
    let scroll_offset = self.editor.read(cx).scroll_offset_y;
    let position_map = Rc::new(PositionMap {
      shaped_lines: prepaint.shaped_lines.clone(),
      line_texts: prepaint.line_texts.clone(),
      bounds: prepaint.bounds,
      line_height: prepaint.line_height,
      viewport: prepaint.viewport.clone(),
      scroll_offset,
      projection: prepaint.projection.clone(),
      block_map: prepaint.block_map.clone(),
    });

    window.set_cursor_style(CursorStyle::IBeam, &prepaint.scroll_hitbox);
    let mouse_position = window.mouse_position();
    if prepaint.bounds.contains(&mouse_position)
      && prepaint.scroll_hitbox.should_handle_scroll(window)
      && !position_hits_review_comment_line(&position_map, mouse_position)
      && position_hits_blank_line(&position_map, mouse_position, self.diff_view)
    {
      window.set_window_cursor_style(CursorStyle::Arrow);
    }

    if is_primary {
      window.handle_input(
        &focus_handle,
        ElementInputHandler::new(bounds, self.editor.clone()),
        cx,
      );

      window.on_mouse_event({
        let editor = self.editor.clone();
        let position_map = Rc::clone(&position_map);
        let scroll_hitbox = prepaint.scroll_hitbox.clone();
        let diff_view = self.diff_view;
        move |event: &MouseDownEvent, phase, window, cx| {
          // The laid-out bounds can outgrow the visible column; the hitbox
          // carries the clip, so a press under an overlay never selects.
          if phase == DispatchPhase::Bubble
            && event.button == MouseButton::Left
            && scroll_hitbox.is_hovered(window)
          {
            if position_hits_blank_line(&position_map, event.position, diff_view) {
              cx.stop_propagation();
              return;
            }
            editor.update(cx, |editor, cx| {
              editor.mouse_left_down(event, &position_map, window, cx);
            });
          }
        }
      });

      window.on_mouse_event({
        let editor = self.editor.clone();
        move |event: &MouseUpEvent, phase, window, cx| {
          if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            editor.update(cx, |editor, cx| {
              editor.mouse_left_up(event, window, cx);
            });
          }
        }
      });
    }

    let allow_hover = is_primary || matches!(self.diff_view, DiffElementView::SplitLeft);
    if allow_hover {
      window.on_mouse_event({
        let editor = self.editor.clone();
        let position_map = Rc::clone(&position_map);
        let scroll_hitbox = prepaint.scroll_hitbox.clone();
        let review_comment_side = match self.diff_view {
          DiffElementView::SplitLeft => Some(ReviewCommentSide::Left),
          DiffElementView::SplitRight => Some(ReviewCommentSide::Right),
          DiffElementView::Inline => None,
        };
        move |event: &MouseMoveEvent, phase, window, cx| {
          if phase == DispatchPhase::Bubble {
            let is_selecting = editor.read(cx).is_selecting;
            if is_selecting && is_primary {
              editor.update(cx, |editor, cx| {
                editor.mouse_dragged(event, &position_map, window, cx);
              });
            } else {
              let is_occluded = !scroll_hitbox.is_hovered(window);
              editor.update(cx, |editor, cx| {
                editor.mouse_moved(
                  event,
                  &position_map,
                  is_occluded,
                  is_primary,
                  review_comment_side,
                  cx,
                );
              });
            }
          }
        }
      });
    }

    window.on_mouse_event({
      let editor = self.editor.clone();
      let scroll_hitbox = prepaint.scroll_hitbox.clone();
      let line_height = prepaint.line_height;
      move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && scroll_hitbox.should_handle_scroll(window) {
          editor.update(cx, |editor, cx| {
            let document = editor.document().read(cx);
            let doc_line_count = document.len_lines();
            let total_lines = editor.display_line_count(doc_line_count);
            let now = Instant::now();
            let reset_lock = editor
              .last_scroll_time
              .map(|last| now.duration_since(last) > Duration::from_millis(SCROLL_AXIS_TIMEOUT_MS))
              .unwrap_or(true);
            if reset_lock {
              editor.scroll_axis_lock = None;
              editor.last_scroll_x = editor.scroll_handle.offset().x;
            }
            editor.last_scroll_time = Some(now);

            // Note: Negative delta because scrolling down should increase scroll_offset
            let pixel_delta = event.delta.pixel_delta(line_height);
            let delta_x_px = pixel_delta.x;
            let delta_y_px = -pixel_delta.y;
            let delta_y = match event.delta {
              ScrollDelta::Pixels(point) => -(point.y / px(PIXEL_SCROLL_DIVISOR)),
              ScrollDelta::Lines(point) => -(point.y * LINE_SCROLL_MULTIPLIER),
            };
            let abs_x = delta_x_px.abs();
            let abs_y = delta_y_px.abs();
            let axis = match editor.scroll_axis_lock {
              None => {
                let axis = if abs_x > abs_y * SCROLL_AXIS_RATIO {
                  ScrollAxis::Horizontal
                } else {
                  ScrollAxis::Vertical
                };
                if axis == ScrollAxis::Horizontal {
                  editor.last_scroll_x = editor.scroll_handle.offset().x;
                }
                editor.scroll_axis_lock = Some(axis);
                axis
              }
              Some(axis) => {
                if axis == ScrollAxis::Vertical && abs_x > abs_y * SCROLL_AXIS_SWITCH_RATIO {
                  editor.last_scroll_x = editor.scroll_handle.offset().x;
                  editor.scroll_axis_lock = Some(ScrollAxis::Horizontal);
                  ScrollAxis::Horizontal
                } else if axis == ScrollAxis::Horizontal && abs_y > abs_x * SCROLL_AXIS_SWITCH_RATIO
                {
                  editor.scroll_axis_lock = Some(ScrollAxis::Vertical);
                  ScrollAxis::Vertical
                } else {
                  axis
                }
              }
            };

            if axis == ScrollAxis::Horizontal {
              editor.set_horizontal_scroll_offset(editor.scroll_handle.offset().x + delta_x_px);
              cx.notify();
              return;
            }

            editor.scroll_offset_y = Editor::clamp_vertical_scroll_for_height(
              editor.scroll_offset_y + delta_y,
              bounds.size.height,
              line_height,
              total_lines,
            );
            let clamped_scroll_x = editor.clamp_horizontal_scroll_x(editor.last_scroll_x);
            if editor.scroll_handle.offset().x != clamped_scroll_x {
              editor.set_horizontal_scroll_offset(clamped_scroll_x);
            } else {
              editor.last_scroll_x = clamped_scroll_x;
            }
            let viewport = editor.viewport_range(line_height, total_lines);
            let doc_viewports = editor.doc_ranges_for_display_viewport(viewport.clone());
            editor.document.update(cx, |doc, cx| {
              doc.schedule_viewport_highlights_for_ranges(
                &doc_viewports,
                None,
                crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
                cx,
              );
            });
            cx.notify();
          });
          cx.stop_propagation();
        }
      }
    });

    for quad in &prepaint.line_backgrounds {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.gap_separators {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.word_diff_quads {
      window.paint_quad(quad.clone());
    }

    if !prepaint.diag_paths.is_empty() {
      let stripe_color = cx.theme().muted_foreground.opacity(0.35);
      let mask = ContentMask { bounds };
      window.with_content_mask(Some(mask), |window| {
        for path in &prepaint.diag_paths {
          window.paint_path(path.clone(), stripe_color);
        }
      });
    }

    for quad in &prepaint.conflict_borders {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.group_borders {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.indent_guides {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.selection_quads {
      window.paint_quad(quad.clone());
    }

    for (line_idx, shaped_line) in &prepaint.shaped_lines {
      let y = line_y(
        bounds.top(),
        prepaint.line_height,
        *line_idx,
        prepaint.scroll_offset,
      );
      shaped_line
        .paint(
          point(bounds.left(), y),
          prepaint.line_height,
          TextAlign::Left,
          None,
          window,
          cx,
        )
        .ok();
    }

    let cursor_visible = self.editor.read(cx).cursor_blink.read(cx).visible();
    if is_primary
      && is_focused
      && cursor_visible
      && let Some(cursor_quad) = &prepaint.cursor_quad
    {
      window.paint_quad(cursor_quad.clone());
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::editor::tests::EditorTestContext;
  use crate::projection::{Projection, ReviewCommentSide};
  use gpui::{Context, Modifiers, Render, TestAppContext, Window, point, px, size};
  use std::sync::Arc;
  use syntax::TokenType;

  fn test_bounds(width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(Point::default(), size(px(width), px(height)))
  }

  fn test_editor(cx: &mut TestAppContext) -> Entity<Editor> {
    EditorTestContext::with_text(cx.clone(), "").editor
  }

  fn projection_with_review_comment_line() -> Arc<Projection> {
    let lines = vec![
      DisplayLine::Doc {
        doc_line: 0,
        old_line: Some(0),
        change: None,
        hunk: None,
        group_id: None,
        secondary: false,
      },
      DisplayLine::ReviewComment {
        id: 1,
        side: ReviewCommentSide::Right,
        group_id: None,
        background: None,
        secondary: false,
        text: Arc::from("comment"),
        is_header: true,
      },
      DisplayLine::Doc {
        doc_line: 1,
        old_line: Some(1),
        change: None,
        hunk: None,
        group_id: None,
        secondary: false,
      },
    ];
    Arc::new(Projection::from_lines(2, lines, HashMap::new(), None, None))
  }

  fn test_position_map(projection: Option<Arc<Projection>>) -> PositionMap {
    let block_map = projection
      .as_ref()
      .map(|projection| projection.block_map().clone())
      .unwrap_or_default();
    PositionMap {
      shaped_lines: Vec::new(),
      line_texts: HashMap::new(),
      bounds: test_bounds(200.0, 100.0),
      line_height: px(20.0),
      viewport: 0..3,
      scroll_offset: 0.0,
      projection,
      block_map,
    }
  }

  struct EditorCursorTestView {
    editor: Entity<Editor>,
  }

  impl Render for EditorCursorTestView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
      EditorElement::new(self.editor.clone())
    }
  }

  #[gpui::test]
  fn test_editor_element_tracks_mouse_move_inside_bounds(cx: &mut TestAppContext) {
    let editor = test_editor(cx);
    let editor_for_assert = editor.clone();
    let (_view, cx) = cx.add_window_view(move |_window, _cx| EditorCursorTestView {
      editor: editor.clone(),
    });

    let hover_point = point(px(12.0), px(12.0));
    cx.simulate_mouse_move(hover_point, None, Modifiers::none());
    let last_mouse_position =
      editor_for_assert.read_with(cx, |editor, _| editor.last_mouse_position);

    assert_eq!(last_mouse_position, Some(hover_point));
  }

  #[test]
  fn test_position_hits_review_comment_line_true_for_comment_line() {
    let position_map = test_position_map(Some(projection_with_review_comment_line()));
    let position = point(px(8.0), px(24.0));

    assert!(position_hits_review_comment_line(&position_map, position));
  }

  #[test]
  fn test_position_hits_review_comment_line_false_for_doc_line() {
    let position_map = test_position_map(Some(projection_with_review_comment_line()));
    let position = point(px(8.0), px(4.0));

    assert!(!position_hits_review_comment_line(&position_map, position));
  }

  #[test]
  fn test_position_hits_review_comment_line_false_without_projection() {
    let position_map = test_position_map(None);
    let position = point(px(8.0), px(24.0));

    assert!(!position_hits_review_comment_line(&position_map, position));
  }

  #[gpui::test]
  fn test_line_visibility_uses_block_map_for_review_comments(cx: &mut TestAppContext) {
    let element = EditorElement::split_left(test_editor(cx));
    let projection = projection_with_review_comment_line();
    let block_map = projection.block_map();
    let display_line = &projection.lines[1];

    assert_eq!(
      element.line_visibility(display_line, block_map.block_at_display_line(1)),
      LineVisibility::Blank
    );
    assert_eq!(
      element.line_visibility(display_line, None),
      LineVisibility::Text
    );
  }

  #[test]
  fn mixed_hunk_border_colors_follow_diff_sides() {
    let theme = ui::Theme::dark();
    let kinds = [DiffLineKind::Remove, DiffLineKind::Add];

    assert_eq!(
      hunk_border_colors_for_kinds(&theme, DiffElementView::Inline, kinds),
      Some((theme.diff_gutter_removed(), theme.diff_gutter_added()))
    );
    assert_eq!(
      hunk_border_colors_for_kinds(&theme, DiffElementView::SplitLeft, kinds),
      Some((theme.diff_gutter_removed(), theme.diff_gutter_removed()))
    );
    assert_eq!(
      hunk_border_colors_for_kinds(&theme, DiffElementView::SplitRight, kinds),
      Some((theme.diff_gutter_added(), theme.diff_gutter_added()))
    );
  }

  #[test]
  fn display_line_group_id_includes_blank_split_rows_and_blocks() {
    let group_id: Arc<str> = Arc::from("hunk-blank-row");
    let projection = Arc::new(Projection::from_lines(
      2,
      vec![
        DisplayLine::Doc {
          doc_line: 0,
          old_line: Some(0),
          change: Some(ChangeKind::Added),
          hunk: Some(HunkState::Unstaged),
          group_id: Some(group_id.clone()),
          secondary: false,
        },
        DisplayLine::ReviewComment {
          id: 1,
          side: ReviewCommentSide::Right,
          group_id: Some(group_id.clone()),
          background: None,
          secondary: false,
          text: Arc::from("comment"),
          is_header: true,
        },
        DisplayLine::Doc {
          doc_line: 1,
          old_line: Some(1),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        },
      ],
      HashMap::new(),
      None,
      None,
    ));
    let block_map = projection.block_map();

    assert_eq!(
      group_id_for_display_line(0, Some(&projection), block_map).map(|id| id.as_ref()),
      Some("hunk-blank-row")
    );
    assert_eq!(
      group_id_for_display_line(1, Some(&projection), block_map).map(|id| id.as_ref()),
      Some("hunk-blank-row")
    );
  }

  #[gpui::test]
  fn test_calculate_viewport_simple(cx: &mut TestAppContext) {
    let editor = test_editor(cx);
    let element = EditorElement::new(editor);

    // 400px height, 20px line height = 20 visible lines
    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 0..20);
  }

  #[gpui::test]
  fn test_calculate_viewport_with_scroll(cx: &mut TestAppContext) {
    let editor = test_editor(cx);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 10.0; // Scrolled down 10 lines
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 10..30);
  }

  #[gpui::test]
  fn test_calculate_viewport_at_end(cx: &mut TestAppContext) {
    let editor = test_editor(cx);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 90.0; // Near end
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Clamp to the maximum reachable scroll position for the viewport height.
    assert_eq!(viewport, 83..100);
  }

  #[gpui::test]
  fn test_calculate_viewport_short_document(cx: &mut TestAppContext) {
    let editor = test_editor(cx);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 5; // Document shorter than viewport

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 0..5);
  }

  #[gpui::test]
  fn test_calculate_viewport_fractional_scroll(cx: &mut TestAppContext) {
    let editor = test_editor(cx);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 5.5; // Fractional scroll
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Include one extra line when scroll is fractional to avoid gaps.
    assert_eq!(viewport, 5..26);
  }

  #[gpui::test]
  fn test_calculate_viewport_scroll_past_end(cx: &mut TestAppContext) {
    let editor = test_editor(cx);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 150.0; // Way past end
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Clamp to the maximum reachable scroll position for the viewport height.
    assert_eq!(viewport, 83..100);
  }

  #[gpui::test]
  fn test_calculate_viewport_minimum_one_line(cx: &mut TestAppContext) {
    let editor = test_editor(cx);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 10.0); // Very small height
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Should show at least 1 line even if height is too small
    assert_eq!(viewport, 0..1);
  }

  #[gpui::test]
  fn test_calculate_viewport_large_line_height(cx: &mut TestAppContext) {
    let editor = test_editor(cx);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(40.0); // Large line height
    let scroll_offset = 0.0;
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // 400 / 40 = 10 visible lines
    assert_eq!(viewport, 0..10);
  }

  #[gpui::test]
  fn test_calculate_viewport_single_line_document(cx: &mut TestAppContext) {
    let editor = test_editor(cx);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 1;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 0..1);
  }

  #[gpui::test]
  fn test_calculate_viewport_empty_document(cx: &mut TestAppContext) {
    let editor = test_editor(cx);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 0;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Empty document edge case - start_line gets clamped to 0
    assert!(viewport.is_empty());
  }

  #[test]
  fn test_char_byte_offset_helpers_handle_emoji() {
    let text = "🤓 Branches principales";
    assert_eq!(char_offset_to_byte_offset(text, 0), 0);
    assert_eq!(char_offset_to_byte_offset(text, 1), 4);
    assert_eq!(byte_offset_to_char_offset(text, 1), 0);
    assert_eq!(byte_offset_to_char_offset(text, 4), 1);
  }

  #[test]
  fn test_indent_guide_byte_ranges_with_spaces() {
    assert_eq!(
      indent_guide_byte_ranges("        let x = 1;", 4),
      vec![0..4, 4..8]
    );
  }

  #[test]
  fn test_indent_guide_byte_ranges_with_two_space_blocks() {
    assert_eq!(
      indent_guide_byte_ranges("      let x = 1;", 2),
      vec![0..2, 2..4, 4..6]
    );
  }

  #[test]
  fn test_indent_guide_byte_ranges_with_tabs_and_spaces() {
    assert_eq!(indent_guide_byte_ranges("  \t\tvalue", 4), vec![0..3, 3..4]);
  }

  #[test]
  fn test_indent_guide_byte_ranges_with_single_tab() {
    assert_eq!(indent_guide_byte_ranges("\tvalue", 4), vec![0..1]);
  }

  #[test]
  fn test_indent_guide_byte_ranges_ignores_partial_indent_level() {
    assert_eq!(
      indent_guide_byte_ranges("  value", 4),
      Vec::<Range<usize>>::new()
    );
  }

  #[test]
  fn test_indent_guide_border_color_is_subtle_but_visible() {
    let fill_color = gpui::Hsla {
      h: 212.0 / 360.0,
      s: 0.64,
      l: 0.46,
      a: 0.09,
    };
    let border = indent_guide_border_color(fill_color);
    assert_eq!(border.h, fill_color.h);
    assert_eq!(border.s, fill_color.s);
    assert_eq!(border.l, fill_color.l);
    assert!(border.a > fill_color.a);
    assert!(border.a <= 0.28);
  }

  #[test]
  fn test_conflict_border_edges_split_current_and_incoming_blocks() {
    assert_eq!(
      conflict_border_edges(
        None,
        ConflictLineKind::CurrentMarker,
        Some(ConflictLineKind::Current),
      ),
      Some((true, false))
    );
    assert_eq!(
      conflict_border_edges(
        Some(ConflictLineKind::CurrentMarker),
        ConflictLineKind::Current,
        Some(ConflictLineKind::Divider),
      ),
      Some((false, true))
    );
    assert_eq!(
      conflict_border_edges(
        Some(ConflictLineKind::Divider),
        ConflictLineKind::Incoming,
        Some(ConflictLineKind::IncomingMarker),
      ),
      Some((true, false))
    );
    assert_eq!(
      conflict_border_edges(
        Some(ConflictLineKind::Incoming),
        ConflictLineKind::IncomingMarker,
        None,
      ),
      Some((false, true))
    );
    assert_eq!(
      conflict_border_edges(
        Some(ConflictLineKind::Current),
        ConflictLineKind::Divider,
        Some(ConflictLineKind::Incoming),
      ),
      None
    );
  }

  #[test]
  fn test_word_diff_ranges_highlight_added_camel_case_segment() {
    let old_text = "const getLastNotification = () => \"You have a new message!\";";
    let new_text = "const getLastDataNotification = () => \"You have a new message!\";";

    let (removed, added) = word_diff_ranges(old_text, new_text);

    assert!(removed.is_empty());
    assert_eq!(added.len(), 1);
    assert_eq!(&new_text[added[0].clone()], "Data");
  }

  #[test]
  fn test_benchmark_word_diff_ranges_matches_internal_logic() {
    let old_text = "const getLastNotification = () => \"You have a new message!\";";
    let new_text = "const getLastDataNotification = () => \"You have a new message!\";";

    assert_eq!(
      benchmark_word_diff_ranges(old_text, new_text),
      word_diff_ranges(old_text, new_text)
    );
  }

  #[test]
  fn test_word_diff_ranges_skip_very_long_lines() {
    let old_text = "const value = \"stable\";".repeat(80);
    let new_text = "const value = \"changed\";".repeat(80);

    let (removed, added) = word_diff_ranges(&old_text, &new_text);

    assert!(removed.is_empty());
    assert!(added.is_empty());
  }

  #[gpui::test]
  fn test_display_line_text_for_view_uses_old_text_on_split_left(cx: &mut TestAppContext) {
    let document = cx.new(|cx| Document::new("  new_text", None, cx));
    let display_line = DisplayLine::Modified {
      doc_line: 0,
      old_line: 0,
      old_text: "        old_text".into(),
      hunk: HunkState::Unstaged,
      group_id: None,
      secondary: false,
    };

    let (split_left, split_right, inline) = document.read_with(cx, |document, _| {
      (
        display_line_text_for_view(&display_line, DiffElementView::SplitLeft, document),
        display_line_text_for_view(&display_line, DiffElementView::SplitRight, document),
        display_line_text_for_view(&display_line, DiffElementView::Inline, document),
      )
    });

    assert_eq!(split_left, "        old_text");
    assert_eq!(split_right, "  new_text");
    assert_eq!(inline, "  new_text");
  }

  #[gpui::test]
  fn test_collect_word_diffs_for_viewport_keeps_modified_lines_available(cx: &mut TestAppContext) {
    let document = cx.new(|cx| Document::new("const getLastDataNotification = value;", None, cx));
    let viewport_lines = vec![(
      0,
      DisplayLine::Modified {
        doc_line: 0,
        old_line: 0,
        old_text: "const getLastNotification = value;".into(),
        hunk: HunkState::Unstaged,
        group_id: None,
        secondary: false,
      },
    )];

    let styles = document.read_with(cx, |document, _| {
      let projection = Projection {
        block_map: ProjectionBlockMap::default(),
        lines: viewport_lines
          .iter()
          .map(|(_, line)| line.clone())
          .collect(),
        display_to_doc: vec![Some(0)],
        doc_to_display: vec![Some(0)],
        visible_doc_lines: vec![0],
        groups: HashMap::new(),
      };
      let cache = build_word_diff_cache(&projection, document);
      collect_word_diffs_for_viewport(
        &viewport_lines,
        DiffElementView::SplitRight,
        &cache,
        &Theme::dark(),
      )
    });

    assert!(styles.contains_key(&0));
    assert_eq!(styles[&0].ranges.len(), 1);
  }

  #[test]
  fn test_highlights_to_text_runs_clamps_non_char_boundary_spans() {
    let line = "✅ **Branches parallèles** (API + notifications fusionnées dans develop)";
    let highlights = vec![
      HighlightSpan {
        byte_range: 1..4,
        token_type: TokenType::Keyword,
      },
      HighlightSpan {
        byte_range: 7..20,
        token_type: TokenType::String,
      },
    ];

    let base_style = TextStyle::default();
    let theme = Theme::dark();
    let runs = highlights_to_text_runs(&highlights, line, &theme, &base_style);

    let mut offset = 0usize;
    for run in runs {
      offset += run.len;
      assert!(line.is_char_boundary(offset));
    }
    assert_eq!(offset, line.len());
  }
}
