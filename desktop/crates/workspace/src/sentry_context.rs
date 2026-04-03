use std::{
  collections::HashMap,
  error::Error,
  path::Path,
  sync::{Mutex, OnceLock},
  time::{Duration, Instant},
};

use sentry::protocol::{Breadcrumb, Context, Level, Map, User, Value};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{auth_state::AuthState, workspace::WorkspacePage};

const DEDUP_WINDOW: Duration = Duration::from_secs(300);

fn dedup_state() -> &'static Mutex<HashMap<String, Instant>> {
  static STATE: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
  STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn crash_snapshot_state() -> &'static Mutex<CrashContextSnapshot> {
  static STATE: OnceLock<Mutex<CrashContextSnapshot>> = OnceLock::new();
  STATE.get_or_init(|| Mutex::new(CrashContextSnapshot::default()))
}

fn update_crash_snapshot(f: impl FnOnce(&mut CrashContextSnapshot)) {
  let Ok(mut snapshot) = crash_snapshot_state().lock() else {
    return;
  };
  f(&mut snapshot);
}

fn should_capture_error(key: &str, now: Instant) -> bool {
  let Ok(mut state) = dedup_state().lock() else {
    return true;
  };

  state.retain(|_, captured_at| now.duration_since(*captured_at) < DEDUP_WINDOW);
  match state.get(key).copied() {
    Some(previous) if now.duration_since(previous) < DEDUP_WINDOW => false,
    _ => {
      state.insert(key.to_string(), now);
      true
    }
  }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CrashGitContext {
  pub repo_name: Option<String>,
  pub repo_hash: Option<String>,
  pub selected_file: Option<String>,
  pub branch: Option<String>,
  pub sidebar_mode: String,
  pub diff_view: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CrashGithubPrContext {
  pub owner: String,
  pub repo: String,
  pub number: u64,
  pub selected_file: Option<String>,
  pub active_tab: Option<usize>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CrashContextSnapshot {
  pub pathname: Option<String>,
  pub workspace_page: Option<String>,
  pub git: Option<CrashGitContext>,
  pub github_pr: Option<CrashGithubPrContext>,
}

fn workspace_page_tag(page: WorkspacePage) -> &'static str {
  match page {
    WorkspacePage::Git => "git",
    WorkspacePage::Github => "github",
    WorkspacePage::GithubRepo => "github_repo",
    WorkspacePage::GithubPrDetails => "github_pr_details",
    WorkspacePage::Billing => "billing",
    WorkspacePage::GitConfig => "git_config",
    WorkspacePage::Settings => "settings",
    WorkspacePage::About => "about",
  }
}

fn auth_state_tag(state: &AuthState) -> &'static str {
  match state {
    AuthState::Unknown => "unknown",
    AuthState::Authenticated(_) => "authenticated",
    AuthState::Unauthenticated => "unauthenticated",
  }
}

pub(crate) fn sanitize_repo_path(repo_root: &Path) -> (String, String) {
  let display = repo_root.to_string_lossy().into_owned();
  let name = repo_root
    .file_name()
    .and_then(|segment| segment.to_str())
    .filter(|segment| !segment.is_empty())
    .unwrap_or("repo")
    .to_string();

  let mut hasher = Sha256::new();
  hasher.update(display.as_bytes());
  let hash = format!("{:x}", hasher.finalize());
  (name, hash[..12].to_string())
}

fn to_unknown_context(map: Map<String, Value>) -> Context {
  Context::Other(map)
}

pub(crate) fn add_breadcrumb(category: &str, message: &str, data: Map<String, Value>) {
  sentry::add_breadcrumb(Breadcrumb {
    level: Level::Info,
    category: Some(category.to_string()),
    message: Some(message.to_string()),
    data,
    ..Default::default()
  });
}

pub(crate) fn record_expected_error(operation: &str, reason: &str, mut data: Map<String, Value>) {
  data.insert("operation".into(), operation.to_string().into());
  data.insert("reason".into(), reason.to_string().into());
  data.insert("expected".into(), true.into());
  add_breadcrumb(
    "app.expected_error",
    format!("{operation}: {reason}").as_str(),
    data,
  );
}

pub(crate) fn capture_unexpected_error(
  op: &'static str,
  err: &dyn Error,
  mut data: Map<String, Value>,
) {
  let dedup_key = format!("{op}|{err}");
  if !should_capture_error(dedup_key.as_str(), Instant::now()) {
    data.insert("operation".into(), op.to_string().into());
    data.insert("deduplicated".into(), true.into());
    data.insert("error".into(), err.to_string().into());
    add_breadcrumb(
      "app.unexpected_error",
      "Deduplicated unexpected error",
      data,
    );
    return;
  }

  data.insert("operation".into(), op.to_string().into());
  data.insert("error".into(), err.to_string().into());
  sentry::with_scope(
    |scope| {
      scope.set_tag("error.kind", "unexpected");
      scope.set_tag("error.op", op);
      for (key, value) in data {
        scope.set_extra(key.as_str(), value);
      }
    },
    || {
      let _ = sentry::capture_error(err);
    },
  );
}

pub(crate) fn record_http_status(method: &str, route: &str, status: u16) {
  let mut data = Map::new();
  data.insert("method".into(), method.to_string().into());
  data.insert("route".into(), route.to_string().into());
  data.insert("status".into(), status.into());

  if status == 401 {
    record_expected_error("api.http", "unauthorized", data);
    return;
  }

  if status >= 400 {
    let error = std::io::Error::other(format!("unexpected HTTP status {status}"));
    capture_unexpected_error("api.http", &error, data);
  }
}

pub(crate) fn sync_workspace_page(from: Option<WorkspacePage>, to: WorkspacePage) {
  let from_tag = from.map(workspace_page_tag);
  let to_tag = workspace_page_tag(to);

  sentry::configure_scope(|scope| {
    scope.set_tag("workspace.page", to_tag);

    let mut context = Map::new();
    if let Some(previous) = from_tag {
      context.insert("from".into(), previous.to_string().into());
    }
    context.insert("to".into(), to_tag.to_string().into());
    scope.set_context("ui_state", to_unknown_context(context));
  });

  let mut breadcrumb_data = Map::new();
  if let Some(previous) = from_tag {
    breadcrumb_data.insert("from".into(), previous.to_string().into());
  }
  breadcrumb_data.insert("to".into(), to_tag.to_string().into());
  add_breadcrumb("ui.navigation", "Workspace page changed", breadcrumb_data);
}

pub(crate) fn sync_workspace_route(pathname: &str, page: WorkspacePage) {
  update_crash_snapshot(|snapshot| {
    snapshot.pathname = Some(pathname.to_string());
    snapshot.workspace_page = Some(workspace_page_tag(page).to_string());
  });
}

pub(crate) fn current_crash_context_snapshot() -> CrashContextSnapshot {
  crash_snapshot_state()
    .lock()
    .map(|snapshot| snapshot.clone())
    .unwrap_or_default()
}

pub(crate) fn sync_auth_state(state: &AuthState) {
  sentry::configure_scope(|scope| {
    scope.set_tag("auth.state", auth_state_tag(state));

    match state {
      AuthState::Authenticated(user) => {
        let sentry_user = User {
          id: Some(user.id.clone()),
          email: Some(user.email.clone()),
          username: user.github_login.clone().or_else(|| {
            if user.name.trim().is_empty() {
              None
            } else {
              Some(user.name.clone())
            }
          }),
          ..Default::default()
        };
        scope.set_user(Some(sentry_user));
        scope.set_tag(
          "auth.subscription_active",
          if user.subscription.active_subscription.is_some() {
            "true"
          } else {
            "false"
          },
        );
        scope.set_tag(
          "user.role",
          match user.role {
            crate::api::UserRole::User => "user",
            crate::api::UserRole::Pro => "pro",
            crate::api::UserRole::Admin => "admin",
          },
        );
      }
      AuthState::Unknown | AuthState::Unauthenticated => {
        scope.set_user(None);
        scope.remove_tag("auth.subscription_active");
        scope.remove_tag("user.role");
      }
    }
  });

  let mut data = Map::new();
  data.insert("state".into(), auth_state_tag(state).to_string().into());
  add_breadcrumb("auth.state", "Auth state changed", data);
}

pub(crate) fn sync_git_context(
  repo_root: Option<&Path>,
  selected_file: Option<&Path>,
  branch: Option<&str>,
  sidebar_mode: &str,
  diff_view: &str,
) {
  sentry::configure_scope(|scope| {
    scope.set_tag("git.sidebar_mode", sidebar_mode);
    scope.set_tag("git.diff_view", diff_view);

    let mut context = Map::new();
    context.insert("sidebar_mode".into(), sidebar_mode.to_string().into());
    context.insert("diff_view".into(), diff_view.to_string().into());

    if let Some(repo_root) = repo_root {
      let (repo_name, repo_hash) = sanitize_repo_path(repo_root);
      scope.set_tag("git.repo_name", repo_name.as_str());
      scope.set_tag("git.repo_hash", repo_hash.as_str());
      context.insert("repo_name".into(), repo_name.into());
      context.insert("repo_hash".into(), repo_hash.into());
    } else {
      scope.remove_tag("git.repo_name");
      scope.remove_tag("git.repo_hash");
    }

    if let Some(file) = selected_file {
      let file = file.to_string_lossy().replace(['\n', '\r'], "");
      context.insert("selected_file".into(), file.clone().into());
      scope.set_tag("git.selected_file", file);
    } else {
      scope.remove_tag("git.selected_file");
    }

    if let Some(branch) = branch {
      scope.set_tag("git.branch", branch);
      context.insert("branch".into(), branch.to_string().into());
    } else {
      scope.remove_tag("git.branch");
    }

    scope.set_context("git_state", to_unknown_context(context));
  });

  update_crash_snapshot(|snapshot| {
    let (repo_name, repo_hash) = repo_root
      .map(sanitize_repo_path)
      .map(|(name, hash)| (Some(name), Some(hash)))
      .unwrap_or((None, None));

    snapshot.git = Some(CrashGitContext {
      repo_name,
      repo_hash,
      selected_file: selected_file.map(|path| path.to_string_lossy().replace(['\n', '\r'], "")),
      branch: branch.map(str::to_string),
      sidebar_mode: sidebar_mode.to_string(),
      diff_view: diff_view.to_string(),
    });
  });
}

pub(crate) fn clear_git_context() {
  sentry::configure_scope(|scope| {
    scope.remove_tag("git.repo_name");
    scope.remove_tag("git.repo_hash");
    scope.remove_tag("git.selected_file");
    scope.remove_tag("git.branch");
    scope.remove_tag("git.sidebar_mode");
    scope.remove_tag("git.diff_view");
    scope.remove_context("git_state");
  });

  update_crash_snapshot(|snapshot| {
    snapshot.git = None;
  });
}

pub(crate) fn sync_github_pr_context(
  owner: &str,
  repo: &str,
  number: u64,
  selected_file: Option<&str>,
  active_tab: Option<usize>,
) {
  sentry::configure_scope(|scope| {
    scope.set_tag("github.owner", owner);
    scope.set_tag("github.repo", repo);
    scope.set_tag("github.pr_number", number.to_string());

    let mut context = Map::new();
    context.insert("owner".into(), owner.to_string().into());
    context.insert("repo".into(), repo.to_string().into());
    context.insert("number".into(), number.into());
    if let Some(file) = selected_file {
      context.insert("selected_file".into(), file.to_string().into());
      scope.set_tag("github.selected_file", file);
    } else {
      scope.remove_tag("github.selected_file");
    }
    if let Some(tab) = active_tab {
      context.insert("active_tab".into(), tab.into());
    }
    scope.set_context("github_pr", to_unknown_context(context));
  });

  update_crash_snapshot(|snapshot| {
    snapshot.github_pr = Some(CrashGithubPrContext {
      owner: owner.to_string(),
      repo: repo.to_string(),
      number,
      selected_file: selected_file.map(str::to_string),
      active_tab,
    });
  });
}

pub(crate) fn clear_github_pr_context() {
  sentry::configure_scope(|scope| {
    scope.remove_tag("github.owner");
    scope.remove_tag("github.repo");
    scope.remove_tag("github.pr_number");
    scope.remove_tag("github.selected_file");
    scope.remove_context("github_pr");
  });

  update_crash_snapshot(|snapshot| {
    snapshot.github_pr = None;
  });
}

#[cfg(test)]
mod tests {
  use super::{
    DEDUP_WINDOW, auth_state_tag, sanitize_repo_path, should_capture_error, workspace_page_tag,
  };
  use crate::workspace::WorkspacePage;
  use crate::{
    api::{User, UserRole, UserSubscription},
    auth_state::AuthState,
  };
  use std::{path::Path, time::Instant};

  #[test]
  fn sanitize_repo_path_returns_repo_name_and_short_hash() {
    let (repo_name, repo_hash) =
      sanitize_repo_path(Path::new("/Users/joris/workspace/reviu/desktop"));
    assert_eq!(repo_name, "desktop");
    assert_eq!(repo_hash.len(), 12);
    assert!(!repo_hash.contains('/'));
    assert_ne!(repo_hash, "desktop");
  }

  #[test]
  fn workspace_page_tag_maps_pages() {
    assert_eq!(workspace_page_tag(WorkspacePage::Git), "git");
    assert_eq!(workspace_page_tag(WorkspacePage::Github), "github");
    assert_eq!(workspace_page_tag(WorkspacePage::GithubRepo), "github_repo");
    assert_eq!(
      workspace_page_tag(WorkspacePage::GithubPrDetails),
      "github_pr_details"
    );
  }

  #[test]
  fn should_capture_error_deduplicates_within_window() {
    let now = Instant::now();
    assert!(should_capture_error("git.push|boom", now));
    assert!(!should_capture_error(
      "git.push|boom",
      now + DEDUP_WINDOW / 2
    ));
    assert!(should_capture_error(
      "git.push|boom",
      now + DEDUP_WINDOW * 2
    ));
  }

  #[test]
  fn auth_state_tag_maps_states() {
    let user = User {
      id: "user_123".to_string(),
      name: "Joris".to_string(),
      email: "joris@example.com".to_string(),
      email_verified: true,
      image: None,
      github_login: Some("joris".to_string()),
      role: UserRole::User,
      subscription: UserSubscription::default(),
    };

    assert_eq!(auth_state_tag(&AuthState::Unknown), "unknown");
    assert_eq!(
      auth_state_tag(&AuthState::Unauthenticated),
      "unauthenticated"
    );
    assert_eq!(
      auth_state_tag(&AuthState::Authenticated(Box::new(user))),
      "authenticated"
    );
  }
}
