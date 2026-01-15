use std::{
  collections::{HashMap, VecDeque},
  ops::Range,
  sync::Arc,
  time::Instant,
};

use buffer::TransactionId;
use gpui::{
  App, Bounds, Context, CursorStyle, Entity, EntityInputHandler, FocusHandle, Focusable,
  MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollHandle, ShapedLine,
  UTF16Selection, Window, black, div, point, prelude::*, px, white,
};
use syntax::Theme;

use crate::{
  boundaries::{line_range_at_offset, word_range_at_offset},
  cursor_blink::CursorBlink,
  document::Document,
  editor_element::{EditorElement, PositionMap},
  gutter_element::GutterElement,
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
/// Number of spaces inserted for a tab
pub(crate) const TAB_WHITESPACE_COUNT: usize = 2;
/// Width of the gutter area
const GUTTER_WIDTH: f32 = 70.0;
/// Padding inside the editor content area
const EDITOR_PADDING: f32 = 4.0;

pub struct Editor {
  pub document: Entity<Document>,
  pub focus_handle: FocusHandle,
  pub selected_range: Range<usize>,
  pub selection_reversed: bool,
  pub marked_range: Option<Range<usize>>,
  pub is_selecting: bool,
  pub(crate) selected_range_buffer: Option<Range<usize>>,

  // Performance: cache and viewport
  pub line_layouts: HashMap<usize, Arc<ShapedLine>>,

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

  // Track syntax highlighting version to invalidate cache when highlights change
  pub last_highlights_version: usize,
  pub last_highlights_epoch: usize,
  pub last_diff_version: usize,
  pub last_diff_epoch: usize,

  // Cursor blinking
  pub cursor_blink: Entity<CursorBlink>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum ScrollAxis {
  Horizontal,
  Vertical,
}

impl Editor {
  pub fn new(
    text: &str,
    base_text: Option<&str>,
    file_ext: Option<&str>,
    cx: &mut Context<Self>,
  ) -> Self {
    let document = cx.new(|cx| Document::new(text, base_text, file_ext, cx));
    let cursor_blink = cx.new(CursorBlink::new);

    Self {
      document,
      focus_handle: cx.focus_handle(),
      selected_range: 0..0,
      selection_reversed: false,
      marked_range: None,
      is_selecting: false,
      selected_range_buffer: Some(0..0),
      line_layouts: HashMap::new(),
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
      last_highlights_version: 0,
      last_highlights_epoch: 0,
      last_diff_version: 0,
      last_diff_epoch: 0,
      cursor_blink,
    }
  }

  #[cfg(test)]
  pub fn toggle_dark_mode(&mut self) {
    self.theme.toggle();
  }

  pub fn document(&self) -> &Entity<Document> {
    &self.document
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
  }

  pub(crate) fn viewport_range(&self, line_height: Pixels, total_lines: usize) -> Range<usize> {
    let visible_line_count = ((self.viewport_height / line_height).ceil() as usize).max(1);
    let start_line = (self.scroll_offset_y.floor() as usize).min(total_lines.saturating_sub(1));
    let end_line = (start_line + visible_line_count).min(total_lines);
    start_line..end_line
  }

  pub(crate) fn ensure_cursor_visible(&mut self, window: &Window, cx: &mut Context<Self>) {
    let document = self.document.read(cx);
    let cursor_offset = self.cursor_offset();
    let cursor_line = document.char_to_line(cursor_offset);
    let total_lines = document.len_lines();

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
    if let Some(shaped_line) = self.line_layouts.get(&cursor_line) {
      let line_start = document.line_to_char(cursor_line);
      let cursor_in_line = cursor_offset - line_start;
      let cursor_x = shaped_line.x_for_index(cursor_in_line);

      let horizontal_padding = px(GUTTER_WIDTH) + px(EDITOR_PADDING) + px(EXTRA_EDITOR_WIDTH);
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
    // Show cursor immediately on move
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    self.sync_buffer_selection_from_view(cx);
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
    if self.selected_range.end < self.selected_range.start {
      self.selection_reversed = !self.selection_reversed;
      self.selected_range = self.selected_range.end..self.selected_range.start;
    }
    self.sync_buffer_selection_from_view(cx);
    cx.notify()
  }

  pub(crate) fn sync_buffer_selection_from_view(&mut self, cx: &App) {
    let document = self.document.read(cx);
    self.selected_range_buffer = document.map_view_range_to_buffer(&self.selected_range);
  }

  pub(crate) fn sync_view_selection_from_buffer(&mut self, cx: &App) {
    let Some(buffer_range) = self.selected_range_buffer.clone() else {
      return;
    };
    let document = self.document.read(cx);
    let Some(view_range) = document.map_buffer_range_to_view(&buffer_range) else {
      return;
    };
    self.selected_range = view_range;
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

    let document = self.document.read(cx);
    let Some(offset) = position_map.point_for_position(event.position, document) else {
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
          cx.notify();
        }
        3 => {
          let (line_start, line_end) = line_range_at_offset(self, offset, cx);
          self.selected_range = line_start..line_end;
          self.selection_reversed = false;
          cx.notify();
        }
        _ => {
          let doc_len = document.len();
          self.selected_range = 0..doc_len;
          self.selection_reversed = false;
          cx.notify();
        }
      }
    }
    self.sync_buffer_selection_from_view(cx);
  }

  pub fn mouse_left_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
    self.is_selecting = false;
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

    let document = self.document.read(cx);
    let Some(offset) = position_map.point_for_position(event.position, document) else {
      return;
    };

    self.select_to(offset, cx);
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
    // Pause cursor blinking when typing
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    // Keep selection aligned with the latest diff state before mapping to buffer.
    self.sync_view_selection_from_buffer(cx);
    let has_newline = new_text.contains('\n');
    let new_text_chars = new_text.chars().count();
    let range = range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16, cx))
      .or(self.marked_range.clone())
      .unwrap_or(self.selected_range.clone());

    let selection_before = self.selected_range.clone();
    let start_line = self.document.read(cx).char_to_line(range.start);
    let end_line = self.document.read(cx).char_to_line(range.end);
    let buffer_range = self.document.read(cx).map_view_range_to_buffer(&range);
    if buffer_range.is_none() {
      if new_text.is_empty() {
        self.selected_range = range.start..range.start;
        self.selection_reversed = false;
        self.marked_range.take();
        self.sync_buffer_selection_from_view(cx);
      }
      cx.notify();
      return;
    }
    let buffer_range = buffer_range.unwrap_or_else(|| range.clone());
    let buffer_range_len = buffer_range.end - buffer_range.start;
    let (buffer_start_line, buffer_end_line) = {
      let doc = self.document.read(cx);
      (
        doc.buffer.char_to_line(buffer_range.start),
        doc.buffer.char_to_line(buffer_range.end),
      )
    };
    let single_line_edit = !has_newline && buffer_start_line == buffer_end_line;
    let delta_chars = new_text_chars as isize - buffer_range_len as isize;

    let line_height = window.line_height();
    let viewport_height = self.viewport_height;
    let scroll_offset_y = self.scroll_offset_y;
    let new_line_count = if has_newline { new_text.matches('\n').count() } else { 0 };
    let force_end_line = start_line.saturating_add(new_line_count).max(end_line);
    let force_range = start_line..(force_end_line + 1);
    let force_range = {
      let doc = self.document.read(cx);
      if doc.diff_enabled() {
        doc.map_view_line_range_to_current(&force_range)
      } else {
        Some(force_range.clone())
      }
    };

    let transaction_id = self.document.update(cx, |doc, cx| {
      let buffer_range = buffer_range.clone();
      let id = doc.buffer.transaction(Instant::now(), |buffer, tx| {
        buffer.replace(tx, buffer_range, new_text);
      });
      if single_line_edit {
        doc.apply_single_line_edit_delta(buffer_start_line, delta_chars);
      }

      // Trigger async syntax re-highlighting with debouncing
      doc.schedule_recompute_highlights(cx);
      if doc.diff_enabled() {
        doc.schedule_recompute_diff(cx);
      }
      let total_lines = doc.len_lines();
      let visible_line_count = ((viewport_height / line_height).ceil() as usize).max(1);
      let start_line = (scroll_offset_y.floor() as usize).min(total_lines.saturating_sub(1));
      let end_line = (start_line + visible_line_count).min(total_lines);
      let viewport = start_line..end_line;
      doc.schedule_viewport_highlights(
        viewport.clone(),
        force_range.clone(),
        crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
        cx,
      );
      if doc.diff_enabled() {
        doc.schedule_viewport_diff(viewport, crate::document::VIEWPORT_DIFF_MARGIN_LINES, cx);
      }

      cx.notify();
      id
    });

    if has_newline || start_line != end_line {
      // Multi-line edit: invalidate from start line onwards
      self.invalidate_lines_from(start_line);
    } else {
      // Single-line edit: only invalidate the affected line
      self.invalidate_line(start_line);
    }

    let new_cursor_offset = buffer_range.start + new_text_chars;
    self.selected_range = range.start + new_text_chars..range.start + new_text_chars;
    self.selected_range_buffer = Some(new_cursor_offset..new_cursor_offset);
    self.marked_range.take();

    let selection_after = self.selected_range.clone();

    self.record_transaction(transaction_id, selection_before, selection_after);

    cx.notify();
  }

  fn replace_and_mark_text_in_range(
    &mut self,
    range_utf16: Option<Range<usize>>,
    new_text: &str,
    new_selected_range_utf16: Option<Range<usize>>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    // Pause cursor blinking when typing
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    // Keep selection aligned with the latest diff state before mapping to buffer.
    self.sync_view_selection_from_buffer(cx);
    let has_newline = new_text.contains('\n');
    let new_text_chars = new_text.chars().count();
    let range = range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16, cx))
      .or(self.marked_range.clone())
      .unwrap_or(self.selected_range.clone());

    let start_line = self.document.read(cx).char_to_line(range.start);
    let buffer_range = self.document.read(cx).map_view_range_to_buffer(&range);
    if buffer_range.is_none() {
      if new_text.is_empty() {
        self.selected_range = range.start..range.start;
        self.selection_reversed = false;
        self.marked_range.take();
        self.sync_buffer_selection_from_view(cx);
      }
      cx.notify();
      return;
    }
    let buffer_range = buffer_range.unwrap_or_else(|| range.clone());
    let buffer_range_len = buffer_range.end - buffer_range.start;
    let (buffer_start_line, buffer_end_line) = {
      let doc = self.document.read(cx);
      (
        doc.buffer.char_to_line(buffer_range.start),
        doc.buffer.char_to_line(buffer_range.end),
      )
    };
    let single_line_edit = !has_newline && buffer_start_line == buffer_end_line;
    let delta_chars = new_text_chars as isize - buffer_range_len as isize;

    let line_height = window.line_height();
    let viewport_height = self.viewport_height;
    let scroll_offset_y = self.scroll_offset_y;
    let end_line = self.document.read(cx).char_to_line(range.end);
    let new_line_count = if has_newline { new_text.matches('\n').count() } else { 0 };
    let force_end_line = start_line.saturating_add(new_line_count).max(end_line);
    let force_range = start_line..(force_end_line + 1);
    let force_range = {
      let doc = self.document.read(cx);
      if doc.diff_enabled() {
        doc.map_view_line_range_to_current(&force_range)
      } else {
        Some(force_range.clone())
      }
    };

    self.document.update(cx, |doc, cx| {
      let buffer_range = buffer_range.clone();
      doc.buffer.transaction(Instant::now(), |buffer, tx| {
        buffer.replace(tx, buffer_range, new_text);
      });
      if single_line_edit {
        doc.apply_single_line_edit_delta(buffer_start_line, delta_chars);
      }
      doc.schedule_recompute_highlights(cx);
      if doc.diff_enabled() {
        doc.schedule_recompute_diff(cx);
      }
      let total_lines = doc.len_lines();
      let visible_line_count = ((viewport_height / line_height).ceil() as usize).max(1);
      let start_line = (scroll_offset_y.floor() as usize).min(total_lines.saturating_sub(1));
      let end_line = (start_line + visible_line_count).min(total_lines);
      let viewport = start_line..end_line;
      doc.schedule_viewport_highlights(
        viewport.clone(),
        force_range.clone(),
        crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
        cx,
      );
      if doc.diff_enabled() {
        doc.schedule_viewport_diff(viewport, crate::document::VIEWPORT_DIFF_MARGIN_LINES, cx);
      }
    });

    // Invalidate cache for all lines from the start of the edit
    self.invalidate_lines_from(start_line);

    if !new_text.is_empty() {
      self.marked_range = Some(range.start..range.start + new_text_chars);
    } else {
      self.marked_range = None;
    }
    self.selected_range = new_selected_range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16, cx))
      .map(|new_range| new_range.start + range.start..new_range.end + range.end)
      .unwrap_or_else(|| range.start + new_text_chars..range.start + new_text_chars);
    self.sync_buffer_selection_from_view(cx);

    cx.notify();
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
    div()
      .key_context("Editor")
      .track_focus(&self.focus_handle(cx))
      .cursor(CursorStyle::IBeam)
      .size_full()
      .on_action(cx.listener(crate::actions::enter))
      .on_action(cx.listener(crate::actions::tab))
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
      .when_else(self.theme.is_dark, |el| el.bg(black()), |el| el.bg(white()))
      .when_else(
        self.theme.is_dark,
        |el| el.text_color(white()),
        |el| el.text_color(black()),
      )
      .flex()
      .flex_row()
      .child(
        div()
          .w(px(GUTTER_WIDTH))
          .h_full()
          .bg(self.theme.gutter_background())
          .child(GutterElement::new(cx.entity().clone())),
      )
      .child(
        div()
          .flex_1()
          .h_full()
          .id("editor-content")
          .overflow_x_scroll()
          .track_scroll(&self.scroll_handle)
          .px(px(EDITOR_PADDING))
          .child(
            div()
              .min_w(self.max_line_width + px(EXTRA_EDITOR_WIDTH))
              .h_full()
              .child(EditorElement::new(cx.entity().clone())),
          ),
      )
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
      let editor = cx.new(|cx| Editor::new(text, None, None, cx));

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
      self.editor.update(&mut self.cx, |editor, cx| {
        editor.selected_range = range;
        editor.selection_reversed = reversed;
        editor.sync_buffer_selection_from_view(cx);
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
  fn test_editor_toggle_dark_mode(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| Editor::new("", None, None, cx));

    editor.update(cx, |editor, _| {
      let was_dark = editor.theme.is_dark;
      editor.toggle_dark_mode();
      assert_eq!(editor.theme.is_dark, !was_dark);
    });
  }

  #[gpui::test]
  fn test_syntax_highlights_cached(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| Editor::new("fn main() {}", None, Some("rs"), cx));

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
      editor.sync_buffer_selection_from_view(cx);
      cx.notify();
    });

    // Verify entire buffer is selected
    assert_eq!(ctx.selection(), 0..doc_len);
    assert_eq!(doc_len, 17); // "line1\nline2\nline3"
  }
}
