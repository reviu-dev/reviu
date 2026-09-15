use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum CenterTabKind {
  Chat,
  File,
  Diff,
  InteractiveRebase,
  Terminal,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq, Hash, serde::Serialize)]
pub(super) enum CenterTabSnapshot {
  AgentTool {
    old_text: Option<String>,
    new_text: String,
  },
  Commit {
    oid: String,
  },
  PullRequestRange {
    base: String,
    head: String,
  },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct CenterTab {
  pub(super) kind: CenterTabKind,
  pub(super) path: Option<PathBuf>,
  pub(super) conversation_id: Option<String>,
  pub(super) snapshot: Option<CenterTabSnapshot>,
  pub(super) terminal_id: Option<u64>,
  pub(super) untitled_id: Option<u64>,
}

impl CenterTab {
  pub(super) fn default_tabs() -> Vec<Self> {
    vec![Self::chat()]
  }

  pub(super) fn with_chat_tab(mut tabs: Vec<Self>) -> Vec<Self> {
    if !tabs.iter().any(|tab| tab.kind == CenterTabKind::Chat) {
      tabs.insert(0, Self::chat());
    }
    tabs
  }

  pub(super) fn chat() -> Self {
    Self {
      kind: CenterTabKind::Chat,
      path: None,
      conversation_id: None,
      snapshot: None,
      terminal_id: None,
      untitled_id: None,
    }
  }

  pub(super) fn chat_for(conversation_id: impl Into<String>) -> Self {
    Self {
      kind: CenterTabKind::Chat,
      path: None,
      conversation_id: Some(conversation_id.into()),
      snapshot: None,
      terminal_id: None,
      untitled_id: None,
    }
  }

  pub(super) fn file(path: PathBuf) -> Self {
    Self {
      kind: CenterTabKind::File,
      path: Some(path),
      conversation_id: None,
      snapshot: None,
      terminal_id: None,
      untitled_id: None,
    }
  }

  pub(super) fn untitled(id: u64) -> Self {
    Self {
      kind: CenterTabKind::File,
      path: None,
      conversation_id: None,
      snapshot: None,
      terminal_id: None,
      untitled_id: Some(id),
    }
  }

  pub(super) fn diff(path: PathBuf) -> Self {
    Self {
      kind: CenterTabKind::Diff,
      path: Some(path),
      conversation_id: None,
      snapshot: None,
      terminal_id: None,
      untitled_id: None,
    }
  }

  pub(super) fn agent_snapshot(path: PathBuf, old_text: Option<String>, new_text: String) -> Self {
    Self {
      kind: CenterTabKind::Diff,
      path: Some(path),
      conversation_id: None,
      snapshot: Some(CenterTabSnapshot::AgentTool { old_text, new_text }),
      terminal_id: None,
      untitled_id: None,
    }
  }

  pub(super) fn commit_snapshot(path: PathBuf, oid: String) -> Self {
    Self {
      kind: CenterTabKind::Diff,
      path: Some(path),
      conversation_id: None,
      snapshot: Some(CenterTabSnapshot::Commit { oid }),
      terminal_id: None,
      untitled_id: None,
    }
  }

  pub(super) fn pull_request_snapshot(path: PathBuf, base: String, head: String) -> Self {
    Self {
      kind: CenterTabKind::Diff,
      path: Some(path),
      conversation_id: None,
      snapshot: Some(CenterTabSnapshot::PullRequestRange { base, head }),
      terminal_id: None,
      untitled_id: None,
    }
  }

  pub(super) fn interactive_rebase() -> Self {
    Self {
      kind: CenterTabKind::InteractiveRebase,
      path: None,
      conversation_id: None,
      snapshot: None,
      terminal_id: None,
      untitled_id: None,
    }
  }

  pub(super) fn terminal(id: u64) -> Self {
    Self {
      kind: CenterTabKind::Terminal,
      path: None,
      conversation_id: None,
      snapshot: None,
      terminal_id: Some(id),
      untitled_id: None,
    }
  }

  pub(super) fn path(&self) -> Option<&Path> {
    self.path.as_deref()
  }

  pub(super) fn conversation_id(&self) -> Option<&str> {
    self.conversation_id.as_deref()
  }

  pub(super) fn snapshot(&self) -> Option<&CenterTabSnapshot> {
    self.snapshot.as_ref()
  }

  pub(super) fn terminal_id(&self) -> Option<u64> {
    self.terminal_id
  }

  pub(super) fn untitled_id(&self) -> Option<u64> {
    self.untitled_id
  }

  pub(super) fn is_untitled(&self) -> bool {
    self.kind == CenterTabKind::File && self.path.is_none() && self.untitled_id.is_some()
  }

  pub(super) fn is_closeable(&self) -> bool {
    match self.kind {
      CenterTabKind::Chat => self.conversation_id.is_some(),
      CenterTabKind::File | CenterTabKind::Diff | CenterTabKind::Terminal => true,
      CenterTabKind::InteractiveRebase => false,
    }
  }
}
