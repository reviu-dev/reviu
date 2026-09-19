//! Editor actions for navigation, editing, and selection
//!
//! This module contains all the action handlers for the editor,
//! including text editing, cursor movement, and selection operations.

use gpui::{ClipboardItem, Context, EntityInputHandler, Window, actions};

use crate::{
  boundaries,
  editor::{Editor, navigation::HorizontalMotion},
};

actions!(
  editor,
  [
    Enter,
    Tab,
    Outdent,
    MoveLineUp,
    MoveLineDown,
    DuplicateLineUp,
    DuplicateLineDown,
    DeleteLine,
    NewlineAbove,
    NewlineBelow,
    ToggleComments,
    Backspace,
    BackspaceWord,
    BackspaceAll,
    Delete,
    Up,
    Down,
    PageUp,
    PageDown,
    SelectPageUp,
    SelectPageDown,
    GoToLine,
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
    ReloadFromDisk,
    OverwriteDisk,
    Find,
    FindNext,
    FindPrevious,
    ToggleFindCaseSensitive,
    ToggleFindWholeWord,
    ToggleFindRegex,
    CloseFind,
    Quit,
  ]
);

fn should_handle_backspace_in_display_space(editor: &Editor, cx: &Context<Editor>) -> bool {
  editor.selected_range.is_empty() && editor.is_read_only_display_cursor(cx)
}

pub fn enter(editor: &mut Editor, _: &Enter, _window: &mut Window, cx: &mut Context<Editor>) {
  editor.insert_indented_newline(cx);
}

pub fn tab(editor: &mut Editor, _: &Tab, _window: &mut Window, cx: &mut Context<Editor>) {
  editor.indent_selection(false, cx);
}

pub fn outdent(editor: &mut Editor, _: &Outdent, _window: &mut Window, cx: &mut Context<Editor>) {
  editor.indent_selection(true, cx);
}

pub fn move_line_up(
  editor: &mut Editor,
  _: &MoveLineUp,
  _window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.move_lines(false, cx);
}

pub fn move_line_down(
  editor: &mut Editor,
  _: &MoveLineDown,
  _window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.move_lines(true, cx);
}

pub fn duplicate_line_up(
  editor: &mut Editor,
  _: &DuplicateLineUp,
  _window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.duplicate_lines(false, cx);
}

pub fn duplicate_line_down(
  editor: &mut Editor,
  _: &DuplicateLineDown,
  _window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.duplicate_lines(true, cx);
}

pub fn delete_line(
  editor: &mut Editor,
  _: &DeleteLine,
  _window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.delete_lines(cx);
}

pub fn newline_above(
  editor: &mut Editor,
  _: &NewlineAbove,
  _window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.insert_line(false, cx);
}

pub fn newline_below(
  editor: &mut Editor,
  _: &NewlineBelow,
  _window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.insert_line(true, cx);
}

pub fn toggle_comments(
  editor: &mut Editor,
  _: &ToggleComments,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  use gpui_component::{WindowExt, notification::Notification};
  if let Err(message) = editor.toggle_line_comments(cx) {
    window.push_notification(Notification::warning(message), cx);
  }
}

pub fn backspace(
  editor: &mut Editor,
  _: &Backspace,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.vertical_goal_x = None;
  if should_handle_backspace_in_display_space(editor, cx)
    && editor.move_display_cursor_horizontal(-1, cx)
  {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  if editor.backspace_auto_pair(cx) {
    return;
  }
  let range = if editor.selected_range.is_empty() {
    boundaries::previous_boundary(editor, editor.cursor_offset(), cx)..editor.cursor_offset()
  } else {
    editor.selected_range.clone()
  };
  let range = editor.range_to_utf16(&range, cx);
  editor.replace_text_in_range(Some(range), "", window, cx)
}

pub fn backspace_word(
  editor: &mut Editor,
  _: &BackspaceWord,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.vertical_goal_x = None;
  editor.finalize_transaction(cx);
  let range = if editor.selected_range.is_empty() {
    let document = editor.document.read(cx);
    let cursor = editor.cursor_offset();
    let line = document.char_to_line(cursor);
    let start = if cursor == document.line_to_char(line)
      && document.line_content(line).unwrap_or_default().is_empty()
    {
      boundaries::previous_boundary(editor, cursor, cx)
    } else {
      boundaries::previous_word_boundary(editor, cursor, cx)
    };
    start..cursor
  } else {
    editor.selected_range.clone()
  };
  let range = editor.range_to_utf16(&range, cx);
  editor.replace_text_in_range(Some(range), "", window, cx);
  editor.finalize_transaction(cx);
}

pub fn backspace_all(
  editor: &mut Editor,
  _: &BackspaceAll,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.vertical_goal_x = None;
  editor.finalize_transaction(cx);
  let range = if editor.selected_range.is_empty() {
    let document = editor.document.read(cx);
    let cursor = editor.cursor_offset();
    let line = document.char_to_line(cursor);
    let line_start = document.line_to_char(line);
    let start =
      if cursor == line_start && document.line_content(line).unwrap_or_default().is_empty() {
        boundaries::previous_boundary(editor, cursor, cx)
      } else {
        line_start
      };
    start..cursor
  } else {
    editor.selected_range.clone()
  };
  let range = editor.range_to_utf16(&range, cx);
  editor.replace_text_in_range(Some(range), "", window, cx);
  editor.finalize_transaction(cx);
}

pub fn delete(editor: &mut Editor, _: &Delete, window: &mut Window, cx: &mut Context<Editor>) {
  editor.vertical_goal_x = None;
  let range = if editor.selected_range.is_empty() {
    editor.cursor_offset()..boundaries::next_boundary(editor, editor.cursor_offset(), cx)
  } else {
    editor.selected_range.clone()
  };
  let range = editor.range_to_utf16(&range, cx);
  editor.replace_text_in_range(Some(range), "", window, cx)
}

pub fn up(editor: &mut Editor, _: &Up, window: &mut Window, cx: &mut Context<Editor>) {
  editor.navigate_vertical(-1, false, false, window, cx);
}

pub fn down(editor: &mut Editor, _: &Down, window: &mut Window, cx: &mut Context<Editor>) {
  editor.navigate_vertical(1, false, false, window, cx);
}

pub fn page_up(editor: &mut Editor, _: &PageUp, window: &mut Window, cx: &mut Context<Editor>) {
  editor.navigate_vertical(-1, false, true, window, cx);
}

pub fn page_down(editor: &mut Editor, _: &PageDown, window: &mut Window, cx: &mut Context<Editor>) {
  editor.navigate_vertical(1, false, true, window, cx);
}

pub fn select_page_up(
  editor: &mut Editor,
  _: &SelectPageUp,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.navigate_vertical(-1, true, true, window, cx);
}

pub fn select_page_down(
  editor: &mut Editor,
  _: &SelectPageDown,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.navigate_vertical(1, true, true, window, cx);
}

pub fn go_to_line(
  editor: &mut Editor,
  _: &GoToLine,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.open_go_to_line(window, cx);
}

pub fn left(editor: &mut Editor, _: &Left, window: &mut Window, cx: &mut Context<Editor>) {
  if editor.navigate_old_side(HorizontalMotion::Character(-1), false, window, cx) {
    return;
  }
  editor.vertical_goal_x = None;
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
  if editor.navigate_old_side(HorizontalMotion::Word(-1), false, window, cx) {
    return;
  }
  editor.vertical_goal_x = None;
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
  if editor.navigate_old_side(HorizontalMotion::LineBoundary(true), false, window, cx) {
    return;
  }
  editor.vertical_goal_x = None;
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
  if editor.navigate_old_side(HorizontalMotion::Character(1), false, window, cx) {
    return;
  }
  editor.vertical_goal_x = None;
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
  if editor.navigate_old_side(HorizontalMotion::Word(1), false, window, cx) {
    return;
  }
  editor.vertical_goal_x = None;
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
  if editor.navigate_old_side(HorizontalMotion::LineBoundary(false), false, window, cx) {
    return;
  }
  editor.vertical_goal_x = None;
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
  editor.navigate_document_boundary(true, false, window, cx);
}

pub fn cmd_down(editor: &mut Editor, _: &CmdDown, window: &mut Window, cx: &mut Context<Editor>) {
  editor.navigate_document_boundary(false, false, window, cx);
}

pub fn home(editor: &mut Editor, _: &Home, window: &mut Window, cx: &mut Context<Editor>) {
  editor.navigate_document_boundary(true, false, window, cx);
}

pub fn end(editor: &mut Editor, _: &End, window: &mut Window, cx: &mut Context<Editor>) {
  editor.navigate_document_boundary(false, false, window, cx);
}

pub fn select_up(editor: &mut Editor, _: &SelectUp, window: &mut Window, cx: &mut Context<Editor>) {
  editor.navigate_vertical(-1, true, false, window, cx);
}

pub fn select_down(
  editor: &mut Editor,
  _: &SelectDown,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.navigate_vertical(1, true, false, window, cx);
}

pub fn select_left(
  editor: &mut Editor,
  _: &SelectLeft,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  if editor.navigate_old_side(HorizontalMotion::Character(-1), true, window, cx) {
    return;
  }
  editor.vertical_goal_x = None;
  if editor.select_display_cursor_prev_removed_line_end_from_boundary(cx)
    || editor.select_display_cursor_prev_display_line_end(cx)
    || editor.select_display_cursor_horizontal(-1, cx)
  {
    editor.ensure_cursor_visible(window, cx);
    return;
  }
  editor.select_to(
    boundaries::previous_boundary(editor, editor.cursor_offset(), cx),
    cx,
  );
  editor.ensure_cursor_visible(window, cx);
}

pub fn select_word_left(
  editor: &mut Editor,
  _: &SelectWordLeft,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  if editor.navigate_old_side(HorizontalMotion::Word(-1), true, window, cx) {
    return;
  }
  editor.vertical_goal_x = None;
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
  if editor.navigate_old_side(HorizontalMotion::Character(1), true, window, cx) {
    return;
  }
  editor.vertical_goal_x = None;
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
  if editor.navigate_old_side(HorizontalMotion::Word(1), true, window, cx) {
    return;
  }
  editor.vertical_goal_x = None;
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
  if editor.navigate_old_side(HorizontalMotion::LineBoundary(true), true, window, cx) {
    return;
  }
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
  if editor.navigate_old_side(HorizontalMotion::LineBoundary(false), true, window, cx) {
    return;
  }
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
  editor.navigate_document_boundary(true, true, window, cx);
}

pub fn select_cmd_down(
  editor: &mut Editor,
  _: &SelectCmdDown,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.navigate_document_boundary(false, true, window, cx);
}

pub fn select_all(editor: &mut Editor, _: &SelectAll, _: &mut Window, cx: &mut Context<Editor>) {
  editor.vertical_goal_x = None;
  editor.select_all_display_lines(cx);
}

pub fn paste(editor: &mut Editor, _: &Paste, _window: &mut Window, cx: &mut Context<Editor>) {
  editor.finalize_transaction(cx);
  editor.vertical_goal_x = None;
  if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
    let cursor = editor.cursor_offset();
    let current_line = editor.document.read(cx).char_to_line(cursor);
    editor.replace_literal_text_in_range(None, &text, cx);
    // Invalidate cache from current line onwards since paste may add multiple lines
    editor.invalidate_lines_from(current_line);
    editor.finalize_transaction(cx);
  }
}

pub fn copy(editor: &mut Editor, _: &Copy, _: &mut Window, cx: &mut Context<Editor>) {
  if let Some(text) = editor.selected_text_for_copy(cx) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
  }
}

pub fn cut(editor: &mut Editor, _: &Cut, window: &mut Window, cx: &mut Context<Editor>) {
  if editor.selection_is_read_only() {
    return;
  }
  editor.finalize_transaction(cx);
  editor.vertical_goal_x = None;
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
    editor.finalize_transaction(cx);
  }
}

pub fn undo(editor: &mut Editor, _: &Undo, window: &mut Window, cx: &mut Context<Editor>) {
  editor.undo_edit(false, window, cx);
}

pub fn redo(editor: &mut Editor, _: &Redo, window: &mut Window, cx: &mut Context<Editor>) {
  editor.undo_edit(true, window, cx);
}

pub fn reload_from_disk(
  editor: &mut Editor,
  _: &ReloadFromDisk,
  _: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.reload_changed_file(cx);
}

pub fn overwrite_disk(
  editor: &mut Editor,
  _: &OverwriteDisk,
  _: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.overwrite_changed_file(cx);
}

pub fn save(editor: &mut Editor, _: &Save, _window: &mut Window, cx: &mut Context<Editor>) {
  editor.save(cx);
}

pub fn find(editor: &mut Editor, _: &Find, window: &mut Window, cx: &mut Context<Editor>) {
  editor.open_find_panel(window, cx);
}

pub fn find_next(editor: &mut Editor, _: &FindNext, window: &mut Window, cx: &mut Context<Editor>) {
  editor.find_next_match(window, cx);
}

pub fn find_previous(
  editor: &mut Editor,
  _: &FindPrevious,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.find_previous_match(window, cx);
}

pub fn toggle_find_case_sensitive(
  editor: &mut Editor,
  _: &ToggleFindCaseSensitive,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.toggle_find_case_sensitive(window, cx);
}

pub fn toggle_find_whole_word(
  editor: &mut Editor,
  _: &ToggleFindWholeWord,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.toggle_find_whole_word(window, cx);
}

pub fn toggle_find_regex(
  editor: &mut Editor,
  _: &ToggleFindRegex,
  window: &mut Window,
  cx: &mut Context<Editor>,
) {
  editor.toggle_find_regex(window, cx);
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
    let lines = vec![
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
    ];
    Arc::new(Projection::from_lines(2, lines, HashMap::new(), None, None))
  }

  fn projection_with_removed_line() -> Arc<Projection> {
    let lines = vec![
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
    ];
    Arc::new(Projection::from_lines(2, lines, HashMap::new(), None, None))
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
