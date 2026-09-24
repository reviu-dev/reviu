use std::{
  collections::HashMap,
  error::Error,
  sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
  },
  time::{Duration, Instant},
};

use crate::auth_state::AuthState;
use sentry::protocol::{Breadcrumb, Context, Level, Map, Value};
use serde::{Deserialize, Serialize};

const DEDUP_WINDOW: Duration = Duration::from_secs(300);

static CLOSING: AtomicBool = AtomicBool::new(false);

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

/// Which surfaces were open, never what they held: no path, repository or text.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CrashWorkspaceContext {
  // Reports written before 1.3 named the dock tab after the old Git page sidebar.
  #[serde(alias = "sidebarMode")]
  pub dock_tab: String,
  pub diff_view: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub center: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<String>,
  #[serde(default)]
  pub agent_turn_running: bool,
  #[serde(default)]
  pub in_worktree: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub window_count: Option<usize>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CrashContextSnapshot {
  pub workspace: Option<CrashWorkspaceContext>,
  pub closing: bool,
}

fn auth_state_tag(state: &AuthState) -> &'static str {
  match state {
    AuthState::Unknown => "unknown",
    AuthState::Authenticated(_) => "authenticated",
    AuthState::Unauthenticated => "unauthenticated",
  }
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

fn expected_http_reason(status: u16) -> Option<&'static str> {
  match status {
    401 => Some("unauthorized"),
    _ => None,
  }
}

pub(crate) fn record_http_status(method: &str, route: &str, status: u16) {
  let mut data = Map::new();
  data.insert("method".into(), method.to_string().into());
  data.insert("route".into(), route.to_string().into());
  data.insert("status".into(), status.into());

  if let Some(reason) = expected_http_reason(status) {
    record_expected_error("api.http", reason, data);
    return;
  }

  if status >= 400 {
    let error = std::io::Error::other(format!("unexpected HTTP status {status}"));
    capture_unexpected_error("api.http", &error, data);
  }
}

pub(crate) fn current_crash_context_snapshot() -> CrashContextSnapshot {
  let mut snapshot = crash_snapshot_state()
    .lock()
    .map(|snapshot| snapshot.clone())
    .unwrap_or_default();
  snapshot.closing = CLOSING.load(Ordering::SeqCst);
  snapshot
}

/// A window close or a quit went ahead: a crash from here on happened on the way out.
pub(crate) fn mark_closing() {
  CLOSING.store(true, Ordering::SeqCst);
  add_breadcrumb("app.lifecycle", "Closing", Map::new());
}

pub(crate) fn sync_auth_state(state: &AuthState) {
  sentry::configure_scope(|scope| {
    scope.set_tag("auth.state", auth_state_tag(state));

    match state {
      AuthState::Authenticated(user) => {
        scope.set_user(None);
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

pub(crate) fn sync_workspace_context(context: &CrashWorkspaceContext) {
  // Called on every render of the session page, so only a change reaches Sentry's scope.
  let unchanged = crash_snapshot_state()
    .lock()
    .is_ok_and(|snapshot| snapshot.workspace.as_ref() == Some(context));
  if unchanged {
    return;
  }

  sentry::configure_scope(|scope| {
    scope.set_tag("workspace.dock_tab", &context.dock_tab);
    scope.set_tag("workspace.diff_view", &context.diff_view);
    match context.center.as_deref() {
      Some(center) => scope.set_tag("workspace.center", center),
      None => scope.remove_tag("workspace.center"),
    }
    match context.agent.as_deref() {
      Some(agent) => scope.set_tag("workspace.agent", agent),
      None => scope.remove_tag("workspace.agent"),
    }

    if let Ok(Value::Object(map)) = serde_json::to_value(context) {
      scope.set_context(
        "workspace_state",
        to_unknown_context(map.into_iter().collect()),
      );
    }
  });

  update_crash_snapshot(|snapshot| {
    snapshot.workspace = Some(context.clone());
  });
}

pub(crate) fn clear_workspace_context() {
  sentry::configure_scope(|scope| {
    scope.remove_tag("workspace.dock_tab");
    scope.remove_tag("workspace.diff_view");
    scope.remove_tag("workspace.center");
    scope.remove_tag("workspace.agent");
    scope.remove_context("workspace_state");
  });

  update_crash_snapshot(|snapshot| {
    snapshot.workspace = None;
  });
}

#[cfg(test)]
mod tests {
  use super::{DEDUP_WINDOW, auth_state_tag, expected_http_reason, should_capture_error};
  use crate::{
    api::{User, UserRole, UserSubscription},
    auth_state::AuthState,
  };
  use std::time::Instant;

  #[test]
  fn expected_http_reason_flags_unauthorized_only() {
    assert_eq!(expected_http_reason(401), Some("unauthorized"));
    assert_eq!(expected_http_reason(403), None);
    assert_eq!(expected_http_reason(500), None);
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
