use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};

#[allow(dead_code)]
#[path = "../driver_harness.rs"]
mod driver_harness;

use crate::driver_harness::{
  DriverProcess, TempRunDir, commit_file, init_repo, pretty_json, scenario_diagnostics, wait_until,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq, Eq)]
struct TerminalSmokeArgs {
  driver_bin: Option<PathBuf>,
  backend: String,
  screenshot: Option<PathBuf>,
  keep_temp: bool,
}

fn main() -> Result<()> {
  let args = parse_args(std::env::args().skip(1))?;
  run(args)
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<TerminalSmokeArgs> {
  let mut parsed = TerminalSmokeArgs {
    driver_bin: None,
    backend: "test".to_string(),
    screenshot: None,
    keep_temp: false,
  };
  let mut args = args.into_iter();
  while let Some(argument) = args.next() {
    match argument.as_str() {
      "--driver-bin" => {
        parsed.driver_bin = Some(PathBuf::from(required_value(&mut args, "--driver-bin")?));
      }
      "--backend" => parsed.backend = required_value(&mut args, "--backend")?,
      "--screenshot" => {
        parsed.screenshot = Some(PathBuf::from(required_value(&mut args, "--screenshot")?));
      }
      "--keep-temp" => parsed.keep_temp = true,
      "--help" | "-h" => bail!(usage()),
      other if other.starts_with("--driver-bin=") => {
        parsed.driver_bin = Some(PathBuf::from(other.trim_start_matches("--driver-bin=")));
      }
      other if other.starts_with("--backend=") => {
        parsed.backend = other.trim_start_matches("--backend=").to_string();
      }
      other if other.starts_with("--screenshot=") => {
        parsed.screenshot = Some(PathBuf::from(other.trim_start_matches("--screenshot=")));
      }
      other => bail!("unknown argument: {other}\n{}", usage()),
    }
  }

  ensure!(
    matches!(parsed.backend.as_str(), "test" | "visual"),
    "backend must be test or visual"
  );
  if parsed.backend != "visual" && parsed.screenshot.is_some() {
    bail!("--screenshot requires --backend visual");
  }
  if parsed.backend == "visual" && !cfg!(target_os = "macos") {
    bail!("the visual backend requires macOS");
  }
  Ok(parsed)
}

fn required_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
  args.next().with_context(|| format!("{flag} needs a value"))
}

fn usage() -> &'static str {
  "usage: reviu-terminal-smoke [--driver-bin PATH] [--backend test|visual] [--screenshot PATH] [--keep-temp]"
}

fn run(args: TerminalSmokeArgs) -> Result<()> {
  let mut run_dir = TempRunDir::new("reviu-terminal-smoke", args.keep_temp)?;
  println!("terminal smoke temp: {}", run_dir.path.display());
  if let Err(error) = run_terminal_scenario(&args, &run_dir.path) {
    run_dir.keep = true;
    eprintln!(
      "{}",
      scenario_diagnostics(&run_dir.path).unwrap_or_else(|diagnostic_error| format!(
        "failed to collect diagnostics: {diagnostic_error:#}"
      ))
    );
    eprintln!("kept terminal smoke temp: {}", run_dir.path.display());
    return Err(error);
  }
  Ok(())
}

fn run_terminal_scenario(args: &TerminalSmokeArgs, run_dir: &Path) -> Result<()> {
  let repo = init_repo(&run_dir.join("repo"))?;
  std::fs::create_dir_all(repo.join("src"))?;
  commit_file(&repo, "src/main.rs", "fn main() {}\n", "initial")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, run_dir, true)?;
  driver.command(json!({ "cmd": "path_prompt", "path": repo }))?;
  driver.command(json!({ "cmd": "open_terminal" }))?;

  let expected_root = repo.canonicalize()?;
  wait_for_terminal_state(&mut driver, |state| {
    state
      .get("working_directory")
      .and_then(Value::as_str)
      .is_some_and(|path| Path::new(path) == expected_root)
  })
  .context("waiting for the terminal working directory")?;

  let output_command = if cfg!(windows) {
    "Write-Output 'café 日 🚀'; 1..100; Write-Output 'src/main.rs:1:1'"
  } else {
    "printf 'café 日 🚀\\n'; seq 1 100; echo src/main.rs:1:1"
  };
  driver.command(json!({ "cmd": "type", "text": output_command }))?;
  driver.command(json!({ "cmd": "key", "keystrokes": "enter" }))?;
  let output_state = wait_for_terminal_state(&mut driver, |state| {
    state
      .get("visible_text")
      .and_then(Value::as_str)
      .is_some_and(|text| text.contains("src/main.rs:1:1"))
      && state
        .get("total_lines")
        .and_then(Value::as_u64)
        .is_some_and(|lines| lines > 100)
  })
  .context("waiting for terminal output")?;
  ensure!(
    output_state
      .get("visible_text")
      .and_then(Value::as_str)
      .is_some_and(|text| text.contains("src/main.rs:1:1")),
    "terminal output did not preserve the file location"
  );

  driver.command(json!({
    "cmd": "key",
    "keystrokes": if cfg!(target_os = "macos") { "cmd-f" } else { "ctrl-f" }
  }))?;
  driver.command(json!({ "cmd": "type", "text": "café" }))?;
  let search_state = wait_for_terminal_state(&mut driver, |state| {
    state.get("search_open").and_then(Value::as_bool) == Some(true)
      && state
        .get("search_match_count")
        .and_then(Value::as_u64)
        .is_some_and(|count| count >= 2)
  })
  .context("waiting for terminal search matches")?;
  let active_match = search_state
    .get("active_search_match")
    .and_then(Value::as_u64)
    .context("search should select a match")?;

  driver.command(json!({
    "cmd": "key",
    "keystrokes": if cfg!(target_os = "macos") { "cmd-g" } else { "ctrl-g" }
  }))?;
  wait_for_terminal_state(&mut driver, |state| {
    state
      .get("active_search_match")
      .and_then(Value::as_u64)
      .is_some_and(|next| next != active_match)
  })
  .context("waiting for terminal search navigation")?;

  if args.backend == "visual" {
    let screenshot = args
      .screenshot
      .clone()
      .unwrap_or_else(|| run_dir.join("terminal-search-and-scrollback.png"));
    driver.command(json!({
      "cmd": "screenshot",
      "path": screenshot.display().to_string()
    }))?;
    verify_png(&screenshot)?;
    println!("screenshot: {}", screenshot.display());
  }

  driver.command(json!({ "cmd": "key", "keystrokes": "escape" }))?;
  wait_for_terminal_state(&mut driver, |state| {
    state.get("search_open").and_then(Value::as_bool) == Some(false)
  })
  .context("waiting for terminal search to close")?;

  driver.command(json!({ "cmd": "key", "keystrokes": "shift-home" }))?;
  wait_for_terminal_state(&mut driver, |state| {
    state
      .get("display_offset")
      .and_then(Value::as_u64)
      .is_some_and(|offset| offset > 0)
  })
  .context("waiting to reach the top of terminal scrollback")?;
  driver.command(json!({ "cmd": "key", "keystrokes": "shift-end" }))?;
  wait_for_terminal_state(&mut driver, |state| {
    state.get("display_offset").and_then(Value::as_u64) == Some(0)
  })
  .context("waiting to return to the latest terminal output")?;

  driver.command(json!({ "cmd": "open_terminal_file_link" }))?;
  driver.command(json!({ "cmd": "wait", "ms": 100 }))?;
  let editor_state = driver.command(json!({ "cmd": "editor_stats" }))?;
  ensure!(
    editor_state
      .get("selected_file")
      .and_then(Value::as_str)
      .is_some_and(|path| path == "src/main.rs"),
    "terminal file link did not open the expected file:\n{}",
    pretty_json(&editor_state)
  );

  driver.command(json!({ "cmd": "open_terminal" }))?;
  wait_for_terminal_state(&mut driver, |state| {
    state
      .get("working_directory")
      .and_then(Value::as_str)
      .is_some_and(|path| Path::new(path) == expected_root)
  })
  .context("waiting for a terminal after opening the file link")?;
  driver.command(json!({
    "cmd": "type",
    "text": if cfg!(windows) { "Set-Location src" } else { "cd src" }
  }))?;
  driver.command(json!({ "cmd": "key", "keystrokes": "enter" }))?;
  let expected_cwd = repo.join("src").canonicalize()?;
  wait_for_terminal_state(&mut driver, |state| {
    state
      .get("working_directory")
      .and_then(Value::as_str)
      .is_some_and(|path| Path::new(path) == expected_cwd)
  })
  .context("waiting for the shell cd command")?;

  driver.command(json!({ "cmd": "quit" }))?;
  Ok(())
}

fn wait_for_terminal_state(
  driver: &mut DriverProcess,
  predicate: impl Fn(&Value) -> bool,
) -> Result<Value> {
  let mut last = Value::Null;
  if let Err(error) = wait_until(DEFAULT_TIMEOUT, || {
    driver.command(json!({ "cmd": "clock", "ms": 25 })).ok();
    driver.command(json!({ "cmd": "wait", "ms": 10 })).ok();
    match driver.command(json!({ "cmd": "terminal_state" })) {
      Ok(state) => {
        last = state;
        predicate(&last)
      }
      Err(_) => false,
    }
  }) {
    bail!("{error}:\n{}", pretty_json(&last));
  }
  if predicate(&last) {
    Ok(last)
  } else {
    bail!("terminal state never matched:\n{}", pretty_json(&last))
  }
}

fn verify_png(path: &Path) -> Result<()> {
  let bytes = std::fs::read(path).with_context(|| format!("read screenshot {}", path.display()))?;
  ensure!(
    bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
    "screenshot is not a PNG: {}",
    path.display()
  );
  ensure!(bytes.len() > 1_024, "screenshot is unexpectedly small");
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn defaults_to_the_isolated_test_backend() {
    assert_eq!(
      parse_args(Vec::<String>::new()).expect("default args"),
      TerminalSmokeArgs {
        driver_bin: None,
        backend: "test".to_string(),
        screenshot: None,
        keep_temp: false,
      }
    );
  }

  #[test]
  fn screenshot_requires_the_visual_backend() {
    let error = parse_args(["--screenshot=/tmp/terminal.png".to_string()])
      .expect_err("test backend screenshot should fail");
    assert!(error.to_string().contains("requires --backend visual"));
  }
}
