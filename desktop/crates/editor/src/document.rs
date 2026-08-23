use buffer::TextBuffer;
use gpui::{Context, Task};
use parking_lot::{Mutex, RwLock};
use std::{
  borrow::Cow,
  cell::RefCell,
  collections::VecDeque,
  ops::Range,
  sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
  },
  time::{Duration, Instant},
};
use syntax::languages;
use syntax::{
  HighlightSpan, SyntaxHighlighter, compute_line_bounds, highlight_text_to_line_spans,
  line_index_for_byte,
};

// Avoid expensive full-file highlight passes on very large buffers at open time.
const FULL_HIGHLIGHT_MAX_LINES: usize = 30_000;
const FULL_HIGHLIGHT_MAX_CHARS: usize = 2_000_000;
const INITIAL_VIEWPORT_HIGHLIGHT_LINES: usize = 300;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum HighlightQuality {
  Viewport = 1,
  Full = 2,
}

struct LineHighlight {
  spans: Arc<[HighlightSpan]>,
  quality: HighlightQuality,
}

pub struct Document {
  pub buffer: TextBuffer,

  highlighter: Option<SyntaxHighlighter>,
  // Line-local highlight cache (None = not computed yet)
  highlights: Arc<RwLock<Vec<Option<LineHighlight>>>>,
  pending_highlight_task: Option<Task<()>>,
  pending_viewport_highlight_task: Option<Task<()>>,

  pub highlights_version: Arc<RwLock<usize>>,
  pub highlights_epoch: Arc<RwLock<usize>>,
  dirty_highlight_lines: Arc<RwLock<Vec<usize>>>,
  highlight_generation: Arc<AtomicUsize>,
  viewport_highlight_generation: Arc<AtomicUsize>,
}

impl Document {
  /// Create document with language detection for syntax highlighting
  pub fn new(text: &str, file_ext: Option<&str>, cx: &mut Context<Self>) -> Self {
    let buffer = TextBuffer::from_text(text);

    let highlighter = file_ext
      .and_then(languages::detect_language_config)
      .map(SyntaxHighlighter::new);

    let mut doc = Self {
      buffer,
      highlighter,
      highlights: Arc::new(RwLock::new(Vec::new())),
      pending_highlight_task: None,
      pending_viewport_highlight_task: None,
      highlights_version: Arc::new(RwLock::new(0)),
      highlights_epoch: Arc::new(RwLock::new(0)),
      dirty_highlight_lines: Arc::new(RwLock::new(Vec::new())),
      highlight_generation: Arc::new(AtomicUsize::new(0)),
      viewport_highlight_generation: Arc::new(AtomicUsize::new(0)),
    };

    doc.schedule_initial_highlights(cx);

    doc
  }

  pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
    self.buffer.chars()
  }

  pub fn len(&self) -> usize {
    self.buffer.len()
  }

  pub fn len_lines(&self) -> usize {
    self.buffer.len_lines()
  }

  pub fn is_empty(&self) -> bool {
    self.buffer.is_empty()
  }

  pub fn line_content(&self, line_idx: usize) -> Option<Cow<'_, str>> {
    self.buffer.line_content(line_idx)
  }

  pub fn line_range(&self, line_idx: usize) -> Option<Range<usize>> {
    self.buffer.line_range(line_idx)
  }

  pub fn slice_to_string(&self, range: Range<usize>) -> String {
    self.buffer.slice_to_string(range)
  }

  pub fn char_to_line(&self, char_idx: usize) -> usize {
    self.buffer.char_to_line(char_idx)
  }

  pub fn line_to_char(&self, line_idx: usize) -> usize {
    self.buffer.line_to_char(line_idx)
  }

  #[cfg(test)]
  pub fn insert_char(&mut self, offset: usize, ch: char, cx: &mut Context<Self>) {
    self.buffer.transaction(Instant::now(), |buffer, tx| {
      buffer.insert(tx, offset, &ch.to_string());
    });
    cx.notify();
  }

  pub fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
    self.buffer.transaction(Instant::now(), |buffer, tx| {
      buffer.replace(tx, range, text);
    });
    cx.notify();
  }

  pub fn replace_all(&mut self, text: &str, cx: &mut Context<Self>) {
    self.buffer = TextBuffer::from_text(text);
    self.highlights.write().clear();
    self.dirty_highlight_lines.write().clear();
    self.schedule_initial_highlights(cx);
    cx.notify();
  }

  pub fn should_defer_full_highlight(&self) -> bool {
    should_defer_full_highlight(self.buffer.len_lines(), self.buffer.len())
  }

  fn schedule_initial_highlights(&mut self, cx: &mut Context<Self>) {
    if self.highlighter.is_none() {
      return;
    }

    let line_count = self.buffer.len_lines();
    let char_count = self.buffer.len();
    if should_defer_full_highlight(line_count, char_count) {
      let initial_end = line_count.clamp(1, INITIAL_VIEWPORT_HIGHLIGHT_LINES);
      self.schedule_viewport_highlights(0..initial_end, None, VIEWPORT_HIGHLIGHT_MARGIN_LINES, cx);
    } else {
      self.schedule_recompute_highlights(cx);
    }
  }

  pub fn undo(&mut self, cx: &mut Context<Self>) -> Option<buffer::TransactionId> {
    let result = self.buffer.undo();
    if result.is_some() {
      cx.notify();
    }
    result
  }

  pub fn redo(&mut self, cx: &mut Context<Self>) -> Option<buffer::TransactionId> {
    let result = self.buffer.redo();
    if result.is_some() {
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

  /// Schedule viewport-only highlight for responsive feedback.
  pub fn schedule_viewport_highlights(
    &mut self,
    viewport: Range<usize>,
    force_range: Option<Range<usize>>,
    margin_lines: usize,
    cx: &mut Context<Self>,
  ) {
    let viewports = [viewport];
    self.schedule_viewport_highlights_for_ranges(&viewports, force_range, margin_lines, cx);
  }

  pub fn schedule_viewport_highlights_for_ranges(
    &mut self,
    viewports: &[Range<usize>],
    force_range: Option<Range<usize>>,
    margin_lines: usize,
    cx: &mut Context<Self>,
  ) {
    self.pending_viewport_highlight_task = None;

    let Some(ref mut highlighter) = self.highlighter else {
      return;
    };

    let line_count = self.buffer.len_lines();
    if line_count == 0 {
      return;
    }

    let ranges = normalize_viewport_ranges(viewports, line_count, margin_lines);
    if ranges.is_empty() {
      return;
    }

    let segments = ranges
      .into_iter()
      .map(|range| ViewportHighlightSegment {
        start_line: range.start,
        text: build_viewport_text(&self.buffer, range.start, range.end),
      })
      .collect::<Vec<_>>();
    let config = highlighter.config;
    let highlights_cache = self.highlights.clone();
    let dirty_highlight_lines = self.dirty_highlight_lines.clone();
    let highlights_version = self.highlights_version.clone();
    let viewport_highlight_generation = self.viewport_highlight_generation.clone();
    let my_generation = viewport_highlight_generation.fetch_add(1, Ordering::Relaxed) + 1;

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_executor()
        .spawn(async move {
          let mut highlighted_segments = Vec::with_capacity(segments.len());
          for segment in segments {
            let line_spans = highlight_text_to_line_spans(&segment.text, config)?;
            highlighted_segments.push((segment.start_line, line_spans));
          }
          Ok::<_, String>(highlighted_segments)
        })
        .await;

      if viewport_highlight_generation.load(Ordering::Relaxed) != my_generation {
        return;
      }

      match result {
        Ok(segment_spans) => {
          let _ = this.update(cx, |_doc, cx| {
            let mut highlights = highlights_cache.write();
            if highlights.len() < line_count {
              highlights.resize_with(line_count, || None);
            }

            let mut dirty_lines = dirty_highlight_lines.write();
            let mut updated = false;

            for (start_line, line_spans) in segment_spans {
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

            if updated {
              *highlights_version.write() += 1;
              cx.notify();
            }
          });
        }
        Err(err) => {
          app_log::log!("Viewport highlighting failed: {}", err);
        }
      }
    });

    self.pending_viewport_highlight_task = Some(task);
  }

  /// Schedule async re-highlighting with debouncing
  pub fn schedule_recompute_highlights(&mut self, cx: &mut Context<Self>) {
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
            app_log::log!("Syntax highlighting failed: {}", err);
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
}

const HIGHLIGHT_DEBOUNCE_MS: u64 = 150;
const HIGHLIGHT_BATCH_LINES: usize = 200;
const HIGHLIGHT_POLL_INTERVAL_MS: u64 = 16;
pub(crate) const VIEWPORT_HIGHLIGHT_MARGIN_LINES: usize = 100;

fn should_defer_full_highlight(line_count: usize, char_count: usize) -> bool {
  line_count > FULL_HIGHLIGHT_MAX_LINES || char_count > FULL_HIGHLIGHT_MAX_CHARS
}

fn normalize_viewport_ranges(
  viewports: &[Range<usize>],
  line_count: usize,
  margin_lines: usize,
) -> Vec<Range<usize>> {
  let mut ranges = viewports
    .iter()
    .filter_map(|viewport| {
      let start_line = viewport.start.saturating_sub(margin_lines);
      let end_line = (viewport.end + margin_lines).min(line_count);
      (start_line < end_line).then_some(start_line..end_line)
    })
    .collect::<Vec<_>>();

  ranges.sort_by_key(|range| range.start);

  let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
  for range in ranges {
    if let Some(last) = merged.last_mut()
      && range.start <= last.end
    {
      last.end = last.end.max(range.end);
    } else {
      merged.push(range);
    }
  }

  merged
}

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

struct HighlightBatch {
  start_line: usize,
  lines: Vec<Arc<[HighlightSpan]>>,
}

struct ViewportHighlightSegment {
  start_line: usize,
  text: String,
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

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::{AppContext, TestAppContext};

  #[test]
  fn test_should_defer_full_highlight_for_large_line_count() {
    assert!(should_defer_full_highlight(
      FULL_HIGHLIGHT_MAX_LINES + 1,
      10
    ));
    assert!(!should_defer_full_highlight(FULL_HIGHLIGHT_MAX_LINES, 10));
  }

  #[test]
  fn test_should_defer_full_highlight_for_large_char_count() {
    assert!(should_defer_full_highlight(
      10,
      FULL_HIGHLIGHT_MAX_CHARS + 1
    ));
    assert!(!should_defer_full_highlight(10, FULL_HIGHLIGHT_MAX_CHARS));
  }

  #[test]
  fn test_normalize_viewport_ranges_merges_overlapping_margin_expansions() {
    let ranges = normalize_viewport_ranges(&[10..12, 14..16, 40..42], 100, 3);

    assert_eq!(ranges, vec![7..19, 37..45]);
  }

  #[gpui::test]
  fn test_new_document(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("", None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len(), 0);
      assert!(doc.is_empty());
      assert_eq!(doc.len_lines(), 1);
    });
  }

  #[gpui::test]
  fn test_with_text(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("hello world", None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len(), 11);
      assert!(!doc.is_empty());
      assert_eq!(doc.slice_to_string(0..5), "hello");
      assert_eq!(doc.slice_to_string(6..11), "world");
    });
  }

  #[gpui::test]
  fn test_insert_char(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("hello", None, cx));
    doc.update(cx, |doc, cx| {
      doc.insert_char(5, '!', cx);
      assert_eq!(doc.len(), 6);
      assert_eq!(doc.slice_to_string(0..6), "hello!");
    });
  }

  #[gpui::test]
  fn test_replace(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("hello world", None, cx));
    doc.update(cx, |doc, cx| {
      doc.replace(6..11, "Rust", cx);
      assert_eq!(doc.slice_to_string(0..10), "hello Rust");
    });
  }

  #[gpui::test]
  fn test_multiline_document(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("line1\nline2\nline3", None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len_lines(), 3);
      assert_eq!(doc.line_content(0).as_deref(), Some("line1"));
      assert_eq!(doc.line_content(1).as_deref(), Some("line2"));
      assert_eq!(doc.line_content(2).as_deref(), Some("line3"));
    });
  }

  #[gpui::test]
  fn test_line_range(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("abc\ndef\nghi", None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.line_range(0), Some(0..4));
      assert_eq!(doc.line_range(1), Some(4..8));
      assert_eq!(doc.line_range(2), Some(8..11));
    });
  }

  #[gpui::test]
  fn test_char_line_conversion(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("abc\ndef\nghi", None, cx));
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
    let doc = cx.new(|cx| Document::new("abc", None, cx));
    doc.read_with(cx, |doc, _| {
      let chars: Vec<char> = doc.chars().collect();
      assert_eq!(chars, vec!['a', 'b', 'c']);
    });
  }

  #[gpui::test]
  fn test_unicode_handling(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("héllo 世界", None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len(), 8);
      assert_eq!(doc.slice_to_string(0..5), "héllo");
      assert_eq!(doc.slice_to_string(6..8), "世界");
    });
  }

  #[gpui::test]
  fn test_empty_lines(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("\n\n\n", None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.len_lines(), 4);
      assert_eq!(doc.line_content(0).as_deref(), Some(""));
      assert_eq!(doc.line_content(1).as_deref(), Some(""));
      assert_eq!(doc.line_content(2).as_deref(), Some(""));
    });
  }

  #[gpui::test]
  fn test_replace_multiline(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("line1\nline2\nline3", None, cx));
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
    let doc = cx.new(|cx| Document::new("line1\n", None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.line_content(0).as_deref(), Some("line1"));
    });
  }

  #[gpui::test]
  fn test_line_content_removes_crlf(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("line1\r\n", None, cx));
    doc.read_with(cx, |doc, _| {
      assert_eq!(doc.line_content(0).as_deref(), Some("line1"));
    });
  }

  #[gpui::test]
  fn test_document_undo(cx: &mut TestAppContext) {
    let doc = cx.new(|cx| Document::new("", None, cx));

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
    let doc = cx.new(|cx| Document::new("", None, cx));

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
    let doc = cx.new(|cx| Document::new("", None, cx));

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
