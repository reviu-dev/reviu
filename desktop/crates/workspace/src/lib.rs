use gpui::actions;

actions!(
  workspace,
  [
    NavigateBack,
    CloseWorkspacePage,
    OpenGitPage,
    OpenGithubPage,
    RefreshCurrentPage,
    ToggleTerminalSidebar,
    ShowBranchSwitcher,
    OpenGitHistorySidebar,
    OpenGitChangesSidebar,
    PullChanges,
    PushChanges,
    ForcePushChanges,
    ToggleDiffView,
    SwitchToPrBranch,
    PreviousAnnotation,
    NextAnnotation,
    PreviousPageTab,
    NextPageTab,
    OpenBillingPage,
    OpenGitConfigPage,
    OpenSettingsPage,
    OpenAboutPage,
  ]
);

mod about_page;
mod active_local_repo;
mod api;
pub mod app_log;
mod app_profile;
mod app_update;
mod auth_state;
mod billing_page;
mod dock_badge;

mod config;
mod crash_report;
mod date_format;
mod feedback_dialog;
mod file_preview;
mod file_search_palette;
mod git_config_page;
mod git_page;
mod github_commit_details_page;
mod github_create_repository_dialog;
mod github_home_tabs;
pub mod github_navigation;
mod github_page;
mod github_pr_details_page;
mod github_repo_page;
mod github_shared;
mod interactive_rebase_todo_view;
pub mod navigation;
mod notification_count;
mod number_format;
mod pricing_copy;
mod sentry_context;
mod settings_page;
mod shortcuts;
pub mod status_bar;
mod workspace;

pub use app_profile::{AppProfile, URL_SCHEME_PROD};
pub use crash_report::{
  StartupCrashReport, install_crash_reporter, show_startup_crash_report_notification,
  take_pending_startup_crash_report,
};
pub use git_page::{
  AuthCallbackTarget, CommitChanges, OpenRepository, SaveFile, ShowCommandPalette, ShowFileSearch,
};
pub use shortcuts::{SHOW_COMMAND_PALETTE_SHORTCUT, install_app_key_bindings};
pub use workspace::{WorkspaceView, build_app_menus};
