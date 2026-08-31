use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

// The visual smoke runner shares the broader Git smoke harness but uses only a subset.
#[allow(dead_code)]
#[path = "../driver_harness.rs"]
mod driver_harness;

use crate::driver_harness::{
  DriverProcess, TempRunDir, commit_file, git, git_bare_output, git_no_dir, git_output, init_repo,
  pretty_json, scenario_diagnostics, wait_until,
};

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

  let mut run_dir = TempRunDir::new("reviu-visual-smoke", args.keep_temp)?;
  println!("visual smoke temp: {}", run_dir.path.display());
  let result = run_force_push_dialog_visual(&args, &run_dir.path);
  if let Err(error) = result {
    run_dir.keep = true;
    eprintln!(
      "{}",
      scenario_diagnostics(&run_dir.path)
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), "visual", run_dir)?;
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

fn palette_has_command(state: &Value, command_id: &str) -> bool {
  state
    .get("palette_commands")
    .and_then(Value::as_array)
    .is_some_and(|commands| commands.iter().any(|command| command == command_id))
}

fn bool_field(value: &Value, key: &str) -> Option<bool> {
  value.get(key)?.as_bool()
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
