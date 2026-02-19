use gpui::actions;

actions!(workspace, [CloseWorkspacePage]);

mod api;
mod auth_state;
mod config;
mod git_config_page;
mod git_page;
mod github_page;
mod github_pr_details_page;
mod interactive_rebase_dialog;
mod settings_page;
mod workspace;

pub use git_page::{
  AuthCallbackTarget, CommitChanges, OpenRepository, SaveFile, ShowCommandPalette, ShowFileSearch,
};
pub use workspace::WorkspaceView;
