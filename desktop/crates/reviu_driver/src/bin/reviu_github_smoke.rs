use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

// The live GitHub smoke runner shares the broader Git smoke harness but uses only a subset.
#[allow(dead_code)]
#[path = "../driver_harness.rs"]
mod driver_harness;

use crate::driver_harness::{
  DriverProcess, TempRunDir, git_output, pretty_json, scenario_diagnostics, wait_until,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
const ENABLE_ENV: &str = "REVIU_GITHUB_SMOKE";
const DEFAULT_OWNER: &str = "reviu-dev";
const DEFAULT_REPO: &str = "reviu-github-smoke";
const DEFAULT_PR_BRANCH: &str = "smoke/pr-open";
const DEFAULT_AUTH_TOKEN_ENV: &str = "REVIU_AUTH_TOKEN";

#[derive(Debug, PartialEq, Eq)]
struct GithubSmokeArgs {
  repo: PathBuf,
  driver_bin: Option<PathBuf>,
  backend: String,
  owner: String,
  name: String,
  pr_branch: String,
  auth_token_env: String,
  keep_temp: bool,
}

fn main() -> Result<()> {
  let args = parse_args(std::env::args().skip(1))?;
  run(args)
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<GithubSmokeArgs> {
  let mut repo = None;
  let mut driver_bin = None;
  let mut backend = "test".to_string();
  let mut owner = DEFAULT_OWNER.to_string();
  let mut name = DEFAULT_REPO.to_string();
  let mut pr_branch = DEFAULT_PR_BRANCH.to_string();
  let mut auth_token_env = DEFAULT_AUTH_TOKEN_ENV.to_string();
  let mut keep_temp = false;
  let mut args = args.into_iter();

  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--repo" => repo = Some(PathBuf::from(required_value(&mut args, "--repo")?)),
      "--driver-bin" => {
        driver_bin = Some(PathBuf::from(required_value(&mut args, "--driver-bin")?))
      }
      "--backend" => backend = required_value(&mut args, "--backend")?,
      "--owner" => owner = required_value(&mut args, "--owner")?,
      "--name" => name = required_value(&mut args, "--name")?,
      "--pr-branch" => pr_branch = required_value(&mut args, "--pr-branch")?,
      "--auth-token-env" => auth_token_env = required_value(&mut args, "--auth-token-env")?,
      "--keep-temp" => keep_temp = true,
      "--help" | "-h" => bail!(usage()),
      other if other.starts_with("--repo=") => {
        repo = Some(PathBuf::from(other.trim_start_matches("--repo=")));
      }
      other if other.starts_with("--driver-bin=") => {
        driver_bin = Some(PathBuf::from(other.trim_start_matches("--driver-bin=")));
      }
      other if other.starts_with("--backend=") => {
        backend = other.trim_start_matches("--backend=").to_string();
      }
      other if other.starts_with("--owner=") => {
        owner = other.trim_start_matches("--owner=").to_string();
      }
      other if other.starts_with("--name=") => {
        name = other.trim_start_matches("--name=").to_string();
      }
      other if other.starts_with("--pr-branch=") => {
        pr_branch = other.trim_start_matches("--pr-branch=").to_string();
      }
      other if other.starts_with("--auth-token-env=") => {
        auth_token_env = other.trim_start_matches("--auth-token-env=").to_string();
      }
      other => bail!("unknown argument: {other}\n{}", usage()),
    }
  }

  validate_backend(&backend)?;
  let repo = repo.context("--repo is required")?;
  Ok(GithubSmokeArgs {
    repo,
    driver_bin,
    backend,
    owner,
    name,
    pr_branch,
    auth_token_env,
    keep_temp,
  })
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
  "usage: REVIU_GITHUB_SMOKE=1 reviu-github-smoke --repo PATH [--backend test|visual] [--driver-bin PATH] [--owner OWNER] [--name REPO] [--pr-branch BRANCH] [--auth-token-env ENV] [--keep-temp]"
}

fn run(args: GithubSmokeArgs) -> Result<()> {
  if std::env::var(ENABLE_ENV).ok().as_deref() != Some("1") {
    bail!("set {ENABLE_ENV}=1 to run live GitHub smoke checks");
  }

  let mut run_dir = TempRunDir::new("reviu-github-smoke", args.keep_temp)?;
  println!("github smoke temp: {}", run_dir.path.display());
  let result = run_live_github_smoke(&args, &run_dir.path);
  if let Err(error) = result {
    run_dir.keep = true;
    eprintln!(
      "{}",
      scenario_diagnostics(&run_dir.path)
        .unwrap_or_else(|error| format!("failed to collect diagnostics: {error:#}"))
    );
    eprintln!("kept github smoke temp: {}", run_dir.path.display());
    return Err(error);
  }
  Ok(())
}

fn run_live_github_smoke(args: &GithubSmokeArgs, run_dir: &std::path::Path) -> Result<()> {
  ensure_expected_remote(args)?;
  ensure_gh_can_read_repo(args)?;
  let fixture_pr_number = ensure_open_fixture_pr(args)?;

  let mut driver = DriverProcess::spawn(args.driver_bin.as_deref(), &args.backend, run_dir, false)?;
  if let Some(token) = std::env::var(&args.auth_token_env)
    .ok()
    .filter(|token| !token.is_empty())
  {
    driver.command(json!({ "cmd": "set_auth_token", "token": token }))?;
  }
  let auth = wait_for_auth_state(&mut driver)?;
  println!(
    "Reviu auth: status={}, github_access={}, login={}",
    string_field(&auth, "status").unwrap_or("unknown"),
    string_field(&auth, "github_access").unwrap_or("unknown"),
    string_field(&auth, "github_login").unwrap_or("none")
  );
  if auth.get("has_github_access").and_then(Value::as_bool) != Some(true) {
    bail!(
      "Reviu GitHub auth is not available:\n{}",
      pretty_json(&auth)
    );
  }

  driver.open_repo(&args.repo)?;
  let state = wait_for_driver_state(&mut driver, |state| {
    github_remote_matches(state, &args.owner, &args.name)
      && palette_has_command(state, "show_pull_request")
  })?;
  println!("github remote: {}/{}", args.owner, args.name);
  println!(
    "opened branch: {}",
    current_branch(&state).unwrap_or("unknown")
  );

  let pull_request = wait_for_branch_pull_request(&mut driver)?;
  match branch_pull_request_status(&pull_request) {
    Some("found") => {
      let number = pull_request.get("number").and_then(Value::as_u64);
      if number != Some(fixture_pr_number) {
        bail!(
          "expected Reviu to find PR #{fixture_pr_number}, got:\n{}",
          pretty_json(&pull_request)
        );
      }
      println!("Reviu branch PR: #{}", fixture_pr_number);
    }
    Some("no_access") => {
      let auth = driver.command(json!({ "cmd": "auth_state" }))?;
      bail!(
        "Reviu GitHub auth missing or does not have access to {}/{}:\n{}",
        args.owner,
        args.name,
        pretty_json(&auth)
      )
    }
    Some(status) => bail!(
      "expected Reviu to find the branch pull request, got {status}:\n{}",
      pretty_json(&pull_request)
    ),
    None => bail!("missing branch_pull_request in driver state"),
  }

  driver.run_git_action(json!({ "action": "fetch" }))?;
  wait_for_driver_state(&mut driver, |state| {
    has_branch(state, &format!("origin/{}", args.pr_branch))
      && state.get("command_in_flight").and_then(Value::as_bool) == Some(false)
  })?;
  let notifications = driver.notification_log()?;
  if !has_logged_notification(&notifications, "success", "Fetched from remotes") {
    bail!(
      "fetch did not report the expected notification:\n{}",
      pretty_json(&notifications)
    );
  }
  driver.quit()?;
  Ok(())
}

fn ensure_expected_remote(args: &GithubSmokeArgs) -> Result<()> {
  let remote = git_output(&args.repo, ["remote", "get-url", "origin"])?;
  let expected_https = format!("github.com/{}/{}.git", args.owner, args.name);
  let expected_ssh = format!("github.com:{}/{}.git", args.owner, args.name);
  if !remote.contains(&expected_https) && !remote.contains(&expected_ssh) {
    bail!(
      "origin remote does not point at {}/{}: {remote}",
      args.owner,
      args.name
    );
  }
  Ok(())
}

fn ensure_gh_can_read_repo(args: &GithubSmokeArgs) -> Result<()> {
  let repo = format!("{}/{}", args.owner, args.name);
  let output = gh_json(["repo", "view", &repo, "--json", "nameWithOwner,isPrivate"])?;
  if string_field(&output, "nameWithOwner") != Some(repo.as_str()) {
    bail!("gh read the wrong repository: {}", pretty_json(&output));
  }
  if output.get("isPrivate").and_then(Value::as_bool) != Some(true) {
    bail!("expected {repo} to be private: {}", pretty_json(&output));
  }
  Ok(())
}

fn ensure_open_fixture_pr(args: &GithubSmokeArgs) -> Result<u64> {
  let repo = format!("{}/{}", args.owner, args.name);
  let pull_requests = gh_json([
    "pr",
    "list",
    "--repo",
    &repo,
    "--state",
    "open",
    "--head",
    &args.pr_branch,
    "--json",
    "number,title,headRefName,baseRefName,url",
  ])?;
  let Some(pull_request) = pull_requests.as_array().and_then(|items| items.first()) else {
    bail!("expected one open fixture PR from {}", args.pr_branch);
  };
  if string_field(pull_request, "headRefName") != Some(args.pr_branch.as_str())
    || string_field(pull_request, "baseRefName") != Some("main")
  {
    bail!("unexpected fixture PR: {}", pretty_json(pull_request));
  }
  let number = pull_request
    .get("number")
    .and_then(Value::as_u64)
    .context("fixture PR number")?;
  println!(
    "fixture PR: #{} {}",
    number,
    string_field(pull_request, "url").unwrap_or("unknown")
  );
  Ok(number)
}

fn gh_json<const N: usize>(args: [&str; N]) -> Result<Value> {
  let output = Command::new("gh")
    .env("NO_COLOR", "1")
    .env("CLICOLOR", "0")
    .env_remove("FORCE_COLOR")
    .env_remove("CLICOLOR_FORCE")
    .args(args)
    .output()
    .context("run gh")?;
  if !output.status.success() {
    bail!("gh failed: {}", command_output_details(&output));
  }
  serde_json::from_slice(&output.stdout).context("parse gh JSON")
}

fn wait_for_auth_state(driver: &mut DriverProcess) -> Result<Value> {
  let mut last = Value::Null;
  wait_until(DEFAULT_TIMEOUT, || {
    match driver.command(json!({ "cmd": "auth_state" })) {
      Ok(state) => {
        last = state;
        string_field(&last, "status").is_some_and(|status| status != "unknown")
      }
      Err(_) => false,
    }
  })
  .with_context(|| format!("last auth_state:\n{}", pretty_json(&last)))?;
  Ok(last)
}

fn wait_for_driver_state(
  driver: &mut DriverProcess,
  predicate: impl Fn(&Value) -> bool,
) -> Result<Value> {
  let mut last = Value::Null;
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

fn wait_for_branch_pull_request(driver: &mut DriverProcess) -> Result<Value> {
  let state = wait_for_driver_state(driver, |state| {
    branch_pull_request_status(state).is_some_and(|status| status != "loading")
  })?;
  state
    .get("branch_pull_request")
    .cloned()
    .context("branch_pull_request state")
}

fn branch_pull_request_status(state: &Value) -> Option<&str> {
  state
    .get("branch_pull_request")
    .unwrap_or(state)
    .get("status")
    .and_then(Value::as_str)
}

fn github_remote_matches(state: &Value, owner: &str, name: &str) -> bool {
  state.get("github_remote").is_some_and(|remote| {
    string_field(remote, "owner") == Some(owner) && string_field(remote, "repo") == Some(name)
  })
}

fn palette_has_command(state: &Value, command_id: &str) -> bool {
  state
    .get("palette_commands")
    .and_then(Value::as_array)
    .is_some_and(|commands| commands.iter().any(|command| command == command_id))
}

fn has_branch(state: &Value, name: &str) -> bool {
  state
    .get("branches")
    .and_then(Value::as_array)
    .is_some_and(|branches| {
      branches
        .iter()
        .any(|branch| string_field(branch, "name") == Some(name))
    })
}

fn has_logged_notification(log: &Value, kind: &str, message_part: &str) -> bool {
  let message_part = message_part.to_lowercase();
  log
    .get("notifications")
    .and_then(Value::as_array)
    .is_some_and(|notifications| {
      notifications.iter().any(|notification| {
        string_field(notification, "kind") == Some(kind)
          && string_field(notification, "message")
            .is_some_and(|message| message.to_lowercase().contains(&message_part))
      })
    })
}

fn current_branch(state: &Value) -> Option<&str> {
  state
    .get("branch_status")
    .and_then(|status| status.get("name"))
    .and_then(Value::as_str)
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
  value.get(key)?.as_str()
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_args_requires_repo() {
    assert!(parse_args([]).is_err());
  }

  #[test]
  fn parse_args_accepts_overrides() {
    let args = parse_args([
      "--repo=/tmp/repo".to_string(),
      "--driver-bin".to_string(),
      "/tmp/reviu-driver".to_string(),
      "--backend".to_string(),
      "visual".to_string(),
      "--owner".to_string(),
      "acme".to_string(),
      "--name=widget".to_string(),
      "--pr-branch".to_string(),
      "smoke/pr".to_string(),
      "--auth-token-env".to_string(),
      "TOKEN_ENV".to_string(),
      "--keep-temp".to_string(),
    ])
    .expect("args");

    assert_eq!(args.repo, PathBuf::from("/tmp/repo"));
    assert_eq!(args.driver_bin, Some(PathBuf::from("/tmp/reviu-driver")));
    assert_eq!(args.backend, "visual");
    assert_eq!(args.owner, "acme");
    assert_eq!(args.name, "widget");
    assert_eq!(args.pr_branch, "smoke/pr");
    assert_eq!(args.auth_token_env, "TOKEN_ENV");
    assert!(args.keep_temp);
  }

  #[test]
  fn parse_args_rejects_unknown_backend() {
    assert!(
      parse_args([
        "--repo=/tmp/repo".to_string(),
        "--backend=other".to_string()
      ])
      .is_err()
    );
  }

  #[test]
  fn github_remote_helper_reads_driver_state() {
    let state = json!({
      "github_remote": { "owner": "reviu-dev", "repo": "reviu-github-smoke" },
      "branch_pull_request": { "status": "found", "number": 1 },
      "palette_commands": ["show_pull_request"],
      "branches": [{ "name": "origin/smoke/pr-open" }],
      "notifications": [{ "kind": "success", "message": "Fetched from remotes" }]
    });

    assert!(github_remote_matches(
      &state,
      "reviu-dev",
      "reviu-github-smoke"
    ));
    assert_eq!(branch_pull_request_status(&state), Some("found"));
    assert!(palette_has_command(&state, "show_pull_request"));
    assert!(has_branch(&state, "origin/smoke/pr-open"));
    assert!(has_logged_notification(&state, "success", "fetched"));
  }
}
