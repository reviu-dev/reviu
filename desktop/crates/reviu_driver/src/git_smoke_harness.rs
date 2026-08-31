use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};

const DEFAULT_BACKEND: &str = "test";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const SCENARIOS: &[&str] = &[
  "stash_pop",
  "stash_untracked_pop",
  "apply_stash_conflict",
  "pop_stash_conflict",
  "merge_conflict_abort",
  "merge_conflict_commit",
  "rebase_conflict_continue",
  "rebase_conflict_skip",
  "rebase_conflict_abort",
  "force_push_dialog",
  "stale_force_push_lease",
  "branch_switch_dirty_conflict",
  "pull_dirty_conflict",
  "detached_checkout",
  "cherry_pick",
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GitSmokeArgs {
  pub(crate) backend: String,
  pub(crate) driver_bin: Option<PathBuf>,
  pub(crate) scenarios: Vec<String>,
  pub(crate) keep_temp: bool,
}

impl Default for GitSmokeArgs {
  fn default() -> Self {
    Self {
      backend: DEFAULT_BACKEND.to_string(),
      driver_bin: None,
      scenarios: SCENARIOS
        .iter()
        .map(|scenario| scenario.to_string())
        .collect(),
      keep_temp: false,
    }
  }
}

pub(crate) fn parse_args(args: impl IntoIterator<Item = String>) -> Result<GitSmokeArgs> {
  let mut parsed = GitSmokeArgs::default();
  let mut args = args.into_iter();
  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--backend" => parsed.backend = required_value(&mut args, "--backend")?,
      "--driver-bin" => {
        parsed.driver_bin = Some(PathBuf::from(required_value(&mut args, "--driver-bin")?))
      }
      "--scenario" => push_scenarios(
        &mut parsed.scenarios,
        &required_value(&mut args, "--scenario")?,
      )?,
      "--keep-temp" => parsed.keep_temp = true,
      "--help" | "-h" => bail!(usage()),
      other if other.starts_with("--backend=") => {
        parsed.backend = other.trim_start_matches("--backend=").to_string();
      }
      other if other.starts_with("--driver-bin=") => {
        parsed.driver_bin = Some(PathBuf::from(other.trim_start_matches("--driver-bin=")));
      }
      other if other.starts_with("--scenario=") => {
        push_scenarios(
          &mut parsed.scenarios,
          other.trim_start_matches("--scenario="),
        )?;
      }
      other => bail!("unknown argument: {other}\n{}", usage()),
    }
  }
  Ok(parsed)
}

fn required_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
  args.next().with_context(|| format!("{flag} needs a value"))
}

fn push_scenarios(target: &mut Vec<String>, value: &str) -> Result<()> {
  let requested = value
    .split(',')
    .map(str::trim)
    .filter(|scenario| !scenario.is_empty())
    .map(ToString::to_string)
    .collect::<Vec<_>>();
  if requested.is_empty() {
    bail!("--scenario needs at least one scenario");
  }
  for scenario in &requested {
    if !SCENARIOS.contains(&scenario.as_str()) {
      bail!("unknown scenario: {scenario}\n{}", usage());
    }
  }
  *target = requested;
  Ok(())
}

fn usage() -> String {
  format!(
    "usage: reviu-git-smoke [--backend test|visual] [--driver-bin PATH] [--scenario {}] [--keep-temp]",
    SCENARIOS.join(",")
  )
}

pub(crate) fn run(args: GitSmokeArgs) -> Result<()> {
  let mut run_dir = TempRunDir::new(args.keep_temp)?;
  println!("git smoke temp: {}", run_dir.path.display());

  let mut results = Vec::new();
  for scenario in &args.scenarios {
    let started = Instant::now();
    print!("scenario {scenario} ... ");
    std::io::stdout().flush()?;
    let result = run_one(&args, &run_dir.path, scenario);
    match result {
      Ok(()) => {
        println!("ok ({:?})", started.elapsed());
        results.push((scenario.clone(), true));
      }
      Err(error) => {
        println!("FAILED ({:?})", started.elapsed());
        eprintln!("{error:?}");
        results.push((scenario.clone(), false));
      }
    }
  }

  let failed = results
    .iter()
    .filter(|(_, passed)| !*passed)
    .map(|(scenario, _)| scenario.as_str())
    .collect::<Vec<_>>();
  if !failed.is_empty() {
    run_dir.keep = true;
    eprintln!("kept git smoke temp: {}", run_dir.path.display());
    bail!("failed scenarios: {}", failed.join(", "));
  }

  Ok(())
}

fn run_one(args: &GitSmokeArgs, run_root: &Path, scenario: &str) -> Result<()> {
  let scenario_dir = run_root.join(scenario);
  fs::create_dir_all(&scenario_dir)?;
  match scenario {
    "stash_pop" => scenario_stash_pop(args, &scenario_dir),
    "stash_untracked_pop" => scenario_stash_untracked_pop(args, &scenario_dir),
    "apply_stash_conflict" => scenario_apply_stash_conflict(args, &scenario_dir),
    "pop_stash_conflict" => scenario_pop_stash_conflict(args, &scenario_dir),
    "merge_conflict_abort" => scenario_merge_conflict_abort(args, &scenario_dir),
    "merge_conflict_commit" => scenario_merge_conflict_commit(args, &scenario_dir),
    "rebase_conflict_continue" => scenario_rebase_conflict_continue(args, &scenario_dir),
    "rebase_conflict_skip" => scenario_rebase_conflict_skip(args, &scenario_dir),
    "rebase_conflict_abort" => scenario_rebase_conflict_abort(args, &scenario_dir),
    "force_push_dialog" => scenario_force_push_dialog(args, &scenario_dir),
    "stale_force_push_lease" => scenario_stale_force_push_lease(args, &scenario_dir),
    "branch_switch_dirty_conflict" => scenario_branch_switch_dirty_conflict(args, &scenario_dir),
    "pull_dirty_conflict" => scenario_pull_dirty_conflict(args, &scenario_dir),
    "detached_checkout" => scenario_detached_checkout(args, &scenario_dir),
    "cherry_pick" => scenario_cherry_pick(args, &scenario_dir),
    _ => bail!("unknown scenario: {scenario}"),
  }
}

fn scenario_stash_pop(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "v1\n", "initial")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;

  fs::write(repo.join("a.txt"), "v2\n")?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(1))?;

  driver.run_git_action(serde_json::json!({
    "action": "stash",
    "include_untracked": false,
    "message": "wip"
  }))?;
  wait_for_state(&mut driver, |state| {
    status_count(state) == Some(0) && stash_count(state) == Some(1)
  })?;
  expect_file(&repo, "a.txt", "v1\n")?;

  driver.run_git_action(serde_json::json!({
    "action": "pop_stash",
    "index": 0,
    "name": "wip"
  }))?;
  wait_for_state(&mut driver, |state| {
    status_count(state) == Some(1) && stash_count(state) == Some(0)
  })?;
  expect_file(&repo, "a.txt", "v2\n")?;
  driver.quit()
}

fn scenario_stash_untracked_pop(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "v1\n", "initial")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;

  fs::write(repo.join("untracked.txt"), "new\n")?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(1))?;

  driver.run_git_action(serde_json::json!({
    "action": "stash",
    "include_untracked": true,
    "message": "untracked"
  }))?;
  wait_for_state(&mut driver, |state| {
    status_count(state) == Some(0) && stash_count(state) == Some(1)
  })?;
  if repo.join("untracked.txt").exists() {
    bail!("untracked file stayed in the worktree after stash");
  }

  driver.run_git_action(serde_json::json!({
    "action": "pop_stash",
    "index": 0,
    "name": "untracked"
  }))?;
  wait_for_state(&mut driver, |state| {
    status_count(state) == Some(1) && stash_count(state) == Some(0)
  })?;
  expect_file(&repo, "untracked.txt", "new\n")?;
  driver.quit()
}

fn scenario_apply_stash_conflict(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  setup_stash_conflict(&repo)?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "apply_stash",
    "index": 0,
    "name": "stashed"
  }))?;
  wait_for_state(&mut driver, |state| {
    has_status(state, "a.txt", "Conflicted") && stash_count(state) == Some(1)
  })?;
  let contents = fs::read_to_string(repo.join("a.txt"))?;
  if !contents.contains("<<<<<<<") {
    bail!("applying the stash did not leave conflict markers");
  }
  driver.quit()
}

fn scenario_pop_stash_conflict(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  setup_stash_conflict(&repo)?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "pop_stash",
    "index": 0,
    "name": "stashed"
  }))?;
  wait_for_state(&mut driver, |state| {
    has_status(state, "a.txt", "Conflicted") && stash_count(state) == Some(1)
  })?;
  let contents = fs::read_to_string(repo.join("a.txt"))?;
  if !contents.contains("<<<<<<<") {
    bail!("popping the stash did not leave conflict markers");
  }
  driver.quit()
}

fn scenario_merge_conflict_abort(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  setup_merge_conflict(&repo)?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "merge_branch",
    "branch": { "name": "feature", "kind": "local" }
  }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "merge_in_progress") == Some(true)
      && selected_file(state) == Some("a.txt")
      && has_status(state, "a.txt", "Conflicted")
  })?;

  driver.run_git_action(serde_json::json!({ "action": "abort_merge" }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "merge_in_progress") == Some(false) && status_count(state) == Some(0)
  })?;
  expect_file(&repo, "a.txt", "main\n")?;
  driver.quit()
}

fn scenario_merge_conflict_commit(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  setup_merge_conflict(&repo)?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "merge_branch",
    "branch": { "name": "feature", "kind": "local" }
  }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "merge_in_progress") == Some(true) && has_status(state, "a.txt", "Conflicted")
  })?;

  fs::write(repo.join("a.txt"), "resolved\n")?;
  driver.run_git_action(serde_json::json!({ "action": "stage_all" }))?;
  confirm_active_dialog(&mut driver)?;
  wait_for_state(&mut driver, |state| has_stage(state, "a.txt", "Staged"))?;
  driver.run_git_action(serde_json::json!({
    "action": "commit",
    "message": "merge feature"
  }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "merge_in_progress") == Some(false) && status_count(state) == Some(0)
  })?;
  assert_eq_str(
    &git_output(&repo, ["log", "-1", "--pretty=%s"])?,
    "merge feature",
  )?;
  expect_file(&repo, "a.txt", "resolved\n")?;
  driver.quit()
}

fn scenario_rebase_conflict_continue(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  setup_rebase_conflict(&repo)?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "rebase_branch",
    "branch": { "name": "main", "kind": "local" }
  }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "rebase_in_progress") == Some(true)
      && selected_file(state) == Some("a.txt")
      && has_status(state, "a.txt", "Conflicted")
      && string_field(state, "commit_message") == Some("feature work")
  })?;

  fs::write(repo.join("a.txt"), "resolved\n")?;
  git(&repo, ["add", "a.txt"])?;
  driver.run_git_action(serde_json::json!({ "action": "continue_rebase" }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "rebase_in_progress") == Some(false) && status_count(state) == Some(0)
  })?;
  expect_file(&repo, "a.txt", "resolved\n")?;
  assert_eq_str(
    &git_output(&repo, ["rev-parse", "--abbrev-ref", "HEAD"])?,
    "feature",
  )?;
  driver.quit()
}

fn scenario_rebase_conflict_skip(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  setup_rebase_conflict(&repo)?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "rebase_branch",
    "branch": { "name": "main", "kind": "local" }
  }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "rebase_in_progress") == Some(true)
      && has_status(state, "a.txt", "Conflicted")
  })?;

  driver.run_git_action(serde_json::json!({ "action": "skip_rebase" }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "rebase_in_progress") == Some(false) && status_count(state) == Some(0)
  })?;
  expect_file(&repo, "a.txt", "main\n")?;
  assert_eq_str(
    &git_output(&repo, ["rev-parse", "--abbrev-ref", "HEAD"])?,
    "feature",
  )?;
  driver.quit()
}

fn scenario_rebase_conflict_abort(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  setup_rebase_conflict(&repo)?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "rebase_branch",
    "branch": { "name": "main", "kind": "local" }
  }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "rebase_in_progress") == Some(true)
      && has_status(state, "a.txt", "Conflicted")
  })?;

  driver.run_git_action(serde_json::json!({ "action": "abort_rebase" }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "rebase_in_progress") == Some(false) && status_count(state) == Some(0)
  })?;
  expect_file(&repo, "a.txt", "feature\n")?;
  assert_eq_str(
    &git_output(&repo, ["rev-parse", "--abbrev-ref", "HEAD"])?,
    "feature",
  )?;
  driver.quit()
}

fn scenario_force_push_dialog(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  let remote = dir.join("remote.git");
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
  let remote_before = git_bare_output(&remote, ["rev-parse", "refs/heads/main"])?;
  let local_after_rewrite = git_output(&repo, ["rev-parse", "HEAD"])?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  wait_for_state(&mut driver, |state| {
    state
      .get("palette_commands")
      .and_then(serde_json::Value::as_array)
      .is_some_and(|commands| commands.iter().any(|command| command == "force_push"))
  })?;

  driver.run_git_action(serde_json::json!({ "action": "force_push" }))?;
  let dialog = driver.command(serde_json::json!({ "cmd": "dialog_state" }))?;
  if bool_field(&dialog, "active") != Some(true) {
    bail!("force push did not open a dialog: {dialog}");
  }
  assert_eq_str(
    &git_bare_output(&remote, ["rev-parse", "refs/heads/main"])?,
    &remote_before,
  )?;

  driver.command(serde_json::json!({ "cmd": "cancel_dialog" }))?;
  assert_eq_str(
    &git_bare_output(&remote, ["rev-parse", "refs/heads/main"])?,
    &remote_before,
  )?;

  driver.run_git_action(serde_json::json!({ "action": "force_push" }))?;
  driver.command(serde_json::json!({ "cmd": "confirm_dialog" }))?;
  wait_until(DEFAULT_TIMEOUT, || {
    git_bare_output(&remote, ["rev-parse", "refs/heads/main"])
      .map(|sha| sha == local_after_rewrite)
      .unwrap_or(false)
  })?;
  driver.quit()
}

fn scenario_stale_force_push_lease(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  let remote = dir.join("remote.git");
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

  let other = dir.join("other");
  git_no_dir([
    "clone",
    remote.to_str().context("remote path")?,
    other.to_str().context("other path")?,
  ])?;
  git(&other, ["config", "user.name", "Reviu Smoke"])?;
  git(&other, ["config", "user.email", "smoke@reviu.test"])?;
  commit_file(&other, "a.txt", "remote v2\n", "remote v2")?;
  git(&other, ["push"])?;

  git(&repo, ["fetch"])?;
  commit_file(&repo, "a.txt", "local rewrite\n", "local rewrite")?;
  let local_head = git_output(&repo, ["rev-parse", "HEAD"])?;

  commit_file(&other, "a.txt", "remote v3\n", "remote v3")?;
  git(&other, ["push"])?;
  let remote_after_stale = git_bare_output(&remote, ["rev-parse", "refs/heads/main"])?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  wait_for_state(&mut driver, |state| {
    state
      .get("palette_commands")
      .and_then(serde_json::Value::as_array)
      .is_some_and(|commands| commands.iter().any(|command| command == "force_push"))
  })?;

  driver.run_git_action(serde_json::json!({ "action": "force_push" }))?;
  driver.command(serde_json::json!({ "cmd": "confirm_dialog" }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "command_in_flight") == Some(false)
  })?;
  assert_eq_str(
    &git_bare_output(&remote, ["rev-parse", "refs/heads/main"])?,
    &remote_after_stale,
  )?;
  if git_bare_output(&remote, ["rev-parse", "refs/heads/main"])? == local_head {
    bail!("stale force push overwrote the remote");
  }
  let notifications = driver.command(serde_json::json!({ "cmd": "notification_stats" }))?;
  if usize_field(&notifications, "count").unwrap_or(0) == 0 {
    bail!("stale force push did not report a notification");
  }
  driver.quit()
}

fn scenario_branch_switch_dirty_conflict(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "base\n", "initial")?;
  git(&repo, ["switch", "-c", "feature"])?;
  commit_file(&repo, "a.txt", "feature\n", "feature")?;
  git(&repo, ["switch", "main"])?;
  fs::write(repo.join("a.txt"), "dirty\n")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "switch_branch",
    "branch": { "name": "feature", "kind": "local" }
  }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "command_in_flight") == Some(false)
  })?;

  assert_eq_str(
    &git_output(&repo, ["rev-parse", "--abbrev-ref", "HEAD"])?,
    "main",
  )?;
  expect_file(&repo, "a.txt", "dirty\n")?;
  let notifications = driver.command(serde_json::json!({ "cmd": "notification_stats" }))?;
  if usize_field(&notifications, "count").unwrap_or(0) == 0 {
    bail!("dirty branch switch failure did not surface a notification");
  }
  driver.quit()
}

fn scenario_pull_dirty_conflict(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  let remote = dir.join("remote.git");
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

  let other = dir.join("other");
  git_no_dir([
    "clone",
    remote.to_str().context("remote path")?,
    other.to_str().context("other path")?,
  ])?;
  git(&other, ["config", "user.name", "Reviu Smoke"])?;
  git(&other, ["config", "user.email", "smoke@reviu.test"])?;
  commit_file(&other, "a.txt", "remote\n", "remote")?;
  git(&other, ["push"])?;
  fs::write(repo.join("a.txt"), "dirty\n")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(1))?;
  driver.run_git_action(serde_json::json!({ "action": "pull" }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "command_in_flight") == Some(false)
  })?;

  assert_eq_str(
    &git_output(&repo, ["rev-parse", "--abbrev-ref", "HEAD"])?,
    "main",
  )?;
  expect_file(&repo, "a.txt", "dirty\n")?;
  let notifications = driver.command(serde_json::json!({ "cmd": "notification_stats" }))?;
  if usize_field(&notifications, "count").unwrap_or(0) == 0 {
    bail!("dirty pull failure did not report a notification");
  }
  driver.quit()
}

fn scenario_detached_checkout(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "v1\n", "initial")?;
  let first = git_output(&repo, ["rev-parse", "HEAD"])?;
  commit_file(&repo, "a.txt", "v2\n", "second")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "checkout_detached",
    "target": first
  }))?;
  wait_for_state(&mut driver, |state| {
    state
      .get("branch_status")
      .and_then(|status| status.get("name"))
      .and_then(serde_json::Value::as_str)
      == Some("HEAD")
      && status_count(state) == Some(0)
  })?;
  expect_file(&repo, "a.txt", "v1\n")?;
  driver.quit()
}

fn scenario_cherry_pick(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "v1\n", "initial")?;
  git(&repo, ["switch", "-c", "feature"])?;
  commit_file(&repo, "b.txt", "picked\n", "pick me")?;
  let picked = git_output(&repo, ["rev-parse", "HEAD"])?;
  git(&repo, ["switch", "main"])?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "cherry_pick",
    "commit_hashes": [picked]
  }))?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(0))?;
  expect_file(&repo, "b.txt", "picked\n")?;
  assert_eq_str(&git_output(&repo, ["log", "-1", "--pretty=%s"])?, "pick me")?;
  driver.quit()
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
    let path =
      std::env::temp_dir().join(format!("reviu-git-smoke-{millis}-{}", std::process::id()));
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
  fn spawn(driver_bin: Option<&Path>, backend: &str, run_dir: &Path) -> Result<Self> {
    let home = run_dir.join("home");
    let config = run_dir.join("config");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(config.join("reviu.dev"))?;
    fs::write(
      config.join("reviu.dev/settings.json"),
      serde_json::json!({ "agent_notifications": false }).to_string(),
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
    if !ready
      .get("ok")
      .and_then(serde_json::Value::as_bool)
      .unwrap_or(false)
    {
      bail!("driver did not become ready: {ready}");
    }
    Ok(process)
  }

  fn open_repo(&mut self, repo: &Path) -> Result<()> {
    self.command(serde_json::json!({ "cmd": "path_prompt", "path": repo }))?;
    Ok(())
  }

  fn run_git_action(&mut self, action: serde_json::Value) -> Result<()> {
    self.command(serde_json::json!({ "cmd": "run_git_action", "action": action }))?;
    Ok(())
  }

  fn git_state(&mut self) -> Result<serde_json::Value> {
    self.command(serde_json::json!({ "cmd": "git_state" }))
  }

  fn quit(&mut self) -> Result<()> {
    self.command(serde_json::json!({ "cmd": "quit" }))?;
    Ok(())
  }

  fn command(&mut self, value: serde_json::Value) -> Result<serde_json::Value> {
    writeln!(self.stdin, "{value}")?;
    self.stdin.flush()?;
    let response = self.read_response()?;
    if !response
      .get("ok")
      .and_then(serde_json::Value::as_bool)
      .unwrap_or(false)
    {
      bail!("driver command failed: {response}");
    }
    Ok(response)
  }

  fn read_response(&mut self) -> Result<serde_json::Value> {
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

fn workspace_root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR"))
    .ancestors()
    .nth(2)
    .map(Path::to_path_buf)
    .unwrap_or_else(|| PathBuf::from("."))
}

fn confirm_active_dialog(driver: &mut DriverProcess) -> Result<()> {
  let dialog = driver.command(serde_json::json!({ "cmd": "dialog_state" }))?;
  if bool_field(&dialog, "active") == Some(true) {
    driver.command(serde_json::json!({ "cmd": "confirm_dialog" }))?;
  }
  Ok(())
}

fn wait_for_state(
  driver: &mut DriverProcess,
  predicate: impl Fn(&serde_json::Value) -> bool,
) -> Result<serde_json::Value> {
  let mut last = serde_json::Value::Null;
  wait_until(DEFAULT_TIMEOUT, || match driver.git_state() {
    Ok(state) => {
      last = state;
      predicate(&last)
    }
    Err(_) => false,
  })
  .with_context(|| format!("last git_state: {last}"))?;
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

fn setup_merge_conflict(repo: &Path) -> Result<()> {
  commit_file(repo, "a.txt", "base\n", "initial")?;
  git(repo, ["switch", "-c", "feature"])?;
  commit_file(repo, "a.txt", "feature\n", "feature work")?;
  git(repo, ["switch", "main"])?;
  commit_file(repo, "a.txt", "main\n", "main work")?;
  Ok(())
}

fn setup_rebase_conflict(repo: &Path) -> Result<()> {
  commit_file(repo, "a.txt", "base\n", "initial")?;
  git(repo, ["switch", "-c", "feature"])?;
  commit_file(repo, "a.txt", "feature\n", "feature work")?;
  git(repo, ["switch", "main"])?;
  commit_file(repo, "a.txt", "main\n", "main work")?;
  git(repo, ["switch", "feature"])?;
  Ok(())
}

fn setup_stash_conflict(repo: &Path) -> Result<()> {
  commit_file(repo, "a.txt", "base\n", "initial")?;
  fs::write(repo.join("a.txt"), "stashed\n")?;
  git(repo, ["stash", "push", "-m", "stashed"])?;
  commit_file(repo, "a.txt", "main\n", "main work")?;
  Ok(())
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

fn expect_file(repo: &Path, rel_path: &str, expected: &str) -> Result<()> {
  let actual = fs::read_to_string(repo.join(rel_path))?;
  if actual != expected {
    bail!("expected {rel_path} to be {expected:?}, got {actual:?}");
  }
  Ok(())
}

fn assert_eq_str(actual: &str, expected: &str) -> Result<()> {
  if actual.trim() != expected.trim() {
    bail!("expected {expected:?}, got {actual:?}");
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

fn status_count(state: &serde_json::Value) -> Option<usize> {
  state.get("status_entries")?.as_array().map(Vec::len)
}

fn stash_count(state: &serde_json::Value) -> Option<usize> {
  state.get("stashes")?.as_array().map(Vec::len)
}

fn has_status(state: &serde_json::Value, path: &str, status: &str) -> bool {
  state
    .get("status_entries")
    .and_then(serde_json::Value::as_array)
    .is_some_and(|entries| {
      entries.iter().any(|entry| {
        string_field(entry, "path") == Some(path) && string_field(entry, "status") == Some(status)
      })
    })
}

fn has_stage(state: &serde_json::Value, path: &str, stage: &str) -> bool {
  state
    .get("status_entries")
    .and_then(serde_json::Value::as_array)
    .is_some_and(|entries| {
      entries.iter().any(|entry| {
        string_field(entry, "path") == Some(path) && string_field(entry, "stage") == Some(stage)
      })
    })
}

fn selected_file(state: &serde_json::Value) -> Option<&str> {
  string_field(state, "selected_file")
}

fn string_field<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
  value.get(key)?.as_str()
}

fn bool_field(value: &serde_json::Value, key: &str) -> Option<bool> {
  value.get(key)?.as_bool()
}

fn usize_field(value: &serde_json::Value, key: &str) -> Option<usize> {
  value
    .get(key)?
    .as_u64()
    .and_then(|value| value.try_into().ok())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_args_defaults_to_all_test_scenarios() {
    let args = parse_args([]).expect("args");
    assert_eq!(args.backend, "test");
    assert_eq!(args.scenarios, SCENARIOS);
    assert_eq!(args.driver_bin, None);
    assert!(!args.keep_temp);
  }

  #[test]
  fn parse_args_accepts_overrides() {
    let args = parse_args([
      "--backend=visual".to_string(),
      "--driver-bin".to_string(),
      "/tmp/reviu-driver".to_string(),
      "--scenario".to_string(),
      "stash_pop,merge_conflict_abort".to_string(),
      "--keep-temp".to_string(),
    ])
    .expect("args");

    assert_eq!(args.backend, "visual");
    assert_eq!(args.driver_bin, Some(PathBuf::from("/tmp/reviu-driver")));
    assert_eq!(args.scenarios, ["stash_pop", "merge_conflict_abort"]);
    assert!(args.keep_temp);
  }

  #[test]
  fn parse_args_rejects_unknown_scenario() {
    let error = parse_args(["--scenario=unknown".to_string()]).expect_err("unknown scenario");
    assert!(error.to_string().contains("unknown scenario"));
  }

  #[test]
  fn status_helpers_read_driver_state() {
    let state = serde_json::json!({
      "selected_file": "a.txt",
      "status_entries": [
        { "path": "a.txt", "status": "Conflicted" }
      ],
      "stashes": [
        { "index": 0, "name": "wip" }
      ]
    });

    assert_eq!(selected_file(&state), Some("a.txt"));
    assert_eq!(status_count(&state), Some(1));
    assert_eq!(stash_count(&state), Some(1));
    assert!(has_status(&state, "a.txt", "Conflicted"));
  }
}
