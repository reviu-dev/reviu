use std::{
  ops::Range,
  time::{Duration, Instant},
};

use gpui::{
  App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
  EntityInputHandler, FocusHandle, Focusable, GlobalElementId, LayoutId, MouseButton,
  MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, Rgba,
  SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine, actions, div,
  fill, point, prelude::*, px, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

actions!(
  text_input,
  [
    BackspaceWord,
    BackspaceAll,
    Backspace,
    Delete,
    Up,
    Down,
    CmdUp,
    CmdDown,
    Left,
    AltLeft,
    CmdLeft,
    Right,
    AltRight,
    CmdRight,
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    SelectWordLeft,
    SelectWordRight,
    SelectCmdLeft,
    SelectCmdRight,
    SelectCmdUp,
    SelectCmdDown,
    SelectAll,
    Home,
    End,
    ShowCharacterPalette,
    Paste,
    Cut,
    Copy,
  ]
);

#[derive(Clone, Copy)]
pub struct TextInputColors {
  pub placeholder: Rgba,
  pub selection: Rgba,
  pub cursor: Rgba,
}

impl TextInputColors {
  pub fn new(placeholder: Rgba, selection: Rgba, cursor: Rgba) -> Self {
    Self {
      placeholder,
      selection,
      cursor,
    }
  }
}

#[derive(Clone)]
struct LineLayoutInfo {
  start: usize,
  len: usize,
  layout: WrappedLine,
  y: Pixels,
  height: Pixels,
}

#[derive(Clone)]
pub struct TextInput {
  focus_handle: FocusHandle,
  content: SharedString,
  placeholder: SharedString,
  selected_range: Range<usize>,
  selection_reversed: bool,
  marked_range: Option<Range<usize>>,
  last_layouts: Vec<LineLayoutInfo>,
  last_bounds: Option<Bounds<Pixels>>,
  last_line_height: Pixels,
  desired_height: Pixels,
  target_x: Option<Pixels>,
  is_selecting: bool,
  last_click_time: Option<Instant>,
  last_click_pos: Option<Point<Pixels>>,
  click_count: u8,
  placeholder_color: gpui::Hsla,
  selection_color: gpui::Hsla,
  cursor_color: gpui::Hsla,
}

impl TextInput {
  pub fn new(
    placeholder: impl Into<SharedString>,
    colors: TextInputColors,
    cx: &mut Context<Self>,
  ) -> Self {
    Self {
      focus_handle: cx.focus_handle(),
      content: "".into(),
      placeholder: placeholder.into(),
      selected_range: 0..0,
      selection_reversed: false,
      marked_range: None,
      last_layouts: Vec::new(),
      last_bounds: None,
      last_line_height: px(0.0),
      desired_height: px(0.0),
      target_x: None,
      is_selecting: false,
      last_click_time: None,
      last_click_pos: None,
      click_count: 0,
      placeholder_color: colors.placeholder.into(),
      selection_color: colors.selection.into(),
      cursor_color: colors.cursor.into(),
    }
  }

  pub fn set_colors(&mut self, colors: TextInputColors) {
    self.placeholder_color = colors.placeholder.into();
    self.selection_color = colors.selection.into();
    self.cursor_color = colors.cursor.into();
  }

  pub fn text(&self) -> String {
    self.content.to_string()
  }

  pub fn clear(&mut self) {
    self.content = "".into();
    self.selected_range = 0..0;
    self.selection_reversed = false;
    self.marked_range = None;
    self.last_layouts.clear();
    self.last_bounds = None;
    self.last_line_height = px(0.0);
    self.desired_height = px(0.0);
    self.target_x = None;
    self.is_selecting = false;
    self.last_click_time = None;
    self.last_click_pos = None;
    self.click_count = 0;
  }

  fn sanitize_text(text: &str) -> String {
    text.replace('\n', " ")
  }

  fn reset_target_x(&mut self) {
    self.target_x = None;
  }

  fn caret_index(&self) -> usize {
    if self.selection_reversed {
      self.selected_range.start
    } else {
      self.selected_range.end
    }
  }

  fn position_for_index(&self, index: usize, line_height: Pixels) -> Option<Point<Pixels>> {
    if self.last_layouts.is_empty() || line_height == px(0.0) {
      return None;
    }
    let target = index;
    for line in &self.last_layouts {
      let line_end = line.start + line.len;
      if target <= line_end {
        let rel = target.saturating_sub(line.start);
        if let Some(pos) = line.layout.position_for_index(rel, line_height) {
          return Some(point(pos.x, line.y + pos.y));
        }
        return Some(point(px(0.0), line.y));
      }
    }
    let last = self.last_layouts.last()?;
    let rel = last.len;
    last
      .layout
      .position_for_index(rel, line_height)
      .map(|pos| point(pos.x, last.y + pos.y))
      .or(Some(point(px(0.0), last.y)))
  }

  fn index_for_position(&self, position: Point<Pixels>, line_height: Pixels) -> Option<usize> {
    if self.last_layouts.is_empty() || line_height == px(0.0) {
      return None;
    }

    for line in &self.last_layouts {
      if position.y < line.y {
        continue;
      }
      if position.y < line.y + line.height {
        let local = point(position.x, position.y - line.y);
        let result = line.layout.closest_index_for_position(local, line_height);
        let idx = match result {
          Ok(idx) | Err(idx) => idx,
        };
        return Some(line.start + idx.min(line.len));
      }
    }

    let last = self.last_layouts.last()?;
    Some(last.start + last.len)
  }

  fn move_vertical(&mut self, delta_lines: i32, select: bool, cx: &mut Context<Self>) {
    if self.last_layouts.is_empty() || self.last_line_height == px(0.0) {
      return;
    }
    let line_height = self.last_line_height;
    let caret_index = self.caret_index();
    let Some(caret_pos) = self.position_for_index(caret_index, line_height) else {
      return;
    };
    let target_x = self.target_x.unwrap_or(caret_pos.x);
    self.target_x = Some(target_x);
    let target_y = caret_pos.y + line_height * (delta_lines as f32);
    let new_index = self
      .index_for_position(point(target_x, target_y), line_height)
      .unwrap_or(caret_index);
    if select {
      self.select_to(new_index, cx);
    } else {
      self.move_to(new_index, cx);
    }
  }

  fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    if self.selected_range.is_empty() {
      self.move_to(self.previous_boundary(self.cursor_offset()), cx);
    } else {
      self.move_to(self.selected_range.start, cx)
    }
  }

  fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    if self.selected_range.is_empty() {
      self.move_to(self.next_boundary(self.selected_range.end), cx);
    } else {
      self.move_to(self.selected_range.end, cx)
    }
  }

  fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.select_to(self.previous_boundary(self.cursor_offset()), cx);
  }

  fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.select_to(self.next_boundary(self.cursor_offset()), cx);
  }

  fn alt_left(&mut self, _: &AltLeft, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    if self.selected_range.is_empty() {
      self.move_to(self.previous_word_boundary(self.cursor_offset()), cx);
    } else {
      self.move_to(self.selected_range.start, cx)
    }
  }

  fn alt_right(&mut self, _: &AltRight, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    if self.selected_range.is_empty() {
      self.move_to(self.next_word_boundary(self.selected_range.end), cx);
    } else {
      self.move_to(self.selected_range.end, cx)
    }
  }

  fn cmd_left(&mut self, _: &CmdLeft, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.move_to(0, cx);
  }

  fn cmd_right(&mut self, _: &CmdRight, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.move_to(self.content.len(), cx);
  }

  fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.select_to(self.previous_word_boundary(self.cursor_offset()), cx);
  }

  fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.select_to(self.next_word_boundary(self.selected_range.end), cx);
  }

  fn select_cmd_left(&mut self, _: &SelectCmdLeft, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.select_to(0, cx);
  }

  fn select_cmd_right(&mut self, _: &SelectCmdRight, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.select_to(self.content.len(), cx);
  }

  fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.move_to(0, cx);
    self.select_to(self.content.len(), cx)
  }

  fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.move_to(0, cx);
  }

  fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.move_to(self.content.len(), cx);
  }

  fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    if self.selected_range.is_empty() {
      self.select_to(self.previous_boundary(self.cursor_offset()), cx)
    }
    self.replace_text_in_range(None, "", window, cx)
  }

  fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    if self.selected_range.is_empty() {
      self.select_to(self.next_boundary(self.cursor_offset()), cx)
    }
    self.replace_text_in_range(None, "", window, cx)
  }

  fn backspace_word(&mut self, _: &BackspaceWord, window: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    if self.selected_range.is_empty() {
      let start = self.previous_word_boundary(self.cursor_offset());
      self.selected_range = start..self.cursor_offset();
      self.selection_reversed = false;
    }
    self.replace_text_in_range(None, "", window, cx)
  }

  fn backspace_all(&mut self, _: &BackspaceAll, window: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    if self.selected_range.is_empty() {
      self.selected_range = 0..self.cursor_offset();
      self.selection_reversed = false;
    }
    self.replace_text_in_range(None, "", window, cx)
  }

  fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
    self.move_vertical(-1, false, cx);
  }

  fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
    self.move_vertical(1, false, cx);
  }

  fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
    self.move_vertical(-1, true, cx);
  }

  fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
    self.move_vertical(1, true, cx);
  }

  fn cmd_up(&mut self, _: &CmdUp, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.move_to(0, cx);
  }

  fn cmd_down(&mut self, _: &CmdDown, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.move_to(self.content.len(), cx);
  }

  fn select_cmd_up(&mut self, _: &SelectCmdUp, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.select_to(0, cx);
  }

  fn select_cmd_down(&mut self, _: &SelectCmdDown, _: &mut Window, cx: &mut Context<Self>) {
    self.reset_target_x();
    self.select_to(self.content.len(), cx);
  }

  fn show_character_palette(
    &mut self,
    _: &ShowCharacterPalette,
    window: &mut Window,
    _: &mut Context<Self>,
  ) {
    window.show_character_palette();
  }

  fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
      let sanitized = Self::sanitize_text(&text);
      self.replace_text_in_range(None, &sanitized, window, cx);
    }
  }

  fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
    if !self.selected_range.is_empty() {
      cx.write_to_clipboard(ClipboardItem::new_string(
        self.content[self.selected_range.clone()].to_string(),
      ));
    }
  }

  fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
    if !self.selected_range.is_empty() {
      cx.write_to_clipboard(ClipboardItem::new_string(
        self.content[self.selected_range.clone()].to_string(),
      ));
      self.replace_text_in_range(None, "", window, cx)
    }
  }

  fn on_mouse_down(
    &mut self,
    event: &MouseDownEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.reset_target_x();
    let click_index = self.index_for_mouse_position(event.position);
    let now = Instant::now();
    let is_multi_click = self
      .last_click_time
      .map(|last| now.duration_since(last) <= Duration::from_millis(350))
      .unwrap_or(false)
      && self
        .last_click_pos
        .map(|last| {
          let dx = (event.position.x - last.x).abs();
          let dy = (event.position.y - last.y).abs();
          dx <= px(6.0) && dy <= px(6.0)
        })
        .unwrap_or(false);

    if is_multi_click {
      self.click_count = (self.click_count + 1).min(3);
    } else {
      self.click_count = 1;
    }
    self.last_click_time = Some(now);
    self.last_click_pos = Some(event.position);

    if event.modifiers.shift {
      self.is_selecting = true;
      self.select_to(click_index, cx);
      return;
    }

    match self.click_count {
      2 => {
        let (start, end) = self.word_range_at_offset(click_index);
        self.selected_range = start..end;
        self.selection_reversed = false;
        self.is_selecting = false;
        cx.notify();
      }
      3 => {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        self.is_selecting = false;
        cx.notify();
      }
      _ => {
        self.is_selecting = true;
        self.move_to(click_index, cx);
      }
    }
  }

  fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
    self.is_selecting = false;
  }

  fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
    if self.is_selecting {
      self.select_to(self.index_for_mouse_position(event.position), cx);
    }
  }

  fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
    self.selected_range = offset..offset;
    cx.notify()
  }

  fn cursor_offset(&self) -> usize {
    if self.selection_reversed {
      self.selected_range.start
    } else {
      self.selected_range.end
    }
  }

  fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
    if self.content.is_empty() {
      return 0;
    }
    let Some(bounds) = self.last_bounds.as_ref() else {
      return 0;
    };
    let local = bounds.localize(&position).unwrap_or_else(|| {
      let x = (position.x - bounds.left()).max(px(0.0)).min(bounds.size.width);
      let y = (position.y - bounds.top()).max(px(0.0)).min(bounds.size.height);
      point(x, y)
    });
    self
      .index_for_position(local, self.last_line_height)
      .unwrap_or(self.content.len())
  }

  fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
    if self.selection_reversed {
      self.selected_range.start = offset
    } else {
      self.selected_range.end = offset
    };
    if self.selected_range.end < self.selected_range.start {
      self.selection_reversed = !self.selection_reversed;
      self.selected_range = self.selected_range.end..self.selected_range.start;
    }
    cx.notify()
  }

  fn offset_from_utf16(&self, offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;

    for ch in self.content.chars() {
      if utf16_count >= offset {
        break;
      }
      utf16_count += ch.len_utf16();
      utf8_offset += ch.len_utf8();
    }

    utf8_offset
  }

  fn offset_to_utf16(&self, offset: usize) -> usize {
    let mut utf16_offset = 0;
    let mut utf8_count = 0;

    for ch in self.content.chars() {
      if utf8_count >= offset {
        break;
      }
      utf8_count += ch.len_utf8();
      utf16_offset += ch.len_utf16();
    }

    utf16_offset
  }

  fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
    self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
  }

  fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
    self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
  }

  fn previous_boundary(&self, offset: usize) -> usize {
    self
      .content
      .grapheme_indices(true)
      .rev()
      .find_map(|(idx, _)| (idx < offset).then_some(idx))
      .unwrap_or(0)
  }

  fn next_boundary(&self, offset: usize) -> usize {
    self
      .content
      .grapheme_indices(true)
      .find_map(|(idx, _)| (idx > offset).then_some(idx))
      .unwrap_or(self.content.len())
  }

  fn previous_word_boundary(&self, offset: usize) -> usize {
    if offset == 0 {
      return 0;
    }

    let mut last_segment_start = 0;
    for (idx, segment) in self.content.split_word_bound_indices() {
      let segment_end = idx + segment.len();
      if segment.trim().is_empty() {
        continue;
      }
      if idx < offset && offset <= segment_end {
        return idx;
      }
      if idx < offset {
        last_segment_start = idx;
      }
    }
    last_segment_start
  }

  fn next_word_boundary(&self, offset: usize) -> usize {
    let len = self.content.len();
    if offset >= len {
      return len;
    }

    for (idx, segment) in self.content.split_word_bound_indices() {
      let segment_end = idx + segment.len();
      if segment.trim().is_empty() {
        continue;
      }
      if offset <= idx {
        return segment_end;
      }
      if offset < segment_end {
        return segment_end;
      }
    }
    len
  }

  fn word_range_at_offset(&self, offset: usize) -> (usize, usize) {
    let len = self.content.len();
    if offset >= len {
      return (len, len);
    }

    for (idx, segment) in self.content.split_word_bound_indices() {
      let segment_end = idx + segment.len();
      if idx <= offset && offset < segment_end {
        return (idx, segment_end);
      }
    }

    (offset, offset)
  }
}

impl EntityInputHandler for TextInput {
  fn text_for_range(
    &mut self,
    range_utf16: Range<usize>,
    actual_range: &mut Option<Range<usize>>,
    _window: &mut Window,
    _cx: &mut Context<Self>,
  ) -> Option<String> {
    let range = self.range_from_utf16(&range_utf16);
    actual_range.replace(self.range_to_utf16(&range));
    Some(self.content[range].to_string())
  }

  fn selected_text_range(
    &mut self,
    _ignore_disabled_input: bool,
    _window: &mut Window,
    _cx: &mut Context<Self>,
  ) -> Option<UTF16Selection> {
    Some(UTF16Selection {
      range: self.range_to_utf16(&self.selected_range),
      reversed: self.selection_reversed,
    })
  }

  fn marked_text_range(
    &self,
    _window: &mut Window,
    _cx: &mut Context<Self>,
  ) -> Option<Range<usize>> {
    self
      .marked_range
      .as_ref()
      .map(|range| self.range_to_utf16(range))
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
    self.reset_target_x();
    let range = range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16))
      .or(self.marked_range.clone())
      .unwrap_or(self.selected_range.clone());

    let sanitized = Self::sanitize_text(new_text);
    self.content = (self.content[0..range.start].to_owned()
      + sanitized.as_str()
      + &self.content[range.end..])
      .into();
    let cursor = range.start + sanitized.len();
    self.selected_range = cursor..cursor;
    self.marked_range.take();
    if let Some(bounds) = self.last_bounds {
      let (lines, desired_height, line_height) = compute_layouts(self, window, bounds);
      self.last_layouts = lines;
      self.last_line_height = line_height;
      self.desired_height = desired_height;
    }
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
    self.reset_target_x();
    let range = range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16))
      .or(self.marked_range.clone())
      .unwrap_or(self.selected_range.clone());

    let sanitized = Self::sanitize_text(new_text);
    self.content = (self.content[0..range.start].to_owned()
      + sanitized.as_str()
      + &self.content[range.end..])
      .into();
    if !sanitized.is_empty() {
      self.marked_range = Some(range.start..range.start + sanitized.len());
    } else {
      self.marked_range = None;
    }
    self.selected_range = new_selected_range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16))
      .map(|new_range| new_range.start + range.start..new_range.end + range.end)
      .unwrap_or_else(|| range.start + sanitized.len()..range.start + sanitized.len());

    if let Some(bounds) = self.last_bounds {
      let (lines, desired_height, line_height) = compute_layouts(self, window, bounds);
      self.last_layouts = lines;
      self.last_line_height = line_height;
      self.desired_height = desired_height;
    }
    cx.notify();
  }

  fn bounds_for_range(
    &mut self,
    range_utf16: Range<usize>,
    bounds: Bounds<Pixels>,
    _window: &mut Window,
    _cx: &mut Context<Self>,
  ) -> Option<Bounds<Pixels>> {
    let line_height = self.last_line_height;
    if self.last_layouts.is_empty() || line_height == px(0.0) {
      return None;
    }
    let range = self.range_from_utf16(&range_utf16);
    let start = position_for_index_in_lines(&self.last_layouts, range.start, line_height)?;
    let end = position_for_index_in_lines(&self.last_layouts, range.end, line_height)?;
    let left = bounds.left() + start.x.min(end.x);
    let right = bounds.left() + start.x.max(end.x);
    let top = bounds.top() + start.y.min(end.y);
    let bottom = bounds.top() + start.y.max(end.y) + line_height;
    Some(Bounds::from_corners(point(left, top), point(right, bottom)))
  }

  fn character_index_for_point(
    &mut self,
    point: gpui::Point<Pixels>,
    _window: &mut Window,
    _cx: &mut Context<Self>,
  ) -> Option<usize> {
    let local = self.last_bounds?.localize(&point)?;
    let line_height = self.last_line_height;
    if line_height == px(0.0) {
      return Some(0);
    }
    let utf8_index = self
      .index_for_position(local, line_height)
      .unwrap_or(self.content.len());
    Some(self.offset_to_utf16(utf8_index))
  }
}

struct TextElement {
  input: Entity<TextInput>,
}

struct PrepaintState {
  lines: Vec<LineLayoutInfo>,
  cursor: Option<PaintQuad>,
  selection: Vec<PaintQuad>,
  desired_height: Pixels,
  line_height: Pixels,
}

fn compute_layouts(
  input: &TextInput,
  window: &mut Window,
  bounds: Bounds<Pixels>,
) -> (Vec<LineLayoutInfo>, Pixels, Pixels) {
  let content = input.content.clone();
  let style = window.text_style();
  let (display_text, text_color) = if content.is_empty() {
    (input.placeholder.clone(), input.placeholder_color)
  } else {
    (content, style.color)
  };

  let font_size = style.font_size.to_pixels(window.rem_size());
  let line_height = window.line_height();
  let wrap_width = bounds.size.width.max(px(0.0));

  let mut lines = Vec::new();
  let mut y_offset = px(0.0);
  for (start, line_text) in split_lines(&display_text) {
    let runs = if line_text.is_empty() {
      Vec::new()
    } else if let Some(marked_range) = input.marked_range.as_ref()
      && marked_range.start >= start
      && marked_range.end <= start + line_text.len()
    {
      let line_marked_start = marked_range.start - start;
      let line_marked_end = marked_range.end - start;
      let base_run = TextRun {
        len: line_text.len(),
        font: style.font(),
        color: text_color,
        background_color: None,
        underline: None,
        strikethrough: None,
      };
      vec![
        TextRun {
          len: line_marked_start,
          ..base_run.clone()
        },
        TextRun {
          len: line_marked_end - line_marked_start,
          underline: Some(UnderlineStyle {
            color: Some(base_run.color),
            thickness: px(1.0),
            wavy: false,
          }),
          ..base_run.clone()
        },
        TextRun {
          len: line_text.len() - line_marked_end,
          ..base_run
        },
      ]
      .into_iter()
      .filter(|run| run.len > 0)
      .collect()
    } else {
      vec![TextRun {
        len: line_text.len(),
        font: style.font(),
        color: text_color,
        background_color: None,
        underline: None,
        strikethrough: None,
      }]
    };

    let shaped = window
      .text_system()
      .shape_text(line_text.to_string().into(), font_size, &runs, Some(wrap_width), None)
      .ok()
      .and_then(|mut lines| lines.pop())
      .unwrap_or_default();
    let wrap_count = shaped.wrap_boundaries().len() + 1;
    let height = line_height * (wrap_count as f32);
    lines.push(LineLayoutInfo {
      start,
      len: line_text.len(),
      layout: shaped,
      y: y_offset,
      height,
    });
    y_offset += height;
  }
  let desired_height = y_offset.max(line_height);
  (lines, desired_height, line_height)
}

fn split_lines(text: &str) -> Vec<(usize, &str)> {
  let mut lines = Vec::new();
  let mut start = 0;
  for (idx, ch) in text.char_indices() {
    if ch == '\n' {
      lines.push((start, &text[start..idx]));
      start = idx + ch.len_utf8();
    }
  }
  lines.push((start, &text[start..]));
  lines
}

fn wrap_indices(layout: &WrappedLine) -> Vec<usize> {
  let mut indices = Vec::new();
  let runs = layout.runs();
  for boundary in layout.wrap_boundaries() {
    if let Some(run) = runs.get(boundary.run_ix)
      && let Some(glyph) = run.glyphs.get(boundary.glyph_ix)
    {
      indices.push(glyph.index);
    }
  }
  indices
}

fn position_for_index_in_lines(
  lines: &[LineLayoutInfo],
  index: usize,
  line_height: Pixels,
) -> Option<Point<Pixels>> {
  for line in lines {
    let line_end = line.start + line.len;
    if index <= line_end {
      let rel = index.saturating_sub(line.start);
      if let Some(pos) = line.layout.position_for_index(rel, line_height) {
        return Some(point(pos.x, line.y + pos.y));
      }
      return Some(point(px(0.0), line.y));
    }
  }
  let last = lines.last()?;
  last
    .layout
    .position_for_index(last.len, line_height)
    .map(|pos| point(pos.x, last.y + pos.y))
    .or(Some(point(px(0.0), last.y)))
}

fn selection_quads_for_range(
  lines: &[LineLayoutInfo],
  selection: Range<usize>,
  bounds: Bounds<Pixels>,
  line_height: Pixels,
  selection_color: gpui::Hsla,
) -> Vec<PaintQuad> {
  let mut quads = Vec::new();
  let sel_start = selection.start.min(selection.end);
  let sel_end = selection.end.max(selection.start);
  if sel_start == sel_end {
    return quads;
  }

  for line in lines {
    let line_start = line.start;
    let line_end = line.start + line.len;
    let overlap_start = sel_start.max(line_start);
    let overlap_end = sel_end.min(line_end);
    if overlap_start >= overlap_end {
      continue;
    }

    let rel_start = overlap_start - line_start;
    let rel_end = overlap_end - line_start;
    let mut boundaries = wrap_indices(&line.layout);
    let mut segment_starts = Vec::with_capacity(boundaries.len() + 1);
    segment_starts.push(0);
    segment_starts.extend(boundaries.iter().copied());
    boundaries.push(line.len);

    for (wrap_line_index, (seg_start, seg_end)) in
      segment_starts
        .into_iter()
        .zip(boundaries.into_iter())
        .enumerate()
    {
      let seg_sel_start = rel_start.max(seg_start);
      let seg_sel_end = rel_end.min(seg_end);
      if seg_sel_start >= seg_sel_end {
        continue;
      }
      let wrap_start_x = line.layout.unwrapped_layout.x_for_index(seg_start);
      let start_x = line.layout.unwrapped_layout.x_for_index(seg_sel_start) - wrap_start_x;
      let end_x = line.layout.unwrapped_layout.x_for_index(seg_sel_end) - wrap_start_x;
      let left = bounds.left() + start_x.min(end_x);
      let right = bounds.left() + start_x.max(end_x);
      let top = bounds.top() + line.y + line_height * (wrap_line_index as f32);
      let bottom = top + line_height;
      quads.push(fill(
        Bounds::from_corners(point(left, top), point(right, bottom)),
        selection_color,
      ));
    }
  }

  quads
}

impl IntoElement for TextElement {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for TextElement {
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
    _inspector_id: Option<&gpui::InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    let mut style = Style::default();
    let desired_height = self.input.read(cx).desired_height;
    let height = if desired_height > px(0.0) {
      desired_height
    } else {
      window.line_height()
    };
    style.size.width = relative(1.).into();
    style.size.height = height.into();
    (window.request_layout(style, [], cx), ())
  }

  fn prepaint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&gpui::InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    window: &mut Window,
    cx: &mut App,
  ) -> Self::PrepaintState {
    let input = self.input.read(cx);
    let selected_range = input.selected_range.clone();
    let (lines, desired_height, line_height) = compute_layouts(&input, window, bounds);

    let selection_quads = if selected_range.is_empty() {
      Vec::new()
    } else {
      selection_quads_for_range(
        &lines,
        selected_range.clone(),
        bounds,
        line_height,
        input.selection_color,
      )
    };

    let caret_index = input.caret_index();
    let caret_pos =
      position_for_index_in_lines(&lines, caret_index, line_height)
        .unwrap_or(point(px(0.0), px(0.0)));
    let cursor = Some(fill(
      Bounds::new(
        point(bounds.left() + caret_pos.x, bounds.top() + caret_pos.y),
        size(px(2.0), line_height),
      ),
      input.cursor_color,
    ));
    PrepaintState {
      lines,
      cursor,
      selection: selection_quads,
      desired_height,
      line_height,
    }
  }

  fn paint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&gpui::InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    prepaint: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
    let focus_handle = self.input.read(cx).focus_handle.clone();
    window.handle_input(
      &focus_handle,
      ElementInputHandler::new(bounds, self.input.clone()),
      cx,
    );
    for quad in prepaint.selection.drain(..) {
      window.paint_quad(quad);
    }

    let lines = std::mem::take(&mut prepaint.lines);
    for line in &lines {
      line
        .layout
        .paint(
          point(bounds.left(), bounds.top() + line.y),
          prepaint.line_height,
          gpui::TextAlign::Left,
          None,
          window,
          cx,
        )
        .unwrap();
    }

    if focus_handle.is_focused(window)
      && let Some(cursor) = prepaint.cursor.take()
    {
      window.paint_quad(cursor);
    }

    let desired_height = prepaint.desired_height;
    let line_height = prepaint.line_height;
    self.input.update(cx, |input, cx| {
      input.last_layouts = lines;
      input.last_bounds = Some(bounds);
      input.last_line_height = line_height;
      if input.desired_height != desired_height {
        input.desired_height = desired_height;
        cx.notify();
      }
    });
  }
}

impl Render for TextInput {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    div()
      .flex()
      .items_start()
      .w_full()
      .key_context("TextInput")
      .track_focus(&self.focus_handle(cx))
      .cursor(CursorStyle::IBeam)
      .on_action(cx.listener(Self::backspace_word))
      .on_action(cx.listener(Self::backspace_all))
      .on_action(cx.listener(Self::backspace))
      .on_action(cx.listener(Self::delete))
      .on_action(cx.listener(Self::up))
      .on_action(cx.listener(Self::down))
      .on_action(cx.listener(Self::left))
      .on_action(cx.listener(Self::right))
      .on_action(cx.listener(Self::alt_left))
      .on_action(cx.listener(Self::alt_right))
      .on_action(cx.listener(Self::cmd_left))
      .on_action(cx.listener(Self::cmd_right))
      .on_action(cx.listener(Self::cmd_up))
      .on_action(cx.listener(Self::cmd_down))
      .on_action(cx.listener(Self::select_left))
      .on_action(cx.listener(Self::select_right))
      .on_action(cx.listener(Self::select_up))
      .on_action(cx.listener(Self::select_down))
      .on_action(cx.listener(Self::select_word_left))
      .on_action(cx.listener(Self::select_word_right))
      .on_action(cx.listener(Self::select_cmd_left))
      .on_action(cx.listener(Self::select_cmd_right))
      .on_action(cx.listener(Self::select_cmd_up))
      .on_action(cx.listener(Self::select_cmd_down))
      .on_action(cx.listener(Self::select_all))
      .on_action(cx.listener(Self::home))
      .on_action(cx.listener(Self::end))
      .on_action(cx.listener(Self::show_character_palette))
      .on_action(cx.listener(Self::paste))
      .on_action(cx.listener(Self::cut))
      .on_action(cx.listener(Self::copy))
      .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
      .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
      .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
      .on_mouse_move(cx.listener(Self::on_mouse_move))
      .child(TextElement { input: cx.entity() })
  }
}

impl Focusable for TextInput {
  fn focus_handle(&self, _: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}
