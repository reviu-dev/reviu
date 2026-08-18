//! Interactive driver for the real app: mounts `WorkspaceView` in a test
//! window and takes JSON-lines commands on stdin, one response per line on
//! stdout. Built for agents and humans debugging the UI without the full app.
//!
//! Verbs: `bounds` (of a debug selector), `click` (selector or point), `type`,
//! `key`, `clock` (virtual ms), `wait` (real ms), `park`, `path_prompt`,
//! `quit`. Screenshots need the real-renderer backend, planned for macOS.
//!
//! Usage: `cargo run -p reviu_driver` then e.g.
//! `{"cmd":"bounds","selector":"session-repo-context"}`
//! `{"cmd":"key","keystrokes":"cmd-p"}` `{"cmd":"type","text":"push"}`
//!
//! This is the real app on the real machine: it reads and writes the same
//! config store and repositories the installed app uses.

use std::io::{BufRead as _, Write as _};
use std::time::Duration;

use gpui::{
  App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, TestAppContext,
  TestDispatcher, Window, div, prelude::*,
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
  /// Answer the next "open folder" prompt with this path.
  PathPrompt {
    path: String,
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

/// `debug_bounds` wants a `'static` selector; interning keeps repeated
/// queries from growing the leak.
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

fn main() {
  let mut agent_command = None;
  let mut args = std::env::args().skip(1);
  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--agent-command" => agent_command = args.next(),
      other => {
        eprintln!("unknown argument: {other}");
        std::process::exit(2);
      }
    }
  }
  // Without an override the shell connects the real configured agent.
  agent_chat_panel::set_backend_command_override(agent_command);

  let dispatcher = TestDispatcher::new(0);
  let mut app = TestAppContext::build(dispatcher, None);
  // The app talks to real processes and real time; forbidding parking would
  // panic on their foreign-thread wakeups.
  app.executor().allow_parking();
  app.update(|cx| {
    gpui_component::init(cx);
    ui::init(cx);
    let theme = gpui_component::Theme::global_mut(cx);
    theme.font_family = "Inter".into();
    theme.mono_font_family = "Lilex".into();
  });

  let mut mounted: Option<Entity<WorkspaceView>> = None;
  let (root, cx) = app.add_window_view(|window, cx| {
    let view = cx.new(|cx| WorkspaceView::new(window, cx));
    mounted = Some(view.clone());
    let driver_root = cx.new(|cx| DriverRoot {
      view,
      focus_handle: cx.focus_handle(),
    });
    window.focus(&driver_root.focus_handle(cx), cx);
    Root::new(driver_root, window, cx)
  });
  let _view = mounted.expect("workspace view");
  let _root = root;
  cx.run_until_parked();
  respond(ok(serde_json::json!({ "ready": true })));

  let stdin = std::io::stdin().lock();
  for line in stdin.lines() {
    let Ok(line) = line else { break };
    let line = line.trim();
    if line.is_empty() {
      continue;
    }
    let command: Command = match serde_json::from_str(line) {
      Ok(command) => command,
      Err(parse_error) => {
        respond(err(format!("bad command: {parse_error}")));
        continue;
      }
    };
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
          (Some(selector), _, _) => match cx.debug_bounds(intern_selector(selector)) {
            Some(bounds) => Some(bounds.center()),
            None => None,
          },
          (None, Some(x), Some(y)) => Some(gpui::point(gpui::px(x), gpui::px(y))),
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
        let deadline = std::time::Instant::now() + Duration::from_millis(ms);
        while std::time::Instant::now() < deadline {
          cx.executor().tick();
          std::thread::sleep(Duration::from_millis(10));
        }
        cx.run_until_parked();
        respond(ok(serde_json::json!({})));
      }
      Command::Park => {
        cx.run_until_parked();
        respond(ok(serde_json::json!({})));
      }
      Command::PathPrompt { path } => {
        // Answers a prompt the UI already opened (e.g. after clicking Open
        // repository); simulating with none pending panics in gpui.
        if !cx.did_prompt_for_paths() {
          respond(err("no path prompt is open"));
          continue;
        }
        cx.simulate_path_prompt_response(|_| Some(vec![std::path::PathBuf::from(path.clone())]));
        cx.run_until_parked();
        respond(ok(serde_json::json!({})));
      }
      Command::Quit => {
        respond(ok(serde_json::json!({})));
        return;
      }
    }
  }
}
