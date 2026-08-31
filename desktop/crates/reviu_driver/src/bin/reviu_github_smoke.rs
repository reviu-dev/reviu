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
const DEFAULT_BOT_TOKEN_ENV: &str = "REVIU_GITHUB_BOT_TOKEN";
const DEFAULT_BOT_GH_USER: &str = "joris-gallot-bot";

struct FixturePullRequest {
  number: u64,
  head_oid: String,
  changed_files: Vec<String>,
}

struct BotActor {
  token: String,
  login: String,
}

struct BotReviewComment {
  id: u64,
  marker: String,
}

#[derive(Debug, PartialEq, Eq)]
struct GithubSmokeArgs {
  repo: PathBuf,
  driver_bin: Option<PathBuf>,
  backend: String,
  owner: String,
  name: String,
  pr_branch: String,
  auth_token_env: String,
  bot_token_env: String,
  bot_gh_user: String,
  require_bot: bool,
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
  let mut bot_token_env = DEFAULT_BOT_TOKEN_ENV.to_string();
  let mut bot_gh_user = DEFAULT_BOT_GH_USER.to_string();
  let mut require_bot = false;
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
      "--bot-token-env" => bot_token_env = required_value(&mut args, "--bot-token-env")?,
      "--bot-gh-user" => bot_gh_user = required_value(&mut args, "--bot-gh-user")?,
      "--require-bot" => require_bot = true,
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
      other if other.starts_with("--bot-token-env=") => {
        bot_token_env = other.trim_start_matches("--bot-token-env=").to_string();
      }
      other if other.starts_with("--bot-gh-user=") => {
        bot_gh_user = other.trim_start_matches("--bot-gh-user=").to_string();
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
    bot_token_env,
    bot_gh_user,
    require_bot,
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
  "usage: REVIU_GITHUB_SMOKE=1 reviu-github-smoke --repo PATH [--backend test|visual] [--driver-bin PATH] [--owner OWNER] [--name REPO] [--pr-branch BRANCH] [--auth-token-env ENV] [--bot-token-env ENV] [--bot-gh-user USER] [--require-bot] [--keep-temp]"
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
  let fixture_pr = ensure_open_fixture_pr(args)?;
  let bot_actor = resolve_bot_actor(args)?;
  let bot_review_comment = match &bot_actor {
    Some(bot_actor) => {
      let comment = create_bot_review_comment(args, &fixture_pr, bot_actor)?;
      println!(
        "bot actor: {} created PR review comment #{}",
        bot_actor.login, comment.id
      );
      Some(comment)
    }
    None => {
      println!("bot actor: skipped; no bot credentials available");
      None
    }
  };

  let result =
    run_live_github_driver_smoke(args, run_dir, &fixture_pr, bot_review_comment.as_ref());
  let cleanup = match (&bot_actor, &bot_review_comment) {
    (Some(bot_actor), Some(comment)) => delete_bot_review_comment(args, bot_actor, comment.id),
    _ => Ok(()),
  };
  if result.is_ok() {
    cleanup?;
  } else {
    let _ = cleanup;
  }
  result
}

fn run_live_github_driver_smoke(
  args: &GithubSmokeArgs,
  run_dir: &std::path::Path,
  fixture_pr: &FixturePullRequest,
  bot_review_comment: Option<&BotReviewComment>,
) -> Result<()> {
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
      if number != Some(fixture_pr.number) {
        bail!(
          "expected Reviu to find PR #{}, got:\n{}",
          fixture_pr.number,
          pretty_json(&pull_request)
        );
      }
      println!("Reviu branch PR: #{}", fixture_pr.number);
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

  driver.command(json!({ "cmd": "show_pull_request" }))?;
  let pull_request_panel = wait_for_pull_request_panel(
    &mut driver,
    fixture_pr,
    bot_review_comment.map(|comment| comment.marker.as_str()),
  )?;
  println!(
    "PR panel: {} changed file(s), {} review comment(s)",
    pull_request_panel
      .get("files")
      .and_then(Value::as_array)
      .map(Vec::len)
      .unwrap_or_default(),
    pull_request_panel
      .get("review_comments")
      .and_then(Value::as_u64)
      .unwrap_or_default()
  );
  let open_file = fixture_pr
    .changed_files
    .first()
    .context("fixture PR changed file")?;
  driver.command(json!({ "cmd": "open_pull_request_file", "path": open_file }))?;
  if let Some(comment) = bot_review_comment {
    wait_for_editor_review_comment(&mut driver, open_file, comment.id)?;
    println!("PR diff: review comment #{} visible", comment.id);
  }

  run_pending_review_smoke(&mut driver, open_file)?;

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

fn ensure_open_fixture_pr(args: &GithubSmokeArgs) -> Result<FixturePullRequest> {
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
  let number_text = number.to_string();
  let details = gh_json([
    "pr",
    "view",
    &number_text,
    "--repo",
    &repo,
    "--json",
    "files,headRefOid",
  ])?;
  let head_oid = string_field(&details, "headRefOid")
    .context("fixture PR headRefOid")?
    .to_string();
  let changed_files = details
    .get("files")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|file| string_field(file, "path").map(ToString::to_string))
    .collect::<Vec<_>>();
  if changed_files.is_empty() {
    bail!("fixture PR has no changed files: {}", pretty_json(&details));
  }
  println!(
    "fixture PR: #{} {}",
    number,
    string_field(pull_request, "url").unwrap_or("unknown")
  );
  Ok(FixturePullRequest {
    number,
    head_oid,
    changed_files,
  })
}

fn resolve_bot_actor(args: &GithubSmokeArgs) -> Result<Option<BotActor>> {
  if let Some(token) = std::env::var(&args.bot_token_env)
    .ok()
    .filter(|token| !token.is_empty())
  {
    return bot_actor_from_token(token, args).map(Some);
  }

  let output = gh_command(["auth", "token", "--user", &args.bot_gh_user], None)?;
  if !output.status.success() {
    if args.require_bot {
      bail!(
        "missing bot credentials: {}",
        command_output_details(&output)
      );
    }
    return Ok(None);
  }

  let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
  if token.is_empty() {
    if args.require_bot {
      bail!("gh returned an empty token for {}", args.bot_gh_user);
    }
    return Ok(None);
  }

  bot_actor_from_token(token, args).map(Some)
}

fn bot_actor_from_token(token: String, args: &GithubSmokeArgs) -> Result<BotActor> {
  let user = gh_json_with_token(["api", "user"], Some(&token))?;
  let login = string_field(&user, "login")
    .context("bot token user login")?
    .to_string();
  if login != args.bot_gh_user {
    bail!(
      "bot token belongs to {login}, expected {}",
      args.bot_gh_user
    );
  }
  Ok(BotActor { token, login })
}

fn create_bot_review_comment(
  args: &GithubSmokeArgs,
  fixture_pr: &FixturePullRequest,
  bot_actor: &BotActor,
) -> Result<BotReviewComment> {
  let marker = format!(
    "reviu-github-smoke bot comment {}",
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map(|duration| duration.as_secs())
      .unwrap_or_default()
  );
  let endpoint = format!(
    "repos/{}/{}/pulls/{}/comments",
    args.owner, args.name, fixture_pr.number
  );
  let body =
    format!("{marker}\n\nThis temporary comment is created and deleted by Reviu smoke tests.");
  let path = fixture_pr
    .changed_files
    .first()
    .context("fixture PR changed file")?;
  let comment = gh_json_vec_with_token(
    vec![
      "api".to_string(),
      endpoint,
      "-X".to_string(),
      "POST".to_string(),
      "-f".to_string(),
      format!("body={body}"),
      "-f".to_string(),
      format!("commit_id={}", fixture_pr.head_oid),
      "-f".to_string(),
      format!("path={path}"),
      "-F".to_string(),
      "position=1".to_string(),
    ],
    Some(&bot_actor.token),
  )?;
  let id = comment
    .get("id")
    .and_then(Value::as_u64)
    .with_context(|| format!("created bot comment has no id: {}", pretty_json(&comment)))?;
  Ok(BotReviewComment { id, marker })
}

fn delete_bot_review_comment(
  args: &GithubSmokeArgs,
  bot_actor: &BotActor,
  comment_id: u64,
) -> Result<()> {
  gh_json_vec_with_token(
    vec![
      "api".to_string(),
      format!(
        "repos/{}/{}/pulls/comments/{}",
        args.owner, args.name, comment_id
      ),
      "-X".to_string(),
      "DELETE".to_string(),
    ],
    Some(&bot_actor.token),
  )?;
  Ok(())
}

fn gh_json<const N: usize>(args: [&str; N]) -> Result<Value> {
  gh_json_with_token(args, None)
}

fn gh_json_with_token<const N: usize>(args: [&str; N], token: Option<&str>) -> Result<Value> {
  let output = gh_command(args, token)?;
  gh_output_json(output)
}

fn gh_json_vec_with_token(args: Vec<String>, token: Option<&str>) -> Result<Value> {
  let output = gh_command(args, token)?;
  gh_output_json(output)
}

fn gh_command(
  args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
  token: Option<&str>,
) -> Result<std::process::Output> {
  let mut command = Command::new("gh");
  command
    .env("NO_COLOR", "1")
    .env("CLICOLOR", "0")
    .env_remove("FORCE_COLOR")
    .env_remove("CLICOLOR_FORCE")
    .args(args);
  if let Some(token) = token {
    command.env("GH_TOKEN", token);
  }
  command.output().context("run gh")
}

fn gh_output_json(output: std::process::Output) -> Result<Value> {
  if !output.status.success() {
    bail!("gh failed: {}", command_output_details(&output));
  }
  if output.stdout.iter().all(|byte| byte.is_ascii_whitespace()) {
    return Ok(Value::Null);
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

fn wait_for_pull_request_panel(
  driver: &mut DriverProcess,
  fixture_pr: &FixturePullRequest,
  expected_comment_marker: Option<&str>,
) -> Result<Value> {
  let state = wait_for_driver_state(driver, |state| {
    let Some(panel) = state.get("pull_request_panel") else {
      return false;
    };
    string_field(panel, "active_tab") == Some("pull_request")
      && panel.get("files_loading").and_then(Value::as_bool) == Some(false)
      && panel.get("files_error").is_none_or(Value::is_null)
      && fixture_pr
        .changed_files
        .iter()
        .all(|path| pull_request_panel_has_file(panel, path))
      && expected_comment_marker
        .is_none_or(|marker| pull_request_panel_has_review_comment(panel, marker))
  })?;
  let Some(panel) = state.get("pull_request_panel").cloned() else {
    bail!(
      "driver state is missing pull_request_panel: {}",
      pretty_json(&state)
    );
  };
  Ok(panel)
}

fn run_pending_review_smoke(driver: &mut DriverProcess, path: &str) -> Result<()> {
  let marker = format!(
    "reviu-github-smoke primary pending {}",
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map(|duration| duration.as_secs())
      .unwrap_or_default()
  );
  driver.command(json!({
    "cmd": "create_pull_request_review_comment",
    "path": path,
    "line": 0,
    "body": marker,
  }))?;
  wait_for_pull_request_pending_comment(driver, &marker)?;
  println!("Review panel: pending PR comment created");

  driver.command(json!({ "cmd": "show_review" }))?;
  wait_for_review_panel_pending_comment(driver, &marker)?;
  println!("Review panel: pending PR comment visible");

  driver.command(json!({ "cmd": "discard_pull_request_review" }))?;
  driver.command(json!({ "cmd": "confirm_dialog" }))?;
  wait_for_pending_review_comment_removed(driver, &marker)?;
  println!("Review panel: pending PR review discarded");
  Ok(())
}

fn wait_for_editor_review_comment(
  driver: &mut DriverProcess,
  path: &str,
  comment_id: u64,
) -> Result<Value> {
  let mut last = Value::Null;
  wait_until(DEFAULT_TIMEOUT, || {
    match driver.command(json!({ "cmd": "editor_stats" })) {
      Ok(stats) => {
        last = stats;
        string_field(&last, "selected_file").is_some_and(|selected| selected.ends_with(path))
          && last.get("ready").and_then(Value::as_bool) == Some(true)
          && editor_has_review_comment(&last, comment_id)
      }
      Err(_) => false,
    }
  })
  .with_context(|| format!("last editor_stats:\n{}", pretty_json(&last)))?;
  Ok(last)
}

fn wait_for_pull_request_pending_comment(
  driver: &mut DriverProcess,
  marker: &str,
) -> Result<Value> {
  let state = wait_for_driver_state(driver, |state| {
    state
      .get("pull_request_panel")
      .is_some_and(|panel| pull_request_panel_has_pending_review_comment(panel, marker))
  })?;
  Ok(state)
}

fn wait_for_review_panel_pending_comment(
  driver: &mut DriverProcess,
  marker: &str,
) -> Result<Value> {
  let state = wait_for_driver_state(driver, |state| {
    state
      .get("review_panel")
      .is_some_and(|panel| review_panel_has_pull_request_comment(panel, marker))
  })?;
  Ok(state)
}

fn wait_for_pending_review_comment_removed(
  driver: &mut DriverProcess,
  marker: &str,
) -> Result<Value> {
  let state = wait_for_driver_state(driver, |state| {
    let pull_request_clear = state
      .get("pull_request_panel")
      .is_none_or(|panel| !pull_request_panel_has_pending_review_comment(panel, marker));
    let review_clear = state
      .get("review_panel")
      .is_none_or(|panel| !review_panel_has_pull_request_comment(panel, marker));
    pull_request_clear && review_clear
  })?;
  Ok(state)
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

fn pull_request_panel_has_file(panel: &Value, path: &str) -> bool {
  panel
    .get("files")
    .and_then(Value::as_array)
    .is_some_and(|files| {
      files
        .iter()
        .any(|file| string_field(file, "path") == Some(path))
    })
}

fn pull_request_panel_has_review_comment(panel: &Value, marker: &str) -> bool {
  panel
    .get("review_comment_details")
    .and_then(Value::as_array)
    .is_some_and(|comments| {
      comments
        .iter()
        .any(|comment| string_field(comment, "body").is_some_and(|body| body.contains(marker)))
    })
}

fn pull_request_panel_has_pending_review_comment(panel: &Value, marker: &str) -> bool {
  panel
    .get("review_comment_details")
    .and_then(Value::as_array)
    .is_some_and(|comments| {
      comments.iter().any(|comment| {
        comment.get("is_pending").and_then(Value::as_bool) == Some(true)
          && string_field(comment, "body").is_some_and(|body| body.contains(marker))
      })
    })
}

fn review_panel_has_pull_request_comment(panel: &Value, marker: &str) -> bool {
  panel
    .get("pull_request_comments")
    .and_then(Value::as_array)
    .is_some_and(|comments| {
      comments.iter().any(|comment| {
        string_field(comment, "status") == Some("pending")
          && string_field(comment, "excerpt").is_some_and(|excerpt| excerpt.contains(marker))
      })
    })
}

fn editor_has_review_comment(stats: &Value, comment_id: u64) -> bool {
  stats
    .get("review_comment_ids")
    .and_then(Value::as_array)
    .is_some_and(|ids| ids.iter().any(|id| id.as_u64() == Some(comment_id)))
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
      "--bot-token-env".to_string(),
      "BOT_TOKEN_ENV".to_string(),
      "--bot-gh-user".to_string(),
      "octo-bot".to_string(),
      "--require-bot".to_string(),
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
    assert_eq!(args.bot_token_env, "BOT_TOKEN_ENV");
    assert_eq!(args.bot_gh_user, "octo-bot");
    assert!(args.require_bot);
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
      "pull_request_panel": {
        "active_tab": "pull_request",
        "files": [{ "path": "fixtures/pr-open.txt", "kind": "modified" }],
        "review_comments": 1,
        "review_comment_details": [{ "body": "hello reviu-github-smoke bot comment", "user_login": "octo-bot" }]
      },
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
    let panel = state.get("pull_request_panel").expect("pull request panel");
    assert!(pull_request_panel_has_file(panel, "fixtures/pr-open.txt"));
    assert!(pull_request_panel_has_review_comment(
      panel,
      "reviu-github-smoke bot comment"
    ));
    assert!(has_logged_notification(&state, "success", "fetched"));
  }
}
