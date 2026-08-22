//! Submitting a review on a pull request: the decision, and what it demands
//! before it can go out.

use gpui::SharedString;

use crate::api::{GithubPullRequestReviewComment, GithubPullRequestReviewEvent};
use crate::github_shared::logins_match_case_insensitive;
use crate::pull_request_review_comments::pending_review_id;
use crate::review_submit_dialog::ReviewSubmissionTarget;

/// What the dialog needs to know, all of it read off the same list of comments
/// so the review it submits is the one it counted.
pub(crate) fn review_submission_target(
  owner: String,
  repo: String,
  number: u64,
  comments: &[GithubPullRequestReviewComment],
  viewer_login: Option<&str>,
  author_login: Option<&str>,
) -> ReviewSubmissionTarget {
  let viewer_is_author = match (viewer_login, author_login) {
    (Some(viewer), Some(author)) => logins_match_case_insensitive(viewer, author),
    _ => false,
  };
  ReviewSubmissionTarget {
    owner,
    repo,
    number,
    pending_review_id: pending_review_id(comments),
    pending_comment_count: comments.iter().filter(|comment| comment.is_pending).count(),
    viewer_is_author,
  }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ReviewDecision {
  #[default]
  Comment,
  Approve,
  RequestChanges,
}

impl ReviewDecision {
  /// The order the choices are offered in, and the only place that order lives.
  pub(crate) const ALL: [Self; 3] = [Self::Comment, Self::Approve, Self::RequestChanges];

  pub(crate) fn label(self) -> &'static str {
    match self {
      Self::Comment => "Comment",
      Self::Approve => "Approve",
      Self::RequestChanges => "Request changes",
    }
  }

  pub(crate) fn api_event(self) -> GithubPullRequestReviewEvent {
    match self {
      Self::Comment => GithubPullRequestReviewEvent::Comment,
      Self::Approve => GithubPullRequestReviewEvent::Approve,
      Self::RequestChanges => GithubPullRequestReviewEvent::RequestChanges,
    }
  }

  /// GitHub takes an approval without a word. The other two are the word.
  fn requires_body(self) -> bool {
    match self {
      Self::Comment | Self::RequestChanges => true,
      Self::Approve => false,
    }
  }

  /// An author may say things about their own pull request, not judge it.
  pub(crate) fn allowed_for_author(self) -> bool {
    match self {
      Self::Comment => true,
      Self::Approve | Self::RequestChanges => false,
    }
  }
}

pub(crate) fn decision_index(decision: ReviewDecision) -> usize {
  ReviewDecision::ALL
    .iter()
    .position(|candidate| *candidate == decision)
    .unwrap_or(0)
}

pub(crate) fn decision_from_index(index: usize) -> ReviewDecision {
  ReviewDecision::ALL
    .get(index)
    .copied()
    .unwrap_or(ReviewDecision::Comment)
}

/// Why this review cannot go out, or nothing if it can.
pub(crate) fn validate_review_submission(
  decision: ReviewDecision,
  body: &str,
  viewer_is_author: bool,
) -> Option<SharedString> {
  if viewer_is_author && !decision.allowed_for_author() {
    return Some(
      "Pull request authors cannot approve or request changes on their own pull requests.".into(),
    );
  }
  if decision.requires_body() && body.trim().is_empty() {
    return Some("A review comment is required for this review type".into());
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::pull_request_review_comments::pending_comment_fixture;

  #[test]
  fn the_review_submitted_is_the_one_that_was_counted() {
    let mut published = pending_comment_fixture(3, "src/c.rs", Some(1), "already out");
    published.is_pending = false;
    let comments = [
      pending_comment_fixture(1, "src/a.rs", Some(9), "first"),
      pending_comment_fixture(2, "src/b.rs", Some(4), "second"),
      published,
    ];

    let target = review_submission_target(
      "acme".to_string(),
      "widget".to_string(),
      42,
      &comments,
      Some("octocat"),
      Some("ada"),
    );

    // Two pending comments, and the review that holds them.
    assert_eq!(target.pending_comment_count, 2);
    assert_eq!(target.pending_review_id.as_deref(), Some("review-node"));
    assert!(!target.viewer_is_author);
  }

  #[test]
  fn nothing_pending_submits_the_decision_on_its_own() {
    let mut published = pending_comment_fixture(1, "src/a.rs", Some(9), "already out");
    published.is_pending = false;

    let target = review_submission_target(
      "acme".to_string(),
      "widget".to_string(),
      42,
      &[published],
      None,
      None,
    );

    assert_eq!(target.pending_comment_count, 0);
    // No review to submit: the decision goes out by itself.
    assert_eq!(target.pending_review_id, None);
  }

  #[test]
  fn the_author_is_recognised_whatever_the_case() {
    let target = |viewer: Option<&str>, author: Option<&str>| {
      review_submission_target(
        "acme".to_string(),
        "widget".to_string(),
        42,
        &[],
        viewer,
        author,
      )
      .viewer_is_author
    };

    assert!(target(Some("OctoCat"), Some("octocat")));
    assert!(!target(Some("octocat"), Some("ada")));
    // Signed out, or an author we never read: judge nothing.
    assert!(!target(None, Some("octocat")));
    assert!(!target(Some("octocat"), None));
  }

  #[test]
  fn every_decision_maps_to_its_api_event() {
    assert_eq!(
      ReviewDecision::ALL.map(ReviewDecision::api_event),
      [
        GithubPullRequestReviewEvent::Comment,
        GithubPullRequestReviewEvent::Approve,
        GithubPullRequestReviewEvent::RequestChanges,
      ]
    );
  }

  #[test]
  fn the_offered_order_survives_a_round_trip() {
    for decision in ReviewDecision::ALL {
      assert_eq!(decision_from_index(decision_index(decision)), decision);
    }
    // An index from nowhere lands on the harmless choice.
    assert_eq!(decision_from_index(99), ReviewDecision::Comment);
  }

  #[test]
  fn saying_something_is_the_whole_point_of_the_two_talking_decisions() {
    assert!(validate_review_submission(ReviewDecision::Comment, "   ", false).is_some());
    assert!(validate_review_submission(ReviewDecision::RequestChanges, "\n", false).is_some());
    assert!(validate_review_submission(ReviewDecision::Comment, "please rename", false).is_none());
  }

  #[test]
  fn an_approval_needs_no_words() {
    assert!(validate_review_submission(ReviewDecision::Approve, "", false).is_none());
  }

  #[test]
  fn an_author_can_comment_on_their_own_work_and_nothing_more() {
    assert!(validate_review_submission(ReviewDecision::Comment, "a note", true).is_none());
    assert!(validate_review_submission(ReviewDecision::Approve, "", true).is_some());
    assert!(validate_review_submission(ReviewDecision::RequestChanges, "fix it", true).is_some());
  }
}
