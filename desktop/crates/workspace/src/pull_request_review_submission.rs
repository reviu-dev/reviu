//! Submitting a review on a pull request: the decision, and what it demands
//! before it can go out.

use gpui::SharedString;

use crate::api::GithubPullRequestReviewEvent;

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
