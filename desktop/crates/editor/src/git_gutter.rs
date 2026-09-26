use std::{
  collections::{BTreeMap, HashMap, HashSet},
  ops::Range,
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
  /// In document order: the stops of change navigation, and what expands.
  pub hunks: Vec<GitGutterHunk>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitGutterHunk {
  pub group_id: Arc<str>,
  /// Where the hunk sits in the committed file. Typing in the buffer or staging
  /// leaves it in place where the group id, a hash of line numbers and content,
  /// changes: it is what remembers an expanded hunk.
  pub head_anchor: usize,
  /// The document lines it adds; empty at the line its removal sits above.
  pub doc_lines: Range<usize>,
}

impl GitGutterHunk {
  pub(crate) fn touches_doc_line(&self, doc_line: usize) -> bool {
    if self.doc_lines.is_empty() {
      // A removal sits between two lines: either one reaches it.
      doc_line == self.doc_lines.start || doc_line + 1 == self.doc_lines.start
    } else {
      self.doc_lines.contains(&doc_line)
    }
  }
}

impl GitGutterMarkers {
  /// The inline diff of the file, whole: the plain view marks it, and shows
  /// the hunks it expands from it.
  pub(crate) fn source_projection(doc_line_count: usize, diffs: &DiffSet) -> Projection {
    Projection::from_diffs(
      doc_line_count,
      &diffs.uncommitted,
      &diffs.unstaged,
      &diffs.staged,
      &HashMap::new(),
      false,
    )
  }

  #[cfg(test)]
  pub(crate) fn from_diffs(doc_line_count: usize, diffs: &DiffSet) -> Self {
    Self::from_projection(&Self::source_projection(doc_line_count, diffs))
  }

  /// Keeps only where each change of the diff sits in the document; the diff
  /// already tells staged lines from unstaged ones.
  pub(crate) fn from_projection(projection: &Projection) -> Self {
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
    let mut hunk_index_by_group: HashMap<Arc<str>, usize> = HashMap::new();
    // The committed line the next removal or addition would sit at.
    let mut next_head_line = 0;
    for line in &projection.lines {
      let hunk_line = match line {
        DisplayLine::Doc {
          doc_line,
          change: Some(ChangeKind::Added),
          group_id: Some(group_id),
          ..
        } => Some((group_id, next_head_line, Some(*doc_line), *doc_line)),
        DisplayLine::Modified {
          doc_line,
          old_line,
          group_id: Some(group_id),
          ..
        } => Some((group_id, *old_line, Some(*doc_line), *doc_line)),
        DisplayLine::Removed {
          anchor_line,
          old_line,
          group_id: Some(group_id),
          ..
        } => Some((group_id, *old_line, None, *anchor_line)),
        _ => None,
      };
      match line {
        DisplayLine::Doc {
          old_line: Some(old_line),
          ..
        }
        | DisplayLine::Modified { old_line, .. }
        | DisplayLine::Removed { old_line, .. } => next_head_line = old_line + 1,
        _ => {}
      }
      if let Some((group_id, head_line, added_doc_line, doc_position)) = hunk_line {
        let index = *hunk_index_by_group
          .entry(group_id.clone())
          .or_insert_with(|| {
            markers.hunks.push(GitGutterHunk {
              group_id: group_id.clone(),
              head_anchor: head_line,
              doc_lines: doc_position..doc_position,
            });
            markers.hunks.len() - 1
          });
        if let (Some(doc_line), Some(hunk)) = (added_doc_line, markers.hunks.get_mut(index)) {
          if hunk.doc_lines.is_empty() {
            hunk.doc_lines = doc_line..doc_line + 1;
          } else {
            hunk.doc_lines.end = hunk.doc_lines.end.max(doc_line + 1);
          }
        }
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

/// The plain file with the removed lines of the expanded hunks back in place,
/// and those hunks marked as in the diff so their lines and actions show. Every
/// document line stays: nothing folds as it does in the diff.
pub(crate) fn expanded_file_projection(
  source: &Projection,
  markers: &GitGutterMarkers,
  expanded_head_anchors: &HashSet<usize>,
  doc_line_count: usize,
) -> Option<Projection> {
  let expanded_groups = markers
    .hunks
    .iter()
    .filter(|hunk| expanded_head_anchors.contains(&hunk.head_anchor))
    .map(|hunk| hunk.group_id.clone())
    .collect::<HashSet<_>>();
  if expanded_groups.is_empty() {
    return None;
  }

  let mut expanded_doc_lines = HashMap::new();
  let mut removed_before: BTreeMap<usize, Vec<DisplayLine>> = BTreeMap::new();
  for line in &source.lines {
    match line {
      DisplayLine::Doc {
        doc_line,
        group_id: Some(group_id),
        ..
      } if expanded_groups.contains(group_id) => {
        expanded_doc_lines.insert(*doc_line, line.clone());
      }
      DisplayLine::Removed {
        anchor_line,
        group_id: Some(group_id),
        ..
      } if expanded_groups.contains(group_id) => {
        removed_before
          .entry(*anchor_line)
          .or_default()
          .push(line.clone());
      }
      _ => {}
    }
  }

  let mut lines = Vec::with_capacity(doc_line_count);
  for doc_line in 0..doc_line_count {
    if let Some(removed) = removed_before.remove(&doc_line) {
      lines.extend(removed);
    }
    lines.push(
      expanded_doc_lines
        .remove(&doc_line)
        .unwrap_or(DisplayLine::Doc {
          doc_line,
          old_line: None,
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        }),
    );
  }
  // Removed at the end of the file, below its last line.
  for removed in removed_before.into_values() {
    lines.extend(removed);
  }

  let groups = source
    .groups
    .iter()
    .filter(|(group_id, _)| expanded_groups.contains(*group_id))
    .map(|(group_id, group)| (group_id.clone(), group.clone()))
    .collect();
  Some(Projection::from_lines(
    doc_line_count,
    lines,
    groups,
    None,
    None,
  ))
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
    let hunks = markers
      .hunks
      .iter()
      .map(|hunk| (hunk.head_anchor, hunk.doc_lines.clone()))
      .collect::<Vec<_>>();
    assert_eq!(hunks, vec![(1, 1..2), (3, 3..4)]);
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
    assert_eq!(markers.hunks.len(), 1);
    assert_eq!(markers.hunks[0].head_anchor, 1);
    assert_eq!(markers.hunks[0].doc_lines, 1..1);
    assert!(markers.hunks[0].touches_doc_line(0));
    assert!(markers.hunks[0].touches_doc_line(1));
    assert!(!markers.hunks[0].touches_doc_line(2));
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

  fn doc_and_removed(projection: &Projection) -> Vec<String> {
    projection
      .lines
      .iter()
      .map(|line| match line {
        DisplayLine::Doc {
          doc_line,
          change: Some(ChangeKind::Added),
          ..
        } => format!("+{doc_line}"),
        DisplayLine::Doc { doc_line, .. } => format!("{doc_line}"),
        DisplayLine::Removed { text, .. } => format!("-{}", text.trim_end()),
        _ => "?".to_string(),
      })
      .collect()
  }

  #[test]
  fn an_expanded_hunk_shows_its_removed_lines_and_nothing_folds() {
    let head = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    let buffer = "one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n";
    let source = GitGutterMarkers::source_projection(11, &diffs(head, head, buffer));
    let markers = GitGutterMarkers::from_projection(&source);
    let second = markers.hunks[1].head_anchor;

    assert!(expanded_file_projection(&source, &markers, &HashSet::new(), 11).is_none());

    let projection =
      expanded_file_projection(&source, &markers, &HashSet::from([second]), 11).expect("expanded");
    assert_eq!(
      doc_and_removed(&projection),
      vec![
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "-ten", "+9", "10"
      ]
    );
    assert_eq!(projection.groups.len(), 1);
    assert!(projection.groups.contains_key(&markers.hunks[1].group_id));
  }

  #[test]
  fn a_hunk_keeps_its_head_anchor_when_lines_are_added_above_it() {
    let head = "one\ntwo\nthree\n";
    let before = GitGutterMarkers::from_diffs(4, &diffs(head, head, "one\ntwo\nTHREE\n"));
    let after = GitGutterMarkers::from_diffs(5, &diffs(head, head, "zero\none\ntwo\nTHREE\n"));

    let last_anchor = |markers: &GitGutterMarkers| {
      markers
        .hunks
        .last()
        .map(|hunk| hunk.head_anchor)
        .expect("hunk")
    };
    assert_eq!(last_anchor(&before), 2);
    assert_eq!(last_anchor(&after), 2);
    assert_ne!(
      before.hunks.last().map(|hunk| hunk.group_id.clone()),
      after.hunks.last().map(|hunk| hunk.group_id.clone()),
      "the group id moves with the lines, the anchor does not"
    );
  }
}
