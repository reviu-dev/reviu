use std::path::PathBuf;

mod branch;
mod commit;
mod diff;
mod history;
mod interactive_rebase;
mod status;
mod store;

pub use branch::*;
pub use commit::*;
pub use diff::*;
pub use git2::ApplyLocation;
pub use history::*;
pub use interactive_rebase::*;
pub use status::*;
pub use store::*;

pub fn find_global_config_path() -> Option<PathBuf> {
  git2::Config::find_global().ok()
}
