use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

const DEFAULT_BACKEND: &str = "test";
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Clone, Debug, PartialEq, Eq)]
struct ServerConfig {
  backend: String,
  driver_bin: Option<PathBuf>,
  agent_command: Option<String>,
}

impl Default for ServerConfig {
  fn default() -> Self {
    Self {
      backend: DEFAULT_BACKEND.to_string(),
      driver_bin: None,
      agent_command: None,
    }
  }
}

fn main() -> Result<()> {
  let config = parse_args(std::env::args().skip(1))?;
  McpServer::new(config).run()
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<ServerConfig> {
  let mut config = ServerConfig::default();
  let mut args = args.into_iter();
  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--backend" => config.backend = required_value(&mut args, "--backend")?,
      "--driver-bin" => {
        config.driver_bin = Some(PathBuf::from(required_value(&mut args, "--driver-bin")?))
      }
      "--agent-command" => {
        config.agent_command = Some(required_value(&mut args, "--agent-command")?)
      }
      "--help" | "-h" => bail!(usage()),
      other if other.starts_with("--backend=") => {
        config.backend = other.trim_start_matches("--backend=").to_string();
      }
      other if other.starts_with("--driver-bin=") => {
        config.driver_bin = Some(PathBuf::from(other.trim_start_matches("--driver-bin=")));
      }
      other if other.starts_with("--agent-command=") => {
        config.agent_command = Some(other.trim_start_matches("--agent-command=").to_string());
      }
      other => bail!("unknown argument: {other}\n{}", usage()),
    }
  }
  validate_backend(&config.backend)?;
  Ok(config)
}

fn required_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
  args.next().with_context(|| format!("{flag} needs a value"))
}

fn validate_backend(backend: &str) -> Result<()> {
  match backend {
    "test" | "visual" => Ok(()),
    other => bail!("unknown backend: {other}"),
  }
}

fn usage() -> &'static str {
  "usage: reviu-driver-mcp [--backend test|visual] [--driver-bin PATH] [--agent-command PATH]"
}

struct McpServer {
  config: ServerConfig,
  driver: DriverSession,
}

impl McpServer {
  fn new(config: ServerConfig) -> Self {
    Self {
      driver: DriverSession::new(config.clone()),
      config,
    }
  }

  fn run(&mut self) -> Result<()> {
    let stdin = std::io::stdin().lock();
    for line in stdin.lines() {
      let line = line?;
      if line.trim().is_empty() {
        continue;
      }
      match self.handle_line(&line) {
        Ok(Some(response)) => respond(response),
        Ok(None) => {}
        Err(error) => respond(json_rpc_error(Value::Null, -32603, error.to_string())),
      }
    }
    self.driver.stop();
    Ok(())
  }

  fn handle_line(&mut self, line: &str) -> Result<Option<Value>> {
    let request: Value = serde_json::from_str(line).context("parse MCP request")?;
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let id = request.get("id").cloned();

    match method {
      "initialize" => Ok(id.map(|id| json_rpc_result(id, self.initialize_result(&request)))),
      "notifications/initialized" => Ok(None),
      "ping" => Ok(id.map(|id| json_rpc_result(id, json!({})))),
      "tools/list" => Ok(id.map(|id| json_rpc_result(id, json!({ "tools": tools() })))),
      "tools/call" => {
        let Some(id) = id else { return Ok(None) };
        Ok(Some(self.call_tool(id, request)))
      }
      "shutdown" => Ok(id.map(|id| json_rpc_result(id, Value::Null))),
      other => Ok(id.map(|id| json_rpc_error(id, -32601, format!("unknown method: {other}")))),
    }
  }

  fn initialize_result(&self, request: &Value) -> Value {
    let protocol_version = request
      .get("params")
      .and_then(|params| params.get("protocolVersion"))
      .and_then(Value::as_str)
      .unwrap_or(DEFAULT_PROTOCOL_VERSION);

    json!({
      "protocolVersion": protocol_version,
      "capabilities": {
        "tools": {}
      },
      "serverInfo": {
        "name": "reviu-driver-mcp",
        "version": env!("CARGO_PKG_VERSION")
      }
    })
  }

  fn call_tool(&mut self, id: Value, request: Value) -> Value {
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let Some(name) = params.get("name").and_then(Value::as_str) else {
      return json_rpc_error(id, -32602, "tools/call needs params.name");
    };
    let arguments = params
      .get("arguments")
      .cloned()
      .unwrap_or_else(|| json!({}));

    let result = match name {
      "start" => self.start(arguments),
      "restart" => self.restart(arguments),
      "status" => Ok(self.driver.status()),
      "quit" => self.quit(),
      tool_name => self.driver_command(tool_name, arguments),
    };

    json_rpc_result(id, tool_result(result))
  }

  fn start(&mut self, arguments: Value) -> Result<Value> {
    let config = self.config_from_arguments(arguments)?;
    self.config = config.clone();
    self.driver.reconfigure(config);
    self.driver.ensure_started()?;
    Ok(self.driver.status())
  }

  fn restart(&mut self, arguments: Value) -> Result<Value> {
    let config = self.config_from_arguments(arguments)?;
    self.config = config.clone();
    self.driver.stop();
    self.driver.reconfigure(config);
    self.driver.ensure_started()?;
    Ok(self.driver.status())
  }

  fn quit(&mut self) -> Result<Value> {
    self.driver.quit()
  }

  fn config_from_arguments(&self, arguments: Value) -> Result<ServerConfig> {
    let mut config = self.config.clone();
    if let Some(backend) = optional_string(&arguments, "backend")? {
      validate_backend(&backend)?;
      config.backend = backend;
    }
    if let Some(driver_bin) = optional_string(&arguments, "driver_bin")? {
      config.driver_bin = Some(PathBuf::from(driver_bin));
    }
    if let Some(agent_command) = optional_string(&arguments, "agent_command")? {
      config.agent_command = Some(agent_command);
    }
    Ok(config)
  }

  fn driver_command(&mut self, tool_name: &str, arguments: Value) -> Result<Value> {
    let command = command_for_tool(tool_name, arguments)?;
    self.driver.command(command)
  }
}

struct DriverSession {
  config: ServerConfig,
  process: Option<DriverProcess>,
}

impl DriverSession {
  fn new(config: ServerConfig) -> Self {
    Self {
      config,
      process: None,
    }
  }

  fn reconfigure(&mut self, config: ServerConfig) {
    if self.config != config {
      self.stop();
      self.config = config;
    }
  }

  fn ensure_started(&mut self) -> Result<()> {
    if let Some(process) = &mut self.process
      && process.is_alive()
    {
      return Ok(());
    }
    self.stop();
    self.process = Some(DriverProcess::spawn(&self.config)?);
    Ok(())
  }

  fn command(&mut self, command: Value) -> Result<Value> {
    self.ensure_started()?;
    let process = self.process.as_mut().context("driver process")?;
    match process.command(command) {
      Ok(response) => Ok(response),
      Err(error) => {
        self.stop();
        Err(error)
      }
    }
  }

  fn quit(&mut self) -> Result<Value> {
    let Some(mut process) = self.process.take() else {
      return Ok(json!({ "running": false }));
    };
    let response = process.command(json!({ "cmd": "quit" }))?;
    let _ = process.child.wait();
    Ok(response)
  }

  fn stop(&mut self) {
    if let Some(mut process) = self.process.take() {
      if process.child.try_wait().ok().flatten().is_none() {
        let _ = process.child.kill();
      }
      let _ = process.child.wait();
    }
  }

  fn status(&mut self) -> Value {
    let running = self
      .process
      .as_mut()
      .is_some_and(|process| process.is_alive());
    json!({
      "running": running,
      "backend": self.config.backend,
      "driver_bin": self.config.driver_bin.as_ref().map(|path| path.display().to_string()),
      "agent_command": self.config.agent_command,
    })
  }
}

impl Drop for DriverSession {
  fn drop(&mut self) {
    self.stop();
  }
}

struct DriverProcess {
  child: Child,
  stdin: std::process::ChildStdin,
  stdout: BufReader<std::process::ChildStdout>,
}

impl DriverProcess {
  fn spawn(config: &ServerConfig) -> Result<Self> {
    let mut command = driver_command(config)?;
    command
      .arg("--backend")
      .arg(&config.backend)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::inherit());
    if let Some(agent_command) = &config.agent_command {
      command.arg("--agent-command").arg(agent_command);
    }

    let mut child = command.spawn().context("spawn reviu-driver")?;
    let stdin = child.stdin.take().context("driver stdin")?;
    let stdout = BufReader::new(child.stdout.take().context("driver stdout")?);
    let mut process = Self {
      child,
      stdin,
      stdout,
    };
    let ready = process
      .read_response()
      .context("read driver ready response")?;
    if !ready.get("ok").and_then(Value::as_bool).unwrap_or(false)
      || !ready.get("ready").and_then(Value::as_bool).unwrap_or(false)
    {
      bail!("driver did not become ready: {}", pretty_json(&ready));
    }
    Ok(process)
  }

  fn is_alive(&mut self) -> bool {
    self.child.try_wait().ok().flatten().is_none()
  }

  fn command(&mut self, command: Value) -> Result<Value> {
    writeln!(self.stdin, "{command}")?;
    self.stdin.flush()?;
    let response = self.read_response()?;
    if !response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
      bail!("driver command failed: {}", pretty_json(&response));
    }
    Ok(response)
  }

  fn read_response(&mut self) -> Result<Value> {
    let mut line = String::new();
    let bytes = self.stdout.read_line(&mut line)?;
    if bytes == 0 {
      bail!("driver closed stdout");
    }
    serde_json::from_str(line.trim()).context("parse driver response")
  }
}

fn driver_command(config: &ServerConfig) -> Result<Command> {
  if let Some(driver_bin) = &config.driver_bin {
    return Ok(Command::new(driver_bin));
  }

  if let Some(sibling) = sibling_driver_bin()? {
    return Ok(Command::new(sibling));
  }

  let mut cargo = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()));
  cargo.args([
    "run",
    "-p",
    "reviu_driver",
    "--bin",
    "reviu-driver",
    "--quiet",
    "--",
  ]);
  cargo.current_dir(workspace_root());
  Ok(cargo)
}

fn sibling_driver_bin() -> Result<Option<PathBuf>> {
  let executable = std::env::current_exe().context("current executable")?;
  let driver_name = if cfg!(windows) {
    "reviu-driver.exe"
  } else {
    "reviu-driver"
  };
  let sibling = executable.with_file_name(driver_name);
  Ok(sibling.exists().then_some(sibling))
}

fn workspace_root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR"))
    .ancestors()
    .nth(2)
    .map(Path::to_path_buf)
    .unwrap_or_else(|| PathBuf::from("."))
}

fn command_for_tool(tool_name: &str, arguments: Value) -> Result<Value> {
  Ok(match tool_name {
    "bounds" => json!({
      "cmd": "bounds",
      "selector": required_string(&arguments, "selector")?,
    }),
    "click" => json!({
      "cmd": "click",
      "selector": optional_string(&arguments, "selector")?,
      "x": optional_f64(&arguments, "x")?,
      "y": optional_f64(&arguments, "y")?,
    }),
    "type" => json!({
      "cmd": "type",
      "text": required_string(&arguments, "text")?,
    }),
    "key" => json!({
      "cmd": "key",
      "keystrokes": required_string(&arguments, "keystrokes")?,
    }),
    "clock" => json!({
      "cmd": "clock",
      "ms": required_u64(&arguments, "ms")?,
    }),
    "wait" => json!({
      "cmd": "wait",
      "ms": required_u64(&arguments, "ms")?,
    }),
    "park" => json!({ "cmd": "park" }),
    "path_prompt" => json!({
      "cmd": "path_prompt",
      "path": required_string(&arguments, "path")?,
    }),
    "open_file" => json!({
      "cmd": "open_file",
      "path": required_string(&arguments, "path")?,
    }),
    "open_pull_request_file" => json!({
      "cmd": "open_pull_request_file",
      "path": optional_string(&arguments, "path")?,
    }),
    "scroll" => json!({
      "cmd": "scroll",
      "delta_x": optional_f64(&arguments, "delta_x")?,
      "delta_y": required_f64(&arguments, "delta_y")?,
      "x": optional_f64(&arguments, "x")?,
      "y": optional_f64(&arguments, "y")?,
      "steps": optional_u64(&arguments, "steps")?,
    }),
    "screenshot" => json!({
      "cmd": "screenshot",
      "path": required_string(&arguments, "path")?,
    }),
    "show_changes" => json!({ "cmd": "show_changes" }),
    "show_pull_request" => json!({ "cmd": "show_pull_request" }),
    "show_review" => json!({ "cmd": "show_review" }),
    "create_pull_request_review_comment" => json!({
      "cmd": "create_pull_request_review_comment",
      "path": required_string(&arguments, "path")?,
      "line": required_u64(&arguments, "line")?,
      "body": required_string(&arguments, "body")?,
    }),
    "submit_pull_request_review" => json!({
      "cmd": "submit_pull_request_review",
      "body": required_string(&arguments, "body")?,
    }),
    "discard_pull_request_review" => json!({ "cmd": "discard_pull_request_review" }),
    "hide_dock" => json!({ "cmd": "hide_dock" }),
    "submit_prompt" => json!({
      "cmd": "submit_prompt",
      "text": required_string(&arguments, "text")?,
    }),
    "agent_stats" => json!({ "cmd": "agent_stats" }),
    "editor_stats" => json!({ "cmd": "editor_stats" }),
    "git_state" => json!({ "cmd": "git_state" }),
    "dialog_state" => json!({ "cmd": "dialog_state" }),
    "confirm_dialog" => json!({ "cmd": "confirm_dialog" }),
    "cancel_dialog" => json!({ "cmd": "cancel_dialog" }),
    "notification_stats" => json!({ "cmd": "notification_stats" }),
    "notification_log" => json!({ "cmd": "notification_log" }),
    "auth_state" => json!({ "cmd": "auth_state" }),
    "set_auth_token" => json!({
      "cmd": "set_auth_token",
      "token": required_string(&arguments, "token")?,
    }),
    "run_git_action" => json!({
      "cmd": "run_git_action",
      "action": required_value_field(&arguments, "action")?,
    }),
    other => bail!("unknown tool: {other}"),
  })
}

fn required_value_field(value: &Value, key: &str) -> Result<Value> {
  value
    .get(key)
    .cloned()
    .with_context(|| format!("missing required argument: {key}"))
}

fn required_string(value: &Value, key: &str) -> Result<String> {
  value
    .get(key)
    .and_then(Value::as_str)
    .map(ToString::to_string)
    .with_context(|| format!("missing string argument: {key}"))
}

fn optional_string(value: &Value, key: &str) -> Result<Option<String>> {
  match value.get(key) {
    None | Some(Value::Null) => Ok(None),
    Some(Value::String(value)) => Ok(Some(value.clone())),
    _ => bail!("argument {key} must be a string"),
  }
}

fn required_u64(value: &Value, key: &str) -> Result<u64> {
  value
    .get(key)
    .and_then(Value::as_u64)
    .with_context(|| format!("missing unsigned integer argument: {key}"))
}

fn optional_u64(value: &Value, key: &str) -> Result<Option<u64>> {
  match value.get(key) {
    None | Some(Value::Null) => Ok(None),
    Some(value) => value
      .as_u64()
      .map(Some)
      .with_context(|| format!("argument {key} must be an unsigned integer")),
  }
}

fn required_f64(value: &Value, key: &str) -> Result<f64> {
  value
    .get(key)
    .and_then(Value::as_f64)
    .with_context(|| format!("missing number argument: {key}"))
}

fn optional_f64(value: &Value, key: &str) -> Result<Option<f64>> {
  match value.get(key) {
    None | Some(Value::Null) => Ok(None),
    Some(value) => value
      .as_f64()
      .map(Some)
      .with_context(|| format!("argument {key} must be a number")),
  }
}

fn tools() -> Vec<Value> {
  vec![
    tool(
      "start",
      "Start the reviu-driver process, optionally overriding backend or driver path.",
      object_schema(vec![
        string_property("backend", "Driver backend: test or visual."),
        string_property("driver_bin", "Path to a prebuilt reviu-driver binary."),
        string_property(
          "agent_command",
          "Optional agent command passed to the driver.",
        ),
      ]),
    ),
    tool(
      "restart",
      "Restart the reviu-driver process, optionally overriding backend or driver path.",
      object_schema(vec![
        string_property("backend", "Driver backend: test or visual."),
        string_property("driver_bin", "Path to a prebuilt reviu-driver binary."),
        string_property(
          "agent_command",
          "Optional agent command passed to the driver.",
        ),
      ]),
    ),
    tool(
      "status",
      "Report wrapper and driver process status.",
      empty_schema(),
    ),
    tool(
      "bounds",
      "Return painted bounds for a debug selector. Requires backend test.",
      object_schema(vec![string_property("selector", "Debug selector.")]).required(["selector"]),
    ),
    tool(
      "click",
      "Click a debug selector center in backend test, or point coordinates in either backend.",
      object_schema(vec![
        string_property("selector", "Debug selector to click."),
        number_property("x", "Point x coordinate."),
        number_property("y", "Point y coordinate."),
      ]),
    ),
    tool(
      "type",
      "Type text into the focused input.",
      object_schema(vec![string_property("text", "Text to type.")]).required(["text"]),
    ),
    tool(
      "key",
      "Simulate keystrokes separated by spaces, for example cmd-p or down enter.",
      object_schema(vec![string_property(
        "keystrokes",
        "Keystrokes to simulate.",
      )])
      .required(["keystrokes"]),
    ),
    tool(
      "clock",
      "Advance the GPUI virtual clock by milliseconds.",
      object_schema(vec![integer_property("ms", "Milliseconds to advance.")]).required(["ms"]),
    ),
    tool(
      "wait",
      "Let real time pass while pumping the driver.",
      object_schema(vec![integer_property("ms", "Milliseconds to wait.")]).required(["ms"]),
    ),
    tool(
      "park",
      "Run scheduled GPUI work to quiescence.",
      empty_schema(),
    ),
    tool(
      "path_prompt",
      "Open a repository path in the driver.",
      object_schema(vec![string_property("path", "Repository path.")]).required(["path"]),
    ),
    tool(
      "open_file",
      "Open a repository-relative file path in the center editor.",
      object_schema(vec![string_property("path", "Repository-relative path.")]).required(["path"]),
    ),
    tool(
      "open_pull_request_file",
      "Open a loaded Pull Request file in the center editor. Omitting path opens the first file.",
      object_schema(vec![string_property(
        "path",
        "Repository-relative PR file path.",
      )]),
    ),
    tool(
      "scroll",
      "Simulate a mouse wheel scroll.",
      object_schema(vec![
        number_property("delta_x", "Horizontal scroll delta."),
        number_property("delta_y", "Vertical scroll delta."),
        number_property("x", "Pointer x coordinate."),
        number_property("y", "Pointer y coordinate."),
        integer_property("steps", "Number of scroll events."),
      ])
      .required(["delta_y"]),
    ),
    tool(
      "screenshot",
      "Write a screenshot PNG. Requires backend visual on macOS.",
      object_schema(vec![string_property("path", "Output PNG path.")]).required(["path"]),
    ),
    tool("show_changes", "Open the Changes dock tab.", empty_schema()),
    tool(
      "show_pull_request",
      "Open the Pull Request dock tab.",
      empty_schema(),
    ),
    tool("show_review", "Open the Review dock tab.", empty_schema()),
    tool(
      "create_pull_request_review_comment",
      "Create a pending pull request review comment on the open PR file.",
      object_schema(vec![
        string_property("path", "Repository-relative PR file path."),
        integer_property("line", "Zero-based line number."),
        string_property("body", "Comment body."),
      ])
      .required(["path", "line", "body"]),
    ),
    tool(
      "submit_pull_request_review",
      "Submit the pending pull request review as a comment review.",
      object_schema(vec![string_property("body", "Review body.")]).required(["body"]),
    ),
    tool(
      "discard_pull_request_review",
      "Open the discard pending pull request review confirmation.",
      empty_schema(),
    ),
    tool("hide_dock", "Close the right dock.", empty_schema()),
    tool(
      "submit_prompt",
      "Fill and submit the agent composer.",
      object_schema(vec![string_property("text", "Prompt text.")]).required(["text"]),
    ),
    tool(
      "agent_stats",
      "Return active/background agent turn counts.",
      empty_schema(),
    ),
    tool(
      "editor_stats",
      "Return active editor state.",
      empty_schema(),
    ),
    tool(
      "git_state",
      "Return active repository Git/UI state.",
      empty_schema(),
    ),
    tool(
      "dialog_state",
      "Return whether a dialog is active.",
      empty_schema(),
    ),
    tool(
      "confirm_dialog",
      "Confirm the active dialog.",
      empty_schema(),
    ),
    tool("cancel_dialog", "Cancel the active dialog.", empty_schema()),
    tool(
      "notification_stats",
      "Count active in-app notifications.",
      empty_schema(),
    ),
    tool(
      "notification_log",
      "Return notifications recorded by driver-supported flows.",
      empty_schema(),
    ),
    tool(
      "auth_state",
      "Return Reviu auth state for diagnostics.",
      empty_schema(),
    ),
    tool(
      "set_auth_token",
      "Set an in-memory Reviu API bearer token, then refresh auth state.",
      object_schema(vec![string_property("token", "Reviu API bearer token.")]).required(["token"]),
    ),
    tool(
      "run_git_action",
      "Run a Git action through the same path as the command palette.",
      object_schema(vec![value_property(
        "action",
        "DriverGitAction JSON object.",
      )])
      .required(["action"]),
    ),
    tool(
      "quit",
      "Quit the underlying driver process.",
      empty_schema(),
    ),
  ]
}

fn tool(name: &str, description: &str, input_schema: Schema) -> Value {
  json!({
    "name": name,
    "description": description,
    "inputSchema": input_schema.value,
  })
}

struct Schema {
  value: Value,
}

impl Schema {
  fn required<const N: usize>(mut self, keys: [&str; N]) -> Self {
    if let Some(object) = self.value.as_object_mut() {
      object.insert("required".to_string(), json!(keys.to_vec()));
    }
    self
  }
}

fn empty_schema() -> Schema {
  object_schema(Vec::new())
}

fn object_schema(properties: Vec<(&'static str, Value)>) -> Schema {
  let mut property_map = serde_json::Map::new();
  for (name, schema) in properties {
    property_map.insert(name.to_string(), schema);
  }
  Schema {
    value: json!({
      "type": "object",
      "properties": property_map,
      "additionalProperties": false,
    }),
  }
}

fn string_property(name: &'static str, description: &'static str) -> (&'static str, Value) {
  (
    name,
    json!({ "type": "string", "description": description }),
  )
}

fn number_property(name: &'static str, description: &'static str) -> (&'static str, Value) {
  (
    name,
    json!({ "type": "number", "description": description }),
  )
}

fn integer_property(name: &'static str, description: &'static str) -> (&'static str, Value) {
  (
    name,
    json!({ "type": "integer", "minimum": 0, "description": description }),
  )
}

fn value_property(name: &'static str, description: &'static str) -> (&'static str, Value) {
  (name, json!({ "description": description }))
}

fn tool_result(result: Result<Value>) -> Value {
  match result {
    Ok(value) => json!({
      "content": [{ "type": "text", "text": pretty_json(&value) }],
      "structuredContent": value,
      "isError": false,
    }),
    Err(error) => json!({
      "content": [{ "type": "text", "text": error.to_string() }],
      "isError": true,
    }),
  }
}

fn json_rpc_result(id: Value, result: Value) -> Value {
  json!({
    "jsonrpc": "2.0",
    "id": id,
    "result": result,
  })
}

fn json_rpc_error(id: Value, code: i64, message: impl ToString) -> Value {
  json!({
    "jsonrpc": "2.0",
    "id": id,
    "error": {
      "code": code,
      "message": message.to_string(),
    }
  })
}

fn respond(value: Value) {
  let mut stdout = std::io::stdout().lock();
  let _ = writeln!(stdout, "{value}");
  let _ = stdout.flush();
}

fn pretty_json(value: &Value) -> String {
  serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn args_default_to_test_backend() {
    assert_eq!(parse_args([]).expect("args"), ServerConfig::default());
  }

  #[test]
  fn args_accept_overrides() {
    let args = parse_args([
      "--backend=visual".to_string(),
      "--driver-bin".to_string(),
      "/tmp/reviu-driver".to_string(),
      "--agent-command".to_string(),
      "/tmp/stub-agent".to_string(),
    ])
    .expect("args");

    assert_eq!(args.backend, "visual");
    assert_eq!(args.driver_bin, Some(PathBuf::from("/tmp/reviu-driver")));
    assert_eq!(args.agent_command.as_deref(), Some("/tmp/stub-agent"));
  }

  #[test]
  fn args_reject_unknown_backend() {
    assert!(parse_args(["--backend=other".to_string()]).is_err());
  }

  #[test]
  fn tools_include_core_driver_verbs() {
    let tools = tools();
    let names = tools
      .iter()
      .filter_map(|tool| tool.get("name").and_then(Value::as_str))
      .collect::<Vec<_>>();

    for name in [
      "start",
      "bounds",
      "click",
      "path_prompt",
      "open_pull_request_file",
      "screenshot",
      "git_state",
      "notification_log",
      "show_pull_request",
      "show_review",
      "create_pull_request_review_comment",
      "submit_pull_request_review",
      "discard_pull_request_review",
      "auth_state",
      "set_auth_token",
      "quit",
    ] {
      assert!(names.contains(&name));
    }
  }

  #[test]
  fn maps_tools_to_driver_commands() {
    assert_eq!(
      command_for_tool("key", json!({ "keystrokes": "cmd-p" })).expect("key command"),
      json!({ "cmd": "key", "keystrokes": "cmd-p" })
    );
    assert_eq!(
      command_for_tool("run_git_action", json!({ "action": { "action": "push" } }))
        .expect("git action command"),
      json!({ "cmd": "run_git_action", "action": { "action": "push" } })
    );
    assert_eq!(
      command_for_tool("open_pull_request_file", json!({ "path": "src/lib.rs" }))
        .expect("open pull request file command"),
      json!({ "cmd": "open_pull_request_file", "path": "src/lib.rs" })
    );
    assert_eq!(
      command_for_tool(
        "create_pull_request_review_comment",
        json!({ "path": "src/lib.rs", "line": 0, "body": "note" })
      )
      .expect("create pull request review comment command"),
      json!({
        "cmd": "create_pull_request_review_comment",
        "path": "src/lib.rs",
        "line": 0,
        "body": "note"
      })
    );
    assert_eq!(
      command_for_tool(
        "submit_pull_request_review",
        json!({ "body": "looks good" })
      )
      .expect("submit pull request review command"),
      json!({ "cmd": "submit_pull_request_review", "body": "looks good" })
    );
    assert_eq!(
      command_for_tool("show_pull_request", json!({})).expect("show pull request"),
      json!({ "cmd": "show_pull_request" })
    );
  }

  #[test]
  fn initialize_returns_tools_capability() {
    let server = McpServer::new(ServerConfig::default());
    let result = server.initialize_result(&json!({
      "params": { "protocolVersion": "2025-06-18" }
    }));

    assert_eq!(
      result.get("protocolVersion").and_then(Value::as_str),
      Some("2025-06-18")
    );
    assert!(
      result
        .get("capabilities")
        .and_then(|value| value.get("tools"))
        .is_some()
    );
  }
}
