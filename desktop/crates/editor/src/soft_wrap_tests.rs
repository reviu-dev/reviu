use super::*;
use crate::{
  actions,
  editor::{DisplayCursor, tests::EditorTestContext},
  projection::{DisplayBlockId, GapId, HunkState, Projection, ReviewCommentSide},
  text_offsets::char_offset_to_byte_offset,
};
use gpui::{
  Entity, EntityInputHandler, ParentElement, Render, Styled, TestAppContext, VisualTestContext,
  div, prelude::*,
};
use gpui_component::Root;

struct WrapTestView {
  editor: Entity<Editor>,
  width: Pixels,
}

impl Render for WrapTestView {
  fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
    div().w(self.width).h(px(240.0)).child(self.editor.clone())
  }
}

fn setup<'a>(
  cx: &'a mut TestAppContext,
  text: &str,
) -> (
  Entity<Editor>,
  Entity<WrapTestView>,
  &'a mut VisualTestContext,
) {
  cx.update(gpui_component::init);
  let editor = EditorTestContext::with_text(cx.clone(), text).editor;
  editor.update(cx, |editor, cx| editor.set_soft_wrap(true, cx));
  let view = cx.new(|_| WrapTestView {
    editor: editor.clone(),
    width: px(320.0),
  });
  let root_view = view.clone();
  let (_, cx) = cx.add_window_view(move |window, cx| Root::new(root_view, window, cx));
  cx.run_until_parked();
  (editor, view, cx)
}

fn doc_line(line: usize) -> DisplayLine {
  DisplayLine::Doc {
    doc_line: line,
    old_line: Some(line),
    change: None,
    hunk: None,
    group_id: None,
    secondary: false,
  }
}

#[gpui::test]
fn soft_wrap_geometry_roundtrips_unicode_tabs_and_ime_after_resize(cx: &mut TestAppContext) {
  let text = "hello\t世界 e\u{301} 👩‍💻 and words ".repeat(15);
  let (editor, view, cx) = setup(cx, &text);
  let check_geometry = |cx: &mut VisualTestContext| {
    cx.update(|window, cx| {
      editor.update(cx, |editor, cx| {
        let map = editor
          .selection_position_map
          .clone()
          .expect("painted editor");
        let boundary = editor
          .soft_wrap
          .map
          .boundaries(0, DiffElementView::Inline)
          .get(1)
          .copied()
          .expect("wrapped row");
        let row = editor
          .soft_wrap
          .map
          .cursor_row(0, boundary.column, DiffElementView::Inline);
        let utf16 = text[..boundary.byte].encode_utf16().count();
        let bounds = editor
          .bounds_for_range(utf16..utf16, Bounds::default(), window, cx)
          .expect("IME caret");
        assert_eq!(bounds.left(), map.bounds.left());
        assert_eq!(
          bounds.top(),
          map.bounds.top() + map.line_height * (row as f32 - map.scroll_offset)
        );
        let point = bounds.origin + point(px(0.1), map.line_height / 2.0);
        assert_eq!(
          editor.character_index_for_point(point, window, cx),
          Some(utf16)
        );
        assert_eq!(
          map.display_cursor_for_position(point),
          Some(DisplayCursor {
            line: 0,
            column: boundary.column
          })
        );
        assert_eq!(
          map.point_for_position(point, editor.document.read(cx)),
          Some(boundary.column)
        );
      })
    });
  };
  check_geometry(cx);
  let initial = editor.read_with(cx, |editor, _| editor.visual_line_count(1));
  editor.update(cx, |editor, cx| {
    editor.set_display_selection_with_anchor(
      DisplayCursor {
        line: 0,
        column: 40,
      },
      DisplayCursor { line: 0, column: 4 },
      cx,
    );
  });
  view.update(cx, |view, cx| {
    view.width = px(220.0);
    cx.notify();
  });
  cx.run_until_parked();
  check_geometry(cx);
  editor.read_with(cx, |editor, cx| {
    assert!(editor.visual_line_count(1) > initial);
    assert_eq!(editor.selections.primary().range, 4..40);
    assert!(editor.selections.primary().reversed);
    assert_eq!(
      editor.selected_text_for_copy(cx),
      Some(text.chars().skip(4).take(36).collect())
    );
    assert_eq!(
      editor
        .document
        .read(cx)
        .slice_to_string(0..text.chars().count()),
      text
    );
    assert!(editor.undo_stack.is_empty());
  });
}

#[gpui::test]
fn soft_wrap_vertical_pages_copy_edit_and_undo_use_document_offsets(cx: &mut TestAppContext) {
  let text = format!("{}\nshort\n{}", "a".repeat(600), "b".repeat(600));
  let (editor, _, cx) = setup(cx, &text);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.move_to(2, cx);
      let second = editor.soft_wrap.map.boundaries(0, DiffElementView::Inline)[1].column;
      actions::select_down(editor, &actions::SelectDown, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx),
        Some(DisplayCursor {
          line: 0,
          column: second + 2
        })
      );
      assert_eq!(editor.selected_text_for_copy(cx), Some("a".repeat(second)));
      actions::select_up(editor, &actions::SelectUp, window, cx);
      assert_eq!(editor.selections.primary().range, 2..2);
      editor.viewport_height = editor.measured_editor_line_height() * 4.0;
      actions::select_page_down(editor, &actions::SelectPageDown, window, cx);
      let cursor = editor.current_display_cursor(cx).expect("cursor");
      assert_eq!(
        editor
          .soft_wrap
          .map
          .cursor_row(cursor.line, cursor.column, DiffElementView::Inline),
        3
      );
      actions::select_page_up(editor, &actions::SelectPageUp, window, cx);
      assert_eq!(editor.selections.primary().range, 2..2);
      editor.replace_text_in_range(None, "漢", window, cx);
      editor.sync_soft_wrap(window, cx);
      assert_eq!(editor.document.read(cx).slice_to_string(0..4), "aa漢a");
      actions::undo(editor, &actions::Undo, window, cx);
      editor.sync_soft_wrap(window, cx);
      assert_eq!(
        editor.document.read(cx).slice_to_string(0..text.len()),
        text
      );
      let selection = editor.selections.primary().clone();
      editor.set_soft_wrap(false, cx);
      assert_eq!(editor.visual_line_count(3), 3);
      assert_eq!(editor.selections.primary().range, selection.range);
      assert_eq!(editor.selections.primary().reversed, selection.reversed);
      actions::redo(editor, &actions::Redo, window, cx);
      assert_eq!(editor.document.read(cx).slice_to_string(0..4), "aa漢a");
    })
  });
}

#[gpui::test]
fn soft_wrap_split_alignment_keeps_folds_comments_and_old_side_read_only(cx: &mut TestAppContext) {
  let text = "new\nhidden\nafter";
  let (editor, _, cx) = setup(cx, text);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.set_projection(Some(Projection::from_lines(
        3,
        vec![
          DisplayLine::Modified {
            old_text: "old text ".repeat(30).into(),
            doc_line: 0,
            old_line: 0,
            hunk: HunkState::Unstaged,
            group_id: None,
            secondary: false,
          },
          DisplayLine::Block {
            id: DisplayBlockId::Gap(GapId { start: 1, end: 2 }),
          },
          DisplayLine::ReviewComment {
            id: 1,
            side: ReviewCommentSide::Right,
            group_id: None,
            background: None,
            secondary: false,
            text: "review".into(),
            is_header: true,
          },
          doc_line(2),
        ],
        HashMap::new(),
        None,
        None,
      )));
      editor.sync_soft_wrap(window, cx);
      let map = editor.soft_wrap.map.clone();
      let left_rows = map.boundaries(0, DiffElementView::SplitLeft).len();
      assert!(left_rows > 1);
      assert_eq!(map.boundaries(0, DiffElementView::SplitRight).len(), 1);
      assert_eq!(map.row(1), left_rows);
      assert_eq!(map.row(2), left_rows + 1);
      assert_eq!(map.row(3), left_rows + 2);
      for row in 0..left_rows {
        assert_eq!(map.line(row), 0);
      }
      editor.selection_view = DiffElementView::SplitLeft;
      editor.set_display_cursor(DisplayCursor { line: 0, column: 0 }, cx);
      actions::select_down(editor, &actions::SelectDown, window, cx);
      assert_eq!(
        editor.current_display_cursor(cx).expect("cursor").column,
        map.boundaries(0, DiffElementView::SplitLeft)[1].column
      );
      assert!(
        !editor
          .selected_text_for_copy(cx)
          .expect("old text")
          .contains('\n')
      );
      editor.replace_text_in_range(None, "no", window, cx);
      assert_eq!(
        editor.document.read(cx).slice_to_string(0..text.len()),
        text
      );
      assert!(editor.undo_stack.is_empty());
    })
  });
}

#[gpui::test]
fn soft_wrap_long_line_only_paints_and_hit_tests_visible_fragments(cx: &mut TestAppContext) {
  let text = "a".repeat(100_000);
  let (editor, _, cx) = setup(cx, &text);
  editor.update(cx, |editor, cx| {
    editor.scroll_offset_y = 2000.5;
    cx.notify();
  });
  cx.run_until_parked();
  editor.read_with(cx, |editor, _| {
    let map = editor
      .selection_position_map
      .as_ref()
      .expect("painted viewport");
    assert!(editor.visual_line_count(1) > 2000);
    assert_eq!(map.viewport, 0..1);
    assert_eq!(map.shaped_lines.len(), 1);
    let visible = editor.soft_wrap.map.visible_fragments(
      0,
      DiffElementView::Inline,
      map.scroll_offset,
      map.line_height,
      map.bounds.size.height,
    );
    assert!(visible.len() < 30);
    let point = map.bounds.origin + point(px(0.1), map.line_height);
    let cursor = map
      .display_cursor_for_position(point)
      .expect("wrapped cursor");
    let expected = editor.soft_wrap.map.boundary(2001, DiffElementView::Inline);
    assert_eq!(cursor.column, expected.column);
    assert_eq!(
      char_offset_to_byte_offset(&text, cursor.column),
      expected.byte
    );
  });
}

#[gpui::test]
fn soft_wrap_multicursors_follow_visual_rows_and_keep_boundary_affinities(cx: &mut TestAppContext) {
  let line = "aaaa bbbbbbbbbbbbbbbbbbbb";
  let text = format!("{line}\n{line}");
  let (editor, _, cx) = setup(cx, &text);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      let layout = editor.navigation_layout(0, window, cx).expect("layout");
      editor.viewport_width = layout.x_for_index(1) * 10.0 + px(4.0);
      editor.sync_soft_wrap(window, cx);
      editor.move_to(13, cx);
      editor
        .selections
        .add(line.len() + 1 + 13..line.len() + 1 + 13, false);
      actions::up(editor, &actions::Up, window, cx);
      assert_eq!(
        editor
          .selections
          .iter()
          .map(|selection| selection.head())
          .collect::<Vec<_>>(),
        vec![5, line.len() + 1 + 5]
      );
      assert_eq!(editor.soft_wrap.upstream_selections.len(), 2);
      actions::down(editor, &actions::Down, window, cx);
      assert_eq!(
        editor
          .selections
          .iter()
          .map(|selection| selection.head())
          .collect::<Vec<_>>(),
        vec![13, line.len() + 1 + 13]
      );
      editor.move_to(1, cx);
      actions::add_selection_below(editor, &actions::AddSelectionBelow, window, cx);
      assert_eq!(
        editor
          .selections
          .iter()
          .map(|selection| selection.head())
          .collect::<Vec<_>>(),
        vec![1, 6]
      );
    })
  });
}

#[gpui::test]
fn soft_wrap_incremental_edits_match_full_rebuilds(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, &format!("{}\n{}", "a".repeat(200), "b".repeat(200)));
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      for replacement in ["hello", "\n", "世界", "long words "] {
        editor.move_to(20, cx);
        editor.replace_text_in_range(None, replacement, window, cx);
        editor.sync_soft_wrap(window, cx);
        let incremental = editor.soft_wrap.map.clone();
        editor.soft_wrap.key = None;
        editor.sync_soft_wrap(window, cx);
        assert_eq!(incremental.starts, editor.soft_wrap.map.starts);
        assert_eq!(incremental.lines, editor.soft_wrap.map.lines);
      }
      editor.replace_text_in_range(None, "tracked", window, cx);
      editor.document.update(cx, |document, cx| {
        document.replace(0..150, "external", cx);
      });
      editor.sync_soft_wrap(window, cx);
      let external = editor.soft_wrap.map.clone();
      editor.soft_wrap.key = None;
      editor.sync_soft_wrap(window, cx);
      assert_eq!(external.starts, editor.soft_wrap.map.starts);
      assert_eq!(external.lines, editor.soft_wrap.map.lines);
    })
  });
}

#[gpui::test]
fn soft_wrap_boundary_clicks_keep_the_caret_on_the_clicked_visual_row(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, &"a".repeat(200));
  for upstream in [true, false] {
    let (position, column, expected_row) = editor.read_with(cx, |editor, _| {
      let map = editor
        .selection_position_map
        .as_ref()
        .expect("painted editor");
      let boundary = map.wrap_map.boundaries(0, DiffElementView::Inline)[1];
      let shaped = &map.shaped_lines[0].1;
      let row = usize::from(!upstream);
      (
        point(
          map.bounds.left()
            + if upstream {
              shaped.x_for_index(boundary.byte)
            } else {
              px(0.0)
            },
          map.bounds.top() + map.line_height * (row as f32 + 0.5),
        ),
        boundary.column,
        row,
      )
    });
    cx.simulate_event(gpui::MouseDownEvent {
      position,
      button: gpui::MouseButton::Left,
      click_count: 1,
      ..Default::default()
    });
    cx.simulate_event(gpui::MouseUpEvent {
      position,
      button: gpui::MouseButton::Left,
      ..Default::default()
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
      editor.update(cx, |editor, cx| {
        assert_eq!(editor.cursor_offset(), column);
        let cursor = editor.current_display_cursor(cx).expect("cursor");
        assert_eq!(
          editor.soft_wrap.cursor_row(cursor, editor.selection_view),
          expected_row
        );
        let map = editor
          .selection_position_map
          .as_ref()
          .expect("painted cursor");
        let top = map.bounds.top() + map.line_height * expected_row as f32;
        assert_eq!(
          editor
            .bounds_for_range(column..column, Bounds::default(), window, cx)
            .expect("IME bounds")
            .top(),
          top
        );
      })
    });
  }
}

#[gpui::test]
fn soft_wrap_action_is_available_in_read_only_editors(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, &"text ".repeat(100));
  editor.update(cx, |editor, cx| {
    editor.is_read_only = true;
    cx.notify();
  });
  cx.run_until_parked();
  cx.update(|window, cx| {
    let focus = editor.read(cx).focus_handle.clone();
    focus.focus(window, cx);
    focus.dispatch_action(&actions::ToggleSoftWrap, window, cx);
  });
  cx.run_until_parked();
  assert!(!editor.read_with(cx, |editor, _| editor.soft_wrap_enabled()));
}

#[gpui::test]
fn soft_wrap_search_reveals_later_fragments_without_horizontal_scroll(cx: &mut TestAppContext) {
  let text = format!("{}needle{}", "a".repeat(2500), "b".repeat(200));
  let (editor, _, cx) = setup(cx, &text);
  editor.update(cx, |editor, cx| {
    editor.find.set_query("needle".to_string());
    editor.recompute_find_matches(false, cx);
    editor.select_find_match(0, editor.measured_editor_line_height(), false, cx);
    let found = &editor.find.matches()[0];
    assert_eq!(found.doc_range, 2500..2506);
    let row = editor
      .soft_wrap
      .map
      .cursor_row(0, 2500, DiffElementView::Inline) as f32;
    assert!(editor.scroll_offset_y <= row);
    assert!(
      editor.scroll_offset_y + editor.viewport_height / editor.measured_editor_line_height() > row
    );
    assert_eq!(editor.scroll_handle.offset().x, px(0.0));
  });
  cx.run_until_parked();
  editor.read_with(cx, |editor, _| {
    let map = editor
      .selection_position_map
      .as_ref()
      .expect("painted match");
    assert_eq!(map.viewport, 0..1);
  });
}

#[gpui::test]
fn soft_wrap_large_files_keep_layouts_virtualized_and_reuse_unchanged_wraps(
  cx: &mut TestAppContext,
) {
  let text = (0..50_000)
    .map(|line| format!("line {line}: {}\n", "word ".repeat(20)))
    .collect::<String>();
  let (editor, _, cx) = setup(cx, &text);
  editor.read_with(cx, |editor, _| {
    assert!(editor.visual_line_count(50_001) > 50_001);
    assert!(editor.line_layouts.len() < 100);
  });
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      let before = editor
        .soft_wrap
        .map
        .lines
        .get(&40_000)
        .expect("wrapped line")[1]
        .clone();
      editor.move_to(0, cx);
      editor.replace_text_in_range(None, "changed ", window, cx);
      editor.sync_soft_wrap(window, cx);
      let after = &editor
        .soft_wrap
        .map
        .lines
        .get(&40_000)
        .expect("unchanged wrapped line")[1];
      assert!(Arc::ptr_eq(&before, after));
    })
  });
}
