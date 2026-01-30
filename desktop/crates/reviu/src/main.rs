use editor::*;
use gpui::{
  App, Application, Bounds, Focusable, KeyBinding, WindowBounds, WindowOptions, prelude::*, px,
  size,
};
mod app_root;

use app_root::AppRoot;
use gpui_component::Root;
use gpui_component_assets::Assets;
use workspace::{CommitChanges, OpenRepository, SaveFile, ShowCommandPalette, WorkspaceView};

const INITIAL_WINDOW_WIDTH: f32 = 1200.0;
const INITIAL_WINDOW_HEIGHT: f32 = 800.0;

fn main() {
  Application::new().with_assets(Assets).run(|cx: &mut App| {
    gpui_component::init(cx);
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
      KeyBinding::new("cmd-p", ShowCommandPalette, None),
      KeyBinding::new("cmd-enter", CommitChanges, Some("Input")),
      KeyBinding::new("home", Home, Some("Editor")),
      KeyBinding::new("end", End, Some("Editor")),
      KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some("Editor")),
    ]);

    let window = cx
      .open_window(
        WindowOptions {
          window_bounds: Some(WindowBounds::Windowed(bounds)),
          ..Default::default()
        },
        |window, cx| {
          let view = cx.new(|cx| WorkspaceView::new(window, cx));
          let app_root = cx.new(|cx| AppRoot::new(view, window, cx));
          window.focus(&app_root.focus_handle(cx), cx);
          cx.new(|cx| Root::new(app_root, window, cx))
        },
      )
      .unwrap();

    window
      .update(cx, |_, _, cx| {
        cx.activate(true);
      })
      .unwrap();

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
  });
}
