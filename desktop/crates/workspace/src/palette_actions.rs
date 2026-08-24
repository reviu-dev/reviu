//! Palette actions every page answers the same way.

use gpui::{App, SharedString, Window};
use ui::CommandPaletteAction;

use crate::github_navigation::{
  PullRequestFallback, open_commit_target, open_profile_target, open_pull_request_target,
  open_repo_target,
};
use crate::navigation::NavigationHistory;

/// Handles the navigation, GitHub link and dialog actions shared by every page.
/// Pages match their own actions first and route the rest here.
pub(crate) fn handle_global_command_palette_action(
  action: CommandPaletteAction,
  window: &mut Window,
  cx: &mut App,
) -> Result<(), SharedString> {
  match action {
    CommandPaletteAction::OpenSessionPage => {
      NavigationHistory::navigate("/session", cx);
      Ok(())
    }
    CommandPaletteAction::OpenGitConfigPage => {
      NavigationHistory::navigate("/git-config", cx);
      Ok(())
    }
    CommandPaletteAction::OpenSettingsPage => {
      NavigationHistory::navigate("/settings", cx);
      Ok(())
    }
    CommandPaletteAction::OpenBillingPage => {
      crate::billing_dialog::open_billing_dialog(window, cx);
      Ok(())
    }
    CommandPaletteAction::OpenAboutPage => {
      crate::about_dialog::open_about_dialog(window, cx);
      Ok(())
    }
    CommandPaletteAction::OpenGithubPrDetails {
      owner,
      repo,
      number,
      open_changes_tab,
      review_comment_id,
    } => {
      open_pull_request_target(
        owner,
        repo,
        number,
        open_changes_tab,
        review_comment_id,
        PullRequestFallback::OpenBrowser,
        cx,
      );
      Ok(())
    }
    CommandPaletteAction::OpenGithubRepoDetails {
      owner,
      repo,
      tab,
      issue_number,
      issue_comment_id,
    } => {
      open_repo_target(owner, repo, tab, issue_number, issue_comment_id, cx);
      Ok(())
    }
    CommandPaletteAction::OpenGithubCommitDetails { owner, repo, sha } => {
      open_commit_target(owner, repo, sha, cx);
      Ok(())
    }
    CommandPaletteAction::OpenGithubProfile { login } => {
      open_profile_target(login, cx);
      Ok(())
    }
    CommandPaletteAction::SendFeedback => {
      crate::feedback_dialog::open_feedback_dialog(window, cx);
      Ok(())
    }
    CommandPaletteAction::SignIn => {
      crate::auth_flow::start_github_sign_in(cx, "command_palette");
      Ok(())
    }
    CommandPaletteAction::SignOut => {
      crate::auth_flow::sign_out(cx);
      crate::github_notifications::GithubNotificationsStore::clear(cx);
      Ok(())
    }
    CommandPaletteAction::OpenBrowserExtensions => {
      crate::browser_extensions_dialog::open_browser_extensions_dialog(window, cx);
      Ok(())
    }
    _ => Err("Command not available.".into()),
  }
}
