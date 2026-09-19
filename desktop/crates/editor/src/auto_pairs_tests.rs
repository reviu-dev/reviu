use super::*;
use crate::{actions, editor::tests::EditorTestContext};
use gpui::{ClipboardItem, TestAppContext, VisualTestContext};
use gpui_component::Root;

fn setup<'a>(
  cx: &'a mut TestAppContext,
  marked: &str,
  language: Option<&str>,
) -> (Entity<Editor>, &'a mut VisualTestContext) {
  cx.update(gpui_component::init);
  let (prefix, suffix) = marked.split_once('|').expect("cursor marker");
  let editor =
    EditorTestContext::with_text_and_extension(cx.clone(), &format!("{prefix}{suffix}"), language)
      .editor;
  editor.update(cx, |editor, cx| editor.move_to(prefix.chars().count(), cx));
  let view = editor.clone();
  let (_, cx) = cx.add_window_view(move |window, cx| Root::new(view, window, cx));
  (editor, cx)
}

fn input(editor: &Entity<Editor>, text: &str, cx: &mut VisualTestContext) {
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_text_in_range(None, text, window, cx)
    })
  });
}

fn undo(editor: &Entity<Editor>, redo: bool, cx: &mut VisualTestContext) {
  cx.update(|window, cx| editor.update(cx, |editor, cx| editor.undo_edit(redo, window, cx)));
}

fn backspace(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      actions::backspace(editor, &actions::Backspace, window, cx)
    })
  });
}

fn assert_text(editor: &Entity<Editor>, marked: &str, cx: &VisualTestContext) {
  let (prefix, suffix) = marked.split_once('|').expect("cursor marker");
  editor.read_with(cx, |editor, cx| {
    let document = editor.document.read(cx);
    assert_eq!(
      document.slice_to_string(0..document.len()),
      format!("{prefix}{suffix}")
    );
    assert_eq!(editor.cursor_offset(), prefix.chars().count());
  });
}

#[gpui::test]
fn typescript_typing_accepts_native_explicit_selection_ranges(cx: &mut TestAppContext) {
  for explicit in [false, true] {
    let (editor, cx) = setup(cx, "// 🦀\n|", Some("ts"));
    for character in "const test = (".chars() {
      cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
          let range =
            explicit.then(|| editor.range_to_utf16(&editor.selections.primary().range, cx));
          editor.replace_text_in_range(range, &character.to_string(), window, cx);
        })
      });
    }
    assert_text(&editor, "// 🦀\nconst test = (|)", cx);
    cx.update(|window, cx| {
      editor.update(cx, |editor, cx| {
        let range = explicit.then(|| editor.range_to_utf16(&editor.selections.primary().range, cx));
        editor.replace_text_in_range(range, ")", window, cx);
      })
    });
    assert_text(&editor, "// 🦀\nconst test = ()|", cx);
  }
}

#[gpui::test]
fn native_selected_text_is_surrounded_but_literal_replacements_are_not(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|é🦀", Some("ts"));
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.selections.primary_mut().range = 0..2;
      editor.selections.primary_mut().reversed = true;
      editor.replace_text_in_range(Some(0..3), "(", window, cx);
      assert_eq!(editor.selections.primary().range, 1..3);
      assert!(editor.selections.primary().reversed);
    })
  });
  undo(&editor, false, cx);
  editor.update(cx, |editor, cx| {
    editor.replace_literal_text_in_range(Some(0..3), "(", cx)
  });
  assert_text(&editor, "(|", cx);
}

#[gpui::test]
fn native_input_replacing_another_range_stays_literal(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|abc", Some("ts"));
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_text_in_range(Some(3..3), "(", window, cx);
    })
  });
  assert_text(&editor, "abc(|", cx);
}

#[gpui::test]
fn unfinished_import_does_not_disable_pairs_on_following_typescript_lines(cx: &mut TestAppContext) {
  let before = "import './assets/main.css'\n\n\nimport { createApp } from 'vue'\nimport App from './App.vue\nconsole.log('Hello, Vue 3!')\ncreateApp(App).mount('#app')\n\n const test = |\n\ncosnt test = (a: string) => {\n  console.log(a)\n}";
  let (editor, cx) = setup(cx, before, Some("ts"));
  input(&editor, "(", cx);
  assert_text(&editor, &before.replace('|', "(|)"), cx);
  input(&editor, ")", cx);
  assert_text(&editor, &before.replace('|', "()|"), cx);
}

#[gpui::test]
fn single_line_string_recovery_preserves_real_multiline_contexts(cx: &mut TestAppContext) {
  for language in ["ts", "py"] {
    for quote in ["'", "\""] {
      for newline in ["\n", "\r\n"] {
        let before = format!("value = {quote}unfinished{newline}next = |");
        let (editor, cx) = setup(cx, &before, Some(language));
        input(&editor, "(", cx);
        assert_text(&editor, &before.replace('|', "(|)"), cx);

        let before = format!("value = {quote}continued\\{newline}|");
        let (editor, cx) = setup(cx, &before, Some(language));
        input(&editor, "(", cx);
        assert_text(&editor, &before.replace('|', "(|"), cx);

        let before = format!("value = {quote}continued\\{newline}broken{newline}next = |");
        let (editor, cx) = setup(cx, &before, Some(language));
        input(&editor, "(", cx);
        assert_text(&editor, &before.replace('|', "(|)"), cx);
      }
    }
  }
  for (language, before) in [
    ("ts", "const text = `first\n|"),
    ("ts", "const text = `first\r\n|"),
    ("ts", "/* comment\n|"),
    ("py", "text = '''first\n|"),
    ("py", "text = \"\"\"first\n|"),
    ("rs", "let text = \"first\n|"),
    ("rs", "let text = r#\"first\n|"),
  ] {
    let (editor, cx) = setup(cx, before, Some(language));
    input(&editor, "(", cx);
    assert_text(&editor, &before.replace('|', "(|"), cx);
  }
}

#[gpui::test]
fn pairs_nest_and_skip_only_generated_closers(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|)", None);
  for character in ["(", "[", "{", "é", "}", "]", ")"] {
    input(&editor, character, cx);
  }
  assert_text(&editor, "([{é}])|)", cx);
  input(&editor, ")", cx);
  assert_text(&editor, "([{é}]))|)", cx);
}

#[gpui::test]
fn pair_gestures_and_history_preserve_caret_and_provenance(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "🦀 |", Some("rs"));
  input(&editor, "(", cx);
  assert_text(&editor, "🦀 (|)", cx);
  backspace(&editor, cx);
  assert_text(&editor, "🦀 |", cx);
  undo(&editor, false, cx);
  assert_text(&editor, "🦀 (|)", cx);
  undo(&editor, false, cx);
  assert_text(&editor, "🦀 |", cx);
  undo(&editor, true, cx);
  assert_text(&editor, "🦀 (|)", cx);
  input(&editor, ")", cx);
  assert_text(&editor, "🦀 ()|", cx);
  editor.update(cx, |editor, cx| editor.move_to(3, cx));
  input(&editor, ")", cx);
  assert_text(&editor, "🦀 ()|)", cx);
}

#[gpui::test]
fn backspace_does_not_remove_existing_pairs_or_nonempty_contents(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "(|)", None);
  backspace(&editor, cx);
  assert_text(&editor, "|)", cx);
  input(&editor, "(", cx);
  input(&editor, "é", cx);
  backspace(&editor, cx);
  assert_text(&editor, "(|))", cx);
  backspace(&editor, cx);
  assert_text(&editor, "|)", cx);
}

#[gpui::test]
fn surround_keeps_unicode_selection_direction_and_undo(cx: &mut TestAppContext) {
  for reversed in [false, true] {
    let (editor, cx) = setup(cx, "|é🦀\ntext", Some("ts"));
    editor.update(cx, |editor, _| {
      editor.selections.primary_mut().range = 0..7;
      editor.selections.primary_mut().reversed = reversed;
    });
    input(&editor, "[", cx);
    editor.read_with(cx, |editor, cx| {
      assert_eq!(
        editor.document.read(cx).slice_to_string(0..9),
        "[é🦀\ntext]"
      );
      assert_eq!(editor.selections.primary().range, 1..8);
      assert_eq!(editor.selections.primary().reversed, reversed);
    });
    undo(&editor, false, cx);
    editor.read_with(cx, |editor, _| {
      assert_eq!(editor.selections.primary().range, 0..7);
      assert_eq!(editor.selections.primary().reversed, reversed);
    });
    undo(&editor, true, cx);
    editor.read_with(cx, |editor, _| {
      assert_eq!(editor.selections.primary().range, 1..8);
      assert_eq!(editor.selections.primary().reversed, reversed);
    });
  }
}

#[gpui::test]
fn language_and_surrounding_text_control_pairing(cx: &mut TestAppContext) {
  for (language, before, typed, after) in [
    ("rs", "let x = |;", "(", "let x = (|);"),
    ("rs", "fn f<'a>() |", "{", "fn f<'a>() {|}"),
    ("rs", "let x = |", "'", "let x = '|"),
    ("json", "|", "'", "'|"),
    ("py", "|", "'", "'|'"),
    ("ts", "|", "`", "`|`"),
    ("rs", "|word", "(", "(|word"),
    ("py", "café|", "'", "café'|"),
    ("rs", "// note |", "(", "// note (|"),
    ("rs", "/* note | */", "[", "/* note [| */"),
    (
      "rs",
      "/* outer /* inner */ |",
      "{",
      "/* outer /* inner */ {|",
    ),
    ("py", "# note |", "\"", "# note \"|"),
    ("rs", "let x = \"text |\";", "(", "let x = \"text (|\";"),
    ("rs", "let x = \"text |", "(", "let x = \"text (|"),
    ("py", "s = \"\"\"line\n|", "[", "s = \"\"\"line\n[|"),
    ("rs", "let s = r#\"raw\n|", "(", "let s = r#\"raw\n(|"),
    (
      "rs",
      "let s = r#\"raw\"#; |",
      "(",
      "let s = r#\"raw\"#; (|)",
    ),
    (
      "ts",
      "const pattern = /[()]/; |",
      "(",
      "const pattern = /[()]/; (|)",
    ),
    ("rs", "\\|", "\"", "\\\"|"),
    ("rs", "\\\\|", "\"", "\\\\\"|\""),
    ("py", "value = f|", "\"", "value = f\"|\""),
    ("rs", "let s = r#|", "\"", "let s = r#\"|"),
    ("md", "note |", "\"", "note \"|"),
    ("md", "don't |", "(", "don't (|)"),
    ("yaml", "key: \"incomplete |", "[", "key: \"incomplete [|"),
    ("toml", "# comment |", "(", "# comment (|"),
    ("sql", "-- comment |", "(", "-- comment (|"),
    ("sql", "/* comment |", "(", "/* comment (|"),
    ("lua", "--[[ note\n|", "(", "--[[ note\n(|"),
    ("html", "<!-- note |", "(", "<!-- note (|"),
    ("hs", "{- note |", "(", "{- note (|"),
    ("ml", "(* note |", "(", "(* note (|"),
    ("clj", "'thing |", "(", "'thing (|)"),
  ] {
    let (editor, cx) = setup(cx, before, Some(language));
    input(&editor, typed, cx);
    assert_text(&editor, after, cx);
  }
}

#[gpui::test]
fn escaped_generated_quote_is_inserted_not_skipped(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|", Some("ts"));
  input(&editor, "\"", cx);
  input(&editor, "\\", cx);
  input(&editor, "\"", cx);
  assert_text(&editor, "\"\\\"|\"", cx);
  input(&editor, "\"", cx);
  assert_text(&editor, "\"\\\"\"|", cx);
}

#[gpui::test]
fn paste_explicit_replacement_and_ime_are_literal(cx: &mut TestAppContext) {
  for typed in ["(", "\"", "({hello})"] {
    let (editor, cx) = setup(cx, "|", Some("ts"));
    cx.update(|window, cx| {
      cx.write_to_clipboard(ClipboardItem::new_string(typed.to_string()));
      editor.update(cx, |editor, cx| {
        actions::paste(editor, &actions::Paste, window, cx)
      });
    });
    assert_text(&editor, &format!("{typed}|"), cx);
    undo(&editor, false, cx);
    assert_text(&editor, "|", cx);
  }
  let (editor, cx) = setup(cx, "|🦀", Some("ts"));
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_text_in_range(Some(0..2), "(", window, cx);
    })
  });
  assert_text(&editor, "(|", cx);
  undo(&editor, false, cx);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_and_mark_text_in_range(None, "(", Some(1..1), window, cx);
      editor.replace_text_in_range(None, "\"", window, cx);
    })
  });
  assert_text(&editor, "\"|🦀", cx);
  undo(&editor, false, cx);
  assert_text(&editor, "|🦀", cx);
}

#[gpui::test]
fn paste_replaces_selection_and_composition_without_surrounding(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|old", Some("ts"));
  cx.update(|window, cx| {
    cx.write_to_clipboard(ClipboardItem::new_string("(".to_string()));
    editor.update(cx, |editor, cx| {
      editor.selections.primary_mut().range = 0..3;
      actions::paste(editor, &actions::Paste, window, cx);
    });
  });
  assert_text(&editor, "(|", cx);
  undo(&editor, false, cx);
  cx.update(|window, cx| {
    cx.write_to_clipboard(ClipboardItem::new_string("[".to_string()));
    editor.update(cx, |editor, cx| {
      editor.replace_and_mark_text_in_range(None, "🦀", Some(2..2), window, cx);
      actions::paste(editor, &actions::Paste, window, cx);
    });
  });
  assert_text(&editor, "[|", cx);
  undo(&editor, false, cx);
  assert_text(&editor, "🦀|", cx);
  undo(&editor, false, cx);
  editor.read_with(cx, |editor, cx| {
    assert_eq!(editor.document.read(cx).slice_to_string(0..3), "old");
  });
}

#[gpui::test]
fn pasted_closer_and_bulk_input_are_not_treated_as_typing(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|", None);
  input(&editor, "(", cx);
  cx.update(|window, cx| {
    cx.write_to_clipboard(ClipboardItem::new_string(")".to_string()));
    editor.update(cx, |editor, cx| {
      actions::paste(editor, &actions::Paste, window, cx)
    });
  });
  assert_text(&editor, "()|)", cx);
  input(&editor, ")", cx);
  assert_text(&editor, "())|", cx);
  input(&editor, "([", cx);
  assert_text(&editor, "())([|", cx);
}

#[gpui::test]
fn indentation_and_nested_edits_keep_pair_positions(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|", Some("rs"));
  input(&editor, "{", cx);
  editor.update(cx, |editor, cx| editor.insert_indented_newline(cx));
  input(&editor, "(", cx);
  input(&editor, ")", cx);
  assert_text(&editor, "{\n    ()|\n}", cx);
  let length = editor.read_with(cx, |editor, cx| editor.document.read(cx).len());
  editor.update(cx, |editor, cx| editor.move_to(length - 1, cx));
  input(&editor, "}", cx);
  assert_text(&editor, "{\n    ()\n}|", cx);
}

#[gpui::test]
fn replacing_delimiters_and_external_reload_discard_stale_provenance(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|", None);
  input(&editor, "(", cx);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_text_in_range(Some(1..2), "x", window, cx);
      editor.replace_text_in_range(Some(1..2), ")", window, cx);
      editor.move_to(1, cx);
    })
  });
  input(&editor, ")", cx);
  assert_text(&editor, "()|)", cx);
  undo(&editor, false, cx);
  editor.update(cx, |editor, cx| {
    editor.disk_contents = Some(Arc::from(""));
    editor.observe_disk_contents(Some(Arc::from("()")), None, cx);
    editor.reload_changed_file(cx);
    editor.move_to(1, cx);
  });
  input(&editor, ")", cx);
  assert_text(&editor, "()|)", cx);
}

#[gpui::test]
fn read_only_and_old_diff_side_reject_pair_gestures(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|", None);
  input(&editor, "(", cx);
  for split in [false, true] {
    editor.update(cx, |editor, _| {
      editor.is_read_only = !split;
      editor.selection_view = if split {
        DiffElementView::SplitLeft
      } else {
        DiffElementView::Inline
      };
    });
    input(&editor, "[", cx);
    input(&editor, ")", cx);
    backspace(&editor, cx);
    assert_text(&editor, "(|)", cx);
  }
}
