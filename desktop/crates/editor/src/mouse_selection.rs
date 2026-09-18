use super::*;
use crate::editor_element::{LineVisibility, display_line_text_for_view, line_visibility_for_view};
use gpui::{MouseDownEvent, MouseUpEvent};

#[cfg(test)]
#[path = "mouse_selection_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionMode {
  Character,
  Word,
  Line,
  All,
}

#[derive(Clone, Debug)]
pub(super) struct MouseSelection {
  mode: SelectionMode,
  original: Range<DisplayCursor>,
}

impl Editor {
  pub(crate) fn selection_is_read_only(&self) -> bool {
    self.is_read_only || self.selection_view == DiffElementView::SplitLeft
  }

  pub(super) fn selection_line_text(
    &self,
    line: usize,
    view: DiffElementView,
    cx: &App,
  ) -> Option<String> {
    let document = self.document.read(cx);
    let display_line = self.display_line(line, document.len_lines())?;
    if !matches!(
      display_line,
      DisplayLine::Doc { .. } | DisplayLine::Modified { .. } | DisplayLine::Removed { .. }
    ) || line_visibility_for_view(
      view,
      &display_line,
      self.block_map.block_at_display_line(line),
    ) == LineVisibility::Blank
    {
      return None;
    }
    Some(display_line_text_for_view(&display_line, view, document))
  }

  fn mouse_selection_range(
    &self,
    cursor: DisplayCursor,
    mode: SelectionMode,
    cx: &App,
  ) -> Range<DisplayCursor> {
    let text = self
      .selection_line_text(cursor.line, self.selection_view, cx)
      .unwrap_or_default();
    match mode {
      SelectionMode::Character => cursor..cursor,
      SelectionMode::Word => {
        let (start, end) = mouse_word_range(&text, cursor.column);
        DisplayCursor {
          line: cursor.line,
          column: start,
        }..DisplayCursor {
          line: cursor.line,
          column: end,
        }
      }
      SelectionMode::Line => {
        let total = self.display_line_count(self.document.read(cx).len_lines());
        let end = ((cursor.line + 1)..total)
          .find(|line| {
            self
              .selection_line_text(*line, self.selection_view, cx)
              .is_some()
          })
          .map(|line| DisplayCursor { line, column: 0 })
          .unwrap_or(DisplayCursor {
            line: cursor.line,
            column: text.chars().count(),
          });
        DisplayCursor {
          line: cursor.line,
          column: 0,
        }..end
      }
      SelectionMode::All => {
        let total = self.display_line_count(self.document.read(cx).len_lines());
        let first = (0..total)
          .find(|line| {
            self
              .selection_line_text(*line, self.selection_view, cx)
              .is_some()
          })
          .unwrap_or(0);
        let last = (first..total)
          .rev()
          .find(|line| {
            self
              .selection_line_text(*line, self.selection_view, cx)
              .is_some()
          })
          .unwrap_or(first);
        let column = self
          .selection_line_text(last, self.selection_view, cx)
          .map(|text| text.chars().count())
          .unwrap_or(0);
        DisplayCursor {
          line: first,
          column: 0,
        }..DisplayCursor { line: last, column }
      }
    }
  }

  pub(crate) fn begin_mouse_selection(
    &mut self,
    cursor: DisplayCursor,
    view: DiffElementView,
    event: &MouseDownEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selection_line_text(cursor.line, view, cx).is_none() {
      return;
    }
    let extending = event.modifiers.shift && self.selection_view == view;
    let anchor = self.current_display_anchor(cx).unwrap_or(cursor);
    let previous = self
      .mouse_selection
      .clone()
      .filter(|_| self.display_selection.is_some());
    self.selection_view = view;
    self.target_column = None;
    self.is_selecting = true;
    self.selection_autoscroll_task = None;
    self.last_mouse_position = Some(event.position);
    window.focus(&self.focus_handle, cx);

    let mut mode = match event.click_count {
      0 | 1 => SelectionMode::Character,
      2 => SelectionMode::Word,
      3 => SelectionMode::Line,
      _ => SelectionMode::All,
    };
    if extending
      && event.click_count == 1
      && let Some(previous) = &previous
    {
      mode = previous.mode;
    }
    let original = if extending {
      if let Some(previous) = previous.filter(|previous| previous.mode == mode) {
        previous.original
      } else {
        anchor..anchor
      }
    } else {
      self.mouse_selection_range(cursor, mode, cx)
    };
    self.mouse_selection = Some(MouseSelection { mode, original });
    self.update_mouse_selection(cursor, cx);
    self
      .cursor_blink
      .update(cx, |blink, cx| blink.pause_blinking(cx));
    cx.notify();
  }

  pub fn mouse_left_down(
    &mut self,
    event: &MouseDownEvent,
    position_map: &PositionMap,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !position_map.viewport_bounds.contains(&event.position) {
      return;
    }
    if let Some(cursor) = position_map.display_cursor_for_position(event.position) {
      self.begin_mouse_selection(cursor, position_map.view, event, window, cx);
    }
  }

  fn update_mouse_selection(&mut self, mut cursor: DisplayCursor, cx: &mut Context<Self>) {
    let Some(selection) = self.mouse_selection.clone() else {
      return;
    };
    let total = self.display_line_count(self.document.read(cx).len_lines());
    cursor.line = cursor.line.min(total.saturating_sub(1));
    let forward = cursor >= selection.original.start;
    if self
      .selection_line_text(cursor.line, self.selection_view, cx)
      .is_none()
    {
      let next = (cursor.line..total).find(|line| {
        self
          .selection_line_text(*line, self.selection_view, cx)
          .is_some()
      });
      let previous = (0..cursor.line).rev().find(|line| {
        self
          .selection_line_text(*line, self.selection_view, cx)
          .is_some()
      });
      let Some(line) = (if forward {
        next.or(previous)
      } else {
        previous.or(next)
      }) else {
        return;
      };
      cursor = DisplayCursor {
        line,
        column: if line < cursor.line {
          self
            .selection_line_text(line, self.selection_view, cx)
            .map(|text| text.chars().count())
            .unwrap_or(0)
        } else {
          0
        },
      };
    }
    let text = self
      .selection_line_text(cursor.line, self.selection_view, cx)
      .unwrap_or_default();
    cursor.column = cursor.column.min(text.chars().count());
    let range =
      if selection.mode == SelectionMode::Word && !selection.original.contains(&cursor) && {
        let (start, end) = Self::word_range_in_line(&text, cursor.column);
        start == end
      } {
        cursor..cursor
      } else {
        self.mouse_selection_range(cursor, selection.mode, cx)
      };
    let (anchor, head) = match selection.mode {
      SelectionMode::Character => (selection.original.start, cursor),
      SelectionMode::All => (selection.original.start, selection.original.end),
      SelectionMode::Word | SelectionMode::Line => {
        if cursor < selection.original.start {
          (selection.original.end, range.start)
        } else {
          (
            selection.original.start,
            range.end.max(selection.original.end),
          )
        }
      }
    };
    if self
      .display_selection
      .as_ref()
      .is_some_and(|selection| selection.start == anchor && selection.end == head)
    {
      return;
    }
    self.set_display_selection_with_anchor(anchor, head, cx);
  }

  pub(crate) fn refresh_mouse_selection(&mut self, map: &PositionMap, cx: &mut Context<Self>) {
    if self.is_selecting
      && map.view == self.selection_view
      && let Some(position) = self.last_mouse_position
      && let Some(cursor) = map.display_cursor_for_position(position)
    {
      self.update_mouse_selection(cursor, cx);
    }
  }

  pub fn mouse_left_up(
    &mut self,
    event: &MouseUpEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.is_selecting
      && self.focus_handle.is_focused(window)
      && self.last_mouse_position != Some(event.position)
      && let Some(map) = self.selection_position_map.clone()
      && map.view == self.selection_view
      && let Some(cursor) = map.display_cursor_for_position(event.position)
    {
      self.update_mouse_selection(cursor, cx);
    }
    self.is_selecting = false;
    self.selection_autoscroll_task = None;
  }

  pub fn mouse_dragged(
    &mut self,
    event: &MouseMoveEvent,
    position_map: &PositionMap,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !self.is_selecting || self.selection_view != position_map.view {
      return;
    }
    if event.pressed_button != Some(MouseButton::Left) || !self.focus_handle.is_focused(window) {
      self.is_selecting = false;
      self.selection_autoscroll_task = None;
      return;
    }
    self.last_mouse_position = Some(event.position);
    if let Some(cursor) = position_map.display_cursor_for_position(event.position) {
      self.update_mouse_selection(cursor, cx);
    }
    if self
      .mouse_selection
      .as_ref()
      .is_some_and(|selection| selection.mode == SelectionMode::All)
    {
      return;
    }
    if self.selection_autoscroll_task.is_none() {
      self.selection_autoscroll_task = Some(cx.spawn_in(window, async move |editor, cx| {
        loop {
          cx.background_executor()
            .timer(Duration::from_millis(16))
            .await;
          let keep_scrolling = editor.update_in(cx, |editor, window, cx| {
            if !editor.is_selecting || !editor.focus_handle.is_focused(window) {
              editor.is_selecting = false;
              editor.selection_autoscroll_task = None;
              return false;
            }
            let Some(map) = editor
              .selection_position_map
              .clone()
              .filter(|map| map.view == editor.selection_view)
            else {
              editor.selection_autoscroll_task = None;
              return false;
            };
            let position = window.mouse_position();
            if let Some(cursor) = map.display_cursor_for_position(position) {
              editor.update_mouse_selection(cursor, cx);
            }
            let delta = selection_scroll_delta(
              position,
              map.viewport_bounds,
              map.line_height,
              editor.measured_editor_char_width(),
            );
            let total = editor.display_line_count(editor.document.read(cx).len_lines());
            let vertical = Self::clamp_vertical_scroll_for_height(
              editor.scroll_offset_y + delta.y,
              map.viewport_bounds.size.height,
              map.line_height,
              total,
            );
            let horizontal =
              editor.clamp_horizontal_scroll_x(editor.scroll_handle.offset().x - px(delta.x));
            let changed =
              vertical != editor.scroll_offset_y || horizontal != editor.scroll_handle.offset().x;
            if changed {
              editor.scroll_offset_y = vertical;
              editor.set_horizontal_scroll_offset(horizontal);
              cx.notify();
            } else {
              editor.selection_autoscroll_task = None;
            }
            changed
          });
          if !matches!(keep_scrolling, Ok(true)) {
            break;
          }
        }
      }));
    }
  }

  pub(crate) fn select_all_display_lines(&mut self, cx: &mut Context<Self>) -> bool {
    let range =
      self.mouse_selection_range(DisplayCursor { line: 0, column: 0 }, SelectionMode::All, cx);
    self.mouse_selection = None;
    self.set_display_selection_with_anchor(range.start, range.end, cx);
    self.selected_range = 0..self.document.read(cx).len();
    true
  }
}

fn mouse_word_range(text: &str, column: usize) -> (usize, usize) {
  let length = text.chars().count();
  if length == 0 {
    return (0, 0);
  }
  let column = column.min(length.saturating_sub(1));
  let (start, end) = Editor::word_range_in_line(text, column);
  if start != end {
    return (start, end);
  }
  let characters: Vec<_> = text.chars().collect();
  let mut start = column;
  let mut end = column;
  while start > 0
    && characters
      .get(start - 1)
      .is_some_and(|character| character.is_whitespace())
  {
    start -= 1;
  }
  while characters
    .get(end)
    .is_some_and(|character| character.is_whitespace())
  {
    end += 1;
  }
  (start, end)
}

fn selection_scroll_delta(
  position: Point<Pixels>,
  bounds: Bounds<Pixels>,
  line_height: Pixels,
  character_width: Pixels,
) -> Point<f32> {
  let vertical_margin = line_height.min(bounds.size.height / 3.0);
  let vertical = if position.y < bounds.top() + vertical_margin {
    -((bounds.top() + vertical_margin - position.y) / px(1.0))
      .powf(1.2)
      .min(300.0)
      / 100.0
  } else if position.y > bounds.bottom() - vertical_margin {
    ((position.y - bounds.bottom() + vertical_margin) / px(1.0))
      .powf(1.2)
      .min(300.0)
      / 100.0
  } else {
    0.0
  };
  let horizontal_margin = (character_width * 3.0).min(bounds.size.width / 3.0);
  let left = bounds.left() + horizontal_margin;
  let right = bounds.right() - horizontal_margin;
  let horizontal = if position.x < left {
    -((left - position.x) / px(1.0)).powf(1.2) / 300.0
  } else if position.x > right {
    ((position.x - right) / px(1.0)).powf(1.2) / 300.0
  } else {
    0.0
  };
  point(horizontal * (character_width / px(1.0)), vertical)
}
