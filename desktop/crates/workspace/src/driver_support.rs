use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverBranchKind {
  Local,
  Remote,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DriverBranchRef {
  pub name: String,
  pub kind: DriverBranchKind,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DriverGitAction {
  StageAll,
  UnstageAll,
  Commit {
    message: String,
  },
  Push,
  ForcePush,
  Pull,
  Fetch,
  UndoLastCommit,
  Amend {
    message: Option<String>,
  },
  CheckoutDetached {
    target: String,
  },
  SwitchBranch {
    branch: DriverBranchRef,
  },
  CreateBranch {
    name: String,
  },
  CreateBranchFrom {
    name: String,
    base: DriverBranchRef,
  },
  DeleteBranch {
    branch: DriverBranchRef,
  },
  MergeBranch {
    branch: DriverBranchRef,
  },
  AbortMerge,
  RebaseBranch {
    branch: DriverBranchRef,
  },
  ContinueRebase,
  SkipRebase,
  AbortRebase,
  Stash {
    include_untracked: bool,
    message: Option<String>,
  },
  ApplyStash {
    index: usize,
    name: String,
  },
  PopStash {
    index: usize,
    name: String,
  },
  DropStash {
    index: usize,
    name: String,
  },
  CherryPick {
    commit_hashes: Vec<String>,
  },
  RestoreAll,
}
