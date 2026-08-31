use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverNotificationKind {
  Info,
  Success,
  Warning,
  Error,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DriverNotification {
  pub kind: DriverNotificationKind,
  pub title: Option<String>,
  pub message: String,
}

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
#[serde(tag = "target", rename_all = "snake_case")]
pub enum DriverInteractiveRebaseTarget {
  Branch { branch: DriverBranchRef },
  BranchInPlace { branch: DriverBranchRef },
  HeadCount { count: usize },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverInteractiveRebaseAction {
  Pick,
  Squash,
  Fixup,
  Drop,
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
  InteractiveRebase {
    target: DriverInteractiveRebaseTarget,
    actions: Vec<DriverInteractiveRebaseAction>,
  },
  RestoreAll,
}
