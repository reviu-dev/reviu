use super::*;
use gpui::WeakEntity;
use gpui_component::{
  WindowExt,
  dialog::{DialogFooter, DialogHeader, DialogTitle},
};

fn parse_line(value: &str, line_count: usize) -> Option<usize> {
  let value = value.trim();
  if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
    return None;
  }
  value
    .parse::<usize>()
    .ok()
    .filter(|line| (1..=line_count).contains(line))
    .map(|line| line - 1)
}

struct GoToLineDialog {
  editor: WeakEntity<Editor>,
  input: Entity<InputState>,
  error: Option<String>,
}

impl GoToLineDialog {
  fn new(
    editor: WeakEntity<Editor>,
    line: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let input = cx.new(|cx| {
      let mut input = InputState::new(window, cx).placeholder("Line number in current file");
      input.set_value((line + 1).to_string(), window, cx);
      input.select_all(window, cx);
      input
    });
    cx.subscribe(&input, |this, _, event, cx| {
      if matches!(event, InputEvent::Change) {
        this.error = None;
        cx.notify();
      }
    })
    .detach();
    Self {
      editor,
      input,
      error: None,
    }
  }

  fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let value = self.input.read(cx).value();

    let result = self.editor.update(cx, |editor, cx| {
      let count = editor.document.read(cx).len_lines();
      let Some(line) = parse_line(&value, count) else {
        return Err(format!("Enter a line number from 1 to {count}."));
      };
      editor.go_to_source_line(line, cx);
      Ok(())
    });
    match result {
      Ok(Err(error)) => {
        self.error = Some(error);
        cx.notify();
      }
      Ok(Ok(())) | Err(_) => window.close_dialog(cx),
    }
  }
}

impl Render for GoToLineDialog {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    v_flex()
      .id("go-to-line-dialog")
      .on_action(
        cx.listener(|this, _: &gpui_component::input::Enter, window, cx| this.confirm(window, cx)),
      )
      .on_action(
        cx.listener(|this, _: &gpui_component::dialog::Confirm, window, cx| {
          this.confirm(window, cx)
        }),
      )
      .child(
        DialogHeader::new()
          .p_4()
          .child(DialogTitle::new().child("Go to Line")),
      )
      .child(
        v_flex()
          .px_4()
          .pb_4()
          .gap_2()
          .child(Input::new(&self.input).id("go-to-line-input").w_full())
          .when_some(self.error.clone(), |this, error| {
            this.child(div().text_sm().text_color(cx.theme().danger).child(error))
          }),
      )
      .child(
        DialogFooter::new()
          .px_4()
          .pb_4()
          .justify_end()
          .child(
            Button::new("go-to-line-cancel")
              .label("Cancel")
              .outline()
              .on_click(|_, window, cx| window.close_dialog(cx)),
          )
          .child(
            Button::new("go-to-line-confirm")
              .label("Go")
              .primary()
              .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
          ),
      )
  }
}

impl Editor {
  pub(crate) fn open_go_to_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let line = self.document.read(cx).char_to_line(self.cursor_offset());
    let editor = cx.weak_entity();
    let dialog = cx.new(|cx| GoToLineDialog::new(editor, line, window, cx));
    let focus = dialog.read(cx).input.read(cx).focus_handle(cx);

    window.open_dialog(cx, move |overlay, _, _| {
      overlay.p_0().w(px(360.0)).child(dialog.clone())
    });
    window.on_next_frame(move |window, cx| window.focus(&focus, cx));
  }

  fn go_to_source_line(&mut self, line: usize, cx: &mut Context<Self>) {
    self.vertical_goal_x = None;
    self.selection_view = match self.diff_view_mode {
      DiffViewMode::Inline => DiffElementView::Inline,
      DiffViewMode::Split => DiffElementView::SplitRight,
    };
    let gap = self.block_map.gap_blocks().find_map(|block| {
      block
        .hidden_range
        .as_ref()
        .filter(|range| range.contains(&line))
        .and_then(|_| block.gap_id())
    });
    if let Some(gap) = gap {
      self.pending_navigation_line = Some(line);
      self.expand_gap_down(
        gap,
        line
          .saturating_sub(gap.start)
          .saturating_add(SCROLL_PADDING + 1),
        cx,
      );
    } else {
      self.reveal_source_line(line, cx);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn line_input_is_strict_one_based_and_bounded() {
    for value in [
      "",
      " ",
      "0",
      "-1",
      "+1",
      "1.5",
      "1:2",
      "11",
      "9999999999999999999999999",
      "١",
    ] {
      assert_eq!(parse_line(value, 10), None, "{value}");
    }
    assert_eq!(parse_line(" 1 ", 10), Some(0));
    assert_eq!(parse_line("10", 10), Some(9));
    assert_eq!(parse_line("1", 0), None);
  }

  #[gpui::test]
  fn go_to_line_reveals_folded_source_and_preserves_readonly(cx: &mut gpui::TestAppContext) {
    use crate::editor::tests::EditorTestContext;
    for count in [60, 6000] {
      let original = "context\n".repeat(count);
      let modified = format!("changed\n{}", "context\n".repeat(count - 1));
      let editor = EditorTestContext::with_text(cx.clone(), &modified).editor;
      editor.update(cx, |editor, cx| {
        editor.is_read_only = true;
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
      let target = count / 2;
      editor.update(cx, |editor, cx| {
        assert!(editor.doc_to_display_line(target).is_none());
        editor.go_to_source_line(target, cx);
      });
      cx.run_until_parked();
      editor.read_with(cx, |editor, cx| {
        assert!(editor.pending_navigation_line.is_none());
        let line = editor.doc_to_display_line(target).expect("revealed line");
        assert_eq!(
          editor.current_display_cursor(cx),
          Some(DisplayCursor { line, column: 0 })
        );
        assert_eq!(
          editor
            .document
            .read(cx)
            .char_to_line(editor.cursor_offset()),
          target
        );
        assert!(editor.is_read_only);
        assert!(!editor.is_dirty);
        assert!(editor.undo_stack.is_empty());
      });
    }
  }

  struct DialogHost(Entity<Editor>);

  impl Render for DialogHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
      div()
        .size_full()
        .child(self.0.clone())
        .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
  }

  #[gpui::test]
  fn dialog_validates_without_moving_and_restores_focus_on_confirm_and_cancel(
    cx: &mut gpui::TestAppContext,
  ) {
    use crate::editor::tests::EditorTestContext;
    cx.update(gpui_component::init);
    let editor = EditorTestContext::with_text(cx.clone(), "alpha\nbravo\ncharlie").editor;
    let view = editor.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
      let host = cx.new(|_| DialogHost(view));
      gpui_component::Root::new(host, window, cx)
    });
    cx.update(|window, cx| {
      cx.bind_keys([gpui::KeyBinding::new(
        "ctrl-g",
        crate::actions::GoToLine,
        Some("Editor && !Input"),
      )]);
      let focus = editor.read(cx).focus_handle.clone();
      window.focus(&focus, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-g");
    cx.update(|window, cx| window.simulate_next_frame(cx));
    cx.run_until_parked();
    cx.simulate_input("0");
    cx.simulate_keystrokes("enter");
    assert_eq!(editor.read_with(cx, |editor, _| editor.cursor_offset()), 0);
    cx.update(|window, cx| assert!(!editor.read(cx).focus_handle.is_focused(window)));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("3");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |editor, _| editor.cursor_offset()), 12);
    cx.update(|window, cx| assert!(editor.read(cx).focus_handle.is_focused(window)));
    cx.simulate_keystrokes("ctrl-g");
    cx.update(|window, cx| window.simulate_next_frame(cx));
    cx.run_until_parked();
    cx.simulate_input("1");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |editor, _| editor.cursor_offset()), 12);
    cx.update(|window, cx| assert!(editor.read(cx).focus_handle.is_focused(window)));
  }
}
