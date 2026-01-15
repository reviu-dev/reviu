use editor::*;
use gpui::{
  App, Application, Bounds, Focusable, KeyBinding, WindowBounds, WindowOptions, prelude::*, px,
  size,
};
use workspace::{OpenRepository, SaveFile, WorkspaceView};

const INITIAL_WINDOW_WIDTH: f32 = 1200.0;
const INITIAL_WINDOW_HEIGHT: f32 = 800.0;

fn main() {
  Application::new().run(|cx: &mut App| {
    let bounds = Bounds::centered(
      None,
      size(px(INITIAL_WINDOW_WIDTH), px(INITIAL_WINDOW_HEIGHT)),
      cx,
    );

    cx.bind_keys([
      KeyBinding::new("cmd-o", OpenRepository, None),
      KeyBinding::new("enter", Enter, None),
      KeyBinding::new("tab", Tab, None),
      KeyBinding::new("backspace", Backspace, None),
      KeyBinding::new("alt-backspace", BackspaceWord, None),
      KeyBinding::new("cmd-backspace", BackspaceAll, None),
      KeyBinding::new("delete", Delete, None),
      KeyBinding::new("up", Up, None),
      KeyBinding::new("down", Down, None),
      KeyBinding::new("left", Left, None),
      KeyBinding::new("alt-left", AltLeft, None),
      KeyBinding::new("cmd-left", CmdLeft, None),
      KeyBinding::new("right", Right, None),
      KeyBinding::new("alt-right", AltRight, None),
      KeyBinding::new("cmd-right", CmdRight, None),
      KeyBinding::new("cmd-up", CmdUp, None),
      KeyBinding::new("cmd-down", CmdDown, None),
      KeyBinding::new("shift-up", SelectUp, None),
      KeyBinding::new("shift-down", SelectDown, None),
      KeyBinding::new("shift-cmd-left", SelectCmdLeft, None),
      KeyBinding::new("shift-cmd-right", SelectCmdRight, None),
      KeyBinding::new("shift-cmd-up", SelectCmdUp, None),
      KeyBinding::new("shift-cmd-down", SelectCmdDown, None),
      KeyBinding::new("shift-left", SelectLeft, None),
      KeyBinding::new("shift-alt-left", SelectWordLeft, None),
      KeyBinding::new("shift-right", SelectRight, None),
      KeyBinding::new("shift-alt-right", SelectWordRight, None),
      KeyBinding::new("cmd-a", SelectAll, None),
      KeyBinding::new("cmd-v", Paste, None),
      KeyBinding::new("cmd-c", Copy, None),
      KeyBinding::new("cmd-x", Cut, None),
      KeyBinding::new("cmd-z", Undo, None),
      KeyBinding::new("cmd-shift-z", Redo, None),
      KeyBinding::new("cmd-s", SaveFile, None),
      KeyBinding::new("home", Home, None),
      KeyBinding::new("end", End, None),
      KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, None),
    ]);

    let window = cx
      .open_window(
        WindowOptions {
          window_bounds: Some(WindowBounds::Windowed(bounds)),
          ..Default::default()
        },
        |_, cx| cx.new(WorkspaceView::new),
      )
      .unwrap();

    window
      .update(cx, |view, window, cx| {
        cx.activate(true);
        window.focus(&view.focus_handle(cx), cx);
      })
      .unwrap();

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
  });
}
