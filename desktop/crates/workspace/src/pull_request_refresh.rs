//! When the panel may reuse the pull request it already read from GitHub.

use std::time::{Duration, Instant};

/// How long a read stays good for. Opening the tab again inside that window
/// shows what is already there instead of spending a round trip on it.
const REFETCH_AFTER: Duration = Duration::from_secs(60);

/// Why the pull request is being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PullRequestRefresh {
  /// Incidental: opening the tab, saving a file, committing.
  IfStale,
  /// The user asked, or the pull request itself moved under us.
  Now,
}

pub(crate) fn should_read_pull_request(
  refresh: PullRequestRefresh,
  fetched_at: Option<Instant>,
  now: Instant,
) -> bool {
  match refresh {
    PullRequestRefresh::Now => true,
    PullRequestRefresh::IfStale => {
      fetched_at.is_none_or(|fetched_at| now.saturating_duration_since(fetched_at) >= REFETCH_AFTER)
    }
  }
}

/// Whether a checkout happened under the panel since the last lookup. The
/// staleness window answers "how old", never "of what": the pull request of the
/// branch you left is wrong immediately, not a minute later.
pub(crate) fn branch_switched_since_lookup(
  fetched_at: Option<Instant>,
  looked_up_branch: Option<&str>,
  current_branch: Option<&str>,
) -> bool {
  fetched_at.is_some() && looked_up_branch != current_branch
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_recent_read_is_reused_and_an_old_one_is_not() {
    let now = Instant::now();
    assert!(!should_read_pull_request(
      PullRequestRefresh::IfStale,
      Some(now),
      now
    ));
    assert!(!should_read_pull_request(
      PullRequestRefresh::IfStale,
      Some(now),
      now + REFETCH_AFTER - Duration::from_secs(1)
    ));
    assert!(should_read_pull_request(
      PullRequestRefresh::IfStale,
      Some(now),
      now + REFETCH_AFTER
    ));
  }

  #[test]
  fn nothing_read_yet_is_always_read() {
    assert!(should_read_pull_request(
      PullRequestRefresh::IfStale,
      None,
      Instant::now()
    ));
  }

  #[test]
  fn asking_ignores_how_recent_the_last_read_was() {
    let now = Instant::now();
    assert!(should_read_pull_request(
      PullRequestRefresh::Now,
      Some(now),
      now
    ));
  }

  #[test]
  fn the_same_branch_is_not_a_switch() {
    assert!(!branch_switched_since_lookup(
      Some(Instant::now()),
      Some("main"),
      Some("main")
    ));
  }

  #[test]
  fn another_branch_is_a_switch() {
    assert!(branch_switched_since_lookup(
      Some(Instant::now()),
      Some("main"),
      Some("feature")
    ));
  }

  #[test]
  fn leaving_every_branch_behind_is_a_switch() {
    assert!(branch_switched_since_lookup(
      Some(Instant::now()),
      Some("main"),
      None
    ));
  }

  #[test]
  fn nothing_read_yet_is_left_to_the_first_lookup() {
    assert!(!branch_switched_since_lookup(None, None, Some("feature")));
    assert!(!branch_switched_since_lookup(
      None,
      Some("main"),
      Some("feature")
    ));
  }
}
