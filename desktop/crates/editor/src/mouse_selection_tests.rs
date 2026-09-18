use super::*;
use crate::{
  editor::tests::EditorTestContext, editor_element::EditorElement, gutter_element::GutterElement,
};
use gpui::{Modifiers, Render, TestAppContext, VisualTestContext};

struct SelectionTestView {
  editor: Entity<Editor>,
  split: bool,
  overlay: bool,
}

impl Render for SelectionTestView {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let pane = |left| {
      div()
        .w(px(300.0))
        .h(px(120.0))
        .flex()
        .overflow_hidden()
        .child(div().w(px(50.0)).h_full().flex_shrink_0().child(if left {
          GutterElement::split_left(self.editor.clone())
        } else if self.split {
          GutterElement::split_right(self.editor.clone())
        } else {
          GutterElement::new(self.editor.clone())
        }))
        .child(
          div()
            .id(if left { "left-text" } else { "right-text" })
            .flex_1()
            .h_full()
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
      .relative()
      .flex()
      .text_size(px(14.0))
      .child(pane(self.split))
      .when(self.split, |element| element.child(pane(false)))
      .when(self.overlay, |element| {
        element.child(
          div()
            .id("overlay")
            .absolute()
            .left(px(100.0))
            .top(px(40.0))
            .w(px(180.0))
            .h(px(60.0))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_move(|_, _, cx| cx.stop_propagation()),
        )
      })
  }
}

fn setup<'a>(
  cx: &'a mut TestAppContext,
  text: &str,
) -> (
  Entity<Editor>,
  Entity<SelectionTestView>,
  &'a mut VisualTestContext,
) {
  cx.update(gpui_component::init);
  let editor = EditorTestContext::with_text(cx.clone(), text).editor;
  editor.update(cx, |editor, _| {
    editor.max_line_width = px(250.0);
  });
  let test_editor = editor.clone();
  let (view, cx) = cx.add_window_view(move |_, _| SelectionTestView {
    editor: test_editor,
    split: false,
    overlay: false,
  });
  (editor, view, cx)
}

fn position(
  editor: &Entity<Editor>,
  line: usize,
  column: usize,
  cx: &VisualTestContext,
) -> Point<Pixels> {
  editor.read_with(cx, |editor, _| {
    let map = editor
      .selection_position_map
      .as_ref()
      .expect("painted position map");
    let text = map.line_texts.get(&line).expect("visible line text");
    let byte = char_offset_to_byte_offset(text, column);
    let shaped = &map
      .shaped_lines
      .iter()
      .find(|(row, _)| *row == line)
      .expect("shaped line")
      .1;
    point(
      map.bounds.left() + shaped.x_for_index(byte),
      map.bounds.top() + map.line_height * (line as f32 - map.scroll_offset + 0.5),
    )
  })
}

fn press(cx: &mut VisualTestContext, position: Point<Pixels>, click_count: usize) {
  cx.simulate_event(MouseDownEvent {
    position,
    button: MouseButton::Left,
    click_count,
    ..Default::default()
  });
}

fn drag(cx: &mut VisualTestContext, position: Point<Pixels>) {
  cx.simulate_mouse_move(position, Some(MouseButton::Left), Modifiers::none());
}

fn release(cx: &mut VisualTestContext, position: Point<Pixels>) {
  cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::none());
}

fn copied(editor: &Entity<Editor>, cx: &VisualTestContext) -> Option<String> {
  editor.read_with(cx, |editor, cx| editor.selected_text_for_copy(cx))
}

#[gpui::test]
fn drag_and_release_over_an_occluding_control_do_not_get_stuck(cx: &mut TestAppContext) {
  let (editor, view, cx) = setup(cx, "alpha\nbravo\ncharlie\ndelta\necho");
  view.update(cx, |view, cx| {
    view.overlay = true;
    cx.notify();
  });
  cx.run_until_parked();
  let start = position(&editor, 0, 1, cx);
  press(cx, start, 1);
  let over_control = point(px(140.0), position(&editor, 3, 0, cx).y);
  drag(cx, over_control);
  assert!(
    copied(&editor, cx)
      .expect("selected text")
      .contains("bravo\ncharlie\n")
  );
  release(cx, over_control);
  assert!(!editor.read_with(cx, |editor, _| editor.is_selecting));
  let selection = copied(&editor, cx);
  cx.simulate_mouse_move(start, None, Modifiers::none());
  assert_eq!(copied(&editor, cx), selection);
  press(cx, over_control, 1);
  assert!(!editor.read_with(cx, |editor, _| editor.is_selecting));
  release(cx, over_control);
}

#[gpui::test]
fn selection_tracks_scroll_without_another_mouse_move(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine");
  let start = position(&editor, 0, 1, cx);
  press(cx, start, 1);
  let pointer = position(&editor, 2, 2, cx);
  drag(cx, pointer);
  editor.update(cx, |editor, cx| {
    editor.scroll_offset_y = 2.0;
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    copied(&editor, cx)
      .expect("selected text")
      .contains("three\nfour\n")
  );
  release(cx, pointer);
}

#[gpui::test]
fn drag_clamps_to_text_at_top_and_right_edges(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "alpha\nbravo\ncharlie");
  let start = position(&editor, 1, 2, cx);
  press(cx, start, 1);
  let outside = point(px(600.0), px(-20.0));
  drag(cx, outside);
  assert_eq!(copied(&editor, cx).as_deref(), Some("\nbr"));
  release(cx, outside);
}

#[gpui::test]
async fn horizontal_autoscroll_uses_visible_pane_edges(cx: &mut TestAppContext) {
  let text = "abcdefghijklmnopqrstuvwxyz".repeat(12);
  let (editor, _, cx) = setup(cx, &text);
  editor.update(cx, |editor, cx| {
    editor.max_line_width = px(2400.0);
    cx.notify();
  });
  cx.run_until_parked();
  let start = position(&editor, 0, 1, cx);
  press(cx, start, 1);
  let outside = point(px(380.0), start.y);
  drag(cx, outside);
  let before = copied(&editor, cx).expect("selected text").len();
  for _ in 0..10 {
    cx.background_executor
      .timer(Duration::from_millis(20))
      .await;
    cx.run_until_parked();
  }
  assert!(editor.read_with(cx, |editor, _| editor.scroll_handle.offset().x) < px(0.0));
  assert!(copied(&editor, cx).expect("selected text").len() > before);
  let right_scroll = editor.read_with(cx, |editor, _| editor.scroll_handle.offset().x);
  let gutter = point(px(5.0), start.y);
  drag(cx, gutter);
  for _ in 0..10 {
    cx.background_executor
      .timer(Duration::from_millis(20))
      .await;
    cx.run_until_parked();
  }
  assert!(editor.read_with(cx, |editor, _| editor.scroll_handle.offset().x) > right_scroll);
  release(cx, gutter);
}

#[gpui::test]
fn missing_mouse_up_is_recovered_on_unpressed_move(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "alpha bravo");
  let start = position(&editor, 0, 1, cx);
  press(cx, start, 1);
  let end = position(&editor, 0, 5, cx);
  drag(cx, end);
  cx.simulate_mouse_move(start, None, Modifiers::none());
  assert!(!editor.read_with(cx, |editor, _| editor.is_selecting));
  assert_eq!(copied(&editor, cx).as_deref(), Some("lpha"));
}

#[gpui::test]
fn text_drag_continues_through_gutter_and_reverses(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "alpha\nbravo\ncharlie\ndelta\necho");
  let anchor = position(&editor, 1, 2, cx);
  press(cx, anchor, 1);
  let mut gutter = position(&editor, 3, 0, cx);
  gutter.x = px(20.0);
  drag(cx, gutter);
  assert_eq!(copied(&editor, cx).as_deref(), Some("avo\ncharlie\n"));
  assert_eq!(
    editor.read_with(cx, |editor, _| editor.selected_range.clone()),
    8..20
  );
  gutter.y = position(&editor, 0, 0, cx).y;
  drag(cx, gutter);
  assert_eq!(copied(&editor, cx).as_deref(), Some("alpha\nbr"));
  let end = position(&editor, 2, 3, cx);
  drag(cx, end);
  assert_eq!(copied(&editor, cx).as_deref(), Some("avo\ncha"));
  release(cx, end);
  cx.simulate_mouse_move(anchor, None, Modifiers::none());
  assert_eq!(copied(&editor, cx).as_deref(), Some("avo\ncha"));
}

#[gpui::test]
fn double_click_selects_whitespace_and_line_end_words(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "alpha   bravo");
  let whitespace = position(&editor, 0, 6, cx);
  press(cx, whitespace, 2);
  release(cx, whitespace);
  assert_eq!(copied(&editor, cx).as_deref(), Some("   "));
  let end = position(&editor, 0, 13, cx);
  press(cx, end, 2);
  release(cx, end);
  assert_eq!(copied(&editor, cx).as_deref(), Some("bravo"));
}

#[gpui::test]
fn projection_selection_skips_comment_rows_and_copies_removed_text(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "alpha\ncharlie");
  editor.update(cx, |editor, cx| {
    editor.set_projection(Some(Projection::from_lines(
      2,
      vec![
        DisplayLine::Doc {
          doc_line: 0,
          old_line: Some(0),
          change: None,
          hunk: None,
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
        DisplayLine::Removed {
          text: "bravo".into(),
          old_line: 1,
          anchor_line: 0,
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
      ],
      HashMap::new(),
      None,
      None,
    )));
    cx.notify();
  });
  cx.run_until_parked();
  let start = position(&editor, 0, 0, cx);
  press(cx, start, 1);
  let comment = editor.read_with(cx, |editor, _| {
    let map = editor
      .selection_position_map
      .as_ref()
      .expect("position map");
    point(px(60.0), map.bounds.top() + map.line_height * 1.5)
  });
  drag(cx, comment);
  assert_eq!(copied(&editor, cx).as_deref(), Some("alpha\n"));
  let end = position(&editor, 3, 3, cx);
  drag(cx, end);
  release(cx, end);
  assert_eq!(copied(&editor, cx).as_deref(), Some("alpha\nbravo\ncha"));
  let deleted_word = position(&editor, 2, 2, cx);
  press(cx, deleted_word, 2);
  release(cx, deleted_word);
  assert_eq!(copied(&editor, cx).as_deref(), Some("bravo"));
  press(cx, comment, 1);
  assert!(!editor.read_with(cx, |editor, _| editor.is_selecting));
  release(cx, comment);
}

#[gpui::test]
fn double_click_drag_preserves_words_and_original_anchor(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "zero alpha bravo charlie");
  let start = position(&editor, 0, 7, cx);
  press(cx, start, 2);
  assert_eq!(copied(&editor, cx).as_deref(), Some("alpha"));
  let right = position(&editor, 0, 13, cx);
  drag(cx, right);
  assert_eq!(copied(&editor, cx).as_deref(), Some("alpha bravo"));
  let left = position(&editor, 0, 2, cx);
  drag(cx, left);
  assert_eq!(copied(&editor, cx).as_deref(), Some("zero alpha"));
  drag(cx, start);
  assert_eq!(copied(&editor, cx).as_deref(), Some("alpha"));
  release(cx, start);
}

#[gpui::test]
fn triple_click_handles_empty_and_unterminated_lines(cx: &mut TestAppContext) {
  let (empty_editor, _, empty_cx) = setup(cx, "");
  let empty = point(px(55.0), px(10.0));
  press(empty_cx, empty, 3);
  release(empty_cx, empty);
  assert_eq!(copied(&empty_editor, empty_cx), None);
  assert_eq!(
    empty_editor.read_with(empty_cx, |editor, _| editor.selected_range.clone()),
    0..0
  );

  let (editor, _, cx) = setup(cx, "single line");
  let middle = position(&editor, 0, 5, cx);
  press(cx, middle, 3);
  release(cx, middle);
  assert_eq!(copied(&editor, cx).as_deref(), Some("single line"));
}

#[gpui::test]
fn triple_click_and_gutter_drag_select_whole_lines(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "alpha\nbravo\ncharlie\ndelta\necho");
  let start = position(&editor, 1, 2, cx);
  press(cx, start, 3);
  assert_eq!(copied(&editor, cx).as_deref(), Some("bravo\n"));
  let end = position(&editor, 2, 1, cx);
  drag(cx, end);
  assert_eq!(copied(&editor, cx).as_deref(), Some("bravo\ncharlie\n"));
  let top = position(&editor, 0, 4, cx);
  drag(cx, top);
  assert_eq!(copied(&editor, cx).as_deref(), Some("alpha\nbravo\n"));
  release(cx, top);
  let gutter = point(px(20.0), start.y);
  press(cx, gutter, 1);
  assert_eq!(copied(&editor, cx).as_deref(), Some("bravo\n"));
  drag(cx, end);
  assert_eq!(copied(&editor, cx).as_deref(), Some("bravo\ncharlie\n"));
  release(cx, end);
}

#[gpui::test]
fn shift_click_extends_from_original_anchor(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "alpha bravo charlie");
  let anchor = position(&editor, 0, 8, cx);
  press(cx, anchor, 1);
  let before = position(&editor, 0, 2, cx);
  drag(cx, before);
  release(cx, before);
  let after = position(&editor, 0, 15, cx);
  cx.simulate_mouse_down(
    after,
    MouseButton::Left,
    Modifiers {
      shift: true,
      ..Modifiers::none()
    },
  );
  release(cx, after);
  assert_eq!(copied(&editor, cx).as_deref(), Some("avo cha"));
}

#[gpui::test]
fn quadruple_click_stays_select_all_during_drag(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "alpha\nbravo\n");
  let start = position(&editor, 0, 2, cx);
  press(cx, start, 4);
  drag(cx, position(&editor, 1, 1, cx));
  release(cx, start);
  assert_eq!(copied(&editor, cx).as_deref(), Some("alpha\nbravo\n"));
}

#[gpui::test]
fn unicode_mouse_selection_keeps_character_and_document_ranges_in_sync(cx: &mut TestAppContext) {
  let (editor, _, cx) = setup(cx, "é🙂 alpha\n日本語 fin");
  let start = position(&editor, 0, 1, cx);
  press(cx, start, 1);
  let end = position(&editor, 1, 2, cx);
  drag(cx, end);
  release(cx, end);
  assert_eq!(copied(&editor, cx).as_deref(), Some("🙂 alpha\n日本"));
  assert_eq!(
    editor.read_with(cx, |editor, cx| editor
      .document
      .read(cx)
      .slice_to_string(editor.selected_range.clone())),
    "🙂 alpha\n日本"
  );
}

#[gpui::test]
async fn selection_autoscrolls_with_stationary_pointer_and_stops_on_release(
  cx: &mut TestAppContext,
) {
  let text = (0..100)
    .map(|line| format!("line {line}\n"))
    .collect::<String>();
  let (editor, _, cx) = setup(cx, &text);
  let start = position(&editor, 1, 2, cx);
  press(cx, start, 1);
  let outside = point(px(20.0), px(150.0));
  drag(cx, outside);
  for _ in 0..8 {
    cx.background_executor
      .timer(Duration::from_millis(20))
      .await;
    cx.run_until_parked();
  }
  let scroll = editor.read_with(cx, |editor, _| editor.scroll_offset_y);
  assert!(scroll > 0.0);
  assert!(
    editor.read_with(cx, |editor, _| editor
      .display_selection
      .as_ref()
      .expect("selection")
      .end
      .line)
      > 5
  );
  release(cx, outside);
  cx.background_executor
    .timer(Duration::from_millis(50))
    .await;
  assert_eq!(
    editor.read_with(cx, |editor, _| editor.scroll_offset_y),
    scroll
  );
  assert!(!editor.read_with(cx, |editor, _| editor.is_selecting));
}

#[gpui::test]
fn split_left_copies_old_text_and_cannot_edit_current_document(cx: &mut TestAppContext) {
  let (editor, view, cx) = setup(cx, "new name\nadded\nlast");
  editor.update(cx, |editor, cx| {
    editor.diff_view_mode = super::super::DiffViewMode::Split;
    editor.selection_view = DiffElementView::SplitLeft;
    editor.set_projection(Some(Projection::from_lines(
      3,
      vec![
        DisplayLine::Modified {
          doc_line: 0,
          old_line: 0,
          old_text: "old name".into(),
          hunk: HunkState::Unstaged,
          group_id: None,
          secondary: false,
        },
        DisplayLine::Doc {
          doc_line: 1,
          old_line: None,
          change: Some(ChangeKind::Added),
          hunk: Some(HunkState::Unstaged),
          group_id: None,
          secondary: false,
        },
        DisplayLine::Doc {
          doc_line: 2,
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
    cx.notify();
  });
  view.update(cx, |view, cx| {
    view.split = true;
    cx.notify();
  });
  cx.run_until_parked();
  let start = position(&editor, 0, 0, cx);
  press(cx, start, 1);
  let end = position(&editor, 2, 4, cx);
  drag(cx, end);
  release(cx, end);
  assert_eq!(copied(&editor, cx).as_deref(), Some("old name\nlast"));
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_text_in_range(None, "replacement", window, cx);
      editor.replace_and_mark_text_in_range(None, "ime", None, window, cx);
      editor.find.set_query("new".to_string());
      editor.replace_current_find_match(window, cx);
      editor.replace_all_find_matches(window, cx);
    });
  });
  assert_eq!(
    editor.read_with(cx, |editor, cx| editor
      .document
      .read(cx)
      .slice_to_string(0..editor.document.read(cx).len())),
    "new name\nadded\nlast"
  );
  editor.update(cx, |editor, cx| {
    editor.select_all_display_lines(cx);
  });
  assert_eq!(copied(&editor, cx).as_deref(), Some("old name\nlast"));
  press(cx, start, 1);
  let across_panes = point(px(420.0), start.y);
  drag(cx, across_panes);
  release(cx, across_panes);
  assert_eq!(copied(&editor, cx).as_deref(), Some("old name"));
  assert_eq!(
    editor.read_with(cx, |editor, _| editor.selection_view),
    DiffElementView::SplitLeft
  );
  let right = point(px(355.0), start.y);
  press(cx, right, 2);
  release(cx, right);
  assert_eq!(copied(&editor, cx).as_deref(), Some("new"));
  assert_eq!(
    editor.read_with(cx, |editor, _| editor.selection_view),
    DiffElementView::SplitRight
  );
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_text_in_range(None, "edited", window, cx)
    });
  });
  assert_eq!(
    editor.read_with(cx, |editor, cx| editor
      .document
      .read(cx)
      .slice_to_string(0..editor.document.read(cx).len())),
    "edited name\nadded\nlast"
  );
}
