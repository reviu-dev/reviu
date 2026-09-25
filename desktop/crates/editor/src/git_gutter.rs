use std::{
  collections::{HashMap, HashSet},
  sync::Arc,
};

use git::{DiffLineKind, DiffSet};

use crate::projection::{ChangeKind, DisplayLine, HunkState, Projection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GitGutterLineKind {
  Added,
  /// The line replaces removed ones: part of a hunk that both adds and removes.
  Modified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GitGutterLine {
  pub kind: GitGutterLineKind,
  pub state: HunkState,
}

/// Lines removed with nothing added in their place, just above `before_doc_line`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GitGutterDeletion {
  pub before_doc_line: usize,
  pub state: HunkState,
}

/// What the plain file shows of its changes: removed lines have no row there, so
/// a hunk that only removes becomes a mark between the lines around it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GitGutterMarkers {
  pub lines: HashMap<usize, GitGutterLine>,
  pub deletions: Vec<GitGutterDeletion>,
  /// Each hunk with the first document line it touches, in document order: the
  /// stops of change navigation.
  pub hunk_starts: Vec<(Arc<str>, usize)>,
}

impl GitGutterMarkers {
  /// Reuses the diff projection, which already tells staged lines from unstaged
  /// ones, and keeps only where each change sits in the document.
  pub(crate) fn from_diffs(doc_line_count: usize, diffs: &DiffSet) -> Self {
    let projection = Projection::from_diffs(
      doc_line_count,
      &diffs.uncommitted,
      &diffs.unstaged,
      &diffs.staged,
      &HashMap::new(),
      false,
    );
    let group_adds_and_removes = |group_id: Option<&Arc<str>>| {
      let group = group_id.and_then(|group_id| projection.groups.get(group_id))?;
      let adds = group
        .hunk
        .lines
        .iter()
        .any(|line| line.kind == DiffLineKind::Add);
      let removes = group
        .hunk
        .lines
        .iter()
        .any(|line| line.kind == DiffLineKind::Remove);
      Some((adds, removes))
    };

    let mut markers = Self::default();
    let mut seen_hunks = HashSet::new();
    for line in &projection.lines {
      let hunk_start = match line {
        DisplayLine::Doc {
          doc_line,
          change: Some(ChangeKind::Added),
          group_id: Some(group_id),
          ..
        }
        | DisplayLine::Modified {
          doc_line,
          group_id: Some(group_id),
          ..
        } => Some((group_id, *doc_line)),
        DisplayLine::Removed {
          anchor_line,
          group_id: Some(group_id),
          ..
        } => Some((group_id, *anchor_line)),
        _ => None,
      };
      if let Some((group_id, doc_line)) = hunk_start
        && seen_hunks.insert(group_id.clone())
      {
        markers.hunk_starts.push((group_id.clone(), doc_line));
      }

      match line {
        DisplayLine::Doc {
          doc_line,
          change: Some(ChangeKind::Added),
          hunk: Some(state),
          group_id,
          ..
        } => {
          let kind = match group_adds_and_removes(group_id.as_ref()) {
            Some((_, true)) => GitGutterLineKind::Modified,
            _ => GitGutterLineKind::Added,
          };
          markers.lines.insert(
            *doc_line,
            GitGutterLine {
              kind,
              state: *state,
            },
          );
        }
        DisplayLine::Modified { doc_line, hunk, .. } => {
          markers.lines.insert(
            *doc_line,
            GitGutterLine {
              kind: GitGutterLineKind::Modified,
              state: *hunk,
            },
          );
        }
        DisplayLine::Removed {
          anchor_line,
          hunk,
          group_id,
          ..
        } => {
          // A hunk that also adds is already shown on its added lines.
          if matches!(group_adds_and_removes(group_id.as_ref()), Some((true, _))) {
            continue;
          }
          let deletion = GitGutterDeletion {
            before_doc_line: *anchor_line,
            state: *hunk,
          };
          if markers.deletions.last() != Some(&deletion) {
            markers.deletions.push(deletion);
          }
        }
        _ => {}
      }
    }
    markers
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use git::{DiffKind, FileDiff};
  use std::path::Path;

  fn diffs(head: &str, index: &str, buffer: &str) -> DiffSet {
    let path = Path::new("file.txt");
    let compute = |kind, old: &str, new: &str| {
      git::compute_buffer_diff(kind, Some(old), new, path, false).expect("diff")
    };
    let staged = if head == index {
      FileDiff::empty(DiffKind::Staged)
    } else {
      compute(DiffKind::Staged, head, index)
    };
    DiffSet {
      uncommitted: compute(DiffKind::Uncommitted, head, buffer),
      unstaged: compute(DiffKind::Unstaged, index, buffer),
      staged,
    }
  }

  fn line(kind: GitGutterLineKind, state: HunkState) -> GitGutterLine {
    GitGutterLine { kind, state }
  }

  #[test]
  fn added_and_modified_lines_are_marked_where_they_sit() {
    let head = "one\ntwo\nthree\n";
    let buffer = "one\nTWO\nthree\nfour\n";
    let markers = GitGutterMarkers::from_diffs(5, &diffs(head, head, buffer));

    assert_eq!(
      markers.lines,
      HashMap::from([
        (1, line(GitGutterLineKind::Modified, HunkState::Unstaged)),
        (3, line(GitGutterLineKind::Added, HunkState::Unstaged)),
      ])
    );
    assert!(markers.deletions.is_empty());
    let starts = markers
      .hunk_starts
      .iter()
      .map(|(_, doc_line)| *doc_line)
      .collect::<Vec<_>>();
    assert_eq!(starts, vec![1, 3]);
  }

  #[test]
  fn a_removal_alone_is_a_mark_before_the_next_line() {
    let head = "one\ntwo\nthree\n";
    let buffer = "one\nthree\n";
    let markers = GitGutterMarkers::from_diffs(3, &diffs(head, head, buffer));

    assert!(markers.lines.is_empty());
    assert_eq!(
      markers.deletions,
      vec![GitGutterDeletion {
        before_doc_line: 1,
        state: HunkState::Unstaged,
      }]
    );
    assert_eq!(markers.hunk_starts.len(), 1);
    assert_eq!(markers.hunk_starts[0].1, 1);
  }

  #[test]
  fn staged_changes_keep_their_state() {
    let head = "one\ntwo\n";
    let index = "one\ntwo\nthree\n";
    let buffer = "one\ntwo\nthree\nfour\n";
    let markers = GitGutterMarkers::from_diffs(5, &diffs(head, index, buffer));

    assert_eq!(
      markers.lines,
      HashMap::from([
        (2, line(GitGutterLineKind::Added, HunkState::Staged)),
        (3, line(GitGutterLineKind::Added, HunkState::Unstaged)),
      ])
    );
  }

  #[test]
  fn a_clean_file_has_no_markers() {
    let head = "one\n";
    let markers = GitGutterMarkers::from_diffs(2, &diffs(head, head, head));
    assert_eq!(markers, GitGutterMarkers::default());
  }
}
