use gpui::actions;

actions!(workspace, [CloseWorkspacePage]);

pub const SHOW_COMMAND_PALETTE_SHORTCUT: &str = "cmd-k";

mod about_page;
mod active_local_repo;
mod api;
mod app_profile;
mod app_update;
mod auth_state;
mod billing_page;
mod dock_badge;

mod config;
mod date_format;
mod file_preview;
mod file_search_palette;
mod git_config_page;
mod git_page;
mod github_navigation;
pub mod navigation;
mod github_page;
mod github_pr_details_page;
mod github_repo_page;
mod github_shared;
mod interactive_rebase_todo_view;
mod notification_count;
mod sentry_context;
mod settings_page;
mod workspace;

pub use app_profile::AppProfile;
pub use git_page::{
  AuthCallbackTarget, CommitChanges, OpenRepository, SaveFile, ShowCommandPalette, ShowFileSearch,
};
pub use workspace::WorkspaceView;
