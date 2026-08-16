//! Which branches a palette offers, and in what order.

use git::{BranchKind, BranchRef};
use ui::{CommandPaletteBranch, CommandPaletteBranchKind, CommandPaletteStash};

pub(crate) fn palette_branch(branch: &BranchRef) -> CommandPaletteBranch {
  CommandPaletteBranch {
    name: branch.name.clone().into(),
    kind: match branch.kind {
      BranchKind::Local => CommandPaletteBranchKind::Local,
      BranchKind::Remote => CommandPaletteBranchKind::Remote,
    },
  }
}

/// Everything but the current branch, upstream and default branch first: those
/// are what a rebase targets nine times out of ten.
pub(crate) fn rebase_branch_candidates(
  branches: &[BranchRef],
  current_branch_name: Option<&str>,
  upstream_branch: Option<&BranchRef>,
  default_branch: Option<&BranchRef>,
) -> Vec<CommandPaletteBranch> {
  let mut candidates = branches
    .iter()
    .enumerate()
    .filter(|(_, branch)| {
      !matches!(
        (branch.kind, current_branch_name),
        (BranchKind::Local, Some(current_branch_name)) if branch.name == current_branch_name
      )
    })
    .map(|(index, branch)| {
      (
        index,
        rebase_branch_priority(branch, upstream_branch, default_branch),
        branch,
      )
    })
    .collect::<Vec<_>>();

  candidates.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
  candidates
    .into_iter()
    .map(|(_, _, branch)| palette_branch(branch))
    .collect()
}

/// Every branch but the one checked out: deleting it is not an option.
pub(crate) fn delete_branch_candidates(
  branches: &[BranchRef],
  current_branch_name: Option<&str>,
) -> Vec<CommandPaletteBranch> {
  branches
    .iter()
    .filter(|branch| match branch.kind {
      BranchKind::Local => current_branch_name.is_none_or(|current| branch.name != current),
      BranchKind::Remote => true,
    })
    .map(palette_branch)
    .collect()
}

pub(crate) fn palette_stashes(stashes: &[git::StashEntry]) -> Vec<CommandPaletteStash> {
  stashes
    .iter()
    .map(|stash| CommandPaletteStash {
      index: stash.index,
      name: stash.name.clone().into(),
      oid: stash.oid.clone().into(),
    })
    .collect()
}

pub(crate) fn rebase_branch_priority(
  branch: &BranchRef,
  upstream_branch: Option<&BranchRef>,
  default_branch: Option<&BranchRef>,
) -> usize {
  if upstream_branch.is_some_and(|upstream| upstream == branch) {
    return 0;
  }
  if default_branch.is_some_and(|default| default == branch) {
    return 1;
  }
  match (branch.kind, branch.name.as_str()) {
    (BranchKind::Local, "main" | "master") => 2,
    (BranchKind::Remote, "origin/main" | "origin/master" | "upstream/main" | "upstream/master") => {
      2
    }
    _ => 3,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn deleting_offers_every_branch_but_the_one_checked_out() {
    let branches = vec![
      BranchRef {
        name: "feature".into(),
        kind: BranchKind::Local,
      },
      BranchRef {
        name: "main".into(),
        kind: BranchKind::Local,
      },
      BranchRef {
        name: "origin/feature".into(),
        kind: BranchKind::Remote,
      },
    ];

    let candidates = delete_branch_candidates(&branches, Some("feature"));
    let names = candidates
      .iter()
      .map(|branch| branch.name.to_string())
      .collect::<Vec<_>>();
    assert_eq!(names, vec!["main", "origin/feature"]);

    // A remote branch of the same name is still deletable.
    assert!(
      candidates
        .iter()
        .any(|branch| branch.kind == CommandPaletteBranchKind::Remote)
    );
  }

  #[test]
  fn stashes_travel_with_their_index_and_oid() {
    let stashes = vec![
      git::StashEntry {
        index: 0,
        name: "WIP on main".to_string(),
        oid: "abc123".to_string(),
      },
      git::StashEntry {
        index: 1,
        name: "older".to_string(),
        oid: "def456".to_string(),
      },
    ];

    let palette = palette_stashes(&stashes);
    assert_eq!(palette.len(), 2);
    assert_eq!(palette[0].index, 0);
    assert_eq!(palette[0].name.as_ref(), "WIP on main");
    assert_eq!(palette[1].oid.as_ref(), "def456");
  }

  #[test]
  fn command_palette_rebase_branches_exclude_current_branch_and_prioritize_base_branches() {
    let branches = vec![
      BranchRef {
        name: "feature".into(),
        kind: BranchKind::Local,
      },
      BranchRef {
        name: "topic".into(),
        kind: BranchKind::Local,
      },
      BranchRef {
        name: "main".into(),
        kind: BranchKind::Local,
      },
      BranchRef {
        name: "origin/main".into(),
        kind: BranchKind::Remote,
      },
      BranchRef {
        name: "origin/feature".into(),
        kind: BranchKind::Remote,
      },
    ];

    let rebase_branches = rebase_branch_candidates(
      &branches,
      Some("feature"),
      Some(&BranchRef {
        name: "origin/feature".into(),
        kind: BranchKind::Remote,
      }),
      Some(&BranchRef {
        name: "origin/main".into(),
        kind: BranchKind::Remote,
      }),
    );

    assert_eq!(
      rebase_branches,
      vec![
        CommandPaletteBranch {
          name: "origin/feature".into(),
          kind: CommandPaletteBranchKind::Remote,
        },
        CommandPaletteBranch {
          name: "origin/main".into(),
          kind: CommandPaletteBranchKind::Remote,
        },
        CommandPaletteBranch {
          name: "main".into(),
          kind: CommandPaletteBranchKind::Local,
        },
        CommandPaletteBranch {
          name: "topic".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      ]
    );
  }
}
