//! Where the agent conversations live on disk.

use std::path::{Path, PathBuf};

pub(crate) const AGENT_CHAT_STATE_MAX_AGE: std::time::Duration =
  std::time::Duration::from_secs(60 * 60 * 24 * 30);
pub(crate) const AGENT_CHAT_STATE_MAX_CONVERSATIONS_PER_PROJECT: usize = 200;

pub(crate) fn agent_chat_state_dir() -> Option<std::path::PathBuf> {
  Some(dirs::config_dir()?.join("reviu").join("agent-chats"))
}
/// Agent tool-call locations are absolute; the diff view opens by repo-relative path.
pub(crate) fn agent_path_to_repo_relative(path: PathBuf, repo_root: Option<&Path>) -> PathBuf {
  repo_root
    .and_then(|root| path.strip_prefix(root).ok())
    .map(Path::to_path_buf)
    .unwrap_or(path)
}
#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn agent_path_strips_repo_root_when_absolute() {
    let root = Path::new("/home/u/proj");
    assert_eq!(
      agent_path_to_repo_relative(PathBuf::from("/home/u/proj/src/lib.rs"), Some(root)),
      PathBuf::from("src/lib.rs")
    );
  }

  #[test]
  fn agent_path_kept_when_outside_root_or_already_relative() {
    let root = Path::new("/home/u/proj");
    assert_eq!(
      agent_path_to_repo_relative(PathBuf::from("src/lib.rs"), Some(root)),
      PathBuf::from("src/lib.rs")
    );
    assert_eq!(
      agent_path_to_repo_relative(PathBuf::from("/other/x.rs"), Some(root)),
      PathBuf::from("/other/x.rs")
    );
    assert_eq!(
      agent_path_to_repo_relative(PathBuf::from("/abs/x.rs"), None),
      PathBuf::from("/abs/x.rs")
    );
  }
}
