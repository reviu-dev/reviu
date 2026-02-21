use gpui::actions;

actions!(workspace, [CloseWorkspacePage]);

mod about_page;
mod api;
mod app_update;
mod auth_state;
mod billing_page;
mod config;
mod date_format;
mod git_config_page;
mod git_page;
mod github_page;
mod github_pr_details_page;
mod github_repo_page;
mod interactive_rebase_todo_view;
mod sentry_context;
mod settings_page;
mod workspace;

pub use git_page::{
  AuthCallbackTarget, CommitChanges, OpenRepository, SaveFile, ShowCommandPalette, ShowFileSearch,
};
pub use workspace::WorkspaceView;
