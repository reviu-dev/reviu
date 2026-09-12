//! Walking a diff: conflicts when the file has them, changes otherwise.

use editor::{ConflictNavigationDirection, Editor, HunkNavigationDirection};
use gpui::Context;

use git::RepoStatusKind;
#[cfg(test)]
use gpui::App;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnnotationKind {
  Conflict,
  Change,
}

#[cfg(test)]
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

#[cfg(test)]
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
#[cfg(test)]
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
