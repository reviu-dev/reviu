use gpui::actions;

actions!(
  workspace,
  [
    NavigateBack,
    CloseWorkspacePage,
    OpenGitPage,
    OpenSessionPage,
    RefreshCurrentPage,
    ToggleTerminalSidebar,
    ShowBranchSwitcher,
    OpenGitHistorySidebar,
    OpenGitChangesSidebar,
    FocusFileTree,
    PullChanges,
    PushChanges,
    ForcePushChanges,
    ToggleDiffView,
    SwitchToPrBranch,
    ToggleHideWhitespace,
    PreviousAnnotation,
    NextAnnotation,
    PreviousReviewComment,
    NextReviewComment,
    ToggleCommitByCommit,
    PreviousPrCommit,
    NextPrCommit,
    CommentHunk,
    SendReviewCommentsToAgent,
    AddSelectionToAgent,
    ToggleHunkStage,
    RestoreHunk,
    ToggleFileStage,
    RestoreFile,
    AcceptBothConflict,
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
mod agent_review;
mod agent_settings;
mod analytics;
mod annotations;
mod api;
pub mod app_log;
mod app_profile;
mod app_update;
mod auth_state;
mod billing_page;
mod browser_extensions_dialog;
mod command_usage;
mod dock_badge;

mod changes_list;
mod config;
mod crash_report;
mod date_format;
mod diff_view_policy;
mod dock_panel;
mod feedback_dialog;
mod file_preview;
mod file_search_palette;
mod file_view;
mod git_config_page;
mod git_page;
pub mod github_navigation;
mod github_notifications;
mod github_pr_details_page;
mod github_shared;
mod history_list;
mod hunk_actions;
mod interactive_rebase;
mod interactive_rebase_todo_view;
pub mod navigation;
mod palette_actions;
mod palette_branches;
mod pricing_copy;
mod repo_command;
mod repo_state;
mod sentry_context;
mod session_page;
mod settings_page;
mod shortcuts;
pub mod status_bar;
mod svg_preview;
#[cfg(test)]
mod test_support;
mod workspace;

pub use app_profile::{AppProfile, URL_SCHEME_PROD};
pub use auth_state::AuthStateStore;
pub use crash_report::{
  StartupCrashReport, install_crash_reporter, show_startup_crash_report_notification,
  take_pending_startup_crash_report,
};
pub use git_page::{
  AuthCallbackTarget, CommitChanges, OpenRepository, SaveFile, ShowCommandPalette, ShowFileSearch,
};
pub use shortcuts::{SHOW_COMMAND_PALETTE_SHORTCUT, install_app_key_bindings};
pub use workspace::{WorkspaceView, build_app_menus};
