use gpui::actions;

actions!(
  workspace,
  [
    NavigateBack,
    CloseWorkspacePage,
    OpenSessionPage,
    ToggleTerminalSidebar,
    ShowBranchSwitcher,
    OpenGitHistorySidebar,
    OpenGitChangesSidebar,
    ReturnFocusToEditor,
    OpenFilesSidebar,
    OpenReviewSidebar,
    OpenPullRequestSidebar,
    PullChanges,
    PushChanges,
    ForcePushChanges,
    ToggleDiffView,
    ToggleHideWhitespace,
    PreviousAnnotation,
    NextAnnotation,
    CommentHunk,
    SendReviewCommentsToAgent,
    AddSelectionToAgent,
    JumpToLatestMessage,
    ToggleHunkStage,
    RestoreHunk,
    ToggleFileStage,
    RestoreFile,
    AcceptBothConflict,
    OpenBillingPage,
    OpenGitConfigPage,
    OpenSettingsPage,
    OpenAboutPage,
  ]
);

mod about_dialog;
mod actions;
mod agent_chat_state;
mod agent_notification;
mod agent_review;
mod agent_review_store;
mod agent_settings;
mod analytics;
mod annotations;
mod api;
pub mod app_log_sink;
mod app_profile;
mod app_update;
pub mod auth_flow;
mod auth_state;
mod billing_dialog;
mod browser_extensions_dialog;
mod command_usage;
mod dock_badge;

mod changes_list;
mod config;
mod conversation_hub;
mod crash_report;
mod date_format;
mod diff_toolbar;
mod diff_view_policy;
mod dock_panel;
mod feedback_dialog;
mod file_preview;
mod file_search_palette;
mod file_tree;
mod file_view;
mod git_config_page;
mod git_telemetry;
pub mod github_navigation;
mod github_notifications;
mod github_shared;
mod history_list;
mod hunk_actions;
mod inbox;
mod interactive_rebase;
mod interactive_rebase_todo_view;
mod keybindings_file;
pub mod navigation;
mod open_intent;
mod palette_actions;
mod palette_branches;
mod pricing_copy;
mod pro_promise;
mod pull_request_checks;
mod pull_request_dialog;
mod pull_request_merge;
mod pull_request_refresh;
mod pull_request_review_comments;
mod pull_request_review_submission;
mod pull_request_reviewers;
pub mod pull_request_surface;
mod repo_command;
mod repo_snapshot;
mod repo_state;
mod review_destination;
mod review_list;
mod review_submit_dialog;
mod sentry_context;
mod session_list;
mod session_page;
mod settings_file;
mod settings_page;
mod shortcuts;
pub mod status_bar;
mod status_poll;
mod svg_preview;
#[cfg(test)]
mod test_support;
mod workspace;
mod workspace_window;

pub use actions::{CommitChanges, OpenRepository, SaveFile, ShowCommandPalette, ShowFileSearch};
pub use app_profile::{AppProfile, URL_SCHEME_PROD};
pub use auth_state::AuthStateStore;
pub use crash_report::{
  StartupCrashReport, install_crash_reporter, show_startup_crash_report_notification,
  take_pending_startup_crash_report,
};
pub use shortcuts::{SHOW_COMMAND_PALETTE_SHORTCUT, install_app_key_bindings};
pub use workspace::{WorkspaceView, build_app_menus};
