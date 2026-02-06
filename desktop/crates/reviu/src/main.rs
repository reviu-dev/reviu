use app_root::AppRoot;
use editor::*;
use gpui::{
  App, Application, Bounds, Focusable, KeyBinding, WindowBounds, WindowOptions, prelude::*, px,
  size,
};
use gpui_component::Root;
use std::sync::mpsc;
use std::time::Duration;
use ui::AppAssets;
use workspace::{
  AuthCallbackTarget, CommitChanges, OpenRepository, ShowCommandPalette, ShowFileSearch,
  WorkspaceView,
};

mod app_root;
const INITIAL_WINDOW_WIDTH: f32 = 1200.0;
const INITIAL_WINDOW_HEIGHT: f32 = 800.0;
const REVIU_URL_SCHEME: &str = "reviu";

fn main() {
  let (open_url_tx, open_url_rx) = mpsc::channel::<Vec<String>>();
  let app = Application::new().with_assets(AppAssets);
  app.on_open_urls({
    let open_url_tx = open_url_tx.clone();
    move |urls| {
      let _ = open_url_tx.send(urls);
    }
  });

  app.run(move |cx: &mut App| {
    gpui_component::init(cx);

    let bounds = Bounds::centered(
      None,
      size(px(INITIAL_WINDOW_WIDTH), px(INITIAL_WINDOW_HEIGHT)),
      cx,
    );

    cx.bind_keys([
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
      KeyBinding::new("cmd-s", Save, None),
      KeyBinding::new("cmd-enter", CommitChanges, None),
      KeyBinding::new("cmd-o", OpenRepository, None),
      KeyBinding::new("cmd-shift-p", ShowCommandPalette, None),
      KeyBinding::new("cmd-p", ShowFileSearch, None),
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
        |window, cx| {
          let view = cx.new(|cx| WorkspaceView::new(window, cx));
          let app_root = cx.new(|cx| AppRoot::new(view, window, cx));
          window.focus(&app_root.focus_handle(cx), cx);
          cx.new(|cx| Root::new(app_root, window, cx))
        },
      )
      .unwrap();

    window
      .update(cx, |_, window, cx| {
        window.activate_window();
        cx.activate(true);
      })
      .unwrap();

    let _ = cx.register_url_scheme(REVIU_URL_SCHEME);
    cx.spawn(async move |cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(200))
          .await;
        while let Ok(urls) = open_url_rx.try_recv() {
          let mut codes = Vec::new();
          for url in urls {
            if let Some(code) = extract_auth_code(&url) {
              codes.push(code);
            }
          }
          if codes.is_empty() {
            continue;
          }
          let _ = cx.update(|cx| {
            for code in codes {
              AuthCallbackTarget::handle_auth_code(code, cx);
            }
          });
        }
      }
    })
    .detach();

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
  });
}

fn extract_auth_code(url: &str) -> Option<String> {
  let url = url.strip_prefix(REVIU_URL_SCHEME)?;
  let url = url.strip_prefix("://")?;
  let (path, query) = url.split_once('?')?;
  if path != "auth/callback" {
    return None;
  }
  for pair in query.split('&') {
    let (key, value) = pair.split_once('=')?;
    if key == "code" && !value.is_empty() {
      return Some(value.to_string());
    }
  }
  None
}
