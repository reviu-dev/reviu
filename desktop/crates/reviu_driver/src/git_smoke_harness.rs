use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use crate::driver_harness::{
  DriverProcess, TempRunDir, assert_eq_str, clone_main, commit_file, expect_file, git,
  git_bare_output, git_lines, git_no_dir, git_output, init_repo, pretty_json, scenario_diagnostics,
  wait_until,
};

const DEFAULT_BACKEND: &str = "test";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const SCENARIOS: &[&str] = &[
  "stage_unstage_restore",
  "commit_amend_undo",
  "branch_create_switch_delete",
  "branch_switch_dirty_compatible",
  "push_pull_publish",
  "push_rejected_non_fast_forward",
  "fetch_updates_remote_refs",
  "pull_fast_forward",
  "create_branch_from_remote",
  "delete_remote_branch",
  "merge_remote_branch",
  "stash_pop",
  "stash_untracked_pop",
  "apply_stash_conflict",
  "pop_stash_conflict",
  "merge_conflict_abort",
  "merge_conflict_commit",
  "rebase_conflict_continue",
  "rebase_conflict_skip",
  "rebase_conflict_abort",
  "interactive_rebase_drop",
  "interactive_rebase_squash",
  "interactive_rebase_branch_conflict",
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
  pub(crate) fail_fast: bool,
  pub(crate) list: bool,
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
      fail_fast: false,
      list: false,
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
      "--fail-fast" => parsed.fail_fast = true,
      "--list" => parsed.list = true,
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
    "usage: reviu-git-smoke [--backend test|visual] [--driver-bin PATH] [--scenario {}] [--keep-temp] [--fail-fast] [--list]",
    SCENARIOS.join(",")
  )
}

pub(crate) fn run(args: GitSmokeArgs) -> Result<()> {
  if args.list {
    for scenario in SCENARIOS {
      println!("{scenario}");
    }
    return Ok(());
  }

  let mut run_dir = TempRunDir::new("reviu-git-smoke", args.keep_temp)?;
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
        eprintln!("rerun: {}", rerun_command(&args, scenario));
        eprintln!(
          "{}",
          scenario_diagnostics(&run_dir.path.join(scenario))
            .unwrap_or_else(|error| format!("failed to collect diagnostics: {error:#}"))
        );
        results.push((scenario.clone(), false));
        if args.fail_fast {
          break;
        }
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

fn rerun_command(args: &GitSmokeArgs, scenario: &str) -> String {
  let mut parts = vec!["target/debug/reviu-git-smoke".to_string()];
  parts.push("--backend".to_string());
  parts.push(shell_quote(&args.backend));
  if let Some(driver_bin) = &args.driver_bin {
    parts.push("--driver-bin".to_string());
    parts.push(shell_quote(&driver_bin.display().to_string()));
  }
  parts.push("--scenario".to_string());
  parts.push(shell_quote(scenario));
  parts.push("--keep-temp".to_string());
  parts.join(" ")
}

fn shell_quote(value: &str) -> String {
  if value
    .chars()
    .all(|character| character.is_ascii_alphanumeric() || "_./:-".contains(character))
  {
    return value.to_string();
  }
  format!("'{}'", value.replace('\'', "'\\''"))
}

fn run_one(args: &GitSmokeArgs, run_root: &Path, scenario: &str) -> Result<()> {
  let scenario_dir = run_root.join(scenario);
  fs::create_dir_all(&scenario_dir)?;
  match scenario {
    "stage_unstage_restore" => scenario_stage_unstage_restore(args, &scenario_dir),
    "commit_amend_undo" => scenario_commit_amend_undo(args, &scenario_dir),
    "branch_create_switch_delete" => scenario_branch_create_switch_delete(args, &scenario_dir),
    "branch_switch_dirty_compatible" => {
      scenario_branch_switch_dirty_compatible(args, &scenario_dir)
    }
    "push_pull_publish" => scenario_push_pull_publish(args, &scenario_dir),
    "push_rejected_non_fast_forward" => {
      scenario_push_rejected_non_fast_forward(args, &scenario_dir)
    }
    "fetch_updates_remote_refs" => scenario_fetch_updates_remote_refs(args, &scenario_dir),
    "pull_fast_forward" => scenario_pull_fast_forward(args, &scenario_dir),
    "create_branch_from_remote" => scenario_create_branch_from_remote(args, &scenario_dir),
    "delete_remote_branch" => scenario_delete_remote_branch(args, &scenario_dir),
    "merge_remote_branch" => scenario_merge_remote_branch(args, &scenario_dir),
    "stash_pop" => scenario_stash_pop(args, &scenario_dir),
    "stash_untracked_pop" => scenario_stash_untracked_pop(args, &scenario_dir),
    "apply_stash_conflict" => scenario_apply_stash_conflict(args, &scenario_dir),
    "pop_stash_conflict" => scenario_pop_stash_conflict(args, &scenario_dir),
    "merge_conflict_abort" => scenario_merge_conflict_abort(args, &scenario_dir),
    "merge_conflict_commit" => scenario_merge_conflict_commit(args, &scenario_dir),
    "rebase_conflict_continue" => scenario_rebase_conflict_continue(args, &scenario_dir),
    "rebase_conflict_skip" => scenario_rebase_conflict_skip(args, &scenario_dir),
    "rebase_conflict_abort" => scenario_rebase_conflict_abort(args, &scenario_dir),
    "interactive_rebase_drop" => scenario_interactive_rebase_drop(args, &scenario_dir),
    "interactive_rebase_squash" => scenario_interactive_rebase_squash(args, &scenario_dir),
    "interactive_rebase_branch_conflict" => {
      scenario_interactive_rebase_branch_conflict(args, &scenario_dir)
    }
    "force_push_dialog" => scenario_force_push_dialog(args, &scenario_dir),
    "stale_force_push_lease" => scenario_stale_force_push_lease(args, &scenario_dir),
    "branch_switch_dirty_conflict" => scenario_branch_switch_dirty_conflict(args, &scenario_dir),
    "pull_dirty_conflict" => scenario_pull_dirty_conflict(args, &scenario_dir),
    "detached_checkout" => scenario_detached_checkout(args, &scenario_dir),
    "cherry_pick" => scenario_cherry_pick(args, &scenario_dir),
    _ => bail!("unknown scenario: {scenario}"),
  }
}

fn scenario_stage_unstage_restore(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "v1\n", "initial")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;

  fs::write(repo.join("a.txt"), "v2\n")?;
  fs::write(repo.join("new.txt"), "new\n")?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(2))?;

  driver.run_git_action(serde_json::json!({ "action": "stage_all" }))?;
  wait_for_state(&mut driver, |state| {
    has_stage(state, "a.txt", "Staged") && has_stage(state, "new.txt", "Staged")
  })?;

  driver.run_git_action(serde_json::json!({ "action": "unstage_all" }))?;
  wait_for_state(&mut driver, |state| {
    has_stage(state, "a.txt", "Unstaged") && has_stage(state, "new.txt", "Unstaged")
  })?;

  driver.run_git_action(serde_json::json!({ "action": "restore_all" }))?;
  driver.command(serde_json::json!({ "cmd": "confirm_dialog" }))?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(0))?;
  expect_file(&repo, "a.txt", "v1\n")?;
  if repo.join("new.txt").exists() {
    bail!("restore all left the untracked file behind");
  }
  driver.quit()
}

fn scenario_commit_amend_undo(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "v1\n", "initial")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;

  fs::write(repo.join("a.txt"), "v2\n")?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(1))?;
  driver.run_git_action(serde_json::json!({ "action": "stage_all" }))?;
  wait_for_state(&mut driver, |state| has_stage(state, "a.txt", "Staged"))?;
  driver.run_git_action(serde_json::json!({
    "action": "commit",
    "message": "second"
  }))?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(0))?;
  assert_eq_str(&git_output(&repo, ["log", "-1", "--pretty=%s"])?, "second")?;

  driver.run_git_action(serde_json::json!({
    "action": "amend",
    "message": "second amended"
  }))?;
  wait_until(DEFAULT_TIMEOUT, || {
    git_output(&repo, ["log", "-1", "--pretty=%s"])
      .map(|summary| summary == "second amended")
      .unwrap_or(false)
  })?;
  let history = git_lines(&repo, ["log", "--pretty=%s", "--reverse"])?;
  if history != ["initial", "second amended"] {
    bail!("amend added a commit instead of rewriting: {history:?}");
  }

  driver.run_git_action(serde_json::json!({ "action": "undo_last_commit" }))?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(1))?;
  let history = git_lines(&repo, ["log", "--pretty=%s", "--reverse"])?;
  if history != ["initial"] {
    bail!("undo left unexpected history: {history:?}");
  }
  expect_file(&repo, "a.txt", "v2\n")?;
  driver.quit()
}

fn scenario_branch_create_switch_delete(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "v1\n", "initial")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "create_branch",
    "name": "feature"
  }))?;
  wait_for_branch(&mut driver, "feature")?;
  commit_file(&repo, "feature.txt", "feature\n", "feature work")?;

  driver.run_git_action(serde_json::json!({
    "action": "switch_branch",
    "branch": { "name": "main", "kind": "local" }
  }))?;
  wait_for_branch(&mut driver, "main")?;
  driver.run_git_action(serde_json::json!({
    "action": "create_branch_from",
    "name": "from-main",
    "base": { "name": "main", "kind": "local" }
  }))?;
  wait_for_branch(&mut driver, "from-main")?;
  if repo.join("feature.txt").exists() {
    bail!("branch created from main contains feature-only file");
  }

  driver.run_git_action(serde_json::json!({
    "action": "switch_branch",
    "branch": { "name": "main", "kind": "local" }
  }))?;
  wait_for_branch(&mut driver, "main")?;
  driver.run_git_action(serde_json::json!({
    "action": "delete_branch",
    "branch": { "name": "feature", "kind": "local" }
  }))?;
  wait_for_state(&mut driver, |state| !has_branch(state, "feature"))?;
  driver.quit()
}

fn scenario_branch_switch_dirty_compatible(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "v1\n", "initial")?;
  git(&repo, ["switch", "-c", "feature"])?;
  commit_file(&repo, "b.txt", "feature\n", "feature work")?;
  git(&repo, ["switch", "main"])?;
  fs::write(repo.join("a.txt"), "dirty\n")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(1))?;
  driver.run_git_action(serde_json::json!({
    "action": "switch_branch",
    "branch": { "name": "feature", "kind": "local" }
  }))?;
  wait_for_branch(&mut driver, "feature")?;
  expect_file(&repo, "a.txt", "dirty\n")?;
  expect_file(&repo, "b.txt", "feature\n")?;
  driver.quit()
}

fn scenario_push_pull_publish(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
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
  git(&repo, ["switch", "-c", "feature"])?;
  commit_file(&repo, "feature.txt", "feature\n", "feature work")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({ "action": "push" }))?;
  wait_until(DEFAULT_TIMEOUT, || {
    git_bare_output(&remote, ["rev-parse", "refs/heads/feature"]).is_ok()
  })?;

  driver.run_git_action(serde_json::json!({
    "action": "switch_branch",
    "branch": { "name": "main", "kind": "local" }
  }))?;
  wait_for_branch(&mut driver, "main")?;

  let other = dir.join("other");
  git_no_dir([
    "clone",
    "--branch",
    "main",
    remote.to_str().context("remote path")?,
    other.to_str().context("other path")?,
  ])?;
  git(&other, ["config", "user.name", "Reviu Smoke"])?;
  git(&other, ["config", "user.email", "smoke@reviu.test"])?;
  commit_file(&other, "remote.txt", "remote\n", "remote work")?;
  git(&other, ["push"])?;

  driver.run_git_action(serde_json::json!({ "action": "pull" }))?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(0))?;
  expect_file(&repo, "remote.txt", "remote\n")?;
  driver.quit()
}

fn scenario_push_rejected_non_fast_forward(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
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
  clone_main(&remote, &other)?;
  commit_file(&other, "remote.txt", "remote\n", "remote work")?;
  git(&other, ["push"])?;
  let remote_head = git_bare_output(&remote, ["rev-parse", "refs/heads/main"])?;
  commit_file(&repo, "local.txt", "local\n", "local work")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({ "action": "push" }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "command_in_flight") == Some(false)
  })?;
  assert_eq_str(
    &git_bare_output(&remote, ["rev-parse", "refs/heads/main"])?,
    &remote_head,
  )?;
  let notifications = driver.notification_log()?;
  if !has_logged_notification(&notifications, "error", "push") {
    bail!(
      "push rejection did not report the expected notification:\n{}",
      pretty_json(&notifications)
    );
  }
  driver.quit()
}

fn scenario_fetch_updates_remote_refs(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
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
  clone_main(&remote, &other)?;
  commit_file(&other, "remote.txt", "remote\n", "remote work")?;
  git(&other, ["push"])?;
  let remote_head = git_bare_output(&remote, ["rev-parse", "refs/heads/main"])?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({ "action": "fetch" }))?;
  wait_until(DEFAULT_TIMEOUT, || {
    git_output(&repo, ["rev-parse", "refs/remotes/origin/main"])
      .map(|head| head == remote_head)
      .unwrap_or(false)
  })?;
  let notifications = driver.notification_log()?;
  if !has_logged_notification(&notifications, "success", "Fetched from remotes") {
    bail!(
      "fetch did not report the expected notification:\n{}",
      pretty_json(&notifications)
    );
  }
  driver.quit()
}

fn scenario_pull_fast_forward(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
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
  clone_main(&remote, &other)?;
  commit_file(&other, "remote.txt", "remote\n", "remote work")?;
  git(&other, ["push"])?;
  let remote_head = git_bare_output(&remote, ["rev-parse", "refs/heads/main"])?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({ "action": "pull" }))?;
  wait_until(DEFAULT_TIMEOUT, || {
    git_output(&repo, ["rev-parse", "HEAD"])
      .map(|head| head == remote_head)
      .unwrap_or(false)
  })?;
  expect_file(&repo, "remote.txt", "remote\n")?;
  let notifications = driver.notification_log()?;
  if !has_logged_notification(&notifications, "success", "Pulled from the remote branch") {
    bail!(
      "pull did not report the expected notification:\n{}",
      pretty_json(&notifications)
    );
  }
  driver.quit()
}

fn scenario_create_branch_from_remote(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  let remote = dir.join("remote.git");
  setup_remote_feature_branch(&repo, &remote, dir)?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({ "action": "fetch" }))?;
  wait_for_state(&mut driver, |state| has_branch(state, "origin/feature"))?;
  driver.run_git_action(serde_json::json!({
    "action": "create_branch_from",
    "name": "local-feature",
    "base": { "name": "origin/feature", "kind": "remote" }
  }))?;
  wait_for_branch(&mut driver, "local-feature")?;
  expect_file(&repo, "feature.txt", "feature\n")?;
  let notifications = driver.notification_log()?;
  if !has_logged_notification(&notifications, "success", "Created branch local-feature") {
    bail!(
      "remote branch creation did not report the expected notification:\n{}",
      pretty_json(&notifications)
    );
  }
  driver.quit()
}

fn scenario_delete_remote_branch(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  let remote = dir.join("remote.git");
  setup_remote_feature_branch(&repo, &remote, dir)?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({ "action": "fetch" }))?;
  wait_for_state(&mut driver, |state| has_branch(state, "origin/feature"))?;
  driver.run_git_action(serde_json::json!({
    "action": "delete_branch",
    "branch": { "name": "origin/feature", "kind": "remote" }
  }))?;
  wait_until(DEFAULT_TIMEOUT, || {
    git_bare_output(&remote, ["rev-parse", "refs/heads/feature"]).is_err()
  })?;
  wait_for_state(&mut driver, |state| !has_branch(state, "origin/feature"))?;
  let notifications = driver.notification_log()?;
  if !has_logged_notification(&notifications, "success", "Deleted branch origin/feature") {
    bail!(
      "remote branch deletion did not report the expected notification:\n{}",
      pretty_json(&notifications)
    );
  }
  driver.quit()
}

fn scenario_merge_remote_branch(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  let remote = dir.join("remote.git");
  setup_remote_feature_branch(&repo, &remote, dir)?;
  commit_file(&repo, "local.txt", "local\n", "local work")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({ "action": "fetch" }))?;
  wait_for_state(&mut driver, |state| has_branch(state, "origin/feature"))?;
  driver.run_git_action(serde_json::json!({
    "action": "merge_branch",
    "branch": { "name": "origin/feature", "kind": "remote" }
  }))?;
  wait_for_state(&mut driver, |state| status_count(state) == Some(0))?;
  expect_file(&repo, "feature.txt", "feature\n")?;
  assert_eq_str(
    &git_output(&repo, ["log", "-1", "--pretty=%s"])?,
    "Merge branch 'origin/feature'",
  )?;
  let notifications = driver.notification_log()?;
  if !has_logged_notification(&notifications, "success", "Merged origin/feature") {
    bail!(
      "remote branch merge did not report the expected notification:\n{}",
      pretty_json(&notifications)
    );
  }
  driver.quit()
}

fn scenario_stash_pop(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "v1\n", "initial")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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

fn scenario_interactive_rebase_drop(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "first\n", "first")?;
  commit_file(&repo, "b.txt", "second\n", "second")?;
  commit_file(&repo, "c.txt", "third\n", "third")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "interactive_rebase",
    "target": { "target": "head_count", "count": 2 },
    "actions": ["pick", "drop"]
  }))?;
  wait_until(DEFAULT_TIMEOUT, || {
    git_lines(&repo, ["log", "--pretty=%s", "--reverse"])
      .map(|summaries| summaries == ["first", "second"])
      .unwrap_or(false)
  })?;
  if repo.join("c.txt").exists() {
    bail!("dropped commit file still exists");
  }
  driver.quit()
}

fn scenario_interactive_rebase_squash(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "first\n", "first")?;
  commit_file(&repo, "b.txt", "second\n", "second")?;
  commit_file(&repo, "c.txt", "third\n", "third")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "interactive_rebase",
    "target": { "target": "head_count", "count": 2 },
    "actions": ["pick", "squash"]
  }))?;
  wait_until(DEFAULT_TIMEOUT, || {
    git_lines(&repo, ["log", "--pretty=%s", "--reverse"])
      .map(|summaries| summaries.len() == 2)
      .unwrap_or(false)
  })?;
  let summaries = git_lines(&repo, ["log", "--pretty=%s", "--reverse"])?;
  if summaries != ["first", "second"] {
    bail!("unexpected squashed history: {summaries:?}");
  }
  expect_file(&repo, "b.txt", "second\n")?;
  expect_file(&repo, "c.txt", "third\n")?;
  driver.quit()
}

fn scenario_interactive_rebase_branch_conflict(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  setup_rebase_conflict(&repo)?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
  driver.open_repo(&repo)?;
  driver.run_git_action(serde_json::json!({
    "action": "interactive_rebase",
    "target": {
      "target": "branch",
      "branch": { "name": "main", "kind": "local" }
    },
    "actions": ["pick"]
  }))?;
  wait_for_state(&mut driver, |state| {
    bool_field(state, "rebase_in_progress") == Some(true)
      && selected_file(state) == Some("a.txt")
      && has_status(state, "a.txt", "Conflicted")
      && string_field(state, "commit_message") == Some("feature work")
  })?;
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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
    "--branch",
    "main",
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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
  let notifications = driver.notification_log()?;
  if !has_logged_notification(&notifications, "error", "fetch before force pushing") {
    bail!(
      "stale force push did not report the expected notification:\n{}",
      pretty_json(&notifications)
    );
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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
  let notifications = driver.notification_log()?;
  if !has_logged_notification(&notifications, "error", "checkout target tree") {
    bail!(
      "dirty branch switch failure did not report the expected notification:\n{}",
      pretty_json(&notifications)
    );
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
    "--branch",
    "main",
    remote.to_str().context("remote path")?,
    other.to_str().context("other path")?,
  ])?;
  git(&other, ["config", "user.name", "Reviu Smoke"])?;
  git(&other, ["config", "user.email", "smoke@reviu.test"])?;
  commit_file(&other, "a.txt", "remote\n", "remote")?;
  git(&other, ["push"])?;
  fs::write(repo.join("a.txt"), "dirty\n")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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
  let notifications = driver.notification_log()?;
  if !has_logged_notification(&notifications, "error", "local changes") {
    bail!(
      "dirty pull failure did not report the expected notification:\n{}",
      pretty_json(&notifications)
    );
  }
  driver.quit()
}

fn scenario_detached_checkout(args: &GitSmokeArgs, dir: &Path) -> Result<()> {
  let repo = init_repo(&dir.join("repo"))?;
  commit_file(&repo, "a.txt", "v1\n", "initial")?;
  let first = git_output(&repo, ["rev-parse", "HEAD"])?;
  commit_file(&repo, "a.txt", "v2\n", "second")?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, dir, true)?;
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

fn confirm_active_dialog(driver: &mut DriverProcess) -> Result<()> {
  let dialog = driver.command(serde_json::json!({ "cmd": "dialog_state" }))?;
  if bool_field(&dialog, "active") == Some(true) {
    driver.command(serde_json::json!({ "cmd": "confirm_dialog" }))?;
  }
  Ok(())
}

fn wait_for_branch(driver: &mut DriverProcess, branch: &str) -> Result<serde_json::Value> {
  wait_for_state(driver, |state| current_branch(state) == Some(branch))
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
  .with_context(|| format!("last git_state:\n{}", pretty_json(&last)))?;
  Ok(last)
}

fn setup_remote_feature_branch(repo: &Path, remote: &Path, dir: &Path) -> Result<()> {
  git_no_dir(["init", "--bare", remote.to_str().context("remote path")?])?;
  commit_file(repo, "a.txt", "v1\n", "initial")?;
  git(
    repo,
    [
      "remote",
      "add",
      "origin",
      remote.to_str().context("remote path")?,
    ],
  )?;
  git(repo, ["push", "-u", "origin", "main"])?;

  let other = dir.join("remote-feature-source");
  clone_main(remote, &other)?;
  git(&other, ["switch", "-c", "feature"])?;
  commit_file(&other, "feature.txt", "feature\n", "feature work")?;
  git(&other, ["push", "-u", "origin", "feature"])?;
  Ok(())
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

fn status_count(state: &serde_json::Value) -> Option<usize> {
  state.get("status_entries")?.as_array().map(Vec::len)
}

fn stash_count(state: &serde_json::Value) -> Option<usize> {
  state.get("stashes")?.as_array().map(Vec::len)
}

fn has_logged_notification(log: &serde_json::Value, kind: &str, message_part: &str) -> bool {
  let message_part = message_part.to_lowercase();
  log
    .get("notifications")
    .and_then(serde_json::Value::as_array)
    .is_some_and(|notifications| {
      notifications.iter().any(|notification| {
        string_field(notification, "kind") == Some(kind)
          && string_field(notification, "message")
            .is_some_and(|message| message.to_lowercase().contains(&message_part))
      })
    })
}

fn current_branch(state: &serde_json::Value) -> Option<&str> {
  state
    .get("branch_status")
    .and_then(|status| status.get("name"))
    .and_then(serde_json::Value::as_str)
}

fn has_branch(state: &serde_json::Value, name: &str) -> bool {
  state
    .get("branches")
    .and_then(serde_json::Value::as_array)
    .is_some_and(|branches| {
      branches
        .iter()
        .any(|branch| string_field(branch, "name") == Some(name))
    })
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
    assert!(!args.fail_fast);
    assert!(!args.list);
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
      "--fail-fast".to_string(),
      "--list".to_string(),
    ])
    .expect("args");

    assert_eq!(args.backend, "visual");
    assert_eq!(args.driver_bin, Some(PathBuf::from("/tmp/reviu-driver")));
    assert_eq!(args.scenarios, ["stash_pop", "merge_conflict_abort"]);
    assert!(args.keep_temp);
    assert!(args.fail_fast);
    assert!(args.list);
  }

  #[test]
  fn rerun_command_includes_repro_flags() {
    let args = GitSmokeArgs {
      backend: "test".to_string(),
      driver_bin: Some(PathBuf::from("/tmp/reviu driver")),
      scenarios: vec!["stash_pop".to_string()],
      keep_temp: false,
      fail_fast: false,
      list: false,
    };

    assert_eq!(
      rerun_command(&args, "stash_pop"),
      "target/debug/reviu-git-smoke --backend test --driver-bin '/tmp/reviu driver' --scenario stash_pop --keep-temp"
    );
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
      ],
      "notifications": [
        { "kind": "error", "message": "local changes would be overwritten" }
      ]
    });

    assert_eq!(selected_file(&state), Some("a.txt"));
    assert_eq!(status_count(&state), Some(1));
    assert_eq!(stash_count(&state), Some(1));
    assert!(has_status(&state, "a.txt", "Conflicted"));
    assert!(has_logged_notification(&state, "error", "local changes"));
  }
}
