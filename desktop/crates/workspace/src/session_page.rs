//! Agent-first shell: sessions sidebar, conversation center, right dock.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use agent_chat_panel::{AgentChatPanel, AgentChatPanelEvent, ConversationMeta};
use editor::{
  ConflictResolution, DiffViewMode, Editor, EditorEvent, ReviewCommentCreateHandler,
  ReviewCommentCreateRequest, ReviewCommentDeleteHandler, ReviewCommentDisplayMode,
  ReviewCommentEditHandler,
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

use crate::active_local_repo::{ActiveLocalRepoStore, active_local_repo_snapshot};
use crate::agent_chat_state::{
  agent_chat_state_dir, agent_path_to_repo_relative, prune_agent_chat_state_once,
};
use crate::agent_review::{
  AgentReviewComments, original_lines_for_request, sync_comments_to_editor,
};
use crate::agent_settings::AgentSettings;
use crate::auth_state::AuthStateStore;
use crate::config::ConfigStore;
use crate::date_format::format_relative_time;
use crate::diff_view_policy::{DiffViewInputs, effective_diff_view};
use crate::dock_panel::{CommitMenuCommand, DockPanel, DockPanelEvent, DockPanelTab};
use crate::file_search_palette::open_file_search_palette;
use crate::file_view::{
  BinaryPreview, build_binary_preview, render_binary_preview, render_file_title_with_status,
};
use crate::github_notifications::{self, GithubNotificationsStore};
use crate::navigation::NavigationHistory;
use git::{InteractiveRebaseTarget, RepoStatusKind};

use crate::git_telemetry::{self, GitTelemetry};
use crate::hunk_actions::{resolve_active_conflict, restore_hunk, toggle_hunk_stage};
use crate::interactive_rebase;
use crate::interactive_rebase_todo_view::{
  InteractiveRebaseTodoView, InteractiveRebaseTodoViewCancelHandler,
  InteractiveRebaseTodoViewConfig, InteractiveRebaseTodoViewHandler,
};
use sentry::protocol::Map;

use crate::annotations::{
  AnnotationDirection, AnnotationNavigationState, annotation_navigation_state_for,
  can_navigate_annotations, navigate_annotation,
};
use crate::palette_branches::{
  delete_branch_candidates, palette_branch, palette_stashes, rebase_branch_candidates,
};
use crate::pro_teaser;
use crate::pull_request_dialog::{GithubBranchContext, open_create_pull_request_dialog};
use crate::repo_command::{RepoCommand, RepoCommandOutcome, branch_ref_from_palette};
use crate::repo_state::{PaletteCommand, RepoState, can_accept_all_conflicts, push_flags};
use crate::status_poll;
use crate::svg_preview::SvgPreview;
use crate::workspace::WorkspaceApi;
use crate::{
  CloseWorkspacePage, CommentHunk, SendReviewCommentsToAgent, ShowCommandPalette, ShowFileSearch,
};
use ui::{
  Button, ButtonVariants as _, CommandPalette, CommandPaletteAction, CommandPaletteCommand,
  CommandPaletteConfig, CommandPaletteHandler, CommandPaletteInitialScreen, CommandPalettePage,
  CommandPaletteRepository, ConfirmDialog, SearchFileEntry, SearchFileHandler, StatusThemeExt as _,
  UiIconName, WindowExt as _,
};

const DIFF_VIEW_TOGGLE_DEBUG_SELECTOR: &str = "session-diff-view-toggle";
const PREVIEW_TOGGLE_DEBUG_SELECTOR: &str = "session-preview-toggle";
const WHITESPACE_TOGGLE_DEBUG_SELECTOR: &str = "session-whitespace-toggle";
const ACCEPT_ALL_CURRENT_DEBUG_SELECTOR: &str = "session-accept-all-current";
const ACCEPT_ALL_INCOMING_DEBUG_SELECTOR: &str = "session-accept-all-incoming";
const ANNOTATION_COUNTER_DEBUG_SELECTOR: &str = "session-annotation-counter";
const INTERACTIVE_REBASE_DEBUG_SELECTOR: &str = "session-interactive-rebase";
const DIFF_EDITOR_DEBUG_SELECTOR: &str = "session-diff-editor";
const PREVIEW_PANE_DEBUG_SELECTOR: &str = "session-preview-pane";
const REPO_CONTEXT_DEBUG_SELECTOR: &str = "session-repo-context";
const OPEN_REPOSITORY_ROW_DEBUG_SELECTOR: &str = "session-open-repository";
const REPO_AHEAD_DEBUG_SELECTOR: &str = "session-repo-ahead";
const REPO_BEHIND_DEBUG_SELECTOR: &str = "session-repo-behind";

const SESSIONS_SIDEBAR_DEFAULT_WIDTH: f32 = 250.0;
const SESSIONS_SIDEBAR_MIN_WIDTH: f32 = 200.0;
const SESSIONS_SIDEBAR_MAX_WIDTH: f32 = 420.0;
const INBOX_MAX_HEIGHT: f32 = 220.0;
const CENTER_SWAP_FADE_MS: u64 = 180;
const DOCK_PANEL_DEFAULT_WIDTH: f32 = 320.0;
const DOCK_PANEL_MIN_WIDTH: f32 = 240.0;
const DOCK_PANEL_MAX_WIDTH: f32 = 560.0;

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
  /// The todo of an interactive rebase, waiting to be applied.
  InteractiveRebase,
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

  /// GitHub access changed: the inbox and the branch's pull request depend on it.
  pub fn refresh_github_state(cx: &mut App) {
    Self::with_page(cx, |page, _window, cx| {
      page
        .dock_panel
        .update(cx, |panel, cx| panel.refresh_branch_pull_request_state(cx));
      cx.notify();
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
  /// Entry point from a pull request: land in the repository, fetch, and merge
  /// its base branch so the conflicts can be resolved here.
  pub fn show_repository_and_merge_base(
    repo_root: PathBuf,
    base_branch_name: String,
    cx: &mut App,
  ) {
    NavigationHistory::navigate("/session", cx);
    Self::with_page(cx, move |page, window, cx| {
      page.start_merge_base_branch(repo_root.clone(), base_branch_name.clone(), window, cx);
    });
  }

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
  dock_panel: Entity<DockPanel>,
  selected_repo: Option<PathBuf>,
  center: CenterView,
  editor: Option<Entity<Editor>>,
  binary_preview: Option<BinaryPreview>,
  selected_file: Option<PathBuf>,
  /// Set while the center shows a file as it was in a commit.
  opened_commit: Option<String>,
  interactive_rebase_todo_view: Option<Entity<InteractiveRebaseTodoView>>,
  _interactive_rebase_task: Option<Task<()>>,
  pub(crate) _merge_base_task: Option<Task<()>>,
  /// Mounting a real agent panel in a test would spawn an agent process.
  #[cfg(test)]
  pretend_agent_turn_in_flight: bool,
  open_file_generation: u64,
  open_file_task: Option<Task<()>>,
  agent_review: AgentReviewComments,
  branch_status: Option<git::BranchStatus>,
  branches: Vec<git::BranchRef>,
  upstream_branch: Option<git::BranchRef>,
  default_branch: Option<git::BranchRef>,
  stashes: Vec<git::StashEntry>,
  default_stash_message: Option<SharedString>,
  diff_view: DiffViewMode,
  hide_whitespace: bool,
  show_preview: bool,
  svg_preview: Entity<SvgPreview>,
  // Review export waiting for the agent connection to become ready.
  pending_review_export: Option<String>,
  repo_command_in_flight: bool,
  pro_teaser_shown: bool,
  /// Set while pushing an unpublished branch on the way to the pull request form.
  pending_pull_request: Option<GithubBranchContext>,
  poll_window_active: bool,
  _branch_task: Option<Task<()>>,
  _active_repo_task: Option<Task<()>>,
  _pro_teaser_task: Option<Task<()>>,
  _repo_command_task: Option<Task<()>>,
  _poll_task: Option<Task<()>>,
}

mod agent;
mod render;
#[cfg(test)]
mod test_support;

impl SessionPage {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let selected_repo = ConfigStore::load_recent_repositories()
      .first()
      .map(|repo| repo.path.clone());
    let dock_panel = cx.new(|cx| DockPanel::new(selected_repo.clone(), window, cx));
    let svg_preview = cx.new(|_| SvgPreview::new());
    // The SVG renders on a background task; repaint when it lands.
    cx.observe(&svg_preview, |_, _, cx| cx.notify()).detach();
    cx.subscribe_in(
      &dock_panel,
      window,
      |this, _panel, event: &DockPanelEvent, window, cx| match event {
        DockPanelEvent::OpenFile { path } => {
          this.open_diff(path.clone(), None, window, cx);
        }
        DockPanelEvent::OpenCommitFile { commit_oid, path } => {
          this.open_commit_file(commit_oid.clone(), path.clone(), window, cx);
        }
        DockPanelEvent::Committed => {
          this.refresh_branch(cx);
        }
        DockPanelEvent::ContinueRebase => {
          if let Err(error) = this.run_repo_command(RepoCommand::ContinueRebase, window, cx) {
            window.push_notification(Notification::warning(error), cx);
          }
        }
        DockPanelEvent::RunCommand(command) => {
          if let Err(error) = this.run_commit_menu_command(*command, window, cx) {
            window.push_notification(Notification::warning(error), cx);
          }
        }
        DockPanelEvent::PublishBranchAndCreatePullRequest(context) => {
          this.publish_branch_and_create_pull_request(context.clone(), window, cx);
        }
        DockPanelEvent::StatusRefreshed => {
          this.sync_editor_unmerged_state(cx);
          this.sync_git_telemetry(cx);
        }
      },
    )
    .detach();

    let mut page = Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      agent_chat_view: None,
      dock_panel,
      selected_repo,
      center: CenterView::Conversation,
      editor: None,
      binary_preview: None,
      selected_file: None,
      opened_commit: None,
      interactive_rebase_todo_view: None,
      _interactive_rebase_task: None,
      _merge_base_task: None,
      #[cfg(test)]
      pretend_agent_turn_in_flight: false,
      open_file_generation: 0,
      open_file_task: None,
      agent_review: AgentReviewComments::new(),
      branch_status: None,
      branches: Vec::new(),
      upstream_branch: None,
      default_branch: None,
      stashes: Vec::new(),
      default_stash_message: None,
      diff_view: DiffViewMode::Inline,
      hide_whitespace: false,
      show_preview: false,
      svg_preview,
      pending_review_export: None,
      repo_command_in_flight: false,
      pro_teaser_shown: false,
      pending_pull_request: None,
      poll_window_active: true,
      _branch_task: None,
      _active_repo_task: None,
      _pro_teaser_task: None,
      _repo_command_task: None,
      _poll_task: None,
    };
    SessionPageHandle::register(cx);
    page.refresh_branch(cx);
    page.watch_window_activation(window, cx);
    page.start_polling(cx);
    page
  }

  /// Coming back to the window is the moment the working tree is most likely to
  /// have moved behind our back.
  fn watch_window_activation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    cx.observe_window_activation(window, |this, window, cx| {
      this.poll_window_active = window.is_window_active();
      if this.poll_window_active {
        this.poll_repository(cx);
      }
    })
    .detach();
  }

  /// Nothing in the shell tells us about edits made outside Reviu, so the
  /// working tree is re-read on a timer.
  fn start_polling(&mut self, cx: &mut Context<Self>) {
    self._poll_task = Some(cx.spawn(async move |this, cx| {
      loop {
        let Ok(window_active) = this.update(cx, |this, _| this.poll_window_active) else {
          return;
        };
        cx.background_executor()
          .timer(status_poll::poll_interval(window_active))
          .await;

        let polled = this.update(cx, |this, cx| {
          if !status_poll::should_poll(
            this.poll_window_active,
            this.selected_repo.as_deref(),
            this.repo_command_in_flight,
          ) {
            return;
          }
          this.poll_repository(cx);
        });
        if polled.is_err() {
          return;
        }
      }
    }));
  }

  fn poll_repository(&mut self, cx: &mut Context<Self>) {
    if self.selected_repo.is_none() {
      return;
    }
    self.refresh_branch(cx);
    self.dock_panel.update(cx, |panel, cx| panel.poll(cx));
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

  fn refresh_branch(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let task = cx.spawn(async move |this, cx| {
      let (status, branches, upstream, default_branch, stashes, default_stash_message) = cx
        .background_spawn(async move {
          (
            git::current_branch_status(&repo_root),
            git::list_branches(&repo_root),
            git::current_branch_upstream(&repo_root),
            git::default_remote_branch(&repo_root),
            git::list_stashes(&repo_root),
            git::default_stash_message(&repo_root),
          )
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        this.branch_status = status.ok();
        let branch_status = this.branch_status.clone();
        this
          .dock_panel
          .update(cx, |panel, cx| panel.set_branch_status(branch_status, cx));
        this.branches = branches.unwrap_or_default();
        this.upstream_branch = upstream.ok().flatten();
        this.default_branch = default_branch.ok().flatten();
        this.stashes = stashes.unwrap_or_default();
        this.default_stash_message = default_stash_message.ok().map(Into::into);
        this.publish_active_local_repo(cx);
        cx.notify();
      });
    });
    self._branch_task = Some(task);
  }

  /// The pull request page reads this to know which repository is open here,
  /// which is how it offers to switch branch or resolve conflicts.
  fn publish_active_local_repo(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      ActiveLocalRepoStore::set(cx, None);
      return;
    };
    let branch_name = self
      .branch_status
      .as_ref()
      .map(|status| status.name.clone());
    let has_uncommitted_changes = !self.dock_panel.read(cx).status_entries().is_empty();

    let task = cx.spawn(async move |this, cx| {
      let snapshot = cx
        .background_spawn(async move {
          active_local_repo_snapshot(&repo_root, branch_name, has_uncommitted_changes)
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        // A repository switch mid-flight wins over what we just read.
        if this.selected_repo.as_deref() != Some(snapshot.repo_root.as_path()) {
          return;
        }
        ActiveLocalRepoStore::set(cx, Some(snapshot));
      });
    });
    self._active_repo_task = Some(task);
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
    // Previewing is a detour, not a mode: opening a file always shows its code.
    self.show_preview = false;
    let left_commit_file = self.leave_commit_file(cx);
    let app_settings = crate::config::AppSettings::get(cx);
    self.diff_view = if app_settings.split_diff_view {
      DiffViewMode::Split
    } else {
      DiffViewMode::Inline
    };
    let diff_view = self.effective_diff_view(&rel_path, cx);
    // Reading preference for the session, seeded from the settings once.
    if self.selected_file.is_none() {
      self.hide_whitespace = app_settings.hide_whitespace;
    }
    let hide_whitespace = self.hide_whitespace;
    // Agent line numbers are 1-based; the editor reveals by 0-based doc line.
    let reveal_doc_line = reveal_line.map(|line| line.saturating_sub(1) as usize);

    self.center = CenterView::Diff;
    // Same path, but the snapshot of a commit is not the working-tree file.
    if !left_commit_file && self.selected_file.as_ref() == Some(&rel_path) && self.editor.is_some()
    {
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
      let loaded = cx
        .background_spawn(
          async move { Editor::load_file_for_editor(&load_repo_root, &load_file_path) },
        )
        .await;
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
        this.sync_editor_unmerged_state(cx);
        this.sync_git_telemetry(cx);
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
            EditorEvent::Saved | EditorEvent::HunkStagingChanged => {
              this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
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

  /// Split needs two sides to compare: a whole-file change or a binary preview
  /// falls back to inline.
  /// A file as it was in a commit: a read-only snapshot with its own patch.
  fn open_commit_file(
    &mut self,
    commit_oid: String,
    rel_path: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.show_preview = false;
    self.center = CenterView::Diff;
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    let generation = self.open_file_generation;
    self.selected_file = Some(rel_path.clone());
    self.opened_commit = Some(commit_oid.clone());
    self.editor = None;
    self.binary_preview = None;
    let hide_whitespace = self.hide_whitespace;
    let diff_view = self.effective_diff_view(&rel_path, cx);

    let task = cx.spawn(async move |this, cx| {
      let load_repo_root = repo_root.clone();
      let load_commit_oid = commit_oid.clone();
      let load_rel_path = rel_path.clone();
      let commit_file = cx
        .background_spawn(async move {
          git::load_commit_file_diff(&load_repo_root, &load_commit_oid, &load_rel_path)
        })
        .await;
      let _ = this.update(cx, move |this, cx| {
        if this.open_file_generation != generation {
          return;
        }
        let Ok(commit_file) = commit_file else {
          return;
        };

        let file_path = repo_root.join(&rel_path);
        let editor = cx.new(|cx| Editor::new_with_paths(repo_root.clone(), file_path, cx));
        let diff_set = if commit_file.patch.trim().is_empty() {
          None
        } else {
          git::diff_set_from_patch(&commit_file.patch).ok()
        };
        editor.update(cx, |editor, cx| {
          editor.load_readonly_snapshot(commit_file.content, diff_set, cx);
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_whitespace, cx);
        });
        this.binary_preview =
          build_binary_preview(rel_path.as_path(), commit_file.binary_bytes.clone());
        this.editor = Some(editor);
        this.svg_preview.update(cx, |preview, _| preview.clear());
        cx.notify();
      });
    });
    self.open_file_task = Some(task);
    self.focus_editor_on_next_frame(window, cx);
    cx.notify();
  }

  /// Back to the working tree: the history row stops being the open one.
  fn leave_commit_file(&mut self, cx: &mut Context<Self>) -> bool {
    if self.opened_commit.take().is_none() {
      return false;
    }
    let history = self.dock_panel.read(cx).history_list.clone();
    history.update(cx, |list, cx| list.set_opened(None, cx));
    true
  }

  fn effective_diff_view(&self, path: &Path, cx: &App) -> DiffViewMode {
    effective_diff_view(DiffViewInputs {
      preferred: self.diff_view,
      binary_preview: self.binary_preview.is_some(),
      previewing: self.show_preview && self.previewable(),
      whole_file_change: self.whole_file_change(path, cx),
    })
  }

  /// A file opened from the Files tab with no pending change has nothing to
  /// compare: the toggle would show the same content twice.
  fn selected_file_has_changes(&self, cx: &App) -> bool {
    // A commit snapshot always carries its own patch.
    if self.opened_commit.is_some() {
      return true;
    }
    let Some(path) = self.selected_file.as_deref() else {
      return false;
    };
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .any(|entry| entry.path == path)
  }

  fn selected_file_is_markdown(&self) -> bool {
    self
      .selected_file
      .as_deref()
      .is_some_and(crate::file_preview::is_markdown_path)
  }

  fn selected_file_is_svg(&self) -> bool {
    self
      .selected_file
      .as_deref()
      .is_some_and(crate::file_preview::is_svg_path)
  }

  fn previewable(&self) -> bool {
    self.selected_file_is_markdown() || self.selected_file_is_svg()
  }

  fn toggle_preview(&mut self, cx: &mut Context<Self>) {
    if !self.previewable() {
      self.show_preview = false;
      return;
    }
    self.show_preview = !self.show_preview;
    self.sync_diff_view(cx);
    self.sync_git_telemetry(cx);
    cx.notify();
  }

  fn split_disabled(&self, cx: &App) -> bool {
    let Some(path) = self.selected_file.as_deref() else {
      return true;
    };
    // The preview is not a reason to refuse: asking for split closes it.
    self.binary_preview.is_some() || self.whole_file_change(path, cx)
  }

  fn whole_file_change(&self, path: &Path, cx: &App) -> bool {
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .any(|entry| {
        entry.path == path
          && matches!(
            entry.status,
            git::RepoStatusKind::Untracked
              | git::RepoStatusKind::Added
              | git::RepoStatusKind::Deleted
          )
      })
  }

  fn toggle_diff_view_action(
    &mut self,
    _: &crate::ToggleDiffView,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_diff_view(cx);
    cx.stop_propagation();
  }

  fn previous_annotation_action(
    &mut self,
    _: &crate::PreviousAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.navigate_change(AnnotationDirection::Previous, cx);
  }

  fn next_annotation_action(
    &mut self,
    _: &crate::NextAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.navigate_change(AnnotationDirection::Next, cx);
  }

  fn navigate_change(&mut self, direction: AnnotationDirection, cx: &mut Context<Self>) {
    // A rendered file has nothing to walk.
    if self.center != CenterView::Diff || (self.show_preview && self.previewable()) {
      return;
    }
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let file_status = self.selected_file_status(cx);
    editor.update(cx, |editor, cx| {
      navigate_annotation(editor, file_status, direction, cx)
    });
    cx.stop_propagation();
  }

  /// The status of the open file, unless it comes from a commit: a snapshot has none.
  fn selected_file_status(&self, cx: &App) -> Option<RepoStatusKind> {
    if self.opened_commit.is_some() {
      return None;
    }
    let path = self.selected_file.as_deref()?;
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .find(|entry| entry.path == path)
      .map(|entry| entry.status)
  }

  /// What a crash or a git error should carry about where the user was.
  fn git_telemetry(&self, cx: &App) -> GitTelemetry<'_> {
    GitTelemetry {
      repo_root: self.selected_repo.as_deref(),
      selected_file: self.selected_file.as_deref(),
      branch: self
        .branch_status
        .as_ref()
        .map(|status| status.name.as_str()),
      tab: git_telemetry::dock_tab_tag(self.dock_panel.read(cx).active_tab()),
      diff_view: git_telemetry::diff_view_tag(
        self.diff_view,
        self.show_preview && self.previewable(),
      ),
    }
  }

  /// Pushing to GitHub without Pro is the one moment the teaser is relevant.
  fn maybe_show_pro_teaser(&mut self, cx: &mut Context<Self>) {
    if !pro_teaser::should_show_after_push(
      self.pro_teaser_shown,
      AuthStateStore::has_github_access(cx),
      true,
    ) {
      return;
    }
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };

    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let has_github_remote = cx
        .background_spawn(async move {
          git::current_github_remote_repo(&repo_root)
            .ok()
            .flatten()
            .is_some()
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        if !pro_teaser::should_show_after_push(
          this.pro_teaser_shown,
          AuthStateStore::has_github_access(cx),
          has_github_remote,
        ) {
          return;
        }
        this.pro_teaser_shown = true;
        pro_teaser::show_after_push(window_handle, cx);
      });
    });
    self._pro_teaser_task = Some(task);
  }

  /// Keeps the crash context in step with what the user is looking at.
  fn sync_git_telemetry(&self, cx: &App) {
    self.git_telemetry(cx).sync_or_clear();
  }

  /// A conflicted file is shown whole: once its markers are resolved there is no
  /// diff left to read, only the file.
  fn sync_editor_unmerged_state(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let is_unmerged = matches!(
      self.selected_file_status(cx),
      Some(RepoStatusKind::Conflicted)
    );
    editor.update(cx, |editor, cx| editor.set_is_unmerged(is_unmerged, cx));
  }

  /// The path a renamed file came from, so the diff header can name both sides.
  fn selected_file_old_path(&self, cx: &App) -> Option<PathBuf> {
    if self.opened_commit.is_some() {
      return None;
    }
    let path = self.selected_file.as_deref()?;
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .find(|entry| entry.path == path)
      .and_then(|entry| entry.old_path.clone())
  }

  fn annotation_navigation(&self, cx: &App) -> Option<AnnotationNavigationState> {
    let editor = self.editor.as_ref()?;
    let file_status = self.selected_file_status(cx);
    editor.read_with(cx, |editor, cx| {
      annotation_navigation_state_for(file_status, editor, cx)
    })
  }

  /// Accepting every conflict at once needs a conflicted file still holding markers.
  fn can_accept_all_conflicts(&self, cx: &App) -> bool {
    let file_status = self.selected_file_status(cx);
    self.editor.as_ref().is_some_and(|editor| {
      editor.read_with(cx, |editor, cx| {
        can_accept_all_conflicts(
          file_status,
          editor.is_read_only,
          editor.has_unresolved_conflict_markers(cx),
        )
      })
    })
  }

  fn resolve_all_conflicts(&mut self, resolution: ConflictResolution, cx: &mut Context<Self>) {
    if !self.can_accept_all_conflicts(cx) {
      return;
    }
    let Some(editor) = self.editor.clone() else {
      return;
    };
    editor.update(cx, |editor, cx| {
      editor.resolve_all_conflicts(resolution, cx)
    });
    self.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
  }

  fn toggle_hunk_stage_action(
    &mut self,
    _: &crate::ToggleHunkStage,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.diff_editor() else {
      return;
    };
    let file_status = self.selected_file_status(cx);
    toggle_hunk_stage(&editor, file_status, cx);
    cx.stop_propagation();
  }

  fn restore_hunk_action(
    &mut self,
    _: &crate::RestoreHunk,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.diff_editor() else {
      return;
    };
    let file_status = self.selected_file_status(cx);
    restore_hunk(&editor, file_status, cx);
    cx.stop_propagation();
  }

  fn accept_both_conflict_action(
    &mut self,
    _: &crate::AcceptBothConflict,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.diff_editor() else {
      return;
    };
    let file_status = self.selected_file_status(cx);
    resolve_active_conflict(&editor, file_status, ConflictResolution::Both, cx);
    cx.stop_propagation();
  }

  /// The editor of the open file, unless the center shows something else or a
  /// rendered file hides the diff.
  fn diff_editor(&self) -> Option<Entity<Editor>> {
    if self.center != CenterView::Diff || (self.show_preview && self.previewable()) {
      return None;
    }
    self.editor.clone()
  }

  fn toggle_hide_whitespace_action(
    &mut self,
    _: &crate::ToggleHideWhitespace,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_hide_whitespace(cx);
    cx.stop_propagation();
  }

  fn toggle_hide_whitespace(&mut self, cx: &mut Context<Self>) {
    // No diff on screen, nothing to hide: rendered file, or a file with no change.
    if self.center != CenterView::Diff
      || (self.show_preview && self.previewable())
      || !self.selected_file_has_changes(cx)
    {
      return;
    }
    self.hide_whitespace = !self.hide_whitespace;
    if let Some(editor) = self.editor.as_ref() {
      let value = self.hide_whitespace;
      editor.update(cx, |editor, cx| editor.set_ignore_whitespace(value, cx));
    }
    cx.notify();
  }

  fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
    // While the rendered file holds the pane there is no diff to switch.
    if self.center != CenterView::Diff
      || (self.show_preview && self.previewable())
      || self.split_disabled(cx)
    {
      return;
    }

    self.diff_view = match self.diff_view {
      DiffViewMode::Inline => DiffViewMode::Split,
      DiffViewMode::Split => DiffViewMode::Inline,
    };
    // Shared with the Git page: one preference for every diff surface.
    crate::config::AppSettings::update(cx, |settings| {
      settings.split_diff_view = self.diff_view == DiffViewMode::Split
    });
    self.sync_diff_view(cx);
    self.sync_git_telemetry(cx);
    cx.notify();
  }

  fn sync_diff_view(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let Some(path) = self.selected_file.clone() else {
      return;
    };
    let diff_view = self.effective_diff_view(&path, cx);
    editor.update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
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

  /// The editor handles `cmd-f` when it has focus; this catches it when the
  /// focus sits in the dock instead.
  fn find_action(&mut self, action: &editor::Find, window: &mut Window, cx: &mut Context<Self>) {
    let Some(editor) = self.diff_editor() else {
      return;
    };
    editor.update(cx, |editor, cx| editor::find(editor, action, window, cx));
    cx.stop_propagation();
  }

  /// The selection of the open diff becomes context for the next prompt.
  fn add_selection_to_agent_action(
    &mut self,
    _: &crate::AddSelectionToAgent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    cx.stop_propagation();
    match self.selection_context(cx) {
      Ok((path, text)) => self.deliver_selection_context(path, text, window, cx),
      Err(reason) => window.push_notification(Notification::info(reason), cx),
    }
  }

  /// What `cmd-shift-l` would send, or why it cannot.
  fn selection_context(&self, cx: &App) -> Result<(String, String), &'static str> {
    let Some(editor) = self.diff_editor() else {
      return Err("Open a file diff first");
    };
    let Some(text) = editor.read(cx).selected_text_for_copy(cx) else {
      return Err("Select code in the diff first");
    };
    let path = self
      .selected_file
      .as_ref()
      .map(|path| path.to_string_lossy().to_string())
      .unwrap_or_else(|| "selection".to_string());
    Ok((path, text))
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

  fn open_repository_action(
    &mut self,
    _: &crate::OpenRepository,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.start_open_repository(window, cx);
    cx.stop_propagation();
  }

  fn pull_changes_action(
    &mut self,
    _: &crate::PullChanges,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.run_shortcut_command(PaletteCommand::Pull, RepoCommand::Pull, window, cx);
  }

  fn push_changes_action(
    &mut self,
    _: &crate::PushChanges,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.run_shortcut_command(PaletteCommand::Push, RepoCommand::Push, window, cx);
  }

  fn force_push_changes_action(
    &mut self,
    _: &crate::ForcePushChanges,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.run_shortcut_command(
      PaletteCommand::ForcePush,
      RepoCommand::ForcePush,
      window,
      cx,
    );
  }

  /// A shortcut runs its command only when the palette would have offered it.
  fn run_shortcut_command(
    &mut self,
    rule: PaletteCommand,
    command: RepoCommand,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    cx.stop_propagation();
    if !self.repo_state("", cx).allows(rule) {
      return;
    }
    if let Err(error) = self.run_repo_command(command, window, cx) {
      window.push_notification(Notification::warning(error), cx);
    }
  }

  fn show_branch_switcher_action(
    &mut self,
    _: &crate::ShowBranchSwitcher,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    cx.stop_propagation();
    if self.branches.is_empty() {
      return;
    }
    self.open_command_palette_with_screen(
      Some(CommandPaletteInitialScreen::SwitchBranch),
      window,
      cx,
    );
  }

  fn toggle_terminal_action(
    &mut self,
    _: &crate::ToggleTerminalSidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_dock_tab(DockPanelTab::Terminal, window, cx);
  }

  fn open_history_action(
    &mut self,
    _: &crate::OpenGitHistorySidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_dock_tab(DockPanelTab::History, window, cx);
  }

  fn open_changes_action(
    &mut self,
    _: &crate::OpenGitChangesSidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_dock_tab(DockPanelTab::Changes, window, cx);
  }

  fn open_dock_tab(&mut self, tab: DockPanelTab, window: &mut Window, cx: &mut Context<Self>) {
    self
      .dock_panel
      .update(cx, |panel, cx| panel.open_tab(tab, window, cx));
    cx.stop_propagation();
  }

  fn toggle_file_stage_action(
    &mut self,
    _: &crate::ToggleFileStage,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    cx.stop_propagation();
    let Some(entry) = self.selected_status_entry(cx) else {
      return;
    };
    let staged = crate::changes_list::can_unstage(entry.stage);
    let outcome = if staged {
      self.unstage_selected_file(window, cx)
    } else {
      self.stage_selected_file(window, cx)
    };
    if let Err(error) = outcome {
      window.push_notification(Notification::warning(error), cx);
    }
  }

  fn restore_file_action(
    &mut self,
    _: &crate::RestoreFile,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    cx.stop_propagation();
    let Some(entry) = self.selected_status_entry(cx) else {
      return;
    };
    let changes_list = self.dock_panel.read(cx).changes_list();
    changes_list.update(cx, |list, cx| {
      list.restore_file(entry.path.clone(), entry.status, window, cx)
    });
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
      self.dock_panel.read(cx).status_entries().to_vec();
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
  /// The shell tracks no operation in progress: it never starts a merge or a rebase.
  fn repo_state<'a>(&'a self, commit_message: &'a str, cx: &'a App) -> RepoState<'a> {
    let branch_status = self.branch_status.as_ref();
    let (can_push, can_force_push) = push_flags(branch_status, branch_status.is_some(), false);
    let panel = self.dock_panel.read(cx);
    let status_entries = panel.status_entries();
    RepoState {
      has_repo: self.selected_repo.is_some(),
      merge_in_progress: panel.merge_in_progress(),
      rebase_in_progress: panel.rebase_in_progress(),
      has_head_commit: panel.head_status().has_head_commit,
      can_push,
      can_force_push,
      can_undo_last_commit: panel.head_status().can_undo_last_commit,
      branch_status,
      status_entries,
      selected_entry: self
        .selected_file
        .as_deref()
        .and_then(|path| status_entries.iter().find(|entry| entry.path == path)),
      commit_message,
    }
  }

  fn start_interactive_rebase(
    &mut self,
    target: InteractiveRebaseTarget,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(repo_root) = self.selected_repo.clone() else {
      return Err("No repository selected.".into());
    };
    if !self
      .repo_state("", cx)
      .allows(PaletteCommand::InteractiveRebase)
    {
      return Err("Interactive rebase is currently disabled.".into());
    }

    let preview = interactive_rebase::prepare_commits(&repo_root, &target)?;
    let commits = preview.commits;
    let Some(dropped) = interactive_rebase::dropped_merges_message(preview.dropped_merge_count)
    else {
      self.open_interactive_rebase_todo(target, commits, window, cx);
      return Ok(());
    };

    // Losing merge commits is the user's call, not ours.
    let view = cx.entity();
    window.on_next_frame(move |window, cx| {
      let view = view.clone();
      let target = target.clone();
      let commits = commits.clone();
      let dropped = dropped.clone();
      window.open_alert_dialog(cx, move |alert, _, _| {
        let view = view.clone();
        let target = target.clone();
        let commits = commits.clone();
        ConfirmDialog::new(
          SharedString::from("Drop merge commits?"),
          div().child(dropped.clone()),
        )
        .confirm_text("Continue")
        .cancel_text("Cancel")
        .on_confirm(move |_, window, cx| {
          let target = target.clone();
          let commits = commits.clone();
          view.update(cx, move |view, cx| {
            view.open_interactive_rebase_todo(target, commits, window, cx);
          });
          true
        })
        .build(alert)
      });
    });
    Ok(())
  }

  fn open_interactive_rebase_todo(
    &mut self,
    target: InteractiveRebaseTarget,
    commits: Vec<git::InteractiveRebaseCommit>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let view_for_submit = cx.entity();
    let on_submit: InteractiveRebaseTodoViewHandler =
      Arc::new(move |target, todo_entries, window, cx| {
        view_for_submit.update(cx, |view, cx| {
          view.apply_interactive_rebase(target, todo_entries, window, cx)
        })
      });
    let view_for_cancel = cx.entity();
    let on_cancel: InteractiveRebaseTodoViewCancelHandler = Arc::new(move |window, cx| {
      view_for_cancel.update(cx, |view, cx| {
        view.close_interactive_rebase_todo(window, cx);
      });
    });

    let config = InteractiveRebaseTodoViewConfig::new(target, commits, on_submit, on_cancel);
    let todo_view = cx.new(|cx| InteractiveRebaseTodoView::new(window, cx, config));
    self.interactive_rebase_todo_view = Some(todo_view.clone());
    self.center = CenterView::InteractiveRebase;
    cx.on_next_frame(window, move |_, window, cx| {
      todo_view.update(cx, |view, cx| view.focus_rows_list(window, cx));
    });
    cx.notify();
  }

  pub(super) fn close_interactive_rebase_todo(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.interactive_rebase_todo_view = None;
    self.center = if self.editor.is_some() {
      CenterView::Diff
    } else {
      CenterView::Conversation
    };
    self.focus_editor_on_next_frame(window, cx);
    cx.notify();
  }

  fn apply_interactive_rebase(
    &mut self,
    target: InteractiveRebaseTarget,
    todo_entries: Vec<git::InteractiveRebaseTodoEntry>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(repo_root) = self.selected_repo.clone() else {
      return Err("No repository selected.".into());
    };
    self.close_interactive_rebase_todo(window, cx);

    let success_message = interactive_rebase::success_message(&target);
    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let run_repo_root = repo_root.clone();
      let result = cx
        .background_spawn(async move {
          git::start_interactive_rebase(&run_repo_root, &target, &todo_entries)
        })
        .await;
      let stopped_on_conflict = git::is_rebase_in_progress(&repo_root).unwrap_or(false);
      let conflicted_path = crate::repo_command::first_conflicted_path(&repo_root);
      let rebase_message = stopped_on_conflict
        .then(|| {
          git::current_rebase_commit_message(&repo_root)
            .ok()
            .flatten()
        })
        .flatten();

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          match (&result, stopped_on_conflict) {
            (Ok(()), false) => {
              window.push_notification(Notification::success(success_message.clone()), cx);
              this
                .dock_panel
                .update(cx, |panel, cx| panel.set_commit_message("", window, cx));
            }
            // A rebase that stopped on a conflict is not a failure.
            (Err(error), false) => {
              window.push_notification(
                Notification::error(format!("Interactive rebase failed: {error}")),
                cx,
              );
            }
            _ => {}
          }
          if let Some(message) = rebase_message {
            this.dock_panel.update(cx, |panel, cx| {
              panel.set_commit_message(&message, window, cx)
            });
          }
          this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
          this.refresh_branch(cx);
          if let Some(path) = conflicted_path {
            this.open_diff(path, None, window, cx);
          }
          cx.notify();
        });
      });
    });
    self._interactive_rebase_task = Some(task);
    Ok(())
  }

  fn run_commit_menu_command(
    &mut self,
    command: CommitMenuCommand,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match command {
      CommitMenuCommand::Amend => self.amend_last_commit(window, cx),
      CommitMenuCommand::UndoLastCommit => {
        self.run_repo_command(RepoCommand::UndoLastCommit, window, cx)
      }
      CommitMenuCommand::Push => self.run_repo_command(RepoCommand::Push, window, cx),
      CommitMenuCommand::ForcePush => self.run_repo_command(RepoCommand::ForcePush, window, cx),
    }
  }

  /// Amending takes the message in the commit box, or keeps the old one when
  /// the box is empty.
  fn amend_last_commit(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let message = self.dock_panel.read(cx).commit_message(cx);
    let message = message.trim().to_string();
    let command = RepoCommand::Amend {
      message: (!message.is_empty()).then_some(message),
    };
    let started = self.run_repo_command(command, window, cx);
    if started.is_ok() {
      self
        .dock_panel
        .update(cx, |panel, cx| panel.set_commit_message("", window, cx));
    }
    started
  }

  /// Moving HEAD under a running agent breaks its turn: the branch waits.
  fn run_branch_command(
    &mut self,
    command: RepoCommand,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    if self.agent_turn_in_flight(cx) {
      return Err("Wait for the agent to finish before switching branch.".into());
    }
    self.run_repo_command(command, window, cx)
  }

  fn selected_status_entry(&self, cx: &App) -> Option<git::RepoStatusEntry> {
    let path = self.selected_file.as_deref()?;
    self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .find(|entry| entry.path == path)
      .cloned()
  }

  fn stage_selected_file(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(entry) = self.selected_status_entry(cx) else {
      return Err("No file selected.".into());
    };
    // A conflicted file with markers left asks before being marked resolved.
    let has_markers = self.editor.as_ref().is_none_or(|editor| {
      editor.read_with(cx, |editor, cx| editor.has_unresolved_conflict_markers(cx))
    });
    let changes_list = self.dock_panel.read(cx).changes_list();
    changes_list.update(cx, |list, cx| {
      list.set_open_file_has_conflict_markers(has_markers);
      list.stage_file_with_confirmation(entry.path.clone(), entry.status, window, cx);
    });
    Ok(())
  }

  fn unstage_selected_file(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(entry) = self.selected_status_entry(cx) else {
      return Err("No file selected.".into());
    };
    let changes_list = self.dock_panel.read(cx).changes_list();
    changes_list.update(cx, |list, cx| {
      list.unstage_file(entry.path.clone(), window, cx)
    });
    Ok(())
  }

  fn start_merge_base_branch(
    &mut self,
    repo_root: PathBuf,
    base_branch_name: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_repo.as_deref() != Some(repo_root.as_path())
      && let Err(error) = self.set_selected_repo(repo_root.clone(), window, cx)
    {
      window.push_notification(Notification::warning(error), cx);
      return;
    }

    // A conflict is already waiting: resume it instead of starting another merge.
    if let Some(path) = crate::repo_command::first_conflicted_path(&repo_root) {
      self.open_diff(path, None, window, cx);
      return;
    }

    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let fetch_root = repo_root.clone();
      let branch_name = base_branch_name.clone();
      let resolved = cx
        .background_spawn(async move {
          git::fetch(&fetch_root)?;
          git::resolve_branch_ref(&fetch_root, &branch_name)?.ok_or_else(|| {
            anyhow::anyhow!("branch {branch_name:?} was not found locally or on any remote")
          })
        })
        .await;

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| match resolved {
          Ok(branch) => {
            if let Err(error) = this.run_repo_command(RepoCommand::MergeBranch(branch), window, cx)
            {
              window.push_notification(Notification::warning(error), cx);
            }
          }
          Err(error) => {
            window.push_notification(Notification::error(error.to_string()), cx);
          }
        });
      });
    });
    self._merge_base_task = Some(task);
  }

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
    let pushed = matches!(command, RepoCommand::Push | RepoCommand::ForcePush);
    let telemetry_key = command.telemetry_key();
    self
      .git_telemetry(cx)
      .breadcrumb(command.label(), Map::new());
    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move { command.run(&repo_root) })
        .await;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.repo_command_in_flight = false;
          this
            .git_telemetry(cx)
            .report_outcome(telemetry_key, git_telemetry::outcome_report(&result));
          match result {
            Ok(outcome) => {
              match outcome {
                RepoCommandOutcome::Done { message } => {
                  window.push_notification(Notification::success(message), cx);
                }
                RepoCommandOutcome::UpToDate { message } => {
                  window.push_notification(Notification::info(message), cx);
                }
                RepoCommandOutcome::Conflicted {
                  path,
                  commit_message,
                  ..
                } => {
                  window.push_notification(
                    Notification::warning(format!("Resolve the conflicts in {}", path.display())),
                    cx,
                  );
                  if let Some(message) = commit_message {
                    this.dock_panel.update(cx, |panel, cx| {
                      panel.set_commit_message(&message, window, cx)
                    });
                  }
                  this.open_diff(path, None, window, cx);
                }
              }
              this.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
              this.refresh_branch(cx);
              this.open_pending_pull_request_dialog(window, cx);
              if pushed {
                this.maybe_show_pro_teaser(cx);
              }
            }
            Err(error) => {
              this.pending_pull_request = None;
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

  /// GitHub needs the branch on the remote before it can open a pull request,
  /// so the push comes first and the form only follows if it worked.
  fn publish_branch_and_create_pull_request(
    &mut self,
    context: GithubBranchContext,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.pending_pull_request = Some(context);
    if let Err(error) = self.run_repo_command(RepoCommand::Push, window, cx) {
      self.pending_pull_request = None;
      window.push_notification(Notification::warning(error), cx);
    }
  }

  fn open_pending_pull_request_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(context) = self.pending_pull_request.take() else {
      return;
    };
    let panel = self.dock_panel.clone();
    open_create_pull_request_dialog(
      WorkspaceApi::global(cx).api.clone(),
      self.window_handle,
      Rc::new(move |_context, _pull_request, cx| {
        panel.update(cx, |panel, cx| panel.refresh(cx));
      }),
      context,
      window,
      cx,
    );
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
    // A folder that is not a repository would be remembered as the one to open
    // on the next launch, so it is refused before anything is stored.
    let Some(repo_root) = git::discover_repository_root(&repo_root) else {
      return Err("This folder is not a git repository.".into());
    };
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
    self.dock_panel.update(cx, |panel, cx| {
      panel.set_repo_root(repo_root, cx);
      panel.refresh(cx);
    });
    self.refresh_branch(cx);
    // Without a repository there is no branch refresh to publish from.
    if self.selected_repo.is_none() {
      self.publish_active_local_repo(cx);
    }
    cx.notify();
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

  fn palette_commands(&self, repositories_len: usize, cx: &App) -> Vec<CommandPaletteCommand> {
    let mut commands = Vec::new();
    if repositories_len > 1 {
      commands.push(CommandPaletteCommand::switch_repository());
    }
    commands.push(CommandPaletteCommand::open_repository());
    if repositories_len > 0 {
      commands.push(CommandPaletteCommand::forget_repository());
    }

    if self.selected_repo.is_some() {
      let commit_message = self.dock_panel.read(cx).commit_message(cx);
      let state = self.repo_state(&commit_message, cx);
      if state.allows(PaletteCommand::Commit) {
        commands.push(CommandPaletteCommand::commit());
      }
      if self.can_accept_all_conflicts(cx) {
        commands.push(CommandPaletteCommand::accept_all_current_conflicts());
        commands.push(CommandPaletteCommand::accept_all_incoming_conflicts());
      }
      if state.allows(PaletteCommand::ContinueRebase) {
        commands.push(CommandPaletteCommand::continue_rebase());
      }
      if state.allows(PaletteCommand::SkipRebase) {
        commands.push(CommandPaletteCommand::skip_rebase());
      }
      if state.rebase_in_progress {
        commands.push(CommandPaletteCommand::abort_rebase());
      }
      if state.merge_in_progress {
        commands.push(CommandPaletteCommand::abort_merge());
      }
      if state.allows(PaletteCommand::StageAll) {
        commands.push(CommandPaletteCommand::stage_all());
      }
      if state.allows(PaletteCommand::UnstageAll) {
        commands.push(CommandPaletteCommand::unstage_all());
      }
      if state.allows(PaletteCommand::RestoreAll) {
        commands.push(CommandPaletteCommand::restore_all());
      }
      if let Some(command) = self.dock_panel.read(cx).branch_pull_request_command() {
        commands.push(command);
      }
      if state.allows(PaletteCommand::Push) {
        commands.push(CommandPaletteCommand::push("Push"));
      }
      if state.allows(PaletteCommand::ForcePush) {
        commands.push(CommandPaletteCommand::force_push());
      }
      if state.allows(PaletteCommand::Amend) {
        commands.push(CommandPaletteCommand::amend());
      }
      if state.allows(PaletteCommand::UndoLastCommit) {
        commands.push(CommandPaletteCommand::undo_last_commit());
      }
      if state.allows(PaletteCommand::CheckoutDetached) {
        commands.push(CommandPaletteCommand::checkout_detached());
      }
      if state.allows(PaletteCommand::StageSelectedFile) {
        commands.push(CommandPaletteCommand::stage_selected_file());
      }
      if state.allows(PaletteCommand::UnstageSelectedFile) {
        commands.push(CommandPaletteCommand::unstage_selected_file());
      }
      if state.allows(PaletteCommand::InteractiveRebase) {
        commands.push(CommandPaletteCommand::interactive_rebase());
      }
      if !self.branches.is_empty() {
        commands.push(CommandPaletteCommand::switch_branch());
        if !self.delete_branch_targets().is_empty() {
          commands.push(CommandPaletteCommand::delete_branch());
        }
      }
      if state.allows(PaletteCommand::MergeBranch) && !self.branches.is_empty() {
        commands.push(CommandPaletteCommand::merge_branch());
      }
      if state.allows(PaletteCommand::RebaseBranch) && !self.rebase_branch_targets().is_empty() {
        commands.push(CommandPaletteCommand::rebase_branch());
      }
      if state.allows(PaletteCommand::CherryPick) {
        commands.push(CommandPaletteCommand::cherry_pick());
      }
      if state.allows(PaletteCommand::Stash) {
        commands.push(CommandPaletteCommand::stash());
      }
      if state.allows(PaletteCommand::StashWithUntracked) {
        commands.push(CommandPaletteCommand::stash_with_untracked());
      }
      if !self.stashes.is_empty() {
        commands.push(CommandPaletteCommand::apply_stash());
        commands.push(CommandPaletteCommand::pop_stash());
        commands.push(CommandPaletteCommand::drop_stash());
      }
      if state.allows(PaletteCommand::Pull) {
        commands.push(CommandPaletteCommand::pull());
      }
      commands.push(CommandPaletteCommand::fetch());
    }

    commands.extend(CommandPaletteCommand::default_global_commands(
      CommandPalettePage::Session,
      AuthStateStore::has_github_access(cx),
    ));
    commands
  }

  fn current_branch_name(&self) -> Option<&str> {
    self
      .branch_status
      .as_ref()
      .map(|status| status.name.as_str())
  }

  fn rebase_branch_targets(&self) -> Vec<ui::CommandPaletteBranch> {
    rebase_branch_candidates(
      &self.branches,
      self.current_branch_name(),
      self.upstream_branch.as_ref(),
      self.default_branch.as_ref(),
    )
  }

  fn delete_branch_targets(&self) -> Vec<ui::CommandPaletteBranch> {
    delete_branch_candidates(&self.branches, self.current_branch_name())
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
    let repositories = self.palette_repositories();
    let commands = self.palette_commands(repositories.len(), cx);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let branches = self.branches.iter().map(palette_branch).collect::<Vec<_>>();
    let mut config = CommandPaletteConfig::new(branches, commands, handler)
      .with_repositories(repositories)
      .with_rebase_branches(self.rebase_branch_targets())
      .with_delete_branches(self.delete_branch_targets())
      .with_stashes(palette_stashes(&self.stashes));
    if let Some(message) = self.default_stash_message.clone() {
      config = config.with_default_stash_message(message);
    }
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
        self.dock_panel.update(cx, |panel, cx| panel.commit(cx));
        Ok(())
      }
      CommandPaletteAction::AcceptAllCurrentConflicts => {
        self.resolve_all_conflicts(ConflictResolution::Current, cx);
        Ok(())
      }
      CommandPaletteAction::AcceptAllIncomingConflicts => {
        self.resolve_all_conflicts(ConflictResolution::Incoming, cx);
        Ok(())
      }
      CommandPaletteAction::ContinueRebase => {
        self.run_repo_command(RepoCommand::ContinueRebase, window, cx)
      }
      CommandPaletteAction::SkipRebase => {
        self.run_repo_command(RepoCommand::SkipRebase, window, cx)
      }
      CommandPaletteAction::AbortRebase => {
        self.run_repo_command(RepoCommand::AbortRebase, window, cx)
      }
      CommandPaletteAction::AbortMerge => {
        self.run_repo_command(RepoCommand::AbortMerge, window, cx)
      }
      CommandPaletteAction::StageAll => self.run_repo_command(RepoCommand::StageAll, window, cx),
      CommandPaletteAction::UnstageAll => {
        self.run_repo_command(RepoCommand::UnstageAll, window, cx)
      }
      CommandPaletteAction::CreatePullRequest => {
        self
          .dock_panel
          .update(cx, |panel, cx| panel.create_branch_pull_request(window, cx));
        Ok(())
      }
      CommandPaletteAction::OpenPullRequest => {
        self
          .dock_panel
          .update(cx, |panel, cx| panel.open_branch_pull_request(cx));
        Ok(())
      }
      CommandPaletteAction::RestoreAll => {
        self
          .dock_panel
          .read(cx)
          .changes_list()
          .update(cx, |list, cx| {
            list.confirm_restore_all(window, cx);
          });
        Ok(())
      }
      CommandPaletteAction::Push => self.run_repo_command(RepoCommand::Push, window, cx),
      CommandPaletteAction::ForcePush => self.run_repo_command(RepoCommand::ForcePush, window, cx),
      CommandPaletteAction::Amend => self.amend_last_commit(window, cx),
      CommandPaletteAction::UndoLastCommit => {
        self.run_repo_command(RepoCommand::UndoLastCommit, window, cx)
      }
      CommandPaletteAction::CheckoutDetached { target } => {
        self.run_repo_command(RepoCommand::CheckoutDetached { target }, window, cx)
      }
      CommandPaletteAction::SwitchBranch(branch) => self.run_branch_command(
        RepoCommand::SwitchBranch(branch_ref_from_palette(&branch)),
        window,
        cx,
      ),
      CommandPaletteAction::CreateBranch { name } => {
        self.run_branch_command(RepoCommand::CreateBranch { name }, window, cx)
      }
      CommandPaletteAction::CreateBranchFrom { name, base } => self.run_branch_command(
        RepoCommand::CreateBranchFrom {
          name,
          base: branch_ref_from_palette(&base),
        },
        window,
        cx,
      ),
      CommandPaletteAction::DeleteBranch(branch) => self.run_repo_command(
        RepoCommand::DeleteBranch(branch_ref_from_palette(&branch)),
        window,
        cx,
      ),
      CommandPaletteAction::MergeBranch { name } => self.run_repo_command(
        RepoCommand::MergeBranch(branch_ref_from_palette(&name)),
        window,
        cx,
      ),
      CommandPaletteAction::RebaseBranch { name } => self.run_repo_command(
        RepoCommand::RebaseBranch(branch_ref_from_palette(&name)),
        window,
        cx,
      ),
      CommandPaletteAction::CherryPick { commit_hashes } => {
        self.run_repo_command(RepoCommand::CherryPick { commit_hashes }, window, cx)
      }
      CommandPaletteAction::Stash {
        include_untracked,
        message,
      } => self.run_repo_command(
        RepoCommand::Stash {
          include_untracked,
          message,
        },
        window,
        cx,
      ),
      CommandPaletteAction::ApplyStash(stash) => self.run_repo_command(
        RepoCommand::ApplyStash {
          index: stash.index,
          name: stash.name.to_string(),
        },
        window,
        cx,
      ),
      CommandPaletteAction::PopStash(stash) => self.run_repo_command(
        RepoCommand::PopStash {
          index: stash.index,
          name: stash.name.to_string(),
        },
        window,
        cx,
      ),
      CommandPaletteAction::DropStash(stash) => self.run_repo_command(
        RepoCommand::DropStash {
          index: stash.index,
          name: stash.name.to_string(),
        },
        window,
        cx,
      ),
      CommandPaletteAction::StageSelectedFile => self.stage_selected_file(window, cx),
      CommandPaletteAction::UnstageSelectedFile => self.unstage_selected_file(window, cx),
      CommandPaletteAction::InteractiveRebaseBranch { ref name } => self.start_interactive_rebase(
        InteractiveRebaseTarget::Branch(branch_ref_from_palette(name)),
        window,
        cx,
      ),
      CommandPaletteAction::InteractiveRebaseEditBranch { ref name } => self
        .start_interactive_rebase(
          InteractiveRebaseTarget::BranchInPlace(branch_ref_from_palette(name)),
          window,
          cx,
        ),
      CommandPaletteAction::InteractiveRebaseHeadCount { count } => {
        self.start_interactive_rebase(InteractiveRebaseTarget::HeadCount(count), window, cx)
      }
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
  use super::test_support::*;
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;
  use ui::CommandPaletteCommandId;

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
    assert_eq!(
      session_row_title(&meta_with_title("Fix scroll")),
      "Fix scroll"
    );
  }

  #[gpui::test]
  async fn open_diff_switches_center_and_escape_returns(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-open-diff");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
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
  async fn switching_repository_resets_the_shell_state(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-switch-from");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    let other = TempRepo::init("session-page-switch-to");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
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
        page.dock_panel.read(cx).repo_root(),
        Some(other.path.as_path())
      );
    });
  }

  #[gpui::test]
  async fn amending_and_undoing_reach_the_last_commit(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-amend");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      // The head status travels from the dock into the rules.
      assert!(page.dock_panel.read(cx).head_status().has_head_commit);
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::Amend));
      assert!(ids.contains(&CommandPaletteCommandId::UndoLastCommit));
      assert!(ids.contains(&CommandPaletteCommandId::CheckoutDetached));
    });

    // Amend takes the message in the box and rewords the last commit.
    page.update_in(cx, |page, window, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_commit_message("second, reworded", window, cx)
      });
      page
        .handle_command_palette_action(CommandPaletteAction::Amend, window, cx)
        .expect("amend runs")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    let history = git::list_commit_history(&repo.path, 10).expect("history");
    assert_eq!(history.len(), 2, "the commit was rewritten, not added to");
    assert_eq!(history[0].summary, "second, reworded");
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.dock_panel.read(cx).commit_message(cx),
        "",
        "the box is cleared once the message landed in the commit"
      );
    });

    // Undo puts the work back in the worktree.
    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::UndoLastCommit, window, cx)
        .expect("undo runs")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert_eq!(
      git::list_commit_history(&repo.path, 10)
        .expect("history")
        .len(),
      1
    );
    assert!(repo.path.join("b.txt").exists());
  }

  #[gpui::test]
  async fn branches_and_stashes_reach_the_palette(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-branches-stashes");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let base = git::current_branch_status(&repo.path)
      .expect("branch status")
      .name;
    git::create_branch(&repo.path, "feature").expect("create branch");
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");
    git::create_stash(&repo.path, false, Some("wip")).expect("stash");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert_eq!(page.stashes.len(), 1, "the stash was loaded");
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::SwitchBranch));
      assert!(ids.contains(&CommandPaletteCommandId::DeleteBranch));
      assert!(ids.contains(&CommandPaletteCommandId::MergeBranch));
      assert!(ids.contains(&CommandPaletteCommandId::CherryPick));
      assert!(ids.contains(&CommandPaletteCommandId::ApplyStash));
      assert!(ids.contains(&CommandPaletteCommandId::PopStash));
      assert!(ids.contains(&CommandPaletteCommandId::DropStash));
      assert!(
        !ids.contains(&CommandPaletteCommandId::Stash),
        "the worktree is clean once stashed"
      );

      // The lists behind the screens: never the branch we are on.
      let targets = page.delete_branch_targets();
      assert!(
        targets
          .iter()
          .any(|branch| branch.name.as_ref() == "feature")
      );
      assert!(!targets.iter().any(|branch| branch.name.as_ref() == base));
    });

    // Applying the stash brings the change back.
    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(
          CommandPaletteAction::PopStash(ui::CommandPaletteStash {
            index: 0,
            name: "wip".into(),
            oid: "".into(),
          }),
          window,
          cx,
        )
        .expect("pop the stash")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v2\n"
    );
  }

  #[gpui::test]
  async fn the_branch_and_stash_commands_do_what_they_say(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-branch-commands");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let base = git::current_branch_status(&repo.path)
      .expect("branch status")
      .name;

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    // Git prepares a stash message from HEAD; the palette offers it.
    page.read_with(cx, |page, _| {
      assert!(
        page.default_stash_message.is_some(),
        "the stash screen starts with a message"
      );
    });

    let run = |page: &Entity<SessionPage>,
               cx: &mut gpui::VisualTestContext,
               action: CommandPaletteAction| {
      page.update_in(cx, |page, window, cx| {
        page
          .handle_command_palette_action(action, window, cx)
          .expect("the command runs")
      });
      page.update(cx, |page, _| page._repo_command_task.take())
    };

    // Creating a branch switches to it.
    let task = run(
      &page,
      cx,
      CommandPaletteAction::CreateBranch {
        name: "feature".to_string(),
      },
    );
    task.expect("command task").await;
    cx.run_until_parked();
    assert_eq!(
      git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      "feature"
    );

    // A commit only on this branch, so the base actually matters below.
    commit_text_file(
      &repo.path,
      Path::new("only-feature.txt"),
      "x\n",
      "feature only",
    );

    // Creating from a base starts there, whatever branch we are on.
    let task = run(
      &page,
      cx,
      CommandPaletteAction::CreateBranchFrom {
        name: "from-base".to_string(),
        base: ui::CommandPaletteBranch {
          name: base.clone().into(),
          kind: ui::CommandPaletteBranchKind::Local,
        },
      },
    );
    task.expect("command task").await;
    cx.run_until_parked();
    assert_eq!(
      git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      "from-base"
    );
    assert!(
      !repo.path.join("only-feature.txt").exists(),
      "the new branch starts at the base, not at the branch we were on"
    );

    // Deleting the branch we left behind.
    let task = run(
      &page,
      cx,
      CommandPaletteAction::DeleteBranch(ui::CommandPaletteBranch {
        name: "feature".into(),
        kind: ui::CommandPaletteBranchKind::Local,
      }),
    );
    task.expect("command task").await;
    cx.run_until_parked();
    assert!(
      !git::list_branches(&repo.path)
        .expect("branches")
        .iter()
        .any(|branch| branch.name == "feature")
    );

    // Stashing from the palette puts the change aside.
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");
    let task = run(
      &page,
      cx,
      CommandPaletteAction::Stash {
        include_untracked: false,
        message: Some("from the palette".to_string()),
      },
    );
    task.expect("command task").await;
    cx.run_until_parked();
    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v1\n"
    );
    assert_eq!(git::list_stashes(&repo.path).expect("stashes").len(), 1);

    // The command triggered a status refresh; let it finish before touching git
    // again, or the index lock and the test race.
    let refresh = page.update(cx, |page, cx| {
      page
        .dock_panel
        .update(cx, |panel, _| panel._refresh_task.take())
    });
    if let Some(refresh) = refresh {
      refresh.await;
    }
    cx.run_until_parked();

    // Cherry-picking a commit from the base branch.
    let base_ref = git::BranchRef {
      name: base.clone(),
      kind: git::BranchKind::Local,
    };
    git::switch_branch(&repo.path, &base_ref).expect("switch to base");
    commit_text_file(&repo.path, Path::new("b.txt"), "picked\n", "pick me");
    let picked = git::current_head_sha(&repo.path)
      .expect("head sha")
      .expect("head sha");
    git::switch_branch(
      &repo.path,
      &git::BranchRef {
        name: "from-base".to_string(),
        kind: git::BranchKind::Local,
      },
    )
    .expect("switch back");

    let task = run(
      &page,
      cx,
      CommandPaletteAction::CherryPick {
        commit_hashes: vec![picked],
      },
    );
    task.expect("command task").await;
    cx.run_until_parked();
    assert!(repo.path.join("b.txt").exists());
  }

  #[gpui::test]
  async fn a_branch_switch_waits_for_the_agent(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-branch-switch-guard");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let base = git::current_branch_status(&repo.path)
      .expect("branch status")
      .name;
    git::create_branch(&repo.path, "feature").expect("create branch");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update(cx, |page, _| page.pretend_agent_turn_in_flight = true);

    let refused = page.update_in(cx, |page, window, cx| {
      page.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(ui::CommandPaletteBranch {
          name: "feature".into(),
          kind: ui::CommandPaletteBranchKind::Local,
        }),
        window,
        cx,
      )
    });
    assert_eq!(
      refused.expect_err("refused mid-turn").as_ref(),
      "Wait for the agent to finish before switching branch."
    );

    // Creating a branch switches to it, so it waits too.
    let refused = page.update_in(cx, |page, window, cx| {
      page.handle_command_palette_action(
        CommandPaletteAction::CreateBranch {
          name: "another".to_string(),
        },
        window,
        cx,
      )
    });
    assert_eq!(
      refused.expect_err("refused mid-turn").as_ref(),
      "Wait for the agent to finish before switching branch."
    );
    assert!(
      !git::list_branches(&repo.path)
        .expect("branches")
        .iter()
        .any(|branch| branch.name == "another")
    );
    assert_eq!(
      git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      base,
      "the branch did not move under the agent"
    );

    // Turn over: the switch goes through.
    page.update(cx, |page, _| page.pretend_agent_turn_in_flight = false);
    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(
          CommandPaletteAction::SwitchBranch(ui::CommandPaletteBranch {
            name: "feature".into(),
            kind: ui::CommandPaletteBranchKind::Local,
          }),
          window,
          cx,
        )
        .expect("switch runs")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert_eq!(
      git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      "feature"
    );
  }

  #[gpui::test]
  async fn the_commit_menu_runs_what_it_names(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-commit-menu");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    // An empty box keeps the message of the commit being amended.
    page.update_in(cx, |page, window, cx| {
      page
        .run_commit_menu_command(CommitMenuCommand::Amend, window, cx)
        .expect("amend runs")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    let history = git::list_commit_history(&repo.path, 10).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(
      history[0].summary, "second",
      "an empty box keeps the old message"
    );

    // The Undo entry undoes, it does not push.
    page.update_in(cx, |page, window, cx| {
      page
        .run_commit_menu_command(CommitMenuCommand::UndoLastCommit, window, cx)
        .expect("undo runs")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert_eq!(
      git::list_commit_history(&repo.path, 10)
        .expect("history")
        .len(),
      1
    );
  }

  #[gpui::test]
  async fn the_selected_file_is_staged_and_unstaged_from_the_palette(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-selected-file-stage");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    // Nothing selected: the commands stay out of the palette.
    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(!ids.contains(&CommandPaletteCommandId::StageSelectedFile));
      assert!(!ids.contains(&CommandPaletteCommandId::UnstageSelectedFile));
    });

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::StageSelectedFile));
      assert!(!ids.contains(&CommandPaletteCommandId::UnstageSelectedFile));
    });

    let stage = page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::StageSelectedFile, window, cx)
        .expect("stage the selected file");
      page
        .dock_panel
        .read(cx)
        .changes_list()
        .update(cx, |list, _| list._action_task.take())
    });
    stage.expect("staging task").await;
    cx.run_until_parked();
    let entries = git::list_repo_status(&repo.path).expect("status");
    assert_eq!(entries[0].stage, git::RepoStage::Staged);

    let unstage = page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::UnstageSelectedFile, window, cx)
        .expect("unstage the selected file");
      page
        .dock_panel
        .read(cx)
        .changes_list()
        .update(cx, |list, _| list._action_task.take())
    });
    unstage.expect("unstaging task").await;
    cx.run_until_parked();
    let entries = git::list_repo_status(&repo.path).expect("status");
    assert_eq!(entries[0].stage, git::RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn an_interactive_rebase_runs_from_the_center(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-interactive-rebase");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");
    commit_text_file(&repo.path, Path::new("c.txt"), "v1\n", "third");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::InteractiveRebase));
    });

    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(
          CommandPaletteAction::InteractiveRebaseHeadCount { count: 2 },
          window,
          cx,
        )
        .expect("the todo opens")
    });
    cx.run_until_parked();

    let commits = page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::InteractiveRebase);
      assert!(page.interactive_rebase_todo_view.is_some());
      git::list_interactive_rebase_commits(&repo.path, &InteractiveRebaseTarget::HeadCount(2))
        .expect("preview")
        .commits
    });
    assert_eq!(commits.len(), 2);

    // Drop the last commit, keep the other.
    let todo = vec![
      git::InteractiveRebaseTodoEntry {
        oid: commits[0].oid.clone(),
        action: git::InteractiveRebaseAction::Pick,
      },
      git::InteractiveRebaseTodoEntry {
        oid: commits[1].oid.clone(),
        action: git::InteractiveRebaseAction::Drop,
      },
    ];
    page.update_in(cx, |page, window, cx| {
      page
        .apply_interactive_rebase(InteractiveRebaseTarget::HeadCount(2), todo, window, cx)
        .expect("the rebase starts")
    });
    let task = page.update(cx, |page, _| {
      page._interactive_rebase_task.take().expect("rebase task")
    });
    task.await;
    cx.run_until_parked();

    // The todo left the center, and the dropped commit is gone.
    page.read_with(cx, |page, _| {
      assert!(page.interactive_rebase_todo_view.is_none());
      assert_ne!(page.center, CenterView::InteractiveRebase);
    });
    let summaries = git::list_commit_history(&repo.path, 10)
      .expect("history")
      .into_iter()
      .map(|commit| commit.summary)
      .collect::<Vec<_>>();
    assert!(!summaries.contains(&"third".to_string()));
    assert!(summaries.contains(&"second".to_string()));
  }

  #[gpui::test]
  async fn rebasing_onto_a_branch_stops_on_the_conflict(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-interactive-rebase-branch");
    let base = start_conflicting_rebase_setup(&repo.path);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    // The palette offers the other branches, never the one we are on.
    page.read_with(cx, |page, _| {
      let targets = rebase_branch_candidates(
        &page.branches,
        page
          .branch_status
          .as_ref()
          .map(|status| status.name.as_str()),
        page.upstream_branch.as_ref(),
        page.default_branch.as_ref(),
      );
      assert!(targets.iter().any(|branch| branch.name.as_ref() == base));
      assert!(
        !targets
          .iter()
          .any(|branch| branch.name.as_ref() == "feature")
      );
    });

    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(
          CommandPaletteAction::InteractiveRebaseBranch {
            name: ui::CommandPaletteBranch {
              name: base.clone().into(),
              kind: ui::CommandPaletteBranchKind::Local,
            },
          },
          window,
          cx,
        )
        .expect("the todo opens")
    });
    cx.run_until_parked();

    let commits = page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::InteractiveRebase);
      git::list_interactive_rebase_commits(
        &repo.path,
        &InteractiveRebaseTarget::Branch(git::BranchRef {
          name: base.clone(),
          kind: git::BranchKind::Local,
        }),
      )
      .expect("preview")
      .commits
    });

    let todo = commits
      .iter()
      .map(|commit| git::InteractiveRebaseTodoEntry {
        oid: commit.oid.clone(),
        action: git::InteractiveRebaseAction::Pick,
      })
      .collect::<Vec<_>>();
    page.update_in(cx, |page, window, cx| {
      page
        .apply_interactive_rebase(
          InteractiveRebaseTarget::Branch(git::BranchRef {
            name: base.clone(),
            kind: git::BranchKind::Local,
          }),
          todo,
          window,
          cx,
        )
        .expect("the rebase starts")
    });
    let task = page.update(cx, |page, _| {
      page._interactive_rebase_task.take().expect("rebase task")
    });
    task.await;
    cx.run_until_parked();

    // Stopped on the conflict: the file is on screen with the prepared message.
    assert!(git::is_rebase_in_progress(&repo.path).expect("rebase state"));
    page.read_with(cx, |page, cx| {
      assert_eq!(page.center, CenterView::Diff);
      assert_eq!(page.selected_file.as_deref(), Some(Path::new("a.txt")));
      assert!(page.interactive_rebase_todo_view.is_none());
      assert_eq!(page.dock_panel.read(cx).commit_message(cx), "feature work");
    });
  }

  #[gpui::test]
  async fn cancelling_the_todo_leaves_the_center_as_it_was(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-interactive-rebase-cancel");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("a.txt"), "v2\n", "second");
    commit_text_file(&repo.path, Path::new("a.txt"), "v3\n", "third");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page
        .start_interactive_rebase(InteractiveRebaseTarget::HeadCount(2), window, cx)
        .expect("the todo opens")
    });
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::InteractiveRebase)
    });

    page.update_in(cx, |page, window, cx| {
      page.close_interactive_rebase_todo(window, cx)
    });

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.interactive_rebase_todo_view.is_none());
    });
    // Nothing was rewritten.
    let summaries = git::list_commit_history(&repo.path, 10)
      .expect("history")
      .into_iter()
      .map(|commit| commit.summary)
      .collect::<Vec<_>>();
    assert_eq!(summaries, vec!["third", "second", "first"]);
  }

  #[gpui::test]
  async fn an_interactive_rebase_is_refused_with_uncommitted_changes(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-interactive-rebase-dirty");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("a.txt"), "v2\n", "second");
    std::fs::write(repo.path.join("a.txt"), "v3 working\n").expect("dirty the worktree");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(
        !ids.contains(&CommandPaletteCommandId::InteractiveRebase),
        "rewriting history under uncommitted changes is refused"
      );
    });

    let refused = page.update_in(cx, |page, window, cx| {
      page.start_interactive_rebase(InteractiveRebaseTarget::HeadCount(2), window, cx)
    });
    assert_eq!(
      refused.expect_err("refused").as_ref(),
      "Interactive rebase is currently disabled."
    );
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.interactive_rebase_todo_view.is_none());
    });
  }

  #[gpui::test]
  async fn a_repository_cannot_move_under_a_running_agent(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-turn-guard-from");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-turn-guard-to");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    ConfigStore::persist_recent_repository(&repo.path);
    page.update(cx, |page, _| page.pretend_agent_turn_in_flight = true);

    let switch = page.update_in(cx, |page, window, cx| {
      page.set_selected_repo(other.path.clone(), window, cx)
    });
    assert_eq!(
      switch.expect_err("switching is refused mid-turn").as_ref(),
      "Wait for the agent to finish before switching repository."
    );

    let forget = page.update_in(cx, |page, window, cx| {
      page.forget_repository(repo.path.clone(), window, cx)
    });
    assert_eq!(
      forget
        .expect_err("forgetting the open repository is refused mid-turn")
        .as_ref(),
      "Wait for the agent to finish before forgetting this repository."
    );

    // The shell stayed where it was.
    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_repo.as_deref(), Some(repo.path.as_path()));
    });
    assert!(
      ConfigStore::load_recent_repositories()
        .iter()
        .any(|recent| recent.path == repo.path),
      "a refused forget must not drop the repository from the list"
    );

    // The turn ends: the switch goes through.
    page.update(cx, |page, _| page.pretend_agent_turn_in_flight = false);
    page.update_in(cx, |page, window, cx| {
      page
        .set_selected_repo(other.path.clone(), window, cx)
        .expect("switching once the agent is idle")
    });
    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_repo.as_deref(), Some(other.path.as_path()));
    });
  }

  #[gpui::test]
  async fn switching_to_the_same_repository_is_a_noop(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-switch-same");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
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
  async fn picking_a_folder_that_is_not_a_repository_leaves_the_shell_empty(
    cx: &mut TestAppContext,
  ) {
    let plain_folder = crate::test_support::temp_path("session-page-picker-not-a-repo");
    std::fs::create_dir_all(&plain_folder).expect("create plain folder");

    let (page, cx) = add_session_page_window_without_repo(cx);
    cx.run_until_parked();

    let row = cx
      .debug_bounds(OPEN_REPOSITORY_ROW_DEBUG_SELECTOR)
      .expect("the sidebar offers to open a repository");
    let picked = plain_folder.clone();
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.simulate_path_prompt_response(move |_| Some(vec![picked]));
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert!(
        page.selected_repo.is_none(),
        "a folder without a repository is not selected"
      );
    });
    assert!(
      cx.debug_bounds(OPEN_REPOSITORY_ROW_DEBUG_SELECTOR)
        .is_some(),
      "the sidebar still asks for a repository"
    );
    assert!(
      ConfigStore::load_recent_repositories().is_empty(),
      "and nothing was remembered"
    );

    let _ = std::fs::remove_dir_all(&plain_folder);
  }

  #[gpui::test]
  async fn a_folder_without_a_repository_is_refused_and_not_remembered(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-repo-validation");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let plain_folder = crate::test_support::temp_path("session-page-not-a-repo");
    std::fs::create_dir_all(&plain_folder).expect("create plain folder");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    let refused = page.update_in(cx, |page, window, cx| {
      page.set_selected_repo(plain_folder.clone(), window, cx)
    });
    assert_eq!(
      refused.expect_err("a plain folder is refused").as_ref(),
      "This folder is not a git repository."
    );
    page.read_with(cx, |page, _| {
      assert_eq!(
        page.selected_repo.as_deref(),
        Some(repo.path.as_path()),
        "the shell stays on the repository it had"
      );
    });
    assert!(
      !ConfigStore::load_recent_repositories()
        .iter()
        .any(|recent| recent.path == plain_folder),
      "a refused folder must not come back as the repository to open next launch"
    );

    // A directory inside a repository is accepted, as its root.
    let nested = repo.path.join("src/deep");
    std::fs::create_dir_all(&nested).expect("create nested dirs");
    let other = TempRepo::init("session-page-repo-validation-other");
    commit_text_file(&other.path, Path::new("README.md"), "v1\n", "initial");
    let nested_other = other.path.join("src");
    std::fs::create_dir_all(&nested_other).expect("create nested dir");

    page
      .update_in(cx, |page, window, cx| {
        page.set_selected_repo(nested_other.clone(), window, cx)
      })
      .expect("a folder inside a repository is accepted");
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      let selected = page.selected_repo.clone().expect("selected repository");
      assert_eq!(
        selected.canonicalize().expect("canonical selection"),
        other.path.canonicalize().expect("canonical repo"),
        "the root is selected, not the folder that was picked"
      );
    });

    let _ = std::fs::remove_dir_all(&plain_folder);
  }

  #[gpui::test]
  async fn forgetting_the_only_repository_brings_the_open_row_back(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-forget-only");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    ConfigStore::persist_recent_repository(&repo.path);
    assert!(cx.debug_bounds(REPO_CONTEXT_DEBUG_SELECTOR).is_some());

    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository(repo.path.clone(), window, cx)
        .expect("forget repository");
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(page.selected_repo.is_none()));
    assert!(
      cx.debug_bounds(OPEN_REPOSITORY_ROW_DEBUG_SELECTOR)
        .is_some(),
      "forgetting the last repository must not leave the shell without a way back"
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
      assert!(page.dock_panel.read(cx).repo_root().is_none());
      assert!(page.branch_status.is_none());
    });
  }

  #[gpui::test]
  async fn committing_in_the_shell_updates_the_ahead_counter(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-ahead-counter");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let _remote = publish_to_new_remote(&repo.path, "session-page-ahead-counter");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
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
  async fn publishing_a_branch_opens_the_pull_request_form_after_the_push(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-publish-and-create");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let remote = publish_to_new_remote(&repo.path, "session-page-publish-and-create");
    git::create_branch(&repo.path, "feature").expect("create branch");
    git::switch_branch(
      &repo.path,
      &git::BranchRef {
        name: "feature".to_string(),
        kind: git::BranchKind::Local,
      },
    )
    .expect("switch branch");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "feature work");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.read_with(cx, |page, _| {
      assert!(
        !page
          .branch_status
          .as_ref()
          .expect("branch status")
          .has_upstream,
        "the branch starts unpublished"
      );
    });

    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature".to_string(),
    };
    page.update_in(cx, |page, window, cx| {
      page.publish_branch_and_create_pull_request(context, window, cx);
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("push task")
    });
    command.await;
    cx.run_until_parked();

    let remote_repo = git2::Repository::open(&remote).expect("open remote");
    assert!(
      remote_repo.refname_to_id("refs/heads/feature").is_ok(),
      "the branch reached the remote"
    );
    assert!(
      cx.update(|window, cx| window.has_active_dialog(cx)),
      "the pull request form follows the push"
    );
    page.read_with(cx, |page, _| {
      assert!(page.pending_pull_request.is_none());
    });
  }

  #[gpui::test]
  async fn a_refused_publish_leaves_no_form_waiting_for_the_next_push(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-publish-refused");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let remote = publish_to_new_remote(&repo.path, "session-page-publish-refused");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // Another git command is already running: the publish is refused up front.
    page.update(cx, |page, _| page.repo_command_in_flight = true);
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature".to_string(),
    };
    page.update_in(cx, |page, window, cx| {
      page.publish_branch_and_create_pull_request(context, window, cx);
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert!(page._repo_command_task.is_none(), "nothing was launched");
      assert!(page.pending_pull_request.is_none());
    });
    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));

    // An unrelated push must not inherit the form that was never opened.
    page.update(cx, |page, _| page.repo_command_in_flight = false);
    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(RepoCommand::Push, window, cx)
        .expect("push");
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("push task")
    });
    command.await;
    cx.run_until_parked();

    let remote_repo = git2::Repository::open(&remote).expect("open remote");
    let head = remote_repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("remote head");
    assert_eq!(head.summary(), Some("second"), "the push went through");
    assert!(
      !cx.update(|window, cx| window.has_active_dialog(cx)),
      "a plain push opens no pull request form"
    );
  }

  #[gpui::test]
  async fn a_failed_publish_opens_no_pull_request_form(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-publish-failure");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // No remote at all: the push cannot go anywhere.
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature".to_string(),
    };
    page.update_in(cx, |page, window, cx| {
      page.publish_branch_and_create_pull_request(context, window, cx);
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("push task")
    });
    command.await;
    cx.run_until_parked();

    assert!(
      !cx.update(|window, cx| window.has_active_dialog(cx)),
      "a push that failed must not open the form"
    );
    page.read_with(cx, |page, _| {
      assert!(page.pending_pull_request.is_none());
    });
  }

  #[gpui::test]
  async fn pushing_from_the_counter_clears_it(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-push-counter");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let remote = publish_to_new_remote(&repo.path, "session-page-push-counter");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
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
  async fn the_palette_reaches_the_pull_request_of_the_branch(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-pr-palette");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // No GitHub access: the palette says nothing about pull requests.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(crate::dock_panel::BranchPrState::NoAccess, cx);
      });
    });
    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(!ids.contains(&CommandPaletteCommandId::CreatePullRequest));
      assert!(!ids.contains(&CommandPaletteCommandId::OpenPullRequest));
    });

    // A published branch with no pull request yet.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(
          crate::dock_panel::BranchPrState::Missing(GithubBranchContext {
            owner: "acme".to_string(),
            repo: "widget".to_string(),
            branch: "feature".to_string(),
          }),
          cx,
        );
        panel.set_branch_status(
          Some(git::BranchStatus {
            name: "feature".to_string(),
            ahead: 0,
            behind: 0,
            has_upstream: true,
          }),
          cx,
        );
      });
    });

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::CreatePullRequest));
    });

    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::CreatePullRequest, window, cx)
        .expect("create pull request is allowed");
    });
    cx.run_until_parked();

    assert!(
      cx.update(|window, cx| window.has_active_dialog(cx)),
      "the palette opens the same form as the tab"
    );

    // An existing pull request: the palette opens it instead of the form.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(
          crate::dock_panel::BranchPrState::Found(
            GithubBranchContext {
              owner: "acme".to_string(),
              repo: "widget".to_string(),
              branch: "feature".to_string(),
            },
            Box::new(
              serde_json::from_value(serde_json::json!({
                "number": 42,
                "title": "Add widgets",
                "state": "open",
                "draft": false,
                "repository": { "owner": "acme", "repo": "widget" }
              }))
              .expect("build pull request"),
            ),
          ),
          cx,
        );
      });
    });

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::OpenPullRequest));
      assert!(!ids.contains(&CommandPaletteCommandId::CreatePullRequest));
    });

    // The pull request page is not mounted here: opening it is a no-op, not a crash.
    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::OpenPullRequest, window, cx)
        .expect("open pull request is allowed");
    });
    cx.run_until_parked();
  }

  #[gpui::test]
  async fn the_palette_restores_every_change_after_confirmation(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-restore-all");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    // Nothing changed yet: nothing to restore.
    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(!ids.contains(&CommandPaletteCommandId::RestoreAll));
    });

    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("modify file");
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::RestoreAll));
    });

    // Destructive: the command asks first and touches nothing on its own.
    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::RestoreAll, window, cx)
        .expect("restore all is allowed");
    });
    cx.run_until_parked();

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v2\n",
      "the file is only discarded once the dialog is confirmed"
    );

    // What the dialog runs on confirmation.
    let restore = page.update_in(cx, |page, window, cx| {
      page
        .dock_panel
        .read(cx)
        .changes_list()
        .update(cx, |list, cx| {
          list.restore_all(window, cx);
          list._action_task.take().expect("restore all task")
        })
    });
    restore.await;
    cx.run_until_parked();
    let refresh = page.update(cx, |page, cx| {
      page
        .dock_panel
        .update(cx, |panel, _| panel._refresh_task.take())
    });
    if let Some(task) = refresh {
      task.await;
    }
    cx.run_until_parked();

    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v1\n"
    );
    page.read_with(cx, |page, cx| {
      assert!(
        page.dock_panel.read(cx).status_entries().is_empty(),
        "the changes list follows a discard without an explicit refresh"
      );
    });
  }

  #[gpui::test]
  async fn the_palette_only_offers_what_the_repository_allows(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-palette-rules");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
      page.refresh_branch(cx);
    });
    await_branch_refresh(&page, cx).await;

    let ids = |page: &SessionPage, cx: &App| {
      page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>()
    };

    // Clean worktree, no upstream: nothing to commit, stage or sync.
    page.read_with(cx, |page, cx| {
      let ids = ids(page, cx);
      assert!(!ids.contains(&CommandPaletteCommandId::Commit));
      assert!(!ids.contains(&CommandPaletteCommandId::StageAll));
      assert!(!ids.contains(&CommandPaletteCommandId::UnstageAll));
      assert!(!ids.contains(&CommandPaletteCommandId::Pull));
      assert!(
        ids.contains(&CommandPaletteCommandId::Fetch),
        "fetching is always available"
      );
    });

    // A change and a message: committing and staging show up.
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_commit_message("a message", window, cx)
      });
    });

    page.read_with(cx, |page, cx| {
      let ids = ids(page, cx);
      assert!(ids.contains(&CommandPaletteCommandId::Commit));
      assert!(ids.contains(&CommandPaletteCommandId::StageAll));
    });
  }

  #[gpui::test]
  async fn the_palette_offers_syncing_once_the_branch_tracks_a_remote(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-palette-sync");
    let remote = crate::test_support::TempBareRepo::init("session-page-palette-sync-remote");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    git2::Repository::open(&repo.path)
      .expect("open repo")
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin");
    let branch = git::current_branch_status(&repo.path)
      .expect("branch status")
      .name;
    crate::test_support::push_branch_to_remote(&repo.path, &branch, "origin");
    crate::test_support::set_upstream(&repo.path, &branch, &format!("origin/{branch}"));
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      "v2\n",
      "ahead of the remote",
    );

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(
        ids.contains(&CommandPaletteCommandId::Push),
        "one commit ahead of the upstream is something to push"
      );
      assert!(
        ids.contains(&CommandPaletteCommandId::Pull),
        "a tracked branch can be pulled"
      );
      assert!(
        !ids.contains(&CommandPaletteCommandId::ForcePush),
        "nothing forces a push on a branch that only moved forward"
      );
    });

    // Rewriting a commit the remote already has diverges the branch.
    git::push(&repo.path, false).expect("push the commit first");
    git::undo_last_commit(&repo.path).expect("undo the last commit");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      "v2 rewritten\n",
      "rewritten",
    );
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::ForcePush));
      assert!(!ids.contains(&CommandPaletteCommandId::Push));
    });
  }

  /// Leaves `feature` checked out with a commit that conflicts with the base branch.
  fn start_conflicting_rebase_setup(repo_root: &Path) -> String {
    commit_text_file(repo_root, Path::new("a.txt"), "base\n", "initial");
    let base = git::current_branch_status(repo_root)
      .expect("branch status")
      .name;
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(repo_root, &feature.name).expect("create branch");
    git::switch_branch(repo_root, &feature).expect("switch to feature");
    commit_text_file(repo_root, Path::new("a.txt"), "feature\n", "feature work");
    let base_ref = git::BranchRef {
      name: base.clone(),
      kind: git::BranchKind::Local,
    };
    git::switch_branch(repo_root, &base_ref).expect("switch back");
    commit_text_file(repo_root, Path::new("a.txt"), "main\n", "main work");
    git::switch_branch(repo_root, &feature).expect("switch to feature");
    base
  }

  /// Leaves the repository mid-rebase, stopped on a conflicted file.
  fn start_conflicting_rebase(repo_root: &Path) -> String {
    commit_text_file(repo_root, Path::new("a.txt"), "base\n", "initial");
    let base = git::current_branch_status(repo_root)
      .expect("branch status")
      .name;
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(repo_root, &feature.name).expect("create branch");
    git::switch_branch(repo_root, &feature).expect("switch to feature");
    commit_text_file(repo_root, Path::new("a.txt"), "feature\n", "feature work");
    let base_ref = git::BranchRef {
      name: base.clone(),
      kind: git::BranchKind::Local,
    };
    git::switch_branch(repo_root, &base_ref).expect("switch back");
    commit_text_file(repo_root, Path::new("a.txt"), "main\n", "main work");
    git::switch_branch(repo_root, &feature).expect("switch to feature");
    let _ = git::rebase_branch(repo_root, &base_ref);
    assert!(
      git::is_rebase_in_progress(repo_root).expect("rebase state"),
      "the rebase must be waiting on the conflict"
    );
    base
  }

  #[gpui::test]
  async fn a_rebase_in_progress_turns_the_commit_button_into_continue(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-rebase-continue");
    start_conflicting_rebase(&repo.path);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let panel = page.dock_panel.read(cx);
      assert!(panel.rebase_in_progress());
      assert!(!panel.merge_in_progress());

      // The palette follows: no commit, but the rebase can be continued or dropped.
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(!ids.contains(&CommandPaletteCommandId::Commit));
      assert!(ids.contains(&CommandPaletteCommandId::SkipRebase));
      assert!(ids.contains(&CommandPaletteCommandId::AbortRebase));
      assert!(
        !ids.contains(&CommandPaletteCommandId::ContinueRebase),
        "the conflict is still there"
      );
    });

    // Resolve and stage: continuing becomes possible.
    std::fs::write(repo.path.join("a.txt"), "resolved\n").expect("resolve conflict");
    git::stage_all(&repo.path).expect("stage the resolution");
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::ContinueRebase));
    });

    // The dock button runs it, and the rebase lands.
    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(RepoCommand::ContinueRebase, window, cx)
        .expect("continue the rebase")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert!(!git::is_rebase_in_progress(&repo.path).expect("rebase state"));
  }

  #[gpui::test]
  async fn aborting_a_rebase_from_the_palette_puts_the_branch_back(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-rebase-abort");
    start_conflicting_rebase(&repo.path);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::AbortRebase, window, cx)
        .expect("abort the rebase")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert!(!git::is_rebase_in_progress(&repo.path).expect("rebase state"));
    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "feature\n",
      "the branch is back where it was"
    );
  }

  #[gpui::test]
  async fn skipping_from_the_palette_drops_the_conflicting_commit(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-rebase-skip");
    start_conflicting_rebase(&repo.path);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::SkipRebase, window, cx)
        .expect("skip the conflicting commit")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert!(!git::is_rebase_in_progress(&repo.path).expect("rebase state"));
    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "main\n",
      "the skipped commit left nothing behind"
    );
  }

  #[gpui::test]
  async fn a_file_from_the_history_opens_read_only_in_the_center(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-history-file");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let first = git::current_head_sha(&repo.path)
      .expect("head sha")
      .expect("head sha");
    commit_text_file(&repo.path, Path::new("a.txt"), "v2\n", "second");
    std::fs::write(repo.path.join("a.txt"), "v3 working\n").expect("update worktree");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    let history = page.read_with(cx, |page, cx| page.dock_panel.read(cx).history_list.clone());
    history.update(cx, |list, cx| {
      list.open_commit_file(first.clone(), PathBuf::from("a.txt"), cx)
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert_eq!(page.center, CenterView::Diff);
      assert_eq!(page.opened_commit.as_deref(), Some(first.as_str()));
      let editor = page.editor.as_ref().expect("editor").read(cx);
      // A snapshot has no working-tree status, so it is walked change by change.
      assert!(page.selected_file_status(cx).is_none());
      // The commit content, not what the worktree holds now.
      let first_line = editor
        .document()
        .read(cx)
        .line_content(0)
        .expect("first line")
        .to_string();
      assert_eq!(first_line.trim_end(), "v1");
      assert!(editor.is_read_only, "a commit snapshot cannot be edited");
    });

    // Back to the working tree: the history row stops being the open one.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert!(page.opened_commit.is_none());
      let editor = page.editor.as_ref().expect("editor").read(cx);
      let first_line = editor
        .document()
        .read(cx)
        .line_content(0)
        .expect("first line")
        .to_string();
      assert_eq!(first_line.trim_end(), "v3 working");
      assert!(!editor.is_read_only);
    });
  }

  #[gpui::test]
  async fn merging_the_base_branch_lands_on_the_conflict(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-merge-base");
    let base = start_conflicting_rebase_setup(&repo.path);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // Comes from a pull request: fetch, then merge its base branch here.
    page.update_in(cx, |page, window, cx| {
      page.start_merge_base_branch(repo.path.clone(), base.clone(), window, cx)
    });
    let merge_base = page.update(cx, |page, _| {
      page._merge_base_task.take().expect("merge base task")
    });
    merge_base.await;
    cx.run_until_parked();
    let command = page.update(cx, |page, _| page._repo_command_task.take());
    if let Some(command) = command {
      command.await;
    }
    cx.run_until_parked();

    assert!(git::is_merge_in_progress(&repo.path).expect("merge state"));
    page.read_with(cx, |page, cx| {
      assert_eq!(page.center, CenterView::Diff);
      assert_eq!(page.selected_file.as_deref(), Some(Path::new("a.txt")));
      assert_eq!(
        page.dock_panel.read(cx).commit_message(cx),
        crate::repo_command::merge_commit_message(&base, "feature")
      );
    });

    // Asked again mid-merge: it resumes the conflict instead of merging twice.
    page.update_in(cx, |page, window, cx| {
      page.close_diff(window, cx);
      page.start_merge_base_branch(repo.path.clone(), base.clone(), window, cx)
    });
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert!(
        page._merge_base_task.is_none(),
        "no fetch and no second merge"
      );
      assert_eq!(page.center, CenterView::Diff);
    });
  }

  #[gpui::test]
  async fn a_command_that_conflicts_opens_the_file_to_resolve(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-command-conflict");
    commit_text_file(&repo.path, Path::new("a.txt"), "base\n", "initial");
    let base = git::BranchRef {
      name: git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      kind: git::BranchKind::Local,
    };
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(&repo.path, &feature.name).expect("create branch");
    git::switch_branch(&repo.path, &feature).expect("switch to feature");
    commit_text_file(&repo.path, Path::new("a.txt"), "feature\n", "feature work");
    git::switch_branch(&repo.path, &base).expect("switch back");
    commit_text_file(&repo.path, Path::new("a.txt"), "main\n", "main work");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(RepoCommand::MergeBranch(feature), window, cx)
        .expect("the merge starts");
    });
    let task = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    task.await;
    cx.run_until_parked();

    // The conflict is a stop to resolve, not an error: the file is on screen.
    page.read_with(cx, |page, cx| {
      assert_eq!(page.selected_file.as_deref(), Some(Path::new("a.txt")));
      assert_eq!(page.center, CenterView::Diff);
      // Git prepared the merge message; the box carries it.
      assert_eq!(
        page.dock_panel.read(cx).commit_message(cx),
        crate::repo_command::merge_commit_message("feature", &base.name)
      );
    });

    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    // Mid-merge: aborting is offered, committing waits for the conflict to go.
    page.read_with(cx, |page, cx| {
      assert!(page.dock_panel.read(cx).merge_in_progress());
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::AbortMerge));
      assert!(!ids.contains(&CommandPaletteCommandId::Commit));
      assert!(!ids.contains(&CommandPaletteCommandId::AbortRebase));
    });

    // Resolved and staged: a merge ends with a commit.
    std::fs::write(repo.path.join("a.txt"), "resolved\n").expect("resolve conflict");
    git::stage_all(&repo.path).expect("stage the resolution");
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::Commit));
      assert!(!ids.contains(&CommandPaletteCommandId::ContinueRebase));
    });
  }

  #[gpui::test]
  async fn without_a_repository_the_sidebar_asks_for_one(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-open-repository");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window_without_repo(cx);
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(page.selected_repo.is_none()));
    assert!(
      cx.debug_bounds(REPO_CONTEXT_DEBUG_SELECTOR).is_none(),
      "there is no repository to name yet"
    );
    let row = cx
      .debug_bounds(OPEN_REPOSITORY_ROW_DEBUG_SELECTOR)
      .expect("the sidebar offers to open a repository");

    // Closing the picker changes nothing.
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    page.read_with(cx, |page, _| assert!(page.selected_repo.is_none()));
    assert!(
      cx.debug_bounds(OPEN_REPOSITORY_ROW_DEBUG_SELECTOR)
        .is_some(),
      "a cancelled picker leaves the row where it was"
    );

    // One gesture: the row opens the picker, no palette in between.
    let picked = repo.path.clone();
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.simulate_path_prompt_response(move |_| Some(vec![picked]));
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert_eq!(page.selected_repo.as_deref(), Some(repo.path.as_path()));
      assert_eq!(
        page
          .dock_panel
          .read(cx)
          .repo_root()
          .map(|path| path.to_path_buf()),
        Some(repo.path.clone()),
        "the dock follows the repository that was just opened"
      );
    });
    assert!(
      cx.debug_bounds(OPEN_REPOSITORY_ROW_DEBUG_SELECTOR)
        .is_none(),
      "the row goes back to naming the repository"
    );
    assert!(cx.debug_bounds(REPO_CONTEXT_DEBUG_SELECTOR).is_some());
  }

  #[gpui::test]
  async fn a_window_nobody_looks_at_stops_polling(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-poll-inactive");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();

    cx.deactivate_window();
    cx.run_until_parked();
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("edit outside Reviu");

    // A background window reads nothing, however long it waits.
    cx.executor()
      .advance_clock(status_poll::ACTIVE_STATUS_POLL_INTERVAL);
    cx.run_until_parked();
    cx.executor()
      .advance_clock(status_poll::INACTIVE_STATUS_POLL_INTERVAL);
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert!(
        page.dock_panel.read(cx).status_entries().is_empty(),
        "a background window is not worth a git status"
      );
    });

    // Coming back to the window catches up right away, without waiting for a tick.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_eq!(page.dock_panel.read(cx).status_entries().len(), 1);
    });
  }

  #[gpui::test]
  async fn pushing_to_github_without_pro_offers_pro_once(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-pro-teaser");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let _remote = publish_to_new_remote(&repo.path, "session-page-pro-teaser");
    // origin stays the local remote so the push can work; the repository is
    // still on GitHub, which is what the offer is about.
    git2::Repository::open(&repo.path)
      .expect("open repo")
      .remote("github", "git@github.com:acme/widget.git")
      .expect("add the github remote");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    let push = |page: &Entity<SessionPage>, cx: &mut gpui::VisualTestContext| {
      page.update_in(cx, |page, window, cx| {
        let _ = page.run_repo_command(RepoCommand::Push, window, cx);
      });
      page.update(cx, |page, _| page._repo_command_task.take())
    };

    if let Some(task) = push(&page, cx) {
      task.await;
    }
    cx.run_until_parked();
    let teaser = page.update(cx, |page, _| page._pro_teaser_task.take());
    if let Some(task) = teaser {
      task.await;
    }
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert!(
        page.pro_teaser_shown,
        "pushing to GitHub without Pro is when the offer makes sense"
      );
    });
    assert_eq!(
      cx.update(|window, cx| window.notifications(cx).len()),
      2,
      "the offer is a notification of its own, next to the push result"
    );

    // A second push says nothing more.
    if let Some(task) = push(&page, cx) {
      task.await;
    }
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert!(
        page._pro_teaser_task.is_none(),
        "the second push does not even look"
      );
    });
  }

  #[gpui::test]
  async fn pushing_with_pro_says_nothing_about_pro(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-pro-teaser-pro");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let _remote = publish_to_new_remote(&repo.path, "session-page-pro-teaser-pro");
    git2::Repository::open(&repo.path)
      .expect("open repo")
      .remote("github", "git@github.com:acme/widget.git")
      .expect("add the github remote");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.update(|_, cx| {
      AuthStateStore::set(
        cx,
        crate::auth_state::AuthState::Authenticated(Box::new(
          serde_json::from_value(serde_json::json!({
            "id": "user_123",
            "name": "Joris",
            "email": "joris@example.com",
            "emailVerified": true,
            "image": null,
            "githubLogin": "joris-gallot",
            "role": "pro",
          }))
          .expect("build user"),
        )),
      );
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      let _ = page.run_repo_command(RepoCommand::Push, window, cx);
    });
    let command = page.update(cx, |page, _| page._repo_command_task.take());
    if let Some(task) = command {
      task.await;
    }
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert!(
        !page.pro_teaser_shown,
        "someone who already pays has nothing to be offered"
      );
    });
    assert_eq!(
      cx.update(|window, cx| window.notifications(cx).len()),
      1,
      "only the push result"
    );
  }

  #[gpui::test]
  async fn a_failed_command_is_reported_under_its_own_key(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-telemetry");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    let sink = crate::git_telemetry::test_support::RecordingSink::install();

    // No remote: the push cannot succeed.
    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(RepoCommand::Push, window, cx)
        .expect("push starts");
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    use crate::git_telemetry::test_support::Report;
    let reports = sink.reports();
    assert!(
      reports.contains(&Report::Breadcrumb("Push".to_string())),
      "the command that ran leaves a trail, got {reports:?}"
    );
    assert!(
      reports.iter().any(|report| matches!(
        report,
        Report::Unexpected { operation, .. } if operation == RepoCommand::Push.telemetry_key()
      )),
      "the failure is filed under the command's own key, got {reports:?}"
    );

    // A command that works reports its run, and no error.
    page.update_in(cx, |page, window, cx| {
      page
        .run_repo_command(RepoCommand::StageAll, window, cx)
        .expect("stage all starts");
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    let reports = sink.reports();
    assert!(
      !reports
        .iter()
        .any(|report| matches!(report, Report::Unexpected { .. })),
      "success is not a crash, got {reports:?}"
    );
    crate::git_telemetry::set_test_sink(None);
  }

  #[gpui::test]
  async fn a_conflicted_file_is_shown_whole_until_it_is_resolved(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-unmerged");
    // Long enough that a diff folds most of it away, unlike a whole-file view.
    let long_file = |mid: &str| {
      let mut lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
      lines[19] = mid.to_string();
      format!("{}\n", lines.join("\n"))
    };
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("base"),
      "initial",
    );
    let base = git::BranchRef {
      name: git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      kind: git::BranchKind::Local,
    };
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(&repo.path, &feature.name).expect("create branch");
    git::switch_branch(&repo.path, &feature).expect("switch to feature");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("feature"),
      "feature work",
    );
    git::switch_branch(&repo.path, &base).expect("switch back");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("main"),
      "main work",
    );
    let _ = git::merge_branch(&repo.path, &feature);
    // Markers resolved by hand, but git still calls the file conflicted.
    std::fs::write(repo.path.join("a.txt"), long_file("resolved")).expect("resolve conflict");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    // Whole file: every document line is visible, nothing is folded away.
    let visible_and_total = |page: &SessionPage, cx: &App| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      let projection = editor.projection().expect("projection");
      (
        projection.visible_doc_lines.len(),
        projection.doc_to_display.len(),
      )
    };

    page.read_with(cx, |page, cx| {
      let (visible, total) = visible_and_total(page, cx);
      assert_eq!(
        visible, total,
        "a conflicted file is read whole, there is no diff left in it"
      );
    });

    // Staging the resolution ends the conflict: the file goes back to a diff.
    git::stage_all(&repo.path).expect("stage the resolution");
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_editor_diff(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let (visible, total) = visible_and_total(page, cx);
      assert!(
        visible < total,
        "a resolved file is read as a diff again, got {visible} of {total} lines"
      );
    });
  }

  #[gpui::test]
  async fn a_merge_that_conflicts_switches_the_open_file_to_the_whole_view(
    cx: &mut TestAppContext,
  ) {
    let repo = TempRepo::init("session-page-unmerged-later");
    let long_file = |mid: &str| {
      let mut lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
      lines[19] = mid.to_string();
      format!("{}\n", lines.join("\n"))
    };
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("base"),
      "initial",
    );
    let base = git::BranchRef {
      name: git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      kind: git::BranchKind::Local,
    };
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(&repo.path, &feature.name).expect("create branch");
    git::switch_branch(&repo.path, &feature).expect("switch to feature");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("feature"),
      "feature work",
    );
    git::switch_branch(&repo.path, &base).expect("switch back");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &long_file("main"),
      "main work",
    );

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // The file is open and clean when the merge starts.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    let visible_and_total = |page: &SessionPage, cx: &App| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      let projection = editor.projection().expect("projection");
      (
        projection.visible_doc_lines.len(),
        projection.doc_to_display.len(),
      )
    };
    page.read_with(cx, |page, cx| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      assert!(!editor.is_unmerged(), "a clean file is read as a diff");
      let (visible, total) = visible_and_total(page, cx);
      assert_eq!(visible, total, "and a file without changes shows in full");
    });

    let _ = git::merge_branch(&repo.path, &feature);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_editor_diff(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      assert!(
        editor.is_unmerged(),
        "the file the merge just broke is read whole, without reopening it"
      );
    });
  }

  #[gpui::test]
  async fn the_open_repository_is_published_for_the_pull_request_page(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-active-repo");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    git2::Repository::open(&repo.path)
      .expect("open repo")
      .remote("origin", "git@github.com:acme/widget.git")
      .expect("add remote");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("leave a change behind");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    let publish = page.update(cx, |page, _| page._active_repo_task.take());
    if let Some(task) = publish {
      task.await;
    }
    cx.run_until_parked();

    let published = cx
      .update(|_, cx| crate::active_local_repo::ActiveLocalRepoStore::get(cx))
      .expect("the shell publishes the repository it has open");
    assert_eq!(published.repo_root, repo.path);
    assert_eq!(published.github_owner.as_deref(), Some("acme"));
    assert_eq!(published.github_repo.as_deref(), Some("widget"));
    assert!(published.current_branch.is_some());
    assert!(
      published.has_uncommitted_changes,
      "the pull request page refuses to switch branch over uncommitted work"
    );

    // Forgetting the last repository leaves nothing published.
    ConfigStore::persist_recent_repository(&repo.path);
    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository(repo.path.clone(), window, cx)
        .expect("forget repository");
    });
    cx.run_until_parked();

    assert_eq!(
      cx.update(|_, cx| crate::active_local_repo::ActiveLocalRepoStore::get(cx)),
      None
    );
  }

  #[gpui::test]
  async fn switching_repository_mid_publish_does_not_publish_the_old_one(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-active-repo-race");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-active-repo-race-other");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // The read is in flight when the user switches repository.
    let publish = page.update(cx, |page, cx| {
      page.publish_active_local_repo(cx);
      page._active_repo_task.take().expect("publish task")
    });
    page.update(cx, |page, _| page.selected_repo = Some(other.path.clone()));
    publish.await;
    cx.run_until_parked();

    assert_eq!(
      cx.update(|_, cx| crate::active_local_repo::ActiveLocalRepoStore::get(cx)),
      None,
      "the pull request page must never be pointed at the repository we just left"
    );
  }

  #[gpui::test]
  async fn a_branch_switched_outside_reviu_shows_up_on_the_next_poll(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-poll-branch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    git::create_branch(&repo.path, "feature").expect("create branch");
    git::switch_branch(
      &repo.path,
      &git::BranchRef {
        name: "feature".to_string(),
        kind: git::BranchKind::Local,
      },
    )
    .expect("switch branch");
    cx.executor()
      .advance_clock(status_poll::ACTIVE_STATUS_POLL_INTERVAL);
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert_eq!(
        page
          .branch_status
          .as_ref()
          .map(|status| status.name.clone()),
        Some("feature".to_string()),
        "the poll follows the branch, not just the changed files"
      );
    });
  }

  #[gpui::test]
  async fn an_edit_made_outside_reviu_shows_up_without_any_event(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-poll");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert!(
        page.dock_panel.read(cx).status_entries().is_empty(),
        "a clean working tree has nothing to show"
      );
    });

    // Another editor writes the file: nothing tells the shell about it.
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("edit outside Reviu");
    cx.executor()
      .advance_clock(status_poll::ACTIVE_STATUS_POLL_INTERVAL);
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let paths = page
        .dock_panel
        .read(cx)
        .status_entries()
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();
      assert_eq!(paths, vec![PathBuf::from("README.md")]);
    });
  }
}
