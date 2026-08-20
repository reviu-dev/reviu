use agent_client_protocol::schema::{
  AuthMethod, CancelNotification, ClientCapabilities, ContentBlock, FileSystemCapabilities,
  InitializeRequest, LoadSessionRequest, ModelId, ModelInfo, NewSessionRequest, PermissionOption,
  PromptRequest, ProtocolVersion, ReadTextFileRequest, ReadTextFileResponse,
  RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
  SelectedPermissionOutcome, SessionConfigId, SessionConfigOption, SessionConfigOptionValue,
  SessionId, SessionMode, SessionModeId, SessionNotification, SetSessionConfigOptionRequest,
  SetSessionModeRequest, SetSessionModelRequest, StopReason, TextContent, WriteTextFileRequest,
  WriteTextFileResponse,
};
use agent_client_protocol::{Agent, Client, ConnectionTo};
use anyhow::{Context, Result, anyhow};
use async_channel::{Receiver, Sender, unbounded};
use async_process::Command;

#[cfg(any(test, feature = "test-support"))]
pub mod stub;
mod terminal;
use futures::channel::oneshot;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
pub use terminal::{TerminalSnapshot, TerminalStore};

pub use agent_client_protocol::schema::PermissionOptionKind;
pub use agent_client_protocol::schema::SessionUpdate as AgentEvent;
pub use agent_client_protocol::schema::{
  ModelId as AcpModelId, ModelInfo as AcpModelInfo, SessionConfigId as AcpSessionConfigId,
  SessionConfigKind as AcpSessionConfigKind, SessionConfigOption as AcpSessionConfigOption,
  SessionConfigOptionCategory as AcpSessionConfigOptionCategory,
  SessionConfigOptionValue as AcpSessionConfigOptionValue,
  SessionConfigSelect as AcpSessionConfigSelect,
  SessionConfigSelectGroup as AcpSessionConfigSelectGroup,
  SessionConfigSelectOption as AcpSessionConfigSelectOption,
  SessionConfigSelectOptions as AcpSessionConfigSelectOptions,
  SessionConfigValueId as AcpSessionConfigValueId, SessionMode as AcpSessionMode,
  SessionModeId as AcpSessionModeId,
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PermissionPromptOption {
  pub option_id: String,
  pub label: String,
  pub kind: PermissionOptionKind,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PermissionPrompt {
  pub id: u64,
  pub tool_call_title: String,
  /// The full update from the request: kind, raw input, content and locations,
  /// so the card can show what is actually being approved.
  pub tool_call: agent_client_protocol::schema::ToolCallUpdate,
  pub options: Vec<PermissionPromptOption>,
}

type PermissionReplyMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Option<String>>>>>;

#[derive(Clone, Debug)]
pub struct BackendConfig {
  pub label: String,
  pub command: String,
  pub args: Vec<String>,
  pub env: Vec<(String, String)>,
  /// Agent CLI the adapter shells out to. `None` when the adapter bundles its
  /// own agent, in which case `command` is the whole requirement.
  pub cli_executable: Option<String>,
  pub install_hint: String,
}

impl BackendConfig {
  pub fn new(label: impl Into<String>, command: impl Into<String>, args: Vec<String>) -> Self {
    Self {
      label: label.into(),
      command: command.into(),
      args,
      env: Vec::new(),
      cli_executable: None,
      install_hint: String::new(),
    }
  }

  pub fn env(mut self, env: Vec<(String, String)>) -> Self {
    self.env = env;
    self
  }

  pub fn cli_executable(mut self, cli: Option<impl Into<String>>) -> Self {
    self.cli_executable = cli.map(Into::into);
    self
  }

  pub fn install_hint(mut self, hint: impl Into<String>) -> Self {
    self.install_hint = hint.into();
    self
  }
}

#[derive(Debug, thiserror::Error)]
pub enum BackendAvailability {
  #[error("Available")]
  Ok,
  #[error("`{command}` not found on PATH. {install_hint}")]
  MissingBinary {
    command: String,
    install_hint: String,
  },
}

impl BackendConfig {
  /// Swaps the spawned program (tests and the driver); the packaged args and
  /// the agent CLI belong to the default command, so they are dropped.
  pub fn with_command(mut self, command: impl Into<String>) -> Self {
    self.command = command.into();
    self.args = Vec::new();
    self.cli_executable = None;
    self
  }

  /// Check that the adapter command and the agent CLI it drives are on PATH.
  pub fn check_availability(&self) -> BackendAvailability {
    let missing = [Some(self.command.as_str()), self.cli_executable.as_deref()]
      .into_iter()
      .flatten()
      .find(|binary| which::which(binary).is_err());

    match missing {
      None => BackendAvailability::Ok,
      Some(command) => BackendAvailability::MissingBinary {
        command: command.to_string(),
        install_hint: self.install_hint.clone(),
      },
    }
  }
}

pub fn parse_command_string(s: &str) -> Result<(PathBuf, Vec<String>)> {
  let parts = shell_words::split(s).context("invalid command string")?;
  let first = parts.first().ok_or_else(|| anyhow!("empty command"))?;
  Ok((PathBuf::from(first), parts[1..].to_vec()))
}

/// What the agent did with a steering request (`_session/steering`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SteerOutcome {
  /// Delivered into the running turn; that turn's prompt still owns the end.
  Injected,
  /// No turn was running and the agent started a detached one by itself.
  StartedNewTurn,
  /// No turn was running; the client should send a normal prompt instead.
  PromptRequired,
}

/// Extension method claude-agent-acp exposes to steer the running turn. A
/// plain second `session/prompt` would queue behind it as a fresh turn.
const STEER_METHOD: &str = "_session/steering";

/// Agents advertise the steering extension under `_meta.steering.supported`.
fn parse_steering_support(meta: Option<&agent_client_protocol::schema::Meta>) -> bool {
  meta
    .and_then(|meta| meta.get("steering"))
    .and_then(|steering| steering.get("supported"))
    .and_then(serde_json::Value::as_bool)
    .unwrap_or(false)
}

fn parse_steer_outcome(value: &serde_json::Value) -> SteerOutcome {
  match value.get("outcome").and_then(|v| v.as_str()) {
    Some("startedNewTurn") => SteerOutcome::StartedNewTurn,
    Some("promptRequired") => SteerOutcome::PromptRequired,
    _ => SteerOutcome::Injected,
  }
}

enum DriverCmd {
  Prompt {
    blocks: Vec<ContentBlock>,
    reply: oneshot::Sender<Result<StopReason>>,
  },
  Steer {
    blocks: Vec<ContentBlock>,
    reply: oneshot::Sender<Result<SteerOutcome>>,
  },
  Cancel,
  Stop,
  SetMode {
    mode_id: SessionModeId,
    reply: oneshot::Sender<Result<()>>,
  },
  SetModel {
    model_id: ModelId,
    reply: oneshot::Sender<Result<()>>,
  },
  SetConfigOption {
    config_id: SessionConfigId,
    value: SessionConfigOptionValue,
    reply: oneshot::Sender<Result<()>>,
  },
}

/// Spawns a detached `Future<Output = ()>` on the caller's executor.
pub trait DriverSpawner: Send + Sync + 'static {
  fn spawn(&self, future: futures::future::BoxFuture<'static, ()>);
}

impl<F> DriverSpawner for F
where
  F: Fn(futures::future::BoxFuture<'static, ()>) + Send + Sync + 'static,
{
  fn spawn(&self, future: futures::future::BoxFuture<'static, ()>) {
    self(future);
  }
}

/// Snapshot of metadata returned by the agent during `initialize`.
#[derive(Clone, Debug, Default)]
pub struct AgentInitInfo {
  pub name: Option<String>,
  pub version: Option<String>,
  pub auth_methods: Vec<AuthMethodInfo>,
  pub supports_load_session: bool,
  /// Whether the agent accepts `ContentBlock::Image` in prompts.
  pub supports_images: bool,
  /// Whether the agent answers `_session/steering`; agents without it can only
  /// take a message at the next turn boundary.
  pub supports_steering: bool,
  pub session_id: Option<String>,
  pub available_modes: Vec<SessionMode>,
  pub current_mode_id: Option<SessionModeId>,
  pub available_models: Vec<ModelInfo>,
  pub current_model_id: Option<ModelId>,
  pub config_options: Vec<SessionConfigOption>,
}

#[derive(Clone, Debug)]
pub struct AuthMethodInfo {
  pub id: String,
  pub name: String,
  pub description: Option<String>,
  /// If the agent advertises a terminal-based login, the command line to run.
  pub terminal_command: Option<TerminalAuthCommand>,
}

#[derive(Clone, Debug)]
pub struct TerminalAuthCommand {
  pub args: Vec<String>,
  pub env: Vec<(String, String)>,
}

impl TerminalAuthCommand {
  /// Render as a single shell-safe command string. `base_args` are the
  /// backend's own args, without which the executable resolves to the wrong
  /// program (`npx` alone rather than the packaged adapter).
  pub fn to_shell_string(&self, executable: &str, base_args: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in &self.env {
      parts.push(format!("{}={}", k, shell_words::quote(v)));
    }
    parts.push(executable.to_string());
    for arg in base_args.iter().chain(self.args.iter()) {
      parts.push(shell_words::quote(arg).to_string());
    }
    parts.join(" ")
  }

  /// Try launching the command in the user's native terminal; false on failure.
  pub fn try_launch_terminal(&self, executable: &str, base_args: &[String]) -> bool {
    let shell_cmd = self.to_shell_string(executable, base_args);
    #[cfg(target_os = "macos")]
    {
      let escaped = shell_cmd.replace('\\', "\\\\").replace('"', "\\\"");
      let script = format!("tell application \"Terminal\" to do script \"{escaped}\"");
      return std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .spawn()
        .is_ok();
    }
    #[cfg(target_os = "linux")]
    {
      let bash_cmd = format!("{shell_cmd}; echo; echo 'Press Enter to close'; read _",);
      for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
        if std::process::Command::new(term)
          .arg("-e")
          .arg("bash")
          .arg("-c")
          .arg(&bash_cmd)
          .spawn()
          .is_ok()
        {
          return true;
        }
      }
      return false;
    }
    #[cfg(target_os = "windows")]
    {
      let full = format!("start \"\" cmd /k {shell_cmd}");
      return std::process::Command::new("cmd")
        .arg("/c")
        .arg(full)
        .spawn()
        .is_ok();
    }
    #[allow(unreachable_code)]
    {
      let _ = shell_cmd;
      false
    }
  }
}

/// Multi-turn session against a running ACP agent. Drop kills the child.
pub struct AgentSession {
  cmd_tx: Sender<DriverCmd>,
  event_rx: Option<Receiver<AgentEvent>>,
  permission_rx: Option<Receiver<PermissionPrompt>>,
  permission_replies: PermissionReplyMap,
  init_info: AgentInitInfo,
  terminal_store: Arc<TerminalStore>,
  terminal_updates_rx: Option<Receiver<String>>,
}

impl AgentSession {
  /// Spawn the agent backend and create a new session in `cwd`.
  pub async fn spawn(
    backend: BackendConfig,
    cwd: PathBuf,
    spawner: impl DriverSpawner,
  ) -> Result<Self> {
    Self::spawn_inner(backend, cwd, None, spawner).await
  }

  pub async fn spawn_with_load(
    backend: BackendConfig,
    cwd: PathBuf,
    load_session_id: String,
    spawner: impl DriverSpawner,
  ) -> Result<Self> {
    Self::spawn_inner(backend, cwd, Some(load_session_id), spawner).await
  }

  async fn spawn_inner(
    backend: BackendConfig,
    cwd: PathBuf,
    load_session: Option<String>,
    spawner: impl DriverSpawner,
  ) -> Result<Self> {
    let mut cmd = Command::new(&backend.command);
    cmd.args(&backend.args);
    for (key, value) in &backend.env {
      cmd.env(key, value);
    }
    cmd.current_dir(&cwd);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
      .spawn()
      .with_context(|| format!("spawn {} {:?}", backend.command, backend.args))?;
    let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let stderr = child.stderr.take();
    let transport = agent_client_protocol::ByteStreams::new(stdin, stdout);

    let (cmd_tx, cmd_rx) = unbounded::<DriverCmd>();
    let (event_tx, event_rx) = unbounded::<AgentEvent>();
    let (permission_tx, permission_rx) = unbounded::<PermissionPrompt>();
    let permission_replies: PermissionReplyMap = Arc::new(Mutex::new(HashMap::new()));
    let (terminal_updates_tx, terminal_updates_rx) = unbounded::<String>();
    let terminal_store = TerminalStore::new(terminal_updates_tx);
    let (ready_tx, ready_rx) = oneshot::channel::<Result<AgentInitInfo>>();

    if let Some(stderr) = stderr {
      let stderr_future: futures::future::BoxFuture<'static, ()> = Box::pin(forward_stderr(stderr));
      spawner.spawn(stderr_future);
    }

    let driver_future = Box::pin(run_driver(
      transport,
      cwd,
      load_session,
      cmd_rx,
      event_tx,
      permission_tx,
      permission_replies.clone(),
      terminal_store.clone(),
      ready_tx,
      child,
    ));
    spawner.spawn(driver_future);

    use futures::FutureExt;
    let init_info = futures::select_biased! {
      result = ready_rx.fuse() => match result {
        Ok(Ok(info)) => info,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(anyhow!("Agent process exited before it became ready. Check the agent logs.")),
      },
      _ = smol::Timer::after(Duration::from_secs(60)).fuse() => {
        return Err(anyhow!("Agent did not respond within 60s. Check Node.js installation and network."));
      }
    };

    Ok(Self {
      cmd_tx,
      event_rx: Some(event_rx),
      permission_rx: Some(permission_rx),
      permission_replies,
      init_info,
      terminal_store,
      terminal_updates_rx: Some(terminal_updates_rx),
    })
  }

  /// Live terminals the agent runs through this session.
  pub fn terminal_store(&self) -> Arc<TerminalStore> {
    self.terminal_store.clone()
  }

  /// Channel carrying the id of every terminal whose state changed.
  pub fn take_terminal_updates(&mut self) -> Option<Receiver<String>> {
    self.terminal_updates_rx.take()
  }

  pub fn init_info(&self) -> &AgentInitInfo {
    &self.init_info
  }

  /// Take ownership of the permission prompt stream. Returns `None` if already taken.
  pub fn take_permission_prompts(&mut self) -> Option<Receiver<PermissionPrompt>> {
    self.permission_rx.take()
  }

  /// Answer a pending permission prompt. `option_id = None` cancels.
  pub fn answer_permission(&self, prompt_id: u64, option_id: Option<String>) {
    let sender = self
      .permission_replies
      .lock()
      .ok()
      .and_then(|mut map| map.remove(&prompt_id));
    if let Some(tx) = sender {
      let _ = tx.send(option_id);
    }
  }

  /// Take ownership of the event stream. Returns `None` if already taken.
  pub fn take_events(&mut self) -> Option<Receiver<AgentEvent>> {
    self.event_rx.take()
  }

  /// Cancel the in-flight prompt (best effort).
  pub fn cancel(&self) {
    let _ = self.cmd_tx.try_send(DriverCmd::Cancel);
  }

  /// Set the active session mode (e.g. Plan/Build for Claude, reasoning effort for Codex).
  pub async fn set_mode(&self, mode_id: SessionModeId) -> Result<()> {
    let (tx, rx) = oneshot::channel();
    self
      .cmd_tx
      .send(DriverCmd::SetMode { mode_id, reply: tx })
      .await
      .map_err(|_| anyhow!("agent driver closed"))?;
    rx.await
      .map_err(|_| anyhow!("agent driver dropped reply"))?
  }

  /// Set the active model for the session.
  pub async fn set_model(&self, model_id: ModelId) -> Result<()> {
    let (tx, rx) = oneshot::channel();
    self
      .cmd_tx
      .send(DriverCmd::SetModel {
        model_id,
        reply: tx,
      })
      .await
      .map_err(|_| anyhow!("agent driver closed"))?;
    rx.await
      .map_err(|_| anyhow!("agent driver dropped reply"))?
  }

  /// Set a value on an arbitrary session config option (thinking budget, verbosity, etc).
  pub async fn set_config_option(
    &self,
    config_id: SessionConfigId,
    value: SessionConfigOptionValue,
  ) -> Result<()> {
    let (tx, rx) = oneshot::channel();
    self
      .cmd_tx
      .send(DriverCmd::SetConfigOption {
        config_id,
        value,
        reply: tx,
      })
      .await
      .map_err(|_| anyhow!("agent driver closed"))?;
    rx.await
      .map_err(|_| anyhow!("agent driver dropped reply"))?
  }

  /// Send a plain-text prompt and wait for the agent's `stop_reason`.
  pub async fn send_prompt(&self, text: impl Into<String>) -> Result<StopReason> {
    self
      .send_prompt_blocks(vec![ContentBlock::Text(TextContent::new(text.into()))])
      .await
  }

  /// Send a prompt made of arbitrary content blocks (text, resource links, embedded resources).
  pub async fn send_prompt_blocks(&self, blocks: Vec<ContentBlock>) -> Result<StopReason> {
    let (tx, rx) = oneshot::channel();
    self
      .cmd_tx
      .send(DriverCmd::Prompt { blocks, reply: tx })
      .await
      .map_err(|_| anyhow!("agent driver closed"))?;
    rx.await
      .map_err(|_| anyhow!("agent driver dropped reply"))?
  }

  /// Inject a prompt into the running turn via the steering extension.
  pub async fn steer_prompt_blocks(&self, blocks: Vec<ContentBlock>) -> Result<SteerOutcome> {
    let (tx, rx) = oneshot::channel();
    self
      .cmd_tx
      .send(DriverCmd::Steer { blocks, reply: tx })
      .await
      .map_err(|_| anyhow!("agent driver closed"))?;
    rx.await
      .map_err(|_| anyhow!("agent driver dropped reply"))?
  }
}

impl Drop for AgentSession {
  fn drop(&mut self) {
    let _ = self.cmd_tx.try_send(DriverCmd::Stop);
    self.cmd_tx.close();
  }
}

fn auth_method_id(m: &AuthMethod) -> &str {
  match m {
    AuthMethod::EnvVar(x) => x.id.0.as_ref(),
    AuthMethod::Terminal(x) => x.id.0.as_ref(),
    AuthMethod::Agent(x) => x.id.0.as_ref(),
    _ => "",
  }
}

fn auth_method_name(m: &AuthMethod) -> &str {
  match m {
    AuthMethod::EnvVar(x) => &x.name,
    AuthMethod::Terminal(x) => &x.name,
    AuthMethod::Agent(x) => &x.name,
    _ => "",
  }
}

fn auth_method_description(m: &AuthMethod) -> Option<&str> {
  match m {
    AuthMethod::EnvVar(x) => x.description.as_deref(),
    AuthMethod::Terminal(x) => x.description.as_deref(),
    AuthMethod::Agent(x) => x.description.as_deref(),
    _ => None,
  }
}

fn auth_method_terminal_command(m: &AuthMethod) -> Option<TerminalAuthCommand> {
  match m {
    AuthMethod::Terminal(x) => Some(TerminalAuthCommand {
      args: x.args.clone(),
      env: x.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
    }),
    _ => None,
  }
}

// Prefer AllowOnce over AllowAlways; return None for reject-only sets so
// destructive defaults never auto-apply.
/// Updates that mirror transcript content during a `session/load` replay.
/// Config, mode, model and command updates stay useful and pass through.
fn is_replayable_content(update: &AgentEvent) -> bool {
  matches!(
    update,
    AgentEvent::UserMessageChunk(_)
      | AgentEvent::AgentMessageChunk(_)
      | AgentEvent::AgentThoughtChunk(_)
      | AgentEvent::ToolCall(_)
      | AgentEvent::ToolCallUpdate(_)
      | AgentEvent::Plan(_)
  )
}

fn pick_default_permission_option(
  options: &[PermissionOption],
) -> Option<agent_client_protocol::schema::PermissionOptionId> {
  let by_kind = |kind: &PermissionOptionKind| {
    options
      .iter()
      .find(|o| &o.kind == kind)
      .map(|o| o.option_id.clone())
  };
  by_kind(&PermissionOptionKind::AllowOnce).or_else(|| by_kind(&PermissionOptionKind::AllowAlways))
}

// Resolve `path` against `root`; reject anything that escapes `root` via
// `..` or symlinks. Handles non-existent targets by canonicalizing the
// nearest existing ancestor first.
fn validate_path_in_root(path: &std::path::Path, root: &std::path::Path) -> Result<PathBuf> {
  let root = root
    .canonicalize()
    .with_context(|| format!("canonicalize root {root:?}"))?;
  let mut anchor = path.to_path_buf();
  let mut tail: Vec<std::ffi::OsString> = Vec::new();
  loop {
    if anchor.exists() {
      break;
    }
    match (anchor.parent(), anchor.file_name()) {
      (Some(parent), Some(name)) => {
        tail.push(name.to_os_string());
        anchor = parent.to_path_buf();
      }
      _ => return Err(anyhow!("path has no existing ancestor: {path:?}")),
    }
  }
  let mut resolved = anchor
    .canonicalize()
    .with_context(|| format!("canonicalize {anchor:?}"))?;
  for name in tail.into_iter().rev() {
    resolved.push(name);
  }
  if !resolved.starts_with(&root) {
    return Err(anyhow!(
      "path {resolved:?} is outside the workspace root {root:?}"
    ));
  }
  Ok(resolved)
}

#[cfg(test)]
mod tests {
  use super::*;
  use agent_client_protocol::schema::{PermissionOption, PermissionOptionId};
  use std::fs;

  fn opt(id: &str, kind: PermissionOptionKind) -> PermissionOption {
    PermissionOption::new(PermissionOptionId::new(id), id.to_string(), kind)
  }

  #[test]
  fn truncate_stderr_line_keeps_short_lines_and_caps_long_ones() {
    assert_eq!(truncate_stderr_line("short error"), "short error");

    let long = "x".repeat(STDERR_LINE_MAX_CHARS + 500);
    let truncated = truncate_stderr_line(&long);
    assert!(truncated.starts_with(&"x".repeat(STDERR_LINE_MAX_CHARS)));
    assert!(truncated.ends_with("[... 500 chars truncated]"));
  }

  fn availability_config(command: &str, cli_executable: Option<&str>) -> BackendConfig {
    BackendConfig::new("stub", command, Vec::new())
      .cli_executable(cli_executable)
      .install_hint("install it")
  }

  fn missing_binary(config: BackendConfig) -> Option<String> {
    match config.check_availability() {
      BackendAvailability::MissingBinary { command, .. } => Some(command),
      BackendAvailability::Ok => None,
    }
  }

  #[test]
  fn availability_reports_the_missing_agent_cli_behind_a_present_adapter() {
    let present = std::env::current_exe().expect("current exe");
    let present = present.to_str().expect("utf-8 exe path");

    assert_eq!(missing_binary(availability_config(present, None)), None);
    assert_eq!(
      missing_binary(availability_config(
        present,
        Some("reviu-missing-agent-cli")
      )),
      Some("reviu-missing-agent-cli".to_string())
    );
  }

  #[test]
  fn availability_reports_the_adapter_command_first() {
    assert_eq!(
      missing_binary(availability_config(
        "reviu-missing-adapter",
        Some("reviu-missing-agent-cli")
      )),
      Some("reviu-missing-adapter".to_string())
    );
  }

  #[test]
  fn steering_support_is_read_from_the_initialize_meta() {
    let advertised = serde_json::json!({ "steering": { "supported": true } })
      .as_object()
      .cloned()
      .expect("object");
    assert!(parse_steering_support(Some(&advertised)));

    let declined = serde_json::json!({ "steering": { "supported": false } })
      .as_object()
      .cloned()
      .expect("object");
    assert!(!parse_steering_support(Some(&declined)));

    let unrelated = serde_json::json!({ "goal": { "version": 1 } })
      .as_object()
      .cloned()
      .expect("object");
    assert!(!parse_steering_support(Some(&unrelated)));

    assert!(!parse_steering_support(None));
  }

  #[test]
  fn terminal_auth_keeps_the_backend_args_ahead_of_its_own() {
    let auth = TerminalAuthCommand {
      args: vec!["--terminal-login".into()],
      env: vec![("PI_TOKEN".into(), "a b".into())],
    };
    let config = BackendConfig::new("Pi", "npx", vec!["-y".into(), "pi-acp@0.0.33".into()]);

    assert_eq!(
      auth.to_shell_string(&config.command, &config.args),
      "PI_TOKEN='a b' npx -y pi-acp@0.0.33 --terminal-login"
    );
  }

  #[test]
  fn overriding_the_command_drops_the_agent_cli_requirement() {
    let config = BackendConfig::new("Pi", "npx", vec!["-y".into(), "pi-acp@0.0.33".into()])
      .cli_executable(Some("pi"))
      .with_command("stub_agent");
    assert_eq!(config.cli_executable, None);
    assert!(config.args.is_empty());
  }

  #[test]
  fn parse_command_simple() {
    let (cmd, args) = parse_command_string("npx -y package").unwrap();
    assert_eq!(cmd, PathBuf::from("npx"));
    assert_eq!(args, vec!["-y", "package"]);
  }

  #[test]
  fn parse_command_quoted() {
    let (cmd, args) = parse_command_string(r#"my-cmd "arg with space" other"#).unwrap();
    assert_eq!(cmd, PathBuf::from("my-cmd"));
    assert_eq!(args, vec!["arg with space", "other"]);
  }

  #[test]
  fn parse_command_empty() {
    assert!(parse_command_string("").is_err());
  }

  #[test]
  fn permission_prefers_allow_once() {
    let options = vec![
      opt("reject", PermissionOptionKind::RejectOnce),
      opt("allow-always", PermissionOptionKind::AllowAlways),
      opt("allow", PermissionOptionKind::AllowOnce),
    ];
    let chosen = pick_default_permission_option(&options).unwrap();
    assert_eq!(chosen.0.as_ref(), "allow");
  }

  #[test]
  fn permission_falls_back_to_allow_always() {
    let options = vec![
      opt("reject", PermissionOptionKind::RejectOnce),
      opt("allow-always", PermissionOptionKind::AllowAlways),
    ];
    let chosen = pick_default_permission_option(&options).unwrap();
    assert_eq!(chosen.0.as_ref(), "allow-always");
  }

  #[test]
  fn permission_cancels_when_no_allow_option() {
    let options = vec![
      opt("reject", PermissionOptionKind::RejectOnce),
      opt("reject-always", PermissionOptionKind::RejectAlways),
    ];
    assert!(pick_default_permission_option(&options).is_none());
  }

  #[test]
  fn permission_empty_options_cancels() {
    assert!(pick_default_permission_option(&[]).is_none());
  }

  #[test]
  fn sandbox_allows_path_inside_root() {
    let tmp = tempdir();
    let file = tmp.join("inside.txt");
    fs::write(&file, "x").unwrap();
    let resolved = validate_path_in_root(&file, &tmp).unwrap();
    assert!(resolved.starts_with(tmp.canonicalize().unwrap()));
  }

  #[test]
  fn sandbox_allows_nonexistent_file_with_existing_parent() {
    let tmp = tempdir();
    let new_file = tmp.join("subdir").join("new.txt");
    fs::create_dir_all(new_file.parent().unwrap()).unwrap();
    let resolved = validate_path_in_root(&new_file, &tmp).unwrap();
    assert!(resolved.starts_with(tmp.canonicalize().unwrap()));
  }

  #[test]
  fn sandbox_blocks_path_outside_root() {
    let tmp = tempdir();
    let outside = std::env::temp_dir().join("totally-outside-reviu-test.txt");
    fs::write(&outside, "x").unwrap();
    let err = validate_path_in_root(&outside, &tmp);
    let _ = fs::remove_file(&outside);
    assert!(err.is_err());
  }

  #[test]
  fn sandbox_blocks_dotdot_escape() {
    let tmp = tempdir();
    let escape = tmp.join("..").join("escape.txt");
    let err = validate_path_in_root(&escape, &tmp);
    assert!(err.is_err());
  }

  /// Two fixtures created in the same clock tick would otherwise share a directory.
  static TEMP_DIR_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

  fn tempdir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .expect("system clock before unix epoch")
      .as_nanos();
    let unique = TEMP_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
      "reviu-agent-acp-test-{}-{nanos}-{unique}",
      std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }
}

/// Adapters occasionally dump huge payloads on stderr (model instruction templates,
/// full API bodies); cap each line so real errors above stay visible.
const STDERR_LINE_MAX_CHARS: usize = 2000;

fn truncate_stderr_line(line: &str) -> std::borrow::Cow<'_, str> {
  if line.chars().count() <= STDERR_LINE_MAX_CHARS {
    return std::borrow::Cow::Borrowed(line);
  }
  let head: String = line.chars().take(STDERR_LINE_MAX_CHARS).collect();
  let dropped = line.chars().count() - STDERR_LINE_MAX_CHARS;
  std::borrow::Cow::Owned(format!("{head} [... {dropped} chars truncated]"))
}

async fn forward_stderr(stderr: async_process::ChildStderr) {
  use futures::io::{AsyncBufReadExt, BufReader};
  let reader = BufReader::new(stderr);
  let mut lines = reader.lines();
  while let Some(Ok(line)) = futures::stream::StreamExt::next(&mut lines).await {
    eprintln!("[acp-server] {}", truncate_stderr_line(&line));
  }
}

async fn run_driver(
  transport: agent_client_protocol::ByteStreams<
    async_process::ChildStdin,
    async_process::ChildStdout,
  >,
  cwd: PathBuf,
  load_session: Option<String>,
  cmd_rx: Receiver<DriverCmd>,
  event_tx: Sender<AgentEvent>,
  permission_tx: Sender<PermissionPrompt>,
  permission_replies: PermissionReplyMap,
  terminal_store: Arc<TerminalStore>,
  ready_tx: oneshot::Sender<Result<AgentInitInfo>>,
  mut child: async_process::Child,
) {
  let event_tx_inner = event_tx.clone();
  let fs_root_read = cwd.clone();
  let fs_root_write = cwd.clone();
  let terminal_cwd = cwd.clone();
  let terminal_counter = Arc::new(AtomicU64::new(1));
  let term_create = terminal_store.clone();
  let term_output = terminal_store.clone();
  let term_wait = terminal_store.clone();
  let term_kill = terminal_store.clone();
  let term_release = terminal_store.clone();
  let permission_counter = Arc::new(AtomicU64::new(1));
  // `session/load` replays the whole session history as ordinary updates; the
  // host already holds the transcript, so replayed content must not reach it.
  let replaying = Arc::new(std::sync::atomic::AtomicBool::new(false));
  let replaying_gate = replaying.clone();
  let term_meta = terminal_store.clone();
  let result = Client
    .builder()
    .on_receive_notification(
      async move |notification: SessionNotification, _cx| {
        // Codex-style agents run commands themselves and stream the output
        // through tool-call metadata; feed it into the terminal store even
        // during a replay, so reloaded conversations keep their output.
        crate::terminal::inspect_session_update(&term_meta, &notification.update);
        if replaying_gate.load(Ordering::Relaxed) && is_replayable_content(&notification.update) {
          return Ok(());
        }
        let _ = event_tx_inner.send(notification.update).await;
        Ok(())
      },
      agent_client_protocol::on_receive_notification!(),
    )
    .on_receive_request(
      {
        let permission_tx = permission_tx.clone();
        let permission_replies = permission_replies.clone();
        let permission_counter = permission_counter.clone();
        async move |request: RequestPermissionRequest, responder, _connection| {
          use futures::FutureExt;
          let id = permission_counter.fetch_add(1, Ordering::Relaxed);
          let title = request
            .tool_call
            .fields
            .title
            .clone()
            .unwrap_or_else(|| "Permission required".to_string());
          let prompt = PermissionPrompt {
            id,
            tool_call_title: title,
            tool_call: request.tool_call.clone(),
            options: request
              .options
              .iter()
              .map(|o| PermissionPromptOption {
                option_id: o.option_id.0.to_string(),
                label: o.name.clone(),
                kind: o.kind,
              })
              .collect(),
          };
          let (reply_tx, reply_rx) = oneshot::channel::<Option<String>>();
          if let Ok(mut map) = permission_replies.lock() {
            map.insert(id, reply_tx);
          }
          let send_result = permission_tx.send(prompt).await;

          let outcome = if send_result.is_err() {
            // No listener: fall back to safe default.
            permission_replies.lock().ok().map(|mut m| m.remove(&id));
            match pick_default_permission_option(&request.options) {
              Some(id) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id)),
              None => RequestPermissionOutcome::Cancelled,
            }
          } else {
            let timeout = smol::Timer::after(Duration::from_secs(300)).fuse();
            futures::pin_mut!(timeout);
            futures::select_biased! {
              answer = reply_rx.fuse() => {
                match answer.ok().flatten() {
                  Some(option_id) => RequestPermissionOutcome::Selected(
                    SelectedPermissionOutcome::new(option_id),
                  ),
                  None => RequestPermissionOutcome::Cancelled,
                }
              }
              _ = timeout => {
                permission_replies.lock().ok().map(|mut m| m.remove(&id));
                RequestPermissionOutcome::Cancelled
              }
            }
          };
          responder.respond(RequestPermissionResponse::new(outcome))
        }
      },
      agent_client_protocol::on_receive_request!(),
    )
    .on_receive_request(
      async move |request: ReadTextFileRequest, responder, _connection| {
        let root = fs_root_read.clone();
        let content = smol::unblock(move || -> Result<String> {
          let resolved = validate_path_in_root(&request.path, &root)?;
          let mut text =
            std::fs::read_to_string(&resolved).with_context(|| format!("read {resolved:?}"))?;
          if let Some(line) = request.line {
            let start = line.saturating_sub(1) as usize;
            let mut lines: Vec<&str> = text.lines().collect();
            if start >= lines.len() {
              text = String::new();
            } else {
              let end = match request.limit {
                Some(limit) => (start + limit as usize).min(lines.len()),
                None => lines.len(),
              };
              lines = lines[start..end].to_vec();
              text = lines.join("\n");
            }
          }
          Ok(text)
        })
        .await;
        match content {
          Ok(text) => responder.respond(ReadTextFileResponse::new(text)),
          Err(e) => Err(
            agent_client_protocol::Error::internal_error()
              .data(serde_json::Value::String(e.to_string())),
          ),
        }
      },
      agent_client_protocol::on_receive_request!(),
    )
    .on_receive_request(
      async move |request: WriteTextFileRequest, responder, _connection| {
        let root = fs_root_write.clone();
        let result = smol::unblock(move || -> Result<()> {
          let resolved = validate_path_in_root(&request.path, &root)?;
          if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent)
              .with_context(|| format!("create_dir_all {parent:?}"))?;
          }
          std::fs::write(&resolved, request.content)
            .with_context(|| format!("write {resolved:?}"))?;
          Ok(())
        })
        .await;
        match result {
          Ok(()) => responder.respond(WriteTextFileResponse::new()),
          Err(e) => Err(
            agent_client_protocol::Error::internal_error()
              .data(serde_json::Value::String(e.to_string())),
          ),
        }
      },
      agent_client_protocol::on_receive_request!(),
    )
    .on_receive_request(
      async move |request: agent_client_protocol::schema::CreateTerminalRequest,
                  responder,
                  _connection| {
        let id = format!("term-{}", terminal_counter.fetch_add(1, Ordering::Relaxed));
        let env = request.env.into_iter().map(|v| (v.name, v.value)).collect();
        let cwd = request.cwd.unwrap_or_else(|| terminal_cwd.clone());
        match crate::terminal::spawn_terminal(
          &term_create,
          id.clone(),
          request.command,
          request.args,
          env,
          cwd,
          request.output_byte_limit,
        ) {
          Ok(()) => responder.respond(agent_client_protocol::schema::CreateTerminalResponse::new(
            agent_client_protocol::schema::TerminalId::new(std::sync::Arc::<str>::from(
              id.as_str(),
            )),
          )),
          Err(e) => Err(
            agent_client_protocol::Error::internal_error()
              .data(serde_json::Value::String(e.to_string())),
          ),
        }
      },
      agent_client_protocol::on_receive_request!(),
    )
    .on_receive_request(
      async move |request: agent_client_protocol::schema::TerminalOutputRequest,
                  responder,
                  _connection| {
        match term_output.snapshot(request.terminal_id.0.as_ref()) {
          Some(snap) => {
            let mut response = agent_client_protocol::schema::TerminalOutputResponse::new(
              snap.output,
              snap.truncated,
            );
            if snap.finished {
              response = response.exit_status(
                agent_client_protocol::schema::TerminalExitStatus::new()
                  .exit_code(snap.exit_code)
                  .signal(snap.signal),
              );
            }
            responder.respond(response)
          }
          None => Err(agent_client_protocol::Error::invalid_params()),
        }
      },
      agent_client_protocol::on_receive_request!(),
    )
    .on_receive_request(
      async move |request: agent_client_protocol::schema::WaitForTerminalExitRequest,
                  responder,
                  _connection| {
        let id = request.terminal_id.0.to_string();
        loop {
          match term_wait.snapshot(&id) {
            Some(snap) if snap.finished => {
              return responder.respond(
                agent_client_protocol::schema::WaitForTerminalExitResponse::new(
                  agent_client_protocol::schema::TerminalExitStatus::new()
                    .exit_code(snap.exit_code)
                    .signal(snap.signal),
                ),
              );
            }
            Some(_) => smol::Timer::after(Duration::from_millis(30)).await,
            None => return Err(agent_client_protocol::Error::invalid_params()),
          };
        }
      },
      agent_client_protocol::on_receive_request!(),
    )
    .on_receive_request(
      async move |request: agent_client_protocol::schema::KillTerminalRequest,
                  responder,
                  _connection| {
        term_kill.kill(request.terminal_id.0.as_ref());
        responder.respond(agent_client_protocol::schema::KillTerminalResponse::new())
      },
      agent_client_protocol::on_receive_request!(),
    )
    .on_receive_request(
      async move |request: agent_client_protocol::schema::ReleaseTerminalRequest,
                  responder,
                  _connection| {
        term_release.release(request.terminal_id.0.as_ref());
        responder.respond(agent_client_protocol::schema::ReleaseTerminalResponse::new())
      },
      agent_client_protocol::on_receive_request!(),
    )
    .connect_with(transport, |connection: ConnectionTo<Agent>| async move {
      let capabilities = ClientCapabilities::new()
        .fs(
          FileSystemCapabilities::new()
            .read_text_file(true)
            .write_text_file(true),
        )
        .terminal(true);
      let init = connection
        .send_request(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(capabilities))
        .block_task()
        .await?;
      let info = AgentInitInfo {
        name: init.agent_info.as_ref().map(|i| i.name.clone()),
        version: init.agent_info.as_ref().map(|i| i.version.clone()),
        auth_methods: init
          .auth_methods
          .iter()
          .map(|m| AuthMethodInfo {
            id: auth_method_id(m).to_string(),
            name: auth_method_name(m).to_string(),
            description: auth_method_description(m).map(str::to_string),
            terminal_command: auth_method_terminal_command(m),
          })
          .collect(),
        supports_load_session: init.agent_capabilities.load_session,
        supports_images: init.agent_capabilities.prompt_capabilities.image,
        supports_steering: parse_steering_support(init.meta.as_ref()),
        session_id: None,
        available_modes: Vec::new(),
        current_mode_id: None,
        available_models: Vec::new(),
        current_model_id: None,
        config_options: Vec::new(),
      };
      let (session_id, modes, models, config_options) = match load_session.clone() {
        Some(id) if info.supports_load_session => {
          let sid = SessionId::new(id);
          replaying.store(true, Ordering::Relaxed);
          let loaded = connection
            .send_request(LoadSessionRequest::new(sid.clone(), cwd.clone()))
            .block_task()
            .await;
          replaying.store(false, Ordering::Relaxed);
          match loaded {
            Ok(resp) => (sid, resp.modes, resp.models, resp.config_options),
            // Agents often cannot restore sessions after a restart; fall back to a
            // fresh session instead of failing the whole connection. The local
            // transcript is kept by the host, only provider-side context is lost.
            Err(e) => {
              eprintln!("[agent] failed to load saved session, starting fresh: {e}");
              let resp = connection
                .send_request(NewSessionRequest::new(cwd.clone()))
                .block_task()
                .await?;
              (
                resp.session_id,
                resp.modes,
                resp.models,
                resp.config_options,
              )
            }
          }
        }
        _ => {
          let resp = connection
            .send_request(NewSessionRequest::new(cwd.clone()))
            .block_task()
            .await?;
          (
            resp.session_id,
            resp.modes,
            resp.models,
            resp.config_options,
          )
        }
      };
      let (available_modes, current_mode_id) = match modes {
        Some(s) => (s.available_modes, Some(s.current_mode_id)),
        None => (Vec::new(), None),
      };
      let (available_models, current_model_id) = match models {
        Some(s) => (s.available_models, Some(s.current_model_id)),
        None => (Vec::new(), None),
      };
      let info_with_session = AgentInitInfo {
        session_id: Some(session_id.0.to_string()),
        available_modes,
        current_mode_id,
        available_models,
        current_model_id,
        config_options: config_options.unwrap_or_default(),
        ..info
      };

      let _ = ready_tx.send(Ok(info_with_session));

      use futures::FutureExt;

      let map_prompt_response = |response: std::result::Result<
        agent_client_protocol::schema::PromptResponse,
        agent_client_protocol::Error,
      >| {
        response.map(|r| r.stop_reason).map_err(|e| {
          if e.code == agent_client_protocol::schema::ErrorCode::AuthRequired {
            anyhow!("auth_required: {e}")
          } else {
            anyhow!("acp prompt error: {e}")
          }
        })
      };

      'outer: while let Ok(cmd) = cmd_rx.recv().await {
        match cmd {
          // A steer landing outside a turn: nothing to inject into.
          DriverCmd::Steer { reply, .. } => {
            let _ = reply.send(Ok(SteerOutcome::PromptRequired));
          }
          DriverCmd::Prompt { blocks, reply } => {
            let prompt_fut = connection
              .send_request(PromptRequest::new(session_id.clone(), blocks))
              .block_task()
              .fuse();
            futures::pin_mut!(prompt_fut);
            // Steers injected while this prompt runs; each waits for its own
            // stop reason without replacing the main prompt.
            let mut steers: futures::stream::FuturesUnordered<futures::future::BoxFuture<'_, _>> =
              futures::stream::FuturesUnordered::new();
            let mut response_opt = None;
            loop {
              if response_opt.is_some() && steers.is_empty() {
                break;
              }
              use futures::StreamExt as _;
              futures::select_biased! {
                next_cmd = cmd_rx.recv().fuse() => {
                  match next_cmd {
                    Ok(DriverCmd::Cancel) => {
                      let _ = connection
                        .send_notification(CancelNotification::new(session_id.clone()));
                    }
                    Ok(DriverCmd::Stop) => {
                      let _ = reply.send(Err(anyhow!("agent driver stopping")));
                      break 'outer;
                    }
                    Ok(DriverCmd::Prompt { reply: other_reply, .. }) => {
                      let _ = other_reply.send(Err(anyhow!("another prompt in flight")));
                    }
                    Ok(DriverCmd::Steer { blocks, reply: steer_reply }) => {
                      let params = serde_json::json!({
                        "sessionId": session_id,
                        "prompt": blocks,
                        "_meta": { "steering": { "idleBehavior": "promptRequired" } },
                      });
                      match agent_client_protocol::UntypedMessage::new(STEER_METHOD, params) {
                        Ok(msg) => {
                          let fut = connection.send_request(msg).block_task();
                          steers.push(Box::pin(async move { (steer_reply, fut.await) }));
                        }
                        Err(e) => {
                          let _ = steer_reply.send(Err(anyhow!("steer: {e}")));
                        }
                      }
                    }
                    Ok(DriverCmd::SetMode { mode_id, reply: set_reply }) => {
                      let r = connection
                        .send_request(SetSessionModeRequest::new(session_id.clone(), mode_id))
                        .block_task()
                        .await
                        .map(|_| ())
                        .map_err(|e| anyhow!("set_mode: {e}"));
                      let _ = set_reply.send(r);
                    }
                    Ok(DriverCmd::SetModel { model_id, reply: set_reply }) => {
                      let r = connection
                        .send_request(SetSessionModelRequest::new(session_id.clone(), model_id))
                        .block_task()
                        .await
                        .map(|_| ())
                        .map_err(|e| anyhow!("set_model: {e}"));
                      let _ = set_reply.send(r);
                    }
                    Ok(DriverCmd::SetConfigOption { config_id, value, reply: set_reply }) => {
                      let r = connection
                        .send_request(SetSessionConfigOptionRequest::new(
                          session_id.clone(),
                          config_id,
                          value,
                        ))
                        .block_task()
                        .await
                        .map(|_| ())
                        .map_err(|e| anyhow!("set_config_option: {e}"));
                      let _ = set_reply.send(r);
                    }
                    Err(_) => {
                      let _ = reply.send(Err(anyhow!("agent driver closed")));
                      break 'outer;
                    }
                  }
                }
                steer_done = steers.select_next_some() => {
                  let (steer_reply, response): (
                    oneshot::Sender<Result<SteerOutcome>>,
                    std::result::Result<serde_json::Value, agent_client_protocol::Error>,
                  ) = steer_done;
                  let mapped = response
                    .map(|value| parse_steer_outcome(&value))
                    .map_err(|e| anyhow!("steer: {e}"));
                  let _ = steer_reply.send(mapped);
                }
                response = prompt_fut => {
                  response_opt = Some(response);
                },
              }
            }
            if let Some(response) = response_opt {
              let _ = reply.send(map_prompt_response(response));
            }
          }
          DriverCmd::Cancel => {
            let _ = connection.send_notification(CancelNotification::new(session_id.clone()));
          }
          DriverCmd::Stop => break,
          DriverCmd::SetMode { mode_id, reply } => {
            let r = connection
              .send_request(SetSessionModeRequest::new(session_id.clone(), mode_id))
              .block_task()
              .await
              .map(|_| ())
              .map_err(|e| anyhow!("set_mode: {e}"));
            let _ = reply.send(r);
          }
          DriverCmd::SetModel { model_id, reply } => {
            let r = connection
              .send_request(SetSessionModelRequest::new(session_id.clone(), model_id))
              .block_task()
              .await
              .map(|_| ())
              .map_err(|e| anyhow!("set_model: {e}"));
            let _ = reply.send(r);
          }
          DriverCmd::SetConfigOption {
            config_id,
            value,
            reply,
          } => {
            let r = connection
              .send_request(SetSessionConfigOptionRequest::new(
                session_id.clone(),
                config_id,
                value,
              ))
              .block_task()
              .await
              .map(|_| ())
              .map_err(|e| anyhow!("set_config_option: {e}"));
            let _ = reply.send(r);
          }
        }
      }
      Ok(())
    })
    .await;

  if let Err(e) = result {
    eprintln!("[agent_acp] driver exited: {e}");
  }
  let _ = child.kill();
}
