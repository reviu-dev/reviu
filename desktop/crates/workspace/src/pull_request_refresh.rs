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
}
