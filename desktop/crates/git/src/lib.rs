use std::path::PathBuf;

mod branch;
mod checkpoint;
mod commit;
mod diff;
mod history;
mod interactive_rebase;
mod remote_auth;
mod status;
mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod worktree;

pub use branch::*;
pub use checkpoint::*;
pub use commit::*;
pub use diff::*;
pub use git2::ApplyLocation;
pub use history::*;
pub use interactive_rebase::*;
pub use status::*;
pub use store::*;
pub use worktree::*;

pub fn find_global_config_path() -> Option<PathBuf> {
  git2::Config::find_global().ok()
}
