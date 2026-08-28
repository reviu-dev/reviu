//! Interactive driver for the real app: mounts `WorkspaceView` in a driver
//! window and takes JSON-lines commands on stdin, one response per line on
//! stdout. Built for agents and humans debugging the UI without the full app.
//!
//! `reviu-perf` wraps this driver with repeatable perf scenarios, a temporary
//! repository, the stub agent and `sample`/`ps` collection.
//!
//! Verbs: `bounds` (test backend only, of a debug selector), `click` (selector
//! on test or point on both), `type`, `key`, `clock` (virtual ms), `wait` (real
//! ms), `park`, `path_prompt`, `screenshot` (visual backend only),
//! `show_changes`, `hide_dock`, `submit_prompt`, `quit`.
//!
//! Usage: `cargo run -p reviu_driver -- --backend test` then e.g.
//! `{"cmd":"bounds","selector":"session-repo-context"}`
//! `{"cmd":"key","keystrokes":"cmd-p"}` `{"cmd":"type","text":"push"}`
//!
//! On macOS, `--backend visual` uses GPUI's real Metal renderer off screen and
//! enables `{"cmd":"screenshot","path":"/tmp/reviu.png"}`.

use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
use std::time::Duration;

#[cfg(target_os = "macos")]
use gpui::{AnyWindowHandle, VisualTestAppContext, size};
use gpui::{
  App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, Render,
  TestAppContext, TestDispatcher, Window, div, prelude::*, px,
};
use gpui_component::Root;
use workspace::WorkspaceView;

/// Same shape as the app's root in `crates/reviu/src/app_root.rs`: the view
/// plus the layers that dialogs, sheets and notifications render into.
struct DriverRoot {
  view: Entity<WorkspaceView>,
  focus_handle: FocusHandle,
}

impl Focusable for DriverRoot {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for DriverRoot {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let sheet_layer = Root::render_sheet_layer(window, cx);
    let dialog_layer = Root::render_dialog_layer(window, cx);
    let notification_layer = Root::render_notification_layer(window, cx);

    div()
      .size_full()
      .child(self.view.clone())
      .children(sheet_layer)
      .children(dialog_layer)
      .children(notification_layer)
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
  Test,
  Visual,
}

impl Backend {
  fn parse(value: &str) -> Result<Self, String> {
    match value {
      "test" => Ok(Self::Test),
      "visual" => Ok(Self::Visual),
      other => Err(format!("unknown backend: {other}")),
    }
  }

  fn as_str(self) -> &'static str {
    match self {
      Self::Test => "test",
      Self::Visual => "visual",
    }
  }
}

#[derive(Debug, PartialEq, Eq)]
struct Args {
  backend: Backend,
  agent_command: Option<String>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
  let mut backend = Backend::Test;
  let mut agent_command = None;
  let mut args = args.into_iter();
  while let Some(arg) = args.next() {
    if let Some(value) = arg.strip_prefix("--backend=") {
      backend = Backend::parse(value)?;
      continue;
    }
    match arg.as_str() {
      "--backend" => {
        let value = args
          .next()
          .ok_or_else(|| "--backend needs test or visual".to_string())?;
        backend = Backend::parse(&value)?;
      }
      "--agent-command" => agent_command = args.next(),
      other => return Err(format!("unknown argument: {other}")),
    }
  }
  Ok(Args {
    backend,
    agent_command,
  })
}

#[derive(serde::Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Command {
  /// Painted bounds of a `debug_selector`, if it was painted last frame.
  Bounds {
    selector: String,
  },
  /// Click a selector's center, or an absolute point.
  Click {
    selector: Option<String>,
    x: Option<f32>,
    y: Option<f32>,
  },
  /// Keystrokes separated by spaces, e.g. "cmd-p" or "down down enter".
  Key {
    keystrokes: String,
  },
  /// Type text into the focused input, one keystroke per character.
  Type {
    text: String,
  },
  /// Advance the virtual clock (timers, debounces).
  Clock {
    ms: u64,
  },
  /// Let real time pass (a real agent process answering, network).
  Wait {
    ms: u64,
  },
  /// Run scheduled work to quiescence.
  Park,
  /// Test backend: answer the next OS folder prompt. Visual backend: open it directly.
  PathPrompt {
    path: String,
  },
  /// Capture the visual backend's rendered window as a PNG.
  Screenshot {
    path: String,
  },
  /// Direct driver hook for perf runs: open the Changes dock tab.
  ShowChanges,
  /// Direct driver hook for perf runs: close the right dock.
  HideDock,
  /// Direct driver hook for perf runs: fill and submit the agent composer.
  SubmitPrompt {
    text: String,
  },
  Quit,
}

fn respond(value: serde_json::Value) {
  let mut stdout = std::io::stdout().lock();
  let _ = writeln!(stdout, "{value}");
  let _ = stdout.flush();
}

fn ok(extra: serde_json::Value) -> serde_json::Value {
  let mut base = serde_json::json!({ "ok": true });
  if let (Some(base_map), Some(extra_map)) = (base.as_object_mut(), extra.as_object()) {
    for (key, value) in extra_map {
      base_map.insert(key.clone(), value.clone());
    }
  }
  base
}

fn err(message: impl std::fmt::Display) -> serde_json::Value {
  serde_json::json!({ "ok": false, "error": message.to_string() })
}

/// `debug_bounds` wants a `'static` selector; interning keeps repeated queries
/// from growing the leak.
fn intern_selector(selector: &str) -> &'static str {
  use std::collections::HashSet;
  use std::sync::Mutex;
  static INTERNED: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
  let mut interned = INTERNED.lock().expect("lock interned selectors");
  let set = interned.get_or_insert_with(HashSet::new);
  match set.get(selector) {
    Some(existing) => existing,
    None => {
      let leaked: &'static str = Box::leak(selector.to_string().into_boxed_str());
      set.insert(leaked);
      leaked
    }
  }
}

fn init_app(cx: &mut App) {
  gpui_component::init(cx);
  ui::init(cx);
  if let Some(dir) = agent_registry::icon_cache_dir() {
    ui::set_runtime_asset_dir(dir);
  }
  let theme = gpui_component::Theme::global_mut(cx);
  theme.font_family = "Inter".into();
  theme.mono_font_family = "Lilex".into();
}

fn build_root(window: &mut Window, cx: &mut Context<Root>) -> (Root, Entity<WorkspaceView>) {
  let view = cx.new(|cx| WorkspaceView::new(window, cx));
  let focus_handle = cx.focus_handle();
  let driver_root = cx.new(|_| DriverRoot {
    view: view.clone(),
    focus_handle: focus_handle.clone(),
  });
  window.focus(&focus_handle, cx);
  (Root::new(driver_root, window, cx), view)
}

fn main() {
  let args = match parse_args(std::env::args().skip(1)) {
    Ok(args) => args,
    Err(error) => {
      eprintln!("{error}");
      std::process::exit(2);
    }
  };

  // Without an override the shell connects the real configured agent.
  agent_chat_panel::set_backend_command_override(args.agent_command);

  match args.backend {
    Backend::Test => run_test_backend(),
    Backend::Visual => run_visual_backend(),
  }
}

fn run_test_backend() {
  let dispatcher = TestDispatcher::new(0);
  let mut app = TestAppContext::build(dispatcher, None);
  // The app talks to real processes and real time; forbidding parking would
  // panic on their foreign-thread wakeups.
  app.executor().allow_parking();
  app.update(init_app);

  let mut mounted = None;
  let (root, cx) = app.add_window_view(|window, cx| {
    let (root, view) = build_root(window, cx);
    mounted = Some(view);
    root
  });
  let _root = root;
  let view = mounted.expect("workspace view");
  cx.run_until_parked();
  respond(ok(serde_json::json!({
    "ready": true,
    "backend": Backend::Test.as_str(),
  })));

  let stdin = std::io::stdin().lock();
  for line in stdin.lines() {
    let Ok(line) = line else { break };
    let Some(command) = parse_command_line(&line) else {
      continue;
    };
    handle_test_command(command, &view, cx);
  }
}

fn handle_test_command(
  command: Command,
  view: &Entity<WorkspaceView>,
  cx: &mut gpui::VisualTestContext,
) {
  match command {
    Command::Bounds { selector } => match cx.debug_bounds(intern_selector(&selector)) {
      Some(bounds) => respond(ok(serde_json::json!({
        "x": f32::from(bounds.origin.x),
        "y": f32::from(bounds.origin.y),
        "width": f32::from(bounds.size.width),
        "height": f32::from(bounds.size.height),
      }))),
      None => respond(err(format!("selector not painted: {selector}"))),
    },
    Command::Click { selector, x, y } => {
      let position = match (&selector, x, y) {
        (Some(selector), _, _) => cx
          .debug_bounds(intern_selector(selector))
          .map(|bounds| bounds.center()),
        (None, Some(x), Some(y)) => Some(gpui::point(px(x), px(y))),
        _ => None,
      };
      match position {
        Some(position) => {
          cx.simulate_click(position, gpui::Modifiers::default());
          cx.run_until_parked();
          respond(ok(serde_json::json!({})));
        }
        None => respond(err("click needs a painted selector or x and y")),
      }
    }
    Command::Key { keystrokes } => {
      cx.simulate_keystrokes(&keystrokes);
      cx.run_until_parked();
      respond(ok(serde_json::json!({})));
    }
    Command::Type { text } => {
      cx.simulate_input(&text);
      cx.run_until_parked();
      respond(ok(serde_json::json!({})));
    }
    Command::Clock { ms } => {
      cx.executor().advance_clock(Duration::from_millis(ms));
      cx.run_until_parked();
      respond(ok(serde_json::json!({})));
    }
    Command::Wait { ms } => {
      wait_test(cx, ms);
      respond(ok(serde_json::json!({})));
    }
    Command::Park => {
      cx.run_until_parked();
      respond(ok(serde_json::json!({})));
    }
    Command::PathPrompt { path } => {
      if cx.did_prompt_for_paths() {
        cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from(path)]));
        cx.run_until_parked();
        respond(ok(serde_json::json!({})));
        return;
      }
      let result = cx.update(|window, cx| {
        view.update(cx, |view, cx| {
          view.open_repository_for_driver(PathBuf::from(path), window, cx)
        })
      });
      cx.run_until_parked();
      match result {
        Ok(()) => respond(ok(serde_json::json!({}))),
        Err(error) => respond(err(error)),
      }
    }
    Command::Screenshot { path: _path } => {
      respond(err("screenshot requires --backend visual"));
    }
    Command::ShowChanges => {
      cx.update(|window, cx| {
        view.update(cx, |view, cx| view.show_changes_for_driver(window, cx));
      });
      cx.run_until_parked();
      respond(ok(serde_json::json!({})));
    }
    Command::HideDock => {
      cx.update(|window, cx| {
        view.update(cx, |view, cx| view.hide_dock_for_driver(window, cx));
      });
      cx.run_until_parked();
      respond(ok(serde_json::json!({})));
    }
    Command::SubmitPrompt { text } => {
      let result = cx.update(|window, cx| {
        view.update(cx, |view, cx| {
          view.submit_agent_prompt_for_driver(text, window, cx)
        })
      });
      cx.run_until_parked();
      match result {
        Ok(()) => respond(ok(serde_json::json!({}))),
        Err(error) => respond(err(error)),
      }
    }
    Command::Quit => quit_now(),
  }
}

fn wait_test(cx: &mut gpui::VisualTestContext, ms: u64) {
  let deadline = std::time::Instant::now() + Duration::from_millis(ms);
  let mut flip = false;
  while std::time::Instant::now() < deadline {
    cx.executor().tick();
    // Animations only advance when a frame is painted; an alternating mouse
    // move forces one per iteration (repeats are deduplicated).
    flip = !flip;
    let x = if flip { 0.0 } else { 1.0 };
    cx.simulate_mouse_move(
      gpui::point(px(x), px(0.0)),
      None,
      gpui::Modifiers::default(),
    );
    std::thread::sleep(Duration::from_millis(5));
  }
  cx.run_until_parked();
}

#[cfg(not(target_os = "macos"))]
fn run_visual_backend() {
  eprintln!("visual backend is only supported on macOS");
  std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn run_visual_backend() {
  let mut cx = VisualTestAppContext::with_asset_source(
    gpui_platform::current_platform(false),
    std::sync::Arc::new(ui::AppAssets),
  );
  cx.executor().allow_parking();
  cx.update(init_app);

  let mut mounted = None;
  let window = cx
    .open_offscreen_window(size(px(1510.0), px(945.0)), |window, cx| {
      cx.new(|cx| {
        let (root, view) = build_root(window, cx);
        mounted = Some(view);
        root
      })
    })
    .expect("open visual driver window");
  let view = mounted.expect("workspace view");
  let window = window.into();
  cx.run_until_parked();
  respond(ok(serde_json::json!({
    "ready": true,
    "backend": Backend::Visual.as_str(),
  })));

  let stdin = std::io::stdin().lock();
  for line in stdin.lines() {
    let Ok(line) = line else { break };
    let Some(command) = parse_command_line(&line) else {
      continue;
    };
    handle_visual_command(command, window, &view, &mut cx);
  }
}

#[cfg(target_os = "macos")]
fn handle_visual_command(
  command: Command,
  window: AnyWindowHandle,
  view: &Entity<WorkspaceView>,
  cx: &mut VisualTestAppContext,
) {
  match command {
    Command::Bounds { .. } => {
      respond(err("bounds requires --backend test"));
    }
    Command::Click { selector, x, y } => {
      if selector.is_some() {
        respond(err(
          "selector clicks require --backend test; use x and y with --backend visual",
        ));
        return;
      }
      let Some((x, y)) = x.zip(y) else {
        respond(err("visual click needs x and y"));
        return;
      };
      cx.simulate_click(
        window,
        gpui::point(px(x), px(y)),
        gpui::Modifiers::default(),
      );
      cx.run_until_parked();
      respond(ok(serde_json::json!({})));
    }
    Command::Key { keystrokes } => {
      cx.simulate_keystrokes(window, &keystrokes);
      cx.run_until_parked();
      respond(ok(serde_json::json!({})));
    }
    Command::Type { text } => {
      cx.simulate_input(window, &text);
      cx.run_until_parked();
      respond(ok(serde_json::json!({})));
    }
    Command::Clock { ms } => {
      cx.advance_clock(Duration::from_millis(ms));
      cx.run_until_parked();
      respond(ok(serde_json::json!({})));
    }
    Command::Wait { ms } => {
      wait_visual(cx, window, ms);
      respond(ok(serde_json::json!({})));
    }
    Command::Park => {
      cx.run_until_parked();
      respond(ok(serde_json::json!({})));
    }
    Command::PathPrompt { path } => match open_repository_directly(cx, window, view, path) {
      Ok(()) => respond(ok(serde_json::json!({}))),
      Err(error) => respond(err(error)),
    },
    Command::Screenshot { path } => {
      match save_screenshot(cx, window, std::path::Path::new(&path)) {
        Ok(()) => respond(ok(serde_json::json!({ "path": path }))),
        Err(error) => respond(err(error)),
      }
    }
    Command::ShowChanges => match show_changes_directly(cx, window, view) {
      Ok(()) => respond(ok(serde_json::json!({}))),
      Err(error) => respond(err(error)),
    },
    Command::HideDock => match hide_dock_directly(cx, window, view) {
      Ok(()) => respond(ok(serde_json::json!({}))),
      Err(error) => respond(err(error)),
    },
    Command::SubmitPrompt { text } => match submit_prompt_directly(cx, window, view, text) {
      Ok(()) => respond(ok(serde_json::json!({}))),
      Err(error) => respond(err(error)),
    },
    Command::Quit => quit_now(),
  }
}

#[cfg(target_os = "macos")]
fn wait_visual(cx: &mut VisualTestAppContext, window: AnyWindowHandle, ms: u64) {
  let deadline = std::time::Instant::now() + Duration::from_millis(ms);
  let mut flip = false;
  while std::time::Instant::now() < deadline {
    cx.executor().tick();
    flip = !flip;
    let x = if flip { 0.0 } else { 1.0 };
    cx.simulate_mouse_move(
      window,
      gpui::point(px(x), px(0.0)),
      None,
      gpui::Modifiers::default(),
    );
    std::thread::sleep(Duration::from_millis(5));
  }
  cx.run_until_parked();
}

#[cfg(target_os = "macos")]
fn open_repository_directly(
  cx: &mut VisualTestAppContext,
  window: AnyWindowHandle,
  view: &Entity<WorkspaceView>,
  path: String,
) -> Result<(), String> {
  let path = PathBuf::from(path);
  let result = cx
    .update_window(window, |_, window, cx| {
      view.update(cx, |view, cx| {
        view.open_repository_for_driver(path, window, cx)
      })
    })
    .map_err(|error| error.to_string())?;
  cx.run_until_parked();
  result.map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn show_changes_directly(
  cx: &mut VisualTestAppContext,
  window: AnyWindowHandle,
  view: &Entity<WorkspaceView>,
) -> Result<(), String> {
  cx.update_window(window, |_, window, cx| {
    view.update(cx, |view, cx| view.show_changes_for_driver(window, cx));
  })
  .map_err(|error| error.to_string())?;
  cx.run_until_parked();
  Ok(())
}

#[cfg(target_os = "macos")]
fn hide_dock_directly(
  cx: &mut VisualTestAppContext,
  window: AnyWindowHandle,
  view: &Entity<WorkspaceView>,
) -> Result<(), String> {
  cx.update_window(window, |_, window, cx| {
    view.update(cx, |view, cx| view.hide_dock_for_driver(window, cx));
  })
  .map_err(|error| error.to_string())?;
  cx.run_until_parked();
  Ok(())
}

#[cfg(target_os = "macos")]
fn submit_prompt_directly(
  cx: &mut VisualTestAppContext,
  window: AnyWindowHandle,
  view: &Entity<WorkspaceView>,
  text: String,
) -> Result<(), String> {
  let result = cx
    .update_window(window, |_, window, cx| {
      view.update(cx, |view, cx| {
        view.submit_agent_prompt_for_driver(text, window, cx)
      })
    })
    .map_err(|error| error.to_string())?;
  cx.run_until_parked();
  result.map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn save_screenshot(
  cx: &mut VisualTestAppContext,
  window: AnyWindowHandle,
  path: &std::path::Path,
) -> Result<(), String> {
  if let Some(parent) = path
    .parent()
    .filter(|parent| !parent.as_os_str().is_empty())
  {
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
  }
  let image = cx
    .capture_screenshot(window)
    .map_err(|error| error.to_string())?;
  image.save(path).map_err(|error| error.to_string())
}

fn parse_command_line(line: &str) -> Option<Command> {
  let line = line.trim();
  if line.is_empty() {
    return None;
  }
  match serde_json::from_str(line) {
    Ok(command) => Some(command),
    Err(parse_error) => {
      respond(err(format!("bad command: {parse_error}")));
      None
    }
  }
}

fn quit_now() {
  respond(ok(serde_json::json!({})));
  // Skip the teardown: live agent processes and reactor threads make the
  // test-context drop abort, and there is nothing to save.
  std::process::exit(0);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn backend_defaults_to_test() {
    assert_eq!(
      parse_args([]).expect("args"),
      Args {
        backend: Backend::Test,
        agent_command: None,
      }
    );
  }

  #[test]
  fn backend_and_agent_command_are_parsed() {
    assert_eq!(
      parse_args([
        "--backend".to_string(),
        "visual".to_string(),
        "--agent-command".to_string(),
        "/tmp/stub-agent".to_string(),
      ])
      .expect("args"),
      Args {
        backend: Backend::Visual,
        agent_command: Some("/tmp/stub-agent".to_string()),
      }
    );
  }

  #[test]
  fn backend_equals_form_is_parsed() {
    assert_eq!(
      parse_args(["--backend=visual".to_string()])
        .expect("args")
        .backend,
      Backend::Visual
    );
  }

  #[test]
  fn bad_backend_is_rejected() {
    assert_eq!(
      parse_args(["--backend".to_string(), "other".to_string()]).expect_err("bad backend"),
      "unknown backend: other"
    );
  }

  #[test]
  fn screenshot_command_is_json_lines_compatible() {
    let command: Command =
      serde_json::from_str(r#"{"cmd":"screenshot","path":"/tmp/reviu-driver/screen.png"}"#)
        .expect("command");
    match command {
      Command::Screenshot { path } => assert_eq!(path, "/tmp/reviu-driver/screen.png"),
      _ => panic!("expected screenshot command"),
    }
  }

  #[test]
  fn perf_driver_commands_are_json_lines_compatible() {
    assert!(matches!(
      serde_json::from_str::<Command>(r#"{"cmd":"show_changes"}"#).expect("show changes"),
      Command::ShowChanges
    ));
    assert!(matches!(
      serde_json::from_str::<Command>(r#"{"cmd":"hide_dock"}"#).expect("hide dock"),
      Command::HideDock
    ));
    match serde_json::from_str::<Command>(r#"{"cmd":"submit_prompt","text":"perf-stream"}"#)
      .expect("submit prompt")
    {
      Command::SubmitPrompt { text } => assert_eq!(text, "perf-stream"),
      _ => panic!("expected submit prompt command"),
    }
  }
}
