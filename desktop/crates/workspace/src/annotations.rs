//! Walking a diff: conflicts when the file has them, changes otherwise.

use editor::{ConflictNavigationDirection, Editor, HunkNavigationDirection};
use git::RepoStatusKind;
use gpui::{App, Context};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnnotationKind {
  Conflict,
  Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AnnotationNavigationState {
  pub(crate) active_index: usize,
  pub(crate) total: usize,
  pub(crate) kind: AnnotationKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnnotationDirection {
  Previous,
  Next,
}

impl AnnotationDirection {
  pub(crate) fn conflict(self) -> ConflictNavigationDirection {
    match self {
      Self::Previous => ConflictNavigationDirection::Previous,
      Self::Next => ConflictNavigationDirection::Next,
    }
  }

  pub(crate) fn hunk(self) -> HunkNavigationDirection {
    match self {
      Self::Previous => HunkNavigationDirection::Previous,
      Self::Next => HunkNavigationDirection::Next,
    }
  }
}

pub(crate) fn conflict_navigation_state_for(
  file_status: Option<RepoStatusKind>,
  editor: &Editor,
  cx: &App,
) -> Option<editor::ConflictNavigationState> {
  matches!(file_status, Some(RepoStatusKind::Conflicted))
    .then(|| editor.conflict_navigation_state(cx))
    .flatten()
}

/// A conflicted file is walked conflict by conflict; any other one hunk by hunk.
pub(crate) fn annotation_navigation_state_for(
  file_status: Option<RepoStatusKind>,
  editor: &Editor,
  cx: &App,
) -> Option<AnnotationNavigationState> {
  if let Some(state) = conflict_navigation_state_for(file_status, editor, cx) {
    return Some(AnnotationNavigationState {
      active_index: state.active_index,
      total: state.total,
      kind: AnnotationKind::Conflict,
    });
  }
  editor
    .hunk_navigation_state(cx)
    .map(|state| AnnotationNavigationState {
      active_index: state.active_index,
      total: state.total,
      kind: AnnotationKind::Change,
    })
}

pub(crate) fn can_navigate_annotations(state: Option<AnnotationNavigationState>) -> bool {
  state.is_some_and(|state| state.total > 1)
}

pub(crate) fn navigate_annotation(
  editor: &mut Editor,
  file_status: Option<RepoStatusKind>,
  direction: AnnotationDirection,
  cx: &mut Context<Editor>,
) {
  let walks_conflicts = matches!(file_status, Some(RepoStatusKind::Conflicted))
    && editor.conflict_navigation_state(cx).is_some();
  if walks_conflicts {
    editor.navigate_conflict(direction.conflict(), cx);
  } else {
    editor.navigate_hunk(direction.hunk(), cx);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn can_navigate_annotations_requires_multiple_annotations() {
    assert!(!can_navigate_annotations(None));
    assert!(!can_navigate_annotations(Some(AnnotationNavigationState {
      active_index: 0,
      total: 1,
      kind: AnnotationKind::Conflict,
    })));
    assert!(can_navigate_annotations(Some(AnnotationNavigationState {
      active_index: 0,
      total: 2,
      kind: AnnotationKind::Conflict,
    })));
    assert!(can_navigate_annotations(Some(AnnotationNavigationState {
      active_index: 0,
      total: 3,
      kind: AnnotationKind::Change,
    })));
  }
}
