use super::*;
use crate::{actions, editor::tests::EditorTestContext};
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
fn newline_preserves_whitespace_and_line_endings(cx: &mut TestAppContext) {
  for (before, after) in [
    ("  café|", "  café\n  "),
    ("\tlet value = 1;|", "\tlet value = 1;\n\t"),
    ("  first\r\n  second|", "  first\r\n  second\r\n  "),
    ("  hello|   world", "  hello\n  world"),
    ("  |  text", "  \n  text"),
    ("    |text", "\n    text"),
    ("    |", "\n    "),
    ("\t|", "\n\t"),
    ("|", "\n"),
  ] {
    let (editor, cx) = setup(cx, before, None);
    editor.update(cx, |editor, cx| editor.insert_indented_newline(cx));
    assert_eq!(text(&editor, cx), after, "{before:?}");
    undo(&editor, false, cx);
    assert_eq!(text(&editor, cx), before.replace('|', ""));
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
  }
}

#[gpui::test]
fn newline_indents_blocks_and_places_caret_between_paired_delimiters(cx: &mut TestAppContext) {
  for (language, before, after, cursor_prefix) in [
    (
      "rs",
      "fn main() {|}",
      "fn main() {\n    \n}",
      "fn main() {\n    ",
    ),
    ("json", "{|}", "{\n  \n}", "{\n  "),
    (
      "ts",
      "import App from './App.vue\nif (ready) {|}",
      "import App from './App.vue\nif (ready) {\n  \n}",
      "import App from './App.vue\nif (ready) {\n  ",
    ),
    (
      "rs",
      "fn f<'a>() {|}",
      "fn f<'a>() {\n    \n}",
      "fn f<'a>() {\n    ",
    ),
    (
      "rs",
      "let s = r#\"{\"#; if ready {|}",
      "let s = r#\"{\"#; if ready {\n    \n}",
      "let s = r#\"{\"#; if ready {\n    ",
    ),
    (
      "ts",
      "const values = [|  ];",
      "const values = [\n  \n];",
      "const values = [\n  ",
    ),
    (
      "go",
      "func main() {|}",
      "func main() {\n\t\n}",
      "func main() {\n\t",
    ),
    ("py", "if ready:|", "if ready:\n    ", "if ready:\n    "),
    ("yaml", "key:|", "key:\n  ", "key:\n  "),
    (
      "rs",
      "  if ready {|",
      "  if ready {\n    ",
      "  if ready {\n    ",
    ),
    (
      "rs",
      "if ready { // comment|",
      "if ready { // comment\n    ",
      "if ready { // comment\n    ",
    ),
  ] {
    let (editor, cx) = setup(cx, before, Some(language));
    editor.update(cx, |editor, cx| editor.insert_indented_newline(cx));
    assert_eq!(text(&editor, cx), after, "{language}: {before}");
    let cursor = cursor_prefix.chars().count();
    assert_eq!(
      editor.read_with(cx, |editor, _| editor.selected_range.clone()),
      cursor..cursor
    );
    undo(&editor, false, cx);
    assert_eq!(text(&editor, cx), before.replace('|', ""));
    undo(&editor, true, cx);
    assert_eq!(text(&editor, cx), after);
    assert_eq!(
      editor.read_with(cx, |editor, _| editor.selected_range.clone()),
      cursor..cursor
    );
  }
}

#[gpui::test]
fn newline_does_not_treat_comments_or_strings_as_blocks(cx: &mut TestAppContext) {
  for (language, before) in [
    ("rs", "// {|"),
    ("rs", "/* comment\n  {|\n*/"),
    ("rs", "/* comment\n  {|"),
    ("rs", "/* outer /* nested */\n  {|"),
    ("rs", "let value = \"{|\";"),
    ("rs", "let value = \"text {|"),
    ("py", "# note:|"),
    ("py", "value = \"key:|\""),
    ("py", "value = \"\"\"text\n{|\n\"\"\""),
    ("py", "value = \"\"\"text\n{|"),
  ] {
    let (editor, cx) = setup(cx, before, Some(language));
    let (prefix, suffix) = before.split_once('|').expect("cursor");
    let indent = leading_whitespace(prefix.rsplit('\n').next().unwrap_or_default());
    let expected = format!("{prefix}\n{indent}{suffix}");
    editor.update(cx, |editor, cx| editor.insert_indented_newline(cx));
    assert_eq!(text(&editor, cx), expected, "{language}: {before}");
  }
}

#[gpui::test]
fn newline_replaces_selection_and_restores_it_on_undo(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "fn main() {|old}", Some("rs"));
  editor.update(cx, |editor, cx| {
    editor.selected_range = 11..14;
    editor.selection_reversed = true;
    editor.insert_indented_newline(cx);
  });
  assert_eq!(text(&editor, cx), "fn main() {\n    \n}");
  undo(&editor, false, cx);
  editor.read_with(cx, |editor, _| {
    assert_eq!(editor.selected_range, 11..14);
    assert!(editor.selection_reversed);
  });
  assert_eq!(text(&editor, cx), "fn main() {old}");
}

#[gpui::test]
fn disk_reload_redetects_indentation(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|root\n    child", Some("rs"));
  editor.update(cx, |editor, cx| {
    editor.disk_contents = Some(Arc::from("root\n    child"));
    editor.observe_disk_contents(Some(Arc::from("root\n  child")), None, cx);
    assert_eq!(editor.document.read(cx).indentation.width, 2);
    editor.move_to(editor.document.read(cx).len(), cx);
    editor.indent_selection(false, cx);
  });
  assert_eq!(text(&editor, cx), "root\n  child ");
}

#[gpui::test]
fn outdent_empty_buffer_does_not_create_history(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|", None);
  editor.update(cx, |editor, cx| editor.indent_selection(true, cx));
  editor.read_with(cx, |editor, cx| {
    assert!(!editor.is_dirty);
    assert!(!editor.document.read(cx).buffer.can_undo());
  });
}

#[gpui::test]
fn tab_uses_file_style_and_next_stop_instead_of_fixed_spaces(cx: &mut TestAppContext) {
  for (before, after) in [
    ("x|", "x   "),
    ("  one\n  tw|o", "  one\n  tw  o"),
    ("\tone\n\txy|", "\tone\n\txy\t"),
    ("    one\né|", "    one\né   "),
  ] {
    let (editor, cx) = setup(cx, before, None);
    editor.update(cx, |editor, cx| editor.indent_selection(false, cx));
    assert_eq!(text(&editor, cx), after);
    undo(&editor, false, cx);
    assert_eq!(text(&editor, cx), before.replace('|', ""));
  }
}

#[gpui::test]
fn indent_selection_keeps_text_direction_and_excludes_final_line_start(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|é\n  two\nlast", None);
  editor.update(cx, |editor, _| {
    editor.selected_range = 0..8;
    editor.selection_reversed = true;
  });
  editor.update(cx, |editor, cx| editor.indent_selection(false, cx));
  assert_eq!(text(&editor, cx), "  é\n    two\nlast");
  assert!(editor.read_with(cx, |editor, _| editor.selection_reversed));
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "é\n  two\nlast");
  editor.read_with(cx, |editor, _| {
    assert_eq!(editor.selected_range, 0..8);
    assert!(editor.selection_reversed);
  });
  undo(&editor, true, cx);
  editor.update(cx, |editor, cx| editor.indent_selection(true, cx));
  assert_eq!(text(&editor, cx), "é\n  two\nlast");
}

#[gpui::test]
fn single_line_selection_is_indented_not_replaced(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|hello", None);
  editor.update(cx, |editor, cx| {
    editor.selected_range = 1..4;
    editor.indent_selection(false, cx);
  });
  assert_eq!(text(&editor, cx), "    hello");
  assert_eq!(
    editor.read_with(cx, |editor, _| editor.selected_range.clone()),
    5..8
  );
}

#[gpui::test]
fn outdent_handles_partial_indentation_tabs_and_no_op(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "    first\n      second|\n\t  third", None);
  editor.update(cx, |editor, cx| {
    assert_eq!(editor.document.read(cx).indentation.width, 4);
    editor.indent_selection(true, cx);
  });
  assert_eq!(text(&editor, cx), "    first\n    second\n\t  third");
  undo(&editor, false, cx);
  editor.update(cx, |editor, cx| {
    let length = editor.document.read(cx).len();
    editor.selected_range = 0..length;
    editor.indent_selection(true, cx);
  });
  assert_eq!(text(&editor, cx), "first\n    second\n\tthird");
  let version = editor.read_with(cx, |editor, cx| editor.document.read(cx).buffer.version());
  editor.update(cx, |editor, cx| {
    editor.move_to(0, cx);
    editor.indent_selection(true, cx);
  });
  assert_eq!(
    editor.read_with(cx, |editor, cx| editor.document.read(cx).buffer.version()),
    version
  );
}

#[gpui::test]
fn read_only_views_reject_indentation_and_newlines(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|  text", None);
  for split in [false, true] {
    editor.update(cx, |editor, cx| {
      editor.is_read_only = !split;
      editor.selection_view = if split {
        DiffElementView::SplitLeft
      } else {
        DiffElementView::Inline
      };
      editor.insert_indented_newline(cx);
      editor.indent_selection(false, cx);
      editor.indent_selection(true, cx);
    });
    assert_eq!(text(&editor, cx), "  text");
  }
}

#[gpui::test]
fn keyboard_actions_are_separate_undo_steps(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "|", None);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_text_in_range(None, "é", window, cx);
      actions::tab(editor, &actions::Tab, window, cx);
      actions::enter(editor, &actions::Enter, window, cx);
    })
  });
  assert_eq!(text(&editor, cx), "é   \n");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "é   ");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "é");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "");
}
