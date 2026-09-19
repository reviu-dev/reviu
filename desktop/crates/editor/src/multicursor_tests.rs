use super::*;
use crate::{actions, editor::tests::EditorTestContext};
use gpui::{ClipboardItem, TestAppContext, VisualTestContext};
use gpui_component::Root;

fn setup<'a>(
  cx: &'a mut TestAppContext,
  text: &str,
  ranges: &[Range<usize>],
) -> (Entity<Editor>, &'a mut VisualTestContext) {
  cx.update(gpui_component::init);
  let editor = EditorTestContext::with_text_and_extension(cx.clone(), text, Some("rs")).editor;
  editor.update(cx, |editor, _| {
    if let Some(first) = ranges.first() {
      editor.selections.primary_mut().range = first.clone();
    }
    for range in ranges.iter().skip(1) {
      editor.selections.add(range.clone(), false);
    }
  });
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

fn ranges(editor: &Entity<Editor>, cx: &VisualTestContext) -> Vec<Range<usize>> {
  editor.read_with(cx, |editor, _| {
    editor
      .selections
      .iter()
      .map(|selection| selection.range.clone())
      .collect()
  })
}

fn input(editor: &Entity<Editor>, input: &str, cx: &mut VisualTestContext) {
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_text_in_range(None, input, window, cx)
    })
  });
}

fn undo(editor: &Entity<Editor>, redo: bool, cx: &mut VisualTestContext) {
  cx.update(|window, cx| editor.update(cx, |editor, cx| editor.undo_edit(redo, window, cx)));
}

#[test]
fn merges_overlaps_and_duplicate_carets_without_merging_adjacent_words() {
  let mut selections = Selections::default();
  selections.primary_mut().range = 1..4;
  selections.add(3..6, true);
  assert_eq!(selections.len(), 1);
  assert_eq!(selections.primary().range, 1..6);
  assert!(selections.primary().reversed);
  selections.add(6..8, false);
  assert_eq!(selections.len(), 2);
  selections.add(8..8, false);
  assert_eq!(selections.len(), 2);
  assert_eq!(selections.primary().range, 6..8);
  selections.add(20..20, false);
  selections.add(20..20, false);
  assert_eq!(selections.len(), 3);
}

#[gpui::test]
fn unicode_batch_restores_every_selection_and_primary_in_one_undo(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "café foo 🦀 foo", &[5..8, 11..14]);
  editor.update(cx, |editor, _| {
    editor.selections.primary_mut().reversed = true
  });
  let before = editor.read_with(cx, |editor, _| editor.selections.clone());
  input(&editor, "é🦀", cx);
  assert_eq!(text(&editor, cx), "café é🦀 🦀 é🦀");
  assert_eq!(ranges(&editor, cx), vec![7..7, 12..12]);
  let after = editor.read_with(cx, |editor, _| editor.selections.clone());
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "café foo 🦀 foo");
  assert_eq!(
    editor.read_with(cx, |editor, _| editor.selections.clone()),
    before
  );
  undo(&editor, true, cx);
  assert_eq!(
    editor.read_with(cx, |editor, _| editor.selections.clone()),
    after
  );
}

#[gpui::test]
fn occurrences_wrap_skip_partial_words_and_select_all_keeps_primary(cx: &mut TestAppContext) {
  let (editor, cx) = setup(
    cx,
    "abc\nabc abc\ndefabc\nabc",
    std::slice::from_ref(&(4..4)),
  );
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.select_occurrences(false, window, cx);
      assert_eq!(editor.selections.primary().range, 4..7);
      editor.select_occurrences(false, window, cx);
      assert_eq!(editor.selections.primary().range, 8..11);
      editor.select_occurrences(false, window, cx);
      assert_eq!(editor.selections.primary().range, 19..22);
      editor.select_occurrences(false, window, cx);
      assert_eq!(editor.selections.primary().range, 0..3);
      editor.select_occurrences(false, window, cx);
      assert_eq!(editor.selections.len(), 4);
    })
  });
}

#[gpui::test]
fn explicit_occurrences_preserve_direction_and_primary(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "foo foo food FOO foo", std::slice::from_ref(&(4..7)));
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.selections.primary_mut().reversed = true;
      let primary = editor.selections.primary().id;
      editor.select_occurrences(true, window, cx);
      assert_eq!(editor.selections.len(), 4);
      assert_eq!(editor.selections.primary().id, primary);
      assert!(editor.selections.iter().all(|selection| selection.reversed));
    })
  });
}

#[gpui::test]
fn visual_columns_survive_short_lines_tabs_and_multibyte_text(cx: &mut TestAppContext) {
  let (editor, cx) = setup(
    cx,
    "Hällö\nx\nHallo\n\tHallo",
    std::slice::from_ref(&(3..3)),
  );
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor
        .document
        .update(cx, |document, _| document.indentation.width = 4);
      editor.add_selection_vertical(1, window, cx);
      editor.add_selection_vertical(1, window, cx);
      assert_eq!(editor.selections.primary().range, 11..11);
      editor.add_selection_vertical(-1, window, cx);
      assert_eq!(editor.selections.len(), 3);
      actions::select_left(editor, &actions::SelectLeft, window, cx);
      assert_eq!(editor.selections.len(), 3);
      assert!(editor.selections.iter().all(|selection| selection.reversed));
      actions::right(editor, &actions::Right, window, cx);
      assert!(
        editor
          .selections
          .iter()
          .all(|selection| selection.range.is_empty())
      );
    })
  });
}

#[gpui::test]
fn batch_newlines_indent_each_context_and_merge_shared_indentation(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "{}\n{}", &[1..1, 4..4]);
  editor.update(cx, |editor, cx| editor.insert_indented_newline(cx));
  assert_eq!(text(&editor, cx), "{\n    \n}\n{\n    \n}");
  undo(&editor, false, cx);
  assert_eq!(ranges(&editor, cx), vec![1..1, 4..4]);
}

#[gpui::test]
fn indentation_deduplicates_shared_lines(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "abcd\nefgh", &[0..1, 2..3, 6..7]);
  editor.update(cx, |editor, cx| editor.indent_selection(false, cx));
  assert_eq!(text(&editor, cx), "    abcd\n    efgh");
  assert_eq!(ranges(&editor, cx), vec![4..5, 6..7, 14..15]);
  editor.update(cx, |editor, cx| editor.indent_selection(true, cx));
  assert_eq!(text(&editor, cx), "abcd\nefgh");
  assert_eq!(ranges(&editor, cx), vec![0..1, 2..3, 6..7]);
}

#[gpui::test]
fn pairs_surround_skip_and_delete_independently(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "\n", &[0..0, 1..1]);
  input(&editor, "(", cx);
  assert_eq!(text(&editor, cx), "()\n()");
  assert_eq!(ranges(&editor, cx), vec![1..1, 4..4]);
  input(&editor, ")", cx);
  assert_eq!(text(&editor, cx), "()\n()");
  assert_eq!(ranges(&editor, cx), vec![2..2, 5..5]);
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "\n");
  input(&editor, "[", cx);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      actions::backspace(editor, &actions::Backspace, window, cx)
    })
  });
  assert_eq!(text(&editor, cx), "\n");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "[]\n[]");
  assert_eq!(ranges(&editor, cx), vec![1..1, 4..4]);
}

#[gpui::test]
fn mixed_pair_contexts_and_surround_preserve_direction(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "alpha beta", &[0..5, 6..10]);
  editor.update(cx, |editor, _| {
    editor.selections.primary_mut().reversed = true
  });
  input(&editor, "(", cx);
  assert_eq!(text(&editor, cx), "(alpha) (beta)");
  assert_eq!(ranges(&editor, cx), vec![1..6, 9..13]);
  assert!(editor.read_with(cx, |editor, _| editor.selections.primary().reversed));
}

#[gpui::test]
fn deletion_merges_overlaps_and_respects_graphemes(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "e\u{301}🦀 e\u{301}🦀", &[3..3, 7..7]);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      actions::backspace(editor, &actions::Backspace, window, cx)
    })
  });
  assert_eq!(text(&editor, cx), "e\u{301} e\u{301}");
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      actions::backspace(editor, &actions::Backspace, window, cx)
    })
  });
  assert_eq!(text(&editor, cx), " ");
}

#[gpui::test]
fn overlapping_deletions_merge_and_undo_restores_cursors(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "abcdef", &[3..3, 5..5]);
  editor.update(cx, |editor, cx| {
    editor.delete_selections(true, false, true, cx)
  });
  assert_eq!(text(&editor, cx), "f");
  assert_eq!(ranges(&editor, cx), vec![0..0]);
  undo(&editor, false, cx);
  assert_eq!(ranges(&editor, cx), vec![3..3, 5..5]);
}

#[gpui::test]
fn clipboard_distributes_lines_and_preserves_multiline_chunks(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "one\ntwo\nthree\nfour", &[0..7, 8..18]);
  editor.update(cx, |editor, cx| editor.copy_to_clipboard(cx));
  editor.update(cx, |editor, cx| editor.paste_from_clipboard(cx));
  assert_eq!(text(&editor, cx), "one\ntwo\nthree\nfour");
  editor.update(cx, |editor, cx| {
    cx.write_to_clipboard(ClipboardItem::new_string("é\n🦀".into()));
    editor.paste_from_clipboard(cx);
  });
  assert_eq!(text(&editor, cx), "one\ntwoé\nthree\nfour🦀");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "one\ntwo\nthree\nfour");
}

#[gpui::test]
fn composition_updates_all_cursors_as_one_undo_transaction(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "a b", &[0..1, 2..3]);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_and_mark_text_in_range(None, "e", Some(1..1), window, cx);
      editor.replace_and_mark_text_in_range(None, "é", Some(1..1), window, cx);
      editor.replace_text_in_range(None, "é", window, cx);
    })
  });
  assert_eq!(text(&editor, cx), "é é");
  assert_eq!(ranges(&editor, cx), vec![1..1, 3..3]);
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "a b");
  assert_eq!(ranges(&editor, cx), vec![0..1, 2..3]);
  undo(&editor, true, cx);
  assert_eq!(text(&editor, cx), "é é");
}

#[gpui::test]
fn readonly_and_old_diff_side_refuse_every_batch_edit(cx: &mut TestAppContext) {
  for old_side in [false, true] {
    let (editor, cx) = setup(cx, "abc abc", &[0..3, 4..7]);
    editor.update(cx, |editor, _| {
      editor.is_read_only = !old_side;
      if old_side {
        editor.selection_view = DiffElementView::SplitLeft;
      }
    });
    input(&editor, "x", cx);
    editor.update(cx, |editor, cx| {
      editor.insert_indented_newline(cx);
      editor.indent_selection(false, cx);
      editor.delete_selections(true, false, false, cx);
      cx.write_to_clipboard(ClipboardItem::new_string("wrong".into()));
      editor.paste_from_clipboard(cx);
    });
    assert_eq!(text(&editor, cx), "abc abc");
  }
}

#[gpui::test]
fn adjacent_occurrences_edit_and_surround_without_losing_a_cursor(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "aaaa", &[0..2, 2..4]);
  input(&editor, "é", cx);
  assert_eq!(text(&editor, cx), "éé");
  assert_eq!(ranges(&editor, cx), vec![1..1, 2..2]);
  undo(&editor, false, cx);
  input(&editor, "(", cx);
  assert_eq!(text(&editor, cx), "(aa)(aa)");
  assert_eq!(ranges(&editor, cx), vec![1..3, 5..7]);
}

#[gpui::test]
fn undo_restores_pairs_when_one_cursor_skips_and_another_inserts(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "\nword", &[0..0, 1..1]);
  input(&editor, "(", cx);
  assert_eq!(text(&editor, cx), "()\n(word");
  input(&editor, ")", cx);
  assert_eq!(text(&editor, cx), "()\n()word");
  undo(&editor, false, cx);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      actions::backspace(editor, &actions::Backspace, window, cx)
    })
  });
  assert_eq!(text(&editor, cx), "\nword");
}

#[gpui::test]
fn adding_vertical_selections_keeps_width_and_direction(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "abcdef\nx\nabcdef", std::slice::from_ref(&(3..4)));
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.selections.primary_mut().reversed = true;
      editor.add_selection_vertical(1, window, cx);
      assert_eq!(editor.selections.len(), 2);
      assert_eq!(editor.selections.primary().range, 12..13);
      assert!(editor.selections.primary().reversed);
    })
  });
}

#[gpui::test]
fn native_explicit_utf16_input_targets_every_selection(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "🦀a 🦀a", &[1..2, 4..5]);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      let range = editor.range_to_utf16(&editor.selections.primary().range, cx);
      editor.replace_text_in_range(Some(range), "é", window, cx);
    })
  });
  assert_eq!(text(&editor, cx), "🦀é 🦀é");
  assert_eq!(ranges(&editor, cx), vec![2..2, 5..5]);
}

#[gpui::test]
fn alt_click_adds_and_plain_click_returns_to_one_selection(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "abc\ndef", std::slice::from_ref(&(1..1)));
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      let click = gpui::MouseDownEvent {
        button: MouseButton::Left,
        modifiers: gpui::Modifiers {
          alt: true,
          ..Default::default()
        },
        click_count: 1,
        ..Default::default()
      };
      editor.begin_mouse_selection(
        DisplayCursor { line: 1, column: 2 },
        DiffElementView::Inline,
        &click,
        window,
        cx,
      );
      assert_eq!(editor.selections.len(), 2);
      assert_eq!(editor.selections.primary().range, 6..6);
      editor.begin_mouse_selection(
        DisplayCursor { line: 0, column: 0 },
        DiffElementView::Inline,
        &gpui::MouseDownEvent {
          modifiers: Default::default(),
          ..click
        },
        window,
        cx,
      );
      assert_eq!(editor.selections.len(), 1);
      assert_eq!(editor.selections.primary().range, 0..0);
    })
  });
}

#[gpui::test]
fn inline_removed_lines_cannot_become_editable_cursors(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "a\nb", &[0..0, 2..2]);
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      let doc = |doc_line| DisplayLine::Doc {
        doc_line,
        old_line: Some(doc_line),
        change: None,
        hunk: None,
        group_id: None,
        secondary: false,
      };
      editor.projection = Some(Arc::new(Projection::from_lines(
        2,
        vec![
          doc(0),
          DisplayLine::Removed {
            text: "removed".into(),
            old_line: 1,
            anchor_line: 0,
            hunk: HunkState::Unstaged,
            group_id: None,
            secondary: false,
          },
          doc(1),
        ],
        HashMap::new(),
        None,
        None,
      )));
      actions::up(editor, &actions::Up, window, cx);
      assert_eq!(editor.selections.len(), 2);
      assert_eq!(editor.selections.primary().range, 2..2);
      editor.replace_text_in_range(None, "x", window, cx);
    })
  });
  assert_eq!(text(&editor, cx), "xa\nxb");
}

#[gpui::test]
fn occurrences_reveal_hidden_diff_context_without_losing_selections(cx: &mut TestAppContext) {
  for count in [60, 6000] {
    let middle = count / 2;
    let original = (0..count)
      .map(|row| {
        if row == 0 || row == middle {
          "needle\n"
        } else {
          "context\n"
        }
      })
      .collect::<String>();
    let modified = original.replacen("needle", "needle changed", 1);
    let (editor, cx) = setup(cx, &modified, std::slice::from_ref(&(0..6)));
    editor.update(cx, |editor, cx| {
      editor.diffs = Some(
        git::compute_buffer_diffs(
          &GitFileBases {
            head: Some(original.clone()),
            index: Some(original),
          },
          &modified,
          Path::new("test.rs"),
        )
        .expect("diff")
        .into(),
      );
      editor.rebuild_projection(cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
      editor.update(cx, |editor, cx| {
        assert!(editor.doc_to_display_line(middle).is_none());
        editor.select_occurrences(true, window, cx);
      })
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
      assert!(editor.doc_to_display_line(middle).is_some());
      assert_eq!(editor.selections.len(), 2);
      assert_eq!(editor.selections.primary().range, 0..6);
      assert!(!editor.is_dirty);
    });
  }
}

#[gpui::test]
fn escape_retains_primary_before_propagating(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "abc abc", &[0..3, 4..7]);
  editor.update(cx, |editor, cx| {
    assert!(editor.retain_primary_selection(cx));
    assert_eq!(editor.selections.primary().range, 4..7);
    assert_eq!(editor.selections.len(), 1);
    assert!(!editor.retain_primary_selection(cx));
  });
}
