//! Whether a pull request can be merged from here, and how. The decision is a
//! rule over what GitHub reports, so the panel only has to draw it.

use crate::api::{
  GithubMergeCommitDefaults, GithubPullRequestMergeMethod, GithubPullRequestMergeReadiness,
  GithubPullRequestMergeReadinessStatus,
};

pub(crate) fn merge_method_label(method: GithubPullRequestMergeMethod) -> &'static str {
  match method {
    GithubPullRequestMergeMethod::Merge => "Create a merge commit",
    GithubPullRequestMergeMethod::Squash => "Squash and merge",
    GithubPullRequestMergeMethod::Rebase => "Rebase and merge",
  }
}

pub(crate) fn merge_method_confirm_label(method: GithubPullRequestMergeMethod) -> &'static str {
  match method {
    GithubPullRequestMergeMethod::Merge => "Confirm merge",
    GithubPullRequestMergeMethod::Squash => "Confirm squash and merge",
    GithubPullRequestMergeMethod::Rebase => "Confirm rebase and merge",
  }
}

/// Rebase replays the commits as they are: GitHub takes no title or message.
pub(crate) fn merge_method_supports_commit_message(method: GithubPullRequestMergeMethod) -> bool {
  match method {
    GithubPullRequestMergeMethod::Merge | GithubPullRequestMergeMethod::Squash => true,
    GithubPullRequestMergeMethod::Rebase => false,
  }
}

/// What GitHub would generate for this method's commit, when the backend knew.
pub(crate) fn merge_commit_defaults(
  readiness: Option<&GithubPullRequestMergeReadiness>,
  method: GithubPullRequestMergeMethod,
) -> Option<GithubMergeCommitDefaults> {
  let defaults = readiness?.commit_defaults.as_ref()?;
  match method {
    GithubPullRequestMergeMethod::Merge => defaults.merge.clone(),
    GithubPullRequestMergeMethod::Squash => defaults.squash.clone(),
    GithubPullRequestMergeMethod::Rebase => None,
  }
}

/// Everything the merge call needs, settled before the confirmation asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MergeRequest {
  pub owner: String,
  pub repo: String,
  pub number: u64,
  pub method: GithubPullRequestMergeMethod,
  pub head_sha: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MergeAvailability {
  /// GitHub has not answered yet, or is still deciding. The button waits rather
  /// than claiming the merge is impossible.
  Unknown,
  Blocked(String),
  Ready {
    method: GithubPullRequestMergeMethod,
    head_sha: String,
  },
}

/// The one place that decides what the merge button offers. `preferred` is the
/// viewer's remembered choice; it only wins while the repository still allows it.
pub(crate) fn merge_availability(
  readiness: Option<&GithubPullRequestMergeReadiness>,
  preferred: Option<GithubPullRequestMergeMethod>,
) -> MergeAvailability {
  let Some(readiness) = readiness else {
    return MergeAvailability::Unknown;
  };

  match readiness.status {
    GithubPullRequestMergeReadinessStatus::Checking => return MergeAvailability::Unknown,
    GithubPullRequestMergeReadinessStatus::Merged => {
      return MergeAvailability::Blocked("Already merged".to_string());
    }
    GithubPullRequestMergeReadinessStatus::Closed => {
      return MergeAvailability::Blocked("This pull request is closed".to_string());
    }
    GithubPullRequestMergeReadinessStatus::Draft => {
      return MergeAvailability::Blocked("Still a draft".to_string());
    }
    GithubPullRequestMergeReadinessStatus::Forbidden => {
      return MergeAvailability::Blocked(blocked_message(
        readiness,
        "You cannot merge this pull request",
      ));
    }
    GithubPullRequestMergeReadinessStatus::Blocked => {
      return MergeAvailability::Blocked(blocked_message(readiness, "Not ready to merge"));
    }
    GithubPullRequestMergeReadinessStatus::Ready => {}
  }

  if !readiness.viewer_can_merge {
    return MergeAvailability::Blocked(blocked_message(
      readiness,
      "You cannot merge this pull request",
    ));
  }
  if !readiness.can_merge_now {
    return MergeAvailability::Blocked(blocked_message(readiness, "Not ready to merge"));
  }

  // A repository can forbid every method; a button that names none would lie.
  let Some(method) = preferred
    .filter(|method| readiness.available_methods.contains(method))
    .or(readiness.default_method)
    .or_else(|| readiness.available_methods.first().copied())
  else {
    return MergeAvailability::Blocked("No merge method is allowed here".to_string());
  };

  MergeAvailability::Ready {
    method,
    head_sha: readiness.current_head_sha.clone(),
  }
}

fn blocked_message(readiness: &GithubPullRequestMergeReadiness, fallback: &str) -> String {
  let message = readiness.message.trim();
  if message.is_empty() {
    fallback.to_string()
  } else {
    message.to_string()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn readiness(status: GithubPullRequestMergeReadinessStatus) -> GithubPullRequestMergeReadiness {
    GithubPullRequestMergeReadiness {
      status,
      message: String::new(),
      current_head_sha: "head123".to_string(),
      available_methods: vec![
        GithubPullRequestMergeMethod::Merge,
        GithubPullRequestMergeMethod::Squash,
      ],
      default_method: Some(GithubPullRequestMergeMethod::Squash),
      can_merge_now: true,
      viewer_can_merge: true,
      mergeable_state: None,
      rebaseable: None,
      commit_defaults: None,
    }
  }

  #[test]
  fn nothing_known_yet_waits_instead_of_refusing() {
    assert_eq!(merge_availability(None, None), MergeAvailability::Unknown);
    assert_eq!(
      merge_availability(
        Some(&readiness(GithubPullRequestMergeReadinessStatus::Checking)),
        None
      ),
      MergeAvailability::Unknown
    );
  }

  #[test]
  fn a_ready_pull_request_offers_the_default_method() {
    assert_eq!(
      merge_availability(
        Some(&readiness(GithubPullRequestMergeReadinessStatus::Ready)),
        None
      ),
      MergeAvailability::Ready {
        method: GithubPullRequestMergeMethod::Squash,
        head_sha: "head123".to_string(),
      }
    );
  }

  #[test]
  fn without_a_default_method_the_first_allowed_one_is_used() {
    let mut readiness = readiness(GithubPullRequestMergeReadinessStatus::Ready);
    readiness.default_method = None;

    assert_eq!(
      merge_availability(Some(&readiness), None),
      MergeAvailability::Ready {
        method: GithubPullRequestMergeMethod::Merge,
        head_sha: "head123".to_string(),
      }
    );
  }

  #[test]
  fn a_repository_that_allows_no_method_blocks_the_merge() {
    let mut readiness = readiness(GithubPullRequestMergeReadinessStatus::Ready);
    readiness.default_method = None;
    readiness.available_methods = Vec::new();

    assert_eq!(
      merge_availability(Some(&readiness), None),
      MergeAvailability::Blocked("No merge method is allowed here".to_string())
    );
  }

  #[test]
  fn a_blocked_pull_request_says_why_when_github_bothered_to_explain() {
    let mut blocked = readiness(GithubPullRequestMergeReadinessStatus::Blocked);
    blocked.message = "Review required".to_string();
    assert_eq!(
      merge_availability(Some(&blocked), None),
      MergeAvailability::Blocked("Review required".to_string())
    );

    let mut silent = readiness(GithubPullRequestMergeReadinessStatus::Blocked);
    silent.message = "   ".to_string();
    assert_eq!(
      merge_availability(Some(&silent), None),
      MergeAvailability::Blocked("Not ready to merge".to_string())
    );
  }

  #[test]
  fn ready_is_not_enough_without_the_right_and_the_moment() {
    let mut cannot = readiness(GithubPullRequestMergeReadinessStatus::Ready);
    cannot.viewer_can_merge = false;
    assert_eq!(
      merge_availability(Some(&cannot), None),
      MergeAvailability::Blocked("You cannot merge this pull request".to_string())
    );

    let mut not_now = readiness(GithubPullRequestMergeReadinessStatus::Ready);
    not_now.can_merge_now = false;
    assert_eq!(
      merge_availability(Some(&not_now), None),
      MergeAvailability::Blocked("Not ready to merge".to_string())
    );
  }

  #[test]
  fn a_merged_or_closed_pull_request_says_so() {
    assert_eq!(
      merge_availability(
        Some(&readiness(GithubPullRequestMergeReadinessStatus::Merged)),
        None
      ),
      MergeAvailability::Blocked("Already merged".to_string())
    );
    assert_eq!(
      merge_availability(
        Some(&readiness(GithubPullRequestMergeReadinessStatus::Draft)),
        None
      ),
      MergeAvailability::Blocked("Still a draft".to_string())
    );
  }

  #[test]
  fn the_remembered_method_wins_while_the_repository_still_allows_it() {
    let ready = readiness(GithubPullRequestMergeReadinessStatus::Ready);

    assert_eq!(
      merge_availability(Some(&ready), Some(GithubPullRequestMergeMethod::Merge)),
      MergeAvailability::Ready {
        method: GithubPullRequestMergeMethod::Merge,
        head_sha: "head123".to_string(),
      }
    );

    // Rebase left the allowed list: the choice falls back to the default.
    assert_eq!(
      merge_availability(Some(&ready), Some(GithubPullRequestMergeMethod::Rebase)),
      MergeAvailability::Ready {
        method: GithubPullRequestMergeMethod::Squash,
        head_sha: "head123".to_string(),
      }
    );
  }

  #[test]
  fn rebase_takes_no_title_and_no_message() {
    assert!(merge_method_supports_commit_message(
      GithubPullRequestMergeMethod::Merge
    ));
    assert!(merge_method_supports_commit_message(
      GithubPullRequestMergeMethod::Squash
    ));
    assert!(!merge_method_supports_commit_message(
      GithubPullRequestMergeMethod::Rebase
    ));
  }

  #[test]
  fn each_method_prefills_from_its_own_defaults() {
    use crate::api::{GithubMergeCommitDefaults, GithubPullRequestMergeCommitDefaults};

    let mut ready = readiness(GithubPullRequestMergeReadinessStatus::Ready);
    ready.commit_defaults = Some(GithubPullRequestMergeCommitDefaults {
      merge: Some(GithubMergeCommitDefaults {
        title: "Merge pull request #42 from acme/branch".to_string(),
        message: "The title".to_string(),
      }),
      squash: Some(GithubMergeCommitDefaults {
        title: "The title (#42)".to_string(),
        message: "* first\n\n* second".to_string(),
      }),
    });

    assert_eq!(
      merge_commit_defaults(Some(&ready), GithubPullRequestMergeMethod::Merge)
        .map(|defaults| defaults.title),
      Some("Merge pull request #42 from acme/branch".to_string())
    );
    assert_eq!(
      merge_commit_defaults(Some(&ready), GithubPullRequestMergeMethod::Squash)
        .map(|defaults| defaults.message),
      Some("* first\n\n* second".to_string())
    );
    assert_eq!(
      merge_commit_defaults(Some(&ready), GithubPullRequestMergeMethod::Rebase),
      None
    );
    assert_eq!(
      merge_commit_defaults(None, GithubPullRequestMergeMethod::Merge),
      None
    );
  }
}
