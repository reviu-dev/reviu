//! Agent-first shell: sessions sidebar, conversation center, review panel.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_chat_panel::{AgentChatPanel, AgentChatPanelEvent, ConversationMeta};
use editor::{
  DiffViewMode, Editor, EditorEvent, ReviewCommentCreateHandler, ReviewCommentCreateRequest,
  ReviewCommentDeleteHandler, ReviewCommentDisplayMode, ReviewCommentEditHandler,
};
use gpui::AnimationExt as _;
use gpui::{
  AnyElement, AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, PathPromptOptions,
  Render, SharedString, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, notification::Notification,
  v_flex,
};
use smol::unblock;

use crate::agent_review::{
  AgentReviewComments, original_lines_for_request, sync_comments_to_editor,
};
use crate::agent_settings::AgentSettings;
use crate::auth_state::AuthStateStore;
use crate::config::ConfigStore;
use crate::date_format::format_relative_time;
use crate::file_search_palette::open_file_search_palette;
use crate::file_view::{
  BinaryPreview, build_binary_preview, render_binary_preview, render_file_title,
};
use crate::git_page::{
  agent_chat_state_dir, agent_path_to_repo_relative, prune_agent_chat_state_once,
};
use crate::github_notifications::{self, GithubNotificationsStore};
use crate::navigation::NavigationHistory;
use crate::review_panel::{ReviewPanel, ReviewPanelEvent};
use crate::{
  CloseWorkspacePage, CommentHunk, SendReviewCommentsToAgent, ShowCommandPalette, ShowFileSearch,
};
use ui::{
  Button, ButtonVariants as _, CommandPalette, CommandPaletteAction, CommandPaletteCommand,
  CommandPaletteConfig, CommandPaletteHandler, CommandPaletteInitialScreen, CommandPalettePage,
  CommandPaletteRepository, SearchFileEntry, SearchFileHandler, StatusThemeExt as _, UiIconName,
  WindowExt as _,
};

const REPO_CONTEXT_DEBUG_SELECTOR: &str = "session-repo-context";
const REPO_AHEAD_DEBUG_SELECTOR: &str = "session-repo-ahead";
const REPO_BEHIND_DEBUG_SELECTOR: &str = "session-repo-behind";

const SESSIONS_SIDEBAR_DEFAULT_WIDTH: f32 = 250.0;
const SESSIONS_SIDEBAR_MIN_WIDTH: f32 = 200.0;
const SESSIONS_SIDEBAR_MAX_WIDTH: f32 = 420.0;
const INBOX_MAX_HEIGHT: f32 = 220.0;
const CENTER_SWAP_FADE_MS: u64 = 180;
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

/// The repo-wide git commands the sessions palette runs off the main thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepoCommand {
  StageAll,
  UnstageAll,
  Push,
  Pull,
  Fetch,
}

impl RepoCommand {
  fn run(self, repo_root: &Path) -> anyhow::Result<SharedString> {
    match self {
      Self::StageAll => git::stage_all(repo_root).map(|()| "Staged all changes".into()),
      Self::UnstageAll => git::unstage_all(repo_root).map(|()| "Unstaged all changes".into()),
      Self::Push => git::push(repo_root, false).map(|()| "Pushed to the remote branch".into()),
      Self::Pull => git::pull(repo_root).map(|outcome| match outcome {
        git::PullOutcome::AlreadyUpToDate => "Already up to date".into(),
        git::PullOutcome::Pulled => "Pulled from the remote branch".into(),
      }),
      Self::Fetch => git::fetch(repo_root).map(|()| "Fetched from remotes".into()),
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CenterView {
  Conversation,
  Diff,
}

/// Global entry point so other pages (git page review comments, selections)
/// can route work into the sessions shell.
pub(crate) struct SessionPageHandle {
  page: Option<gpui::WeakEntity<SessionPage>>,
}

impl gpui::Global for SessionPageHandle {}

impl SessionPageHandle {
  pub fn register(cx: &mut Context<SessionPage>) {
    cx.set_global(Self {
      page: Some(cx.entity().downgrade()),
    });
  }

  fn with_page(
    cx: &mut App,
    f: impl FnOnce(&mut SessionPage, &mut Window, &mut Context<SessionPage>),
  ) {
    let Some(page) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.page.clone())
      .and_then(|weak| weak.upgrade())
    else {
      return;
    };
    let window_handle = page.read(cx).window_handle;
    let _ = cx.update_window(window_handle, |_, window, cx| {
      page.update(cx, |page, cx| f(page, window, cx));
    });
  }

  /// Navigate to the sessions shell and send a review-comment batch to the agent.
  pub fn send_review(text: String, cx: &mut App) {
    NavigationHistory::navigate("/session", cx);
    Self::with_page(cx, move |page, window, cx| {
      page.deliver_review_export(text, window, cx);
    });
  }

  /// Navigate to the sessions shell and attach a code selection as agent context.
  pub fn add_selection(path: String, text: String, cx: &mut App) {
    NavigationHistory::navigate("/session", cx);
    Self::with_page(cx, move |page, window, cx| {
      page.deliver_selection_context(path, text, window, cx);
    });
  }
}

pub struct SessionPage {
  focus_handle: FocusHandle,
  window_handle: AnyWindowHandle,
  agent_chat_view: Option<Entity<AgentChatPanel>>,
  review_panel: Entity<ReviewPanel>,
  selected_repo: Option<PathBuf>,
  center: CenterView,
  editor: Option<Entity<Editor>>,
  binary_preview: Option<BinaryPreview>,
  selected_file: Option<PathBuf>,
  open_file_generation: u64,
  open_file_task: Option<Task<()>>,
  agent_review: AgentReviewComments,
  branch_status: Option<git::BranchStatus>,
  // Review export waiting for the agent connection to become ready.
  pending_review_export: Option<String>,
  repo_command_in_flight: bool,
  _branch_task: Option<Task<()>>,
  _repo_command_task: Option<Task<()>>,
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
        ReviewPanelEvent::Committed => {
          this.refresh_branch(cx);
        }
      },
    )
    .detach();

    let mut page = Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      agent_chat_view: None,
      review_panel,
      selected_repo,
      center: CenterView::Conversation,
      editor: None,
      binary_preview: None,
      selected_file: None,
      open_file_generation: 0,
      open_file_task: None,
      agent_review: AgentReviewComments::new(),
      branch_status: None,
      pending_review_export: None,
      repo_command_in_flight: false,
      _branch_task: None,
      _repo_command_task: None,
    };
    SessionPageHandle::register(cx);
    page.refresh_branch(cx);
    page
  }

  /// Connects the agent. Called when the workspace routes to the shell, never
  /// from `render`: spawning a process while painting respawned it in a loop.
  pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.agent_chat_view.is_some() {
      return;
    }
    self.ensure_agent_chat_view(window, cx);
    cx.notify();
  }

  fn deliver_review_export(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
    self.ensure_agent_chat_view(window, cx);
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    let sent = panel.update(cx, |panel, cx| {
      panel.is_ready() && panel.send_external_review(text.clone(), cx)
    });
    if !sent {
      self.pending_review_export = Some(text);
    }
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  fn deliver_selection_context(
    &mut self,
    path: String,
    text: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.ensure_agent_chat_view(window, cx);
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    panel.update(cx, |panel, cx| {
      panel.add_selection_context(path, text, window, cx);
    });
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  fn flush_pending_review_export(&mut self, cx: &mut Context<Self>) {
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    if self.pending_review_export.is_none() || !panel.read(cx).is_ready() {
      return;
    }
    if let Some(text) = self.pending_review_export.take() {
      panel.update(cx, |panel, cx| {
        panel.send_external_review(text, cx);
      });
    }
  }

  fn refresh_branch(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let task = cx.spawn(async move |this, cx| {
      let status = unblock(move || git::current_branch_status(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.branch_status = status.ok();
        cx.notify();
      });
    });
    self._branch_task = Some(task);
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
    // The sessions sidebar owns the conversation list; hide the panel's own controls.
    view.update(cx, |panel, _| {
      panel.set_conversation_controls_visible(false)
    });
    // Sidebar reads conversation state from the panel; re-render when it changes.
    // Also the flush point for a review export queued while the agent was connecting.
    cx.observe(&view, |this, _, cx| {
      this.flush_pending_review_export(cx);
      cx.notify();
    })
    .detach();
    cx.subscribe_in(
      &view,
      window,
      |this, _panel, event: &AgentChatPanelEvent, window, cx| match event {
        AgentChatPanelEvent::OpenPath { path, line } => {
          let rel_path = agent_path_to_repo_relative(path.clone(), this.selected_repo.as_deref());
          this.open_diff(rel_path, *line, window, cx);
        }
        AgentChatPanelEvent::TurnStarted => {
          this.create_turn_checkpoint(cx);
        }
        AgentChatPanelEvent::TurnFinished => {
          this.review_panel.update(cx, |panel, cx| panel.refresh(cx));
          if let Some(editor) = this.editor.clone() {
            editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
          }
          this.sync_agent_review_comments_to_editor(cx);
          this.refresh_branch(cx);
        }
        AgentChatPanelEvent::RollbackRequested { ref_name } => {
          this.rollback_to_checkpoint(ref_name.clone(), window, cx);
        }
      },
    )
    .detach();
    self.agent_chat_view = Some(view);
  }

  fn create_turn_checkpoint(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    let session_id = panel.read(cx).current_conversation().id.clone();

    cx.spawn(async move |this, cx| {
      let result = unblock(move || git::create_checkpoint(&repo_root, &session_id)).await;
      let Ok(checkpoint) = result else {
        return;
      };
      let _ = this.update(cx, |this, cx| {
        if let Some(panel) = this.agent_chat_view.clone() {
          panel.update(cx, |panel, cx| {
            panel.record_checkpoint(checkpoint.ref_name, cx);
          });
        }
      });
    })
    .detach();
  }

  fn rollback_to_checkpoint(
    &mut self,
    ref_name: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    if panel.read(cx).is_turn_in_flight() {
      window.push_notification(
        Notification::info("Wait for the agent to finish before rolling back"),
        cx,
      );
      return;
    }
    let session_id = panel.read(cx).current_conversation().id.clone();

    cx.spawn(async move |this, cx| {
      let restore_repo_root = repo_root.clone();
      let restore_ref = ref_name.clone();
      let result = unblock(move || {
        // Safety net: snapshot the current state so the rollback itself is undoable.
        git::create_checkpoint(&restore_repo_root, &session_id)?;
        git::restore_checkpoint(&restore_repo_root, &restore_ref)
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            if let Some(panel) = this.agent_chat_view.clone() {
              panel.update(cx, |panel, cx| {
                panel.truncate_at_checkpoint(&ref_name, cx);
              });
            }
            this.review_panel.update(cx, |panel, cx| panel.refresh(cx));
            if let Some(editor) = this.editor.clone() {
              editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
            }
            this.sync_agent_review_comments_to_editor(cx);
          }
          Err(error) => {
            let _ = cx.update_window(this.window_handle, |_, window, cx| {
              window
                .push_notification(Notification::error(format!("Rollback failed: {error}")), cx);
            });
          }
        }
        cx.notify();
      });
    })
    .detach();
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
    self.binary_preview = None;

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
        let binary_preview = build_binary_preview(rel_path.as_path(), loaded.binary_bytes.clone());
        let editor =
          cx.new(move |cx| Editor::new_with_loaded_file(repo_root, file_path, loaded, cx));
        editor.update(cx, |editor, cx| {
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_whitespace, cx);
          if let Some(doc_line) = reveal_doc_line {
            editor.reveal_source_line(doc_line, cx);
          }
        });
        this.binary_preview = binary_preview;
        this.editor = Some(editor.clone());
        // Focus once loaded: the requester (file tree, list, search) may still hold
        // focus, and there was no editor to focus when the open was requested.
        if this.center == CenterView::Diff {
          let _ = cx.update_window(this.window_handle, |_, window, cx| {
            let focus_handle = editor.read(cx).focus_handle(cx);
            window.focus(&focus_handle, cx);
          });
        }
        this.install_agent_review_handlers_for_editor(&editor, cx);
        this.sync_agent_review_comments_to_editor(cx);
        cx.subscribe(
          &editor,
          |this, _editor, event: &EditorEvent, cx| match event {
            EditorEvent::Saved => {
              this.review_panel.update(cx, |panel, cx| panel.refresh(cx));
            }
          },
        )
        .detach();
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
          window.on_next_frame(move |_window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.create_agent_review_comment(request, cx);
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

  fn create_agent_review_comment(
    &mut self,
    request: ReviewCommentCreateRequest,
    cx: &mut Context<Self>,
  ) {
    let original = original_lines_for_request(self.editor.as_ref(), &request, cx);
    let created = self
      .agent_review
      .create(&request, self.selected_file.as_deref(), original);

    if let Err(error) = created {
      self.finish_agent_review_create(Some(error), cx);
      return;
    }

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

  fn update_agent_review_comment(
    &mut self,
    comment_id: u64,
    body: Arc<str>,
    cx: &mut Context<Self>,
  ) {
    if !self.agent_review.update(comment_id, body) {
      return;
    }

    self.sync_agent_review_comments_to_editor(cx);
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.finish_review_comment_edit_submission(comment_id, None, cx);
      });
    }
    cx.notify();
  }

  fn delete_agent_review_comment(&mut self, comment_id: u64, cx: &mut Context<Self>) {
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.start_review_comment_delete_submission(comment_id, cx);
      });
    }

    self.agent_review.delete(comment_id);
    self.sync_agent_review_comments_to_editor(cx);

    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.finish_review_comment_delete_submission(comment_id, cx);
      });
    }
    cx.notify();
  }

  fn sync_agent_review_comments_to_editor(&mut self, cx: &mut Context<Self>) {
    let editor = self.editor.clone();
    sync_comments_to_editor(
      &mut self.agent_review,
      editor.as_ref(),
      self.selected_file.as_deref(),
      cx,
    );
  }

  fn copyable_review_comment_count(&self) -> usize {
    self.agent_review.copyable_count()
  }

  fn send_agent_review_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.sync_agent_review_comments_to_editor(cx);

    if self.agent_review.copyable_count() == 0 {
      window.push_notification(Notification::info("No review comments to send"), cx);
      return;
    }

    let review = self.agent_review.export();
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

    self.agent_review.mark_copyable_as_copied();
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
    // Revives the panel when the previous backend connection errored out.
    self.ensure_agent_chat_view(window, cx);
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    panel.update(cx, |panel, cx| panel.new_conversation(cx));
    self.focus_agent_input_on_next_frame(window, cx);
    cx.notify();
  }

  fn delete_session(&mut self, id: &str, cx: &mut Context<Self>) {
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    panel.update(cx, |panel, cx| panel.delete_conversation(id, cx));
    cx.notify();
  }

  fn select_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    self.ensure_agent_chat_view(window, cx);
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

  /// Escape is bound to the editor's CloseFind; it bubbles up here when there was
  /// no find panel to close, which is our cue to close the file view.
  fn close_file_view_action(
    &mut self,
    _: &editor::CloseFind,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.center != CenterView::Diff {
      return;
    }
    self.close_diff(window, cx);
    cx.stop_propagation();
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn show_file_search_action(
    &mut self,
    _: &ShowFileSearch,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_file_search(window, cx);
    cx.stop_propagation();
  }

  fn open_file_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };

    let status_entries: Vec<git::RepoStatusEntry> =
      self.review_panel.read(cx).status_entries().to_vec();
    let mut changed_paths = HashSet::new();
    for entry in &status_entries {
      changed_paths.insert(entry.path.clone());
      if let Some(old_path) = entry.old_path.as_ref() {
        changed_paths.insert(old_path.clone());
      }
    }

    let file_label = |path: &PathBuf| path.to_string_lossy().replace(['\n', '\r'], "");
    let changed = status_entries.iter().map(|entry| {
      SearchFileEntry::new(entry.path.clone(), file_label(&entry.path)).grouped("Changed")
    });
    let unchanged = git::list_repo_head_files(&repo_root)
      .unwrap_or_default()
      .into_iter()
      .filter(|path| !changed_paths.contains(path))
      .map(|path| {
        let label = file_label(&path);
        SearchFileEntry::new(path, label).grouped("Unchanged")
      });
    let entries: Vec<SearchFileEntry> = changed.chain(unchanged).collect();

    let view = cx.entity();
    let handler: SearchFileHandler = Arc::new(move |path, window, cx| {
      view.update(cx, |view, cx| {
        view.open_diff(path, None, window, cx);
      });
      Ok(())
    });
    open_file_search_palette(window, cx, entries, handler, false);
  }

  /// Runs a repo command in the background, then refreshes the changes panel.
  fn run_repo_command(
    &mut self,
    command: RepoCommand,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(repo_root) = self.selected_repo.clone() else {
      return Err("No repository selected.".into());
    };
    if self.repo_command_in_flight {
      return Err("Another git command is still running.".into());
    }

    self.repo_command_in_flight = true;
    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || command.run(&repo_root)).await;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.repo_command_in_flight = false;
          match result {
            Ok(message) => {
              window.push_notification(Notification::success(message), cx);
              this.review_panel.update(cx, |panel, cx| panel.refresh(cx));
              this.refresh_branch(cx);
            }
            Err(error) => {
              window.push_notification(Notification::error(error.to_string()), cx);
            }
          }
          cx.notify();
        });
      });
    });
    self._repo_command_task = Some(task);

    Ok(())
  }

  fn palette_repositories(&self) -> Vec<CommandPaletteRepository> {
    let mut repositories = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| CommandPaletteRepository {
        path: repo.path.to_string_lossy().replace(['\n', '\r'], "").into(),
      })
      .collect::<Vec<_>>();

    if let Some(selected_repo) = self.selected_repo.as_ref() {
      let selected = selected_repo.to_string_lossy().replace(['\n', '\r'], "");
      if !repositories
        .iter()
        .any(|repo| repo.path.as_ref() == selected)
      {
        repositories.insert(
          0,
          CommandPaletteRepository {
            path: selected.into(),
          },
        );
      }
    }

    repositories
  }

  /// A session belongs to a repository: switching swaps the conversation set,
  /// the changes panel and the branch, so the agent is respawned on the new cwd.
  fn set_selected_repo(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    if self.selected_repo.as_deref() == Some(repo_root.as_path()) {
      return Ok(());
    }
    if self.agent_turn_in_flight(cx) {
      return Err("Wait for the agent to finish before switching repository.".into());
    }

    ConfigStore::persist_recent_repository(&repo_root);
    self.apply_selected_repo(Some(repo_root), window, cx);
    Ok(())
  }

  fn apply_selected_repo(
    &mut self,
    repo_root: Option<PathBuf>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.selected_repo = repo_root.clone();
    self.close_diff(window, cx);
    self.center = CenterView::Conversation;
    self.editor = None;
    self.binary_preview = None;
    self.selected_file = None;
    self.open_file_task = None;
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    self.agent_review.clear();
    self.pending_review_export = None;
    self.branch_status = None;
    // Conversations are stored per repository, so the panel is rebuilt on the
    // next render with the new cwd and state directory.
    self.agent_chat_view = None;
    self.review_panel.update(cx, |panel, cx| {
      panel.set_repo_root(repo_root, cx);
      panel.refresh(cx);
    });
    self.refresh_branch(cx);
    cx.notify();
  }

  fn agent_turn_in_flight(&self, cx: &App) -> bool {
    self
      .agent_chat_view
      .as_ref()
      .is_some_and(|panel| panel.read(cx).is_turn_in_flight())
  }

  fn forget_repository(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let forgetting_selected = self.selected_repo.as_deref() == Some(repo_root.as_path());
    if forgetting_selected && self.agent_turn_in_flight(cx) {
      return Err("Wait for the agent to finish before forgetting this repository.".into());
    }

    ConfigStore::forget_recent_repository(&repo_root);
    if !forgetting_selected {
      cx.notify();
      return Ok(());
    }

    let next_repo = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| repo.path)
      .find(|path| path != &repo_root);
    self.apply_selected_repo(next_repo, window, cx);
    Ok(())
  }

  fn start_open_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
      files: false,
      directories: true,
      multiple: false,
      prompt: Some("Select a repository".into()),
    });

    cx.spawn_in(window, async move |this, cx| {
      let Ok(Ok(Some(paths))) = receiver.await else {
        return;
      };
      let Some(path) = paths.into_iter().next() else {
        return;
      };

      let _ = this.update_in(cx, |this, window, cx| {
        if let Err(error) = this.set_selected_repo(path, window, cx) {
          window.push_notification(Notification::warning(error), cx);
        }
      });
    })
    .detach();
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.open_command_palette_with_screen(None, window, cx);
  }

  fn open_command_palette_with_screen(
    &mut self,
    initial_screen: Option<CommandPaletteInitialScreen>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let include_github = AuthStateStore::has_github_access(cx);
    let repositories = self.palette_repositories();
    let mut commands = Vec::new();
    if repositories.len() > 1 {
      commands.push(CommandPaletteCommand::switch_repository());
    }
    commands.push(CommandPaletteCommand::open_repository());
    if !repositories.is_empty() {
      commands.push(CommandPaletteCommand::forget_repository());
    }
    if self.selected_repo.is_some() {
      commands.push(CommandPaletteCommand::commit());
      commands.push(CommandPaletteCommand::stage_all());
      commands.push(CommandPaletteCommand::unstage_all());
      commands.push(CommandPaletteCommand::push("Push"));
      commands.push(CommandPaletteCommand::pull());
      commands.push(CommandPaletteCommand::fetch());
    }
    commands.extend(CommandPaletteCommand::default_global_commands(
      CommandPalettePage::Session,
      include_github,
    ));

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let mut config =
      CommandPaletteConfig::new(Vec::new(), commands, handler).with_repositories(repositories);
    if let Some(initial_screen) = initial_screen {
      config = config.with_initial_screen(initial_screen);
    }
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
      CommandPaletteAction::Commit => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        self.review_panel.update(cx, |panel, cx| panel.commit(cx));
        Ok(())
      }
      CommandPaletteAction::StageAll => self.run_repo_command(RepoCommand::StageAll, window, cx),
      CommandPaletteAction::UnstageAll => {
        self.run_repo_command(RepoCommand::UnstageAll, window, cx)
      }
      CommandPaletteAction::Push => self.run_repo_command(RepoCommand::Push, window, cx),
      CommandPaletteAction::Pull => self.run_repo_command(RepoCommand::Pull, window, cx),
      CommandPaletteAction::Fetch => self.run_repo_command(RepoCommand::Fetch, window, cx),
      CommandPaletteAction::OpenRepository => {
        self.start_open_repository(window, cx);
        Ok(())
      }
      CommandPaletteAction::SwitchRepository(repository) => {
        let repo_root = PathBuf::from(repository.path.as_ref());
        if !repo_root.is_dir() {
          return Err(format!("Repository not found: {}", repo_root.display()).into());
        }
        self.set_selected_repo(repo_root, window, cx)
      }
      CommandPaletteAction::ForgetRepository(repository) => {
        self.forget_repository(PathBuf::from(repository.path.as_ref()), window, cx)
      }
      other => crate::palette_actions::handle_global_command_palette_action(other, window, cx),
    }
  }

  /// Ahead/behind counter that runs the matching sync command when clicked.
  fn render_sync_counter(
    &self,
    id: &'static str,
    icon: gpui_component::IconName,
    count: usize,
    color: gpui::Hsla,
    tooltip: &'static str,
    in_flight: bool,
    command: RepoCommand,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let color = if in_flight {
      cx.theme().muted_foreground
    } else {
      color
    };

    h_flex()
      .id(id)
      .debug_selector(move || id.to_string())
      .items_center()
      .gap_1()
      .flex_shrink_0()
      .when(!in_flight, |this| {
        this
          .cursor_pointer()
          .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tooltip).build(window, cx)
          })
          .on_click(cx.listener(move |this, _, window, cx| {
            // The row switches repository; the counter runs its command instead.
            cx.stop_propagation();
            if let Err(error) = this.run_repo_command(command, window, cx) {
              window.push_notification(Notification::warning(error), cx);
            }
          }))
      })
      .child(gpui_component::Icon::new(icon).size_3().text_color(color))
      .child(div().text_xs().text_color(color).child(count.to_string()))
      .into_any_element()
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
      .h(px(40.))
      .min_h(px(40.))
      .max_h(px(40.))
      .flex_shrink_0()
      .items_center()
      .justify_between()
      .px_3()
      .border_b_1()
      .border_color(theme.border)
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

    let rows: Vec<_> = conversations
      .into_iter()
      .enumerate()
      .map(|(ix, meta)| {
        let is_current = meta.id == current_id;
        let id = meta.id.clone();
        let delete_id = meta.id.clone();
        let title = session_row_title(&meta);
        let time = format_relative_secs(meta.updated_at_secs, now);
        let group_name = SharedString::from(format!("session-row-{}", meta.id));

        div()
          .id(("session-page-session-row", ix))
          .group(group_name.clone())
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
                  .group_hover(group_name.clone(), |this| this.opacity(0.0))
                  .child(time),
              )
              .child(
                Button::new(("session-page-session-delete", ix))
                  .icon(UiIconName::Trash)
                  .xsmall()
                  .ghost()
                  .opacity(0.0)
                  .group_hover(group_name.clone(), |this| this.opacity(1.0))
                  .tooltip("Delete session")
                  .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.delete_session(&delete_id, cx);
                  })),
              ),
          )
      })
      .collect();

    let github_section = AuthStateStore::has_github_access(cx).then(|| self.render_inbox(cx));

    let repo_name = self
      .selected_repo
      .as_deref()
      .and_then(|path| path.file_name())
      .map(|name| name.to_string_lossy().into_owned());

    let branch_status = self.branch_status.clone();
    let sync_in_flight = self.repo_command_in_flight;

    let repo_context = repo_name.map(|name| {
      h_flex()
        .id("session-repo-context")
        .debug_selector(|| REPO_CONTEXT_DEBUG_SELECTOR.to_string())
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(theme.border)
        .cursor_pointer()
        .hover(|this| this.bg(theme.secondary_hover))
        .tooltip(|window, cx| {
          gpui_component::tooltip::Tooltip::new("Switch repository").build(window, cx)
        })
        .on_click(cx.listener(|this, _, window, cx| {
          this.open_command_palette_with_screen(
            Some(CommandPaletteInitialScreen::SwitchRepository),
            window,
            cx,
          );
        }))
        .child(
          div()
            .text_xs()
            .text_color(theme.foreground)
            .truncate()
            .child(name),
        )
        .when_some(branch_status.clone(), |this, status| {
          this.child(
            h_flex()
              .items_center()
              .gap_1()
              .min_w(px(0.0))
              .flex_1()
              .child(
                gpui_component::Icon::new(UiIconName::GitBranch)
                  .size_3()
                  .text_color(theme.muted_foreground),
              )
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .truncate()
                  .child(SharedString::from(status.name)),
              ),
          )
        })
        .when_some(branch_status, |this, status| {
          this
            .when(status.behind > 0, |this| {
              this.child(self.render_sync_counter(
                REPO_BEHIND_DEBUG_SELECTOR,
                gpui_component::IconName::ArrowDown,
                status.behind,
                theme.status_red(),
                "Pull",
                sync_in_flight,
                RepoCommand::Pull,
                cx,
              ))
            })
            .when(status.ahead > 0, |this| {
              this.child(self.render_sync_counter(
                REPO_AHEAD_DEBUG_SELECTOR,
                gpui_component::IconName::ArrowUp,
                status.ahead,
                theme.status_green(),
                "Push",
                sync_in_flight,
                RepoCommand::Push,
                cx,
              ))
            })
        })
    });

    v_flex()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.sidebar)
      .child(header)
      .child(if rows.is_empty() {
        v_flex()
          .flex_1()
          .min_h_0()
          .items_center()
          .justify_center()
          .gap_2()
          .px_4()
          .child(
            Icon::new(UiIconName::MessageCirclePlus)
              .size_4()
              .text_color(theme.muted_foreground),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child("No sessions yet"),
          )
          .child(
            div()
              .text_xs()
              .text_center()
              .text_color(theme.muted_foreground.opacity(0.8))
              .child("Message the agent to start one"),
          )
          .into_any_element()
      } else {
        div()
          .id("session-page-session-list")
          .flex_1()
          .min_h_0()
          .overflow_y_scroll()
          .py_1()
          .children(rows)
          .into_any_element()
      })
      .children(github_section)
      .children(repo_context)
      .into_any_element()
  }

  fn render_inbox(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let notifications = GithubNotificationsStore::list(cx);
    let unread = GithubNotificationsStore::unread_count(cx);

    let header = h_flex()
      .items_center()
      .gap_2()
      .px_3()
      .py_1()
      .child(
        div()
          .flex_1()
          .text_xs()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.muted_foreground)
          .child("GitHub inbox"),
      )
      .when(unread > 0, |this| {
        this.child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(unread.to_string()),
        )
      });

    let rows: Vec<_> = notifications
      .into_iter()
      .enumerate()
      .map(|(ix, notification)| {
        let group_name = SharedString::from(format!("inbox-row-{}", notification.id));
        let done_id = notification.id.clone();
        let time = format_relative_time(&notification.updated_at);
        let repo = notification.repository.full_name.clone();
        let title = notification.subject.title.clone();
        let is_unread = notification.unread;

        div()
          .id(("session-page-inbox-row", ix))
          .group(group_name.clone())
          .mx_2()
          .px_2()
          .py_1p5()
          .rounded(px(6.0))
          .cursor_pointer()
          .hover(|s| s.bg(theme.secondary_hover))
          .on_click(cx.listener(move |_, _, _, cx| {
            github_notifications::open_notification(&notification, cx);
          }))
          .child(
            v_flex()
              .gap_0p5()
              .child(
                h_flex()
                  .items_center()
                  .gap_2()
                  .when(is_unread, |this| {
                    this.child(
                      div()
                        .flex_shrink_0()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(theme.primary),
                    )
                  })
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
                    Button::new(("session-page-inbox-done", ix))
                      .icon(UiIconName::Check)
                      .xsmall()
                      .ghost()
                      .opacity(0.0)
                      .group_hover(group_name.clone(), |this| this.opacity(1.0))
                      .tooltip("Mark as done")
                      .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        github_notifications::mark_notification_done(done_id.clone(), cx);
                      })),
                  ),
              )
              .child(
                h_flex()
                  .items_center()
                  .gap_2()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(div().flex_1().min_w(px(0.0)).truncate().child(repo))
                  .child(div().child(time)),
              ),
          )
      })
      .collect();

    let body = if rows.is_empty() {
      div()
        .px_3()
        .py_2()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child("No notifications")
        .into_any_element()
    } else {
      div()
        .id("session-page-inbox-list")
        .max_h(px(INBOX_MAX_HEIGHT))
        .overflow_y_scroll()
        .pb_1()
        .children(rows)
        .into_any_element()
    };

    v_flex()
      .py_1()
      .border_t_1()
      .border_color(theme.border)
      .child(header)
      .child(body)
      .into_any_element()
  }

  fn render_center(&mut self, cx: &mut Context<Self>) -> AnyElement {
    // Keyed on what is shown, so every swap remounts and replays the fade.
    let (id, view) = match self.center {
      CenterView::Conversation => (
        SharedString::from("session-center-conversation"),
        self.render_conversation(cx),
      ),
      CenterView::Diff => {
        let file = self
          .selected_file
          .as_deref()
          .map(|path| path.to_string_lossy().into_owned())
          .unwrap_or_default();
        (
          SharedString::from(format!("session-center-diff-{file}")),
          self.render_diff_view(cx),
        )
      }
    };
    div()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .child(view)
      .with_animation(
        id,
        gpui::Animation::new(std::time::Duration::from_millis(CENTER_SWAP_FADE_MS))
          .with_easing(gpui::ease_out_quint()),
        |view, delta| view.opacity(delta),
      )
      .into_any_element()
  }

  fn render_conversation(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let mut container = div()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.background);
    if let Some(view) = self.agent_chat_view.clone() {
      container = container.child(view);
    }
    container.into_any_element()
  }

  fn render_diff_header(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let copyable_count = self.copyable_review_comment_count();
    let file_dirty = self
      .editor
      .as_ref()
      .is_some_and(|editor| editor.read(cx).is_dirty);
    let save_editor = self.editor.clone();
    let file_title = self
      .selected_file
      .as_deref()
      .map(|path| render_file_title(path, file_dirty, cx));

    h_flex()
      .h(px(40.))
      .min_h(px(40.))
      .max_h(px(40.))
      .flex_shrink_0()
      .items_center()
      .gap_3()
      .px_3()
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
      .children(file_title)
      .when(save_editor.is_some(), |this| {
        let save_editor = save_editor.clone();
        this.child(
          Button::new("session-page-save-file")
            .label("Save")
            .xsmall()
            .ghost()
            .disabled(!file_dirty)
            .on_click(move |_, _, cx| {
              if let Some(editor) = save_editor.clone() {
                editor.update(cx, |editor, cx| editor.save(cx));
              }
            }),
        )
      })
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
    let body: AnyElement = if let Some(preview) = self.binary_preview.as_ref() {
      render_binary_preview(preview, cx)
    } else if let Some(editor) = self.editor.clone() {
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
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    div()
      .size_full()
      .min_h_0()
      .track_focus(&self.focus_handle)
      .on_action(cx.listener(Self::close_workspace_page_action))
      .on_action(cx.listener(Self::close_file_view_action))
      .on_action(cx.listener(Self::show_command_palette_action))
      .on_action(cx.listener(Self::show_file_search_action))
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
  use crate::agent_review::LocalAgentReviewCommentState;
  use crate::workspace::WorkspaceApi;
  use editor::ReviewCommentMode;
  use editor::ReviewCommentSide;
  use git2::{Repository, Signature};
  use gpui::TestAppContext;
  use std::path::Path;
  use std::sync::atomic::{AtomicU64, Ordering};
  use std::time::{SystemTime, UNIX_EPOCH};

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
  fn repo_command_stage_and_unstage_move_the_whole_worktree() {
    let repo = TempRepo::init("session-page-stage-all");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let message = RepoCommand::StageAll.run(&repo.path).expect("stage all");
    assert_eq!(message.as_ref(), "Staged all changes");
    let staged = git::list_repo_status(&repo.path)
      .expect("status")
      .into_iter()
      .filter(|entry| !matches!(entry.stage, git::RepoStage::Unstaged))
      .count();
    assert_eq!(staged, 1);

    let message = RepoCommand::UnstageAll
      .run(&repo.path)
      .expect("unstage all");
    assert_eq!(message.as_ref(), "Unstaged all changes");
    let staged = git::list_repo_status(&repo.path)
      .expect("status")
      .into_iter()
      .filter(|entry| !matches!(entry.stage, git::RepoStage::Unstaged))
      .count();
    assert_eq!(staged, 0);
  }

  #[test]
  fn repo_command_surfaces_the_git_error_instead_of_a_message() {
    let missing = std::env::temp_dir().join("reviu-session-page-not-a-repo");
    let _ = std::fs::create_dir_all(&missing);

    let error = RepoCommand::StageAll.run(&missing).expect_err("not a repo");

    assert!(!error.to_string().is_empty());
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
    assert_eq!(
      session_row_title(&meta_with_title("Fix scroll")),
      "Fix scroll"
    );
  }

  struct TempRepo {
    path: PathBuf,
  }

  impl TempRepo {
    fn init(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!("reviu-{prefix}-{}-{nanos}", std::process::id()));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Repository::init(&path).expect("init git repository");
      Self { path }
    }
  }

  impl Drop for TempRepo {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  fn commit_text_file(repo_root: &Path, rel_path: &Path, contents: &str, message: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::write(repo_root.join(rel_path), contents).expect("write worktree file");

    let mut index = repo.index().expect("open index");
    index.add_path(rel_path).expect("stage file");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("find tree");
    let signature = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    let parents: Vec<_> = parent.iter().collect();
    repo
      .commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parents,
      )
      .expect("commit");
  }

  fn isolate_config_store_for_test() {
    static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
      "reviu-session-page-test-config-{}-{id}.sqlite",
      std::process::id()
    ));
    let _ = std::fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));
  }

  /// Mounts the page for real. The agent is only connected by `activate`, which
  /// the workspace calls when it routes here, so rendering spawns no process.
  fn add_session_page_window(
    repo_root: PathBuf,
    cx: &mut TestAppContext,
  ) -> (Entity<SessionPage>, &mut gpui::VisualTestContext) {
    // The recent-repository store is process-global, so parallel tests would race
    // over it; the repo is set on the page explicitly below instead.
    isolate_config_store_for_test();
    cx.update(|cx| {
      gpui_component::init(cx);
      if !cx.has_global::<crate::config::AppSettings>() {
        cx.set_global(crate::config::AppSettings::default());
      }
      if !cx.has_global::<AuthStateStore>() {
        cx.set_global(AuthStateStore::default());
      }
      if !cx.has_global::<WorkspaceApi>() {
        cx.set_global(WorkspaceApi::new());
      }
    });

    let mut mounted: Option<Entity<SessionPage>> = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let page = cx.new(|cx| SessionPage::new(window, cx));
      mounted = Some(page.clone());
      gpui_component::Root::new(page, window, cx)
    });
    let page = mounted.expect("session page");
    page.update(cx, |page, cx| {
      page.selected_repo = Some(repo_root.clone());
      page.review_panel.update(cx, |panel, cx| {
        panel.set_repo_root(Some(repo_root.clone()), cx)
      });
    });
    (page, cx)
  }

  async fn await_open_file(page: &Entity<SessionPage>, cx: &mut gpui::VisualTestContext) {
    let task = page.update(cx, |page, _| page.open_file_task.take());
    if let Some(task) = task {
      task.await;
    }
    cx.run_until_parked();
  }

  fn create_request(line: usize, body: &str) -> ReviewCommentCreateRequest {
    ReviewCommentCreateRequest {
      line,
      side: ReviewCommentSide::Right,
      start_line: None,
      start_side: None,
      in_reply_to_id: None,
      body: Arc::from(body),
      mode: ReviewCommentMode::SingleComment,
    }
  }

  #[gpui::test]
  async fn open_diff_switches_center_and_escape_returns(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-open-diff");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
      assert_eq!(page.center, CenterView::Diff);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert!(page.editor.is_some());
      assert_eq!(page.selected_file, Some(PathBuf::from("README.md")));
    });

    page.update_in(cx, |page, window, cx| {
      page.close_workspace_page_action(&CloseWorkspacePage, window, cx);
    });
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      // Editor kept for instant reopen of the same file.
      assert!(page.editor.is_some());
    });
  }

  #[gpui::test]
  async fn review_comments_create_sync_and_delete(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-comments");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, _window, cx| {
      page.create_agent_review_comment(create_request(0, "extract helper"), cx);
    });

    let comment_id = page.read_with(cx, |page, _| {
      assert_eq!(page.agent_review.all().len(), 1);
      let comment = &page.agent_review.all()[0];
      assert_eq!(comment.state, LocalAgentReviewCommentState::Draft);
      assert_eq!(comment.path, PathBuf::from("README.md"));
      // The snapshot of the commented line is what later tells whether the
      // agent addressed the comment.
      assert_eq!(comment.original_start_line, Some(1));
      assert_eq!(comment.original_lines, vec!["v2".to_string()]);
      assert_eq!(page.copyable_review_comment_count(), 1);
      comment.id
    });

    page.update(cx, |page, cx| {
      page.delete_agent_review_comment(comment_id, cx);
    });
    page.read_with(cx, |page, _| {
      assert!(page.agent_review.is_empty());
      assert_eq!(page.copyable_review_comment_count(), 0);
    });
  }

  #[gpui::test]
  async fn send_without_agent_panel_keeps_drafts(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-send-no-agent");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "still a draft"), cx);
      // No agent chat view mounted: the send must not mark anything as sent.
      assert!(page.agent_chat_view.is_none());
      page.send_agent_review_to_agent(window, cx);
    });

    page.read_with(cx, |page, _| {
      assert_eq!(page.agent_review.all().len(), 1);
      assert_eq!(
        page.agent_review.all()[0].state,
        LocalAgentReviewCommentState::Draft
      );
      assert_eq!(page.center, CenterView::Diff);
    });
  }

  #[gpui::test]
  async fn switching_repository_resets_the_shell_state(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-switch-from");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    let other = TempRepo::init("session-page-switch-to");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, _window, cx| {
      page.create_agent_review_comment(create_request(0, "keep this"), cx);
    });

    page.update_in(cx, |page, window, cx| {
      page
        .set_selected_repo(other.path.clone(), window, cx)
        .expect("switch repository");
    });

    page.read_with(cx, |page, cx| {
      assert_eq!(page.selected_repo.as_deref(), Some(other.path.as_path()));
      // The open diff and its draft comments belong to the previous repository.
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.editor.is_none());
      assert!(page.selected_file.is_none());
      assert!(page.agent_review.is_empty());
      // Conversations are stored per repository, so the panel is rebuilt.
      assert!(page.agent_chat_view.is_none());
      assert_eq!(
        page.review_panel.read(cx).repo_root(),
        Some(other.path.as_path())
      );
    });
  }

  #[gpui::test]
  async fn switching_to_the_same_repository_is_a_noop(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-switch-same");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page
        .set_selected_repo(repo.path.clone(), window, cx)
        .expect("same repository");
    });

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert!(page.editor.is_some());
    });
  }

  #[gpui::test]
  async fn forgetting_the_selected_repository_falls_back_to_the_next_recent_one(
    cx: &mut TestAppContext,
  ) {
    let repo = TempRepo::init("session-page-forget-selected");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-forget-fallback");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    cx.run_until_parked();
    ConfigStore::persist_recent_repository(&other.path);
    ConfigStore::persist_recent_repository(&repo.path);

    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository(repo.path.clone(), window, cx)
        .expect("forget repository");
    });

    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_repo.as_deref(), Some(other.path.as_path()));
    });
    assert!(
      !ConfigStore::load_recent_repositories()
        .iter()
        .any(|recent| recent.path == repo.path)
    );
  }

  #[gpui::test]
  async fn palette_repositories_put_the_open_repository_first(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-palette-repos");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-palette-repos-other");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    ConfigStore::persist_recent_repository(&other.path);

    page.read_with(cx, |page, _| {
      let repositories = page.palette_repositories();
      // The open repository is not in the recents yet, it still leads the list.
      assert_eq!(
        repositories.first().map(|repo| repo.path.to_string()),
        Some(repo.path.to_string_lossy().to_string())
      );
      assert_eq!(repositories.len(), 2);
    });

    // Once it is a recent too, it must not be listed twice. Order is left to the
    // recents, whose timestamps have a one-second granularity.
    ConfigStore::persist_recent_repository(&repo.path);
    page.read_with(cx, |page, _| {
      let repositories = page.palette_repositories();
      assert_eq!(repositories.len(), 2);
      assert_eq!(
        repositories
          .iter()
          .filter(|entry| entry.path.as_ref() == repo.path.to_string_lossy())
          .count(),
        1
      );
    });
  }

  #[gpui::test]
  async fn switching_to_a_repository_that_moved_reports_an_error(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-switch-missing");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let missing = std::env::temp_dir().join("reviu-session-page-not-a-repo");
    let _ = std::fs::remove_dir_all(&missing);

    let error = page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(
          CommandPaletteAction::SwitchRepository(ui::CommandPaletteRepository {
            path: missing.to_string_lossy().to_string().into(),
          }),
          window,
          cx,
        )
        .expect_err("missing repository")
    });

    assert!(error.contains("Repository not found"), "{error}");
    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_repo.as_deref(), Some(repo.path.as_path()));
    });
  }

  #[gpui::test]
  async fn forgetting_another_repository_keeps_the_open_one(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-forget-other");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-forget-other-recent");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    ConfigStore::persist_recent_repository(&repo.path);
    ConfigStore::persist_recent_repository(&other.path);

    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository(other.path.clone(), window, cx)
        .expect("forget repository");
    });

    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_repo.as_deref(), Some(repo.path.as_path()));
    });
    let recents = ConfigStore::load_recent_repositories();
    assert!(recents.iter().any(|recent| recent.path == repo.path));
    assert!(!recents.iter().any(|recent| recent.path == other.path));
  }

  #[gpui::test]
  async fn forgetting_the_last_repository_clears_the_selection(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-forget-last");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    ConfigStore::persist_recent_repository(&repo.path);

    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository(repo.path.clone(), window, cx)
        .expect("forget repository");
    });

    page.read_with(cx, |page, cx| {
      assert!(page.selected_repo.is_none());
      assert!(page.review_panel.read(cx).repo_root().is_none());
      assert!(page.branch_status.is_none());
    });
  }

  fn init_bare_repo(prefix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system clock before unix epoch")
      .as_nanos();
    path.push(format!(
      "reviu-{prefix}-bare-{}-{nanos}",
      std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("create temp dir");
    git2::Repository::init_bare(&path).expect("init bare repository");
    path
  }

  /// Publishes the current branch to a fresh bare remote and tracks it, so the
  /// ahead/behind counters have something to count.
  fn publish_to_new_remote(repo_root: &Path, prefix: &str) -> PathBuf {
    let remote_root = init_bare_repo(prefix);
    let repo = git2::Repository::open(repo_root).expect("open repo");
    repo
      .remote("origin", &remote_root.to_string_lossy())
      .expect("add remote");

    let head = repo.head().expect("head");
    let branch = head.shorthand().expect("branch name").to_string();
    let mut remote = repo.find_remote("origin").expect("find remote");
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(|_, _, _| git2::Cred::default());
    let mut options = git2::PushOptions::new();
    options.remote_callbacks(callbacks);
    remote
      .push(
        &[format!("refs/heads/{branch}:refs/heads/{branch}")],
        Some(&mut options),
      )
      .expect("push branch");

    repo
      .find_branch(&branch, git2::BranchType::Local)
      .expect("find local branch")
      .set_upstream(Some(&format!("origin/{branch}")))
      .expect("set upstream");

    remote_root
  }

  async fn await_branch_refresh(page: &Entity<SessionPage>, cx: &mut gpui::VisualTestContext) {
    let task = page.update(cx, |page, _| page._branch_task.take());
    if let Some(task) = task {
      task.await;
    }
    cx.run_until_parked();
  }

  #[gpui::test]
  async fn committing_in_the_shell_updates_the_ahead_counter(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-ahead-counter");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let _remote = publish_to_new_remote(&repo.path, "session-page-ahead-counter");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    cx.run_until_parked();

    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.read_with(cx, |page, _| {
      let status = page.branch_status.as_ref().expect("branch status");
      assert_eq!(status.ahead, 0);
      assert_eq!(status.behind, 0);
      assert!(status.has_upstream);
    });

    // A commit made from the shell must show up as something to push.
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert_eq!(page.branch_status.as_ref().expect("status").ahead, 1);
    });
  }

  #[gpui::test]
  async fn pushing_from_the_counter_clears_it(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-push-counter");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let remote = publish_to_new_remote(&repo.path, "session-page-push-counter");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    cx.run_until_parked();

    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.read_with(cx, |page, _| {
      assert_eq!(page.branch_status.as_ref().expect("status").ahead, 1);
    });

    // What the ahead counter runs when clicked.
    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(RepoCommand::Push, window, cx)
        .expect("push");
    });
    let command_task = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command_task.await;
    cx.run_until_parked();
    await_branch_refresh(&page, cx).await;

    let remote_repo = git2::Repository::open(&remote).expect("open remote");
    let head = remote_repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("remote head");
    assert_eq!(head.summary(), Some("second"));

    page.read_with(cx, |page, _| {
      assert_eq!(page.branch_status.as_ref().expect("status").ahead, 0);
    });
  }

  #[gpui::test]
  async fn a_second_git_command_is_refused_while_one_runs(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-command-in-flight");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    cx.run_until_parked();

    let error = page.update_in(cx, |page, window, cx| {
      page.repo_command_in_flight = true;
      page
        .run_repo_command(RepoCommand::Fetch, window, cx)
        .expect_err("refused while busy")
    });

    assert!(error.contains("still running"), "{error}");
  }

  #[gpui::test]
  async fn the_repo_line_is_painted_without_connecting_an_agent(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-repo-line");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(REPO_CONTEXT_DEBUG_SELECTOR).is_some(),
      "the repository line should be painted"
    );
    page.read_with(cx, |page, _| {
      // Rendering must not spawn the agent; the workspace calls activate.
      assert!(page.agent_chat_view.is_none());
    });
  }

  #[gpui::test]
  async fn sync_counters_are_painted_only_when_there_is_something_to_sync(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-counter-paint");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let _remote = publish_to_new_remote(&repo.path, "session-page-counter-paint");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    cx.run_until_parked();

    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(REPO_AHEAD_DEBUG_SELECTOR).is_none(),
      "nothing to push, no counter"
    );
    assert!(cx.debug_bounds(REPO_BEHIND_DEBUG_SELECTOR).is_none());

    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(REPO_AHEAD_DEBUG_SELECTOR).is_some(),
      "one commit to push, the counter shows up"
    );
    assert!(cx.debug_bounds(REPO_BEHIND_DEBUG_SELECTOR).is_none());
  }

  #[gpui::test]
  async fn clicking_the_ahead_counter_pushes_instead_of_switching_repository(
    cx: &mut TestAppContext,
  ) {
    let repo = TempRepo::init("session-page-counter-click");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let remote = publish_to_new_remote(&repo.path, "session-page-counter-click");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    let counter = cx
      .debug_bounds(REPO_AHEAD_DEBUG_SELECTOR)
      .expect("ahead counter bounds");
    cx.simulate_click(counter.center(), gpui::Modifiers::default());

    let command_task = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("push task")
    });
    command_task.await;
    cx.run_until_parked();

    // The click ran the push and did not open the repository switcher.
    let remote_repo = git2::Repository::open(&remote).expect("open remote");
    let head = remote_repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("remote head");
    assert_eq!(head.summary(), Some("second"));

    // The row under the counter opens the repository switcher: it must not fire.
    let switcher_open = cx.update(|window, cx| window.has_active_dialog(cx));
    assert!(!switcher_open, "the repository switcher should stay closed");
  }
}
