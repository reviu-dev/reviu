use gpui::{App, Global, SharedString};
use gpui_router::use_navigate;

use crate::auth_state::AuthStateStore;

pub struct NavigationHistory {
  stack: Vec<SharedString>,
}

impl Global for NavigationHistory {}

impl NavigationHistory {
  pub fn init(cx: &mut App) {
    cx.set_global(Self { stack: Vec::new() });
  }

  /// Navigate to `path`, pushing the current location onto the history stack.
  /// GitHub paths (`/github/*`) are gated behind GitHub access — if the user
  /// doesn't have access, they are redirected to `/billing` instead.
  pub fn navigate(path: impl Into<SharedString>, cx: &mut App) {
    let path = path.into();

    let current = Self::current_pathname(cx);
    if current == path {
      return;
    }

    // GitHub access gating: redirect /github/* to /billing when no access
    if requires_github_access(&path) && !AuthStateStore::has_github_access(cx) {
      // Push current so closing billing returns here
      cx.global_mut::<Self>().stack.push(current);
      Self::set_pathname("/billing", cx);
      return;
    }

    cx.global_mut::<Self>().stack.push(current);
    Self::set_pathname(&path, cx);
  }

  /// Navigate back in history. Falls back to `/git` if the stack is empty.
  pub fn navigate_back(cx: &mut App) {
    let target = cx
      .global_mut::<Self>()
      .stack
      .pop()
      .unwrap_or_else(|| "/git".into());

    // If back target requires GitHub access and user lost access, fall back to /git
    let target = if requires_github_access(&target) && !AuthStateStore::has_github_access(cx) {
      "/git".into()
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
  path.starts_with("/github")
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::TestAppContext;

  #[gpui::test]
  async fn test_navigate_pushes_to_stack(cx: &mut TestAppContext) {
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);

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
      gpui_router::init(cx);
      NavigationHistory::init(cx);

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
      gpui_router::init(cx);
      NavigationHistory::init(cx);

      NavigationHistory::navigate_replace("/git", cx);
      NavigationHistory::navigate("/git", cx);

      // Stack should be empty — no duplicate push
      assert_eq!(cx.global::<NavigationHistory>().stack.len(), 0);
    });
  }

  #[gpui::test]
  async fn test_navigate_replace_does_not_push(cx: &mut TestAppContext) {
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);

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
}
