use buffer::TextBuffer;
use gpui::{Context, Task};
use parking_lot::{Mutex, RwLock};
use ropey::Rope;
use std::{
  borrow::Cow,
  cell::RefCell,
  collections::{HashMap, VecDeque, hash_map::DefaultHasher},
  hash::{Hash, Hasher},
  ops::Range,
  sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
  },
  time::{Duration, Instant},
};
use syntax::languages;
use syntax::{HighlightSpan, SyntaxHighlighter};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum HighlightQuality {
  Viewport = 1,
  Full = 2,
}

struct LineHighlight {
  spans: Arc<[HighlightSpan]>,
  quality: HighlightQuality,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
  Unchanged,
  Added,
  Deleted,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DiffGutterKind {
  None,
  Added,
  Modified,
}

#[derive(Copy, Clone, Debug)]
pub struct DiffLineInfo {
  pub kind: DiffLineKind,
  pub gutter: DiffGutterKind,
  pub base_line: Option<usize>,
  pub current_line: Option<usize>,
}

#[derive(Clone, Debug)]
struct DiffLine {
  kind: DiffLineKind,
  gutter: DiffGutterKind,
  base_line: Option<usize>,
  current_line: Option<usize>,
  deleted_text: Option<Arc<str>>,
  word_diff_ranges: Option<Arc<[Range<usize>]>>,
  len_chars: usize,
}

#[derive(Clone, Debug)]
struct DiffState {
  lines: Vec<DiffLine>,
  line_starts: Vec<usize>,
  len_chars: usize,
  current_to_view: Vec<Option<usize>>,
  base_to_view: Vec<Option<usize>>,
}

pub struct Document {
  pub buffer: TextBuffer,

  // Syntax highlighting support
  highlighter: Option<SyntaxHighlighter>,
  // Line-local highlight cache (None = not computed yet)
  highlights: Arc<RwLock<Vec<Option<LineHighlight>>>>,
  // Base-file highlights for diff deleted lines
  base_highlights: Arc<RwLock<Vec<Option<LineHighlight>>>>,
  pending_highlight_task: Option<Task<()>>,
  pending_viewport_highlight_task: Option<Task<()>>,
  pending_viewport_diff_task: Option<Task<()>>,

  // Flag to track when highlights have been updated (for cache invalidation)
  pub highlights_version: Arc<RwLock<usize>>,
  // Track highlight generations to invalidate stale caches on re-run
  pub highlights_epoch: Arc<RwLock<usize>>,
  dirty_highlight_lines: Arc<RwLock<Vec<usize>>>,
  highlight_generation: Arc<AtomicUsize>,
  viewport_highlight_generation: Arc<AtomicUsize>,

  // Diff view support
  diff_base_lines: Option<Arc<Vec<Arc<str>>>>,
  diff_base_hashes: Option<Arc<Vec<u64>>>,
  diff_state: Arc<RwLock<Option<DiffState>>>,
  pending_diff_task: Option<Task<()>>,
  pub diff_version: Arc<RwLock<usize>>,
  pub diff_epoch: Arc<RwLock<usize>>,
  dirty_diff_lines: Arc<RwLock<Vec<usize>>>,
  diff_generation: Arc<AtomicUsize>,
  viewport_diff_generation: Arc<AtomicUsize>,
}

impl Document {
  /// Create document with optional diff base and language detection for syntax highlighting
  pub fn new(
    text: &str,
    base_text: Option<&str>,
    file_ext: Option<&str>,
    cx: &mut Context<Self>,
  ) -> Self {
    let buffer = TextBuffer::from_text(text);

    let highlighter = file_ext
      .and_then(languages::detect_language_config)
      .map(SyntaxHighlighter::new);

    let diff_base_lines = base_text.map(|text| Arc::new(split_lines_to_arcs(text)));
    let diff_base_hashes = diff_base_lines
      .as_ref()
      .map(|lines| Arc::new(hash_lines(lines)));

    let base_highlights = diff_base_lines
      .as_ref()
      .map(|lines| {
        let mut entries = Vec::new();
        entries.resize_with(lines.len(), || None);
        entries
      })
      .unwrap_or_default();

    let mut doc = Self {
      buffer,
      highlighter,
      highlights: Arc::new(RwLock::new(Vec::new())),
      base_highlights: Arc::new(RwLock::new(base_highlights)),
      pending_highlight_task: None,
      pending_viewport_highlight_task: None,
      pending_viewport_diff_task: None,
      highlights_version: Arc::new(RwLock::new(0)),
      highlights_epoch: Arc::new(RwLock::new(0)),
      dirty_highlight_lines: Arc::new(RwLock::new(Vec::new())),
      highlight_generation: Arc::new(AtomicUsize::new(0)),
      viewport_highlight_generation: Arc::new(AtomicUsize::new(0)),
      diff_base_lines,
      diff_base_hashes,
      diff_state: Arc::new(RwLock::new(None)),
      pending_diff_task: None,
      diff_version: Arc::new(RwLock::new(0)),
      diff_epoch: Arc::new(RwLock::new(0)),
      dirty_diff_lines: Arc::new(RwLock::new(Vec::new())),
      diff_generation: Arc::new(AtomicUsize::new(0)),
      viewport_diff_generation: Arc::new(AtomicUsize::new(0)),
    };

    // Schedule initial highlighting
    if doc.highlighter.is_some() {
      doc.schedule_recompute_highlights(cx);
    }
    if doc.diff_base_lines.is_some() {
      doc.schedule_recompute_diff(cx);
    }

    doc
  }

  pub fn chars(&self) -> Box<dyn Iterator<Item = char> + '_> {
    if self.diff_base_lines.is_none() {
      Box::new(self.buffer.chars())
    } else {
      Box::new(ViewChars::new(self))
    }
  }

  pub fn len(&self) -> usize {
    let diff_state = self.diff_state.read();
    if let Some(state) = diff_state.as_ref() {
      state.len_chars
    } else {
      self.buffer.len()
    }
  }

  pub fn len_lines(&self) -> usize {
    let diff_state = self.diff_state.read();
    if let Some(state) = diff_state.as_ref() {
      state.lines.len()
    } else {
      self.buffer.len_lines()
    }
  }

  pub fn is_empty(&self) -> bool {
    self.len() == 0
  }

  pub fn line_content(&self, line_idx: usize) -> Option<Cow<'_, str>> {
    let diff_state = self.diff_state.read();
    if let Some(state) = diff_state.as_ref() {
      let line = state.lines.get(line_idx)?;
      return match line.kind {
        DiffLineKind::Deleted => line
          .deleted_text
          .as_ref()
          .map(|text| Cow::Owned(text.to_string())),
        _ => line
          .current_line
          .and_then(|current_line| self.buffer.line_content(current_line)),
      };
    }
    self.buffer.line_content(line_idx)
  }

  pub fn line_range(&self, line_idx: usize) -> Option<Range<usize>> {
    let diff_state = self.diff_state.read();
    if let Some(state) = diff_state.as_ref() {
      if line_idx >= state.lines.len() {
        return None;
      }
      let start = state.line_starts[line_idx];
      let end = if line_idx + 1 < state.lines.len() {
        state.line_starts[line_idx + 1]
      } else {
        state.len_chars
      };
      return Some(start..end);
    }
    self.buffer.line_range(line_idx)
  }

  pub fn slice_to_string(&self, range: Range<usize>) -> String {
    let diff_state = self.diff_state.read();
    if diff_state.is_none() {
      return self.buffer.slice_to_string(range);
    }

    if range.start >= range.end {
      return String::new();
    }

    let total_lines = self.len_lines();
    if total_lines == 0 {
      return String::new();
    }

    let start_line = self.char_to_line(range.start);
    let end_line = self.char_to_line(range.end.saturating_sub(1));
    let mut result = String::new();

    for line_idx in start_line..=end_line {
      let line_start = self.line_to_char(line_idx);
      let mut line_text = self.line_content(line_idx).unwrap_or_default().into_owned();
      if line_idx + 1 < total_lines {
        line_text.push('\n');
      }
      let line_len = line_text.chars().count();
      let slice_start = range.start.saturating_sub(line_start).min(line_len);
      let slice_end = range.end.saturating_sub(line_start).min(line_len);
      if slice_start < slice_end {
        result.push_str(&slice_chars(&line_text, slice_start, slice_end));
      }
    }

    result
  }

  pub fn char_to_line(&self, char_idx: usize) -> usize {
    let diff_state = self.diff_state.read();
    if let Some(state) = diff_state.as_ref() {
      if state.line_starts.is_empty() {
        return 0;
      }
      match state.line_starts.binary_search(&char_idx) {
        Ok(idx) => idx,
        Err(idx) => idx
          .saturating_sub(1)
          .min(state.line_starts.len().saturating_sub(1)),
      }
    } else {
      self.buffer.char_to_line(char_idx)
    }
  }

  pub fn line_to_char(&self, line_idx: usize) -> usize {
    let diff_state = self.diff_state.read();
    if let Some(state) = diff_state.as_ref() {
      state
        .line_starts
        .get(line_idx)
        .copied()
        .unwrap_or(state.len_chars)
    } else {
      self.buffer.line_to_char(line_idx)
    }
  }

  #[cfg(test)]
  pub fn insert_char(&mut self, offset: usize, ch: char, cx: &mut Context<Self>) {
    self.buffer.transaction(Instant::now(), |buffer, tx| {
      buffer.insert(tx, offset, &ch.to_string());
    });
    cx.notify();
  }

  pub fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
    let Some(buffer_range) = self.map_view_range_to_buffer(&range) else {
      cx.notify();
      return;
    };
    self.buffer.transaction(Instant::now(), |buffer, tx| {
      buffer.replace(tx, buffer_range, text);
    });
    cx.notify();
  }

  pub fn undo(&mut self, cx: &mut Context<Self>) -> Option<buffer::TransactionId> {
    let result = self.buffer.undo();
    if result.is_some() {
      if self.diff_base_lines.is_some() {
        self.schedule_recompute_diff(cx);
      }
      cx.notify();
    }
    result
  }

  pub fn redo(&mut self, cx: &mut Context<Self>) -> Option<buffer::TransactionId> {
    let result = self.buffer.redo();
    if result.is_some() {
      if self.diff_base_lines.is_some() {
        self.schedule_recompute_diff(cx);
      }
      cx.notify();
    }
    result
  }

  pub fn can_undo(&self) -> bool {
    self.buffer.can_undo()
  }

  pub fn can_redo(&self) -> bool {
    self.buffer.can_redo()
  }

  pub fn set_group_interval(&mut self, interval: Duration) {
    self.buffer.set_group_interval(interval);
  }

  /// Get syntax highlights for a specific line
  pub fn get_highlights_for_line(&self, line_idx: usize) -> Option<Arc<[HighlightSpan]>> {
    let highlights = self.highlights.read();
    let diff_state = self.diff_state.read();
    if let Some(state) = diff_state.as_ref() {
      let line = state.lines.get(line_idx)?;
      if line.kind == DiffLineKind::Deleted {
        let base_line = line.base_line?;
        let base_highlights = self.base_highlights.read();
        if base_line >= base_highlights.len() {
          return None;
        }
        return base_highlights
          .get(base_line)?
          .as_ref()
          .map(|entry| Arc::clone(&entry.spans));
      }

      let current_line = line.current_line?;
      if current_line >= highlights.len() {
        return None;
      }
      return highlights
        .get(current_line)?
        .as_ref()
        .map(|entry| Arc::clone(&entry.spans));
    }
    if line_idx >= highlights.len() {
      return None;
    }
    highlights
      .get(line_idx)?
      .as_ref()
      .map(|entry| Arc::clone(&entry.spans))
  }

  pub fn drain_dirty_highlight_lines(&self) -> Vec<usize> {
    let mut dirty = self.dirty_highlight_lines.write();
    if dirty.is_empty() {
      Vec::new()
    } else {
      dirty.drain(..).collect()
    }
  }

  pub fn drain_dirty_diff_lines(&self) -> Vec<usize> {
    let mut dirty = self.dirty_diff_lines.write();
    if dirty.is_empty() {
      Vec::new()
    } else {
      dirty.drain(..).collect()
    }
  }

  pub fn diff_line_info(&self, line_idx: usize) -> Option<DiffLineInfo> {
    let diff_state = self.diff_state.read();
    let state = diff_state.as_ref()?;
    let line = state.lines.get(line_idx)?;
    Some(DiffLineInfo {
      kind: line.kind,
      gutter: line.gutter,
      base_line: line.base_line,
      current_line: line.current_line,
    })
  }

  pub fn diff_word_ranges(&self, line_idx: usize) -> Option<Arc<[Range<usize>]>> {
    let diff_state = self.diff_state.read();
    let state = diff_state.as_ref()?;
    let line = state.lines.get(line_idx)?;
    line.word_diff_ranges.as_ref().map(Arc::clone)
  }

  pub fn diff_enabled(&self) -> bool {
    self.diff_base_lines.is_some()
  }

  // Keep diff line starts in sync for single-line edits between diff recomputes.
  pub(crate) fn apply_single_line_edit_delta(&mut self, current_line: usize, delta: isize) {
    if delta == 0 || !self.diff_enabled() {
      return;
    }
    let mut diff_state = self.diff_state.write();
    let Some(state) = diff_state.as_mut() else {
      return;
    };
    if state.current_to_view.len() != self.buffer.len_lines() {
      return;
    }
    let Some(view_line) = state.current_to_view.get(current_line).copied().flatten() else {
      return;
    };
    let Some(line) = state.lines.get_mut(view_line) else {
      return;
    };
    let old_len = line.len_chars as isize;
    let new_len = (old_len + delta).max(0);
    let applied_delta = new_len - old_len;
    if applied_delta == 0 {
      return;
    }
    line.len_chars = new_len as usize;
    for start in state.line_starts.iter_mut().skip(view_line + 1) {
      *start = (*start as isize + applied_delta).max(0) as usize;
    }
    state.len_chars = (state.len_chars as isize + applied_delta).max(0) as usize;
  }

  pub fn map_view_line_range_to_current(&self, range: &Range<usize>) -> Option<Range<usize>> {
    let diff_state = self.diff_state.read();
    let state = match diff_state.as_ref() {
      Some(state) => state,
      None => return Some(range.clone()),
    };

    if range.start >= range.end || state.lines.is_empty() {
      return None;
    }

    let mut min_current: Option<usize> = None;
    let mut max_current: Option<usize> = None;
    for line_idx in range.start..range.end.min(state.lines.len()) {
      if let Some(current_line) = state.lines[line_idx].current_line {
        min_current = Some(match min_current {
          Some(min) => min.min(current_line),
          None => current_line,
        });
        max_current = Some(match max_current {
          Some(max) => max.max(current_line),
          None => current_line,
        });
      }
    }

    let min_current = min_current?;
    let max_current = max_current?;
    Some(min_current..(max_current + 1))
  }

  pub fn is_line_editable(&self, line_idx: usize) -> bool {
    let diff_state = self.diff_state.read();
    if let Some(state) = diff_state.as_ref() {
      return matches!(
        state.lines.get(line_idx).map(|line| line.kind),
        Some(DiffLineKind::Added | DiffLineKind::Unchanged)
      );
    }
    true
  }

  pub fn is_range_editable(&self, range: &Range<usize>) -> bool {
    let diff_state = self.diff_state.read();
    let state = match diff_state.as_ref() {
      Some(state) => state,
      None => return true,
    };

    if state.lines.is_empty() {
      return true;
    }

    let start_line = char_to_line_with_starts(&state.line_starts, range.start);
    let end_line = if range.end > range.start {
      char_to_line_with_starts(&state.line_starts, range.end.saturating_sub(1))
    } else {
      start_line
    };

    for line_idx in start_line..=end_line.min(state.lines.len().saturating_sub(1)) {
      if matches!(state.lines[line_idx].kind, DiffLineKind::Deleted) {
        return false;
      }
    }

    true
  }

  pub fn map_view_range_to_buffer(&self, range: &Range<usize>) -> Option<Range<usize>> {
    if !self.is_range_editable(range) {
      return None;
    }

    let diff_state = self.diff_state.read();
    let state = match diff_state.as_ref() {
      Some(state) => state,
      None => return Some(range.clone()),
    };

    if state.lines.is_empty() {
      return Some(range.clone());
    }

    let start = self.map_view_offset_to_buffer(range.start)?;
    let end = self.map_view_offset_to_buffer(range.end)?;
    Some(start..end)
  }

  pub fn map_buffer_range_to_view(&self, range: &Range<usize>) -> Option<Range<usize>> {
    let diff_state = self.diff_state.read();
    let state = match diff_state.as_ref() {
      Some(state) => state,
      None => return Some(range.clone()),
    };

    if state.lines.is_empty() {
      return Some(range.clone());
    }

    let start = self.map_buffer_offset_to_view_with_state(range.start, state)?;
    let end = self.map_buffer_offset_to_view_with_state(range.end, state)?;
    Some(start..end)
  }

  fn map_view_offset_to_buffer(&self, offset: usize) -> Option<usize> {
    let diff_state = self.diff_state.read();
    let state = diff_state.as_ref()?;
    let line_idx = char_to_line_with_starts(&state.line_starts, offset);
    let line = state.lines.get(line_idx)?;
    let current_line = line.current_line?;
    let view_line_start = state.line_starts.get(line_idx).copied()?;
    let column = offset.saturating_sub(view_line_start);
    let buffer_line_range = self.buffer.line_range(current_line)?;
    let buffer_line_start = buffer_line_range.start;
    let buffer_line_len = buffer_line_range.len();
    Some(buffer_line_start + column.min(buffer_line_len))
  }

  fn map_buffer_offset_to_view_with_state(
    &self,
    offset: usize,
    state: &DiffState,
  ) -> Option<usize> {
    let offset = offset.min(self.buffer.len());
    let current_line = self.buffer.char_to_line(offset);
    let view_line = state.current_to_view.get(current_line)?.to_owned()?;
    let view_line_start = state.line_starts.get(view_line).copied()?;
    let current_line_start = self.buffer.line_to_char(current_line);
    let column = offset.saturating_sub(current_line_start);
    let line_len = state
      .lines
      .get(view_line)
      .map(|line| line.len_chars)
      .unwrap_or(0);
    Some(view_line_start + column.min(line_len))
  }

  fn viewport_to_current_range(
    &self,
    viewport: Range<usize>,
    margin_lines: usize,
  ) -> Option<Range<usize>> {
    let diff_state = self.diff_state.read();
    let state = match diff_state.as_ref() {
      Some(state) => state,
      None => {
        let line_count = self.buffer.len_lines();
        let start_line = viewport.start.saturating_sub(margin_lines);
        let end_line = (viewport.end + margin_lines).min(line_count);
        return Some(start_line..end_line);
      }
    };

    let view_start = viewport.start.saturating_sub(margin_lines);
    let view_end = (viewport.end + margin_lines).min(state.lines.len());
    if view_start >= view_end {
      return None;
    }

    let mut min_current: Option<usize> = None;
    let mut max_current: Option<usize> = None;

    for line_idx in view_start..view_end {
      if let Some(current_line) = state.lines[line_idx].current_line {
        min_current = Some(match min_current {
          Some(min) => min.min(current_line),
          None => current_line,
        });
        max_current = Some(match max_current {
          Some(max) => max.max(current_line),
          None => current_line,
        });
      }
    }

    let min_current = min_current?;
    let max_current = max_current?;
    let line_count = self.buffer.len_lines();
    Some(min_current..(max_current + 1).min(line_count))
  }

  /// Schedule viewport-only highlight for responsive feedback.
  pub fn schedule_viewport_highlights(
    &mut self,
    viewport: Range<usize>,
    force_range: Option<Range<usize>>,
    margin_lines: usize,
    cx: &mut Context<Self>,
  ) {
    self.pending_viewport_highlight_task = None;

    let (base_lines, base_range, view_window) = {
      let diff_state = self.diff_state.read();
      let state = diff_state.as_ref();
      let base_lines = self.diff_base_lines.clone();
      if let (Some(state), Some(base_lines)) = (state, base_lines.as_ref()) {
        let view_start = viewport
          .start
          .saturating_sub(margin_lines)
          .min(state.lines.len());
        let view_end = (viewport.end + margin_lines).min(state.lines.len());
        let mut min_base: Option<usize> = None;
        let mut max_base: Option<usize> = None;
        for line in &state.lines[view_start..view_end] {
          if line.kind == DiffLineKind::Deleted
            && let Some(base_line) = line.base_line
          {
            min_base = Some(match min_base {
              Some(min) => min.min(base_line),
              None => base_line,
            });
            max_base = Some(match max_base {
              Some(max) => max.max(base_line),
              None => base_line,
            });
          }
        }
        let base_range = min_base.map(|min| min..(max_base.unwrap_or(min) + 1));
        (
          Some(Arc::clone(base_lines)),
          base_range,
          Some(view_start..view_end),
        )
      } else {
        (None, None, None)
      }
    };
    let base_line_count = base_lines.as_ref().map(|lines| lines.len()).unwrap_or(0);

    let line_count = self.buffer.len_lines();
    let current_range = self.viewport_to_current_range(viewport, margin_lines);
    if line_count == 0 && base_range.is_none() {
      return;
    }
    if current_range.is_none() && base_range.is_none() {
      return;
    }

    let Some(ref mut highlighter) = self.highlighter else {
      return;
    };
    let (start_line, text) = match current_range.as_ref() {
      Some(range) if range.start < range.end => (
        range.start,
        Some(build_viewport_text(&self.buffer, range.start, range.end)),
      ),
      _ => (0, None),
    };
    let config = highlighter.config;
    let highlights_cache = self.highlights.clone();
    let base_highlights_cache = self.base_highlights.clone();
    let diff_state = self.diff_state.clone();
    let dirty_highlight_lines = self.dirty_highlight_lines.clone();
    let highlights_version = self.highlights_version.clone();
    let viewport_highlight_generation = self.viewport_highlight_generation.clone();
    let my_generation = viewport_highlight_generation.fetch_add(1, Ordering::Relaxed) + 1;

    let task = cx.spawn(async move |this, cx| {
      let result = if let Some(text) = text {
        Some(
          cx.background_executor()
            .spawn(async move { highlight_text_to_line_spans(&text, config) })
            .await,
        )
      } else {
        None
      };

      let base_result = if let (Some(base_lines), Some(base_range)) = (base_lines, base_range) {
        let base_text =
          build_viewport_text_from_lines(&base_lines, base_range.start, base_range.end);
        Some(
          cx.background_executor()
            .spawn(async move { highlight_text_to_line_spans(&base_text, config) })
            .await
            .map(|spans| (base_range, spans)),
        )
      } else {
        None
      };

      if viewport_highlight_generation.load(Ordering::Relaxed) != my_generation {
        return;
      }

      let _ = this.update(cx, |_doc, cx| {
        let mut updated = false;
        let mut dirty_lines = dirty_highlight_lines.write();

        if let Some(result) = result {
          match result {
            Ok(line_spans) => {
              let mut highlights = highlights_cache.write();
              if highlights.len() < line_count {
                highlights.resize_with(line_count, || None);
              }

              for (offset, spans) in line_spans.into_iter().enumerate() {
                let line_idx = start_line + offset;
                if line_idx >= highlights.len() {
                  break;
                }

                let replace = match highlights[line_idx].as_ref() {
                  None => true,
                  Some(existing) => {
                    if let Some(force_range) = force_range.as_ref() {
                      force_range.contains(&line_idx) || existing.quality != HighlightQuality::Full
                    } else {
                      existing.quality != HighlightQuality::Full
                    }
                  }
                };

                if replace {
                  highlights[line_idx] = Some(LineHighlight {
                    spans,
                    quality: HighlightQuality::Viewport,
                  });
                  dirty_lines.push(line_idx);
                  updated = true;
                }
              }
            }
            Err(err) => {
              eprintln!("Viewport highlighting failed: {}", err);
            }
          }
        }

        if let Some(base_result) = base_result {
          match base_result {
            Ok((base_range, base_spans)) => {
              let mut base_updated = false;
              let mut base_highlights = base_highlights_cache.write();
              if base_highlights.len() < base_line_count {
                base_highlights.resize_with(base_line_count, || None);
              }

              for (offset, spans) in base_spans.into_iter().enumerate() {
                let base_line = base_range.start + offset;
                if base_line >= base_highlights.len() {
                  break;
                }

                let replace = match base_highlights[base_line].as_ref() {
                  None => true,
                  Some(existing) => existing.quality != HighlightQuality::Full,
                };

                if replace {
                  base_highlights[base_line] = Some(LineHighlight {
                    spans,
                    quality: HighlightQuality::Viewport,
                  });
                  base_updated = true;
                }
              }

              if base_updated {
                if let (Some(view_window), Some(state)) =
                  (view_window.as_ref(), diff_state.read().as_ref())
                {
                  let view_end = view_window.end.min(state.lines.len());
                  for line_idx in view_window.start..view_end {
                    let line = &state.lines[line_idx];
                    if line.kind == DiffLineKind::Deleted
                      && let Some(base_line) = line.base_line
                      && base_range.contains(&base_line)
                    {
                      dirty_lines.push(line_idx);
                    }
                  }
                }
                updated = true;
              }
            }
            Err(err) => {
              eprintln!("Viewport base highlighting failed: {}", err);
            }
          }
        }

        if updated {
          *highlights_version.write() += 1;
          cx.notify();
        }
      });
    });

    self.pending_viewport_highlight_task = Some(task);
  }

  /// Schedule async re-highlighting with debouncing
  pub fn schedule_recompute_highlights(&mut self, cx: &mut Context<Self>) {
    // Cancel previous task
    self.pending_highlight_task = None;
    self.pending_viewport_highlight_task = None;
    self
      .viewport_highlight_generation
      .fetch_add(1, Ordering::Relaxed);

    let Some(ref mut highlighter) = self.highlighter else {
      return;
    };

    let text = self.buffer.slice_to_string(0..self.buffer.len());
    let line_count = self.buffer.len_lines();
    let highlights_cache = self.highlights.clone();
    let highlights_version = self.highlights_version.clone();
    let dirty_highlight_lines = self.dirty_highlight_lines.clone();
    let highlight_generation = self.highlight_generation.clone();

    // Clone highlighter config for background work
    let config = highlighter.config;
    let my_generation = highlight_generation.fetch_add(1, Ordering::Relaxed) + 1;

    {
      let mut highlights = highlights_cache.write();
      if highlights.len() < line_count {
        highlights.resize_with(line_count, || None);
      } else if highlights.len() > line_count {
        highlights.truncate(line_count);
      }
    }
    dirty_highlight_lines.write().clear();

    let task = cx.spawn(async move |this, cx| {
      // Debounce: wait 150ms
      cx.background_executor()
        .timer(Duration::from_millis(HIGHLIGHT_DEBOUNCE_MS))
        .await;

      let batches = Arc::new(Mutex::new(VecDeque::new()));
      let done = Arc::new(AtomicBool::new(false));
      let error = Arc::new(Mutex::new(None));

      let worker_batches = batches.clone();
      let worker_done = done.clone();
      let worker_error = error.clone();
      let worker_generation = highlight_generation.clone();

      let _background_task = cx.background_executor().spawn(async move {
        let result = stream_highlights(
          text,
          config,
          worker_batches,
          worker_generation,
          my_generation,
        );
        if let Err(err) = result {
          *worker_error.lock() = Some(err);
        }
        worker_done.store(true, Ordering::Release);
      });

      loop {
        if highlight_generation.load(Ordering::Relaxed) != my_generation {
          break;
        }

        let mut pending = Vec::new();
        {
          let mut queue = batches.lock();
          while let Some(batch) = queue.pop_front() {
            pending.push(batch);
          }
        }

        if !pending.is_empty() {
          let _ = this.update(cx, |_doc, cx| {
            let mut highlights = highlights_cache.write();
            let mut dirty_lines = dirty_highlight_lines.write();
            let mut updated = false;

            for batch in pending {
              for (offset, line_highlights) in batch.lines.into_iter().enumerate() {
                let line_idx = batch.start_line + offset;
                if line_idx < highlights.len() {
                  highlights[line_idx] = Some(LineHighlight {
                    spans: line_highlights,
                    quality: HighlightQuality::Full,
                  });
                  dirty_lines.push(line_idx);
                  updated = true;
                }
              }
            }

            if updated {
              *highlights_version.write() += 1;
              cx.notify();
            }
          });
        }

        if done.load(Ordering::Acquire) {
          if let Some(err) = error.lock().take() {
            eprintln!("Syntax highlighting failed: {}", err);
            let _ = this.update(cx, |_doc, cx| {
              let mut highlights = highlights_cache.write();
              let mut dirty_lines = dirty_highlight_lines.write();
              for (line_idx, slot) in highlights.iter_mut().enumerate() {
                *slot = None;
                dirty_lines.push(line_idx);
              }
              *highlights_version.write() += 1;
              cx.notify();
            });
          }
          break;
        }

        cx.background_executor()
          .timer(Duration::from_millis(HIGHLIGHT_POLL_INTERVAL_MS))
          .await;
      }
    });

    self.pending_highlight_task = Some(task);
  }

  pub fn schedule_viewport_diff(
    &mut self,
    viewport: Range<usize>,
    margin_lines: usize,
    cx: &mut Context<Self>,
  ) {
    self.pending_viewport_diff_task = None;

    let (base_lines, base_hashes) = match (&self.diff_base_lines, &self.diff_base_hashes) {
      (Some(lines), Some(hashes)) => (Arc::clone(lines), Arc::clone(hashes)),
      _ => return,
    };

    let diff_epoch_snapshot = *self.diff_epoch.read();
    let state_snapshot = self.diff_state.read();
    let state = match state_snapshot.as_ref() {
      Some(state) => state,
      None => return,
    };

    if state.lines.is_empty() {
      return;
    }

    let view_len = state.lines.len();
    let view_start = viewport.start.saturating_sub(margin_lines).min(view_len);
    let view_end = (viewport.end + margin_lines).min(view_len);
    if view_start >= view_end {
      return;
    }

    let anchor_before = find_anchor_before(&state.lines, view_start);
    let anchor_after = find_anchor_after(&state.lines, view_end);

    let replace_start = anchor_before.map(|idx| idx + 1).unwrap_or(0);
    let replace_end = anchor_after.unwrap_or(view_len);
    if replace_start > replace_end || replace_end > view_len {
      return;
    }

    let base_len = base_lines.len();
    let current_line_count = self.buffer.len_lines();

    let base_start = anchor_before
      .and_then(|idx| state.lines[idx].base_line.map(|line| line + 1))
      .unwrap_or(0)
      .min(base_len);
    let current_start = anchor_before
      .and_then(|idx| state.lines[idx].current_line.map(|line| line + 1))
      .unwrap_or(0)
      .min(current_line_count);
    let base_end = anchor_after
      .and_then(|idx| state.lines[idx].base_line)
      .unwrap_or(base_len)
      .min(base_len);
    let current_end = anchor_after
      .and_then(|idx| state.lines[idx].current_line)
      .unwrap_or(current_line_count)
      .min(current_line_count);

    if base_start > base_end || current_start > current_end {
      return;
    }

    let base_slice: Vec<Arc<str>> = base_lines[base_start..base_end].to_vec();
    let base_hashes_slice: Vec<u64> = base_hashes[base_start..base_end].to_vec();
    let current_lines = collect_buffer_lines_range(&self.buffer, current_start, current_end);
    let current_hashes = hash_string_lines(&current_lines);

    let diff_state = self.diff_state.clone();
    let diff_version = self.diff_version.clone();
    let diff_epoch = self.diff_epoch.clone();
    let dirty_diff_lines = self.dirty_diff_lines.clone();
    let viewport_diff_generation = self.viewport_diff_generation.clone();
    let my_generation = viewport_diff_generation.fetch_add(1, Ordering::Relaxed) + 1;

    let task = cx.spawn(async move |this, cx| {
      let mut partial_lines = cx
        .background_executor()
        .spawn(async move {
          let mut partial_state = compute_diff_state(
            &base_slice,
            &base_hashes_slice,
            current_lines,
            current_hashes,
          );
          offset_diff_lines(&mut partial_state.lines, base_start, current_start);
          partial_state.lines
        })
        .await;

      if viewport_diff_generation.load(Ordering::Relaxed) != my_generation {
        return;
      }

      let _ = this.update(cx, |_doc, cx| {
        if *diff_epoch.read() != diff_epoch_snapshot {
          return;
        }

        let mut state_guard = diff_state.write();
        let Some(state) = state_guard.as_mut() else {
          return;
        };

        if replace_end > state.lines.len() {
          return;
        }

        let old_len = state.lines.len();
        let replace_old_len = replace_end.saturating_sub(replace_start);
        let partial_len = partial_lines.len();
        state
          .lines
          .splice(replace_start..replace_end, partial_lines.drain(..));

        let (line_starts, len_chars) = build_line_starts(&state.lines);
        state.line_starts = line_starts;
        state.len_chars = len_chars;
        let (current_to_view, base_to_view) =
          build_mappings(&state.lines, base_len, current_line_count);
        state.current_to_view = current_to_view;
        state.base_to_view = base_to_view;

        if state.lines.len() != old_len || partial_len != replace_old_len {
          *diff_epoch.write() += 1;
          dirty_diff_lines.write().clear();
        } else {
          let mut dirty = dirty_diff_lines.write();
          dirty.extend(replace_start..replace_start + partial_len);
        }

        *diff_version.write() += 1;
        cx.notify();
      });
    });

    self.pending_viewport_diff_task = Some(task);
  }

  pub fn schedule_recompute_diff(&mut self, cx: &mut Context<Self>) {
    self.pending_diff_task = None;

    let (base_lines, base_hashes) = match (&self.diff_base_lines, &self.diff_base_hashes) {
      (Some(lines), Some(hashes)) => (Arc::clone(lines), Arc::clone(hashes)),
      _ => return,
    };

    let rope_snapshot = self.buffer.snapshot();

    let diff_state = self.diff_state.clone();
    let diff_version = self.diff_version.clone();
    let diff_epoch = self.diff_epoch.clone();
    let dirty_diff_lines = self.dirty_diff_lines.clone();
    let diff_generation = self.diff_generation.clone();
    let my_generation = diff_generation.fetch_add(1, Ordering::Relaxed) + 1;

    let task = cx.spawn(async move |this, cx| {
      cx.background_executor()
        .timer(Duration::from_millis(DIFF_DEBOUNCE_MS))
        .await;

      if diff_generation.load(Ordering::Relaxed) != my_generation {
        return;
      }

      let state = cx
        .background_executor()
        .spawn(async move {
          let (current_lines, current_hashes) = collect_rope_lines_and_hashes(&rope_snapshot);
          compute_diff_state(&base_lines, &base_hashes, current_lines, current_hashes)
        })
        .await;

      if diff_generation.load(Ordering::Relaxed) != my_generation {
        return;
      }

      let _ = this.update(cx, |_doc, cx| {
        *diff_state.write() = Some(state);
        *diff_epoch.write() += 1;
        *diff_version.write() += 1;

        let mut dirty = dirty_diff_lines.write();
        dirty.clear();
        if let Some(state) = diff_state.read().as_ref() {
          dirty.extend(0..state.lines.len());
        }

        cx.notify();
      });
    });

    self.pending_diff_task = Some(task);
  }
}

const HIGHLIGHT_DEBOUNCE_MS: u64 = 150;
const DIFF_DEBOUNCE_MS: u64 = 300;
const HIGHLIGHT_BATCH_LINES: usize = 200;
const HIGHLIGHT_POLL_INTERVAL_MS: u64 = 16;
pub(crate) const VIEWPORT_HIGHLIGHT_MARGIN_LINES: usize = 100;
pub(crate) const VIEWPORT_DIFF_MARGIN_LINES: usize = 100;

fn build_viewport_text(buffer: &TextBuffer, start_line: usize, end_line: usize) -> String {
  let mut text = String::new();
  for line_idx in start_line..end_line {
    if line_idx > start_line {
      text.push('\n');
    }
    if let Some(content) = buffer.line_content(line_idx) {
      text.push_str(&content);
    }
  }
  text
}

fn build_viewport_text_from_lines(
  lines: &[Arc<str>],
  start_line: usize,
  end_line: usize,
) -> String {
  let mut text = String::new();
  let start = start_line.min(lines.len());
  let end = end_line.min(lines.len());
  for (offset, line) in lines[start..end].iter().enumerate() {
    if offset > 0 {
      text.push('\n');
    }
    text.push_str(line);
  }
  text
}

fn highlight_text_to_line_spans(
  text: &str,
  config: &'static syntax::LanguageConfig,
) -> Result<Vec<Arc<[HighlightSpan]>>, String> {
  let line_bounds = compute_line_bounds(text);
  let line_starts: Vec<usize> = line_bounds.iter().map(|(start, _)| *start).collect();
  let mut line_spans: Vec<Vec<HighlightSpan>> = vec![Vec::new(); line_bounds.len()];

  let mut highlighter = SyntaxHighlighter::new(config);
  highlighter.highlight_text_stream(
    text,
    |_| true,
    |span| {
      let start_line = line_index_for_byte(&line_starts, span.byte_range.start);
      let end_offset = span.byte_range.end.saturating_sub(1);
      let end_line = line_index_for_byte(&line_starts, end_offset);

      let line_slice = &line_bounds[start_line..=end_line];
      for (offset, &(line_start, line_end)) in line_slice.iter().enumerate() {
        let line_idx = start_line + offset;
        let local_start = span.byte_range.start.max(line_start) - line_start;
        let local_end = span.byte_range.end.min(line_end) - line_start;
        if local_end > local_start {
          line_spans[line_idx].push(HighlightSpan {
            byte_range: local_start..local_end,
            token_type: span.token_type,
          });
        }
      }
      true
    },
  )?;

  Ok(line_spans.into_iter().map(Arc::from).collect())
}

struct HighlightBatch {
  start_line: usize,
  lines: Vec<Arc<[HighlightSpan]>>,
}

fn stream_highlights(
  text: String,
  config: &'static syntax::LanguageConfig,
  batches: Arc<Mutex<VecDeque<HighlightBatch>>>,
  highlight_generation: Arc<AtomicUsize>,
  my_generation: usize,
) -> Result<(), String> {
  let line_bounds = compute_line_bounds(&text);
  let line_starts: Vec<usize> = line_bounds.iter().map(|(start, _)| *start).collect();
  let line_ends: Vec<usize> = line_bounds.iter().map(|(_, end)| *end).collect();
  let line_count = line_bounds.len();
  let state = RefCell::new(HighlightStreamState::new(line_count));

  let mut highlighter = SyntaxHighlighter::new(config);
  let cancel = |generation: &Arc<AtomicUsize>| generation.load(Ordering::Relaxed) != my_generation;

  highlighter.highlight_text_stream(
    &text,
    |range| {
      if cancel(&highlight_generation) {
        return false;
      }
      state
        .borrow_mut()
        .flush_ready(range.end, &line_ends, &batches);
      true
    },
    |span| {
      if cancel(&highlight_generation) {
        return false;
      }
      state
        .borrow_mut()
        .push_span(&line_bounds, &line_starts, span);
      true
    },
  )?;

  if !cancel(&highlight_generation) {
    state
      .borrow_mut()
      .flush_ready(usize::MAX, &line_ends, &batches);
  }

  Ok(())
}

struct HighlightStreamState {
  line_spans: Vec<Vec<HighlightSpan>>,
  next_flush_line: usize,
}

impl HighlightStreamState {
  fn new(line_count: usize) -> Self {
    Self {
      line_spans: vec![Vec::new(); line_count],
      next_flush_line: 0,
    }
  }

  fn push_span(
    &mut self,
    line_bounds: &[(usize, usize)],
    line_starts: &[usize],
    span: HighlightSpan,
  ) {
    if line_bounds.is_empty() {
      return;
    }
    let start_line = line_index_for_byte(line_starts, span.byte_range.start);
    let end_offset = span.byte_range.end.saturating_sub(1);
    let end_line = line_index_for_byte(line_starts, end_offset);

    let lines = &line_bounds[start_line..=end_line];
    for (offset, &(line_start, line_end)) in lines.iter().enumerate() {
      let line_idx = start_line + offset;
      let local_start = span.byte_range.start.max(line_start) - line_start;
      let local_end = span.byte_range.end.min(line_end) - line_start;
      if local_end > local_start {
        self.line_spans[line_idx].push(HighlightSpan {
          byte_range: local_start..local_end,
          token_type: span.token_type,
        });
      }
    }
  }

  fn flush_ready(
    &mut self,
    progress_end: usize,
    line_ends: &[usize],
    batches: &Mutex<VecDeque<HighlightBatch>>,
  ) {
    let ready_line = upper_bound(line_ends, progress_end);
    if ready_line <= self.next_flush_line {
      return;
    }

    let mut start = self.next_flush_line;
    while start < ready_line {
      let batch_end = (start + HIGHLIGHT_BATCH_LINES).min(ready_line);
      let mut batch_lines = Vec::with_capacity(batch_end - start);
      for line_idx in start..batch_end {
        let spans = std::mem::take(&mut self.line_spans[line_idx]);
        batch_lines.push(Arc::from(spans));
      }
      batches.lock().push_back(HighlightBatch {
        start_line: start,
        lines: batch_lines,
      });
      start = batch_end;
    }
    self.next_flush_line = ready_line;
  }
}

fn compute_line_bounds(text: &str) -> Vec<(usize, usize)> {
  let bytes = text.as_bytes();
  let mut bounds = Vec::new();
  let mut line_start = 0;

  for (idx, byte) in bytes.iter().enumerate() {
    if *byte == b'\n' {
      let mut line_end = idx;
      if line_end > line_start && bytes[line_end - 1] == b'\r' {
        line_end -= 1;
      }
      bounds.push((line_start, line_end));
      line_start = idx + 1;
    }
  }

  let mut line_end = bytes.len();
  if line_end > line_start && bytes[line_end - 1] == b'\r' {
    line_end -= 1;
  }
  bounds.push((line_start, line_end));

  bounds
}

fn line_index_for_byte(line_starts: &[usize], offset: usize) -> usize {
  match line_starts.binary_search(&offset) {
    Ok(idx) => idx,
    Err(idx) => idx.saturating_sub(1),
  }
}

fn upper_bound(values: &[usize], key: usize) -> usize {
  let mut low = 0;
  let mut high = values.len();
  while low < high {
    let mid = (low + high) / 2;
    if values[mid] <= key {
      low = mid + 1;
    } else {
      high = mid;
    }
  }
  low
}

fn char_to_line_with_starts(line_starts: &[usize], char_idx: usize) -> usize {
  if line_starts.is_empty() {
    return 0;
  }
  match line_starts.binary_search(&char_idx) {
    Ok(idx) => idx,
    Err(idx) => idx
      .saturating_sub(1)
      .min(line_starts.len().saturating_sub(1)),
  }
}

fn slice_chars(text: &str, start: usize, end: usize) -> String {
  if start >= end {
    return String::new();
  }
  let mut start_byte = None;
  let mut end_byte = None;
  for (idx, (byte_idx, _)) in text.char_indices().enumerate() {
    if idx == start {
      start_byte = Some(byte_idx);
    }
    if idx == end {
      end_byte = Some(byte_idx);
      break;
    }
  }
  let start_byte = start_byte.unwrap_or(text.len());
  let end_byte = end_byte.unwrap_or(text.len());
  text[start_byte..end_byte].to_string()
}

struct ViewChars<'a> {
  doc: &'a Document,
  line_idx: usize,
  char_idx: usize,
  line_chars: Vec<char>,
  line_count: usize,
  emit_newline: bool,
}

impl<'a> ViewChars<'a> {
  fn new(doc: &'a Document) -> Self {
    let line_count = doc.len_lines();
    let mut chars = Self {
      doc,
      line_idx: 0,
      char_idx: 0,
      line_chars: Vec::new(),
      line_count,
      emit_newline: false,
    };
    if line_count > 0 {
      chars.load_line();
    }
    chars
  }

  fn load_line(&mut self) {
    if self.line_idx >= self.line_count {
      return;
    }
    let line_text = self
      .doc
      .line_content(self.line_idx)
      .unwrap_or_default()
      .into_owned();
    self.line_chars = line_text.chars().collect();
    self.char_idx = 0;
    self.emit_newline = self.line_idx + 1 < self.line_count;
  }
}

impl Iterator for ViewChars<'_> {
  type Item = char;

  fn next(&mut self) -> Option<Self::Item> {
    if self.line_idx >= self.line_count {
      return None;
    }
    if self.char_idx < self.line_chars.len() {
      let ch = self.line_chars[self.char_idx];
      self.char_idx += 1;
      return Some(ch);
    }
    if self.emit_newline {
      self.emit_newline = false;
      return Some('\n');
    }
    self.line_idx += 1;
    if self.line_idx >= self.line_count {
      return None;
    }
    self.load_line();
    self.next()
  }
}

fn split_lines_to_arcs(text: &str) -> Vec<Arc<str>> {
  text
    .split('\n')
    .map(|line| line.strip_suffix('\r').unwrap_or(line))
    .map(Arc::<str>::from)
    .collect()
}

fn collect_buffer_lines_range(
  buffer: &TextBuffer,
  start_line: usize,
  end_line: usize,
) -> Vec<String> {
  let line_count = buffer.len_lines();
  let start = start_line.min(line_count);
  let end = end_line.min(line_count);
  let mut lines = Vec::with_capacity(end.saturating_sub(start));
  for line_idx in start..end {
    let line = buffer
      .line_content(line_idx)
      .map(|cow| cow.into_owned())
      .unwrap_or_default();
    lines.push(line);
  }
  lines
}

fn collect_rope_lines_and_hashes(rope: &Rope) -> (Vec<String>, Vec<u64>) {
  let line_count = rope.len_lines();
  let mut lines = Vec::with_capacity(line_count);
  let mut hashes = Vec::with_capacity(line_count);

  for line_idx in 0..line_count {
    let line_slice = rope.line(line_idx);
    let mut owned = line_slice.to_string();
    if owned.ends_with('\n') {
      owned.pop();
      if owned.ends_with('\r') {
        owned.pop();
      }
    }
    hashes.push(hash_line(&owned));
    lines.push(owned);
  }

  (lines, hashes)
}

fn hash_line(text: &str) -> u64 {
  let mut hasher = DefaultHasher::new();
  text.hash(&mut hasher);
  hasher.finish()
}

fn hash_lines(lines: &[Arc<str>]) -> Vec<u64> {
  lines.iter().map(|line| hash_line(line)).collect()
}

fn hash_string_lines(lines: &[String]) -> Vec<u64> {
  lines.iter().map(|line| hash_line(line)).collect()
}

fn build_line_starts(lines: &[DiffLine]) -> (Vec<usize>, usize) {
  let mut line_starts = Vec::with_capacity(lines.len());
  let mut offset = 0;
  for (idx, line) in lines.iter().enumerate() {
    line_starts.push(offset);
    offset += line.len_chars;
    if idx + 1 < lines.len() {
      offset += 1;
    }
  }
  (line_starts, offset)
}

fn build_mappings(
  lines: &[DiffLine],
  base_len: usize,
  current_len: usize,
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
  let mut current_to_view = vec![None; current_len];
  let mut base_to_view = vec![None; base_len];
  for (view_idx, line) in lines.iter().enumerate() {
    if let Some(current_line) = line.current_line
      && current_line < current_to_view.len()
      && current_to_view[current_line].is_none()
    {
      current_to_view[current_line] = Some(view_idx);
    }
    if let Some(base_line) = line.base_line
      && base_line < base_to_view.len()
      && base_to_view[base_line].is_none()
    {
      base_to_view[base_line] = Some(view_idx);
    }
  }
  (current_to_view, base_to_view)
}

fn build_diff_state_from_lines(
  lines: Vec<DiffLine>,
  base_len: usize,
  current_len: usize,
) -> DiffState {
  let (line_starts, len_chars) = build_line_starts(&lines);
  let (current_to_view, base_to_view) = build_mappings(&lines, base_len, current_len);
  DiffState {
    lines,
    line_starts,
    len_chars,
    current_to_view,
    base_to_view,
  }
}

fn offset_diff_lines(lines: &mut [DiffLine], base_offset: usize, current_offset: usize) {
  for line in lines.iter_mut() {
    if let Some(base_line) = line.base_line {
      line.base_line = Some(base_line + base_offset);
    }
    if let Some(current_line) = line.current_line {
      line.current_line = Some(current_line + current_offset);
    }
  }
}

fn is_anchor_line(line: &DiffLine) -> bool {
  matches!(line.kind, DiffLineKind::Unchanged)
    && line.base_line.is_some()
    && line.current_line.is_some()
}

fn find_anchor_before(lines: &[DiffLine], idx: usize) -> Option<usize> {
  if idx == 0 || lines.is_empty() {
    return None;
  }
  (0..idx)
    .rev()
    .find(|&line_idx| is_anchor_line(&lines[line_idx]))
}

fn find_anchor_after(lines: &[DiffLine], idx: usize) -> Option<usize> {
  if idx >= lines.len() {
    return None;
  }
  (idx..lines.len()).find(|&line_idx| is_anchor_line(&lines[line_idx]))
}

#[derive(Copy, Clone, Debug)]
enum DiffOp {
  Equal {
    base_start: usize,
    current_start: usize,
    len: usize,
  },
  Delete {
    base_start: usize,
    len: usize,
  },
  Insert {
    current_start: usize,
    len: usize,
  },
}

fn reorder_hunk_lines(lines: &mut Vec<DiffLine>, start: usize) {
  if start >= lines.len() {
    return;
  }
  let mut deleted = Vec::new();
  let mut added = Vec::new();
  let mut other = Vec::new();
  for line in lines.drain(start..) {
    match line.kind {
      DiffLineKind::Deleted => deleted.push(line),
      DiffLineKind::Added => added.push(line),
      _ => other.push(line),
    }
  }
  lines.extend(deleted);
  lines.extend(added);
  lines.extend(other);
}

#[derive(Clone, Debug)]
struct WordSpan {
  start: usize,
  end: usize,
  hash: u64,
}

fn word_spans(text: &str) -> Vec<WordSpan> {
  let mut spans = Vec::new();
  let mut char_pos = 0;
  for (_byte_idx, segment) in text.split_word_bound_indices() {
    let segment_len = segment.chars().count();
    if segment_len > 0 {
      let start = char_pos;
      let end = char_pos + segment_len;
      spans.push(WordSpan {
        start,
        end,
        hash: hash_line(segment),
      });
    }
    char_pos += segment_len;
  }
  spans
}

fn push_merged_range(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
  if range.start >= range.end {
    return;
  }
  if let Some(last) = ranges.last_mut()
    && range.start <= last.end
  {
    last.end = last.end.max(range.end);
    return;
  }
  ranges.push(range);
}

fn diff_word_ranges(base: &str, current: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
  if base == current {
    return (Vec::new(), Vec::new());
  }

  let base_spans = word_spans(base);
  let current_spans = word_spans(current);
  if base_spans.is_empty() && current_spans.is_empty() {
    return (Vec::new(), Vec::new());
  }

  let base_hashes: Vec<u64> = base_spans.iter().map(|span| span.hash).collect();
  let current_hashes: Vec<u64> = current_spans.iter().map(|span| span.hash).collect();
  let ops = diff_segment(&base_hashes, &current_hashes, 0, 0);

  let mut removed = Vec::new();
  let mut added = Vec::new();
  for op in ops {
    match op {
      DiffOp::Delete { base_start, len } => {
        let end = base_start + len;
        for span in &base_spans[base_start..end] {
          push_merged_range(&mut removed, span.start..span.end);
        }
      }
      DiffOp::Insert { current_start, len } => {
        let end = current_start + len;
        for span in &current_spans[current_start..end] {
          push_merged_range(&mut added, span.start..span.end);
        }
      }
      _ => {}
    }
  }

  (removed, added)
}

fn apply_word_diffs(lines: &mut [DiffLine], base_lines: &[Arc<str>], current_lines: &[String]) {
  let mut idx = 0;
  while idx < lines.len() {
    if lines[idx].gutter != DiffGutterKind::Modified {
      idx += 1;
      continue;
    }

    let start = idx;
    while idx < lines.len() && lines[idx].gutter == DiffGutterKind::Modified {
      idx += 1;
    }
    let end = idx;

    let mut deleted_lines = Vec::new();
    let mut added_lines = Vec::new();
    for (line_idx, line) in lines.iter().enumerate().take(end).skip(start) {
      match line.kind {
        DiffLineKind::Deleted => deleted_lines.push(line_idx),
        DiffLineKind::Added => added_lines.push(line_idx),
        _ => {}
      }
    }

    let pair_count = deleted_lines.len().min(added_lines.len());
    for pair_idx in 0..pair_count {
      let deleted_idx = deleted_lines[pair_idx];
      let added_idx = added_lines[pair_idx];
      let Some(base_line_idx) = lines[deleted_idx].base_line else {
        continue;
      };
      let Some(current_line_idx) = lines[added_idx].current_line else {
        continue;
      };
      let base_text = base_lines
        .get(base_line_idx)
        .map(|line| line.as_ref())
        .unwrap_or("");
      let current_text = current_lines
        .get(current_line_idx)
        .map(String::as_str)
        .unwrap_or("");
      let (removed_ranges, added_ranges) = diff_word_ranges(base_text, current_text);
      if !removed_ranges.is_empty() {
        lines[deleted_idx].word_diff_ranges = Some(Arc::from(removed_ranges.into_boxed_slice()));
      }
      if !added_ranges.is_empty() {
        lines[added_idx].word_diff_ranges = Some(Arc::from(added_ranges.into_boxed_slice()));
      }
    }
  }
}

fn compute_diff_state(
  base_lines: &[Arc<str>],
  base_hashes: &[u64],
  current_lines: Vec<String>,
  current_hashes: Vec<u64>,
) -> DiffState {
  let ops = diff_lines(base_hashes, &current_hashes);
  let base_lengths: Vec<usize> = base_lines.iter().map(|line| line.chars().count()).collect();
  let current_lengths: Vec<usize> = current_lines
    .iter()
    .map(|line| line.chars().count())
    .collect();

  let mut lines = Vec::new();
  let mut hunk_start = None;
  let mut hunk_has_insert = false;
  let mut hunk_has_delete = false;

  let close_hunk = |lines: &mut Vec<DiffLine>,
                    hunk_start: &mut Option<usize>,
                    hunk_has_insert: &mut bool,
                    hunk_has_delete: &mut bool| {
    if let Some(start) = *hunk_start {
      if *hunk_has_insert && *hunk_has_delete {
        reorder_hunk_lines(lines, start);
      }
      let kind = if *hunk_has_insert && !*hunk_has_delete {
        DiffGutterKind::Added
      } else {
        DiffGutterKind::Modified
      };
      for line in &mut lines[start..] {
        line.gutter = kind;
      }
    }
    *hunk_start = None;
    *hunk_has_insert = false;
    *hunk_has_delete = false;
  };

  for op in ops {
    match op {
      DiffOp::Equal {
        base_start,
        current_start,
        len,
      } => {
        close_hunk(
          &mut lines,
          &mut hunk_start,
          &mut hunk_has_insert,
          &mut hunk_has_delete,
        );
        for offset in 0..len {
          let base_line = base_start + offset;
          let current_line = current_start + offset;
          let len_chars = current_lengths.get(current_line).copied().unwrap_or(0);
          lines.push(DiffLine {
            kind: DiffLineKind::Unchanged,
            gutter: DiffGutterKind::None,
            base_line: Some(base_line),
            current_line: Some(current_line),
            deleted_text: None,
            word_diff_ranges: None,
            len_chars,
          });
        }
      }
      DiffOp::Delete { base_start, len } => {
        if hunk_start.is_none() {
          hunk_start = Some(lines.len());
        }
        hunk_has_delete = true;
        for offset in 0..len {
          let base_line = base_start + offset;
          let deleted_text = base_lines.get(base_line).cloned();
          let len_chars = base_lengths.get(base_line).copied().unwrap_or(0);
          lines.push(DiffLine {
            kind: DiffLineKind::Deleted,
            gutter: DiffGutterKind::None,
            base_line: Some(base_line),
            current_line: None,
            deleted_text,
            word_diff_ranges: None,
            len_chars,
          });
        }
      }
      DiffOp::Insert { current_start, len } => {
        if hunk_start.is_none() {
          hunk_start = Some(lines.len());
        }
        hunk_has_insert = true;
        for offset in 0..len {
          let current_line = current_start + offset;
          let len_chars = current_lengths.get(current_line).copied().unwrap_or(0);
          lines.push(DiffLine {
            kind: DiffLineKind::Added,
            gutter: DiffGutterKind::None,
            base_line: None,
            current_line: Some(current_line),
            deleted_text: None,
            word_diff_ranges: None,
            len_chars,
          });
        }
      }
    }
  }

  close_hunk(
    &mut lines,
    &mut hunk_start,
    &mut hunk_has_insert,
    &mut hunk_has_delete,
  );

  apply_word_diffs(&mut lines, base_lines, &current_lines);

  build_diff_state_from_lines(lines, base_lines.len(), current_lines.len())
}

fn diff_lines(base_hashes: &[u64], current_hashes: &[u64]) -> Vec<DiffOp> {
  let anchors = patience_anchors(base_hashes, current_hashes);
  let mut ops = Vec::new();
  let mut base_idx = 0;
  let mut current_idx = 0;

  for (anchor_base, anchor_current) in anchors {
    append_ops(
      &mut ops,
      diff_segment(
        &base_hashes[base_idx..anchor_base],
        &current_hashes[current_idx..anchor_current],
        base_idx,
        current_idx,
      ),
    );
    push_op(
      &mut ops,
      DiffOp::Equal {
        base_start: anchor_base,
        current_start: anchor_current,
        len: 1,
      },
    );
    base_idx = anchor_base + 1;
    current_idx = anchor_current + 1;
  }

  append_ops(
    &mut ops,
    diff_segment(
      &base_hashes[base_idx..],
      &current_hashes[current_idx..],
      base_idx,
      current_idx,
    ),
  );

  ops
}

fn append_ops(ops: &mut Vec<DiffOp>, new_ops: Vec<DiffOp>) {
  for op in new_ops {
    push_op(ops, op);
  }
}

fn push_op(ops: &mut Vec<DiffOp>, op: DiffOp) {
  if let Some(last) = ops.last_mut() {
    match (last, op) {
      (
        DiffOp::Equal {
          base_start,
          current_start,
          len,
        },
        DiffOp::Equal {
          base_start: new_base_start,
          current_start: new_current_start,
          len: new_len,
        },
      ) if *base_start + *len == new_base_start && *current_start + *len == new_current_start => {
        *len += new_len;
        return;
      }
      (
        DiffOp::Delete { base_start, len },
        DiffOp::Delete {
          base_start: new_base_start,
          len: new_len,
        },
      ) if *base_start + *len == new_base_start => {
        *len += new_len;
        return;
      }
      (
        DiffOp::Insert { current_start, len },
        DiffOp::Insert {
          current_start: new_current_start,
          len: new_len,
        },
      ) if *current_start + *len == new_current_start => {
        *len += new_len;
        return;
      }
      _ => {}
    }
  }
  ops.push(op);
}

fn diff_segment(
  base: &[u64],
  current: &[u64],
  base_start: usize,
  current_start: usize,
) -> Vec<DiffOp> {
  if base.is_empty() && current.is_empty() {
    return Vec::new();
  }
  if base.is_empty() {
    return vec![DiffOp::Insert {
      current_start,
      len: current.len(),
    }];
  }
  if current.is_empty() {
    return vec![DiffOp::Delete {
      base_start,
      len: base.len(),
    }];
  }

  let mut prefix = 0;
  while prefix < base.len() && prefix < current.len() && base[prefix] == current[prefix] {
    prefix += 1;
  }

  let mut suffix = 0;
  while suffix + prefix < base.len()
    && suffix + prefix < current.len()
    && base[base.len() - 1 - suffix] == current[current.len() - 1 - suffix]
  {
    suffix += 1;
  }

  let mut ops = Vec::new();
  if prefix > 0 {
    ops.push(DiffOp::Equal {
      base_start,
      current_start,
      len: prefix,
    });
  }

  let base_mid = &base[prefix..base.len() - suffix];
  let current_mid = &current[prefix..current.len() - suffix];

  if !base_mid.is_empty() || !current_mid.is_empty() {
    append_ops(
      &mut ops,
      diff_small_or_replace(
        base_mid,
        current_mid,
        base_start + prefix,
        current_start + prefix,
      ),
    );
  }

  if suffix > 0 {
    ops.push(DiffOp::Equal {
      base_start: base_start + base.len() - suffix,
      current_start: current_start + current.len() - suffix,
      len: suffix,
    });
  }

  ops
}

fn diff_small_or_replace(
  base: &[u64],
  current: &[u64],
  base_start: usize,
  current_start: usize,
) -> Vec<DiffOp> {
  const SMALL_DIFF_MAX: usize = 200;
  if base.is_empty() && current.is_empty() {
    return Vec::new();
  }
  if base.is_empty() {
    return vec![DiffOp::Insert {
      current_start,
      len: current.len(),
    }];
  }
  if current.is_empty() {
    return vec![DiffOp::Delete {
      base_start,
      len: base.len(),
    }];
  }
  if base.len() + current.len() <= SMALL_DIFF_MAX {
    return diff_small(base, current, base_start, current_start);
  }
  vec![
    DiffOp::Delete {
      base_start,
      len: base.len(),
    },
    DiffOp::Insert {
      current_start,
      len: current.len(),
    },
  ]
}

fn diff_small(
  base: &[u64],
  current: &[u64],
  base_start: usize,
  current_start: usize,
) -> Vec<DiffOp> {
  let n = base.len();
  let m = current.len();
  let mut dp = vec![0usize; (n + 1) * (m + 1)];

  for i in 0..n {
    for j in 0..m {
      let idx = (i + 1) * (m + 1) + (j + 1);
      if base[i] == current[j] {
        dp[idx] = dp[i * (m + 1) + j] + 1;
      } else {
        let left = dp[(i + 1) * (m + 1) + j];
        let up = dp[i * (m + 1) + (j + 1)];
        dp[idx] = left.max(up);
      }
    }
  }

  #[derive(Copy, Clone, Debug, PartialEq, Eq)]
  enum Edit {
    Equal,
    Delete,
    Insert,
  }

  let mut edits = Vec::new();
  let mut i = n;
  let mut j = m;
  while i > 0 && j > 0 {
    if base[i - 1] == current[j - 1] {
      edits.push(Edit::Equal);
      i -= 1;
      j -= 1;
    } else {
      let up = dp[(i - 1) * (m + 1) + j];
      let left = dp[i * (m + 1) + (j - 1)];
      if up >= left {
        edits.push(Edit::Delete);
        i -= 1;
      } else {
        edits.push(Edit::Insert);
        j -= 1;
      }
    }
  }
  while i > 0 {
    edits.push(Edit::Delete);
    i -= 1;
  }
  while j > 0 {
    edits.push(Edit::Insert);
    j -= 1;
  }
  edits.reverse();

  let mut ops = Vec::new();
  let mut base_pos = 0;
  let mut current_pos = 0;
  let mut run_kind = None;
  let mut run_len = 0;
  let mut run_base_start = 0;
  let mut run_current_start = 0;

  let flush = |ops: &mut Vec<DiffOp>,
               run_kind: &mut Option<Edit>,
               run_len: &mut usize,
               run_base_start: &mut usize,
               run_current_start: &mut usize| {
    if let Some(kind) = *run_kind {
      let op = match kind {
        Edit::Equal => DiffOp::Equal {
          base_start: base_start + *run_base_start,
          current_start: current_start + *run_current_start,
          len: *run_len,
        },
        Edit::Delete => DiffOp::Delete {
          base_start: base_start + *run_base_start,
          len: *run_len,
        },
        Edit::Insert => DiffOp::Insert {
          current_start: current_start + *run_current_start,
          len: *run_len,
        },
      };
      push_op(ops, op);
      *run_kind = None;
      *run_len = 0;
    }
  };

  for edit in edits {
    let (next_kind, advance_base, advance_current) = match edit {
      Edit::Equal => (Edit::Equal, 1, 1),
      Edit::Delete => (Edit::Delete, 1, 0),
      Edit::Insert => (Edit::Insert, 0, 1),
    };

    if run_kind != Some(next_kind) {
      flush(
        &mut ops,
        &mut run_kind,
        &mut run_len,
        &mut run_base_start,
        &mut run_current_start,
      );
      run_kind = Some(next_kind);
      run_len = 0;
      run_base_start = base_pos;
      run_current_start = current_pos;
    }

    run_len += 1;
    base_pos += advance_base;
    current_pos += advance_current;
  }

  flush(
    &mut ops,
    &mut run_kind,
    &mut run_len,
    &mut run_base_start,
    &mut run_current_start,
  );

  ops
}

fn patience_anchors(base: &[u64], current: &[u64]) -> Vec<(usize, usize)> {
  let mut base_info: HashMap<u64, (usize, usize)> = HashMap::new();
  let mut current_info: HashMap<u64, (usize, usize)> = HashMap::new();

  for (idx, hash) in base.iter().enumerate() {
    let entry = base_info.entry(*hash).or_insert((0, idx));
    entry.0 += 1;
  }
  for (idx, hash) in current.iter().enumerate() {
    let entry = current_info.entry(*hash).or_insert((0, idx));
    entry.0 += 1;
  }

  let mut pairs = Vec::new();
  for (hash, (base_count, base_idx)) in base_info {
    if base_count == 1
      && let Some((current_count, current_idx)) = current_info.get(&hash)
      && *current_count == 1
    {
      pairs.push((base_idx, *current_idx));
    }
  }

  pairs.sort_by_key(|(base_idx, _)| *base_idx);
  longest_increasing_subsequence(&pairs)
}

fn longest_increasing_subsequence(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
  if pairs.is_empty() {
    return Vec::new();
  }

  let mut pile_tops: Vec<usize> = Vec::new();
  let mut predecessors: Vec<Option<usize>> = vec![None; pairs.len()];

  for (idx, &(_, current_idx)) in pairs.iter().enumerate() {
    let pos = lower_bound_by(&pile_tops, current_idx, |&pile_idx| pairs[pile_idx].1);
    if pos > 0 {
      predecessors[idx] = Some(pile_tops[pos - 1]);
    }
    if pos == pile_tops.len() {
      pile_tops.push(idx);
    } else {
      pile_tops[pos] = idx;
    }
  }

  let mut result = Vec::new();
  let mut idx = *pile_tops.last().unwrap();
  loop {
    result.push(pairs[idx]);
    if let Some(prev) = predecessors[idx] {
      idx = prev;
    } else {
      break;
    }
  }
  result.reverse();
  result
}

fn lower_bound_by<T, F>(values: &[T], target: usize, mut key: F) -> usize
where
  F: FnMut(&T) -> usize,
{
  let mut low = 0;
  let mut high = values.len();
  while low < high {
    let mid = (low + high) / 2;
    if key(&values[mid]) < target {
      low = mid + 1;
    } else {
      high = mid;
    }
  }
  low
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::{AppContext, TestAppContext};

  #[gpui::test]
  fn test_new_document(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("", None, None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len(), 0);
      assert!(doc.is_empty());
      assert_eq!(doc.len_lines(), 1);
    });
  }

  #[gpui::test]
  fn test_with_text(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("hello world", None, None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len(), 11);
      assert!(!doc.is_empty());
      assert_eq!(doc.slice_to_string(0..5), "hello");
      assert_eq!(doc.slice_to_string(6..11), "world");
    });
  }

  #[gpui::test]
  fn test_insert_char(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("hello", None, None, cx));
    doc.update(cx, |doc, cx| {
      doc.insert_char(5, '!', cx);
      assert_eq!(doc.len(), 6);
      assert_eq!(doc.slice_to_string(0..6), "hello!");
    });
  }

  #[gpui::test]
  fn test_replace(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("hello world", None, None, cx));
    doc.update(cx, |doc, cx| {
      doc.replace(6..11, "Rust", cx);
      assert_eq!(doc.slice_to_string(0..10), "hello Rust");
    });
  }

  #[gpui::test]
  fn test_multiline_document(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("line1\nline2\nline3", None, None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len_lines(), 3);
      assert_eq!(doc.line_content(0).as_deref(), Some("line1"));
      assert_eq!(doc.line_content(1).as_deref(), Some("line2"));
      assert_eq!(doc.line_content(2).as_deref(), Some("line3"));
    });
  }

  #[gpui::test]
  fn test_line_range(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("abc\ndef\nghi", None, None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.line_range(0), Some(0..4));
      assert_eq!(doc.line_range(1), Some(4..8));
      assert_eq!(doc.line_range(2), Some(8..11));
    });
  }

  #[gpui::test]
  fn test_char_line_conversion(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("abc\ndef\nghi", None, None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.char_to_line(0), 0);
      assert_eq!(doc.char_to_line(4), 1);
      assert_eq!(doc.char_to_line(8), 2);

      assert_eq!(doc.line_to_char(0), 0);
      assert_eq!(doc.line_to_char(1), 4);
      assert_eq!(doc.line_to_char(2), 8);
    });
  }

  #[gpui::test]
  fn test_chars_iterator(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("abc", None, None, cx));
    doc.read_with(cx, |doc, _| {
      let chars: Vec<char> = doc.chars().collect();
      assert_eq!(chars, vec!['a', 'b', 'c']);
    });
  }

  #[gpui::test]
  fn test_unicode_handling(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("héllo 世界", None, None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len(), 8);
      assert_eq!(doc.slice_to_string(0..5), "héllo");
      assert_eq!(doc.slice_to_string(6..8), "世界");
    });
  }

  #[gpui::test]
  fn test_empty_lines(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("\n\n\n", None, None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len_lines(), 4);
      assert_eq!(doc.line_content(0).as_deref(), Some(""));
      assert_eq!(doc.line_content(1).as_deref(), Some(""));
      assert_eq!(doc.line_content(2).as_deref(), Some(""));
    });
  }

  #[gpui::test]
  fn test_replace_multiline(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("line1\nline2\nline3", None, None, cx));
    doc.update(cx, |doc, cx| {
      doc.replace(6..11, "new1\nnew2", cx);
      assert_eq!(doc.len_lines(), 4);
      assert_eq!(doc.line_content(0).as_deref(), Some("line1"));
      assert_eq!(doc.line_content(1).as_deref(), Some("new1"));
      assert_eq!(doc.line_content(2).as_deref(), Some("new2"));
      assert_eq!(doc.line_content(3).as_deref(), Some("line3"));
    });
  }

  #[gpui::test]
  fn test_line_content_removes_newlines(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("line1\n", None, None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.line_content(0).as_deref(), Some("line1"));
    });
  }

  #[gpui::test]
  fn test_line_content_removes_crlf(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("line1\r\n", None, None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.line_content(0).as_deref(), Some("line1"));
    });
  }

  #[gpui::test]
  fn test_diff_modified_line_order(_cx: &mut TestAppContext) {
    let base_text = "old";
    let current_text = "new";
    let base_lines = split_lines_to_arcs(base_text);
    let base_hashes = hash_lines(&base_lines);
    let current_lines: Vec<String> = split_lines_to_arcs(current_text)
      .iter()
      .map(|line| line.to_string())
      .collect();
    let current_hashes = hash_string_lines(&current_lines);

    let state = compute_diff_state(&base_lines, &base_hashes, current_lines, current_hashes);

    assert_eq!(state.lines.len(), 2);
    assert_eq!(state.lines[0].kind, DiffLineKind::Deleted);
    assert_eq!(state.lines[1].kind, DiffLineKind::Added);
  }

  #[gpui::test]
  fn test_document_undo(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("", None, None, cx));

    doc.update(cx, |doc, _cx| {
      doc.set_group_interval(std::time::Duration::from_millis(0));
    });

    doc.update(cx, |doc, cx| {
      doc.insert_char(0, 'a', cx);
      doc.insert_char(1, 'b', cx);
    });

    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.slice_to_string(0..2), "ab");
    });

    doc.update(cx, |doc, cx| {
      assert!(doc.undo(cx).is_some());
    });

    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len(), 1);
      assert_eq!(doc.slice_to_string(0..1), "a");
    });
  }

  #[gpui::test]
  fn test_document_redo(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("", None, None, cx));

    doc.update(cx, |doc, _cx| {
      doc.set_group_interval(std::time::Duration::from_millis(0));
    });

    doc.update(cx, |doc, cx| {
      doc.insert_char(0, 'a', cx);
      doc.undo(cx);
    });

    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len(), 0);
    });

    doc.update(cx, |doc, cx| {
      assert!(doc.redo(cx).is_some());
    });

    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.slice_to_string(0..1), "a");
    });
  }

  #[gpui::test]
  fn test_document_can_undo_redo(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("", None, None, cx));

    doc.update(cx, |doc, _cx| {
      doc.set_group_interval(std::time::Duration::from_millis(0));
    });

    doc.read_with(cx, |doc, _| {
      assert!(!doc.can_undo());
      assert!(!doc.can_redo());
    });

    doc.update(cx, |doc, cx| {
      doc.insert_char(0, 'a', cx);
    });

    doc.read_with(cx, |doc, _| {
      assert!(doc.can_undo());
      assert!(!doc.can_redo());
    });

    doc.update(cx, |doc, cx| {
      doc.undo(cx);
    });

    doc.read_with(cx, |doc, _| {
      assert!(!doc.can_undo());
      assert!(doc.can_redo());
    });
  }
}
