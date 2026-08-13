//! Agent-first shell: sessions sidebar, conversation center, review panel.

use std::path::PathBuf;
use std::sync::Arc;

use agent_chat_panel::{AgentChatPanel, AgentChatPanelEvent, ConversationMeta};
use gpui::{
  AnyElement, App, Context, Entity, FocusHandle, Focusable, Render, SharedString, Window, div,
  prelude::*, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};

use crate::agent_settings::AgentSettings;
use crate::auth_state::AuthStateStore;
use crate::config::ConfigStore;
use crate::git_page::{agent_chat_state_dir, prune_agent_chat_state_once};
use crate::github_navigation::{
  open_commit_target, open_pr_target, open_profile_target, open_repo_target,
};
use crate::navigation::NavigationHistory;
use crate::review_panel::ReviewPanel;
use crate::workspace::WorkspaceApi;
use crate::{CloseWorkspacePage, ShowCommandPalette};
use ui::{
  Button, ButtonVariants as _, CommandPalette, CommandPaletteAction, CommandPaletteCommand,
  CommandPaletteConfig, CommandPaletteHandler, CommandPalettePage, UiIconName,
};

const SESSIONS_SIDEBAR_DEFAULT_WIDTH: f32 = 250.0;
const SESSIONS_SIDEBAR_MIN_WIDTH: f32 = 200.0;
const SESSIONS_SIDEBAR_MAX_WIDTH: f32 = 420.0;
const REVIEW_PANEL_DEFAULT_WIDTH: f32 = 320.0;
const REVIEW_PANEL_MIN_WIDTH: f32 = 240.0;
const REVIEW_PANEL_MAX_WIDTH: f32 = 560.0;

pub(crate) fn format_relative_secs(updated_at_secs: u64, now_secs: u64) -> String {
  let delta = now_secs.saturating_sub(updated_at_secs);
  match delta {
    0..=59 => "now".to_string(),
    60..=3_599 => format!("{}m", delta / 60),
    3_600..=86_399 => format!("{}h", delta / 3_600),
    _ => format!("{}d", delta / 86_400),
  }
}

fn now_secs() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

pub(crate) fn session_row_title(meta: &ConversationMeta) -> SharedString {
  let trimmed = meta.title.trim();
  if trimmed.is_empty() {
    "New session".into()
  } else {
    trimmed.to_string().into()
  }
}

pub struct SessionPage {
  focus_handle: FocusHandle,
  agent_chat_view: Option<Entity<AgentChatPanel>>,
  review_panel: Entity<ReviewPanel>,
  selected_repo: Option<PathBuf>,
}

impl SessionPage {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let selected_repo = ConfigStore::load_recent_repositories()
      .first()
      .map(|repo| repo.path.clone());
    let review_panel = cx.new(|cx| ReviewPanel::new(selected_repo.clone(), window, cx));

    Self {
      focus_handle: cx.focus_handle(),
      agent_chat_view: None,
      review_panel,
      selected_repo,
    }
  }

  fn ensure_agent_chat_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(view) = self.agent_chat_view.as_ref()
      && view.read(cx).needs_reconnect()
    {
      self.agent_chat_view = None;
    }
    if self.agent_chat_view.is_some() {
      return;
    }
    prune_agent_chat_state_once();
    let cwd = self
      .selected_repo
      .clone()
      .unwrap_or_else(|| PathBuf::from("."));
    let state_dir =
      agent_chat_state_dir().map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &cwd));
    let backend = AgentSettings::load();
    let view = cx.new(|cx| AgentChatPanel::new(backend, cwd, state_dir, window, cx));
    // Sidebar reads conversation state from the panel; re-render when it changes.
    cx.observe(&view, |_, _, cx| cx.notify()).detach();
    cx.subscribe(&view, |this, _panel, event: &AgentChatPanelEvent, cx| {
      match event {
        // Diff view arrives in P2; ignore tool-call path clicks for now.
        AgentChatPanelEvent::OpenPath { .. } => {}
        AgentChatPanelEvent::TurnFinished => {
          this.review_panel.update(cx, |panel, cx| panel.refresh(cx));
        }
      }
    })
    .detach();
    self.agent_chat_view = Some(view);
  }

  fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    panel.update(cx, |panel, cx| panel.new_conversation(cx));
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  fn select_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    if panel.read(cx).current_conversation().id == id {
      return;
    }
    panel.update(cx, |panel, cx| panel.load_conversation(id, cx));
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  fn focus_agent_input_on_next_frame(&self, window: &mut Window, _cx: &mut Context<Self>) {
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    window.on_next_frame(move |window, cx| {
      let focus_handle = panel.read(cx).input_focus_handle(cx);
      window.focus(&focus_handle, cx);
    });
  }

  fn close_workspace_page_action(
    &mut self,
    _: &CloseWorkspacePage,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    NavigationHistory::navigate_back(cx);
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let include_github = AuthStateStore::has_github_access(cx);
    let commands =
      CommandPaletteCommand::default_global_commands(CommandPalettePage::Git, include_github);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let config = CommandPaletteConfig::new(Vec::new(), commands, handler);
    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    ui::open_palette_dialog(palette, window, cx);
  }

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match action {
      CommandPaletteAction::OpenGitPage => {
        NavigationHistory::navigate("/git", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPage => {
        crate::github_page::GithubPageHandle::refresh(cx);
        NavigationHistory::navigate("/github", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
        open_changes_tab,
        review_comment_id,
      } => {
        open_pr_target(owner, repo, number, open_changes_tab, review_comment_id, cx);
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
      CommandPaletteAction::OpenSettingsPage => {
        NavigationHistory::navigate("/settings", cx);
        Ok(())
      }
      CommandPaletteAction::OpenBillingPage => {
        NavigationHistory::navigate("/billing", cx);
        Ok(())
      }
      CommandPaletteAction::OpenAboutPage => {
        NavigationHistory::navigate("/about", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGitConfigPage => {
        NavigationHistory::navigate("/git-config", cx);
        Ok(())
      }
      CommandPaletteAction::SendFeedback => {
        crate::feedback_dialog::open_feedback_dialog(window, cx);
        Ok(())
      }
      CommandPaletteAction::SearchGithubRepository => {
        let api = WorkspaceApi::global(cx).api.clone();
        crate::github_search_dialog::open_github_search_dialog(api, window, cx);
        Ok(())
      }
      CommandPaletteAction::CreateGithubRepository => {
        let api = WorkspaceApi::global(cx).api.clone();
        crate::github_create_repository_dialog::open_create_repository_dialog(api, window, cx);
        Ok(())
      }
      _ => Err("Command not available.".into()),
    }
  }

  fn render_sessions_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let (conversations, current_id) = match self.agent_chat_view.as_ref() {
      Some(panel) => {
        let panel = panel.read(cx);
        (
          panel.list_conversations(),
          panel.current_conversation().id.clone(),
        )
      }
      None => (Vec::new(), String::new()),
    };
    let now = now_secs();

    let header = h_flex()
      .items_center()
      .justify_between()
      .px_3()
      .py_2()
      .child(
        div()
          .text_xs()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.muted_foreground)
          .child("Sessions"),
      )
      .child(
        Button::new("session-page-new-session")
          .icon(UiIconName::SquarePen)
          .ghost()
          .compact()
          .small()
          .tooltip("New session")
          .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx))),
      );

    let rows = conversations.into_iter().enumerate().map(|(ix, meta)| {
      let is_current = meta.id == current_id;
      let id = meta.id.clone();
      let title = session_row_title(&meta);
      let time = format_relative_secs(meta.updated_at_secs, now);

      div()
        .id(("session-page-session-row", ix))
        .mx_2()
        .px_2()
        .py_1p5()
        .rounded(px(6.0))
        .cursor_pointer()
        .when(is_current, |this| this.bg(theme.secondary_active))
        .hover(|s| s.bg(theme.secondary_hover))
        .on_click(cx.listener(move |this, _, window, cx| {
          this.select_session(&id, window, cx);
        }))
        .child(
          h_flex()
            .items_center()
            .gap_2()
            .child(
              div()
                .flex_1()
                .min_w(px(0.0))
                .text_sm()
                .truncate()
                .text_color(theme.foreground)
                .child(title),
            )
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(time),
            ),
        )
    });

    v_flex()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.sidebar)
      .border_r_1()
      .border_color(theme.border)
      .child(header)
      .child(
        div()
          .id("session-page-session-list")
          .flex_1()
          .min_h_0()
          .overflow_y_scroll()
          .py_1()
          .children(rows),
      )
      .into_any_element()
  }

  fn render_center(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let mut container = div().size_full().min_w(px(0.0)).min_h_0().bg(theme.background);
    if let Some(view) = self.agent_chat_view.clone() {
      container = container.child(view);
    }
    container.into_any_element()
  }

  fn render_review_panel(&mut self, _cx: &mut Context<Self>) -> AnyElement {
    div()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .child(self.review_panel.clone())
      .into_any_element()
  }
}

impl Render for SessionPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.ensure_agent_chat_view(window, cx);

    div()
      .size_full()
      .min_h_0()
      .track_focus(&self.focus_handle)
      .on_action(cx.listener(Self::close_workspace_page_action))
      .on_action(cx.listener(Self::show_command_palette_action))
      .child(
        ui::h_resizable("session-page-shell")
          .child(
            ui::resizable_panel()
              .size(px(SESSIONS_SIDEBAR_DEFAULT_WIDTH))
              .size_range(px(SESSIONS_SIDEBAR_MIN_WIDTH)..px(SESSIONS_SIDEBAR_MAX_WIDTH))
              .child(self.render_sessions_sidebar(cx)),
          )
          .child(ui::resizable_panel().child(self.render_center(cx)))
          .child(
            ui::resizable_panel()
              .size(px(REVIEW_PANEL_DEFAULT_WIDTH))
              .size_range(px(REVIEW_PANEL_MIN_WIDTH)..px(REVIEW_PANEL_MAX_WIDTH))
              .child(self.render_review_panel(cx)),
          ),
      )
  }
}

impl Focusable for SessionPage {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    if let Some(view) = self.agent_chat_view.as_ref() {
      return view.read(cx).input_focus_handle(cx);
    }
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn meta_with_title(title: &str) -> ConversationMeta {
    ConversationMeta {
      id: "1".to_string(),
      started_at_secs: 0,
      updated_at_secs: 0,
      title: title.to_string(),
      message_count: 0,
      session_id: None,
    }
  }

  #[test]
  fn format_relative_secs_buckets() {
    assert_eq!(format_relative_secs(100, 100), "now");
    assert_eq!(format_relative_secs(100, 159), "now");
    assert_eq!(format_relative_secs(100, 160), "1m");
    assert_eq!(format_relative_secs(100, 100 + 3_600), "1h");
    assert_eq!(format_relative_secs(100, 100 + 86_400), "1d");
    assert_eq!(format_relative_secs(100, 100 + 3 * 86_400), "3d");
  }

  #[test]
  fn format_relative_secs_clamps_future_timestamps() {
    assert_eq!(format_relative_secs(200, 100), "now");
  }

  #[test]
  fn session_row_title_falls_back_when_empty() {
    assert_eq!(session_row_title(&meta_with_title("")), "New session");
    assert_eq!(session_row_title(&meta_with_title("   ")), "New session");
    assert_eq!(session_row_title(&meta_with_title("Fix scroll")), "Fix scroll");
  }
}
