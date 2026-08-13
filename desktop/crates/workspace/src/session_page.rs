//! Agent-first shell: sessions sidebar, conversation center, review panel.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use agent_chat_panel::{AgentChatPanel, AgentChatPanelEvent, ConversationMeta};
use editor::{
  DiffViewMode, Editor, ReviewComment, ReviewCommentCreateHandler, ReviewCommentCreateRequest,
  ReviewCommentDeleteHandler, ReviewCommentDisplayMode, ReviewCommentEditHandler,
  ReviewCommentSide,
};
use gfm_markdown_viewer::SuggestionContext;
use gpui::{
  AnyElement, App, Context, Entity, FocusHandle, Focusable, Render, SharedString, Task, Window,
  div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Sizable as _, h_flex, notification::Notification, v_flex,
};
use smol::unblock;

use crate::agent_review::{
  LocalAgentReviewComment, LocalAgentReviewCommentState, agent_review_comment_is_copyable,
  agent_review_line_label, format_agent_review_export, next_agent_review_comment_state,
};
use crate::agent_settings::AgentSettings;
use crate::auth_state::AuthStateStore;
use crate::config::ConfigStore;
use crate::git_page::{
  agent_chat_state_dir, agent_path_to_repo_relative, prune_agent_chat_state_once,
};
use crate::github_navigation::{
  open_commit_target, open_pr_target, open_profile_target, open_repo_target,
};
use crate::navigation::NavigationHistory;
use crate::review_panel::{ReviewPanel, ReviewPanelEvent};
use crate::workspace::WorkspaceApi;
use crate::{CloseWorkspacePage, CommentHunk, SendReviewCommentsToAgent, ShowCommandPalette};
use ui::{
  Button, ButtonVariants as _, CommandPalette, CommandPaletteAction, CommandPaletteCommand,
  CommandPaletteConfig, CommandPaletteHandler, CommandPalettePage, UiIconName, WindowExt as _,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CenterView {
  Conversation,
  Diff,
}

pub struct SessionPage {
  focus_handle: FocusHandle,
  agent_chat_view: Option<Entity<AgentChatPanel>>,
  review_panel: Entity<ReviewPanel>,
  selected_repo: Option<PathBuf>,
  center: CenterView,
  editor: Option<Entity<Editor>>,
  selected_file: Option<PathBuf>,
  open_file_generation: u64,
  open_file_task: Option<Task<()>>,
  agent_review_comments: Vec<LocalAgentReviewComment>,
  next_agent_review_comment_id: u64,
}

impl SessionPage {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let selected_repo = ConfigStore::load_recent_repositories()
      .first()
      .map(|repo| repo.path.clone());
    let review_panel = cx.new(|cx| ReviewPanel::new(selected_repo.clone(), window, cx));
    cx.subscribe_in(
      &review_panel,
      window,
      |this, _panel, event: &ReviewPanelEvent, window, cx| match event {
        ReviewPanelEvent::OpenFile { path } => {
          this.open_diff(path.clone(), None, window, cx);
        }
      },
    )
    .detach();

    Self {
      focus_handle: cx.focus_handle(),
      agent_chat_view: None,
      review_panel,
      selected_repo,
      center: CenterView::Conversation,
      editor: None,
      selected_file: None,
      open_file_generation: 0,
      open_file_task: None,
      agent_review_comments: Vec::new(),
      next_agent_review_comment_id: 1,
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
    cx.subscribe_in(
      &view,
      window,
      |this, _panel, event: &AgentChatPanelEvent, window, cx| match event {
        AgentChatPanelEvent::OpenPath { path, line } => {
          let rel_path = agent_path_to_repo_relative(path.clone(), this.selected_repo.as_deref());
          this.open_diff(rel_path, *line, window, cx);
        }
        AgentChatPanelEvent::TurnFinished => {
          this.review_panel.update(cx, |panel, cx| panel.refresh(cx));
          if let Some(editor) = this.editor.clone() {
            editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
          }
          this.sync_agent_review_comments_to_editor(cx);
        }
      },
    )
    .detach();
    self.agent_chat_view = Some(view);
  }

  fn open_diff(
    &mut self,
    rel_path: PathBuf,
    reveal_line: Option<u32>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let app_settings = crate::config::AppSettings::get(cx);
    let diff_view = if app_settings.split_diff_view {
      DiffViewMode::Split
    } else {
      DiffViewMode::Inline
    };
    let hide_whitespace = app_settings.hide_whitespace;
    // Agent line numbers are 1-based; the editor reveals by 0-based doc line.
    let reveal_doc_line = reveal_line.map(|line| line.saturating_sub(1) as usize);

    self.center = CenterView::Diff;
    if self.selected_file.as_ref() == Some(&rel_path) && self.editor.is_some() {
      if let (Some(doc_line), Some(editor)) = (reveal_doc_line, self.editor.clone()) {
        editor.update(cx, |editor, cx| editor.reveal_source_line(doc_line, cx));
      }
      self.focus_editor_on_next_frame(window, cx);
      cx.notify();
      return;
    }

    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    let generation = self.open_file_generation;
    self.selected_file = Some(rel_path.clone());
    self.editor = None;

    let file_path = repo_root.join(&rel_path);
    let load_repo_root = repo_root.clone();
    let load_file_path = file_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let loaded =
        unblock(move || Editor::load_file_for_editor(&load_repo_root, &load_file_path)).await;
      let _ = this.update(cx, move |this, cx| {
        if this.open_file_generation != generation {
          return;
        }
        if this.selected_file.as_ref() != Some(&rel_path) {
          return;
        }
        let editor =
          cx.new(move |cx| Editor::new_with_loaded_file(repo_root, file_path, loaded, cx));
        editor.update(cx, |editor, cx| {
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_whitespace, cx);
          if let Some(doc_line) = reveal_doc_line {
            editor.reveal_source_line(doc_line, cx);
          }
        });
        this.editor = Some(editor.clone());
        this.install_agent_review_handlers_for_editor(&editor, cx);
        this.sync_agent_review_comments_to_editor(cx);
        cx.notify();
      });
    });
    self.open_file_task = Some(task);
    self.focus_editor_on_next_frame(window, cx);
    cx.notify();
  }

  fn close_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.center != CenterView::Diff {
      return;
    }
    self.center = CenterView::Conversation;
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  fn focus_editor_on_next_frame(&self, window: &mut Window, cx: &mut Context<Self>) {
    let view = cx.entity().downgrade();
    window.on_next_frame(move |window, cx| {
      let _ = view.update(cx, |this, cx| {
        if let Some(editor) = this.editor.as_ref() {
          let focus_handle = editor.read(cx).focus_handle(cx);
          window.focus(&focus_handle, cx);
        }
      });
    });
  }

  fn install_agent_review_handlers_for_editor(
    &mut self,
    editor: &Entity<Editor>,
    cx: &mut Context<Self>,
  ) {
    let view = cx.entity().downgrade();
    editor.update(cx, |editor, cx| {
      let create_handler: ReviewCommentCreateHandler = Arc::new({
        let view = view.clone();
        move |request, window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.create_agent_review_comment(request, window, cx);
            });
          });
        }
      });
      editor.set_review_comment_create_handler(Some(create_handler), cx);
      editor.set_review_comment_replies_enabled(false, cx);
      editor.set_review_comment_display_mode(ReviewCommentDisplayMode::LocalNote, cx);

      let edit_handler: ReviewCommentEditHandler = Arc::new({
        let view = view.clone();
        move |comment_id, body, window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |_window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.update_agent_review_comment(comment_id, body, cx);
            });
          });
        }
      });
      editor.set_review_comment_edit_handler(Some(edit_handler), cx);

      let delete_handler: ReviewCommentDeleteHandler = Arc::new({
        let view = view.clone();
        move |comment_id, window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |_window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.delete_agent_review_comment(comment_id, cx);
            });
          });
        }
      });
      editor.set_review_comment_delete_handler(Some(delete_handler), cx);
    });
  }

  fn agent_review_original_lines_for_request(
    &self,
    request: &ReviewCommentCreateRequest,
    cx: &App,
  ) -> (Option<usize>, Vec<String>) {
    if request.side != ReviewCommentSide::Right {
      return (None, Vec::new());
    }

    let Some(editor) = self.editor.as_ref() else {
      return (None, Vec::new());
    };

    let start = request.start_line.unwrap_or(request.line).min(request.line);
    let end = request.start_line.unwrap_or(request.line).max(request.line);
    let document = editor.read(cx).document().clone();
    let document = document.read(cx);
    let mut lines = Vec::new();

    for line_ix in start..=end {
      let Some(line) = document.line_content(line_ix) else {
        continue;
      };
      lines.push(line.trim_end_matches(['\r', '\n']).to_string());
    }

    if lines.is_empty() {
      (None, Vec::new())
    } else {
      (Some(start.saturating_add(1)), lines)
    }
  }

  fn create_agent_review_comment(
    &mut self,
    request: ReviewCommentCreateRequest,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let parent = request.in_reply_to_id.and_then(|parent_id| {
      self
        .agent_review_comments
        .iter()
        .find(|comment| comment.id == parent_id)
        .cloned()
    });
    let Some(path) = parent
      .as_ref()
      .map(|comment| comment.path.clone())
      .or_else(|| self.selected_file.clone())
    else {
      self.finish_agent_review_create(Some(Arc::from("No selected file")), cx);
      return;
    };

    let (original_start_line, original_lines) = if request.in_reply_to_id.is_some() {
      (None, Vec::new())
    } else {
      self.agent_review_original_lines_for_request(&request, cx)
    };

    let id = self.next_agent_review_comment_id;
    self.next_agent_review_comment_id = self.next_agent_review_comment_id.saturating_add(1);
    self.agent_review_comments.push(LocalAgentReviewComment {
      id,
      in_reply_to_id: request.in_reply_to_id,
      path,
      line: request.line,
      side: request.side,
      start_line: request.start_line,
      start_side: request.start_side,
      body: request.body.clone(),
      original_start_line,
      original_lines,
      state: LocalAgentReviewCommentState::Draft,
    });
    self.sync_agent_review_comments_to_editor(cx);
    self.finish_agent_review_create(None, cx);
    cx.notify();
  }

  fn finish_agent_review_create(&mut self, error: Option<Arc<str>>, cx: &mut Context<Self>) {
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.finish_review_comment_create_submission(error, cx);
      });
    }
  }

  fn update_agent_review_comment(&mut self, comment_id: u64, body: Arc<str>, cx: &mut Context<Self>) {
    if let Some(comment) = self
      .agent_review_comments
      .iter_mut()
      .find(|comment| comment.id == comment_id)
    {
      comment.body = body;
      self.sync_agent_review_comments_to_editor(cx);
      if let Some(editor) = self.editor.clone() {
        editor.update(cx, |editor, cx| {
          editor.finish_review_comment_edit_submission(comment_id, None, cx);
        });
      }
      cx.notify();
    }
  }

  fn delete_agent_review_comment(&mut self, comment_id: u64, cx: &mut Context<Self>) {
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.start_review_comment_delete_submission(comment_id, cx);
      });
    }

    self
      .agent_review_comments
      .retain(|comment| comment.id != comment_id && comment.in_reply_to_id != Some(comment_id));
    self.sync_agent_review_comments_to_editor(cx);
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.finish_review_comment_delete_submission(comment_id, cx);
      });
    }
    cx.notify();
  }

  fn local_agent_review_comment_to_editor_comment(
    comment: &LocalAgentReviewComment,
  ) -> ReviewComment {
    let line_label = Some(Arc::<str>::from(agent_review_line_label(comment)));
    let suggestion_context = if comment.original_lines.is_empty() {
      None
    } else {
      Some(SuggestionContext {
        original_start_line: comment.original_start_line,
        suggested_start_line: comment.original_start_line,
        original_lines: comment.original_lines.clone(),
        path: Arc::from(comment.path.to_string_lossy().as_ref()),
      })
    };

    ReviewComment {
      id: comment.id,
      in_reply_to_id: comment.in_reply_to_id,
      line: comment.line,
      side: comment.side,
      author: Arc::from(""),
      avatar_url: None,
      line_label,
      body: comment.body.clone(),
      suggestion_context,
      created_at: Arc::from(""),
      thread_id: None,
      is_resolved: false,
      is_outdated: matches!(comment.state, LocalAgentReviewCommentState::Outdated),
      viewer_can_resolve: false,
      viewer_can_unresolve: false,
      is_pending: false,
    }
  }

  fn refresh_agent_review_comment_states_for_selected_file(&mut self, cx: &App) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let Some(selected_file) = self.selected_file.clone() else {
      return;
    };

    let current_file_lines = {
      let document = editor.read(cx).document().clone();
      let document = document.read(cx);
      (0..document.len_lines())
        .filter_map(|line_ix| {
          document
            .line_content(line_ix)
            .map(|line| line.trim_end_matches(['\r', '\n']).to_string())
        })
        .collect::<Vec<_>>()
    };

    for comment in &mut self.agent_review_comments {
      if comment.path != selected_file {
        continue;
      }
      comment.state = next_agent_review_comment_state(comment, &current_file_lines);
    }
  }

  fn sync_agent_review_comments_to_editor(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let Some(selected_file) = self.selected_file.clone() else {
      editor.update(cx, |editor, cx| {
        editor.set_review_comments(Vec::new(), cx);
        editor.set_editable_review_comment_ids(std::iter::empty::<u64>(), cx);
      });
      return;
    };

    self.refresh_agent_review_comment_states_for_selected_file(cx);

    let comments = self
      .agent_review_comments
      .iter()
      .filter(|comment| comment.path == selected_file)
      .filter(|comment| agent_review_comment_is_copyable(comment))
      .map(Self::local_agent_review_comment_to_editor_comment)
      .collect::<Vec<_>>();
    let editable_ids = comments
      .iter()
      .map(|comment| comment.id)
      .collect::<Vec<_>>();

    editor.update(cx, |editor, cx| {
      editor.set_editable_review_comment_ids(editable_ids, cx);
      editor.set_review_comments(comments, cx);
    });
  }

  fn copyable_review_comment_count(&self) -> usize {
    self
      .agent_review_comments
      .iter()
      .filter(|comment| agent_review_comment_is_copyable(comment))
      .count()
  }

  fn send_agent_review_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.sync_agent_review_comments_to_editor(cx);
    let copyable_ids = self
      .agent_review_comments
      .iter()
      .filter(|comment| agent_review_comment_is_copyable(comment))
      .map(|comment| comment.id)
      .collect::<HashSet<_>>();

    if copyable_ids.is_empty() {
      window.push_notification(Notification::info("No review comments to send"), cx);
      return;
    }

    let review = format_agent_review_export(&self.agent_review_comments);
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };

    let dispatched = panel.update(cx, |panel, cx| {
      if !panel.is_ready() {
        return false;
      }
      panel.send_external_review(review, cx)
    });

    if !dispatched {
      window.push_notification(
        Notification::info("Agent not ready yet. Try again in a moment."),
        cx,
      );
      cx.notify();
      return;
    }

    for comment in &mut self.agent_review_comments {
      if copyable_ids.contains(&comment.id) {
        comment.state = LocalAgentReviewCommentState::Copied;
      }
    }
    self.sync_agent_review_comments_to_editor(cx);
    // Back to the conversation to watch the agent address the comments.
    self.close_diff(window, cx);
    cx.notify();
  }

  fn send_review_comments_to_agent_action(
    &mut self,
    _: &SendReviewCommentsToAgent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.send_agent_review_to_agent(window, cx);
    cx.stop_propagation();
  }

  fn comment_hunk_action(&mut self, _: &CommentHunk, window: &mut Window, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    if editor.update(cx, |editor, cx| {
      editor.start_review_comment_for_active_hunk(window, cx)
    }) {
      cx.stop_propagation();
    }
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
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.center == CenterView::Diff {
      self.close_diff(window, cx);
      return;
    }
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
    match self.center {
      CenterView::Conversation => self.render_conversation(cx),
      CenterView::Diff => self.render_diff_view(cx),
    }
  }

  fn render_conversation(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let mut container = div().size_full().min_w(px(0.0)).min_h_0().bg(theme.background);
    if let Some(view) = self.agent_chat_view.clone() {
      container = container.child(view);
    }
    container.into_any_element()
  }

  fn render_diff_header(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let copyable_count = self.copyable_review_comment_count();
    let (dir, file) = self
      .selected_file
      .as_deref()
      .map(crate::review_panel::split_path_label)
      .unwrap_or_default();

    h_flex()
      .items_center()
      .gap_3()
      .px_3()
      .py_2()
      .border_b_1()
      .border_color(theme.border)
      .child(
        Button::new("session-page-diff-back")
          .label("Chat")
          .icon(UiIconName::MessageCircle)
          .ghost()
          .compact()
          .small()
          .tooltip("Back to the conversation (Esc)")
          .on_click(cx.listener(|this, _, window, cx| this.close_diff(window, cx))),
      )
      .child(
        h_flex()
          .flex_1()
          .min_w(px(0.0))
          .overflow_hidden()
          .text_sm()
          .whitespace_nowrap()
          .when(!dir.is_empty(), |this| {
            this.child(
              div()
                .text_color(theme.muted_foreground)
                .truncate()
                .child(dir),
            )
          })
          .child(div().text_color(theme.foreground).child(file)),
      )
      .when(copyable_count > 0, |this| {
        this.child(
          Button::new("session-page-send-review")
            .primary()
            .compact()
            .small()
            .label(if copyable_count == 1 {
              "Send 1 comment to agent".to_string()
            } else {
              format!("Send {copyable_count} comments to agent")
            })
            .tooltip("Send review comments to the agent (cmd-shift-a)")
            .on_click(cx.listener(|this, _, window, cx| {
              this.send_agent_review_to_agent(window, cx);
            })),
        )
      })
      .into_any_element()
  }

  fn render_diff_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let body: AnyElement = if let Some(editor) = self.editor.clone() {
      div()
        .flex_1()
        .min_h_0()
        .min_w(px(0.0))
        .child(editor)
        .into_any_element()
    } else {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading diff..."),
        )
        .into_any_element()
    };

    v_flex()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.background)
      .child(self.render_diff_header(cx))
      .child(body)
      .into_any_element()
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
      .on_action(cx.listener(Self::send_review_comments_to_agent_action))
      .on_action(cx.listener(Self::comment_hunk_action))
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
    if self.center == CenterView::Diff
      && let Some(editor) = self.editor.as_ref()
    {
      return editor.read(cx).focus_handle(cx);
    }
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
