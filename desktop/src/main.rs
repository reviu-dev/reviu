mod error;
mod git;
mod state;
mod storage;
mod syntax_highlighter;
mod ui;
mod workspace;

use gpui::{
  px, size, App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
};

use workspace::Workspace;

fn main() {
  env_logger::init();

  Application::new().run(|cx: &mut App| {
    // Register workspace actions and keybindings
    Workspace::register(cx);

    let bounds = Bounds::centered(None, size(px(1200.), px(800.0)), cx);
    cx.open_window(
      WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
          title: Some("Reviu".into()),
          appears_transparent: false,
          traffic_light_position: None,
        }),
        ..Default::default()
      },
      |_, cx| cx.new(Workspace::new),
    )
    .unwrap();
    cx.activate(true);
  });
}
