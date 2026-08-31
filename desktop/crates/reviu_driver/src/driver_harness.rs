use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

pub(crate) struct TempRunDir {
  pub(crate) path: PathBuf,
  pub(crate) keep: bool,
}

impl TempRunDir {
  pub(crate) fn new(prefix: &str, keep: bool) -> Result<Self> {
    let millis = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_millis();
    let path = std::env::temp_dir().join(format!("{prefix}-{millis}-{}", std::process::id()));
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

pub(crate) struct DriverProcess {
  child: Child,
  stdin: std::process::ChildStdin,
  stdout: BufReader<std::process::ChildStdout>,
}

impl DriverProcess {
  pub(crate) fn spawn(driver_bin: Option<&Path>, backend: &str, run_dir: &Path) -> Result<Self> {
    let home = run_dir.join("home");
    let config = run_dir.join("config");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(config.join("reviu.dev"))?;
    fs::write(
      config.join("reviu.dev/settings.json"),
      json!({ "agent_notifications": false }).to_string(),
    )?;

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
    command
      .arg("--backend")
      .arg(backend)
      .env("HOME", &home)
      .env("XDG_CONFIG_HOME", &config)
      .env("REVIU_PROFILE", "dev")
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::from(fs::File::create(
        run_dir.join("driver.stderr.log"),
      )?));

    let mut child = command.spawn().context("spawn reviu-driver")?;
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

  pub(crate) fn open_repo(&mut self, repo: &Path) -> Result<()> {
    self.command(json!({ "cmd": "path_prompt", "path": repo }))?;
    Ok(())
  }

  pub(crate) fn run_git_action(&mut self, action: Value) -> Result<()> {
    self.command(json!({ "cmd": "run_git_action", "action": action }))?;
    Ok(())
  }

  pub(crate) fn git_state(&mut self) -> Result<Value> {
    self.command(json!({ "cmd": "git_state" }))
  }

  pub(crate) fn notification_log(&mut self) -> Result<Value> {
    self.command(json!({ "cmd": "notification_log" }))
  }

  pub(crate) fn quit(&mut self) -> Result<()> {
    self.command(json!({ "cmd": "quit" }))?;
    Ok(())
  }

  pub(crate) fn command(&mut self, value: Value) -> Result<Value> {
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

pub(crate) fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> Result<()> {
  let deadline = Instant::now() + timeout;
  while Instant::now() < deadline {
    if predicate() {
      return Ok(());
    }
    std::thread::sleep(Duration::from_millis(50));
  }
  bail!("timed out after {timeout:?}")
}

pub(crate) fn init_repo(path: &Path) -> Result<PathBuf> {
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

pub(crate) fn clone_main(remote: &Path, destination: &Path) -> Result<()> {
  git_no_dir([
    "clone",
    "--branch",
    "main",
    remote.to_str().context("remote path")?,
    destination.to_str().context("clone path")?,
  ])?;
  git(destination, ["config", "user.name", "Reviu Smoke"])?;
  git(destination, ["config", "user.email", "smoke@reviu.test"])?;
  Ok(())
}

pub(crate) fn commit_file(
  repo: &Path,
  rel_path: &str,
  contents: &str,
  message: &str,
) -> Result<()> {
  let path = repo.join(rel_path);
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)?;
  }
  fs::write(&path, contents)?;
  git(repo, ["add", rel_path])?;
  git(repo, ["commit", "-m", message])?;
  Ok(())
}

pub(crate) fn expect_file(repo: &Path, rel_path: &str, expected: &str) -> Result<()> {
  let actual = fs::read_to_string(repo.join(rel_path))?;
  if actual != expected {
    bail!("expected {rel_path} to be {expected:?}, got {actual:?}");
  }
  Ok(())
}

pub(crate) fn assert_eq_str(actual: &str, expected: &str) -> Result<()> {
  if actual.trim() != expected.trim() {
    bail!("expected {expected:?}, got {actual:?}");
  }
  Ok(())
}

pub(crate) fn git<const N: usize>(repo: &Path, args: [&str; N]) -> Result<()> {
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

pub(crate) fn git_no_dir<const N: usize>(args: [&str; N]) -> Result<()> {
  let output = Command::new("git").args(args).output().context("run git")?;
  if !output.status.success() {
    bail!("git failed: {}", command_output_details(&output));
  }
  Ok(())
}

pub(crate) fn git_output<const N: usize>(repo: &Path, args: [&str; N]) -> Result<String> {
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

pub(crate) fn git_lines<const N: usize>(repo: &Path, args: [&str; N]) -> Result<Vec<String>> {
  Ok(
    git_output(repo, args)?
      .lines()
      .map(ToString::to_string)
      .collect(),
  )
}

pub(crate) fn git_bare_output<const N: usize>(repo: &Path, args: [&str; N]) -> Result<String> {
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

pub(crate) fn scenario_diagnostics(scenario_dir: &Path) -> Result<String> {
  let mut diagnostics = String::new();
  diagnostics.push_str("diagnostics:\n");

  let repo = scenario_dir.join("repo");
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
    append_diagnostic_section(
      &mut diagnostics,
      "git stash list",
      diagnostic_git_output(&repo, ["stash", "list"]),
    );
  } else {
    diagnostics.push_str(&format!("repo missing: {}\n", repo.display()));
  }

  let remote = scenario_dir.join("remote.git");
  if remote.exists() {
    append_diagnostic_section(
      &mut diagnostics,
      "remote heads",
      diagnostic_git_bare_output(&remote, ["show-ref", "--heads"]),
    );
  }

  let stderr_log = scenario_dir.join("driver.stderr.log");
  if stderr_log.exists() {
    append_diagnostic_section(
      &mut diagnostics,
      "driver stderr tail",
      Ok(tail_file(&stderr_log, 80)?),
    );
  }

  Ok(diagnostics)
}

pub(crate) fn pretty_json(value: &Value) -> String {
  serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
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

fn diagnostic_git_bare_output<const N: usize>(repo: &Path, args: [&str; N]) -> Result<String> {
  let output = Command::new("git")
    .args(["--git-dir", repo.to_str().context("repo path")?])
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

fn command_output_details(output: &std::process::Output) -> String {
  let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
  let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
  [stderr, stdout]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

fn workspace_root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR"))
    .ancestors()
    .nth(2)
    .map(Path::to_path_buf)
    .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pretty_json_formats_json() {
    assert_eq!(pretty_json(&json!({ "ok": true })), "{\n  \"ok\": true\n}");
  }
}
