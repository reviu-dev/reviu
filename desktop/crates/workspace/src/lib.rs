mod config;
mod git_page;
mod settings_page;
mod workspace;

pub use git_page::{CommitChanges, OpenRepository, SaveFile};
pub use workspace::WorkspaceView;
