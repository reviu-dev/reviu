use super::*;
use crate::{actions, editor::tests::EditorTestContext};
use gpui::{TestAppContext, VisualTestContext};
use gpui_component::Root;

fn setup<'a>(
  cx: &'a mut TestAppContext,
  text: &str,
) -> (Entity<Editor>, &'a mut VisualTestContext) {
  cx.update(gpui_component::init);
  let editor = EditorTestContext::with_text(cx.clone(), text).editor;
  let view = editor.clone();
  let (_, cx) = cx.add_window_view(move |window, cx| Root::new(view, window, cx));
  (editor, cx)
}

#[gpui::test]
fn vertical_motion_preserves_visual_goal_across_short_lines_and_tabs(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "    abcd\nx\n\tabcd\n    abcd");
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor
        .document
        .update(cx, |document, _| document.indentation.width = 4);
      editor.move_to(6, cx);
      actions::down(editor, &actions::Down, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor { line: 1, column: 1 })
      );
      actions::down(editor, &actions::Down, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor { line: 2, column: 3 })
      );
      actions::down(editor, &actions::Down, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor { line: 3, column: 6 })
      );
      actions::select_up(editor, &actions::SelectUp, window, cx);
      assert!(editor.selections.primary().reversed);
      actions::select_down(editor, &actions::SelectDown, window, cx);
      assert!(editor.selections.primary().range.is_empty());
    })
  });
}

#[gpui::test]
fn pages_use_viewport_height_and_keep_selection_anchor(cx: &mut TestAppContext) {
  let text = (0..100).map(|_| "abcdef\n").collect::<String>();
  let (editor, cx) = setup(cx, &text);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.viewport_height = px(200.0);
      editor.editor_line_height = px(20.0);
      editor.move_to(3, cx);
      actions::select_page_down(editor, &actions::SelectPageDown, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor { line: 9, column: 3 })
      );
      assert_eq!(editor.selections.primary().range, 3..66);
      assert_eq!(editor.scroll_offset_y, 9.0);
      actions::select_page_up(editor, &actions::SelectPageUp, window, cx);
      assert_eq!(editor.selections.primary().range, 3..3);
      assert_eq!(editor.scroll_offset_y, 0.0);
      editor.viewport_height = px(100.0);
      actions::page_down(editor, &actions::PageDown, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor { line: 4, column: 3 })
      );
    })
  });
}

fn doc_line(doc_line: usize, old_line: Option<usize>) -> DisplayLine {
  DisplayLine::Doc {
    doc_line,
    old_line,
    change: old_line.is_none().then_some(ChangeKind::Added),
    hunk: None,
    group_id: None,
    secondary: false,
  }
}

fn mixed_projection() -> Projection {
  Projection::from_lines(
    6,
    vec![
      doc_line(0, Some(0)),
      DisplayLine::Removed {
        text: "deleted".into(),
        anchor_line: 0,
        old_line: 1,
        hunk: HunkState::Unstaged,
        group_id: None,
        secondary: false,
      },
      DisplayLine::ReviewComment {
        id: 1,
        side: ReviewCommentSide::Right,
        group_id: None,
        background: None,
        secondary: false,
        text: "not code".into(),
        is_header: true,
      },
      DisplayLine::Block {
        id: crate::projection::DisplayBlockId::Gap(GapId { start: 1, end: 3 }),
      },
      DisplayLine::Modified {
        old_text: "long old 👩‍💻 text".into(),
        doc_line: 3,
        old_line: 4,
        hunk: HunkState::Unstaged,
        group_id: None,
        secondary: false,
      },
      doc_line(4, None),
      doc_line(5, Some(5)),
    ],
    HashMap::new(),
    None,
    None,
  )
}

#[gpui::test]
fn projected_pages_skip_review_rows_and_folds_but_include_removed_code(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "start\nhidden\nhidden\nn\nnew\nlast");
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.set_projection(Some(mixed_projection()));
      editor.viewport_height = px(80.0);
      editor.editor_line_height = px(20.0);
      editor.move_to(0, cx);
      actions::select_page_down(editor, &actions::SelectPageDown, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor { line: 4, column: 0 })
      );
      assert_eq!(
        editor.selected_text_for_copy(cx).as_deref(),
        Some("start\ndeleted\n")
      );
      actions::select_up(editor, &actions::SelectUp, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor { line: 1, column: 0 })
      );
      actions::select_down(editor, &actions::SelectDown, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor { line: 4, column: 0 })
      );
      actions::select_cmd_up(editor, &actions::SelectCmdUp, window, cx);
      assert!(editor.selections.primary().range.is_empty());
      assert!(editor.undo_stack.is_empty());
    })
  });
}

#[gpui::test]
fn old_side_navigation_uses_old_text_and_never_edits_the_current_file(cx: &mut TestAppContext) {
  let original = "start\nhidden\nhidden\nn\nnew\nlast";
  let (editor, cx) = setup(cx, original);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.set_projection(Some(mixed_projection()));
      editor.selection_view = DiffElementView::SplitLeft;
      editor.set_display_cursor(DisplayCursor { line: 4, column: 0 }, cx);
      actions::cmd_right(editor, &actions::CmdRight, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor {
          line: 4,
          column: 17
        })
      );
      actions::select_cmd_left(editor, &actions::SelectCmdLeft, window, cx);
      assert_eq!(
        editor.selected_text_for_copy(cx).as_deref(),
        Some("long old 👩‍💻 text")
      );
      editor.replace_text_in_range(None, "replacement", window, cx);
      assert_eq!(
        editor
          .document
          .read(cx)
          .slice_to_string(0..editor.document.read(cx).len()),
        original
      );
      actions::down(editor, &actions::Down, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx).map(|cursor| cursor.line),
        Some(6)
      );
      assert_eq!(editor.selection_view, DiffElementView::SplitLeft);
      actions::up(editor, &actions::Up, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx).map(|cursor| cursor.line),
        Some(4)
      );
      actions::cmd_left(editor, &actions::CmdLeft, window, cx);
      editor.set_display_cursor(
        DisplayCursor {
          line: 4,
          column: 12,
        },
        cx,
      );
      actions::left(editor, &actions::Left, window, cx);
      assert_eq!(
        editor
          .current_display_cursor(cx)
          .map(|cursor| cursor.column),
        Some(9)
      );
      assert!(editor.undo_stack.is_empty());
      assert!(!editor.is_dirty);
    })
  });
}

#[gpui::test]
fn unicode_goal_survives_short_lines_and_resets_after_horizontal_motion(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "ab👩‍💻界cd\nx\nab👩‍💻界cd\nab👩‍💻界cd");
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.move_to(6, cx);
      actions::down(editor, &actions::Down, window, cx);
      actions::down(editor, &actions::Down, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor { line: 2, column: 6 })
      );
      actions::left(editor, &actions::Left, window, cx);
      actions::down(editor, &actions::Down, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor { line: 3, column: 5 })
      );
      actions::page_down(editor, &actions::PageDown, window, cx);
      assert_eq!(
        editor
          .current_display_cursor(cx)
          .map(|cursor| cursor.column),
        Some(8)
      );
      actions::page_up(editor, &actions::PageUp, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor { line: 0, column: 5 })
      );
    })
  });
}

#[gpui::test]
fn visible_cursor_navigation_does_not_move_scroll_offsets(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, &"abcdef\n".repeat(20));
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.viewport_height = px(120.0);
      editor.editor_line_height = px(20.0);
      editor.move_to(7 * 4 + 2, cx);
      editor.scroll_offset_y = 3.0;
      actions::up(editor, &actions::Up, window, cx);
      assert_eq!(editor.scroll_offset_y, 3.0);
      actions::right(editor, &actions::Right, window, cx);
      assert_eq!(editor.scroll_offset_y, 3.0);
      assert_eq!(editor.scroll_handle.offset().x, px(0.0));
    })
  });
}

#[gpui::test]
fn readonly_keyboard_selection_works_without_allowing_typing_or_delete(cx: &mut TestAppContext) {
  let original = "alpha\nbravo\ncharlie";
  let (editor, cx) = setup(cx, original);
  cx.update(|window, cx| {
    cx.bind_keys([
      gpui::KeyBinding::new("shift-down", actions::SelectDown, Some("Editor && !Input")),
      gpui::KeyBinding::new("delete", actions::Delete, Some("Editor && !Input")),
    ]);
    editor.update(cx, |editor, cx| {
      editor.is_read_only = true;
      window.focus(&editor.focus_handle, cx);
      cx.notify();
    });
  });
  cx.run_until_parked();
  cx.simulate_keystrokes("shift-down");
  assert_eq!(
    editor
      .read_with(cx, |editor, cx| editor.selected_text_for_copy(cx))
      .as_deref(),
    Some("alpha\n")
  );
  cx.simulate_keystrokes("delete");
  cx.simulate_input("replacement");
  editor.read_with(cx, |editor, cx| {
    assert_eq!(
      editor
        .document
        .read(cx)
        .slice_to_string(0..editor.document.read(cx).len()),
      original
    );
    assert!(!editor.is_dirty);
    assert!(editor.undo_stack.is_empty());
  });
}
