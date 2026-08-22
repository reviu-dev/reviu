//! Agent-first shell: sessions sidebar, conversation center, right dock.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use agent_chat_panel::{AgentChatPanel, AgentChatPanelEvent};
use editor::{
  ConflictResolution, DiffViewMode, Editor, EditorEvent, ReviewCommentCancelHandler,
  ReviewCommentCreateHandler, ReviewCommentCreateRequest, ReviewCommentDeleteHandler,
  ReviewCommentDisplayMode, ReviewCommentEditHandler, ReviewCommentSendHandler,
};
use gpui::AnimationExt as _;
use gpui::{
  AnyElement, AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, PathPromptOptions,
  Render, SharedString, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Sizable as _, h_flex, notification::Notification, v_flex,
};

use crate::active_local_repo::{ActiveLocalRepoStore, active_local_repo_snapshot};
use crate::agent_chat_state::{
  agent_chat_state_dir, agent_path_to_repo_relative, prune_agent_chat_state_once,
};
use crate::agent_review::{
  AgentReviewComments, ReviewSend, original_lines_for_request, sync_comments_to_editor,
};
use crate::agent_review_store::{read_review, review_path_for_repo, write_review};
use crate::review_destination::{AgentReviewHandlers, ReviewDestination, configure_review};
use crate::agent_settings::AgentSettings;
use crate::auth_state::AuthStateStore;
use crate::config::ConfigStore;
use crate::diff_view_policy::{DiffViewInputs, effective_diff_view};
use crate::dock_panel::{CommitMenuCommand, DockPanel, DockPanelEvent, DockPanelTab};
use crate::file_search_palette::open_file_search_palette;
use crate::file_view::{
  BinaryPreview, build_binary_preview, render_binary_preview, render_file_title_with_status,
};
use crate::inbox::Inbox;
use crate::navigation::NavigationHistory;
use crate::session_list::{SessionList, SessionListEvent};
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
use crate::repo_snapshot::{RepoSnapshot, RepoSnapshotEvent};
use crate::repo_state::{PaletteCommand, RepoState, can_accept_all_conflicts, push_flags};
use crate::review_list::review_panel_comments;
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
  agent_notification: Option<gpui::WindowHandle<crate::agent_notification::AgentNotification>>,
  dock_panel: Entity<DockPanel>,
  inbox: Entity<Inbox>,
  session_list: Entity<SessionList>,
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
  /// A test has no agent to send to: the export a send built lands here.
  #[cfg(test)]
  last_review_export: Option<String>,
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
  repo_command_in_flight: bool,
  pro_teaser_shown: bool,
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
  _pro_teaser_task: Option<Task<()>>,
  _repo_command_task: Option<Task<()>>,
  _poll_task: Option<Task<()>>,
}

mod agent;
mod commands;
mod file_viewer;
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
    let inbox = cx.new(|_| Inbox::new());
    let repo_snapshot = cx.new(|_| RepoSnapshot::new(selected_repo.clone()));
    cx.subscribe(
      &repo_snapshot,
      |this, snapshot, event: &RepoSnapshotEvent, cx| match event {
        RepoSnapshotEvent::Refreshed => {
          let branch_status = snapshot.read(cx).branch_status().cloned();
          this
            .dock_panel
            .update(cx, |panel, cx| panel.set_branch_status(branch_status, cx));
          this.publish_active_local_repo(cx);
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
        SessionListEvent::NewSession => this.new_session(window, cx),
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
          // A save can empty the file's diff (or refill it): the split view
          // must follow what the file can actually show.
          this.sync_diff_view(cx);
          this.sync_git_telemetry(cx);
        }
        DockPanelEvent::ToggleZoom => {
          this.toggle_dock_zoom(cx);
        }
        DockPanelEvent::OpenReviewComment { path, line } => {
          this.open_diff(path.clone(), Some(*line as u32), window, cx);
        }
        DockPanelEvent::DeleteReviewComment { id } => {
          this.delete_agent_review_comment(*id, cx);
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
      },
    )
    .detach();

    let mut page = Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      agent_chat_view: None,
      agent_notification: None,
      dock_panel,
      inbox,
      session_list,
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
      #[cfg(test)]
      last_review_export: None,
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
      repo_command_in_flight: false,
      pro_teaser_shown: false,
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
      _pro_teaser_task: None,
      _repo_command_task: None,
      _poll_task: None,
    };
    SessionPageHandle::register(cx);
    page.reload_review_for_repo(cx);
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
    // An empty draft is not on disk; the sidebar must not invent a row for it.
    let current = panel
      .has_persistable_content()
      .then(|| panel.current_conversation().clone());
    self.session_list.update(cx, |list, cx| {
      list.set_loading(loading_id, cx);
      list.upsert_current(current, current_id, cx);
    });
  }

  /// Full refresh from the store's meta index, for lifecycle changes
  /// (panel created, conversation created/loaded/deleted, repo switched).
  fn refresh_session_list(&mut self, cx: &mut Context<Self>) {
    let (conversations, current_id) = match self.agent_chat_view.as_ref() {
      Some(panel) => {
        let panel = panel.read(cx);
        (
          panel.list_conversations(cx),
          panel.current_conversation().id.clone(),
        )
      }
      None => (Vec::new(), String::new()),
    };
    self.session_list.update(cx, |list, cx| {
      list.set_conversations(conversations, current_id, cx)
    });
  }

  fn refresh_branch(&mut self, cx: &mut Context<Self>) {
    self
      .repo_snapshot
      .update(cx, |snapshot, cx| snapshot.refresh(cx));
  }

  /// The pull request page reads this to know which repository is open here,
  /// which is how it offers to switch branch or resolve conflicts.
  fn publish_active_local_repo(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      ActiveLocalRepoStore::set(cx, None);
      return;
    };
    let branch_name = self
      .repo_snapshot
      .read(cx)
      .branch_status()
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

  /// What a crash or a git error should carry about where the user was.
  fn git_telemetry<'a>(&'a self, cx: &'a App) -> GitTelemetry<'a> {
    GitTelemetry {
      repo_root: self.selected_repo.as_deref(),
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

  /// A closed dock opens on the tab; the active tab's shortcut closes it.
  fn open_dock_tab(&mut self, tab: DockPanelTab, window: &mut Window, cx: &mut Context<Self>) {
    cx.stop_propagation();
    if self.dock_open && self.dock_panel.read(cx).active_tab() == tab {
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
      page.notify_agent_attention("Reviu agent finished", window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
      cx.update(|_, cx| cx.windows().len()),
      windows_before + 1,
      "an inactive window grows the popup"
    );

    // A newer notification replaces the old one instead of stacking.
    page.update_in(cx, |page, window, cx| {
      page.notify_agent_attention("Reviu agent needs a decision", window, cx);
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
      page.notify_agent_attention("Reviu agent finished", window, cx);
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
      page.notify_agent_attention("Reviu agent finished", window, cx);
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
}
