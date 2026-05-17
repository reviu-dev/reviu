use agent_client_protocol::schema::{
  AuthMethod, CancelNotification, ClientCapabilities, ContentBlock, FileSystemCapabilities,
  InitializeRequest, NewSessionRequest, PermissionOption, PromptRequest, ProtocolVersion,
  ReadTextFileRequest, ReadTextFileResponse, RequestPermissionOutcome, RequestPermissionRequest,
  RequestPermissionResponse, SelectedPermissionOutcome, SessionNotification, StopReason,
  TextContent, WriteTextFileRequest, WriteTextFileResponse,
};
use agent_client_protocol::{Agent, Client, ConnectionTo};
use anyhow::{Context, Result, anyhow};
use async_channel::{Receiver, Sender, unbounded};
use async_process::Command;
use futures::channel::oneshot;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub use agent_client_protocol::schema::PermissionOptionKind;
pub use agent_client_protocol::schema::SessionUpdate as AgentEvent;

#[derive(Clone, Debug)]
pub struct PermissionPromptOption {
  pub option_id: String,
  pub label: String,
  pub kind: PermissionOptionKind,
}

#[derive(Clone, Debug)]
pub struct PermissionPrompt {
  pub id: u64,
  pub tool_call_title: String,
  pub options: Vec<PermissionPromptOption>,
}

type PermissionReplyMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Option<String>>>>>;

#[derive(Clone, Debug)]
pub struct BackendConfig {
  pub label: &'static str,
  pub command: &'static str,
  pub args: Vec<String>,
  pub install_hint: &'static str,
}

impl BackendConfig {
  pub fn claude() -> Self {
    Self {
      label: "Claude",
      command: "npx",
      args: vec![
        "-y".into(),
        "@agentclientprotocol/claude-agent-acp@0.35.0".into(),
      ],
      install_hint: "Requires Node.js. The package is fetched via npx on first run. Sign in with `claude /login` to use your subscription.",
    }
  }

  pub fn codex() -> Self {
    Self {
      label: "Codex",
      command: "npx",
      args: vec!["-y".into(), "@zed-industries/codex-acp@0.9.4".into()],
      install_hint: "Requires Node.js and `codex login` for ChatGPT subscription auth.",
    }
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
  /// Check if the backend binary is reachable on PATH.
  pub fn check_availability(&self) -> BackendAvailability {
    if which::which(self.command).is_ok() {
      BackendAvailability::Ok
    } else {
      BackendAvailability::MissingBinary {
        command: self.command.to_string(),
        install_hint: self.install_hint.to_string(),
      }
    }
  }
}

pub fn parse_command_string(s: &str) -> Result<(PathBuf, Vec<String>)> {
  let parts = shell_words::split(s).context("invalid command string")?;
  let first = parts.first().ok_or_else(|| anyhow!("empty command"))?;
  Ok((PathBuf::from(first), parts[1..].to_vec()))
}

enum DriverCmd {
  Prompt {
    text: String,
    reply: oneshot::Sender<Result<StopReason>>,
  },
  Cancel,
  Stop,
}

/// Spawn handle used to launch the driver task. Caller passes a GPUI / smol /
/// custom executor that knows how to run a detached `Future<Output = ()>`.
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
  /// Render as a single shell-safe command string.
  pub fn to_shell_string(&self, executable: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in &self.env {
      parts.push(format!("{}={}", k, shell_words::quote(v)));
    }
    parts.push(executable.to_string());
    for arg in &self.args {
      parts.push(shell_words::quote(arg).to_string());
    }
    parts.join(" ")
  }

  /// Try to launch the command in the user's native terminal. Returns true if
  /// a terminal was spawned. Only macOS is currently supported; other
  /// platforms should fall back to clipboard copy.
  pub fn try_launch_terminal(&self, executable: &str) -> bool {
    let shell_cmd = self.to_shell_string(executable);
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
    #[cfg(not(target_os = "macos"))]
    {
      let _ = shell_cmd;
      false
    }
  }
}

/// Multi-turn session against a running ACP agent.
///
/// Dropping the session signals the driver to stop and kills the child.
pub struct AgentSession {
  cmd_tx: Sender<DriverCmd>,
  event_rx: Option<Receiver<AgentEvent>>,
  permission_rx: Option<Receiver<PermissionPrompt>>,
  permission_replies: PermissionReplyMap,
  init_info: AgentInitInfo,
}

impl AgentSession {
  /// Spawn the agent backend and create a new session in `cwd`.
  ///
  /// `spawner` runs the long-lived driver future on the caller's executor.
  pub async fn spawn(
    backend: BackendConfig,
    cwd: PathBuf,
    spawner: impl DriverSpawner,
  ) -> Result<Self> {
    let mut cmd = Command::new(backend.command);
    cmd.args(&backend.args);
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
    let (ready_tx, ready_rx) = oneshot::channel::<Result<AgentInitInfo>>();

    if let Some(stderr) = stderr {
      let stderr_future: futures::future::BoxFuture<'static, ()> = Box::pin(forward_stderr(stderr));
      spawner.spawn(stderr_future);
    }

    let driver_future = Box::pin(run_driver(
      transport,
      cwd,
      cmd_rx,
      event_tx,
      permission_tx,
      permission_replies.clone(),
      ready_tx,
      child,
    ));
    spawner.spawn(driver_future);

    use futures::FutureExt;
    let init_info = futures::select_biased! {
      result = ready_rx.fuse() => match result {
        Ok(Ok(info)) => info,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(anyhow!("agent driver failed before ready")),
      },
      _ = smol::Timer::after(Duration::from_secs(60)).fuse() => {
        return Err(anyhow!("agent did not respond within 60s (check Node.js / network)"));
      }
    };

    Ok(Self {
      cmd_tx,
      event_rx: Some(event_rx),
      permission_rx: Some(permission_rx),
      permission_replies,
      init_info,
    })
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

  /// Send a prompt and wait for the agent's `stop_reason`.
  pub async fn send_prompt(&self, text: impl Into<String>) -> Result<StopReason> {
    let (tx, rx) = oneshot::channel();
    self
      .cmd_tx
      .send(DriverCmd::Prompt {
        text: text.into(),
        reply: tx,
      })
      .await
      .map_err(|_| anyhow!("agent driver closed"))?;
    rx.await.map_err(|_| anyhow!("agent driver dropped reply"))?
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

/// Pick the safest reasonable permission option. Prefers `AllowOnce` (no
/// memory of choice) over `AllowAlways`; returns `None` (cancel) if no
/// allow-style option is available, so destructive defaults never auto-apply.
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

/// Returns Ok if `path` resolves to a location inside `root`.
///
/// For writes, the target file may not exist yet, so we canonicalize the
/// closest existing ancestor and join the remaining tail before checking.
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

  fn tempdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "reviu-agent-acp-test-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }
}

async fn forward_stderr(stderr: async_process::ChildStderr) {
  use futures::io::{AsyncBufReadExt, BufReader};
  let reader = BufReader::new(stderr);
  let mut lines = reader.lines();
  while let Some(Ok(line)) = futures::stream::StreamExt::next(&mut lines).await {
    eprintln!("[acp-server] {line}");
  }
}

async fn run_driver(
  transport: agent_client_protocol::ByteStreams<async_process::ChildStdin, async_process::ChildStdout>,
  cwd: PathBuf,
  cmd_rx: Receiver<DriverCmd>,
  event_tx: Sender<AgentEvent>,
  permission_tx: Sender<PermissionPrompt>,
  permission_replies: PermissionReplyMap,
  ready_tx: oneshot::Sender<Result<AgentInitInfo>>,
  mut child: async_process::Child,
) {
  let event_tx_inner = event_tx.clone();
  let fs_root_read = cwd.clone();
  let fs_root_write = cwd.clone();
  let permission_counter = Arc::new(AtomicU64::new(1));
  let result = Client
    .builder()
    .on_receive_notification(
      async move |notification: SessionNotification, _cx| {
        let _ = event_tx_inner.send(notification.update).await;
        Ok(())
      },
      agent_client_protocol::on_receive_notification!(),
    )
    .on_receive_request({
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
          options: request
            .options
            .iter()
            .map(|o| PermissionPromptOption {
              option_id: o.option_id.0.to_string(),
              label: o.name.clone(),
              kind: o.kind.clone(),
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
          let mut text = std::fs::read_to_string(&resolved)
            .with_context(|| format!("read {resolved:?}"))?;
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
          Err(e) => Err(agent_client_protocol::Error::internal_error().data(
            serde_json::Value::String(e.to_string()),
          )),
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
          Err(e) => Err(agent_client_protocol::Error::internal_error().data(
            serde_json::Value::String(e.to_string()),
          )),
        }
      },
      agent_client_protocol::on_receive_request!(),
    )
    .connect_with(transport, |connection: ConnectionTo<Agent>| async move {
      let capabilities = ClientCapabilities::new().fs(
        FileSystemCapabilities::new()
          .read_text_file(true)
          .write_text_file(true),
      );
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
      };
      let session = connection
        .send_request(NewSessionRequest::new(cwd))
        .block_task()
        .await?;
      let session_id = session.session_id;

      let _ = ready_tx.send(Ok(info));

      use futures::FutureExt;

      'outer: while let Ok(cmd) = cmd_rx.recv().await {
        match cmd {
          DriverCmd::Prompt { text, reply } => {
            let prompt_fut = connection
              .send_request(PromptRequest::new(
                session_id.clone(),
                vec![ContentBlock::Text(TextContent::new(text))],
              ))
              .block_task()
              .fuse();
            futures::pin_mut!(prompt_fut);
            let response_opt;
            loop {
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
                    Err(_) => {
                      let _ = reply.send(Err(anyhow!("agent driver closed")));
                      break 'outer;
                    }
                  }
                }
                response = prompt_fut => {
                  response_opt = Some(response);
                  break;
                },
              }
            }
            if let Some(response) = response_opt {
              let mapped = response
                .map(|r| r.stop_reason)
                .map_err(|e| anyhow!("acp prompt error: {e}"));
              let _ = reply.send(mapped);
            }
          }
          DriverCmd::Cancel => {
            let _ = connection.send_notification(CancelNotification::new(session_id.clone()));
          }
          DriverCmd::Stop => break,
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
