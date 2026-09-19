use super::*;
use crate::{actions, editor::tests::EditorTestContext};
use gpui::{TestAppContext, VisualTestContext};
use gpui_component::Root;

fn setup<'a>(
  cx: &'a mut TestAppContext,
  marked: &str,
) -> (Entity<Editor>, &'a mut VisualTestContext) {
  cx.update(gpui_component::init);
  let offset = marked.split('|').next().unwrap_or_default().chars().count();
  let text = marked.replacen('|', "", 1);
  let editor = EditorTestContext::with_text(cx.clone(), &text).editor;
  editor.update(cx, |editor, cx| editor.move_to(offset, cx));
  let view = editor.clone();
  let (_, cx) = cx.add_window_view(move |window, cx| Root::new(view, window, cx));
  (editor, cx)
}

fn state(editor: &Entity<Editor>, cx: &VisualTestContext) -> (String, Range<usize>, bool) {
  editor.read_with(cx, |editor, cx| {
    let document = editor.document.read(cx);
    (
      document.slice_to_string(0..document.len()),
      editor.selected_range.clone(),
      editor.selection_reversed,
    )
  })
}

fn clipboard(cx: &mut VisualTestContext) -> ClipboardItem {
  cx.update(|_, cx| cx.read_from_clipboard().expect("clipboard item"))
}

fn copy(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      actions::copy(editor, &actions::Copy, window, cx)
    })
  });
}

fn cut(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      actions::cut(editor, &actions::Cut, window, cx)
    })
  });
}

fn paste(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      actions::paste(editor, &actions::Paste, window, cx)
    })
  });
}

fn undo(editor: &Entity<Editor>, redo: bool, cx: &mut VisualTestContext) {
  cx.update(|window, cx| editor.update(cx, |editor, cx| editor.undo_edit(redo, window, cx)));
}

#[gpui::test]
fn copy_current_line_preserves_text_endings_and_selection(cx: &mut TestAppContext) {
  for (before, copied) in [
    ("one\né|🙂\nlast", "é🙂\n"),
    ("one\né|🙂", "é🙂\n"),
    ("o|ne\r\né🙂", "one\r\n"),
    ("one\r\né|🙂", "é🙂\r\n"),
    ("one\r\né|🙂\nlast", "é🙂\n"),
    ("one\n|\nlast", "\n"),
    ("one\r\n|\r\nlast", "\r\n"),
    ("one\n|", "\n"),
    ("|", "\n"),
  ] {
    let (editor, cx) = setup(cx, before);
    let original = state(&editor, cx);
    copy(&editor, cx);
    let item = clipboard(cx);
    assert_eq!(item.text().as_deref(), Some(copied), "{before}");
    assert_eq!(item.metadata().map(String::as_str), Some(LINEWISE_METADATA));
    assert_eq!(state(&editor, cx), original);
    assert!(!editor.read_with(cx, |editor, cx| editor.document.read(cx).buffer.can_undo()));
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
  }
}

#[gpui::test]
fn cut_current_line_is_one_transaction_with_cursor_at_line_start(cx: &mut TestAppContext) {
  for (before, after, copied, cursor) in [
    ("one\né|🙂\nlast", "one\nlast", "é🙂\n", 4),
    ("one\né|🙂", "one\n", "é🙂\n", 4),
    ("one\r\né|🙂\r\nlast", "one\r\nlast", "é🙂\r\n", 5),
    ("one\r\né|🙂", "one\r\n", "é🙂\r\n", 5),
    ("one\n|\nlast", "one\nlast", "\n", 4),
    ("one\r\n|\r\nlast", "one\r\nlast", "\r\n", 5),
    ("on|ly", "", "only\n", 0),
  ] {
    let (editor, cx) = setup(cx, before);
    let original = state(&editor, cx);
    cut(&editor, cx);
    assert_eq!(clipboard(cx).text().as_deref(), Some(copied));
    assert_eq!(state(&editor, cx), (after.into(), cursor..cursor, false));
    undo(&editor, false, cx);
    assert_eq!(state(&editor, cx), original);
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
    assert!(!editor.read_with(cx, |editor, cx| editor.document.read(cx).buffer.can_undo()));
    undo(&editor, true, cx);
    assert_eq!(state(&editor, cx), (after.into(), cursor..cursor, false));
  }
}

#[gpui::test]
fn cut_empty_final_line_only_updates_clipboard(cx: &mut TestAppContext) {
  for before in ["|", "one\n|", "one\r\n|"] {
    let (editor, cx) = setup(cx, before);
    let original = state(&editor, cx);
    cut(&editor, cx);
    assert_eq!(state(&editor, cx), original);
    assert!(!editor.read_with(cx, |editor, cx| editor.document.read(cx).buffer.can_undo()));
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
    assert_eq!(
      clipboard(cx).text().as_deref(),
      Some(if before.contains('\r') { "\r\n" } else { "\n" })
    );
  }
}

#[gpui::test]
fn linewise_paste_inserts_before_line_and_preserves_cursor_column(cx: &mut TestAppContext) {
  for (before, after, cursor) in [
    ("one\né|🙂\nlast", "one\né🙂\né🙂\nlast", 8),
    ("one\né|🙂", "one\né🙂\né🙂", 8),
    ("one\r\né|🙂", "one\r\né🙂\r\né🙂", 10),
    ("one\n|", "one\n\n", 5),
    ("|", "\n", 1),
  ] {
    let (editor, cx) = setup(cx, before);
    let original = state(&editor, cx);
    copy(&editor, cx);
    paste(&editor, cx);
    assert_eq!(state(&editor, cx), (after.into(), cursor..cursor, false));
    undo(&editor, false, cx);
    assert_eq!(state(&editor, cx), original);
    assert!(!editor.read_with(cx, |editor, cx| editor.document.read(cx).buffer.can_undo()));
    undo(&editor, true, cx);
    assert_eq!(state(&editor, cx), (after.into(), cursor..cursor, false));
  }
}

#[gpui::test]
fn cut_and_paste_last_line_never_concatenates_with_target(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "first\nla|st");
  cut(&editor, cx);
  editor.update(cx, |editor, cx| editor.move_to(2, cx));
  paste(&editor, cx);
  assert_eq!(state(&editor, cx), ("last\nfirst\n".into(), 7..7, false));
  undo(&editor, false, cx);
  assert_eq!(state(&editor, cx), ("first\n".into(), 2..2, false));
  undo(&editor, false, cx);
  assert_eq!(state(&editor, cx), ("first\nlast".into(), 8..8, false));
}

#[gpui::test]
fn explicit_selection_replaces_literally_and_undo_restores_direction(cx: &mut TestAppContext) {
  for reversed in [false, true] {
    let (editor, cx) = setup(cx, "one\né|🙂\nlast");
    copy(&editor, cx);
    editor.update(cx, |editor, _| {
      editor.selected_range = 0..3;
      editor.selection_reversed = reversed;
    });
    let original = state(&editor, cx);
    paste(&editor, cx);
    assert_eq!(state(&editor, cx), ("é🙂\n\né🙂\nlast".into(), 3..3, false));
    undo(&editor, false, cx);
    assert_eq!(state(&editor, cx), original);
    copy(&editor, cx);
    assert_eq!(clipboard(cx).text().as_deref(), Some("one"));
    assert!(clipboard(cx).metadata().is_none());
    cut(&editor, cx);
    assert_eq!(clipboard(cx).text().as_deref(), Some("one"));
    assert!(clipboard(cx).metadata().is_none());
    assert_eq!(state(&editor, cx), ("\né🙂\nlast".into(), 0..0, false));
    undo(&editor, false, cx);
    assert_eq!(state(&editor, cx), original);
  }
}

#[gpui::test]
fn explicitly_copied_text_pastes_at_cursor_even_for_a_whole_line(cx: &mut TestAppContext) {
  for (selection, after, cursor) in [
    (1..3, "alpha\nbelpta", 10),
    (0..6, "alpha\nbealpha\nta", 14),
  ] {
    let (editor, cx) = setup(cx, "|alpha\nbeta");
    editor.update(cx, |editor, _| {
      editor.selected_range = selection;
      editor.selection_reversed = true;
    });
    let selected = state(&editor, cx);
    copy(&editor, cx);
    assert_eq!(state(&editor, cx), selected);
    assert!(clipboard(cx).metadata().is_none());
    editor.update(cx, |editor, cx| editor.move_to(8, cx));
    paste(&editor, cx);
    assert_eq!(state(&editor, cx), (after.into(), cursor..cursor, false));
  }
}

#[gpui::test]
fn non_code_and_blank_diff_rows_do_not_copy_or_edit_nearby_text(cx: &mut TestAppContext) {
  for split in [false, true] {
    let (editor, cx) = setup(cx, "ne|w");
    editor.update(cx, |editor, cx| {
      let line = if split {
        editor.selection_view = DiffElementView::SplitLeft;
        DisplayLine::Doc {
          doc_line: 0,
          old_line: None,
          change: Some(ChangeKind::Added),
          hunk: Some(HunkState::Unstaged),
          group_id: None,
          secondary: false,
        }
      } else {
        DisplayLine::ReviewComment {
          id: 1,
          side: ReviewCommentSide::Right,
          group_id: None,
          background: None,
          secondary: false,
          text: "not code".into(),
          is_header: true,
        }
      };
      editor.set_projection(Some(Projection::from_lines(
        1,
        vec![line],
        HashMap::new(),
        None,
        None,
      )));
      editor.set_display_cursor(DisplayCursor { line: 0, column: 0 }, cx);
    });
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("keep".into())));
    let original = state(&editor, cx);
    copy(&editor, cx);
    cut(&editor, cx);
    paste(&editor, cx);
    assert_eq!(clipboard(cx).text().as_deref(), Some("keep"));
    assert_eq!(state(&editor, cx), original);
  }
}

#[gpui::test]
fn external_or_unknown_metadata_pastes_at_cursor(cx: &mut TestAppContext) {
  for item in [
    ClipboardItem::new_string("é🙂\n".into()),
    ClipboardItem::new_string_with_metadata("é🙂\n".into(), "other-app".into()),
  ] {
    let (editor, cx) = setup(cx, "fi|rst");
    cx.update(|_, cx| cx.write_to_clipboard(item));
    paste(&editor, cx);
    assert_eq!(state(&editor, cx), ("fié🙂\nrst".into(), 5..5, false));
    undo(&editor, false, cx);
    assert_eq!(state(&editor, cx), ("first".into(), 2..2, false));
  }
}

#[gpui::test]
fn linewise_metadata_survives_paste_into_another_editor(cx: &mut TestAppContext) {
  let item = {
    let (source, cx) = setup(cx, "é|🙂");
    copy(&source, cx);
    clipboard(cx)
  };
  let (target, cx) = setup(cx, "fi|rst");
  cx.update(|_, cx| cx.write_to_clipboard(item));
  paste(&target, cx);
  assert_eq!(state(&target, cx), ("é🙂\nfirst".into(), 5..5, false));
}

#[gpui::test]
fn clipboard_edits_do_not_merge_with_typing(cx: &mut TestAppContext) {
  for cutting in [false, true] {
    let (editor, cx) = setup(cx, "|old\nnext");
    copy(&editor, cx);
    cx.update(|window, cx| {
      editor.update(cx, |editor, cx| {
        editor.replace_text_in_range(None, "x", window, cx)
      })
    });
    if cutting {
      cut(&editor, cx);
    } else {
      paste(&editor, cx);
    }
    let edited = state(&editor, cx);
    undo(&editor, false, cx);
    assert_eq!(state(&editor, cx), ("xold\nnext".into(), 1..1, false));
    undo(&editor, false, cx);
    assert_eq!(state(&editor, cx), ("old\nnext".into(), 0..0, false));
    undo(&editor, true, cx);
    undo(&editor, true, cx);
    assert_eq!(state(&editor, cx), edited);
  }
}

#[gpui::test]
fn linewise_paste_replaces_active_composition_literally(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "o|ld");
  copy(&editor, cx);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_and_mark_text_in_range(None, "é", Some(1..1), window, cx);
    })
  });
  paste(&editor, cx);
  assert_eq!(state(&editor, cx), ("oold\nld".into(), 5..5, false));
  assert!(editor.read_with(cx, |editor, _| editor.marked_range.is_none()));
}

#[gpui::test]
fn read_only_editor_allows_copy_but_not_cut_or_paste(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "one\né|🙂");
  editor.update(cx, |editor, _| editor.is_read_only = true);
  for selection in [5..5, 4..6] {
    editor.update(cx, |editor, _| editor.selected_range = selection.clone());
    let original = state(&editor, cx);
    copy(&editor, cx);
    let item = clipboard(cx);
    assert_eq!(
      item.text().as_deref(),
      Some(if selection.is_empty() {
        "é🙂\n"
      } else {
        "é🙂"
      })
    );
    cut(&editor, cx);
    paste(&editor, cx);
    assert_eq!(clipboard(cx).text(), item.text());
    assert_eq!(state(&editor, cx), original);
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
  }
}

fn project_old_text(editor: &Entity<Editor>, split: bool, cx: &mut VisualTestContext) {
  editor.update(cx, |editor, cx| {
    editor.diff_view_mode = if split {
      DiffViewMode::Split
    } else {
      DiffViewMode::Inline
    };
    editor.selection_view = if split {
      DiffElementView::SplitLeft
    } else {
      DiffElementView::Inline
    };
    let old_line = if split {
      DisplayLine::Modified {
        doc_line: 0,
        old_line: 0,
        old_text: "old é🙂".into(),
        hunk: HunkState::Unstaged,
        group_id: None,
        secondary: false,
      }
    } else {
      DisplayLine::Removed {
        text: "old é🙂".into(),
        old_line: 0,
        anchor_line: 0,
        hunk: HunkState::Unstaged,
        group_id: None,
        secondary: false,
      }
    };
    editor.set_projection(Some(Projection::from_lines(
      1,
      vec![
        old_line,
        DisplayLine::Doc {
          doc_line: 0,
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
    )));
    editor.set_display_cursor(DisplayCursor { line: 0, column: 5 }, cx);
  });
}

#[gpui::test]
fn old_diff_lines_allow_line_and_explicit_copy_without_modifying_document(cx: &mut TestAppContext) {
  for split in [false, true] {
    for selected in [false, true] {
      let (editor, cx) = setup(cx, "new te|xt");
      project_old_text(&editor, split, cx);
      if selected {
        editor.update(cx, |editor, cx| {
          editor.set_display_selection_with_anchor(
            DisplayCursor { line: 0, column: 6 },
            DisplayCursor { line: 0, column: 4 },
            cx,
          )
        });
      }
      let original = state(&editor, cx);
      copy(&editor, cx);
      let item = clipboard(cx);
      assert_eq!(
        item.text().as_deref(),
        Some(if selected { "é🙂" } else { "old é🙂\n" })
      );
      assert_eq!(item.metadata().is_some(), !selected);
      cut(&editor, cx);
      paste(&editor, cx);
      assert_eq!(clipboard(cx).text(), item.text());
      assert_eq!(state(&editor, cx), original);
      assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
      editor.update(cx, |editor, cx| {
        editor.selection_view = if split {
          DiffElementView::SplitRight
        } else {
          DiffElementView::Inline
        };
        editor.set_display_cursor(DisplayCursor { line: 1, column: 2 }, cx);
      });
      paste(&editor, cx);
      assert_eq!(
        state(&editor, cx).0,
        if selected {
          "neé🙂w text"
        } else {
          "old é🙂\nnew text"
        }
      );
    }
  }
}
