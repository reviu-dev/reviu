use gpui::{App, Global, SharedString};
use gpui_router::use_navigate;

use crate::auth_state::AuthStateStore;

const NAVIGATION_HISTORY_MAX_ENTRIES: usize = 100;

pub struct NavigationHistory {
  stack: Vec<SharedString>,
}

impl Global for NavigationHistory {}

impl NavigationHistory {
  pub fn init(cx: &mut App) {
    cx.set_global(Self { stack: Vec::new() });
  }

  fn push_entry(&mut self, entry: SharedString) {
    self.stack.push(entry);
    let len = self.stack.len();
    if len > NAVIGATION_HISTORY_MAX_ENTRIES {
      let excess = len - NAVIGATION_HISTORY_MAX_ENTRIES;
      self.stack.drain(0..excess);
    }
  }

  /// Navigate to `path`, pushing the current location onto the history stack.
  /// Deep GitHub paths are gated behind GitHub access. `/github` itself always
  /// stays accessible as the GitHub home / access page.
  pub fn navigate(path: impl Into<SharedString>, cx: &mut App) {
    let path = path.into();

    let current = Self::current_pathname(cx);
    if current == path {
      return;
    }

    // GitHub access gating: redirect protected GitHub routes to /github when no access.
    if requires_github_access(&path) && !AuthStateStore::has_github_access(cx) {
      if current != "/github" {
        cx.global_mut::<Self>().push_entry(current);
      }
      Self::set_pathname("/github", cx);
      return;
    }

    cx.global_mut::<Self>().push_entry(current);
    Self::set_pathname(&path, cx);
  }

  /// Navigate back in history. Falls back to `/git` if the stack is empty.
  pub fn navigate_back(cx: &mut App) {
    let target = cx
      .global_mut::<Self>()
      .stack
      .pop()
      .unwrap_or_else(|| "/git".into());

    // If back target requires GitHub access and user lost access, fall back to /github.
    let target = if requires_github_access(&target) && !AuthStateStore::has_github_access(cx) {
      "/github".into()
    } else {
      target
    };

    Self::set_pathname(&target, cx);
  }

  /// Navigate without pushing to the history stack (for redirects).
  pub fn navigate_replace(path: impl Into<SharedString>, cx: &mut App) {
    let path = path.into();
    Self::set_pathname(&path, cx);
  }

  /// Returns the current pathname from the router state.
  pub fn current_pathname(cx: &App) -> SharedString {
    gpui_router::use_location(cx).pathname.clone()
  }

  fn set_pathname(path: &str, cx: &mut App) {
    let path: SharedString = path.to_string().into();
    use_navigate(cx)(path);
    cx.refresh_windows();
  }
}

/// Build path: `/github/{owner}/{repo}/pull/{number}`
pub fn build_pr_path(owner: &str, repo: &str, number: u64) -> SharedString {
  format!("/github/{owner}/{repo}/pull/{number}").into()
}

/// Build path with tab suffix: `/github/{owner}/{repo}/pull/{number}/{tab}`
pub fn build_pr_tab_path(owner: &str, repo: &str, number: u64, tab: &str) -> SharedString {
  format!("/github/{owner}/{repo}/pull/{number}/{tab}").into()
}

/// Build path: `/github/{owner}/{repo}`
pub fn build_repo_path(owner: &str, repo: &str) -> SharedString {
  format!("/github/{owner}/{repo}").into()
}

/// Build path: `/github/{login}`
pub fn build_github_profile_path(login: &str) -> SharedString {
  format!("/github/{login}").into()
}

/// Build path: `/github/{owner}/{repo}/commit/{sha}`
pub fn build_commit_path(owner: &str, repo: &str, sha: &str) -> SharedString {
  format!("/github/{owner}/{repo}/commit/{sha}").into()
}

/// Build path with tab suffix: `/github/{owner}/{repo}/{tab}`
pub fn build_repo_tab_path(owner: &str, repo: &str, tab: &str) -> SharedString {
  format!("/github/{owner}/{repo}/{tab}").into()
}

/// Returns the last segment of the current pathname (the tab), or empty string.
pub fn current_tab_segment(cx: &gpui::App) -> &str {
  let pathname = &gpui_router::use_location(cx).pathname;
  pathname.rsplit('/').next().unwrap_or("")
}

fn requires_github_access(path: &str) -> bool {
  path.starts_with("/github/")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    api::{
      CustomerStateSubscription, CustomerStateSubscriptionStatus, User, UserRole, UserSubscription,
    },
    auth_state::{AuthState, AuthStateStore},
  };
  use gpui::TestAppContext;

  fn make_subscription() -> CustomerStateSubscription {
    CustomerStateSubscription {
      id: "sub_123".to_string(),
      created_at: "2026-01-01T00:00:00Z".to_string(),
      modified_at: None,
      status: CustomerStateSubscriptionStatus::Active,
      amount: 2_000,
      currency: "usd".to_string(),
      recurring_interval: "month".to_string(),
      current_period_start: "2026-01-01T00:00:00Z".to_string(),
      current_period_end: Some("2099-01-01T00:00:00Z".to_string()),
      trial_start: None,
      trial_end: None,
      cancel_at_period_end: false,
      canceled_at: None,
      started_at: Some("2026-01-01T00:00:00Z".to_string()),
      ends_at: None,
      product_id: "prod_123".to_string(),
    }
  }

  fn make_user(role: UserRole, subscribed: bool) -> User {
    User {
      id: "user_123".to_string(),
      name: "Joris".to_string(),
      email: "joris@example.com".to_string(),
      email_verified: true,
      image: None,
      github_login: Some("joris-gallot".to_string()),
      role,
      subscription: UserSubscription {
        portal_url: None,
        active_subscription: subscribed.then(make_subscription),
      },
    }
  }

  fn init_navigation_test(cx: &mut App) {
    gpui_router::init(cx);
    NavigationHistory::init(cx);
    cx.set_global(AuthStateStore::default());
  }

  #[gpui::test]
  async fn test_navigate_pushes_to_stack(cx: &mut TestAppContext) {
    cx.update(|cx| {
      init_navigation_test(cx);

      // Start at /git
      NavigationHistory::navigate_replace("/git", cx);
      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/git");

      // Navigate to /settings
      NavigationHistory::navigate("/settings", cx);
      assert_eq!(
        NavigationHistory::current_pathname(cx).as_ref(),
        "/settings"
      );

      // History should have /git
      assert_eq!(cx.global::<NavigationHistory>().stack.len(), 1);
      assert_eq!(cx.global::<NavigationHistory>().stack[0].as_ref(), "/git");
    });
  }

  #[gpui::test]
  async fn test_navigate_back(cx: &mut TestAppContext) {
    cx.update(|cx| {
      init_navigation_test(cx);

      NavigationHistory::navigate_replace("/git", cx);
      NavigationHistory::navigate("/settings", cx);
      NavigationHistory::navigate("/about", cx);

      // Back from /about -> /settings
      NavigationHistory::navigate_back(cx);
      assert_eq!(
        NavigationHistory::current_pathname(cx).as_ref(),
        "/settings"
      );

      // Back from /settings -> /git
      NavigationHistory::navigate_back(cx);
      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/git");

      // Back from /git -> /git (fallback)
      NavigationHistory::navigate_back(cx);
      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/git");
    });
  }

  #[gpui::test]
  async fn test_navigate_same_path_is_noop(cx: &mut TestAppContext) {
    cx.update(|cx| {
      init_navigation_test(cx);

      NavigationHistory::navigate_replace("/git", cx);
      NavigationHistory::navigate("/git", cx);

      // Stack should be empty — no duplicate push
      assert_eq!(cx.global::<NavigationHistory>().stack.len(), 0);
    });
  }

  #[gpui::test]
  async fn test_navigate_replace_does_not_push(cx: &mut TestAppContext) {
    cx.update(|cx| {
      init_navigation_test(cx);

      NavigationHistory::navigate_replace("/git", cx);
      NavigationHistory::navigate_replace("/settings", cx);

      // Stack should be empty — replace doesn't push
      assert_eq!(cx.global::<NavigationHistory>().stack.len(), 0);
      assert_eq!(
        NavigationHistory::current_pathname(cx).as_ref(),
        "/settings"
      );
    });
  }

  #[gpui::test]
  async fn test_navigate_allows_github_home_without_access(cx: &mut TestAppContext) {
    cx.update(|cx| {
      init_navigation_test(cx);
      AuthStateStore::set(cx, AuthState::Unauthenticated);

      NavigationHistory::navigate_replace("/git", cx);
      NavigationHistory::navigate("/github", cx);

      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/github");
    });
  }

  #[gpui::test]
  async fn test_navigate_redirects_protected_github_route_to_home_without_access(
    cx: &mut TestAppContext,
  ) {
    cx.update(|cx| {
      init_navigation_test(cx);
      AuthStateStore::set(cx, AuthState::Unauthenticated);

      NavigationHistory::navigate_replace("/git", cx);
      NavigationHistory::navigate("/github/owner/repo", cx);

      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/github");
      assert_eq!(cx.global::<NavigationHistory>().stack.len(), 1);
      assert_eq!(cx.global::<NavigationHistory>().stack[0].as_ref(), "/git");
    });
  }

  #[gpui::test]
  async fn test_navigate_caps_history_at_max_entries(cx: &mut TestAppContext) {
    cx.update(|cx| {
      init_navigation_test(cx);

      NavigationHistory::navigate_replace("/git", cx);
      for i in 0..(NAVIGATION_HISTORY_MAX_ENTRIES + 5) {
        NavigationHistory::navigate(format!("/page-{i}"), cx);
      }

      assert_eq!(
        cx.global::<NavigationHistory>().stack.len(),
        NAVIGATION_HISTORY_MAX_ENTRIES
      );
      // Navigated 105 times; kept the last 100 pushed entries (/page-4 .. /page-103).
      assert_eq!(
        cx.global::<NavigationHistory>()
          .stack
          .first()
          .unwrap()
          .as_ref(),
        "/page-4"
      );
      assert_eq!(
        cx.global::<NavigationHistory>()
          .stack
          .last()
          .unwrap()
          .as_ref(),
        "/page-103"
      );
    });
  }

  #[gpui::test]
  async fn test_navigate_back_falls_back_to_github_home_when_access_is_lost(
    cx: &mut TestAppContext,
  ) {
    cx.update(|cx| {
      init_navigation_test(cx);
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_user(UserRole::User, true))),
      );

      NavigationHistory::navigate_replace("/settings", cx);
      cx.global_mut::<NavigationHistory>()
        .stack
        .push("/github/owner/repo".into());
      AuthStateStore::set(cx, AuthState::Unauthenticated);
      NavigationHistory::navigate_back(cx);

      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/github");
    });
  }
}
