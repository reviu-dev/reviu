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

/// Global entry point so other pages can route work into the sessions shell.
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
mod commands;
mod palette;
mod render;
mod repo;
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
    // One preference for every diff surface, the shell and PR Changes alike.
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
