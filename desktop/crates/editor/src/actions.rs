//! Editor actions for navigation, editing, and selection
//!
//! This module contains all the action handlers for the editor,
//! including text editing, cursor movement, and selection operations.

use gpui::{ClipboardItem, Context, EntityInputHandler, Window, actions};

use crate::{boundaries, editor::Editor};

actions!(
  editor,
  [
    Enter,
    Tab,
    Backspace,
    BackspaceWord,
    BackspaceAll,
    Delete,
    Up,
    Down,
    Left,
    AltLeft,
    CmdLeft,
    Right,
    CmdRight,
    AltRight,
    CmdUp,
    CmdDown,
    SelectUp,
    SelectDown,
    SelectLeft,
    SelectRight,
    SelectCmdLeft,
    SelectCmdRight,
    SelectCmdUp,
    SelectCmdDown,
    SelectWordLeft,
    SelectWordRight,
    SelectAll,
    Home,
    End,
    ShowCharacterPalette,
    Paste,
    Cut,
    Copy,
    Undo,
    Redo,
    Save,
    Find,
    CloseFind,
    Quit,
  ]
);

fn should_handle_backspace_in_display_space(editor: &Editor, cx: &Context<Editor>) -> bool {
  editor.selected_range.is_empty() && editor.is_read_only_display_cursor(cx)
}

pub fn enter(editor: &mut Editor, _: &Enter, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  editor.replace_text_in_range(None, "\n", window, cx);
}

pub fn tab(editor: &mut Editor, _: &Tab, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  let spaces = " ".repeat(crate::editor::TAB_SPACES);
  editor.replace_text_in_range(None, &spaces, window, cx);
}

pub fn backspace(
  editor: &mut Editor,
  _: &Backspace,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.target_column = None;
  if should_handle_backspace_in_display_space(editor, cx)
    && editor.move_display_cursor_horizontal(-1, cx)
  {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.selected_range.is_empty() {
    editor.select_to(
      boundaries::previous_boundary(editor, editor.cursor_offset(), cx),
      cx,
    )
  }
  editor.replace_text_in_range(None, "", window, cx)
}

pub fn backspace_word(
  editor: &mut Editor,
  _: &BackspaceWord,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.target_column = None;
  if editor.selected_range.is_empty() {
    let document = editor.document.read(cx);
    let cursor = editor.cursor_offset();
    let line = document.char_to_line(cursor);
    let line_start = document.line_to_char(line);

    if cursor == line_start && document.line_content(line).unwrap_or_default().is_empty() {
      editor.select_to(boundaries::previous_boundary(editor, cursor, cx), cx);
    } else {
      editor.select_to(boundaries::previous_word_boundary(editor, cursor, cx), cx);
    }
  }
  editor.replace_text_in_range(None, "", window, cx)
}

pub fn backspace_all(
  editor: &mut Editor,
  _: &BackspaceAll,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.target_column = None;
  if editor.selected_range.is_empty() {
    let document = editor.document.read(cx);
    let cursor = editor.cursor_offset();
    let line = document.char_to_line(cursor);
    let line_start = document.line_to_char(line);

    if cursor == line_start && document.line_content(line).unwrap_or_default().is_empty() {
      editor.select_to(boundaries::previous_boundary(editor, cursor, cx), cx);
    } else {
      editor.select_to(line_start, cx);
    }
  }
  editor.replace_text_in_range(None, "", window, cx)
}

pub fn delete(editor: &mut Editor, _: &Delete, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  if editor.selected_range.is_empty() {
    editor.select_to(
      boundaries::next_boundary(editor, editor.cursor_offset(), cx),
      cx,
    )
  }
  editor.replace_text_in_range(None, "", window, cx)
}

pub fn up(editor: &mut Editor, _: &Up, window: &mut Window, cx: &mut Context<Editor>) {
  if editor.move_display_cursor_vertical(-1, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  let new_cursor = {
    let document = editor.document.read(cx);
    let cursor_offset = editor.cursor_offset();
    let current_line = document.char_to_line(cursor_offset);

    if current_line > 0 {
      let target_column = *editor
        .target_column
        .get_or_insert_with(|| cursor_offset - document.line_to_char(current_line));

      let target_line = editor.previous_visible_doc_line(current_line).unwrap_or(0);
      let target_start = document.line_to_char(target_line);
      let target_len = document
        .line_content(target_line)
        .map(|line| line.chars().count())
        .unwrap_or(0);

      target_start + target_column.min(target_len)
    } else {
      editor.target_column = None;
      0
    }
  };

  editor.move_to(new_cursor, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn down(editor: &mut Editor, _: &Down, window: &mut Window, cx: &mut Context<Editor>) {
  if editor.move_display_cursor_vertical(1, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  let new_cursor = {
    let document = editor.document.read(cx);
    let cursor_offset = editor.cursor_offset();
    let current_line = document.char_to_line(cursor_offset);
    let doc_line_count = document.len_lines();

    if current_line < doc_line_count.saturating_sub(1) {
      let target_column = *editor
        .target_column
        .get_or_insert_with(|| cursor_offset - document.line_to_char(current_line));

      let target_line = editor
        .next_visible_doc_line(current_line, doc_line_count)
        .unwrap_or(current_line);
      let target_start = document.line_to_char(target_line);
      let target_len = document
        .line_content(target_line)
        .map(|line| line.chars().count())
        .unwrap_or(0);

      target_start + target_column.min(target_len)
    } else {
      editor.target_column = None;
      document.len()
    }
  };

  editor.move_to(new_cursor, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn left(editor: &mut Editor, _: &Left, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  if editor.collapse_removed_selection(true, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.selected_range.is_empty()
    && editor.move_display_cursor_prev_removed_line_end_from_boundary(cx)
  {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.selected_range.is_empty() && editor.move_display_cursor_prev_display_line_end(cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.selected_range.is_empty() && editor.move_display_cursor_horizontal(-1, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.selected_range.is_empty() {
    editor.move_to(
      boundaries::previous_boundary(editor, editor.cursor_offset(), cx),
      cx,
    );
  } else {
    editor.move_to(editor.selected_range.start, cx)
  }
  editor.ensure_cursor_visible(window, cx);
}

pub fn alt_left(editor: &mut Editor, _: &AltLeft, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  if editor.collapse_removed_selection(true, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.move_display_cursor_word_horizontal(-1, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.selected_range.is_empty() {
    editor.move_to(
      boundaries::previous_word_boundary(editor, editor.cursor_offset(), cx),
      cx,
    );
  } else {
    editor.move_to(editor.selected_range.start, cx)
  }
  editor.ensure_cursor_visible(window, cx);
}

pub fn cmd_left(editor: &mut Editor, _: &CmdLeft, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  if editor.collapse_removed_selection(true, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.move_display_cursor_line_boundary(true, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  let document = editor.document.read(cx);
  let cursor = editor.cursor_offset();
  let line = document.char_to_line(cursor);
  let line_start = document.line_to_char(line);
  editor.move_to(line_start, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn right(editor: &mut Editor, _: &Right, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  if editor.collapse_removed_selection(false, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.selected_range.is_empty() && editor.move_display_cursor_horizontal(1, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.selected_range.is_empty() {
    editor.move_to(
      boundaries::next_boundary(editor, editor.selected_range.end, cx),
      cx,
    );
  } else {
    editor.move_to(editor.selected_range.end, cx)
  }
  editor.ensure_cursor_visible(window, cx);
}

pub fn alt_right(editor: &mut Editor, _: &AltRight, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  if editor.collapse_removed_selection(false, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.move_display_cursor_word_horizontal(1, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.selected_range.is_empty() {
    editor.move_to(
      boundaries::next_word_boundary(editor, editor.selected_range.end, cx),
      cx,
    );
  } else {
    editor.move_to(editor.selected_range.end, cx)
  }
  editor.ensure_cursor_visible(window, cx);
}

pub fn cmd_right(editor: &mut Editor, _: &CmdRight, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  if editor.collapse_removed_selection(false, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.move_display_cursor_line_boundary(false, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  let document = editor.document.read(cx);
  let cursor = editor.cursor_offset();
  let line = document.char_to_line(cursor);
  let line_range = document.line_range(line).unwrap_or(0..0);
  let line_content = document.line_content(line).unwrap_or_default();
  let line_end = line_range.start + line_content.chars().count();
  editor.move_to(line_end, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn cmd_up(editor: &mut Editor, _: &CmdUp, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  let document = editor.document.read(cx);
  let target_line = editor
    .projection()
    .and_then(|projection| projection.visible_doc_lines.first().copied())
    .unwrap_or(0);
  let target = document.line_to_char(target_line);
  editor.move_to(target, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn cmd_down(editor: &mut Editor, _: &CmdDown, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  let document = editor.document.read(cx);
  let doc_line_count = document.len_lines();
  let target_line = editor
    .projection()
    .and_then(|projection| projection.visible_doc_lines.last().copied())
    .unwrap_or_else(|| doc_line_count.saturating_sub(1));
  let line_start = document.line_to_char(target_line);
  let line_content = document.line_content(target_line).unwrap_or_default();
  let line_end = line_start + line_content.chars().count();
  editor.move_to(line_end, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn home(editor: &mut Editor, _: &Home, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  editor.move_to(0, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn end(editor: &mut Editor, _: &End, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  let doc_len = editor.document.read(cx).len();
  editor.move_to(doc_len, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_up(editor: &mut Editor, _: &SelectUp, window: &mut Window, cx: &mut Context<Editor>) {
  if editor.select_display_cursor_vertical(-1, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  let anchor = if editor.selection_reversed {
    editor.selected_range.end
  } else {
    editor.selected_range.start
  };

  let cursor = {
    let document = editor.document.read(cx);
    let cursor_offset = editor.cursor_offset();
    let current_line = document.char_to_line(cursor_offset);

    if current_line > 0 {
      let target_column = *editor
        .target_column
        .get_or_insert_with(|| cursor_offset - document.line_to_char(current_line));

      let target_line = editor.previous_visible_doc_line(current_line).unwrap_or(0);
      let target_start = document.line_to_char(target_line);
      let target_len = document
        .line_content(target_line)
        .map(|line| line.chars().count())
        .unwrap_or(0);

      target_start + target_column.min(target_len)
    } else {
      editor.target_column = None;
      0
    }
  };

  if anchor <= cursor {
    editor.selected_range = anchor..cursor;
    editor.selection_reversed = false;
  } else {
    editor.selected_range = cursor..anchor;
    editor.selection_reversed = true;
  }
  editor.ensure_cursor_visible(window, cx);
  cx.notify();
}

pub fn select_down(
  editor: &mut Editor,
  _: &SelectDown,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  if editor.select_display_cursor_vertical(1, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  let anchor = if editor.selection_reversed {
    editor.selected_range.end
  } else {
    editor.selected_range.start
  };

  let cursor = {
    let document = editor.document.read(cx);
    let cursor_offset = editor.cursor_offset();
    let current_line = document.char_to_line(cursor_offset);
    let total_lines = document.len_lines();

    if current_line + 1 < total_lines {
      let target_column = *editor
        .target_column
        .get_or_insert_with(|| cursor_offset - document.line_to_char(current_line));

      let target_line = editor
        .next_visible_doc_line(current_line, total_lines)
        .unwrap_or(current_line);
      let target_start = document.line_to_char(target_line);
      let target_len = document
        .line_content(target_line)
        .map(|line| line.chars().count())
        .unwrap_or(0);

      target_start + target_column.min(target_len)
    } else {
      editor.target_column = None;
      document.len()
    }
  };

  if anchor <= cursor {
    editor.selected_range = anchor..cursor;
    editor.selection_reversed = false;
  } else {
    editor.selected_range = cursor..anchor;
    editor.selection_reversed = true;
  }
  editor.ensure_cursor_visible(window, cx);
  cx.notify();
}

pub fn select_left(editor: &mut Editor, _: &SelectLeft, _: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  if editor.select_display_cursor_prev_removed_line_end_from_boundary(cx) {
    return;
  }
  if editor.select_display_cursor_prev_display_line_end(cx) {
    return;
  }
  if editor.select_display_cursor_horizontal(-1, cx) {
    return;
  }
  editor.select_to(
    boundaries::previous_boundary(editor, editor.cursor_offset(), cx),
    cx,
  );
}

pub fn select_word_left(
  editor: &mut Editor,
  _: &SelectWordLeft,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.target_column = None;
  if editor.select_display_cursor_word_horizontal(-1, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  editor.select_to(
    boundaries::previous_word_boundary(editor, editor.cursor_offset(), cx),
    cx,
  );
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_right(
  editor: &mut Editor,
  _: &SelectRight,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.target_column = None;
  if editor.select_display_cursor_horizontal(1, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  editor.select_to(
    boundaries::next_boundary(editor, editor.cursor_offset(), cx),
    cx,
  );
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_word_right(
  editor: &mut Editor,
  _: &SelectWordRight,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.target_column = None;
  if editor.select_display_cursor_word_horizontal(1, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  editor.select_to(
    boundaries::next_word_boundary(editor, editor.cursor_offset(), cx),
    cx,
  );
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_cmd_left(
  editor: &mut Editor,
  _: &SelectCmdLeft,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  if editor.select_display_cursor_line_boundary(true, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  let document = editor.document.read(cx);
  let cursor = editor.cursor_offset();
  let line = document.char_to_line(cursor);
  let line_start = document.line_to_char(line);
  editor.select_to(line_start, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_cmd_right(
  editor: &mut Editor,
  _: &SelectCmdRight,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  if editor.select_display_cursor_line_boundary(false, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  let document = editor.document.read(cx);
  let cursor = editor.cursor_offset();
  let line = document.char_to_line(cursor);
  let line_range = document.line_range(line).unwrap_or(0..0);
  let line_content = document.line_content(line).unwrap_or_default();
  let line_end = line_range.start + line_content.chars().count();
  editor.select_to(line_end, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_cmd_up(
  editor: &mut Editor,
  _: &SelectCmdUp,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  if editor.select_display_cursor_to_display_boundary(true, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  let document = editor.document.read(cx);
  let target_line = editor
    .projection()
    .and_then(|projection| projection.visible_doc_lines.first().copied())
    .unwrap_or(0);
  let target = document.line_to_char(target_line);
  editor.select_to(target, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_cmd_down(
  editor: &mut Editor,
  _: &SelectCmdDown,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  if editor.select_display_cursor_to_display_boundary(false, cx) {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  let document = editor.document.read(cx);
  let doc_line_count = document.len_lines();
  let target_line = editor
    .projection()
    .and_then(|projection| projection.visible_doc_lines.last().copied())
    .unwrap_or_else(|| doc_line_count.saturating_sub(1));
  let line_start = document.line_to_char(target_line);
  let line_content = document.line_content(target_line).unwrap_or_default();
  let line_end = line_start + line_content.chars().count();
  editor.select_to(line_end, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_all(editor: &mut Editor, _: &SelectAll, _: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  if editor.select_all_display_lines(cx) {
    return;
  }
  let doc_len = editor.document.read(cx).len();

  editor.move_to(0, cx);
  editor.select_to(doc_len, cx);
}

pub fn paste(editor: &mut Editor, _: &Paste, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
    let cursor = editor.cursor_offset();
    let current_line = editor.document.read(cx).char_to_line(cursor);
    editor.replace_text_in_range(None, &text, window, cx);
    // Invalidate cache from current line onwards since paste may add multiple lines
    editor.invalidate_lines_from(current_line);
  }
}

pub fn copy(editor: &mut Editor, _: &Copy, _: &mut Window, cx: &mut Context<Editor>) {
  if let Some(text) = editor.selected_text_for_copy(cx) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
  }
}

pub fn cut(editor: &mut Editor, _: &Cut, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  if !editor.selected_range.is_empty() {
    let cursor = editor.cursor_offset();
    let current_line = editor.document.read(cx).char_to_line(cursor);
    cx.write_to_clipboard(ClipboardItem::new_string(
      editor
        .document
        .read(cx)
        .slice_to_string(editor.selected_range.clone()),
    ));
    editor.replace_text_in_range(None, "", window, cx);
    // Invalidate cache from current line onwards since cut may affect multiple lines
    editor.invalidate_lines_from(current_line);
  }
}

pub fn undo(editor: &mut Editor, _: &Undo, _window: &mut Window, cx: &mut Context<Editor>) {
  if let Some(transaction) = editor.undo_stack.pop_back() {
    let buffer_tx_id = editor.document.update(cx, |doc, cx| {
      let result = doc.undo(cx);

      if result.is_some() {
        doc.schedule_recompute_highlights(cx);
      }

      result
    });

    if buffer_tx_id.is_some() {
      editor.selected_range = transaction.selection_before.clone();
      editor.selection_reversed = false;

      editor.line_layouts.clear();

      editor.redo_stack.push_back(transaction);
      editor.is_dirty = true;

      cx.notify();
      editor.schedule_diff_recompute(cx);
    } else {
      editor.undo_stack.push_back(transaction);
    }
  }
}

pub fn redo(editor: &mut Editor, _: &Redo, _window: &mut Window, cx: &mut Context<Editor>) {
  if let Some(transaction) = editor.redo_stack.pop_back() {
    let buffer_tx_id = editor.document.update(cx, |doc, cx| {
      let result = doc.redo(cx);

      if result.is_some() {
        doc.schedule_recompute_highlights(cx);
      }

      result
    });

    if buffer_tx_id.is_some() {
      editor.selected_range = transaction.selection_after.clone();
      editor.selection_reversed = false;

      editor.line_layouts.clear();

      editor.undo_stack.push_back(transaction);
      editor.is_dirty = true;

      cx.notify();
      editor.schedule_diff_recompute(cx);
    } else {
      editor.redo_stack.push_back(transaction);
    }
  }
}

pub fn save(editor: &mut Editor, _: &Save, _window: &mut Window, cx: &mut Context<Editor>) {
  editor.save(cx);
}

pub fn find(editor: &mut Editor, _: &Find, window: &mut Window, cx: &mut Context<Editor>) {
  editor.open_find_panel(window, cx);
}

pub fn close_find(
  editor: &mut Editor,
  _: &CloseFind,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  // Actions stop propagating by default in the bubble phase; let escape reach the
  // host (to close the file view) when there was no find panel to close.
  if !editor.close_find_panel(window, cx) {
    cx.propagate();
  }
}

pub fn show_character_palette(
  _editor: &mut Editor,
  _: &ShowCharacterPalette,
  window: &mut Window,
  _: &mut Context<Editor>,
) {
  window.show_character_palette();
}

pub fn quit(_editor: &mut Editor, _: &Quit, _: &mut Window, cx: &mut Context<Editor>) {
  cx.quit();
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    editor::{DisplayCursor, DisplaySelection},
    projection::{DisplayLine, HunkState, Projection},
  };
  use std::{collections::HashMap, sync::Arc};

  fn editable_projection() -> Arc<Projection> {
    Arc::new(Projection {
      lines: vec![
        DisplayLine::Doc {
          doc_line: 0,
          old_line: Some(0),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
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
      display_to_doc: vec![Some(0), Some(1)],
      doc_to_display: vec![Some(0), Some(1)],
      visible_doc_lines: vec![0, 1],
      start_gap: None,
      end_gap: None,
      groups: HashMap::new(),
    })
  }

  fn projection_with_removed_line() -> Arc<Projection> {
    Arc::new(Projection {
      lines: vec![
        DisplayLine::Doc {
          doc_line: 0,
          old_line: Some(0),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        },
        DisplayLine::Removed {
          text: "removed".into(),
          anchor_line: 0,
          old_line: 0,
          hunk: HunkState::Unstaged,
          group_id: None,
          secondary: false,
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
      display_to_doc: vec![Some(0), None, Some(1)],
      doc_to_display: vec![Some(0), Some(2)],
      visible_doc_lines: vec![0, 1],
      start_gap: None,
      end_gap: None,
      groups: HashMap::new(),
    })
  }

  #[gpui::test]
  fn should_not_handle_backspace_in_display_space_for_editable_cursor(
    cx: &mut gpui::TestAppContext,
  ) {
    let mut ctx = crate::editor::tests::EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.projection = Some(editable_projection());
      editor.move_to(2, cx);
      assert!(!should_handle_backspace_in_display_space(editor, cx));
    });
  }

  #[gpui::test]
  fn should_handle_backspace_in_display_space_for_removed_cursor(cx: &mut gpui::TestAppContext) {
    let mut ctx = crate::editor::tests::EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, _cx| {
      editor.projection = Some(projection_with_removed_line());
      editor.selected_range = 0..0;
      editor.selection_reversed = false;
      editor.display_selection = Some(DisplaySelection {
        start: DisplayCursor { line: 1, column: 3 },
        end: DisplayCursor { line: 1, column: 3 },
      });

      assert!(should_handle_backspace_in_display_space(editor, _cx));
    });
  }

  #[gpui::test]
  fn should_not_handle_backspace_in_display_space_when_selection_is_not_empty(
    cx: &mut gpui::TestAppContext,
  ) {
    let mut ctx = crate::editor::tests::EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, _cx| {
      editor.projection = Some(projection_with_removed_line());
      editor.selected_range = 0..1;
      editor.selection_reversed = false;
      editor.display_selection = Some(DisplaySelection {
        start: DisplayCursor { line: 1, column: 3 },
        end: DisplayCursor { line: 1, column: 3 },
      });

      assert!(!should_handle_backspace_in_display_space(editor, _cx));
    });
  }
}
