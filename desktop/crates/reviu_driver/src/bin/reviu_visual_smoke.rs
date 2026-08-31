use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq, Eq)]
struct VisualSmokeArgs {
  driver_bin: Option<PathBuf>,
  screenshot: Option<PathBuf>,
  keep_temp: bool,
}

fn main() -> Result<()> {
  let args = parse_args(std::env::args().skip(1))?;
  run(args)
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<VisualSmokeArgs> {
  let mut parsed = VisualSmokeArgs {
    driver_bin: None,
    screenshot: None,
    keep_temp: false,
  };
  let mut args = args.into_iter();
  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--driver-bin" => {
        parsed.driver_bin = Some(PathBuf::from(required_value(&mut args, "--driver-bin")?))
      }
      "--screenshot" => {
        parsed.screenshot = Some(PathBuf::from(required_value(&mut args, "--screenshot")?))
      }
      "--keep-temp" => parsed.keep_temp = true,
      "--help" | "-h" => bail!(usage()),
      other if other.starts_with("--driver-bin=") => {
        parsed.driver_bin = Some(PathBuf::from(other.trim_start_matches("--driver-bin=")));
      }
      other if other.starts_with("--screenshot=") => {
        parsed.screenshot = Some(PathBuf::from(other.trim_start_matches("--screenshot=")));
      }
      other => bail!("unknown argument: {other}\n{}", usage()),
    }
  }
  Ok(parsed)
}

fn required_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
  args.next().with_context(|| format!("{flag} needs a value"))
}

fn usage() -> &'static str {
  "usage: reviu-visual-smoke [--driver-bin PATH] [--screenshot PATH] [--keep-temp]"
}

fn run(args: VisualSmokeArgs) -> Result<()> {
  if !cfg!(target_os = "macos") {
    bail!("visual smoke requires macOS because the driver visual backend is macOS-only");
  }

  let mut run_dir = TempRunDir::new(args.keep_temp)?;
  println!("visual smoke temp: {}", run_dir.path.display());
  let result = run_force_push_dialog_visual(&args, &run_dir.path);
  if let Err(error) = result {
    run_dir.keep = true;
    eprintln!(
      "{}",
      diagnostics(&run_dir.path)
        .unwrap_or_else(|error| format!("failed to collect diagnostics: {error:#}"))
    );
    eprintln!("kept visual smoke temp: {}", run_dir.path.display());
    return Err(error);
  }
  Ok(())
}

fn run_force_push_dialog_visual(args: &VisualSmokeArgs, run_dir: &Path) -> Result<()> {
  let repo = init_repo(&run_dir.join("repo"))?;
  let remote = run_dir.join("remote.git");
  git_no_dir(["init", "--bare", remote.to_str().context("remote path")?])?;
  commit_file(&repo, "a.txt", "v1\n", "initial")?;
  git(
    &repo,
    [
      "remote",
      "add",
      "origin",
      remote.to_str().context("remote path")?,
    ],
  )?;
  git(&repo, ["push", "-u", "origin", "main"])?;
  commit_file(&repo, "a.txt", "v2\n", "second")?;
  git(&repo, ["push"])?;
  git(&repo, ["reset", "--hard", "HEAD~1"])?;
  commit_file(&repo, "a.txt", "rewritten\n", "rewritten")?;

  let local_after_rewrite = git_output(&repo, ["rev-parse", "HEAD"])?;
  let screenshot = args
    .screenshot
    .clone()
    .unwrap_or_else(|| run_dir.join("force-push-dialog.png"));

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), run_dir)?;
  driver.command(json!({ "cmd": "path_prompt", "path": repo }))?;
  wait_for_driver_state(&mut driver, |state| {
    palette_has_command(state, "force_push")
  })?;

  driver.command(json!({
    "cmd": "run_git_action",
    "action": { "action": "force_push" }
  }))?;
  wait_until(DEFAULT_TIMEOUT, || {
    driver
      .command(json!({ "cmd": "dialog_state" }))
      .ok()
      .and_then(|state| bool_field(&state, "active"))
      == Some(true)
  })?;

  driver.command(json!({
    "cmd": "screenshot",
    "path": screenshot.display().to_string()
  }))?;
  verify_png(&screenshot)?;
  println!("screenshot: {}", screenshot.display());

  driver.command(json!({ "cmd": "confirm_dialog" }))?;
  wait_until(DEFAULT_TIMEOUT, || {
    git_bare_output(&remote, ["rev-parse", "refs/heads/main"])
      .map(|head| head == local_after_rewrite)
      .unwrap_or(false)
  })?;
  driver.command(json!({ "cmd": "quit" }))?;
  Ok(())
}

struct TempRunDir {
  path: PathBuf,
  keep: bool,
}

impl TempRunDir {
  fn new(keep: bool) -> Result<Self> {
    let millis = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_millis();
    let path = std::env::temp_dir().join(format!(
      "reviu-visual-smoke-{millis}-{}",
      std::process::id()
    ));
    fs::create_dir_all(&path).with_context(|| format!("create {}", path.display()))?;
    Ok(Self { path, keep })
  }
}

impl Drop for TempRunDir {
  fn drop(&mut self) {
    if !self.keep {
      let _ = fs::remove_dir_all(&self.path);
    }
  }
}

struct DriverProcess {
  child: Child,
  stdin: std::process::ChildStdin,
  stdout: BufReader<std::process::ChildStdout>,
}

impl DriverProcess {
  fn spawn(driver_bin: Option<&Path>, run_dir: &Path) -> Result<Self> {
    let mut command = match driver_bin {
      Some(driver_bin) => Command::new(driver_bin),
      None => {
        let mut cargo =
          Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()));
        cargo
          .args([
            "run",
            "-p",
            "reviu_driver",
            "--bin",
            "reviu-driver",
            "--quiet",
            "--",
          ])
          .current_dir(workspace_root());
        cargo
      }
    };

    let home = run_dir.join("home");
    let config = run_dir.join("config");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(config.join("reviu.dev"))?;
    fs::write(
      config.join("reviu.dev/settings.json"),
      json!({ "agent_notifications": false }).to_string(),
    )?;

    command
      .arg("--backend")
      .arg("visual")
      .env("HOME", &home)
      .env("XDG_CONFIG_HOME", &config)
      .env("REVIU_PROFILE", "dev")
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::from(fs::File::create(
        run_dir.join("driver.stderr.log"),
      )?));

    let mut child = command.spawn().context("spawn visual reviu-driver")?;
    let stdin = child.stdin.take().context("driver stdin")?;
    let stdout = BufReader::new(child.stdout.take().context("driver stdout")?);
    let mut process = Self {
      child,
      stdin,
      stdout,
    };
    let ready = process.read_response()?;
    if !ready.get("ok").and_then(Value::as_bool).unwrap_or(false)
      || !ready.get("ready").and_then(Value::as_bool).unwrap_or(false)
    {
      bail!("driver did not become ready: {}", pretty_json(&ready));
    }
    Ok(process)
  }

  fn command(&mut self, value: Value) -> Result<Value> {
    writeln!(self.stdin, "{value}")?;
    self.stdin.flush()?;
    let response = self.read_response()?;
    if !response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
      bail!("driver command failed:\n{}", pretty_json(&response));
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

impl Drop for DriverProcess {
  fn drop(&mut self) {
    if self.child.try_wait().ok().flatten().is_none() {
      let _ = self.child.kill();
      let _ = self.child.wait();
    }
  }
}

fn wait_for_driver_state(
  driver: &mut DriverProcess,
  predicate: impl Fn(&Value) -> bool,
) -> Result<Value> {
  let mut last = Value::Null;
  wait_until(DEFAULT_TIMEOUT, || {
    match driver.command(json!({ "cmd": "git_state" })) {
      Ok(state) => {
        last = state;
        predicate(&last)
      }
      Err(_) => false,
    }
  })
  .with_context(|| format!("last git_state:\n{}", pretty_json(&last)))?;
  Ok(last)
}

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> Result<()> {
  let deadline = Instant::now() + timeout;
  while Instant::now() < deadline {
    if predicate() {
      return Ok(());
    }
    std::thread::sleep(Duration::from_millis(50));
  }
  bail!("timed out after {timeout:?}")
}

fn palette_has_command(state: &Value, command_id: &str) -> bool {
  state
    .get("palette_commands")
    .and_then(Value::as_array)
    .is_some_and(|commands| commands.iter().any(|command| command == command_id))
}

fn bool_field(value: &Value, key: &str) -> Option<bool> {
  value.get(key)?.as_bool()
}

fn init_repo(path: &Path) -> Result<PathBuf> {
  fs::create_dir_all(path)?;
  let init = Command::new("git")
    .args(["init", "--initial-branch=main"])
    .arg(path)
    .output()
    .context("run git init")?;
  if !init.status.success() {
    git_no_dir(["init", path.to_str().context("repo path")?])?;
    git(path, ["checkout", "-b", "main"])?;
  }
  git(path, ["config", "user.name", "Reviu Smoke"])?;
  git(path, ["config", "user.email", "smoke@reviu.test"])?;
  Ok(path.to_path_buf())
}

fn commit_file(repo: &Path, rel_path: &str, contents: &str, message: &str) -> Result<()> {
  let path = repo.join(rel_path);
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)?;
  }
  fs::write(&path, contents)?;
  git(repo, ["add", rel_path])?;
  git(repo, ["commit", "-m", message])?;
  Ok(())
}

fn verify_png(path: &Path) -> Result<()> {
  let metadata =
    fs::metadata(path).with_context(|| format!("read screenshot {}", path.display()))?;
  if metadata.len() == 0 {
    bail!("screenshot is empty: {}", path.display());
  }

  let image = image::ImageReader::open(path)
    .with_context(|| format!("open screenshot {}", path.display()))?
    .with_guessed_format()
    .context("guess screenshot format")?
    .decode()
    .context("decode screenshot")?;
  if image.width() == 0 || image.height() == 0 {
    bail!(
      "screenshot has invalid dimensions: {}x{}",
      image.width(),
      image.height()
    );
  }
  Ok(())
}

fn git<const N: usize>(repo: &Path, args: [&str; N]) -> Result<()> {
  let output = Command::new("git")
    .args(["-C", repo.to_str().context("repo path")?])
    .args(args)
    .output()
    .context("run git")?;
  if !output.status.success() {
    bail!("git failed: {}", command_output_details(&output));
  }
  Ok(())
}

fn git_no_dir<const N: usize>(args: [&str; N]) -> Result<()> {
  let output = Command::new("git").args(args).output().context("run git")?;
  if !output.status.success() {
    bail!("git failed: {}", command_output_details(&output));
  }
  Ok(())
}

fn git_output<const N: usize>(repo: &Path, args: [&str; N]) -> Result<String> {
  let output = Command::new("git")
    .args(["-C", repo.to_str().context("repo path")?])
    .args(args)
    .output()
    .context("run git")?;
  if !output.status.success() {
    bail!("git failed: {}", command_output_details(&output));
  }
  Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_bare_output<const N: usize>(repo: &Path, args: [&str; N]) -> Result<String> {
  let output = Command::new("git")
    .args(["--git-dir", repo.to_str().context("repo path")?])
    .args(args)
    .output()
    .context("run git")?;
  if !output.status.success() {
    bail!("git failed: {}", command_output_details(&output));
  }
  Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn command_output_details(output: &std::process::Output) -> String {
  let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
  let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
  [stderr, stdout]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

fn diagnostics(run_dir: &Path) -> Result<String> {
  let mut diagnostics = String::new();
  diagnostics.push_str("diagnostics:\n");
  let repo = run_dir.join("repo");
  if repo.exists() {
    append_diagnostic_section(
      &mut diagnostics,
      "git status --short --branch",
      diagnostic_git_output(&repo, ["status", "--short", "--branch"]),
    );
    append_diagnostic_section(
      &mut diagnostics,
      "git log --oneline --decorate --graph --all -8",
      diagnostic_git_output(
        &repo,
        ["log", "--oneline", "--decorate", "--graph", "--all", "-8"],
      ),
    );
  }
  let stderr_log = run_dir.join("driver.stderr.log");
  if stderr_log.exists() {
    append_diagnostic_section(
      &mut diagnostics,
      "driver stderr tail",
      Ok(tail_file(&stderr_log, 80)?),
    );
  }
  Ok(diagnostics)
}

fn append_diagnostic_section(
  diagnostics: &mut String,
  title: &str,
  output: Result<String, anyhow::Error>,
) {
  diagnostics.push_str("\n--- ");
  diagnostics.push_str(title);
  diagnostics.push_str(" ---\n");
  match output {
    Ok(output) if output.trim().is_empty() => diagnostics.push_str("(empty)\n"),
    Ok(output) => {
      diagnostics.push_str(output.trim());
      diagnostics.push('\n');
    }
    Err(error) => diagnostics.push_str(&format!("failed: {error:#}\n")),
  }
}

fn diagnostic_git_output<const N: usize>(repo: &Path, args: [&str; N]) -> Result<String> {
  let output = Command::new("git")
    .args(["-C", repo.to_str().context("repo path")?])
    .args(args)
    .output()
    .context("run diagnostic git")?;
  Ok(command_output_details(&output))
}

fn tail_file(path: &Path, line_count: usize) -> Result<String> {
  let contents = fs::read_to_string(path)?;
  let lines = contents.lines().collect::<Vec<_>>();
  let start = lines.len().saturating_sub(line_count);
  Ok(lines[start..].join("\n"))
}

fn workspace_root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR"))
    .ancestors()
    .nth(2)
    .map(Path::to_path_buf)
    .unwrap_or_else(|| PathBuf::from("."))
}

fn pretty_json(value: &Value) -> String {
  serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_args_defaults() {
    let args = parse_args([]).expect("args");
    assert_eq!(
      args,
      VisualSmokeArgs {
        driver_bin: None,
        screenshot: None,
        keep_temp: false,
      }
    );
  }

  #[test]
  fn parse_args_accepts_overrides() {
    let args = parse_args([
      "--driver-bin=/tmp/reviu-driver".to_string(),
      "--screenshot".to_string(),
      "/tmp/reviu.png".to_string(),
      "--keep-temp".to_string(),
    ])
    .expect("args");

    assert_eq!(args.driver_bin, Some(PathBuf::from("/tmp/reviu-driver")));
    assert_eq!(args.screenshot, Some(PathBuf::from("/tmp/reviu.png")));
    assert!(args.keep_temp);
  }

  #[test]
  fn palette_command_helper_reads_driver_state() {
    let state = json!({
      "palette_commands": ["push", "force_push"]
    });

    assert!(palette_has_command(&state, "force_push"));
    assert!(!palette_has_command(&state, "pull"));
  }
}
