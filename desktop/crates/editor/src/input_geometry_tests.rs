use super::*;
use crate::editor::tests::EditorTestContext;
use gpui::{Modifiers, PlatformInputHandler, TestAppContext, VisualTestContext};

struct InputTestView {
  editor: Entity<Editor>,
  split: bool,
  font_size: Pixels,
}

impl Render for InputTestView {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let pane = |left| {
      div().w(px(300.0)).h(px(120.0)).pl(px(50.0)).child(
        div()
          .id(if left { "left" } else { "right" })
          .size_full()
          .overflow_x_scroll()
          .track_scroll(&self.editor.read(cx).scroll_handle)
          .child(
            div()
              .min_w(self.editor.read(cx).max_line_width)
              .h_full()
              .child(if left {
                EditorElement::split_left(self.editor.clone())
              } else if self.split {
                EditorElement::split_right(self.editor.clone())
              } else {
                EditorElement::new(self.editor.clone())
              }),
          ),
      )
    };
    div()
      .flex()
      .mt(px(25.0))
      .text_size(self.font_size)
      .when(self.split, |element| element.child(pane(true)))
      .child(pane(false))
  }
}

fn setup<'a>(
  cx: &'a mut TestAppContext,
  text: &str,
) -> (
  Entity<Editor>,
  Entity<InputTestView>,
  &'a mut VisualTestContext,
) {
  cx.update(gpui_component::init);
  let editor = EditorTestContext::with_text(cx.clone(), text).editor;
  let test_editor = editor.clone();
  let (view, cx) = cx.add_window_view(move |_, _| InputTestView {
    editor: test_editor,
    split: false,
    font_size: px(14.0),
  });
  (editor, view, cx)
}

fn map(editor: &Entity<Editor>, cx: &VisualTestContext) -> Rc<PositionMap> {
  editor.read_with(cx, |editor, _| {
    editor
      .selection_position_map
      .clone()
      .expect("painted editor")
  })
}

fn position(map: &PositionMap, line: usize, column: usize) -> Point<Pixels> {
  let text = map.line_texts.get(&line).expect("visible line");
  let shaped = &map
    .shaped_lines
    .iter()
    .find(|(row, _)| *row == line)
    .expect("shaped line")
    .1;
  point(
    map.bounds.left() + shaped.x_for_index(char_offset_to_byte_offset(text, column)),
    map.bounds.top() + map.line_height * (line as f32 - map.scroll_offset),
  )
}

fn bounds(
  editor: &Entity<Editor>,
  range: Range<usize>,
  cx: &mut VisualTestContext,
) -> Option<Bounds<Pixels>> {
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.bounds_for_range(range, Bounds::default(), window, cx)
    })
  })
}

fn index(
  editor: &Entity<Editor>,
  point: Point<Pixels>,
  cx: &mut VisualTestContext,
) -> Option<usize> {
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.character_index_for_point(point, window, cx)
    })
  })
}

#[gpui::test]
fn native_geometry_uses_utf16_and_shaped_unicode_positions(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "é e\u{301} 😀漢字\n\t東京");
  let map = map(&editor, cx);
  for (line, column, utf16) in [
    (0, 0, 0),
    (0, 1, 1),
    (0, 4, 4),
    (0, 6, 7),
    (0, 8, 9),
    (1, 1, 11),
    (1, 3, 13),
  ] {
    let expected = position(&map, line, column);
    let actual = bounds(&editor, utf16..utf16, cx).expect("caret bounds");
    assert_eq!(actual.origin, expected);
    assert_eq!(actual.size.height, map.line_height);
    assert_eq!(actual.size.width, px(1.0));
    assert_eq!(
      index(
        &editor,
        expected + point(px(0.1), map.line_height / 2.0),
        cx
      ),
      Some(utf16)
    );
  }
  let composed = bounds(&editor, 5..9, cx).expect("composed range");
  assert_eq!(composed.left(), position(&map, 0, 5).x);
  assert_eq!(composed.right(), position(&map, 0, 8).x);
  let multiline = bounds(&editor, 5..13, cx).expect("first line of range");
  assert_eq!(multiline, composed);
  assert_eq!(
    bounds(&editor, usize::MAX..usize::MAX, cx),
    bounds(&editor, 13..13, cx)
  );
}

#[gpui::test]
fn native_geometry_tracks_scroll_clipping_and_font_size(cx: &mut TestAppContext) {
  let text = (0..20)
    .map(|_| "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz\n")
    .collect::<String>();
  let (editor, view, cx) = setup(cx, &text);
  editor.update(cx, |editor, cx| {
    editor.scroll_offset_y = 2.5;
    editor.max_line_width = px(700.0);
    editor.scroll_handle.set_offset(point(px(-80.0), px(0.0)));
    cx.notify();
  });
  cx.run_until_parked();
  let scrolled = map(&editor, cx);
  assert_eq!(scrolled.scroll_offset, 2.5);
  assert!(scrolled.bounds.left() < scrolled.viewport_bounds.left());
  let visible = position(&scrolled, 3, 20);
  assert_eq!(
    bounds(&editor, 179..179, cx)
      .expect("scrolled caret")
      .origin,
    visible
  );
  assert_eq!(
    index(
      &editor,
      visible + point(px(0.1), scrolled.line_height / 2.0),
      cx
    ),
    Some(179)
  );
  assert_eq!(bounds(&editor, 0..0, cx), None);
  assert_eq!(bounds(&editor, 159..159, cx), None);
  let clipped = bounds(&editor, 126..126, cx).expect("partially visible line");
  assert_eq!(clipped.top(), scrolled.viewport_bounds.top());
  assert_eq!(clipped.size.height, scrolled.line_height / 2.0);
  for outside in [
    point(scrolled.viewport_bounds.left() - px(1.0), visible.y),
    point(scrolled.viewport_bounds.right() + px(1.0), visible.y),
    point(visible.x, scrolled.viewport_bounds.top() - px(1.0)),
    point(visible.x, scrolled.viewport_bounds.bottom() + px(1.0)),
  ] {
    assert_eq!(index(&editor, outside, cx), None);
  }
  view.update(cx, |view, cx| {
    view.font_size = px(22.0);
    cx.notify();
  });
  cx.run_until_parked();
  let resized = map(&editor, cx);
  assert!(resized.line_height > scrolled.line_height);
  assert_eq!(
    bounds(&editor, 174..174, cx).expect("resized caret").origin,
    position(&resized, 3, 15)
  );
}

fn projected_lines() -> Vec<DisplayLine> {
  vec![
    DisplayLine::Removed {
      text: "old 😀".into(),
      anchor_line: 0,
      old_line: 0,
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
      text: "comment".into(),
      is_header: true,
    },
    DisplayLine::Modified {
      old_text: "old text".into(),
      doc_line: 0,
      old_line: 1,
      hunk: HunkState::Unstaged,
      group_id: None,
      secondary: false,
    },
    DisplayLine::Doc {
      doc_line: 1,
      old_line: Some(2),
      change: None,
      hunk: None,
      group_id: None,
      secondary: false,
    },
  ]
}

#[gpui::test]
fn native_geometry_maps_diff_rows_and_rejects_old_pane(cx: &mut TestAppContext) {
  let (editor, view, cx) = setup(cx, "😀new\nlast");
  editor.update(cx, |editor, cx| {
    editor.set_projection(Some(Projection::from_lines(
      2,
      projected_lines(),
      HashMap::new(),
      None,
      None,
    )));
    cx.notify();
  });
  cx.run_until_parked();
  let inline = map(&editor, cx);
  assert_eq!(
    bounds(&editor, 2..2, cx).expect("projected caret").origin,
    position(&inline, 2, 1)
  );
  for line in [0, 1] {
    let point = point(
      inline.bounds.left() + px(2.0),
      inline.bounds.top() + inline.line_height * (line as f32 + 0.5),
    );
    assert_eq!(index(&editor, point, cx), None);
  }
  view.update(cx, |view, cx| {
    view.split = true;
    cx.notify();
  });
  editor.update(cx, |editor, cx| {
    editor.selection_view = DiffElementView::SplitRight;
    cx.notify();
  });
  cx.run_until_parked();
  let right = map(&editor, cx);
  assert_eq!(right.view, DiffElementView::SplitRight);
  assert!(right.bounds.left() >= px(350.0));
  let caret = bounds(&editor, 2..2, cx).expect("right pane caret");
  assert_eq!(caret.origin, position(&right, 2, 1));
  assert_eq!(
    index(
      &editor,
      caret.origin + point(px(0.1), right.line_height / 2.0),
      cx
    ),
    Some(2)
  );
  let left = point(px(60.0), caret.top() + right.line_height / 2.0);
  assert_eq!(index(&editor, left, cx), None);
  cx.simulate_click(left, Modifiers::none());
  cx.run_until_parked();
  assert_eq!(map(&editor, cx).view, DiffElementView::SplitLeft);
  assert_eq!(bounds(&editor, 2..2, cx), None);
  assert_eq!(index(&editor, left, cx), None);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_and_mark_text_in_range(None, "漢字", None, window, cx);
    })
  });
  assert_eq!(
    editor.read_with(cx, |editor, cx| editor
      .document
      .read(cx)
      .slice_to_string(0..9)),
    "😀new\nlast"
  );
}

#[gpui::test]
fn native_geometry_handles_empty_document_and_deleted_only_diff(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "");
  let empty = bounds(&editor, 0..0, cx).expect("empty caret");
  assert_eq!(
    index(&editor, empty.origin + point(px(1.0), px(1.0)), cx),
    Some(0)
  );
  editor.update(cx, |editor, cx| {
    editor.set_projection(Some(Projection::from_lines(
      1,
      projected_lines().into_iter().take(1).collect(),
      HashMap::new(),
      None,
      None,
    )));
    cx.notify();
  });
  cx.run_until_parked();
  assert_eq!(bounds(&editor, 0..0, cx), None);
  assert_eq!(
    index(&editor, empty.origin + point(px(1.0), px(1.0)), cx),
    None
  );
}

#[gpui::test]
fn native_composition_geometry_preserves_transactions_and_selection(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "😀 ");
  editor.update(cx, |editor, cx| {
    editor.selections.primary_mut().range = 2..2;
    cx.notify();
  });
  for text in ["e\u{301}", "にほん", "日本😀"] {
    cx.update(|window, cx| {
      editor.update(cx, |editor, cx| {
        editor.replace_and_mark_text_in_range(
          None,
          text,
          Some(0..text.encode_utf16().count()),
          window,
          cx,
        );
      })
    });
    cx.run_until_parked();
    let composition =
      bounds(&editor, 3..3 + text.encode_utf16().count(), cx).expect("composition bounds");
    assert_eq!(composition.origin, position(&map(&editor, cx), 0, 2));
    let candidate = cx
      .update(|window, cx| {
        editor.update(cx, |editor, cx| {
          let marked = editor.marked_text_range(window, cx);
          let selected = editor
            .selected_text_range(true, window, cx)
            .expect("selection");
          PlatformInputHandler::compute_ime_candidate_bounds(marked, &selected, |range| {
            editor.bounds_for_range(range, Bounds::default(), window, cx)
          })
        })
      })
      .expect("candidate anchor");
    assert_eq!(candidate.origin, composition.origin);
  }
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_text_in_range(None, "日本😀", window, cx);
      editor.undo_edit(false, window, cx);
      assert_eq!(editor.document.read(cx).slice_to_string(0..2), "😀 ");
      assert_eq!(editor.selections.primary().range, 2..2);
      editor.undo_edit(true, window, cx);
      assert_eq!(editor.document.read(cx).slice_to_string(0..5), "😀 日本😀");
      editor.selections.primary_mut().range = 1..5;
      editor.selections.primary_mut().reversed = true;
      editor.bounds_for_range(1..7, Bounds::default(), window, cx);
      let selection = editor
        .selected_text_range(true, window, cx)
        .expect("selection");
      assert_eq!(selection.range, 2..7);
      assert!(selection.reversed);
    })
  });
}
