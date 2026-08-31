//! Agent-first shell: sessions sidebar, conversation center, right dock.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use agent_chat_panel::{AgentChatPanel, AgentChatPanelEvent, ConversationStore, TurnGate};
use editor::{
  ConflictResolution, DiffViewMode, Editor, EditorEvent, ReviewCommentCancelHandler,
  ReviewCommentCreateHandler, ReviewCommentCreateRequest, ReviewCommentDeleteHandler,
  ReviewCommentEditHandler, ReviewCommentSendHandler,
};
use gpui::AnimationExt as _;
use gpui::{
  AnyElement, AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, PathPromptOptions,
  Render, SharedString, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Sizable as _, dialog::DialogFooter, h_flex,
  notification::Notification, v_flex,
};

use crate::agent_chat_state::{
  agent_chat_state_dir, agent_path_to_repo_relative, prune_agent_chat_state_once,
};
use crate::agent_review::{
  AgentReviewComments, ReviewSend, original_lines_for_request, sync_comments_to_editor,
};
use crate::agent_review_store::{read_review, review_path_for_repo, write_review};
use crate::agent_settings::AgentSettings;
use crate::auth_state::AuthStateStore;
use crate::config::ConfigStore;
use crate::conversation_hub::ConversationHub;
use crate::diff_view_policy::{DiffViewInputs, effective_diff_view};
use crate::dock_panel::{CommitMenuCommand, DockPanel, DockPanelEvent, DockPanelTab};
use crate::file_search_palette::open_file_search_palette;
use crate::file_view::{
  BinaryPreview, build_binary_preview, render_binary_preview, render_file_title_with_status,
};
use crate::inbox::Inbox;
use crate::navigation::NavigationHistory;
use crate::open_intent::OpenIntent;
use crate::review_destination::{AgentReviewHandlers, ReviewDestination, configure_review};
use crate::session_list::{SessionList, SessionListEvent, SessionStatus};
use crate::session_page::file_viewer::{OpenedSnapshot, UnsavedEditorAction};
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
use crate::pull_request_dialog::{GithubBranchContext, open_create_pull_request_dialog};
use crate::repo_command::{RepoCommand, RepoCommandOutcome, branch_ref_from_palette};
use crate::repo_snapshot::{RepoSnapshot, RepoSnapshotEvent};
use crate::repo_state::{
  PaletteCommand, RepoState, can_accept_all_conflicts, push_flags, should_publish_branch,
};
use crate::review_list::{ReviewSection, review_panel_comments};
use crate::status_poll;
use crate::svg_preview::SvgPreview;
use crate::workspace::WorkspaceApi;
use crate::{
  CloseWorkspacePage, CommentHunk, JumpToLatestMessage, SendReviewCommentsToAgent,
  ShowCommandPalette, ShowFileSearch,
};
use ui::{
  Button, ButtonVariants as _, CommandPalette, CommandPaletteAction, CommandPaletteCommand,
  CommandPaletteConfig, CommandPaletteHandler, CommandPaletteInitialScreen, CommandPalettePage,
  CommandPaletteRepository, ConfirmDialog, SearchFileEntry, SearchFileHandler, StatusThemeExt as _,
  UiIconName, WindowExt as _,
};

/// How long a browse waits before it loads. Long enough to cross a list without
/// reading every file, short enough not to feel late when you stop.
pub(crate) const BROWSE_DEBOUNCE: Duration = Duration::from_millis(100);

const DIFF_VIEW_TOGGLE_DEBUG_SELECTOR: &str = "session-diff-view-toggle";
const PREVIEW_TOGGLE_DEBUG_SELECTOR: &str = "session-preview-toggle";
const WHITESPACE_TOGGLE_DEBUG_SELECTOR: &str = "session-whitespace-toggle";
const SAVE_BUTTON_DEBUG_SELECTOR: &str = "session-save-file";
const ACCEPT_ALL_CURRENT_DEBUG_SELECTOR: &str = "session-accept-all-current";
const ACCEPT_ALL_INCOMING_DEBUG_SELECTOR: &str = "session-accept-all-incoming";
const ANNOTATION_COUNTER_DEBUG_SELECTOR: &str = "session-annotation-counter";
const INTERACTIVE_REBASE_DEBUG_SELECTOR: &str = "session-interactive-rebase";
const DIFF_EDITOR_DEBUG_SELECTOR: &str = "session-diff-editor";
const PREVIEW_PANE_DEBUG_SELECTOR: &str = "session-preview-pane";
const REPO_CONTEXT_DEBUG_SELECTOR: &str = "session-repo-context";
const REPO_SWITCH_DEBUG_SELECTOR: &str = "session-repo-switch";
const OPEN_REPOSITORY_ROW_DEBUG_SELECTOR: &str = "session-open-repository";
const UNSAVED_EDITOR_SAVE_DEBUG_SELECTOR: &str = "session-unsaved-editor-save";
const UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR: &str = "session-unsaved-editor-discard";
const UNSAVED_EDITOR_CANCEL_DEBUG_SELECTOR: &str = "session-unsaved-editor-cancel";
const REPO_AHEAD_DEBUG_SELECTOR: &str = "session-repo-ahead";
const REPO_BEHIND_DEBUG_SELECTOR: &str = "session-repo-behind";
const REPO_PUBLISH_DEBUG_SELECTOR: &str = "session-repo-publish";
const REPO_SYNC_LOADING_DEBUG_SELECTOR: &str = "session-repo-sync-loading";

const SESSIONS_SIDEBAR_DEFAULT_WIDTH: f32 = 250.0;
const SESSIONS_SIDEBAR_MIN_WIDTH: f32 = 200.0;
const SESSIONS_SIDEBAR_MAX_WIDTH: f32 = 420.0;
const CENTER_SWAP_FADE_MS: u64 = 180;
const CONVERSATION_SPLIT_DEFAULT_WIDTH: f32 = 420.0;
const CONVERSATION_SPLIT_MIN_WIDTH: f32 = 320.0;
const CONVERSATION_SPLIT_MAX_WIDTH: f32 = 640.0;
const DOCK_PANEL_DEFAULT_WIDTH: f32 = 320.0;
const DOCK_PANEL_MIN_WIDTH: f32 = 240.0;
const DOCK_PANEL_MAX_WIDTH: f32 = 560.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CenterView {
  Conversation,
  Diff,
  /// The todo of an interactive rebase, waiting to be applied.
  InteractiveRebase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepoCommandInFlight {
  Push,
  ForcePush,
  Publish,
  Pull,
  Fetch,
  Other,
}

impl RepoCommandInFlight {
  fn for_command(command: &RepoCommand, branch_status: Option<&git::BranchStatus>) -> Self {
    match command {
      RepoCommand::Push if branch_status.is_some_and(|status| !status.has_upstream) => {
        Self::Publish
      }
      RepoCommand::Push => Self::Push,
      RepoCommand::ForcePush => Self::ForcePush,
      RepoCommand::Pull => Self::Pull,
      RepoCommand::Fetch => Self::Fetch,
      _ => Self::Other,
    }
  }

  fn sync_label(self) -> Option<&'static str> {
    match self {
      Self::Push => Some("Pushing..."),
      Self::ForcePush => Some("Force pushing..."),
      Self::Publish => Some("Publishing..."),
      Self::Pull => Some("Pulling..."),
      Self::Fetch => Some("Fetching..."),
      Self::Other => None,
    }
  }
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
    f: impl FnOnce(&mut SessionPage, &mut Window, &mut Context<SessionPage>) + 'static,
  ) {
    let Some(page) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.page.clone())
      .and_then(|weak| weak.upgrade())
    else {
      return;
    };
    let window_handle = page.read(cx).window_handle;
    // Deferred: callers often sit inside this very window's update, where a
    // re-entrant `update_window` is a silent no-op.
    cx.defer(move |cx| {
      let _ = cx.update_window(window_handle, |_, window, cx| {
        page.update(cx, |page, cx| f(page, window, cx));
      });
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
}

/// The pin remembers which session it was set on: any other session shown
/// means the user moved on, and the dock follows them again.
struct CheckoutOverride {
  session_id: Option<String>,
  path: PathBuf,
}

#[derive(Clone)]
struct CheckoutInfo {
  path: PathBuf,
  branch: Option<String>,
}

pub struct SessionPage {
  focus_handle: FocusHandle,
  window_handle: AnyWindowHandle,
  agent_chat_view: Option<Entity<AgentChatPanel>>,
  /// All per-repo stores live here; everything cross-repo reads through it.
  conversation_hub: ConversationHub,
  /// The FALLBACK repo's store: where sessions land when nothing on screen
  /// names a repo. Sessions of other repos carry their own store handle.
  chat_store: Option<Entity<ConversationStore>>,
  /// Repos already swept for orphaned worktrees this run.
  swept_repos: HashSet<PathBuf>,
  /// The repo whose review batch is loaded; follows the active session.
  reviewed_repo: Option<PathBuf>,
  /// Sessions kept alive off screen so their agents keep working; MRU first.
  background_chat_panels: Vec<(String, Entity<AgentChatPanel>)>,
  turn_gate: TurnGate,
  agent_notification: Option<gpui::WindowHandle<crate::agent_notification::AgentNotification>>,
  dock_panel: Entity<DockPanel>,
  inbox: Entity<Inbox>,
  session_list: Entity<SessionList>,
  /// The repository's identity: recents, persistence keys, GitHub. The dock
  /// and the diff follow `checkout_root` instead, which may be a worktree.
  fallback_repo: Option<PathBuf>,
  /// What the git surfaces currently point at; compared to detect switches.
  synced_checkout: Option<PathBuf>,
  /// A checkout pinned from the dock header; view state, dies with the pinned
  /// session and never persists.
  checkout_override: Option<CheckoutOverride>,
  /// The shown session's repo checkouts (main first), refreshed in background.
  available_checkouts: Vec<CheckoutInfo>,
  _checkout_options_task: Option<Task<()>>,
  center: CenterView,
  editor: Option<Entity<Editor>>,
  binary_preview: Option<BinaryPreview>,
  selected_file: Option<PathBuf>,
  /// Set while the center shows a file as it was in a commit.
  /// The centre shows a read-only snapshot instead of a working-tree file:
  /// no staging, no hunk actions, no git status.
  opened_snapshot: Option<OpenedSnapshot>,
  interactive_rebase_todo_view: Option<Entity<InteractiveRebaseTodoView>>,
  _interactive_rebase_task: Option<Task<()>>,
  pub(crate) _merge_base_task: Option<Task<()>>,
  /// Mounting a real agent panel in a test would spawn an agent process.
  #[cfg(test)]
  pretend_agent_turn_in_flight: bool,
  /// A test has no agent to send to: the export a send built lands here.
  #[cfg(test)]
  last_review_export: Option<String>,
  #[cfg(any(test, feature = "test-support"))]
  driver_notifications: Vec<crate::DriverNotification>,
  open_file_generation: u64,
  open_file_task: Option<Task<()>>,
  agent_review: AgentReviewComments,
  /// Where this repository's batch is written; none without a repository.
  review_store_path: Option<PathBuf>,
  /// Tests point the batch at a temporary directory of their own; production
  /// reads the real state directory instead.
  review_state_dir: Option<PathBuf>,
  repo_snapshot: Entity<RepoSnapshot>,
  diff_view: DiffViewMode,
  hide_whitespace: bool,
  show_preview: bool,
  svg_preview: Entity<SvgPreview>,
  // Review export waiting for the agent connection to become ready.
  pending_review_export: Option<String>,
  repo_command_in_flight: Option<RepoCommandInFlight>,
  /// Set while pushing an unpublished branch on the way to the pull request form.
  pending_pull_request: Option<GithubBranchContext>,
  dock_open: bool,
  dock_width: f32,
  dock_zoomed: bool,
  diff_chat_open: bool,
  conversation_split_width: f32,
  sidebar_open: bool,
  sidebar_width: f32,
  sidebar_slide_armed: bool,
  /// The slide only plays on a real open/close, never on the first paint.
  dock_slide_armed: bool,
  poll_window_active: bool,
  _active_repo_task: Option<Task<()>>,
  _repo_command_task: Option<Task<()>>,
  _pull_request_link_task: Option<Task<()>>,
  _browse_task: Option<Task<()>>,
  _poll_task: Option<Task<()>>,
}

mod agent;
mod commands;
#[cfg(any(test, feature = "test-support"))]
mod driver;
mod file_viewer;
mod palette;
mod pull_request_link;
mod render;
mod repo;
mod review_github;
#[cfg(test)]
pub(crate) mod test_support;

impl SessionPage {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let fallback_repo = ConfigStore::load_recent_repositories()
      .first()
      .map(|repo| repo.path.clone());
    let dock_panel = cx.new(|cx| DockPanel::new(fallback_repo.clone(), window, cx));
    let inbox = cx.new(|_| Inbox::new());
    let repo_snapshot = cx.new(|_| RepoSnapshot::new(fallback_repo.clone()));
    cx.subscribe(
      &repo_snapshot,
      |this, snapshot, event: &RepoSnapshotEvent, cx| match event {
        RepoSnapshotEvent::Refreshed => {
          let branch_status = snapshot.read(cx).branch_status().cloned();
          this
            .dock_panel
            .update(cx, |panel, cx| panel.set_branch_status(branch_status, cx));
          cx.notify();
        }
      },
    )
    .detach();
    let session_list = cx.new(|_| SessionList::new());
    cx.subscribe_in(
      &session_list,
      window,
      |this, _list, event: &SessionListEvent, window, cx| match event {
        SessionListEvent::NewSessionIn { repo_root } => {
          this.new_session_in(repo_root.clone(), window, cx)
        }
        SessionListEvent::ToggleRepoCollapsed { repo_root } => {
          let repo_root = repo_root.clone();
          this
            .session_list
            .update(cx, |list, cx| list.toggle_repo_collapsed(&repo_root, cx));
        }
        SessionListEvent::NewWorktreeSessionIn { repo_root, base } => {
          this.new_worktree_session_in(repo_root.clone(), base.clone(), window, cx)
        }
        SessionListEvent::Collapse => this.close_sidebar(cx),
        SessionListEvent::Selected { id } => this.select_session(id, window, cx),
        SessionListEvent::Deleted { id } => this.delete_session(id, window, cx),
      },
    )
    .detach();
    let svg_preview = cx.new(|_| SvgPreview::new());
    // The SVG renders on a background task; repaint when it lands.
    cx.observe(&svg_preview, |_, _, cx| cx.notify()).detach();
    cx.subscribe_in(
      &dock_panel,
      window,
      |this, _panel, event: &DockPanelEvent, window, cx| match event {
        DockPanelEvent::OpenFile { path, intent } => {
          let (path, intent) = (path.clone(), *intent);
          this.open_for_intent(
            intent,
            move |this, window, cx| this.open_diff(path, None, intent, window, cx),
            window,
            cx,
          );
        }
        DockPanelEvent::OpenCommitFile {
          commit_oid,
          path,
          intent,
        } => {
          let (commit_oid, path, intent) = (commit_oid.clone(), path.clone(), *intent);
          this.open_for_intent(
            intent,
            move |this, window, cx| this.open_commit_file(commit_oid, path, intent, window, cx),
            window,
            cx,
          );
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
          // A save can empty the file's diff (or refill it): the split view
          // must follow what the file can actually show.
          this.sync_diff_view(cx);
          this.sync_git_telemetry(cx);
          this.refresh_checkout_options(cx);
        }
        DockPanelEvent::PinCheckout { path } => {
          this.pin_checkout(path.clone(), window, cx);
        }
        DockPanelEvent::FollowSessionCheckout => {
          this.follow_session_checkout(window, cx);
        }
        DockPanelEvent::ToggleZoom => {
          this.toggle_dock_zoom(cx);
        }
        DockPanelEvent::OpenReviewComment { path, line, intent } => {
          let (path, line, intent) = (path.clone(), *line as u32, *intent);
          this.open_for_intent(
            intent,
            move |this, window, cx| this.open_diff(path, Some(line), intent, window, cx),
            window,
            cx,
          );
        }
        DockPanelEvent::DeleteReviewComment { id } => {
          this.delete_agent_review_comment(*id, cx);
        }
        DockPanelEvent::DeletePullRequestReviewComment { id } => {
          this.confirm_github_review_comment_delete(*id, window, cx);
        }
        DockPanelEvent::OpenPullRequestFile {
          base_oid,
          head_oid,
          path,
          line,
          intent,
        } => {
          let (base_oid, head_oid, path, intent) =
            (base_oid.clone(), head_oid.clone(), path.clone(), *intent);
          let line = line.map(|line| line as u32);
          this.open_for_intent(
            intent,
            move |this, window, cx| {
              this.open_pull_request_file(base_oid, head_oid, path, line, intent, window, cx)
            },
            window,
            cx,
          );
        }
        DockPanelEvent::SendReviewComment { id } => {
          this.send_agent_review_comment_to_agent(*id, window, cx);
        }
        DockPanelEvent::SendReview => {
          this.send_agent_review_to_agent(window, cx);
        }
        DockPanelEvent::DiscardReview => {
          this.confirm_discard_agent_review(window, cx);
        }
        DockPanelEvent::PullRequestReviewCommentsChanged => {
          this.sync_github_review_comments(cx);
        }
        DockPanelEvent::PullRequestReviewCommentSubmitted { error } => {
          this.finish_github_review_comment(error.clone(), window, cx);
        }
        DockPanelEvent::SubmitPullRequestReview => {
          this.dock_panel.update(cx, |panel, cx| {
            panel.submit_pull_request_review(window, cx);
          });
        }
        DockPanelEvent::DiscardPullRequestReview => {
          this.dock_panel.update(cx, |panel, cx| {
            panel.discard_pull_request_review(window, cx);
          });
        }
      },
    )
    .detach();

    let mut page = Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      agent_chat_view: None,
      conversation_hub: ConversationHub::new(),
      chat_store: None,
      swept_repos: HashSet::new(),
      reviewed_repo: fallback_repo.clone(),
      background_chat_panels: Vec::new(),
      turn_gate: TurnGate::new(),
      agent_notification: None,
      dock_panel,
      inbox,
      session_list,
      synced_checkout: fallback_repo.clone(),
      fallback_repo,
      checkout_override: None,
      available_checkouts: Vec::new(),
      _checkout_options_task: None,
      center: CenterView::Conversation,
      editor: None,
      binary_preview: None,
      selected_file: None,
      opened_snapshot: None,
      interactive_rebase_todo_view: None,
      _interactive_rebase_task: None,
      _merge_base_task: None,
      #[cfg(test)]
      pretend_agent_turn_in_flight: false,
      #[cfg(test)]
      last_review_export: None,
      #[cfg(any(test, feature = "test-support"))]
      driver_notifications: Vec::new(),
      open_file_generation: 0,
      open_file_task: None,
      agent_review: AgentReviewComments::new(),
      review_store_path: None,
      review_state_dir: None,
      repo_snapshot,
      diff_view: DiffViewMode::Inline,
      hide_whitespace: false,
      show_preview: false,
      svg_preview,
      pending_review_export: None,
      repo_command_in_flight: None,
      pending_pull_request: None,
      dock_open: true,
      dock_width: DOCK_PANEL_DEFAULT_WIDTH,
      dock_zoomed: false,
      diff_chat_open: true,
      conversation_split_width: CONVERSATION_SPLIT_DEFAULT_WIDTH,
      sidebar_open: true,
      sidebar_width: SESSIONS_SIDEBAR_DEFAULT_WIDTH,
      sidebar_slide_armed: false,
      dock_slide_armed: false,
      poll_window_active: true,
      _active_repo_task: None,
      _repo_command_task: None,
      _pull_request_link_task: None,
      _browse_task: None,
      _poll_task: None,
    };
    SessionPageHandle::register(cx);
    // Bounded aggregation: the recent repos get their stores up front so the
    // all-repos sidebar can list them without touching anything else.
    for recent in ConfigStore::load_recent_repositories()
      .into_iter()
      .take(crate::conversation_hub::MAX_TRACKED_REPOS)
    {
      let _ = page.conversation_hub.store_for(&recent.path, cx);
    }
    page.reload_review_for_repo(cx);
    page.refresh_branch(cx);
    page.watch_window_activation(window, cx);
    page.start_polling(cx);
    // A link to a pull request has to find this shell, deep link included.
    crate::pull_request_surface::PullRequestSurfaceHandle::register(cx);
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
          let checkout = this.checkout_root(cx);
          if !status_poll::should_poll(
            this.poll_window_active,
            checkout.as_deref(),
            this.repo_command_in_flight.is_some(),
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
    if self.checkout_root(cx).is_none() {
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

  /// Per-notify sync: only the current conversation's row can change while
  /// the agent streams, so this never touches disk. Lifecycle changes go
  /// through `refresh_session_list`.
  fn sync_session_list(&mut self, cx: &mut Context<Self>) {
    let Some(panel) = self.agent_chat_view.as_ref() else {
      self.session_list.update(cx, |list, cx| {
        list.set_conversations(Vec::new(), String::new(), cx)
      });
      return;
    };
    let panel = panel.read(cx);
    let current_id = panel.current_conversation().id.clone();
    let loading_id = panel.loading_conversation_id().map(str::to_string);
    let existing_current_row = self
      .session_list
      .read(cx)
      .contains_conversation(&current_id);
    // An empty draft is not on disk; the sidebar must not invent a row for it.
    let current = (panel.has_persistable_content() || existing_current_row)
      .then(|| panel.current_conversation().clone());
    let current = current.map(|meta| crate::session_list::SessionRow {
      repo_root: panel.repo_root().to_path_buf(),
      meta,
    });
    let statuses = self.session_statuses(cx);
    self.session_list.update(cx, |list, cx| {
      list.set_loading(loading_id, cx);
      list.upsert_current(current, current_id, cx);
      list.set_statuses(statuses, cx);
    });
  }

  /// Live agent state per conversation: derived from the panels alive right
  /// now, background ones included. A session with no panel is Idle.
  fn session_statuses(&self, cx: &App) -> std::collections::HashMap<String, SessionStatus> {
    let status_of = |panel: &Entity<AgentChatPanel>| {
      let panel = panel.read(cx);
      let status = if panel.awaiting_permission() {
        SessionStatus::Waiting
      } else if panel.is_turn_in_flight() {
        SessionStatus::Working
      } else if panel.needs_reconnect() || panel.last_turn_failed() {
        SessionStatus::Failed
      } else {
        SessionStatus::Idle
      };
      (panel.current_conversation().id.clone(), status)
    };
    self
      .agent_chat_view
      .iter()
      .map(&status_of)
      .chain(
        self
          .background_chat_panels
          .iter()
          .map(|(_, panel)| status_of(panel)),
      )
      .filter(|(_, status)| *status != SessionStatus::Idle)
      .collect()
  }

  /// Full refresh from the store's meta index, for lifecycle changes
  /// (panel created, conversation created/loaded/deleted, repo switched).
  fn refresh_session_list(&mut self, cx: &mut Context<Self>) {
    let sections = self.conversation_hub.sections(cx);
    let section_order: Vec<PathBuf> = sections.iter().map(|(repo, _)| repo.clone()).collect();
    let conversations: Vec<crate::session_list::SessionRow> = sections
      .into_iter()
      .flat_map(|(repo, metas)| {
        metas
          .into_iter()
          .map(move |meta| crate::session_list::SessionRow {
            repo_root: repo.clone(),
            meta,
          })
          .collect::<Vec<_>>()
      })
      .collect();
    let current_id = self
      .agent_chat_view
      .as_ref()
      .map(|panel| panel.read(cx).current_conversation().id.clone())
      .unwrap_or_default();
    let statuses = self.session_statuses(cx);
    let worktree_branches = self.conversation_hub.worktree_branches(cx);
    self.session_list.update(cx, |list, cx| {
      list.set_conversations(conversations, current_id, cx);
      list.set_section_order(section_order, cx);
      list.set_statuses(statuses, cx);
      list.set_worktree_branches(worktree_branches, cx);
    });
  }

  fn refresh_branch(&mut self, cx: &mut Context<Self>) {
    self
      .repo_snapshot
      .update(cx, |snapshot, cx| snapshot.refresh(cx));
  }

  /// The checkout the git surfaces should show: a pinned one first, else the
  /// active session's worktree when it has one, the main checkout otherwise.
  pub(crate) fn checkout_root(&self, cx: &App) -> Option<PathBuf> {
    self.fallback_repo.as_ref()?;
    if let Some(pinned) = self.active_checkout_override(cx) {
      return Some(pinned);
    }
    self
      .agent_chat_view
      .as_ref()
      .map(|panel| panel.read(cx).cwd().to_path_buf())
      .or_else(|| self.fallback_repo.clone())
  }

  pub(super) fn target_checkout_differs_from_editor(
    &self,
    target_checkout: &Path,
    cx: &App,
  ) -> bool {
    self.checkout_root(cx).as_deref() != Some(target_checkout)
  }

  /// The pin holds only while the session it was set on stays shown.
  fn active_checkout_override(&self, cx: &App) -> Option<PathBuf> {
    let pin = self.checkout_override.as_ref()?;
    let shown_id = self
      .agent_chat_view
      .as_ref()
      .map(|panel| panel.read(cx).current_conversation().id.clone());
    (shown_id == pin.session_id).then(|| pin.path.clone())
  }

  /// The repo the shown session belongs to; the fallback repo only fills in
  /// while nothing is on screen. Everything that follows "where you are"
  /// (review batch, session creation, context row) derives from this one place.
  pub(super) fn session_repo(&self, cx: &App) -> Option<PathBuf> {
    self
      .agent_chat_view
      .as_ref()
      .map(|panel| panel.read(cx).repo_root().to_path_buf())
      .or_else(|| self.fallback_repo.clone())
  }

  /// Points the dock, the branch header and the diff at the active session's
  /// checkout. No-op while the checkout has not changed, so streaming and
  /// same-checkout switches cost nothing.
  pub(super) fn sync_active_checkout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    // A pin left behind by a session switch, or pointing at a deleted
    // worktree, is dead weight: the dock goes back to following the session.
    if let Some(pin) = self.checkout_override.as_ref()
      && (self.active_checkout_override(cx).is_none() || !pin.path.is_dir())
    {
      self.checkout_override = None;
    }
    let checkout = self.checkout_root(cx);
    // The memo alone is not trusted: a fixture or future code may assign
    // `fallback_repo` directly, so the dock's actual root double-checks it.
    if self.synced_checkout == checkout
      && self.dock_panel.read(cx).repo_root() == checkout.as_deref()
    {
      return;
    }
    self.synced_checkout = checkout.clone();
    // The open diff belongs to the checkout being left.
    self.close_diff(window, cx);
    self.center = CenterView::Conversation;
    self.editor = None;
    self.binary_preview = None;
    self.selected_file = None;
    self.opened_snapshot = None;
    self.open_file_task = None;
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    self.repo_snapshot.update(cx, |snapshot, cx| {
      snapshot.set_repo_root(checkout.clone(), cx)
    });
    self.dock_panel.update(cx, |panel, cx| {
      panel.set_repo_root(checkout, cx);
      panel.refresh(cx);
    });
    self.refresh_branch(cx);
    // The review batch belongs to a REPO (worktree sessions share their
    // repo's batch); it follows the active session across repos.
    let review_repo = self.session_repo(cx);
    if self.reviewed_repo != review_repo {
      self.persist_agent_review();
      self.reviewed_repo = review_repo;
      self.reload_review_for_repo(cx);
    }
    self.refresh_checkout_options(cx);
    self.push_checkout_selector(cx);
    cx.notify();
  }

  /// Pins the git surfaces on one of the repo's checkouts without touching
  /// the session; picking the session's own checkout just unpins.
  pub(super) fn pin_checkout(
    &mut self,
    path: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.editor_is_dirty(cx) && self.target_checkout_differs_from_editor(&path, cx) {
      self.open_unsaved_editor_dialog(UnsavedEditorAction::PinCheckout { path }, window, cx);
      return;
    }
    self.pin_checkout_without_unsaved_prompt(path, window, cx);
  }

  pub(super) fn pin_checkout_without_unsaved_prompt(
    &mut self,
    path: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let session_checkout = self
      .agent_chat_view
      .as_ref()
      .map(|panel| panel.read(cx).cwd().to_path_buf())
      .or_else(|| self.fallback_repo.clone());
    if session_checkout.as_deref() == Some(path.as_path()) {
      self.checkout_override = None;
    } else {
      let session_id = self
        .agent_chat_view
        .as_ref()
        .map(|panel| panel.read(cx).current_conversation().id.clone());
      self.checkout_override = Some(CheckoutOverride { session_id, path });
    }
    self.sync_active_checkout(window, cx);
    // The sync may no-op when the pin lands on the shown checkout; the
    // selector's pinned state still has to repaint.
    self.push_checkout_selector(cx);
    cx.notify();
  }

  pub(super) fn follow_session_checkout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let session_checkout = self
      .agent_chat_view
      .as_ref()
      .map(|panel| panel.read(cx).cwd().to_path_buf())
      .or_else(|| self.fallback_repo.clone());
    if let Some(session_checkout) = session_checkout
      && self.editor_is_dirty(cx)
      && self.target_checkout_differs_from_editor(&session_checkout, cx)
    {
      self.open_unsaved_editor_dialog(UnsavedEditorAction::FollowSessionCheckout, window, cx);
      return;
    }
    self.follow_session_checkout_without_unsaved_prompt(window, cx);
  }

  pub(super) fn follow_session_checkout_without_unsaved_prompt(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.checkout_override.take().is_none() {
      return;
    }
    self.sync_active_checkout(window, cx);
    self.push_checkout_selector(cx);
    cx.notify();
  }

  /// Lists the shown session's repo checkouts off the UI thread; the selector
  /// repaints when the listing lands.
  pub(super) fn refresh_checkout_options(&mut self, cx: &mut Context<Self>) {
    let Some(repo) = self.session_repo(cx) else {
      self.available_checkouts = Vec::new();
      self.push_checkout_selector(cx);
      return;
    };
    let list_repo = repo.clone();
    self._checkout_options_task = Some(cx.spawn(async move |this, cx| {
      let (main_branch, worktrees) = cx
        .background_spawn(async move {
          let main_branch = git::current_branch_status(&list_repo)
            .ok()
            .map(|status| status.name);
          let worktrees = git::list_worktrees(&list_repo).unwrap_or_default();
          (main_branch, worktrees)
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        let mut checkouts = vec![CheckoutInfo {
          path: repo,
          branch: main_branch,
        }];
        checkouts.extend(worktrees.into_iter().map(|worktree| CheckoutInfo {
          path: worktree.path,
          branch: worktree.branch,
        }));
        this.available_checkouts = checkouts;
        this.push_checkout_selector(cx);
      });
    }));
  }

  fn push_checkout_selector(&mut self, cx: &mut Context<Self>) {
    let displayed = self.checkout_root(cx);
    let pinned = self.active_checkout_override(cx).is_some();
    let options: Vec<crate::dock_panel::CheckoutOption> = self
      .available_checkouts
      .iter()
      .map(|checkout| crate::dock_panel::CheckoutOption {
        path: checkout.path.clone(),
        branch: checkout.branch.clone().map(SharedString::from),
        is_displayed: displayed.as_deref() == Some(checkout.path.as_path()),
      })
      .collect();
    self.dock_panel.update(cx, |panel, cx| {
      panel.set_checkout_selector(options, pinned, cx);
    });
  }

  /// What a crash or a git error should carry about where the user was.
  fn git_telemetry<'a>(&'a self, cx: &'a App) -> GitTelemetry<'a> {
    GitTelemetry {
      repo_root: self.synced_checkout.as_deref(),
      selected_file: self.selected_file.as_deref(),
      branch: self.repo_snapshot.read(cx).current_branch_name(),
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
    if self.repo_snapshot.read(cx).branches().is_empty() {
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

  fn open_files_action(
    &mut self,
    _: &crate::OpenFilesSidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_dock_tab(DockPanelTab::Files, window, cx);
  }

  fn open_review_action(
    &mut self,
    _: &crate::OpenReviewSidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_dock_tab(DockPanelTab::Review, window, cx);
  }

  fn open_pull_request_action(
    &mut self,
    _: &crate::OpenPullRequestSidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_dock_tab(DockPanelTab::PullRequest, window, cx);
  }

  /// Browsing waits: holding an arrow down must load the file it stops on, not
  /// every file it crosses. Choosing one never waits, and drops what was queued.
  fn open_for_intent(
    &mut self,
    intent: OpenIntent,
    open: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self._browse_task = None;
    if intent.takes_focus() {
      open(self, window, cx);
      return;
    }

    let window_handle = self.window_handle;
    self._browse_task = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(BROWSE_DEBOUNCE).await;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| open(this, window, cx));
      });
    }));
  }

  /// Escape hands the keyboard back to the work without closing the panel: the
  /// list keeps its place, and the tab's shortcut brings you back to it.
  fn return_focus_to_editor_action(
    &mut self,
    _: &crate::ReturnFocusToEditor,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    // A shell owns its own escape.
    if self.dock_panel.read(cx).active_tab() == DockPanelTab::Terminal {
      cx.propagate();
      return;
    }
    let handle = self.focus_handle(cx);
    window.focus(&handle, cx);
    cx.stop_propagation();
  }

  pub(crate) fn window_handle(&self) -> AnyWindowHandle {
    self.window_handle
  }

  /// Opens the dock on a tab without the toggle: something outside asked for
  /// this surface, so closing it would be the opposite of the answer.
  pub(crate) fn show_dock_tab(
    &mut self,
    tab: DockPanelTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !self.dock_open {
      self.dock_slide_armed = true;
    }
    self.dock_open = true;
    self
      .dock_panel
      .update(cx, |panel, cx| panel.open_tab(tab, window, cx));
    cx.notify();
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn open_file_for_driver(
    &mut self,
    rel_path: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    if self.checkout_root(cx).is_none() {
      return Err("No repository selected.".into());
    }
    self.open_diff(rel_path, None, OpenIntent::Open, window, cx);
    Ok(())
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn show_changes_for_driver(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.show_dock_tab(DockPanelTab::Changes, window, cx);
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn show_pull_request_for_driver(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.show_dock_tab(DockPanelTab::PullRequest, window, cx);
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn hide_dock_for_driver(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.close_dock(window, cx);
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn submit_agent_prompt_for_driver(
    &mut self,
    text: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(panel) = self.agent_chat_view.clone() else {
      return Err("No agent session is open.".into());
    };
    panel.update(cx, |panel, cx| {
      panel.submit_prompt_for_driver(&text, window, cx);
    });
    Ok(())
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn agent_stats_for_driver(&self, cx: &App) -> serde_json::Value {
    let active_in_flight = self
      .agent_chat_view
      .as_ref()
      .is_some_and(|panel| panel.read(cx).is_turn_in_flight());
    let active_ready = self
      .agent_chat_view
      .as_ref()
      .is_some_and(|panel| panel.read(cx).is_ready());
    let background_in_flight = self
      .background_chat_panels
      .iter()
      .filter(|(_, panel)| panel.read(cx).is_turn_in_flight())
      .count();
    let background_ready = self
      .background_chat_panels
      .iter()
      .filter(|(_, panel)| panel.read(cx).is_ready())
      .count();
    serde_json::json!({
      "active_in_flight": active_in_flight,
      "active_ready": active_ready,
      "background_count": self.background_chat_panels.len(),
      "background_in_flight": background_in_flight,
      "background_ready": background_ready,
      "total_in_flight": background_in_flight + usize::from(active_in_flight),
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn editor_stats_for_driver(&self, cx: &App) -> serde_json::Value {
    let selected_file = self
      .selected_file
      .as_ref()
      .map(|path| path.display().to_string());
    let Some(editor) = self.editor.as_ref() else {
      return serde_json::json!({
        "ready": false,
        "selected_file": selected_file,
      });
    };
    editor.read_with(cx, |editor, cx| {
      let document = editor.document().read(cx);
      let line_count = document.len_lines();
      let display_line_count = editor.display_line_count(line_count);
      serde_json::json!({
        "ready": editor.projection().is_some(),
        "selected_file": selected_file,
        "line_count": line_count,
        "display_line_count": display_line_count,
        "scroll_offset_y": editor.scroll_offset_y,
        "line_layout_cache_size": editor.line_layouts.len(),
        "virtual_line_layout_cache_size": editor.virtual_line_layouts.len(),
        "word_diff_cache_size": editor.word_diff_cache_size(),
      })
    })
  }

  /// The shortcut of a tab means "take me there". It only means "get out of the
  /// way" when the keyboard is already in that surface, which is what lets it
  /// bring you back from the editor.
  fn open_dock_tab(&mut self, tab: DockPanelTab, window: &mut Window, cx: &mut Context<Self>) {
    cx.stop_propagation();
    let panel = self.dock_panel.read(cx);
    if self.dock_open && panel.active_tab() == tab && panel.tab_has_focus(tab, window, cx) {
      self.close_dock(window, cx);
      return;
    }
    if !self.dock_open {
      self.dock_slide_armed = true;
    }
    self.dock_open = true;
    self
      .dock_panel
      .update(cx, |panel, cx| panel.open_tab(tab, window, cx));
    cx.notify();
  }

  fn close_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.dock_open {
      return;
    }
    self.dock_open = false;
    self.dock_slide_armed = true;
    self.set_dock_zoomed(false, cx);
    // The dock stays mounted while it slides out; focus must not stay in it.
    // The page resolves to the editor when the diff is open, the composer
    // otherwise, so shortcuts keep working right after the close.
    let view = cx.entity().downgrade();
    window.on_next_frame(move |window, cx| {
      let _ = view.update(cx, |this, cx| {
        let handle = this.focus_handle(cx);
        window.focus(&handle, cx);
      });
    });
    cx.notify();
  }

  fn close_sidebar(&mut self, cx: &mut Context<Self>) {
    if !self.sidebar_open {
      return;
    }
    self.sidebar_open = false;
    self.sidebar_slide_armed = true;
    cx.notify();
  }

  fn open_sidebar(&mut self, cx: &mut Context<Self>) {
    if self.sidebar_open {
      return;
    }
    self.sidebar_open = true;
    self.sidebar_slide_armed = true;
    cx.notify();
  }

  fn resize_sidebar(&mut self, width: f32, cx: &mut Context<Self>) {
    let clamped = width.clamp(SESSIONS_SIDEBAR_MIN_WIDTH, SESSIONS_SIDEBAR_MAX_WIDTH);
    if (clamped - self.sidebar_width).abs() > f32::EPSILON {
      self.sidebar_width = clamped;
      cx.notify();
    }
  }

  fn toggle_dock_zoom(&mut self, cx: &mut Context<Self>) {
    let zoomed = !self.dock_zoomed;
    self.set_dock_zoomed(zoomed, cx);
    cx.notify();
  }

  fn set_dock_zoomed(&mut self, zoomed: bool, cx: &mut Context<Self>) {
    self.dock_zoomed = zoomed;
    self
      .dock_panel
      .update(cx, |panel, cx| panel.set_zoomed(zoomed, cx));
  }

  fn resize_dock(&mut self, width: f32, cx: &mut Context<Self>) {
    let clamped = width.clamp(DOCK_PANEL_MIN_WIDTH, DOCK_PANEL_MAX_WIDTH);
    if (clamped - self.dock_width).abs() > f32::EPSILON {
      self.dock_width = clamped;
      cx.notify();
    }
  }

  fn resize_conversation_split(&mut self, width: f32, cx: &mut Context<Self>) {
    let clamped = width.clamp(CONVERSATION_SPLIT_MIN_WIDTH, CONVERSATION_SPLIT_MAX_WIDTH);
    if (clamped - self.conversation_split_width).abs() > f32::EPSILON {
      self.conversation_split_width = clamped;
      cx.notify();
    }
  }

  fn hide_diff_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.center != CenterView::Diff || !self.diff_chat_open {
      return;
    }
    self.diff_chat_open = false;
    self.sync_agent_chat_close_control(cx);
    self.focus_editor_on_next_frame(window, cx);
    cx.notify();
  }

  fn sync_agent_chat_close_control(&mut self, cx: &mut Context<Self>) {
    let Some(panel) = self.agent_chat_view.clone() else {
      return;
    };
    let visible = self.center == CenterView::Diff && self.diff_chat_open;
    panel.update(cx, |panel, cx| panel.set_close_control_visible(visible, cx));
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
    let Some(repo_root) = self.checkout_root(cx) else {
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
        view.open_diff(path, None, OpenIntent::Open, window, cx);
      });
      Ok(())
    });
    open_file_search_palette(window, cx, entries, handler, false);
  }
}

/// Under test there is no fallback to the real state directory: a test writes
/// where it says it does, or nowhere at all.
fn review_store_path_for(repo: Option<&Path>, test_state_dir: Option<&Path>) -> Option<PathBuf> {
  let repo = repo?;
  let state_dir = if cfg!(test) {
    test_state_dir?.to_path_buf()
  } else {
    agent_chat_state_dir()?
  };
  Some(review_path_for_repo(&state_dir, repo))
}

fn load_agent_review(path: Option<&Path>) -> AgentReviewComments {
  match path.and_then(read_review) {
    Some(stored) => AgentReviewComments::restored(stored.comments, stored.next_id),
    None => AgentReviewComments::new(),
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

  #[gpui::test]
  async fn agent_attention_pops_a_window_only_while_inactive(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-agent-notify");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    let windows_before = cx.update(|_, cx| cx.windows().len());

    // Test windows are never platform-active, which is exactly the state the
    // popup exists for; the active-window early return has no simulator.
    cx.deactivate_window();
    page.update_in(cx, |page, window, cx| {
      page.notify_agent_attention("Reviu agent finished", None, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
      cx.update(|_, cx| cx.windows().len()),
      windows_before + 1,
      "an inactive window grows the popup"
    );

    // A newer notification replaces the old one instead of stacking.
    page.update_in(cx, |page, window, cx| {
      page.notify_agent_attention("Reviu agent needs a decision", None, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), windows_before + 1);

    page.update(cx, |page, cx| page.dismiss_agent_notification(cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), windows_before);
  }

  #[gpui::test]
  async fn accepting_the_popup_closes_it(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-agent-notify-accept");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    let windows_before = cx.update(|_, cx| cx.windows().len());

    cx.deactivate_window();
    page.update_in(cx, |page, window, cx| {
      page.notify_agent_attention("Reviu agent finished", None, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), windows_before + 1);

    let handle = page.read_with(cx, |page, _| page.agent_notification.expect("popup handle"));
    let _ = cx.update(|_, cx| {
      handle.update(cx, |_, _, cx| {
        cx.emit(crate::agent_notification::AgentNotificationEvent::Accepted);
      })
    });
    cx.run_until_parked();

    assert_eq!(
      cx.update(|_, cx| cx.windows().len()),
      windows_before,
      "accepting closes the popup"
    );
    page.read_with(cx, |page, _| {
      assert!(page.agent_notification.is_none());
    });
  }

  #[gpui::test]
  async fn the_notification_setting_gates_the_popup(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-agent-notify-off");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    cx.update(|_, cx| {
      let mut settings = crate::config::AppSettings::get(cx);
      settings.agent_notifications = false;
      cx.set_global(settings);
    });
    let windows_before = cx.update(|_, cx| cx.windows().len());

    cx.deactivate_window();
    page.update_in(cx, |page, window, cx| {
      page.notify_agent_attention("Reviu agent finished", None, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
      cx.update(|_, cx| cx.windows().len()),
      windows_before,
      "the setting keeps the popup away"
    );
  }

  #[gpui::test(iterations = 10)]
  async fn committing_in_the_shell_updates_the_ahead_counter(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-ahead-counter");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let _remote = publish_to_new_remote(&repo.path, "session-page-ahead-counter");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.read_with(cx, |page, cx| {
      let snapshot = page.repo_snapshot.read(cx);
      let status = snapshot.branch_status().expect("branch status");
      assert_eq!(status.ahead, 0);
      assert_eq!(status.behind, 0);
      assert!(status.has_upstream);
    });

    // A commit made from the shell must show up as something to push.
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let snapshot = page.repo_snapshot.read(cx);
      assert_eq!(snapshot.branch_status().expect("status").ahead, 1);
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
    page.read_with(cx, |page, cx| {
      let snapshot = page.repo_snapshot.read(cx);
      assert_eq!(snapshot.branch_status().expect("status").ahead, 1);
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
    assert_eq!(head.summary().expect("read summary"), Some("second"));

    page.read_with(cx, |page, cx| {
      let snapshot = page.repo_snapshot.read(cx);
      assert_eq!(snapshot.branch_status().expect("status").ahead, 0);
    });
  }

  #[gpui::test]
  async fn without_a_repository_the_sidebar_asks_for_one(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-open-repository");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window_without_repo(cx);
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(page.fallback_repo.is_none()));
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
    page.read_with(cx, |page, _| assert!(page.fallback_repo.is_none()));
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
      assert_eq!(page.fallback_repo.as_deref(), Some(repo.path.as_path()));
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

  #[gpui::test(iterations = 10)]
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

  #[gpui::test(iterations = 10)]
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

    page.read_with(cx, |page, cx| {
      assert_eq!(
        page
          .repo_snapshot
          .read(cx)
          .current_branch_name()
          .map(str::to_string),
        Some("feature".to_string()),
        "the poll follows the branch, not just the changed files"
      );
    });
  }

  #[gpui::test(iterations = 10)]
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

  #[gpui::test]
  async fn a_link_to_the_open_branch_pull_request_shows_its_panel(cx: &mut TestAppContext) {
    let repo = TempRepo::init("pull-request-surface-link");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(surface_pull_request(42), cx);
      });
    });
    cx.run_until_parked();

    // Case does not decide whether a link is yours.
    assert!(cx.update(
      |_, cx| crate::pull_request_surface::PullRequestSurfaceHandle::show(
        "ACME", "Widget", 42, None, cx
      )
    ));
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert!(page.dock_open);
      assert_eq!(
        page.dock_panel.read(cx).active_tab(),
        DockPanelTab::PullRequest
      );
    });
  }

  #[gpui::test]
  async fn a_link_to_another_pull_request_has_no_home_here(cx: &mut TestAppContext) {
    let repo = TempRepo::init("pull-request-surface-other-link");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(surface_pull_request(42), cx);
      });
    });
    cx.run_until_parked();

    // Another number, another repository: neither is what this branch proposes.
    assert!(!cx.update(
      |_, cx| crate::pull_request_surface::PullRequestSurfaceHandle::show(
        "acme", "widget", 7, None, cx
      )
    ));
    assert!(!cx.update(
      |_, cx| crate::pull_request_surface::PullRequestSurfaceHandle::show(
        "acme", "other", 42, None, cx
      )
    ));

    // And a branch that lost its pull request stops claiming links.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(crate::dock_panel::BranchPrState::NoRemote, cx);
      });
    });
    assert!(!cx.update(
      |_, cx| crate::pull_request_surface::PullRequestSurfaceHandle::show(
        "acme", "widget", 42, None, cx
      )
    ));
  }
}
