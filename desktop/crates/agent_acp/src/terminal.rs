//! Client-side ACP terminals: the agent runs its commands in PTYs we own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use terminal_core::{ProgramOptions, TailBudget, TerminalBounds, TerminalEvent, TerminalSession};

/// Retained output when the agent sets no byte limit.
const DEFAULT_OUTPUT_BYTE_LIMIT: usize = 128 * 1024;
/// Lines of a command kept for its chat card, which shows their end.
const DISPLAY_TAIL_LINES: usize = 200;
/// History a command's PTY keeps; the agent reads at most its byte limit.
const COMMAND_SCROLLBACK_LINES: usize = 100_000;

#[derive(Clone, Debug, Default)]
pub struct TerminalSnapshot {
  /// The command line as the agent asked for it, for display.
  pub command: String,
  /// The end of the output, with its colors as SGR sequences.
  pub output: String,
  pub truncated: bool,
  pub exit_code: Option<u32>,
  pub signal: Option<String>,
  pub finished: bool,
  pub killed: bool,
  /// Whether a stop control makes sense: only processes this client owns.
  pub can_kill: bool,
}

struct TerminalEntry {
  snapshot: TerminalSnapshot,
  byte_limit: usize,
  source: TerminalSource,
}

enum TerminalSource {
  /// A command running in a PTY this client owns.
  Pty {
    session: TerminalSession,
    kill_requested: bool,
  },
  /// Output an agent streams for a command it runs itself (codex).
  Streamed {
    /// Bytes of a UTF-8 character split across chunks, kept for the next.
    pending_bytes: Vec<u8>,
  },
  /// A PTY command that ended: its output is kept, its PTY is gone.
  Finished {
    agent_output: String,
    agent_output_clipped: bool,
  },
}

/// Live terminals of one agent session, shared between the ACP handlers and
/// the UI. Every change pushes the terminal id onto the updates channel.
pub struct TerminalStore {
  entries: Mutex<HashMap<String, TerminalEntry>>,
  updates_tx: async_channel::Sender<String>,
}

impl TerminalStore {
  pub(crate) fn new(updates_tx: async_channel::Sender<String>) -> Arc<Self> {
    Arc::new(Self {
      entries: Mutex::new(HashMap::new()),
      updates_tx,
    })
  }

  pub fn snapshot(&self, id: &str) -> Option<TerminalSnapshot> {
    let entries = self.entries.lock().ok()?;
    let entry = entries.get(id)?;
    let mut snapshot = entry.snapshot.clone();
    if let TerminalSource::Pty { session, .. } = &entry.source {
      let tail = session.tail_text(TailBudget::Lines(DISPLAY_TAIL_LINES), true);
      snapshot.output = tail.text;
      snapshot.truncated = tail.clipped;
    }
    Some(snapshot)
  }

  /// What the agent reads back: plain text, at most its byte limit, and
  /// whether earlier output was left out.
  pub(crate) fn agent_output(&self, id: &str) -> Option<(String, bool)> {
    let entries = self.entries.lock().ok()?;
    let entry = entries.get(id)?;
    Some(match &entry.source {
      TerminalSource::Pty { session, .. } => {
        let tail = session.tail_text(TailBudget::Bytes(entry.byte_limit), false);
        (tail.text, tail.clipped)
      }
      TerminalSource::Streamed { .. } => (
        agent_visible_output(&entry.snapshot.output),
        entry.snapshot.truncated,
      ),
      TerminalSource::Finished {
        agent_output,
        agent_output_clipped,
      } => (agent_output.clone(), *agent_output_clipped),
    })
  }

  /// Kills the command and whatever it started; the exit lands as a normal
  /// finish, marked as killed.
  pub fn kill(&self, id: &str) {
    if let Ok(mut entries) = self.entries.lock()
      && let Some(entry) = entries.get_mut(id)
      && let TerminalSource::Pty {
        session,
        kill_requested,
      } = &mut entry.source
      && session.kill()
    {
      *kill_requested = true;
    }
  }

  /// The agent is done with this terminal: stop the process but keep the
  /// snapshot readable, the transcript still renders it.
  pub(crate) fn release(&self, id: &str) {
    self.kill(id);
  }

  fn notify(&self, id: &str) {
    let _ = self.updates_tx.try_send(id.to_string());
  }

  /// Applies the PTY's events; returns true once the command has ended and
  /// its output is frozen.
  fn process_pty_events(&self, id: &str, events: Vec<TerminalEvent>) -> bool {
    let Ok(mut entries) = self.entries.lock() else {
      return true;
    };
    let Some(entry) = entries.get_mut(id) else {
      return true;
    };
    let TerminalSource::Pty {
      session,
      kill_requested,
    } = &mut entry.source
    else {
      return true;
    };
    let result = session.process_events(events);
    if !result.exited {
      drop(entries);
      if result.changed {
        self.notify(id);
      }
      return false;
    }

    let (exit_code, signal) = session.child_exit().map(exit_parts).unwrap_or((None, None));
    let display = session.tail_text(TailBudget::Lines(DISPLAY_TAIL_LINES), true);
    let agent_output = session.tail_text(TailBudget::Bytes(entry.byte_limit), false);
    entry.snapshot.output = display.text;
    entry.snapshot.truncated = display.clipped;
    entry.snapshot.exit_code = exit_code;
    entry.snapshot.signal = signal;
    entry.snapshot.finished = true;
    entry.snapshot.killed = *kill_requested;
    entry.snapshot.can_kill = false;
    entry.source = TerminalSource::Finished {
      agent_output: agent_output.text,
      agent_output_clipped: agent_output.clipped,
    };
    drop(entries);
    self.notify(id);
    true
  }

  fn append_output(&self, id: &str, chunk: &[u8]) {
    if let Ok(mut entries) = self.entries.lock()
      && let Some(entry) = entries.get_mut(id)
      && let TerminalSource::Streamed { pending_bytes } = &mut entry.source
    {
      // A multi-byte character split across two reads must not turn into
      // replacement glyphs: hold the incomplete tail for the next chunk.
      pending_bytes.extend_from_slice(chunk);
      loop {
        match std::str::from_utf8(pending_bytes) {
          Ok(valid) => {
            entry.snapshot.output.push_str(valid);
            pending_bytes.clear();
            break;
          }
          Err(e) => {
            let valid = e.valid_up_to();
            entry
              .snapshot
              .output
              .push_str(std::str::from_utf8(&pending_bytes[..valid]).unwrap_or(""));
            match e.error_len() {
              Some(len) => {
                entry.snapshot.output.push('\u{FFFD}');
                pending_bytes.drain(..valid + len);
              }
              None => {
                pending_bytes.drain(..valid);
                break;
              }
            }
          }
        }
      }
      let over = entry.snapshot.output.len().saturating_sub(entry.byte_limit);
      if over > 0 {
        let mut cut = over;
        while cut < entry.snapshot.output.len() && !entry.snapshot.output.is_char_boundary(cut) {
          cut += 1;
        }
        entry.snapshot.output.drain(..cut);
        entry.snapshot.truncated = true;
      }
    }
    self.notify(id);
  }

  fn finish(&self, id: &str, exit_code: Option<u32>, signal: Option<String>, killed: bool) {
    if let Ok(mut entries) = self.entries.lock()
      && let Some(entry) = entries.get_mut(id)
    {
      // A process dying mid-character leaves a stub tail: flush it lossily.
      if let TerminalSource::Streamed { pending_bytes } = &mut entry.source
        && !pending_bytes.is_empty()
      {
        let tail = std::mem::take(pending_bytes);
        entry
          .snapshot
          .output
          .push_str(&String::from_utf8_lossy(&tail));
      }
      entry.snapshot.exit_code = exit_code;
      entry.snapshot.signal = signal;
      entry.snapshot.finished = true;
      entry.snapshot.killed = killed;
      entry.snapshot.can_kill = false;
    }
    self.notify(id);
  }

  /// An agent-owned terminal (e.g. codex runs commands itself and streams
  /// them through `_meta`): tracked for display, but not killable here.
  fn upsert_external(&self, id: &str) {
    if let Ok(mut entries) = self.entries.lock() {
      entries
        .entry(id.to_string())
        .or_insert_with(|| TerminalEntry {
          snapshot: TerminalSnapshot::default(),
          byte_limit: DEFAULT_OUTPUT_BYTE_LIMIT,
          source: TerminalSource::Streamed {
            pending_bytes: Vec::new(),
          },
        });
    }
    self.notify(id);
  }
}

/// Feeds the store from codex-style terminal metadata on tool call updates:
/// `_meta.terminal_info` opens one, `terminal_output`/`terminal_output_delta`
/// carry output, `terminal_exit` closes it.
pub(crate) fn inspect_session_update(
  store: &Arc<TerminalStore>,
  update: &agent_client_protocol::schema::SessionUpdate,
) {
  use agent_client_protocol::schema::SessionUpdate;
  let meta = match update {
    SessionUpdate::ToolCall(call) => call.meta.as_ref(),
    SessionUpdate::ToolCallUpdate(update) => update.meta.as_ref(),
    _ => None,
  };
  let Some(meta) = meta else { return };
  if let Some(id) = meta
    .get("terminal_info")
    .and_then(|v| v.get("terminal_id"))
    .and_then(|v| v.as_str())
  {
    store.upsert_external(id);
  }
  for key in ["terminal_output_delta", "terminal_output"] {
    if let Some(delta) = meta.get(key)
      && let (Some(id), Some(data)) = (
        delta.get("terminal_id").and_then(|v| v.as_str()),
        delta.get("data").and_then(|v| v.as_str()),
      )
    {
      store.upsert_external(id);
      store.append_output(id, data.as_bytes());
    }
  }
  if let Some(exit) = meta.get("terminal_exit")
    && let Some(id) = exit.get("terminal_id").and_then(|v| v.as_str())
  {
    let exit_code = exit
      .get("exit_code")
      .and_then(|v| v.as_u64())
      .map(|c| c as u32);
    let signal = exit
      .get("signal")
      .and_then(|v| v.as_str())
      .map(str::to_string);
    store.upsert_external(id);
    store.finish(id, exit_code, signal, false);
  }
}

fn exit_parts(status: std::process::ExitStatus) -> (Option<u32>, Option<String>) {
  let code = status.code().map(|c| c as u32);
  #[cfg(unix)]
  let signal = {
    use std::os::unix::process::ExitStatusExt as _;
    status.signal().map(|s| format!("{s}"))
  };
  #[cfg(not(unix))]
  let signal = None;
  (code, signal)
}

const GIT_COLOR_CONFIG_KEYS: &[&str] = &[
  "color.ui",
  "color.diff",
  "color.status",
  "color.branch",
  "color.grep",
  "color.interactive",
];

fn git_color_config_env(inherited_count: Option<&str>) -> Vec<(String, String)> {
  let index = inherited_count
    .and_then(|count| count.parse::<usize>().ok())
    .unwrap_or(0);
  let mut env = Vec::with_capacity(1 + GIT_COLOR_CONFIG_KEYS.len() * 2);
  env.push((
    "GIT_CONFIG_COUNT".to_string(),
    (index + GIT_COLOR_CONFIG_KEYS.len()).to_string(),
  ));
  for (offset, key) in GIT_COLOR_CONFIG_KEYS.iter().enumerate() {
    let index = index + offset;
    env.push((format!("GIT_CONFIG_KEY_{index}"), (*key).to_string()));
    env.push((format!("GIT_CONFIG_VALUE_{index}"), "always".to_string()));
  }
  env
}

pub(crate) fn apply_color_env(cmd: &mut async_process::Command) {
  // Piped stdio is not a TTY, so tools silence their colors; these opt-ins
  // bring them back for the terminal cards. The agent's env still overrides.
  let inherited_git_config_count = std::env::var("GIT_CONFIG_COUNT").ok();
  let git_color_env = git_color_config_env(inherited_git_config_count.as_deref());
  cmd.env_remove("NO_COLOR");
  cmd.env("TERM", "xterm-256color");
  cmd.env("COLORTERM", "truecolor");
  cmd.env("CLICOLOR", "1");
  cmd.env("CLICOLOR_FORCE", "1");
  cmd.env("FORCE_COLOR", "1");
  cmd.env("CARGO_TERM_COLOR", "always");
  cmd.env("PY_COLORS", "1");
  cmd.env("RUST_LOG_STYLE", "always");
  cmd.envs(git_color_env);
}

/// A pager waits for keys the agent can never send: `git log` or `gh` would
/// hang its turn. Git reads its own variable before `PAGER` and `core.pager`.
const PAGER_ENV: [(&str, &str); 2] = [("PAGER", "cat"), ("GIT_PAGER", "cat")];

/// What the agent reads back: the colors forced for the chat cards are
/// noise to a model, and progress lines keep only what a terminal would show.
pub(crate) fn agent_visible_output(output: &str) -> String {
  let mut text = ansi_text::strip_ansi_escapes(output);
  if output.ends_with('\n') && !text.ends_with('\n') {
    text.push('\n');
  }
  text
}

/// Runs the requested command in its own PTY and keeps the store fed from
/// its events until it ends; `kill` interrupts it.
pub(crate) fn spawn_terminal(
  store: &Arc<TerminalStore>,
  id: String,
  command: String,
  args: Vec<String>,
  env: Vec<(String, String)>,
  cwd: std::path::PathBuf,
  output_byte_limit: Option<u64>,
) -> Result<()> {
  let display = if args.is_empty() {
    command.clone()
  } else {
    format!("{command} {}", args.join(" "))
  };
  let agent_sets_no_color = env.iter().any(|(name, _)| name == "NO_COLOR");
  let mut program_env = PAGER_ENV
    .iter()
    .map(|(name, value)| (name.to_string(), value.to_string()))
    .collect::<HashMap<_, _>>();
  program_env.extend(env);
  let (program, program_args) = program_reading_no_input(command, args, !agent_sets_no_color);
  let session = TerminalSession::spawn_program(
    cwd,
    TerminalBounds::default(),
    ProgramOptions {
      program,
      args: program_args,
      env: program_env,
      scrollback_lines: COMMAND_SCROLLBACK_LINES,
    },
  )
  .with_context(|| format!("run {display}"))?;
  let events = session.event_receiver();

  {
    let mut entries = store
      .entries
      .lock()
      .map_err(|_| anyhow!("terminal store poisoned"))?;
    entries.insert(
      id.clone(),
      TerminalEntry {
        snapshot: TerminalSnapshot {
          command: display,
          can_kill: true,
          ..Default::default()
        },
        byte_limit: output_byte_limit
          .map(|limit| limit as usize)
          .unwrap_or(DEFAULT_OUTPUT_BYTE_LIMIT),
        source: TerminalSource::Pty {
          session,
          kill_requested: false,
        },
      },
    );
  }
  store.notify(&id);

  let store = store.clone();
  smol::spawn(async move {
    while let Ok(first) = events.recv().await {
      let mut batch = vec![first];
      while let Ok(event) = events.try_recv() {
        batch.push(event);
      }
      if store.process_pty_events(&id, batch) {
        return;
      }
    }
  })
  .detach();

  Ok(())
}

/// Agent commands must never wait on a question nobody will answer, so the
/// program reads `/dev/null` like it did with pipes, while its output still
/// goes to the PTY.
#[cfg(unix)]
fn program_reading_no_input(
  command: String,
  args: Vec<String>,
  scrub_no_color: bool,
) -> (String, Vec<String>) {
  // A NO_COLOR inherited from Reviu's own launch would gray the chat cards
  // out; the PTY can only add variables, so the wrapper drops it.
  let script = if scrub_no_color {
    "unset NO_COLOR; exec \"$0\" \"$@\" </dev/null"
  } else {
    "exec \"$0\" \"$@\" </dev/null"
  };
  let args = ["-c".to_string(), script.to_string(), command]
    .into_iter()
    .chain(args)
    .collect();
  ("/bin/sh".to_string(), args)
}

#[cfg(not(unix))]
fn program_reading_no_input(
  command: String,
  args: Vec<String>,
  _scrub_no_color: bool,
) -> (String, Vec<String>) {
  (command, args)
}

#[cfg(test)]
mod tests {
  use super::*;
  use agent_client_protocol::schema::{
    SessionUpdate, ToolCallId, ToolCallUpdate, ToolCallUpdateFields,
  };

  fn update_with_meta(meta: serde_json::Value) -> SessionUpdate {
    let mut update = ToolCallUpdate::new(ToolCallId::new("t1"), ToolCallUpdateFields::new());
    update.meta = Some(meta.as_object().expect("object").clone());
    SessionUpdate::ToolCallUpdate(update)
  }

  fn insert_entry(store: &TerminalStore, id: &str, byte_limit: usize) {
    store.entries.lock().unwrap().insert(
      id.to_string(),
      TerminalEntry {
        snapshot: TerminalSnapshot::default(),
        byte_limit,
        source: TerminalSource::Streamed {
          pending_bytes: Vec::new(),
        },
      },
    );
  }

  #[test]
  fn git_color_config_starts_a_runtime_config_when_none_exists() {
    let env = git_color_config_env(None);
    assert_eq!(
      env.first(),
      Some(&("GIT_CONFIG_COUNT".to_string(), "6".to_string()))
    );
    assert!(env.contains(&("GIT_CONFIG_KEY_0".to_string(), "color.ui".to_string())));
    assert!(env.contains(&("GIT_CONFIG_VALUE_0".to_string(), "always".to_string())));
    assert!(env.contains(&("GIT_CONFIG_KEY_1".to_string(), "color.diff".to_string())));
    assert!(env.contains(&("GIT_CONFIG_VALUE_1".to_string(), "always".to_string())));
  }

  #[test]
  fn git_color_config_appends_to_an_existing_runtime_config() {
    let env = git_color_config_env(Some("2"));
    assert_eq!(
      env.first(),
      Some(&("GIT_CONFIG_COUNT".to_string(), "8".to_string()))
    );
    assert!(env.contains(&("GIT_CONFIG_KEY_2".to_string(), "color.ui".to_string())));
    assert!(env.contains(&("GIT_CONFIG_KEY_3".to_string(), "color.diff".to_string())));
  }

  fn run_command(script: &str, env: Vec<(String, String)>) -> Arc<TerminalStore> {
    let (tx, _rx) = async_channel::unbounded();
    let store = TerminalStore::new(tx);
    spawn_terminal(
      &store,
      "t".to_string(),
      "sh".to_string(),
      vec!["-c".to_string(), script.to_string()],
      env,
      std::env::current_dir().expect("cwd"),
      None,
    )
    .expect("spawns");
    for _ in 0..500 {
      if store
        .snapshot("t")
        .is_some_and(|snapshot| snapshot.finished)
      {
        return store;
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("the command never finished");
  }

  #[cfg(unix)]
  #[test]
  fn commands_write_to_a_terminal_but_read_no_input() {
    let store = run_command(
      "[ -t 1 ] && echo output-is-a-terminal; [ -t 0 ] || echo no-input; printf \"$TERM:$PAGER:$GIT_PAGER\"",
      Vec::new(),
    );
    assert_eq!(
      store.agent_output("t").expect("entry").0,
      "output-is-a-terminal\nno-input\nxterm-256color:cat:cat\n"
    );
  }

  #[cfg(unix)]
  #[test]
  fn the_card_keeps_colors_and_the_agent_reads_plain_text() {
    let store = run_command("printf '\\033[32mok\\033[0m done\\n'; exit 3", Vec::new());
    let snapshot = store.snapshot("t").expect("entry");
    assert_eq!(snapshot.output, "\u{1b}[0;32mok\u{1b}[0m done\n");
    assert_eq!(snapshot.exit_code, Some(3));
    assert_eq!(
      store.agent_output("t").expect("entry"),
      ("ok done\n".to_string(), false)
    );
  }

  #[test]
  fn the_agent_reads_output_without_colors_or_overwritten_progress() {
    assert_eq!(
      super::agent_visible_output(
        "\u{1b}[1m\u{1b}[31merror\u{1b}[0m: mismatched types\nprogress 10%\rprogress 100%\n"
      ),
      "error: mismatched types\nprogress 100%\n"
    );
  }

  #[cfg(unix)]
  #[test]
  fn the_agents_env_overrides_our_defaults() {
    let store = run_command(
      "printf \"$PAGER\"",
      vec![("PAGER".to_string(), "less".to_string())],
    );
    assert_eq!(
      store.agent_output("t").expect("entry").0,
      "less\n",
      "an explicit agent env must win over our defaults"
    );
  }

  #[cfg(unix)]
  #[test]
  fn an_inherited_no_color_is_scrubbed_from_spawned_commands() {
    // Process-global, but harmless to parallel tests: every command this
    // module spawns drops NO_COLOR unless the agent sets it.
    unsafe { std::env::set_var("NO_COLOR", "1") };
    let store = run_command("printf \"${NO_COLOR-unset}\"", Vec::new());
    unsafe { std::env::remove_var("NO_COLOR") };
    assert_eq!(
      store.agent_output("t").expect("entry").0,
      "unset\n",
      "a user's NO_COLOR must not silence the terminal cards"
    );
  }

  #[cfg(unix)]
  #[test]
  fn killing_a_command_stops_what_it_started() {
    let (tx, _rx) = async_channel::unbounded();
    let store = TerminalStore::new(tx);
    spawn_terminal(
      &store,
      "t".to_string(),
      "sh".to_string(),
      vec!["-c".to_string(), "sleep 30 & wait".to_string()],
      Vec::new(),
      std::env::current_dir().expect("cwd"),
      None,
    )
    .expect("spawns");
    std::thread::sleep(std::time::Duration::from_millis(200));
    store.kill("t");
    for _ in 0..250 {
      if store
        .snapshot("t")
        .is_some_and(|snapshot| snapshot.finished)
      {
        break;
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let snapshot = store.snapshot("t").expect("entry");
    assert!(snapshot.finished && snapshot.killed);
    assert_eq!(snapshot.signal.as_deref(), Some("9"));
  }

  #[test]
  fn output_over_the_byte_limit_truncates_from_the_start_on_a_char_boundary() {
    let (tx, _rx) = async_channel::unbounded();
    let store = TerminalStore::new(tx);
    insert_entry(&store, "t", 16);
    // Multi-byte content: é is two bytes, the cut must never split one.
    store.append_output("t", "aaaaaaaaaa".as_bytes());
    store.append_output("t", "ééééé".as_bytes());
    let snap = store.snapshot("t").expect("entry");
    assert!(snap.truncated, "the cap was exceeded");
    assert!(snap.output.len() <= 16, "capped, got {}", snap.output.len());
    assert!(
      snap.output.ends_with("ééééé"),
      "the newest output survives, got {:?}",
      snap.output
    );
  }

  #[test]
  fn a_character_split_across_chunks_is_reassembled() {
    let (tx, _rx) = async_channel::unbounded();
    let store = TerminalStore::new(tx);
    insert_entry(&store, "t", 1024);
    let bytes = "voilà".as_bytes();
    // Split in the middle of the two-byte à.
    let cut = bytes.len() - 1;
    store.append_output("t", &bytes[..cut]);
    assert_eq!(
      store.snapshot("t").unwrap().output,
      "voil",
      "the incomplete tail is held back"
    );
    store.append_output("t", &bytes[cut..]);
    assert_eq!(store.snapshot("t").unwrap().output, "voilà");
    // Truly invalid bytes still surface as replacement glyphs.
    store.append_output("t", &[0xFF, b'!']);
    assert_eq!(store.snapshot("t").unwrap().output, "voilà\u{FFFD}!");
  }

  #[test]
  fn finish_flushes_a_pending_incomplete_tail() {
    let (tx, _rx) = async_channel::unbounded();
    let store = TerminalStore::new(tx);
    insert_entry(&store, "t", 1024);
    let bytes = "é".as_bytes();
    store.append_output("t", &bytes[..1]);
    store.finish("t", Some(1), None, false);
    let snap = store.snapshot("t").unwrap();
    assert_eq!(
      snap.output, "\u{FFFD}",
      "the stub tail is not silently lost"
    );
  }

  #[test]
  fn codex_meta_deltas_feed_an_external_terminal() {
    let (tx, _rx) = async_channel::unbounded();
    let store = TerminalStore::new(tx);

    inspect_session_update(
      &store,
      &update_with_meta(serde_json::json!({
        "terminal_info": { "terminal_id": "item-1", "cwd": "/repo" }
      })),
    );
    inspect_session_update(
      &store,
      &update_with_meta(serde_json::json!({
        "terminal_output_delta": { "terminal_id": "item-1", "data": "hello " }
      })),
    );
    inspect_session_update(
      &store,
      &update_with_meta(serde_json::json!({
        "terminal_output_delta": { "terminal_id": "item-1", "data": "world\n" },
        "terminal_exit": { "terminal_id": "item-1", "exit_code": 2, "signal": null }
      })),
    );

    let snap = store.snapshot("item-1").expect("tracked");
    assert_eq!(snap.output, "hello world\n");
    assert!(snap.finished);
    assert_eq!(snap.exit_code, Some(2));
    assert!(!snap.can_kill, "agent-owned commands offer no stop control");
  }

  #[test]
  fn a_delta_for_an_unseen_terminal_creates_its_entry() {
    let (tx, _rx) = async_channel::unbounded();
    let store = TerminalStore::new(tx);
    inspect_session_update(
      &store,
      &update_with_meta(serde_json::json!({
        "terminal_output": { "terminal_id": "late", "data": "aggregated output" }
      })),
    );
    let snap = store.snapshot("late").expect("created on the fly");
    assert_eq!(snap.output, "aggregated output");
    assert!(!snap.finished);
  }
}
