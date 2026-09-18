use super::*;
use crate::editor::tests::EditorTestContext;
use gpui::{TestAppContext, VisualTestContext};
use gpui_component::Root;

fn setup<'a>(
  cx: &'a mut TestAppContext,
  marked: &str,
  language: Option<&str>,
) -> (Entity<Editor>, &'a mut VisualTestContext) {
  cx.update(gpui_component::init);
  let offset = marked.split('|').next().unwrap_or_default().chars().count();
  let text = marked.replacen('|', "", 1);
  let editor = EditorTestContext::with_text_and_extension(cx.clone(), &text, language).editor;
  editor.update(cx, |editor, cx| editor.move_to(offset, cx));
  let view = editor.clone();
  let (_, cx) = cx.add_window_view(move |window, cx| Root::new(view, window, cx));
  (editor, cx)
}

fn text(editor: &Entity<Editor>, cx: &VisualTestContext) -> String {
  editor.read_with(cx, |editor, cx| {
    let document = editor.document.read(cx);
    document.slice_to_string(0..document.len())
  })
}

fn undo(editor: &Entity<Editor>, redo: bool, cx: &mut VisualTestContext) {
  cx.update(|window, cx| editor.update(cx, |editor, cx| editor.undo_edit(redo, window, cx)));
}

#[gpui::test]
fn moving_lines_preserves_unicode_columns_and_line_endings(cx: &mut TestAppContext) {
  for (before, down, after, cursor) in [
    ("one\né|🙂\nlast", false, "é🙂\none\nlast", 1),
    ("é|🙂\none\nlast", true, "one\né🙂\nlast", 5),
    ("one\r\né|🙂", false, "é🙂\r\none", 1),
    ("o|ne\r\né🙂", true, "é🙂\r\none", 5),
    ("one\n|", false, "\none", 0),
    ("a\r\nb|\nc", false, "b\r\na\nc", 1),
  ] {
    let (editor, cx) = setup(cx, before, None);
    let original_selection = editor.read_with(cx, |editor, _| editor.selected_range.clone());
    editor.update(cx, |editor, cx| editor.move_lines(down, cx));
    assert_eq!(text(&editor, cx), after, "{before}");
    assert_eq!(
      editor.read_with(cx, |editor, _| editor.selected_range.clone()),
      cursor..cursor
    );
    undo(&editor, false, cx);
    assert_eq!(text(&editor, cx), before.replace('|', ""));
    assert_eq!(
      editor.read_with(cx, |editor, _| editor.selected_range.clone()),
      original_selection
    );
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
    undo(&editor, true, cx);
    assert_eq!(text(&editor, cx), after);
  }
}

#[gpui::test]
fn moving_selection_excludes_last_line_start_and_keeps_direction(cx: &mut TestAppContext) {
  for (before, after, end) in [
    ("a\nb\nc\nlast", "c\na\nb\nlast", 6),
    ("a\nb\nc", "c\na\nb", 5),
  ] {
    let (editor, cx) = setup(cx, &format!("|{before}"), None);
    editor.update(cx, |editor, cx| {
      editor.selected_range = 0..4;
      editor.selection_reversed = true;
      editor.move_lines(true, cx);
    });
    assert_eq!(text(&editor, cx), after);
    editor.read_with(cx, |editor, _| {
      assert_eq!(editor.selected_range, 2..end);
      assert!(editor.selection_reversed);
    });
    undo(&editor, false, cx);
    assert_eq!(text(&editor, cx), before);
    editor.read_with(cx, |editor, _| {
      assert_eq!(editor.selected_range, 0..4);
      assert!(editor.selection_reversed);
    });
  }
}

#[gpui::test]
fn boundary_moves_and_identical_lines_do_not_dirty_the_document(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|same\nsame", None);
  editor.update(cx, |editor, cx| {
    editor.move_lines(false, cx);
    editor.move_lines(true, cx);
    assert_eq!(editor.selected_range, 5..5);
    editor.move_lines(true, cx);
  });
  assert_eq!(text(&editor, cx), "same\nsame");
  assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
  assert!(!editor.read_with(cx, |editor, cx| editor.document.read(cx).buffer.can_undo()));
}

#[gpui::test]
fn duplicate_moves_selection_to_the_requested_copy(cx: &mut TestAppContext) {
  for (before, after) in [
    ("é|🙂", "é🙂\né🙂"),
    ("é|🙂\r\nx", "é🙂\r\né🙂\r\nx"),
    ("|", "\n"),
    ("a\n|", "a\n\n"),
  ] {
    for down in [false, true] {
      let (editor, cx) = setup(cx, before, None);
      let old_cursor = editor.read_with(cx, |editor, _| editor.cursor_offset());
      editor.update(cx, |editor, cx| editor.duplicate_lines(down, cx));
      assert_eq!(text(&editor, cx), after);
      let added = after.chars().count() - before.replace('|', "").chars().count();
      assert_eq!(
        editor.read_with(cx, |editor, _| editor.cursor_offset()),
        old_cursor + if down { added } else { 0 }
      );
      undo(&editor, false, cx);
      assert_eq!(text(&editor, cx), before.replace('|', ""));
    }
  }
  let (editor, cx) = setup(cx, "|a\nb\nc", None);
  editor.update(cx, |editor, cx| {
    editor.selected_range = 0..4;
    editor.selection_reversed = true;
    editor.duplicate_lines(true, cx);
  });
  assert_eq!(text(&editor, cx), "a\nb\na\nb\nc");
  editor.read_with(cx, |editor, _| {
    assert_eq!(editor.selected_range, 4..8);
    assert!(editor.selection_reversed);
  });
}

#[gpui::test]
fn deleting_lines_preserves_column_and_handles_eof(cx: &mut TestAppContext) {
  for (before, after, cursor) in [
    ("ab|cd\nxyz\nlast", "xyz\nlast", 2),
    ("first\r\nab|cd", "first", 2),
    ("first\n|", "first", 0),
    ("|only", "", 0),
    ("a\n|b\n", "a\n", 2),
    ("x|yz\ne\u{301}cho", "e\u{301}cho", 2),
    ("a|b\n👩‍💻work", "👩‍💻work", 3),
  ] {
    let (editor, cx) = setup(cx, before, None);
    editor.update(cx, |editor, cx| editor.delete_lines(cx));
    assert_eq!(text(&editor, cx), after);
    assert_eq!(
      editor.read_with(cx, |editor, _| editor.cursor_offset()),
      cursor
    );
    undo(&editor, false, cx);
    assert_eq!(text(&editor, cx), before.replace('|', ""));
  }
}

#[gpui::test]
fn deleting_a_selection_uses_whole_rows_and_is_one_undo_step(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|a\nb\nc", None);
  editor.update(cx, |editor, cx| {
    editor.selected_range = 1..4;
    editor.selection_reversed = true;
    editor.delete_lines(cx);
  });
  assert_eq!(text(&editor, cx), "c");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "a\nb\nc");
  editor.read_with(cx, |editor, _| {
    assert_eq!(editor.selected_range, 1..4);
    assert!(editor.selection_reversed);
    assert!(!editor.is_dirty);
  });
  editor.update(cx, |editor, cx| {
    editor.selected_range = 0..editor.document.read(cx).len();
    editor.delete_lines(cx);
  });
  assert_eq!(text(&editor, cx), "");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "a\nb\nc");
}

#[gpui::test]
fn insert_above_and_below_keep_existing_text_and_use_indentation(cx: &mut TestAppContext) {
  for (before, below, after, cursor) in [
    ("fn ma|in() {}", true, "fn main() {}\n", 13),
    ("fn ma|in() {", true, "fn main() {\n    ", 16),
    ("  co|de();\nnext", false, "  \n  code();\nnext", 2),
    ("{\n  code();\n|}", false, "{\n  code();\n  \n}", 14),
    ("one\r\n  va|lue", true, "one\r\n  value\r\n  ", 16),
    ("|", false, "\n", 0),
  ] {
    let (editor, cx) = setup(cx, before, Some("rs"));
    editor.update(cx, |editor, cx| editor.insert_line(below, cx));
    assert_eq!(text(&editor, cx), after, "{before}");
    assert_eq!(
      editor.read_with(cx, |editor, _| editor.cursor_offset()),
      cursor,
      "{before}"
    );
    undo(&editor, false, cx);
    assert_eq!(text(&editor, cx), before.replace('|', ""));
  }
}

#[gpui::test]
fn line_comments_align_and_roundtrip_with_blank_lines(cx: &mut TestAppContext) {
  for (language, marker) in [
    ("rs", "//"),
    ("ts", "//"),
    ("py", "#"),
    ("yaml", "#"),
    ("sql", "--"),
    ("lua", "--"),
    ("clj", ";"),
  ] {
    let original = "  café\n\n    🙂\nlast";
    let (editor, cx) = setup(cx, &format!("|{original}"), Some(language));
    editor.update(cx, |editor, cx| {
      editor.selected_range = 0..14;
      editor.selection_reversed = true;
      editor.toggle_line_comments(cx).expect("supported language");
    });
    assert_eq!(
      text(&editor, cx),
      format!("  {marker} café\n\n  {marker}   🙂\nlast"),
      "{language}"
    );
    let after = text(&editor, cx);
    undo(&editor, false, cx);
    assert_eq!(text(&editor, cx), original);
    assert!(editor.read_with(cx, |editor, _| editor.selection_reversed));
    undo(&editor, true, cx);
    assert_eq!(text(&editor, cx), after);
    editor.update(cx, |editor, cx| {
      editor.toggle_line_comments(cx).expect("uncomment")
    });
    assert_eq!(text(&editor, cx), original);
  }
}

#[gpui::test]
fn mixed_comments_add_one_layer_and_remove_only_one_optional_space(cx: &mut TestAppContext) {
  let original = "//old\n  code\n//  aligned";
  let (editor, cx) = setup(cx, &format!("|{original}"), Some("rs"));
  editor.update(cx, |editor, cx| {
    editor.selected_range = 0..editor.document.read(cx).len();
    editor.toggle_line_comments(cx).expect("comment");
  });
  assert_eq!(text(&editor, cx), "// //old\n//   code\n// //  aligned");
  editor.update(cx, |editor, cx| {
    editor.toggle_line_comments(cx).expect("uncomment")
  });
  assert_eq!(text(&editor, cx), original);
}

#[gpui::test]
fn block_comments_roundtrip_without_losing_whitespace(cx: &mut TestAppContext) {
  for (language, original, expected) in [
    (
      "css",
      "  a {\n    color: red;\n  }  ",
      "  /* a {\n    color: red;\n  } */  ",
    ),
    ("html", "  <p>é</p>", "  <!-- <p>é</p> -->"),
    ("ml", "let value = 1", "(* let value = 1 *)"),
    ("css", "   ", "   /*  */"),
  ] {
    let (editor, cx) = setup(cx, &format!("|{original}"), Some(language));
    editor.update(cx, |editor, cx| {
      editor.selected_range = 0..editor.document.read(cx).len();
      editor.toggle_line_comments(cx).expect("comment");
    });
    assert_eq!(text(&editor, cx), expected);
    editor.update(cx, |editor, cx| {
      editor.toggle_line_comments(cx).expect("uncomment")
    });
    assert_eq!(text(&editor, cx), original);
    undo(&editor, false, cx);
    assert_eq!(text(&editor, cx), expected);
  }
}

#[gpui::test]
fn block_comment_selection_stays_inside_the_delimiters(cx: &mut TestAppContext) {
  for source in ["é🙂", ""] {
    let (editor, cx) = setup(cx, &format!("|{source}"), Some("css"));
    let length = source.chars().count();
    editor.update(cx, |editor, cx| {
      editor.selected_range = 0..length;
      editor.toggle_line_comments(cx).expect("comment");
      assert_eq!(editor.selected_range, 3..3 + length);
    });
    undo(&editor, false, cx);
    assert_eq!(text(&editor, cx), source);
    assert_eq!(
      editor.read_with(cx, |editor, _| editor.selected_range.clone()),
      0..length
    );
    undo(&editor, true, cx);
    assert_eq!(
      editor.read_with(cx, |editor, _| editor.selected_range.clone()),
      3..3 + length
    );
  }
}

#[gpui::test]
fn unsupported_or_unsafe_comments_leave_the_buffer_untouched(cx: &mut TestAppContext) {
  for (language, source) in [
    (Some("json"), "{}"),
    (None, "text"),
    (Some("css"), "a /* old */ b"),
    (Some("css"), "/* a */\n/* b */"),
    (Some("html"), "a -- b"),
  ] {
    let (editor, cx) = setup(cx, &format!("|{source}"), language);
    editor.update(cx, |editor, cx| {
      editor.selected_range = 0..editor.document.read(cx).len();
      assert!(editor.toggle_line_comments(cx).is_err());
      assert!(!editor.is_dirty);
    });
    assert_eq!(text(&editor, cx), source);
  }
}

#[gpui::test]
fn removed_inline_rows_cannot_be_edited_by_line_actions(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|current", Some("rs"));
  editor.update(cx, |editor, cx| {
    editor.set_projection(Some(Projection::from_lines(
      1,
      vec![
        DisplayLine::Removed {
          text: "old".into(),
          old_line: 0,
          anchor_line: 0,
          hunk: HunkState::Unstaged,
          group_id: None,
          secondary: false,
        },
        DisplayLine::Doc {
          doc_line: 0,
          old_line: None,
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
    let cursor = DisplayCursor { line: 0, column: 1 };
    editor.display_selection = Some(DisplaySelection {
      start: cursor,
      end: cursor,
    });
    assert!(editor.is_read_only_display_cursor(cx));
    editor.move_lines(true, cx);
    editor.duplicate_lines(true, cx);
    editor.delete_lines(cx);
    editor.insert_line(true, cx);
    editor.toggle_line_comments(cx).expect("read-only no-op");
  });
  assert_eq!(text(&editor, cx), "current");
  assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
}

#[gpui::test]
fn line_actions_never_edit_read_only_or_old_split_content(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "a\n|b\nc", Some("rs"));
  for split in [false, true] {
    editor.update(cx, |editor, cx| {
      editor.is_read_only = !split;
      editor.selection_view = if split {
        DiffElementView::SplitLeft
      } else {
        DiffElementView::Inline
      };
      editor.move_lines(false, cx);
      editor.move_lines(true, cx);
      editor.duplicate_lines(false, cx);
      editor.duplicate_lines(true, cx);
      editor.delete_lines(cx);
      editor.insert_line(false, cx);
      editor.insert_line(true, cx);
      editor.toggle_line_comments(cx).expect("read-only no-op");
    });
    assert_eq!(text(&editor, cx), "a\nb\nc");
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
  }
}
