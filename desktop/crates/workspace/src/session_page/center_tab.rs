use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum CenterTabKind {
  Chat,
  File,
  Diff,
  InteractiveRebase,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct CenterTab {
  pub(super) kind: CenterTabKind,
  pub(super) path: Option<PathBuf>,
  pub(super) conversation_id: Option<String>,
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
    }
  }

  pub(super) fn chat_for(conversation_id: impl Into<String>) -> Self {
    Self {
      kind: CenterTabKind::Chat,
      path: None,
      conversation_id: Some(conversation_id.into()),
    }
  }

  pub(super) fn file(path: PathBuf) -> Self {
    Self {
      kind: CenterTabKind::File,
      path: Some(path),
      conversation_id: None,
    }
  }

  pub(super) fn diff(path: PathBuf) -> Self {
    Self {
      kind: CenterTabKind::Diff,
      path: Some(path),
      conversation_id: None,
    }
  }

  pub(super) fn interactive_rebase() -> Self {
    Self {
      kind: CenterTabKind::InteractiveRebase,
      path: None,
      conversation_id: None,
    }
  }

  pub(super) fn path(&self) -> Option<&Path> {
    self.path.as_deref()
  }

  pub(super) fn conversation_id(&self) -> Option<&str> {
    self.conversation_id.as_deref()
  }

  pub(super) fn is_closeable(&self) -> bool {
    match self.kind {
      CenterTabKind::Chat => self.conversation_id.is_some(),
      CenterTabKind::File | CenterTabKind::Diff => true,
      CenterTabKind::InteractiveRebase => false,
    }
  }
}
