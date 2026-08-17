//! When the shell re-reads the repository on its own, without an event to react to.

use std::path::Path;
use std::time::Duration;

/// Edits made outside Reviu should show up quickly while the user watches.
pub(crate) const ACTIVE_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// A window in the background is not worth a `git status` every three seconds.
pub(crate) const INACTIVE_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) fn poll_interval(window_active: bool) -> Duration {
  if window_active {
    ACTIVE_STATUS_POLL_INTERVAL
  } else {
    INACTIVE_STATUS_POLL_INTERVAL
  }
}

/// A poll never runs behind the user's back: not while a git command of its own
/// is running, and not on a window nobody is looking at.
pub(crate) fn should_poll(
  window_active: bool,
  selected_repo: Option<&Path>,
  command_in_flight: bool,
) -> bool {
  window_active && selected_repo.is_some() && !command_in_flight
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_background_window_polls_far_less_often() {
    assert_eq!(poll_interval(true), Duration::from_secs(3));
    assert_eq!(poll_interval(false), Duration::from_secs(60));
    assert!(poll_interval(false) > poll_interval(true));
  }

  #[test]
  fn polling_waits_for_an_active_window_a_repository_and_an_idle_repo() {
    let repo = Path::new("/tmp/repo");

    assert!(should_poll(true, Some(repo), false));
    assert!(!should_poll(false, Some(repo), false), "inactive window");
    assert!(!should_poll(true, None, false), "no repository");
    assert!(
      !should_poll(true, Some(repo), true),
      "a git command is already running"
    );
  }
}
