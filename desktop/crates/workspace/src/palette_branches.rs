//! Which branches a palette offers, and in what order.

use git::{BranchKind, BranchRef};
use ui::{CommandPaletteBranch, CommandPaletteBranchKind};

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
