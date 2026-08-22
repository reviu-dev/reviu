//! Client-side ACP terminals: the agent runs its commands in processes we own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use futures::channel::oneshot;

/// Retained output when the agent sets no byte limit.
const DEFAULT_OUTPUT_BYTE_LIMIT: usize = 128 * 1024;

#[derive(Clone, Debug, Default)]
pub struct TerminalSnapshot {
  /// The command line as the agent asked for it, for display.
  pub command: String,
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
  kill_tx: Option<oneshot::Sender<()>>,
  /// Bytes of a UTF-8 character split across read chunks, kept for the next.
  pending_bytes: Vec<u8>,
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
    entries.get(id).map(|e| e.snapshot.clone())
  }

  /// Ask the running process to die; the exit lands as a normal finish.
  pub fn kill(&self, id: &str) {
    let kill_tx = self
      .entries
      .lock()
      .ok()
      .and_then(|mut entries| entries.get_mut(id).and_then(|e| e.kill_tx.take()));
    if let Some(tx) = kill_tx {
      let _ = tx.send(());
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

  fn append_output(&self, id: &str, chunk: &[u8]) {
    if let Ok(mut entries) = self.entries.lock()
      && let Some(entry) = entries.get_mut(id)
    {
      // A multi-byte character split across two reads must not turn into
      // replacement glyphs: hold the incomplete tail for the next chunk.
      entry.pending_bytes.extend_from_slice(chunk);
      loop {
        match std::str::from_utf8(&entry.pending_bytes) {
          Ok(valid) => {
            entry.snapshot.output.push_str(valid);
            entry.pending_bytes.clear();
            break;
          }
          Err(e) => {
            let valid = e.valid_up_to();
            entry
              .snapshot
              .output
              .push_str(std::str::from_utf8(&entry.pending_bytes[..valid]).unwrap_or(""));
            match e.error_len() {
              Some(len) => {
                entry.snapshot.output.push('\u{FFFD}');
                entry.pending_bytes.drain(..valid + len);
              }
              None => {
                entry.pending_bytes.drain(..valid);
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
      if !entry.pending_bytes.is_empty() {
        let tail = std::mem::take(&mut entry.pending_bytes);
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
      entry.kill_tx = None;
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
          kill_tx: None,
          pending_bytes: Vec::new(),
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

/// Spawn the requested command and stream its output into the store. The
/// readers and the exit waiter run as detached tasks; `kill` interrupts.
pub(crate) fn spawn_terminal(
  store: &Arc<TerminalStore>,
  id: String,
  command: String,
  args: Vec<String>,
  env: Vec<(String, String)>,
  cwd: std::path::PathBuf,
  output_byte_limit: Option<u64>,
) -> Result<()> {
  let mut cmd = async_process::Command::new(&command);
  cmd.args(&args);
  apply_color_env(&mut cmd);
  cmd.envs(env);
  cmd.current_dir(&cwd);
  cmd.stdin(std::process::Stdio::null());
  cmd.stdout(std::process::Stdio::piped());
  cmd.stderr(std::process::Stdio::piped());
  let mut child = cmd
    .spawn()
    .with_context(|| format!("spawn {command} {args:?}"))?;

  let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
  let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;
  let (kill_tx, kill_rx) = oneshot::channel::<()>();

  let display = if args.is_empty() {
    command.clone()
  } else {
    format!("{command} {}", args.join(" "))
  };
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
          .map(|l| l as usize)
          .unwrap_or(DEFAULT_OUTPUT_BYTE_LIMIT),
        kill_tx: Some(kill_tx),
        pending_bytes: Vec::new(),
      },
    );
  }
  store.notify(&id);

  for reader in [
    Box::new(stdout) as Box<dyn futures::AsyncRead + Unpin + Send>,
    Box::new(stderr) as Box<dyn futures::AsyncRead + Unpin + Send>,
  ] {
    let store = store.clone();
    let id = id.clone();
    let mut reader = reader;
    smol::spawn(async move {
      use futures::AsyncReadExt as _;
      let mut buf = [0u8; 8192];
      loop {
        match reader.read(&mut buf).await {
          Ok(0) | Err(_) => break,
          Ok(n) => store.append_output(&id, &buf[..n]),
        }
      }
    })
    .detach();
  }

  let store = store.clone();
  smol::spawn(async move {
    use futures::FutureExt as _;
    let mut kill_rx = kill_rx.fuse();
    let mut status_fut = Box::pin(child.status()).fuse();
    futures::select_biased! {
      _ = kill_rx => {
        drop(status_fut);
        let _ = child.kill();
        let status = child.status().await;
        match status {
          Ok(status) => {
            let (code, signal) = exit_parts(status);
            store.finish(&id, code, signal, true);
          }
          Err(_) => store.finish(&id, None, None, true),
        }
      }
      status = status_fut => {
        match status {
          Ok(status) => {
            let (code, signal) = exit_parts(status);
            store.finish(&id, code, signal, false);
          }
          Err(_) => store.finish(&id, None, None, false),
        }
      }
    }
  })
  .detach();

  Ok(())
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
        kill_tx: None,
        pending_bytes: Vec::new(),
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

  #[cfg(unix)]
  #[test]
  fn spawned_commands_get_the_color_forcing_env() {
    let (tx, _rx) = async_channel::unbounded();
    let store = Arc::new(TerminalStore::new(tx));
    spawn_terminal(
      &store,
      "t".to_string(),
      "sh".to_string(),
      vec![
        "-c".to_string(),
        "printf \"$CARGO_TERM_COLOR:$PY_COLORS:$RUST_LOG_STYLE:$CLICOLOR_FORCE:$FORCE_COLOR:$TERM:$COLORTERM:$CLICOLOR:${NO_COLOR-unset}\"".to_string(),
      ],
      Vec::new(),
      std::env::current_dir().expect("cwd"),
      None,
    )
    .expect("spawns");
    for _ in 0..250 {
      if store.snapshot("t").is_some_and(|s| s.finished) {
        break;
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let snap = store.snapshot("t").expect("entry");
    assert!(snap.finished, "the probe command finished");
    assert_eq!(
      snap.output,
      "always:1:always:1:1:xterm-256color:truecolor:1:unset"
    );
  }

  #[cfg(unix)]
  #[test]
  fn git_commands_get_color_config_always() {
    let git_exists = std::process::Command::new("git")
      .arg("--version")
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .status()
      .is_ok_and(|status| status.success());
    if !git_exists {
      return;
    }

    let (tx, _rx) = async_channel::unbounded();
    let store = Arc::new(TerminalStore::new(tx));
    spawn_terminal(
      &store,
      "t".to_string(),
      "sh".to_string(),
      vec![
        "-c".to_string(),
        "git config --get color.ui && git config --get color.diff".to_string(),
      ],
      Vec::new(),
      std::env::current_dir().expect("cwd"),
      None,
    )
    .expect("spawns");
    for _ in 0..250 {
      if store.snapshot("t").is_some_and(|s| s.finished) {
        break;
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let snap = store.snapshot("t").expect("entry");
    assert!(snap.finished, "the probe command finished");
    assert_eq!(snap.output, "always\nalways\n");
  }

  #[cfg(unix)]
  #[test]
  fn the_agents_env_overrides_the_color_forcing_defaults() {
    let (tx, _rx) = async_channel::unbounded();
    let store = Arc::new(TerminalStore::new(tx));
    spawn_terminal(
      &store,
      "t".to_string(),
      "sh".to_string(),
      vec![
        "-c".to_string(),
        "printf \"$CARGO_TERM_COLOR:$PY_COLORS:$RUST_LOG_STYLE\"".to_string(),
      ],
      vec![
        ("CARGO_TERM_COLOR".to_string(), "never".to_string()),
        ("PY_COLORS".to_string(), "0".to_string()),
        ("RUST_LOG_STYLE".to_string(), "never".to_string()),
      ],
      std::env::current_dir().expect("cwd"),
      None,
    )
    .expect("spawns");
    for _ in 0..250 {
      if store.snapshot("t").is_some_and(|s| s.finished) {
        break;
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
      store.snapshot("t").expect("entry").output,
      "never:0:never",
      "an explicit agent env must win over our defaults"
    );
  }

  #[cfg(unix)]
  #[test]
  fn an_inherited_no_color_is_scrubbed_from_spawned_commands() {
    // Process-global, but harmless to parallel tests: apply_color_env strips
    // NO_COLOR from every child this module spawns.
    unsafe { std::env::set_var("NO_COLOR", "1") };
    let (tx, _rx) = async_channel::unbounded();
    let store = Arc::new(TerminalStore::new(tx));
    spawn_terminal(
      &store,
      "t".to_string(),
      "sh".to_string(),
      vec!["-c".to_string(), "printf \"${NO_COLOR-unset}\"".to_string()],
      Vec::new(),
      std::env::current_dir().expect("cwd"),
      None,
    )
    .expect("spawns");
    for _ in 0..250 {
      if store.snapshot("t").is_some_and(|s| s.finished) {
        break;
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    unsafe { std::env::remove_var("NO_COLOR") };
    assert_eq!(
      store.snapshot("t").expect("entry").output,
      "unset",
      "a user's NO_COLOR must not silence the terminal cards"
    );
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
