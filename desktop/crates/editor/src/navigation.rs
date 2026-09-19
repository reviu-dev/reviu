use super::*;
use crate::editor_element::{expand_tab_stops, highlights_to_text_runs};
use gpui::{TextRun, TextStyle};

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;

#[derive(Clone, Copy)]
pub(crate) enum HorizontalMotion {
  Character(i32),
  Word(i32),
  LineBoundary(bool),
}

impl Editor {
  pub(super) fn next_selectable_display_line(
    &self,
    start: usize,
    direction: i32,
    cx: &App,
  ) -> Option<usize> {
    let total = self.display_line_count(self.document.read(cx).len_lines());
    if direction < 0 {
      (0..start).rev().find(|line| {
        self
          .selection_line_text(*line, self.selection_view, cx)
          .is_some()
      })
    } else {
      (start.saturating_add(1)..total).find(|line| {
        self
          .selection_line_text(*line, self.selection_view, cx)
          .is_some()
      })
    }
  }

  pub(super) fn navigation_layout(
    &self,
    line: usize,
    window: &Window,
    cx: &App,
  ) -> Option<ShapedLine> {
    let text = self.selection_line_text(line, self.selection_view, cx)?;
    let style = TextStyle {
      font_family: cx.theme().mono_font_family.clone(),
      font_size: cx.theme().mono_font_size.into(),
      ..TextStyle::default()
    };
    let doc_line = self.navigation_doc_line(line, cx);
    let runs = doc_line
      .and_then(|line| self.document.read(cx).get_highlights_for_line(line))
      .map(|highlights| highlights_to_text_runs(&highlights, &text, &self.theme, &style))
      .unwrap_or_else(|| {
        vec![TextRun {
          len: text.len(),
          font: style.font(),
          color: style.color,
          background_color: None,
          underline: None,
          strikethrough: None,
        }]
      });
    let shaped = window.text_system().shape_line(
      text.replace('\t', " ").into(),
      style.font_size.to_pixels(window.rem_size()),
      &runs,
      None,
    );
    Some(expand_tab_stops(
      shaped,
      &text,
      self.measured_editor_char_width() * self.document.read(cx).indentation.width as f32,
    ))
  }

  fn navigation_doc_line(&self, line: usize, cx: &App) -> Option<usize> {
    match self.display_line(line, self.document.read(cx).len_lines())? {
      DisplayLine::Doc { doc_line, .. } => Some(doc_line),
      DisplayLine::Modified { doc_line, .. }
        if self.selection_view != DiffElementView::SplitLeft =>
      {
        Some(doc_line)
      }
      _ => None,
    }
  }

  fn apply_navigation(&mut self, cursor: DisplayCursor, selecting: bool, cx: &mut Context<Self>) {
    if selecting {
      let anchor = self.current_display_anchor(cx).unwrap_or(cursor);
      self.set_display_selection_with_anchor(anchor, cursor, cx);
    } else {
      self.set_display_cursor(cursor, cx);
    }
  }

  pub(crate) fn navigate_vertical(
    &mut self,
    direction: i32,
    selecting: bool,
    page: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.sync_soft_wrap(window, cx);
    if self.soft_wrap.enabled {
      self.navigate_wrapped_vertical(direction, selecting, page, window, cx);
      return;
    }
    let Some(cursor) = self.current_display_cursor(cx) else {
      return;
    };
    let Some(layout) = self.navigation_layout(cursor.line, window, cx) else {
      return;
    };
    let goal = self.vertical_goal_x.unwrap_or_else(|| {
      layout.x_for_index(char_offset_to_byte_offset(&layout.text, cursor.column))
    });
    let total = self.display_line_count(self.document.read(cx).len_lines());
    let rows = if page {
      let metrics = self.vertical_scroll_metrics(self.measured_editor_line_height(), total);
      (metrics.viewport_lines.floor() as usize)
        .saturating_sub(1)
        .max(1)
    } else {
      1
    };
    let desired = if direction < 0 {
      cursor.line.saturating_sub(rows)
    } else {
      cursor
        .line
        .saturating_add(rows)
        .min(total.saturating_sub(1))
    };
    let target_line = if self
      .selection_line_text(desired, self.selection_view, cx)
      .is_some()
    {
      desired
    } else {
      self
        .next_selectable_display_line(desired, direction, cx)
        .or_else(|| self.next_selectable_display_line(desired, -direction, cx))
        .unwrap_or(cursor.line)
    };
    let Some(target_layout) = self.navigation_layout(target_line, window, cx) else {
      return;
    };
    let column = if target_line == cursor.line {
      if direction < 0 {
        0
      } else {
        target_layout.text.chars().count()
      }
    } else {
      let byte = target_layout.closest_index_for_x(goal);
      let byte = target_layout
        .text
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .chain(std::iter::once(target_layout.text.len()))
        .min_by(|left, right| {
          (target_layout.x_for_index(*left) - goal)
            .abs()
            .partial_cmp(&(target_layout.x_for_index(*right) - goal).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(byte);
      byte_offset_to_char_offset(&target_layout.text, byte)
    };
    if page {
      self.scroll_offset_y += target_line as f32 - cursor.line as f32;
    }
    self.apply_navigation(
      DisplayCursor {
        line: target_line,
        column,
      },
      selecting,
      cx,
    );
    self.vertical_goal_x = Some(goal);
    self.ensure_cursor_visible(window, cx);
    cx.notify();
  }

  fn navigate_wrapped_vertical(
    &mut self,
    direction: i32,
    selecting: bool,
    page: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(cursor) = self.current_display_cursor(cx) else {
      return;
    };
    let Some(layout) = self.navigation_layout(cursor.line, window, cx) else {
      return;
    };
    let map = self.soft_wrap.map.clone();
    let row = self.soft_wrap.cursor_row(cursor, self.selection_view);
    let boundary = map.boundary(row, self.selection_view);
    let goal = self.vertical_goal_x.unwrap_or_else(|| {
      layout.x_for_index(char_offset_to_byte_offset(&layout.text, cursor.column))
        - layout.x_for_index(boundary.byte)
    });
    let total = map.count(self.display_line_count(self.document.read(cx).len_lines()));
    let distance = if page {
      ((self.viewport_height / self.measured_editor_line_height()).floor() as usize)
        .saturating_sub(1)
        .max(1)
    } else {
      1
    };
    let mut target = if direction < 0 {
      row.saturating_sub(distance)
    } else {
      row.saturating_add(distance).min(total.saturating_sub(1))
    };
    loop {
      let line = map.line(target);
      let fragment = target.saturating_sub(map.row(line));
      if fragment < map.boundaries(line, self.selection_view).len()
        && self
          .selection_line_text(line, self.selection_view, cx)
          .is_some()
      {
        break;
      }
      let next = if direction < 0 {
        target.saturating_sub(1)
      } else {
        target.saturating_add(1).min(total.saturating_sub(1))
      };
      if next == target {
        target = row;
        break;
      }
      target = next;
    }
    let line = map.line(target);
    let Some(layout) = self.navigation_layout(line, window, cx) else {
      return;
    };
    let fragment = target.saturating_sub(map.row(line));
    let Some(bytes) = map.fragment_bytes(line, fragment, self.selection_view, layout.text.len())
    else {
      return;
    };
    let x = layout.x_for_index(bytes.start) + goal;
    let byte = layout
      .text
      .get(bytes.clone())
      .unwrap_or_default()
      .grapheme_indices(true)
      .map(|(byte, _)| bytes.start + byte)
      .chain(std::iter::once(bytes.end))
      .min_by(|left, right| {
        (layout.x_for_index(*left) - x)
          .abs()
          .partial_cmp(&(layout.x_for_index(*right) - x).abs())
          .unwrap_or(std::cmp::Ordering::Equal)
      })
      .unwrap_or(bytes.start);
    if page {
      self.scroll_offset_y += target as f32 - row as f32;
    }
    let target_cursor = DisplayCursor {
      line,
      column: if target == row {
        if direction < 0 {
          0
        } else {
          layout.text.chars().count()
        }
      } else {
        map.boundary(target, self.selection_view).column
          + layout
            .text
            .get(bytes.start..byte)
            .unwrap_or_default()
            .chars()
            .count()
      },
    };
    self.apply_navigation(target_cursor, selecting, cx);
    self.soft_wrap.upstream_cursor =
      (map.cursor_row(line, target_cursor.column, self.selection_view) > target)
        .then_some(target_cursor);
    self.vertical_goal_x = Some(goal);
    self.ensure_cursor_visible(window, cx);
    cx.notify();
  }

  pub(crate) fn navigate_document_boundary(
    &mut self,
    start: bool,
    selecting: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.vertical_goal_x = None;
    let total = self.display_line_count(self.document.read(cx).len_lines());
    let line = if start {
      (0..total).find(|line| {
        self
          .selection_line_text(*line, self.selection_view, cx)
          .is_some()
      })
    } else {
      (0..total).rev().find(|line| {
        self
          .selection_line_text(*line, self.selection_view, cx)
          .is_some()
      })
    };
    if let Some(line) = line {
      let column = if start {
        0
      } else {
        self.display_line_len(line, cx)
      };
      self.apply_navigation(DisplayCursor { line, column }, selecting, cx);
      self.ensure_cursor_visible(window, cx);
    }
  }

  pub(crate) fn navigate_old_side(
    &mut self,
    motion: HorizontalMotion,
    selecting: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> bool {
    if self.selection_view != DiffElementView::SplitLeft {
      return false;
    }
    self.vertical_goal_x = None;
    let Some(mut cursor) = self.current_display_cursor(cx) else {
      return true;
    };
    if !selecting
      && let Some(selection) = &self.display_selection
      && selection.start != selection.end
    {
      let start = matches!(
        motion,
        HorizontalMotion::Character(-1)
          | HorizontalMotion::Word(-1)
          | HorizontalMotion::LineBoundary(true)
      );
      cursor = if start {
        selection.start.min(selection.end)
      } else {
        selection.start.max(selection.end)
      };
      self.apply_navigation(cursor, false, cx);
      self.ensure_cursor_visible(window, cx);
      return true;
    }
    let text = self
      .selection_line_text(cursor.line, self.selection_view, cx)
      .unwrap_or_default();
    cursor.column = cursor.column.min(text.chars().count());
    match motion {
      HorizontalMotion::LineBoundary(start) => {
        cursor.column = if start { 0 } else { text.chars().count() }
      }
      HorizontalMotion::Word(direction) => {
        let column = if direction < 0 {
          Self::previous_word_boundary_in_line(&text, cursor.column)
        } else {
          Self::next_word_boundary_in_line(&text, cursor.column)
        };
        if column == cursor.column {
          if let Some(line) = self.next_selectable_display_line(cursor.line, direction, cx) {
            cursor = DisplayCursor {
              line,
              column: if direction < 0 {
                self.display_line_len(line, cx)
              } else {
                0
              },
            };
          }
        } else {
          cursor.column = column;
        }
      }
      HorizontalMotion::Character(direction) => {
        let byte = char_offset_to_byte_offset(&text, cursor.column);
        let boundary = if direction < 0 {
          text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .take_while(|index| *index < byte)
            .last()
        } else {
          text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .find(|index| *index > byte)
        };
        if let Some(byte) = boundary {
          cursor.column = byte_offset_to_char_offset(&text, byte);
        } else if let Some(line) = self.next_selectable_display_line(cursor.line, direction, cx) {
          cursor = DisplayCursor {
            line,
            column: if direction < 0 {
              self.display_line_len(line, cx)
            } else {
              0
            },
          };
        }
      }
    }
    self.apply_navigation(cursor, selecting, cx);
    self.ensure_cursor_visible(window, cx);
    true
  }
}
