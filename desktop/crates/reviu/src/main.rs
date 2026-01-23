use editor::*;
use gpui::{
  App, Application, Bounds, Focusable, KeyBinding, WindowBounds, WindowOptions, prelude::*, px,
  size,
};
use ui::{
  AltLeft as InputAltLeft, AltRight as InputAltRight, Backspace as InputBackspace,
  BackspaceAll as InputBackspaceAll, BackspaceWord as InputBackspaceWord,
  CmdDown as InputCmdDown, CmdLeft as InputCmdLeft, CmdRight as InputCmdRight,
  CmdUp as InputCmdUp, Copy as InputCopy, Cut as InputCut, Delete as InputDelete,
  Down as InputDown, End as InputEnd, Home as InputHome, Left as InputLeft,
  Paste as InputPaste, Right as InputRight, SelectAll as InputSelectAll,
  SelectCmdDown as InputSelectCmdDown, SelectCmdLeft as InputSelectCmdLeft,
  SelectCmdRight as InputSelectCmdRight, SelectCmdUp as InputSelectCmdUp,
  SelectDown as InputSelectDown, SelectLeft as InputSelectLeft,
  SelectRight as InputSelectRight, SelectUp as InputSelectUp,
  SelectWordLeft as InputSelectWordLeft, SelectWordRight as InputSelectWordRight,
  ShowCharacterPalette as InputShowCharacterPalette, Up as InputUp,
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
      KeyBinding::new("enter", Enter, Some("Editor")),
      KeyBinding::new("tab", Tab, Some("Editor")),
      KeyBinding::new("backspace", Backspace, Some("Editor")),
      KeyBinding::new("alt-backspace", BackspaceWord, Some("Editor")),
      KeyBinding::new("cmd-backspace", BackspaceAll, Some("Editor")),
      KeyBinding::new("delete", Delete, Some("Editor")),
      KeyBinding::new("up", Up, Some("Editor")),
      KeyBinding::new("down", Down, Some("Editor")),
      KeyBinding::new("left", Left, Some("Editor")),
      KeyBinding::new("alt-left", AltLeft, Some("Editor")),
      KeyBinding::new("cmd-left", CmdLeft, Some("Editor")),
      KeyBinding::new("right", Right, Some("Editor")),
      KeyBinding::new("alt-right", AltRight, Some("Editor")),
      KeyBinding::new("cmd-right", CmdRight, Some("Editor")),
      KeyBinding::new("cmd-up", CmdUp, Some("Editor")),
      KeyBinding::new("cmd-down", CmdDown, Some("Editor")),
      KeyBinding::new("shift-up", SelectUp, Some("Editor")),
      KeyBinding::new("shift-down", SelectDown, Some("Editor")),
      KeyBinding::new("shift-cmd-left", SelectCmdLeft, Some("Editor")),
      KeyBinding::new("shift-cmd-right", SelectCmdRight, Some("Editor")),
      KeyBinding::new("shift-cmd-up", SelectCmdUp, Some("Editor")),
      KeyBinding::new("shift-cmd-down", SelectCmdDown, Some("Editor")),
      KeyBinding::new("shift-left", SelectLeft, Some("Editor")),
      KeyBinding::new("shift-alt-left", SelectWordLeft, Some("Editor")),
      KeyBinding::new("shift-right", SelectRight, Some("Editor")),
      KeyBinding::new("shift-alt-right", SelectWordRight, Some("Editor")),
      KeyBinding::new("cmd-a", SelectAll, Some("Editor")),
      KeyBinding::new("cmd-v", Paste, Some("Editor")),
      KeyBinding::new("cmd-c", Copy, Some("Editor")),
      KeyBinding::new("cmd-x", Cut, Some("Editor")),
      KeyBinding::new("cmd-z", Undo, Some("Editor")),
      KeyBinding::new("cmd-shift-z", Redo, Some("Editor")),
      KeyBinding::new("cmd-s", SaveFile, Some("Editor")),
      KeyBinding::new("home", Home, Some("Editor")),
      KeyBinding::new("end", End, Some("Editor")),
      KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some("Editor")),
      KeyBinding::new("backspace", InputBackspace, Some("TextInput")),
      KeyBinding::new("alt-backspace", InputBackspaceWord, Some("TextInput")),
      KeyBinding::new("cmd-backspace", InputBackspaceAll, Some("TextInput")),
      KeyBinding::new("delete", InputDelete, Some("TextInput")),
      KeyBinding::new("up", InputUp, Some("TextInput")),
      KeyBinding::new("down", InputDown, Some("TextInput")),
      KeyBinding::new("left", InputLeft, Some("TextInput")),
      KeyBinding::new("alt-left", InputAltLeft, Some("TextInput")),
      KeyBinding::new("cmd-left", InputCmdLeft, Some("TextInput")),
      KeyBinding::new("alt-cmd-left", InputCmdLeft, Some("TextInput")),
      KeyBinding::new("right", InputRight, Some("TextInput")),
      KeyBinding::new("alt-right", InputAltRight, Some("TextInput")),
      KeyBinding::new("cmd-right", InputCmdRight, Some("TextInput")),
      KeyBinding::new("alt-cmd-right", InputCmdRight, Some("TextInput")),
      KeyBinding::new("cmd-up", InputCmdUp, Some("TextInput")),
      KeyBinding::new("cmd-down", InputCmdDown, Some("TextInput")),
      KeyBinding::new("shift-up", InputSelectUp, Some("TextInput")),
      KeyBinding::new("shift-down", InputSelectDown, Some("TextInput")),
      KeyBinding::new("shift-cmd-up", InputSelectCmdUp, Some("TextInput")),
      KeyBinding::new("shift-cmd-down", InputSelectCmdDown, Some("TextInput")),
      KeyBinding::new("shift-left", InputSelectLeft, Some("TextInput")),
      KeyBinding::new("shift-alt-left", InputSelectWordLeft, Some("TextInput")),
      KeyBinding::new("shift-cmd-left", InputSelectCmdLeft, Some("TextInput")),
      KeyBinding::new("shift-right", InputSelectRight, Some("TextInput")),
      KeyBinding::new("shift-alt-right", InputSelectWordRight, Some("TextInput")),
      KeyBinding::new("shift-cmd-right", InputSelectCmdRight, Some("TextInput")),
      KeyBinding::new("cmd-a", InputSelectAll, Some("TextInput")),
      KeyBinding::new("cmd-v", InputPaste, Some("TextInput")),
      KeyBinding::new("cmd-c", InputCopy, Some("TextInput")),
      KeyBinding::new("cmd-x", InputCut, Some("TextInput")),
      KeyBinding::new("home", InputHome, Some("TextInput")),
      KeyBinding::new("end", InputEnd, Some("TextInput")),
      KeyBinding::new("ctrl-cmd-space", InputShowCharacterPalette, Some("TextInput")),
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
