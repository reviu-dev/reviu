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
}

struct TerminalEntry {
  snapshot: TerminalSnapshot,
  byte_limit: usize,
  kill_tx: Option<oneshot::Sender<()>>,
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
      entry
        .snapshot
        .output
        .push_str(&String::from_utf8_lossy(chunk));
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
      entry.snapshot.exit_code = exit_code;
      entry.snapshot.signal = signal;
      entry.snapshot.finished = true;
      entry.snapshot.killed = killed;
      entry.kill_tx = None;
    }
    self.notify(id);
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
          ..Default::default()
        },
        byte_limit: output_byte_limit
          .map(|l| l as usize)
          .unwrap_or(DEFAULT_OUTPUT_BYTE_LIMIT),
        kill_tx: Some(kill_tx),
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
