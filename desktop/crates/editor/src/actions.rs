//! Editor actions for navigation, editing, and selection
//!
//! This module contains all the action handlers for the editor,
//! including text editing, cursor movement, and selection operations.

use gpui::{ClipboardItem, Context, EntityInputHandler, Window, actions};

use crate::{boundaries, editor::{Editor, TAB_WHITESPACE_COUNT}};

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
    Quit,
  ]
);

pub fn enter(editor: &mut Editor, _: &Enter, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  editor.replace_text_in_range(None, "\n", window, cx);

  editor.ensure_cursor_visible(window, cx);
}

pub fn tab(editor: &mut Editor, _: &Tab, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  let tab_text = " ".repeat(TAB_WHITESPACE_COUNT);
  editor.replace_text_in_range(None, &tab_text, window, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn backspace(
  editor: &mut Editor,
  _: &Backspace,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.target_column = None;
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

    // If we're at the beginning of an empty line, behave like simple backspace
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

    // If we're at the beginning of an empty line, behave like simple backspace
    if cursor == line_start && document.line_content(line).unwrap_or_default().is_empty() {
      editor.select_to(boundaries::previous_boundary(editor, cursor, cx), cx);
    } else {
      // Delete from start of current line to cursor
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
  let new_cursor = {
    let document = editor.document.read(cx);
    let cursor_offset = editor.cursor_offset();
    let current_line = document.char_to_line(cursor_offset);

    if current_line > 0 {
      if editor.target_column.is_none() {
        let line_start = document.line_to_char(current_line);
        editor.target_column = Some(cursor_offset - line_start);
      }

      let target_column = editor.target_column.unwrap();

      // Calculate new position in target line
      let target_line = current_line - 1;
      let target_start = document.line_to_char(target_line);
      let target_len = document.line_content(target_line).unwrap_or_default().len();

      Some(target_start + target_column.min(target_len))
    } else {
      // On first line, go to beginning of buffer
      editor.target_column = None;
      Some(0)
    }
  };

  if let Some(cursor) = new_cursor {
    editor.move_to(cursor, cx);
    editor.ensure_cursor_visible(window, cx);
  }
}

pub fn down(editor: &mut Editor, _: &Down, window: &mut Window, cx: &mut Context<Editor>) {
  let new_cursor = {
    let document = editor.document.read(cx);
    let cursor_offset = editor.cursor_offset();
    let current_line = document.char_to_line(cursor_offset);

    if current_line < document.len_lines().saturating_sub(1) {
      if editor.target_column.is_none() {
        let line_start = document.line_to_char(current_line);
        editor.target_column = Some(cursor_offset - line_start);
      }

      let target_column = editor.target_column.unwrap();

      let target_line = current_line + 1;
      let target_start = document.line_to_char(target_line);
      let target_len = document.line_content(target_line).unwrap_or_default().len();

      Some(target_start + target_column.min(target_len))
    } else {
      editor.target_column = None;
      Some(document.len())
    }
  };

  if let Some(cursor) = new_cursor {
    editor.move_to(cursor, cx);
    editor.ensure_cursor_visible(window, cx);
  }
}

pub fn left(editor: &mut Editor, _: &Left, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
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
  let document = editor.document.read(cx);
  let cursor = editor.cursor_offset();
  let line = document.char_to_line(cursor);
  let line_start = document.line_to_char(line);
  editor.move_to(line_start, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn right(editor: &mut Editor, _: &Right, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
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
  let document = editor.document.read(cx);
  let cursor = editor.cursor_offset();
  let line = document.char_to_line(cursor);
  let line_range = document.line_range(line).unwrap_or(0..0);
  // Go to end of line content (before the newline)
  let line_content = document.line_content(line).unwrap_or_default();
  let line_end = line_range.start + line_content.len();
  editor.move_to(line_end, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn cmd_up(editor: &mut Editor, _: &CmdUp, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  editor.move_to(0, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn cmd_down(editor: &mut Editor, _: &CmdDown, window: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  let document = editor.document.read(cx);
  editor.move_to(document.len(), cx);
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
  // Keep the anchor point of the selection
  let anchor = if editor.selection_reversed {
    editor.selected_range.end
  } else {
    editor.selected_range.start
  };

  // Calculate new cursor position (same logic as up())
  let new_cursor = {
    let document = editor.document.read(cx);
    let cursor_offset = editor.cursor_offset();
    let current_line = document.char_to_line(cursor_offset);

    if current_line > 0 {
      if editor.target_column.is_none() {
        let line_start = document.line_to_char(current_line);
        editor.target_column = Some(cursor_offset - line_start);
      }

      let target_column = editor.target_column.unwrap();

      let target_line = current_line - 1;
      let target_start = document.line_to_char(target_line);
      let target_len = document.line_content(target_line).unwrap_or_default().len();

      Some(target_start + target_column.min(target_len))
    } else {
      editor.target_column = None;
      Some(0)
    }
  };

  // Move cursor and extend selection
  let cursor = new_cursor.unwrap();
  if anchor <= cursor {
    editor.selected_range = anchor..cursor;
    editor.selection_reversed = false;
  } else {
    editor.selected_range = cursor..anchor;
    editor.selection_reversed = true;
  }
  editor.sync_buffer_selection_from_view(cx);
  editor.ensure_cursor_visible(window, cx);
  cx.notify();
}

pub fn select_down(
  editor: &mut Editor,
  _: &SelectDown,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  // Keep the anchor point of the selection
  let anchor = if editor.selection_reversed {
    editor.selected_range.end
  } else {
    editor.selected_range.start
  };

  // Calculate new cursor position (same logic as down())
  let new_cursor = {
    let document = editor.document.read(cx);
    let cursor_offset = editor.cursor_offset();
    let current_line = document.char_to_line(cursor_offset);
    let total_lines = document.len_lines();

    if current_line + 1 < total_lines {
      if editor.target_column.is_none() {
        let line_start = document.line_to_char(current_line);
        editor.target_column = Some(cursor_offset - line_start);
      }

      let target_column = editor.target_column.unwrap();

      let target_line = current_line + 1;
      let target_start = document.line_to_char(target_line);
      let target_len = document.line_content(target_line).unwrap_or_default().len();

      Some(target_start + target_column.min(target_len))
    } else {
      // On last line, go to end of buffer
      editor.target_column = None;
      Some(document.len())
    }
  };

  // Move cursor and extend selection
  let cursor = new_cursor.unwrap();
  if anchor <= cursor {
    editor.selected_range = anchor..cursor;
    editor.selection_reversed = false;
  } else {
    editor.selected_range = cursor..anchor;
    editor.selection_reversed = true;
  }
  editor.sync_buffer_selection_from_view(cx);
  editor.ensure_cursor_visible(window, cx);
  cx.notify();
}

pub fn select_left(editor: &mut Editor, _: &SelectLeft, _: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
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
  let document = editor.document.read(cx);
  let cursor = editor.cursor_offset();
  let line = document.char_to_line(cursor);
  let line_range = document.line_range(line).unwrap_or(0..0);
  let line_content = document.line_content(line).unwrap_or_default();
  let line_end = line_range.start + line_content.len();
  editor.select_to(line_end, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_cmd_up(
  editor: &mut Editor,
  _: &SelectCmdUp,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.select_to(0, cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_cmd_down(
  editor: &mut Editor,
  _: &SelectCmdDown,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  let document = editor.document.read(cx);
  editor.select_to(document.len(), cx);
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_all(editor: &mut Editor, _: &SelectAll, _: &mut Window, cx: &mut Context<Editor>) {
  editor.target_column = None;
  let doc_len = editor.document.read(cx).len();

  editor.move_to(0, cx);
  editor.select_to(doc_len, cx);
}

// === Clipboard Actions ===

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
  if !editor.selected_range.is_empty() {
    cx.write_to_clipboard(ClipboardItem::new_string(
      editor
        .document
        .read(cx)
        .slice_to_string(editor.selected_range.clone()),
    ));
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

// === Undo/Redo Actions ===

pub fn undo(editor: &mut Editor, _: &Undo, _window: &mut Window, cx: &mut Context<Editor>) {
  if let Some(transaction) = editor.undo_stack.pop_back() {
    let buffer_tx_id = editor.document.update(cx, |doc, cx| {
      let result = doc.undo(cx);

      // Trigger async syntax re-highlighting after undo
      if result.is_some() {
        doc.schedule_recompute_highlights(cx);
      }

      result
    });

    // Only restore selection if buffer undo succeeded
    if buffer_tx_id.is_some() {
      // Restore cursor position from before the transaction
      editor.selected_range = transaction.selection_before.clone();
      editor.selection_reversed = false;
      editor.sync_buffer_selection_from_view(cx);

      // Invalidate cache (content may have changed significantly)
      editor.line_layouts.clear();

      // Move transaction to redo stack
      editor.redo_stack.push_back(transaction);

      cx.notify();
    } else {
      // Buffer undo failed, push transaction back
      editor.undo_stack.push_back(transaction);
    }
  }
}

pub fn redo(editor: &mut Editor, _: &Redo, _window: &mut Window, cx: &mut Context<Editor>) {
  if let Some(transaction) = editor.redo_stack.pop_back() {
    let buffer_tx_id = editor.document.update(cx, |doc, cx| {
      let result = doc.redo(cx);

      // Trigger async syntax re-highlighting after redo
      if result.is_some() {
        doc.schedule_recompute_highlights(cx);
      }

      result
    });

    // Only restore selection if buffer redo succeeded
    if buffer_tx_id.is_some() {
      // Restore cursor position from after the transaction
      editor.selected_range = transaction.selection_after.clone();
      editor.selection_reversed = false;
      editor.sync_buffer_selection_from_view(cx);

      // Invalidate cache
      editor.line_layouts.clear();

      // Move transaction to undo stack
      editor.undo_stack.push_back(transaction);

      cx.notify();
    } else {
      // Buffer redo failed, push transaction back
      editor.redo_stack.push_back(transaction);
    }
  }
}

// === System Actions ===

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
