use std::{
  collections::{HashMap, HashSet},
  path::{Path, PathBuf},
  rc::Rc,
  sync::Arc,
  time::{Duration, Instant},
};

use agent_chat_panel::AgentChatPanel;
use editor::{
  CloseFind, ConflictNavigationDirection, ConflictNavigationState, ConflictResolution,
  DiffViewMode, Editor, Find, HunkAction, HunkNavigationDirection, HunkState,
  ReviewCommentCancelHandler, ReviewCommentCreateHandler, ReviewCommentCreateRequest,
  ReviewCommentDeleteHandler, ReviewCommentDisplayMode, ReviewCommentEditHandler,
};
use git::{
  BranchKind, BranchRef, BranchStatus, CommitChangedFile, CommitFileChangeKind, HeadCommitStatus,
  HistoryCommitNode, HistoryRevision, InteractiveRebaseTarget, InteractiveRebaseTodoEntry,
  MergeBranchOutcome, PullOutcome, RebaseBranchOutcome, RepoStage, RepoStatusEntry, RepoStatusKind,
  abort_merge, abort_rebase, amend_commit, apply_stash, branch_has_unpublished_commits,
  checkout_detached_target, cherry_pick_commits, commit_changes, continue_rebase, create_branch,
  create_branch_from, create_stash, current_branch_status, current_branch_upstream,
  current_github_remote_repo, current_head_sha, current_history_revision,
  current_rebase_commit_message, default_remote_branch, default_stash_message, delete_branch,
  delete_untracked_file, detached_head_label, diff_set_from_patch, drop_stash, fetch,
  head_commit_status, is_merge_in_progress, is_rebase_in_progress, list_branches,
  list_commit_changed_files, list_commit_history, list_interactive_rebase_commits,
  list_repo_head_files, list_repo_status, list_stashes, load_commit_file_diff, merge_branch,
  pop_stash, pull, push, rebase_branch, resolve_branch_ref, restore_file, restore_renamed_file,
  skip_rebase, stage_all, stage_file, start_interactive_rebase, switch_branch, undo_last_commit,
  unstage_all, unstage_file,
};
use gpui::{
  Anchor, AnyElement, AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, Global,
  InteractiveElement, ParentElement, PathPromptOptions, Pixels, Render, SharedString, Styled,
  Subscription, Task, WeakEntity, Window, actions, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, IndexPath, Selectable, Sizable, StyledExt,
  button::{Button, ButtonGroup, ButtonVariant, ButtonVariants as _},
  checkbox::Checkbox,
  dialog::{DialogDescription, DialogFooter, DialogHeader, DialogTitle},
  h_flex,
  input::InputEvent,
  kbd::Kbd,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  menu::{DropdownMenu, PopupMenuItem},
  notification::Notification,
  select::{Select, SelectEvent, SelectState},
  spinner::Spinner,
  tag::Tag,
  text::TextView,
  tooltip::Tooltip,
  tree::{TreeItem, TreeState, tree},
  v_flex,
};
use sentry::protocol::{Map, Value};
use smol::unblock;

pub(crate) fn agent_chat_state_dir() -> Option<std::path::PathBuf> {
  Some(dirs::config_dir()?.join("reviu").join("agent-chats"))
}

/// Agent tool-call locations are absolute; the diff view opens by repo-relative path.
pub(crate) fn agent_path_to_repo_relative(path: PathBuf, repo_root: Option<&Path>) -> PathBuf {
  repo_root
    .and_then(|root| path.strip_prefix(root).ok())
    .map(Path::to_path_buf)
    .unwrap_or(path)
}

pub(crate) fn prune_agent_chat_state_once() {
  use std::sync::OnceLock;
  static PRUNED: OnceLock<()> = OnceLock::new();
  PRUNED.get_or_init(|| {
    if let Some(dir) = agent_chat_state_dir() {
      let _ =
        AgentChatPanel::prune_old_state(&dir, std::time::Duration::from_secs(60 * 60 * 24 * 30));
    }
  });
}
use terminal::TerminalView;

use crate::{
  active_local_repo::{ActiveLocalRepo, ActiveLocalRepoStore},
  api::{ApiClient, GithubPullRequest},
  auth_state::{AuthState, AuthStateStore},
  config::{AppSettings, ConfigStore, RecentRepository},
  file_preview::{is_markdown_path, is_previewable_path, is_svg_path},
  file_search_palette::open_file_search_palette as open_shared_file_search_palette,
  file_view::{BinaryPreview, build_binary_preview, render_binary_preview},
  github_navigation::{
    open_commit_target, open_profile_target, open_repo_target, should_open_externally,
  },
  github_notifications::GithubNotificationsStore,
  github_pr_details_page::GithubPrDetailsPageHandle,
  github_shared,
  interactive_rebase_todo_view::{
    InteractiveRebaseTodoView, InteractiveRebaseTodoViewCancelHandler,
    InteractiveRebaseTodoViewConfig, InteractiveRebaseTodoViewHandler,
  },
  navigation::NavigationHistory,
  sentry_context,
  shortcuts::{self, ShortcutId},
  workspace::WorkspaceApi,
};
use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteBranch, CommandPaletteBranchKind,
  CommandPaletteCommand, CommandPaletteConfig, CommandPaletteHandler, CommandPaletteInitialScreen,
  CommandPalettePage, CommandPaletteRepository, CommandPaletteStash, ConfirmDialog,
  DropdownSelectConfig, DropdownSelectItem, FILE_ICON_SIZE_PX, Input, InputState,
  PAGE_HEADER_HEIGHT, SearchFileEntry, SearchFileHandler, SelectableRowStyle, StatusAlert,
  StatusThemeExt, Textarea, TextareaState, UiIconName, WindowExt, dropdown_select,
  file_icon_path_for_path_with_theme, selectable_list_item,
};

mod command_palette;
mod commit;
mod file_list;
mod history;
mod pull_request_dialog;
mod rebase;
mod remote;
mod render;
mod review_comments;
mod staging;
#[cfg(test)]
mod test_support;

pub(crate) use pull_request_dialog::open_create_pull_request_dialog;

use file_list::{GitFileListDelegate, format_git_file_name_label, render_repo_status_label};
use history::{HistoryCommitFileRow, HistoryRenderRow, HistoryTreeNode};

const SIDEBAR_DEFAULT_WIDTH: f32 = 400.0;
const SIDEBAR_MIN_WIDTH: f32 = 250.0;
const SIDEBAR_MAX_WIDTH: f32 = 1500.0;
const STATUS_POLL_INTERVAL_MS: u64 = 3_000;
const INACTIVE_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(60);
const UNPUBLISHED_BRANCH_RECHECK_INTERVAL: Duration = Duration::from_secs(30);
const EDITOR_HEADER_HEIGHT: f32 = 40.0;
const HISTORY_MAX_COMMITS: usize = 200;
const HISTORY_AUTHOR_MAX_WIDTH: f32 = 180.0;
const DETACHED_BRANCH_SELECT_SENTINEL: &str = "__reviu_detached_head__";
const TRIGGER_DROPDOWN_SELECT_WIDTH: f32 = 350.0;
const EMPTY_REPOSITORY_TITLE: &str = "Select a repository";
const EMPTY_REPOSITORY_HINT_PREFIX: &str = "Press";
const EMPTY_REPOSITORY_HINT_SUFFIX: &str = "to add a repository.";
const GIT_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR: &str = "git-markdown-preview-editor-pane";
const GIT_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR: &str = "git-markdown-preview-render-pane";
const GIT_TERMINAL_BUTTON_DEBUG_SELECTOR: &str = "git-terminal-button";
const GIT_TERMINAL_SIDEBAR_DEBUG_SELECTOR: &str = "git-terminal-sidebar";
const TERMINAL_SIDEBAR_DEFAULT_WIDTH: f32 = 480.0;
const TERMINAL_SIDEBAR_MIN_WIDTH: f32 = 320.0;
const TERMINAL_SIDEBAR_MAX_WIDTH: f32 = 1200.0;
type RepoSelectHandler = Rc<dyn Fn(PathBuf, &mut Window, &mut App)>;
type BranchSelectHandler = Rc<dyn Fn(BranchRef, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileStageButtonAction {
  Stage,
  Unstage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GithubBranchContext {
  pub owner: String,
  pub repo: String,
  pub branch: String,
}

/// Invoked after the dialog successfully creates a pull request.
pub(crate) type PullRequestCreatedHandler =
  Rc<dyn Fn(&GithubBranchContext, &GithubPullRequest, &mut gpui::App)>;

fn git_page_created_handler(git_page: WeakEntity<GitPage>) -> PullRequestCreatedHandler {
  Rc::new(move |context, pull_request, cx| {
    let _ = git_page.update(cx, |git_page, cx| {
      git_page.apply_created_pull_request(context, pull_request, cx);
    });
  })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GitBranchPullRequestButtonState {
  Hidden,
  LockedPro,
  Checking,
  PublishAndCreate,
  OpenExisting {
    owner: String,
    repo: String,
    number: u64,
  },
  Create,
}

struct GitBranchSwitchNotificationId;
struct GitActionErrorNotificationId;
struct GitProPushHintNotificationId;

#[derive(Clone, Debug, PartialEq, Eq)]
enum GitPageOpenAction {
  MergeBaseBranch { base_branch_name: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveConflictResolutionSnapshot {
  merge_in_progress: bool,
  rebase_in_progress: bool,
  conflicted_path: Option<PathBuf>,
}

enum GitPageOpenActionResult {
  ResumeActiveConflict(ActiveConflictResolutionSnapshot),
  MergeBaseBranchReady(BranchRef),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitCommitPrimaryButtonState {
  ContinueRebase,
  Commit,
  PublishBranch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnnotationKind {
  Conflict,
  Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AnnotationNavigationState {
  active_index: usize,
  total: usize,
  kind: AnnotationKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnnotationDirection {
  Previous,
  Next,
}

impl AnnotationDirection {
  fn conflict(self) -> ConflictNavigationDirection {
    match self {
      Self::Previous => ConflictNavigationDirection::Previous,
      Self::Next => ConflictNavigationDirection::Next,
    }
  }

  fn hunk(self) -> HunkNavigationDirection {
    match self {
      Self::Previous => HunkNavigationDirection::Previous,
      Self::Next => HunkNavigationDirection::Next,
    }
  }
}

fn git_refresh_in_progress(status_loading: bool, branch_loading: bool) -> bool {
  status_loading || branch_loading
}

#[derive(Clone, Default)]
pub struct GitPageHandle {
  git_page: Option<WeakEntity<GitPage>>,
}

impl Global for GitPageHandle {}

impl GitPageHandle {
  pub fn register(cx: &mut Context<GitPage>) {
    cx.set_global(Self {
      git_page: Some(cx.entity().downgrade()),
    });
  }

  pub fn is_refreshing(cx: &App) -> bool {
    let Some(weak) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.git_page.clone())
    else {
      return false;
    };

    weak
      .read_with(cx, |this, _cx| {
        git_refresh_in_progress(
          this.status_refresh_in_progress,
          this.branch_refresh_in_progress,
        )
      })
      .unwrap_or(false)
  }

  pub fn refresh_page(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.refresh_current_page(cx));
  }

  pub fn show_repository_and_merge_base(
    repo_root: PathBuf,
    base_branch_name: String,
    cx: &mut App,
  ) {
    NavigationHistory::navigate("/git", cx);

    let Some(weak) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.git_page.clone())
    else {
      return;
    };

    let _ = weak.update(cx, |this, cx| {
      this.open_repository_with_action(
        repo_root,
        GitPageOpenAction::MergeBaseBranch { base_branch_name },
        cx,
      );
    });
  }
}

struct GitCommandPaletteContents {
  commands: Vec<CommandPaletteCommand>,
  branches: Vec<CommandPaletteBranch>,
  rebase_branches: Vec<CommandPaletteBranch>,
  delete_branches: Vec<CommandPaletteBranch>,
  stashes: Vec<CommandPaletteStash>,
  default_stash_message: Option<SharedString>,
}

actions!(
  workspace,
  [
    OpenRepository,
    SaveFile,
    ShowCommandPalette,
    ShowFileSearch,
    CommitChanges
  ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitSidebarMode {
  Changes,
  History,
}

#[derive(Clone)]
struct RecentRepoItem {
  path: PathBuf,
  name: SharedString,
  prefix: SharedString,
  is_selected: bool,
  is_action: bool,
}

impl RecentRepoItem {
  fn new(repo: &RecentRepository, selected_repo: Option<&Path>) -> Self {
    let label = repo.path.to_string_lossy().replace(['\n', '\r'], "");
    let name = repo
      .path
      .file_name()
      .and_then(|name| name.to_str())
      .unwrap_or(label.as_str())
      .replace(['\n', '\r'], "");
    let prefix = label.strip_suffix(name.as_str()).unwrap_or("").to_string();
    Self {
      path: repo.path.clone(),
      name: name.into(),
      prefix: prefix.into(),
      is_selected: selected_repo.is_some_and(|selected| selected == repo.path.as_path()),
      is_action: false,
    }
  }

  // Sentinel item (empty path) that triggers the open-repository picker on select.
  fn open_action() -> Self {
    Self {
      path: PathBuf::new(),
      name: "Open repository…".into(),
      prefix: SharedString::default(),
      is_selected: false,
      is_action: true,
    }
  }
}

impl DropdownSelectItem for RecentRepoItem {
  type Value = PathBuf;

  fn value(&self) -> &Self::Value {
    &self.path
  }

  fn selected(&self) -> bool {
    self.is_selected
  }

  fn matches(&self, query: &str) -> bool {
    if self.is_action {
      return true;
    }

    let query = query.trim();
    if query.is_empty() {
      return true;
    }

    let lowered_query = query.to_lowercase();
    self.name.to_lowercase().contains(&lowered_query)
      || self.prefix.to_lowercase().contains(&lowered_query)
  }

  fn render_item(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
    if self.is_action {
      return h_flex()
        .min_w_0()
        .items_center()
        .gap_1()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::FolderOpen).size_3())
        .child(self.name.clone())
        .into_any_element();
    }

    h_flex()
      .min_w_0()
      .overflow_hidden()
      .items_center()
      .max_w(px(TRIGGER_DROPDOWN_SELECT_WIDTH - 40.0))
      .gap_0()
      .text_sm()
      .child(
        div()
          .text_ellipsis_start()
          .overflow_hidden()
          .text_color(cx.theme().muted_foreground)
          .child(self.prefix.clone()),
      )
      .child(div().flex_shrink(1.).child(self.name.clone()))
      .into_any_element()
  }

  fn render_selected(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
    h_flex()
      .min_w_0()
      .items_center()
      .text_sm()
      .gap_0()
      .child(
        div()
          .overflow_hidden()
          .text_ellipsis_start()
          .text_color(cx.theme().muted_foreground)
          .child(self.prefix.clone()),
      )
      .child(
        div()
          .flex_shrink(1.)
          .text_color(cx.theme().foreground)
          .child(self.name.clone()),
      )
      .into_any_element()
  }
}

#[derive(Clone)]
struct BranchSelectItem {
  branch: BranchRef,
  label: SharedString,
  is_current: bool,
}

impl BranchSelectItem {
  fn new(branch: BranchRef, is_current: bool) -> Self {
    let label: SharedString = branch.name.clone().into();
    Self {
      branch,
      label,
      is_current,
    }
  }

  fn detached(label: SharedString, is_current: bool) -> Self {
    Self {
      branch: GitPage::detached_branch_select_value(),
      label,
      is_current,
    }
  }
}

impl DropdownSelectItem for BranchSelectItem {
  type Value = BranchRef;

  fn value(&self) -> &Self::Value {
    &self.branch
  }

  fn selected(&self) -> bool {
    self.is_current
  }

  fn matches(&self, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
      return true;
    }

    self.label.to_lowercase().contains(&query.to_lowercase())
  }

  fn render_item(&self, _window: &mut Window, _cx: &mut App) -> AnyElement {
    div()
      .min_w_0()
      .max_w(px(TRIGGER_DROPDOWN_SELECT_WIDTH - 40.0))
      .flex_1()
      .overflow_hidden()
      .text_sm()
      .text_ellipsis()
      .child(self.label.clone())
      .into_any_element()
  }

  fn render_selected(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
    div()
      .min_w_0()
      .flex_1()
      .overflow_hidden()
      .text_sm()
      .text_color(cx.theme().foreground)
      .text_ellipsis()
      .child(self.label.clone())
      .into_any_element()
  }
}

#[derive(Clone, Default)]
pub struct AuthCallbackTarget {
  git_page: Option<WeakEntity<GitPage>>,
}

impl Global for AuthCallbackTarget {}

impl AuthCallbackTarget {
  pub fn register_git_page(cx: &mut Context<GitPage>) {
    cx.set_global(Self {
      git_page: Some(cx.entity().downgrade()),
    });
  }

  pub fn handle_auth_code(code: String, cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.handle_auth_code(code, cx));
  }

  pub fn start_sign_in(cx: &mut App, source: &'static str) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.start_github_sign_in(source, cx));
  }

  pub fn sign_out(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.logout(cx));
  }

  pub fn refresh_me(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.refresh_auth_state(cx));
  }

  pub fn handle_subscription_callback(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.handle_subscription_callback(cx));
  }
}

pub struct GitPage {
  focus_handle: FocusHandle,
  history_tree_wrapper_focus: FocusHandle,
  api: ApiClient,
  repo_dropdown_items: Vec<RecentRepoItem>,
  branch_dropdown_items: Vec<BranchSelectItem>,
  file_list: Entity<ListState<GitFileListDelegate>>,
  history_tree: Entity<TreeState>,
  window_handle: AnyWindowHandle,
  selected_repo: Option<PathBuf>,
  status_entries: Vec<RepoStatusEntry>,
  branch_status: Option<BranchStatus>,
  has_head_commit: bool,
  can_undo_last_commit: bool,
  can_push: bool,
  can_force_push: bool,
  has_unpublished_branch_commits: bool,
  unpublished_branch_check_key: Option<UnpublishedBranchCheckKey>,
  unpublished_branch_checked_at: Option<Instant>,
  force_push_after_rebase: bool,
  push_pull_in_progress: bool,
  publish_branch_and_create_pr_in_progress: bool,
  pro_push_hint_shown: bool,
  fetch_in_progress: bool,
  has_staged_changes: bool,
  merge_in_progress: bool,
  rebase_in_progress: bool,
  sidebar_mode: GitSidebarMode,
  history_commits: Vec<HistoryCommitNode>,
  history_revision: Option<HistoryRevision>,
  history_loading: bool,
  history_expanded_commit_oids: HashSet<String>,
  history_commit_files: HashMap<String, Vec<HistoryCommitFileRow>>,
  history_commit_files_loading: HashSet<String>,
  pending_history_file_loads: HashSet<String>,
  history_opened_commit_file: Option<(String, PathBuf)>,
  history_rows_cache: Vec<HistoryRenderRow>,
  history_tree_nodes: HashMap<String, HistoryTreeNode>,
  selected_file: Option<PathBuf>,
  selected_file_source: Option<SelectedFileSource>,
  selected_file_index_hint: Option<IndexPath>,
  select_first_file_after_restore: bool,
  force_list_selection: bool,
  editor: Option<Entity<Editor>>,
  terminal_view: Entity<TerminalView>,
  interactive_rebase_todo_view: Option<Entity<InteractiveRebaseTodoView>>,
  diff_view: DiffViewMode,
  hide_whitespace: bool,
  git_unified_file_view: bool,
  show_markdown_preview: bool,
  show_terminal_sidebar: bool,
  agent_review: AgentReviewComments,
  binary_preview: Option<BinaryPreview>,
  svg_preview: Entity<SvgPreview>,
  branch_pr_lookup_context: Option<GithubBranchContext>,
  branch_pr_lookup_result: Option<GithubPullRequest>,
  branch_pr_lookup_loading: bool,
  pending_open_action: Option<GitPageOpenAction>,
  pending_conflict_reveal_path: Option<PathBuf>,
  auth_state: AuthState,
  auth_task: Option<Task<()>>,
  branch_pr_lookup_task: Option<Task<()>>,
  open_file_task: Option<Task<()>>,
  status_task: Option<Task<()>>,
  status_refresh_in_progress: bool,
  history_task: Option<Task<()>>,
  history_files_task: Option<Task<()>>,
  history_open_file_task: Option<Task<()>>,
  branch_task: Option<Task<()>>,
  branch_refresh_in_progress: bool,
  branch_pr_lookup_generation: u64,
  open_file_generation: u64,
  status_refresh_generation: u64,
  branch_refresh_generation: u64,
  poll_task: Option<Task<()>>,
  poll_window_active: bool,
  commit_input: Entity<TextareaState>,
  operation_error: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SelectedFileUpdate {
  clear_selection: bool,
  sync_diff_view: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectedFileSource {
  StatusEntry,
  ProjectFile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UnpublishedBranchCheckKey {
  repo_root: PathBuf,
  branch_name: String,
  ahead: usize,
  behind: usize,
  has_upstream: bool,
  head_sha: Option<String>,
}

use crate::agent_review::AgentReviewComments;
use crate::diff_view_policy::{DiffViewInputs, effective_diff_view};
use crate::svg_preview::SvgPreview;

impl GitPage {
  fn sidebar_mode_tag(mode: GitSidebarMode) -> &'static str {
    match mode {
      GitSidebarMode::Changes => "changes",
      GitSidebarMode::History => "history",
    }
  }

  fn diff_view_tag(diff_view: DiffViewMode) -> &'static str {
    match diff_view {
      DiffViewMode::Inline => "inline",
      DiffViewMode::Split => "split",
    }
  }

  fn active_diff_view_tag(&self) -> &'static str {
    if self.show_markdown_preview
      && self
        .selected_file
        .as_ref()
        .is_some_and(|path| is_previewable_path(path))
    {
      "markdown_preview"
    } else {
      Self::diff_view_tag(self.diff_view)
    }
  }

  fn github_branch_context_from_active_repo(
    local_repo: &ActiveLocalRepo,
  ) -> Option<GithubBranchContext> {
    let owner = local_repo.github_owner.as_deref()?.trim();
    let repo = local_repo.github_repo.as_deref()?.trim();
    let branch = local_repo.current_branch.as_deref()?.trim();

    if owner.is_empty() || repo.is_empty() || branch.is_empty() || branch == "HEAD" {
      return None;
    }

    Some(GithubBranchContext {
      owner: owner.to_string(),
      repo: repo.to_string(),
      branch: branch.to_string(),
    })
  }

  fn github_branch_context(&self, cx: &App) -> Option<GithubBranchContext> {
    ActiveLocalRepoStore::get(cx)
      .as_ref()
      .and_then(Self::github_branch_context_from_active_repo)
  }

  fn branch_has_github_upstream(branch_status: Option<&BranchStatus>) -> bool {
    matches!(
      branch_status,
      Some(status) if status.has_upstream && !Self::is_detached_head(Some(status))
    )
  }

  fn branch_pr_lookup_context(&self, cx: &App) -> Option<GithubBranchContext> {
    (AuthStateStore::has_github_access(cx)
      && Self::branch_has_github_upstream(self.branch_status.as_ref()))
    .then(|| self.github_branch_context(cx))
    .flatten()
  }

  fn branch_pr_button_state(
    branch_context: Option<&GithubBranchContext>,
    can_open_in_app: bool,
    has_github_upstream: bool,
    can_publish_branch: bool,
    lookup_loading: bool,
    lookup_result: Option<&GithubPullRequest>,
  ) -> GitBranchPullRequestButtonState {
    let Some(_branch_context) = branch_context else {
      return GitBranchPullRequestButtonState::Hidden;
    };

    if !can_open_in_app {
      return GitBranchPullRequestButtonState::LockedPro;
    }

    if !has_github_upstream {
      return if can_publish_branch {
        GitBranchPullRequestButtonState::PublishAndCreate
      } else {
        GitBranchPullRequestButtonState::Hidden
      };
    }

    if lookup_loading {
      return GitBranchPullRequestButtonState::Checking;
    }

    if let Some(pull_request) = lookup_result {
      return GitBranchPullRequestButtonState::OpenExisting {
        owner: pull_request.repository.owner.clone(),
        repo: pull_request.repository.repo.clone(),
        number: pull_request.number,
      };
    }

    GitBranchPullRequestButtonState::Create
  }

  fn create_pull_request_branch_context(&self, cx: &App) -> Option<GithubBranchContext> {
    let branch_context = self.github_branch_context(cx)?;

    matches!(
      Self::branch_pr_button_state(
        Some(&branch_context),
        AuthStateStore::has_github_access(cx),
        Self::branch_has_github_upstream(self.branch_status.as_ref()),
        Self::should_publish_branch_and_create_pull_request(
          self.branch_status.as_ref(),
          self.has_unpublished_branch_commits,
        ),
        self.branch_pr_lookup_loading,
        self.branch_pr_lookup_result.as_ref(),
      ),
      GitBranchPullRequestButtonState::Create
    )
    .then_some(branch_context)
  }

  fn current_branch_pr_button_state(&self, cx: &App) -> GitBranchPullRequestButtonState {
    let branch_context = self.github_branch_context(cx);
    Self::branch_pr_button_state(
      branch_context.as_ref(),
      AuthStateStore::has_github_access(cx),
      Self::branch_has_github_upstream(self.branch_status.as_ref()),
      Self::should_publish_branch_and_create_pull_request(
        self.branch_status.as_ref(),
        self.has_unpublished_branch_commits,
      ),
      self.branch_pr_lookup_loading,
      self.branch_pr_lookup_result.as_ref(),
    )
  }

  fn should_apply_created_pull_request(
    active_context: Option<&GithubBranchContext>,
    created_context: &GithubBranchContext,
  ) -> bool {
    active_context == Some(created_context)
  }

  fn apply_created_pull_request(
    &mut self,
    created_context: &GithubBranchContext,
    pull_request: &GithubPullRequest,
    cx: &mut Context<Self>,
  ) {
    if !Self::should_apply_created_pull_request(
      self.branch_pr_lookup_context(cx).as_ref(),
      created_context,
    ) {
      return;
    }

    self.branch_pr_lookup_generation = self.branch_pr_lookup_generation.wrapping_add(1);
    self.branch_pr_lookup_task = None;
    self.branch_pr_lookup_context = Some(created_context.clone());
    self.branch_pr_lookup_result = Some(pull_request.clone());
    self.branch_pr_lookup_loading = false;
    cx.notify();
  }

  fn publish_branch_and_create_pull_request_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let Some(branch_context) = self.github_branch_context(cx) else {
      return;
    };
    if !AuthStateStore::has_github_access(cx)
      || !Self::should_publish_branch_and_create_pull_request(
        self.branch_status.as_ref(),
        self.has_unpublished_branch_commits,
      )
      || self.push_pull_in_progress
      || self.publish_branch_and_create_pr_in_progress
    {
      return;
    }

    let api = self.api.clone();
    let window_handle = self.window_handle;
    self.add_git_breadcrumb("Publish branch and create PR started", Map::new());
    self.push_pull_in_progress = true;
    self.publish_branch_and_create_pr_in_progress = true;

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || push(&repo_root, false)).await;
      let _ = this.update(cx, |this, cx| {
        this.push_pull_in_progress = false;
        this.publish_branch_and_create_pr_in_progress = false;

        match result {
          Ok(()) => {
            this.force_push_after_rebase = false;
            this.add_git_breadcrumb("Publish branch and create PR succeeded", Map::new());
            this.reload_status(cx);
            let on_created = git_page_created_handler(cx.entity().downgrade());
            let _ = cx.update_window(window_handle, |_, window, cx| {
              open_create_pull_request_dialog(
                api.clone(),
                window_handle,
                on_created,
                branch_context.clone(),
                window,
                cx,
              );
            });
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Publish branch and create PR failed", data.clone());
            this.record_git_unexpected_error(
              "git.publish_and_create_pr",
              error_message.as_str(),
              data,
            );
            this.push_git_action_error_notification(
              "Publish branch failed",
              error_message.into(),
              cx,
            );
            this.reload_status(cx);
          }
        }
      });
    });

    self.status_task = Some(task);
  }

  fn sentry_git_data(&self) -> Map<String, Value> {
    let mut data = Map::new();
    if let Some(repo_root) = self.selected_repo.as_deref() {
      let (repo_name, repo_hash) = sentry_context::sanitize_repo_path(repo_root);
      data.insert("repo_name".into(), repo_name.into());
      data.insert("repo_hash".into(), repo_hash.into());
    }
    if let Some(selected_file) = self.selected_file.as_deref() {
      let file = selected_file.to_string_lossy().replace(['\n', '\r'], "");
      data.insert("selected_file".into(), file.into());
    }
    if let Some(branch) = self
      .branch_status
      .as_ref()
      .map(|status| status.name.clone())
    {
      data.insert("branch".into(), branch.into());
    }
    data.insert(
      "sidebar_mode".into(),
      Self::sidebar_mode_tag(self.sidebar_mode).into(),
    );
    data.insert("diff_view".into(), self.active_diff_view_tag().into());
    data
  }

  fn add_git_breadcrumb(&self, message: &str, mut data: Map<String, Value>) {
    let base = self.sentry_git_data();
    for (key, value) in base {
      data.entry(key).or_insert(value);
    }
    sentry_context::add_breadcrumb("git.action", message, data);
  }

  fn record_git_unexpected_error(
    &self,
    op: &'static str,
    error: &str,
    mut data: Map<String, Value>,
  ) {
    let base = self.sentry_git_data();
    for (key, value) in base {
      data.entry(key).or_insert(value);
    }
    let io_error = std::io::Error::other(error.to_string());
    sentry_context::capture_unexpected_error(op, &io_error, data);
  }

  fn record_git_expected_error(&self, operation: &str, reason: &str, mut data: Map<String, Value>) {
    let base = self.sentry_git_data();
    for (key, value) in base {
      data.entry(key).or_insert(value);
    }
    sentry_context::record_expected_error(operation, reason, data);
  }

  fn sync_sentry_git_context(&self) {
    sentry_context::sync_git_context(
      self.selected_repo.as_deref(),
      self.selected_file.as_deref(),
      self
        .branch_status
        .as_ref()
        .map(|status| status.name.as_str()),
      Self::sidebar_mode_tag(self.sidebar_mode),
      self.active_diff_view_tag(),
    );
  }

  fn sync_active_local_repo(&self, cx: &mut Context<Self>) {
    let snapshot = self.selected_repo.as_ref().map(|repo_root| {
      let github_remote = current_github_remote_repo(repo_root).ok().flatten();
      ActiveLocalRepo {
        repo_root: repo_root.clone(),
        github_owner: github_remote.as_ref().map(|remote| remote.owner.clone()),
        github_repo: github_remote.as_ref().map(|remote| remote.repo.clone()),
        current_branch: self
          .branch_status
          .as_ref()
          .map(|status| status.name.clone()),
        head_sha: current_head_sha(repo_root).ok().flatten(),
        has_uncommitted_changes: !self.status_entries.is_empty(),
      }
    });
    ActiveLocalRepoStore::set(cx, snapshot);
  }

  fn clear_branch_pr_lookup(&mut self) {
    self.branch_pr_lookup_task = None;
    self.branch_pr_lookup_context = None;
    self.branch_pr_lookup_result = None;
    self.branch_pr_lookup_loading = false;
    self.branch_pr_lookup_generation = self.branch_pr_lookup_generation.wrapping_add(1);
  }

  fn refresh_branch_pr_lookup_if_needed(&mut self, cx: &mut Context<Self>) {
    let next_context = self.branch_pr_lookup_context(cx);
    if self.branch_pr_lookup_context.as_ref() == next_context.as_ref() {
      return;
    }

    self.refresh_branch_pr_lookup(cx);
  }

  fn refresh_branch_pr_lookup(&mut self, cx: &mut Context<Self>) {
    let next_context = self.branch_pr_lookup_context(cx);
    self.branch_pr_lookup_generation = self.branch_pr_lookup_generation.wrapping_add(1);
    let generation = self.branch_pr_lookup_generation;
    self.branch_pr_lookup_task = None;
    self.branch_pr_lookup_context = next_context.clone();
    self.branch_pr_lookup_result = None;
    self.branch_pr_lookup_loading = false;

    let Some(context) = next_context else {
      cx.notify();
      return;
    };

    self.branch_pr_lookup_loading = true;
    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let context_for_fetch = context.clone();
      let result = unblock(move || {
        api.fetch_pull_request_for_branch(
          context_for_fetch.owner.as_str(),
          context_for_fetch.repo.as_str(),
          context_for_fetch.branch.as_str(),
        )
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        if this.branch_pr_lookup_generation != generation
          || this.branch_pr_lookup_context.as_ref() != Some(&context)
        {
          return;
        }

        this.branch_pr_lookup_task = None;
        this.branch_pr_lookup_loading = false;
        this.branch_pr_lookup_result = result.ok().flatten();
        cx.notify();
      });
    });

    self.branch_pr_lookup_task = Some(task);
    cx.notify();
  }

  fn should_refresh_file_list(sidebar_mode: GitSidebarMode) -> bool {
    sidebar_mode == GitSidebarMode::Changes
  }

  fn branch_name_changed(previous: Option<&BranchStatus>, next: Option<&BranchStatus>) -> bool {
    previous.map(|status| status.name.as_str()) != next.map(|status| status.name.as_str())
  }

  fn has_staged_changes(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| matches!(entry.stage, RepoStage::Staged | RepoStage::PartiallyStaged))
  }

  fn selected_file_update(
    selected_file: Option<&Path>,
    selected_file_source: Option<SelectedFileSource>,
    status_entries: &[RepoStatusEntry],
    has_history_file_selection: bool,
    sync_diff_when_selected_retained: bool,
  ) -> SelectedFileUpdate {
    if has_history_file_selection {
      return SelectedFileUpdate::default();
    }

    let Some(selected_file) = selected_file else {
      return SelectedFileUpdate::default();
    };

    let is_selected_file_present = status_entries
      .iter()
      .any(|entry| entry.path.as_path() == selected_file);
    if !is_selected_file_present {
      if selected_file_source == Some(SelectedFileSource::ProjectFile) {
        return SelectedFileUpdate {
          clear_selection: false,
          sync_diff_view: sync_diff_when_selected_retained,
        };
      }

      return SelectedFileUpdate {
        clear_selection: true,
        sync_diff_view: false,
      };
    }

    SelectedFileUpdate {
      clear_selection: false,
      sync_diff_view: sync_diff_when_selected_retained,
    }
  }

  fn selected_branch_from_status(current: Option<&BranchStatus>) -> Option<BranchRef> {
    current.map(|status| {
      if Self::is_detached_head(Some(status)) {
        Self::detached_branch_select_value()
      } else {
        BranchRef {
          name: status.name.clone(),
          kind: BranchKind::Local,
        }
      }
    })
  }

  fn is_detached_head(branch_status: Option<&BranchStatus>) -> bool {
    branch_status.is_some_and(|status| status.name == "HEAD")
  }

  fn detached_branch_select_value() -> BranchRef {
    BranchRef {
      name: DETACHED_BRANCH_SELECT_SENTINEL.to_string(),
      kind: BranchKind::Local,
    }
  }

  fn is_detached_branch_select_value(branch: &BranchRef) -> bool {
    branch.kind == BranchKind::Local && branch.name == DETACHED_BRANCH_SELECT_SENTINEL
  }

  fn branch_select_items(
    branches: Vec<BranchRef>,
    selected: Option<&BranchRef>,
    detached_label: Option<&str>,
  ) -> Vec<BranchSelectItem> {
    let mut items = branches
      .into_iter()
      .map(|branch| {
        let is_current = selected == Some(&branch);
        BranchSelectItem::new(branch, is_current)
      })
      .collect::<Vec<_>>();

    if selected.is_some_and(Self::is_detached_branch_select_value) {
      let label = detached_label
        .map(|label| format!("HEAD ({label})"))
        .unwrap_or_else(|| "HEAD (detached)".to_string());
      items.insert(0, BranchSelectItem::detached(label.into(), true));
    }

    items
  }

  fn should_apply_branch_refresh(
    selected_repo: Option<&Path>,
    requested_repo: &Path,
    current_generation: u64,
    refresh_generation: u64,
  ) -> bool {
    selected_repo == Some(requested_repo) && current_generation == refresh_generation
  }

  fn should_apply_status_refresh(
    selected_repo: Option<&Path>,
    requested_repo: &Path,
    current_generation: u64,
    refresh_generation: u64,
  ) -> bool {
    selected_repo == Some(requested_repo) && current_generation == refresh_generation
  }

  fn advance_status_refresh_generation(&mut self) -> u64 {
    self.status_refresh_generation = self.status_refresh_generation.wrapping_add(1);
    self.status_refresh_generation
  }

  fn current_status_refresh_generation(&self) -> u64 {
    self.status_refresh_generation
  }

  fn status_poll_interval(window_active: bool) -> Duration {
    if window_active {
      Duration::from_millis(STATUS_POLL_INTERVAL_MS)
    } else {
      INACTIVE_STATUS_POLL_INTERVAL
    }
  }

  fn should_poll_status(
    window_active: bool,
    selected_repo: Option<&Path>,
    status_refresh_in_progress: bool,
  ) -> bool {
    window_active && selected_repo.is_some() && !status_refresh_in_progress
  }

  fn unpublished_branch_check_key(
    repo_root: &Path,
    branch_status: &BranchStatus,
    head_sha: Option<String>,
  ) -> UnpublishedBranchCheckKey {
    UnpublishedBranchCheckKey {
      repo_root: repo_root.to_path_buf(),
      branch_name: branch_status.name.clone(),
      ahead: branch_status.ahead,
      behind: branch_status.behind,
      has_upstream: branch_status.has_upstream,
      head_sha,
    }
  }

  fn should_recheck_unpublished_branch(
    next_key: &UnpublishedBranchCheckKey,
    cached_key: Option<&UnpublishedBranchCheckKey>,
    force_recheck: bool,
  ) -> bool {
    force_recheck || cached_key != Some(next_key)
  }

  fn resolve_polled_unpublished_branch_commits(
    repo_root: &Path,
    branch_status: Option<&BranchStatus>,
    cached_key: Option<&UnpublishedBranchCheckKey>,
    cached_value: bool,
    force_recheck: bool,
  ) -> Option<(bool, Option<UnpublishedBranchCheckKey>, bool)> {
    let Some(branch_status) = branch_status else {
      return Some((false, None, true));
    };

    if Self::is_detached_head(Some(branch_status)) {
      return Some((false, None, true));
    }

    if branch_status.has_upstream {
      let next_key = Self::unpublished_branch_check_key(repo_root, branch_status, None);
      return Some((branch_status.ahead > 0, Some(next_key), true));
    }

    let head_sha = current_head_sha(repo_root).ok().flatten();
    let next_key = Self::unpublished_branch_check_key(repo_root, branch_status, head_sha);
    if !Self::should_recheck_unpublished_branch(&next_key, cached_key, force_recheck) {
      return Some((cached_value, Some(next_key), false));
    }

    let has_unpublished_branch_commits = branch_has_unpublished_commits(repo_root).ok()?;
    Some((has_unpublished_branch_commits, Some(next_key), true))
  }

  fn should_refresh_editor_for_path(selected_file: Option<&Path>, rel_path: &Path) -> bool {
    selected_file == Some(rel_path)
  }

  fn first_conflicted_path(repo_root: &Path) -> Option<PathBuf> {
    list_repo_status(repo_root)
      .ok()?
      .into_iter()
      .find(|entry| entry.status == RepoStatusKind::Conflicted)
      .map(|entry| entry.path)
  }

  fn open_editor_has_unresolved_conflict_markers(&self, cx: &App) -> bool {
    self.editor.as_ref().is_none_or(|editor| {
      editor.read_with(cx, |editor, cx| editor.has_unresolved_conflict_markers(cx))
    })
  }

  fn apply_status_snapshot(
    &mut self,
    entries: Vec<RepoStatusEntry>,
    branch_status: Option<BranchStatus>,
    head_status: Option<HeadCommitStatus>,
    has_unpublished_branch_commits: bool,
    merge_in_progress: bool,
    rebase_in_progress: bool,
    rebase_commit_message: Option<String>,
    sync_diff_when_selected_retained: bool,
    cx: &mut Context<Self>,
  ) -> bool {
    let was_rebase_in_progress = self.rebase_in_progress;
    self.status_entries = entries;
    let branch_changed =
      Self::branch_name_changed(self.branch_status.as_ref(), branch_status.as_ref());
    self.branch_status = branch_status;
    if branch_changed
      || (self.force_push_after_rebase
        && self
          .branch_status
          .as_ref()
          .is_some_and(|status| status.ahead == 0))
    {
      self.force_push_after_rebase = false;
    }
    self.merge_in_progress = merge_in_progress;
    self.rebase_in_progress = rebase_in_progress;
    if !rebase_in_progress {
      self.operation_error = None;
    }
    self.sync_rebase_commit_input(
      was_rebase_in_progress,
      rebase_in_progress,
      rebase_commit_message,
      cx,
    );
    self.has_staged_changes = Self::has_staged_changes(&self.status_entries);
    let head_status = head_status.unwrap_or(HeadCommitStatus {
      has_head_commit: false,
      can_undo_last_commit: false,
    });
    self.has_head_commit = head_status.has_head_commit;
    self.can_undo_last_commit = head_status.can_undo_last_commit;
    self.has_unpublished_branch_commits = has_unpublished_branch_commits;
    let (can_push, can_force_push) = Self::push_flags(
      self.branch_status.as_ref(),
      self.has_head_commit,
      self.force_push_after_rebase,
    );
    self.can_push = can_push;
    self.can_force_push = can_force_push;
    self.sync_active_local_repo(cx);
    self.refresh_branch_pr_lookup_if_needed(cx);

    let selected_file_update = Self::selected_file_update(
      self.selected_file.as_deref(),
      self.selected_file_source,
      &self.status_entries,
      self.history_opened_commit_file.is_some(),
      sync_diff_when_selected_retained,
    );
    if selected_file_update.clear_selection {
      self.invalidate_open_file_task();
      self.selected_file = None;
      self.selected_file_source = None;
      self.editor = None;
      self.binary_preview = None;
      self.ensure_page_shortcut_focus(cx);
    } else if selected_file_update.sync_diff_view {
      self.sync_diff_view(cx);
    }

    self.sync_editor_unmerged_state(cx);

    if self.select_first_file_after_restore {
      self.select_first_file_after_restore = false;
      if let Some(first_path) = self.status_entries.first().map(|entry| entry.path.clone()) {
        self.open_status_file(first_path, cx);
      }
    }

    self.sync_sentry_git_context();

    branch_changed
  }

  fn split_disabled_for_path(&self, rel_path: &Path) -> bool {
    if self.selected_file.as_deref() == Some(rel_path) && self.binary_preview.is_some() {
      return true;
    }

    if let Some((commit_oid, selected_path)) = self.history_opened_commit_file.as_ref()
      && selected_path == rel_path
      && let Some(status) = self.history_file_status_kind(commit_oid, rel_path)
    {
      return matches!(
        status,
        RepoStatusKind::Untracked | RepoStatusKind::Added | RepoStatusKind::Deleted
      );
    }

    self.status_entries.iter().any(|entry| {
      entry.path == rel_path
        && matches!(
          entry.status,
          RepoStatusKind::Untracked | RepoStatusKind::Added | RepoStatusKind::Deleted
        )
    })
  }

  fn selected_file_is_markdown(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|path| is_markdown_path(path))
      .unwrap_or(false)
  }

  fn selected_file_is_svg(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|path| is_svg_path(path))
      .unwrap_or(false)
  }

  fn effective_diff_view_for_path(&self, path: &Path) -> DiffViewMode {
    effective_diff_view(DiffViewInputs {
      preferred: self.diff_view,
      binary_preview: self.selected_file.as_deref() == Some(path) && self.binary_preview.is_some(),
      previewing: self.show_markdown_preview && is_previewable_path(path),
      whole_file_change: self.split_disabled_for_path(path),
    })
  }

  fn sync_diff_view(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let diff_view = if let Some(path) = self.selected_file.as_ref() {
      self.effective_diff_view_for_path(path)
    } else {
      self.diff_view
    };
    let hide_ws = self.hide_whitespace;
    editor.update(cx, |editor, cx| {
      editor.set_diff_view_mode(diff_view, cx);
      editor.set_ignore_whitespace(hide_ws, cx);
    });
  }

  fn selected_file_index(&self, cx: &Context<Self>) -> Option<IndexPath> {
    let selected = self.selected_file.as_ref()?;
    let delegate = self.file_list.read(cx).delegate();
    if let Some(hint) = self.selected_file_index_hint
      && let Some(row) = delegate.row_at(hint)
      && row.entry.path == *selected
    {
      return Some(hint);
    }
    delegate.find_index_for_path(selected)
  }

  fn set_file_list_selected_index(&self, index: Option<IndexPath>, cx: &mut Context<Self>) {
    let file_list = self.file_list.clone();
    let window_handle = self.window_handle;
    let _ = cx.update_window(window_handle, move |_, window, cx| {
      file_list.update(cx, |state, cx| {
        state.set_selected_index(index, window, cx);
      });
    });
  }

  fn refresh_file_list(&mut self, cx: &mut Context<Self>) {
    self.git_unified_file_view = crate::config::AppSettings::get(cx).git_unified_file_view;
    let rows = self.status_entries.clone();
    let split_sections = !self.git_unified_file_view;
    let opened_path = self.selected_file.clone();
    self.file_list.update(cx, |state, cx| {
      state.delegate_mut().set_rows(rows.clone(), split_sections);
      state.delegate_mut().set_opened_path(opened_path);
      cx.notify();
    });

    let selected_index = if self.force_list_selection {
      self.force_list_selection = false;
      self.selected_file_index(cx)
    } else {
      // Try to preserve the selected file by path rather than raw index
      self.selected_file_index(cx)
    };
    self.set_file_list_selected_index(selected_index, cx);
  }

  fn handle_auth_code(&mut self, code: String, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let service = self.api.keychain_service().to_string();
    let username = self.api.keychain_username().to_string();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.exchange_code_for_token(&code)).await;
      match result {
        Ok(token) => {
          let secret = token.clone().into_bytes();
          let write_task = cx.update(|cx| cx.write_credentials(&service, &username, &secret));
          let _ = write_task.await;
          let _ = this.update(cx, |this, cx| {
            this.api.set_bearer_token(token);
            crate::analytics::track(cx, "sign_in_completed");
            this.refresh_auth_state(cx);
          });
        }
        Err(_) => {
          let _ = this.update(cx, |this, cx| {
            this.set_auth_state(AuthState::Unauthenticated, cx);
          });
        }
      }
    });

    self.auth_task = Some(task);
  }

  fn handle_subscription_callback(&mut self, cx: &mut Context<Self>) {
    crate::analytics::track(cx, "subscription_callback_received");
    self.refresh_auth_state(cx);

    NavigationHistory::navigate("/billing", cx);
  }

  fn logout(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let service = self.api.keychain_service().to_string();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || api.sign_out()).await;
      let delete_task = cx.update(|cx| cx.delete_credentials(&service));
      let _ = delete_task.await;
      let _ = this.update(cx, |this, cx| {
        this.set_auth_state(AuthState::Unauthenticated, cx);
      });
    });

    self.auth_task = Some(task);
  }

  fn load_bearer_from_keychain(&mut self, cx: &mut Context<Self>) {
    let service = self.api.keychain_service().to_string();
    let task = cx.spawn(async move |this, cx| {
      let read_result = cx.update(|cx| cx.read_credentials(&service)).await;
      let _ = this.update(cx, |this, cx| {
        if let Ok(Some((_username, secret))) = read_result
          && let Ok(token) = String::from_utf8(secret)
        {
          this.api.set_bearer_token(token);
          this.refresh_auth_state(cx);
          return;
        }
        this.set_auth_state(AuthState::Unauthenticated, cx);
      });
    });

    self.auth_task = Some(task);
  }

  fn refresh_auth_state(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_me()).await;
      let _ = this.update(cx, |this, cx| {
        let state = match result {
          Ok(Some(user)) => AuthState::Authenticated(Box::new(user)),
          Ok(None) => AuthState::Unauthenticated,
          Err(_) => AuthState::Unauthenticated,
        };
        this.set_auth_state(state, cx);
      });
    });

    self.auth_task = Some(task);
  }

  fn start_github_sign_in(&mut self, source: &'static str, cx: &mut Context<Self>) {
    crate::analytics::track_with(
      cx,
      "sign_in_started",
      Some(serde_json::json!({ "source": source })),
    );
    let api = self.api.clone();
    let task = cx.spawn(async move |_, cx| {
      let result = unblock(move || api.sign_in_with_github()).await;
      if let Ok(Some(url)) = result {
        cx.update(|cx| cx.open_url(&url));
      }
    });

    self.auth_task = Some(task);
  }

  fn set_auth_state(&mut self, state: AuthState, cx: &mut Context<Self>) {
    let had_github_access = AuthStateStore::has_github_access(cx);
    self.auth_state = state.clone();
    AuthStateStore::set(cx, state);

    if !had_github_access && AuthStateStore::has_github_access(cx) {
      self.fetch_initial_notifications(cx);
    }

    self.refresh_branch_pr_lookup(cx);
    cx.refresh_windows();
    cx.notify();
  }

  fn fetch_initial_notifications(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    cx.spawn(async move |_, cx| {
      let result = unblock(move || api.fetch_github_notifications()).await;
      cx.update(|cx| {
        if let Ok(notifications) = result {
          GithubNotificationsStore::set(cx, notifications);
          cx.refresh_windows();
        }
      });
    })
    .detach();
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let recent = ConfigStore::load_recent_repositories();
    let app_settings = AppSettings::get(cx);
    let selected_repo = recent.first().map(|repo| repo.path.clone());
    let repo_dropdown_items: Vec<RecentRepoItem> = recent
      .iter()
      .map(|repo| RecentRepoItem::new(repo, selected_repo.as_deref()))
      .collect();
    let git_page_weak = cx.entity().downgrade();
    let file_list =
      cx.new(|cx| ListState::new(GitFileListDelegate::new(git_page_weak), window, cx));
    let _ = file_list.read(cx).focus_handle(cx).tab_stop(true);
    let history_tree = cx.new(|cx| TreeState::new(cx));

    let commit_input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });

    let terminal_working_directory = selected_repo.clone();

    let mut view = Self {
      focus_handle: cx.focus_handle(),
      history_tree_wrapper_focus: cx.focus_handle().tab_stop(true),
      api: WorkspaceApi::global(cx).api.clone(),
      repo_dropdown_items,
      branch_dropdown_items: Vec::new(),
      file_list,
      history_tree,
      window_handle: window.window_handle(),
      selected_repo,
      status_entries: Vec::new(),
      branch_status: None,
      has_head_commit: false,
      can_undo_last_commit: false,
      can_push: false,
      can_force_push: false,
      has_unpublished_branch_commits: false,
      unpublished_branch_check_key: None,
      unpublished_branch_checked_at: None,
      force_push_after_rebase: false,
      push_pull_in_progress: false,
      publish_branch_and_create_pr_in_progress: false,
      pro_push_hint_shown: false,
      fetch_in_progress: false,
      has_staged_changes: false,
      merge_in_progress: false,
      rebase_in_progress: false,
      sidebar_mode: GitSidebarMode::Changes,
      history_commits: Vec::new(),
      history_revision: None,
      history_loading: false,
      history_expanded_commit_oids: HashSet::new(),
      history_commit_files: HashMap::new(),
      history_commit_files_loading: HashSet::new(),
      pending_history_file_loads: HashSet::new(),
      history_opened_commit_file: None,
      history_rows_cache: Vec::new(),
      history_tree_nodes: HashMap::new(),
      selected_file: None,
      selected_file_source: None,
      selected_file_index_hint: None,
      select_first_file_after_restore: false,
      force_list_selection: false,
      editor: None,
      terminal_view: cx.new(|cx| TerminalView::new(terminal_working_directory.clone(), cx)),
      interactive_rebase_todo_view: None,
      diff_view: if app_settings.split_diff_view {
        DiffViewMode::Split
      } else {
        DiffViewMode::Inline
      },
      hide_whitespace: app_settings.hide_whitespace,
      git_unified_file_view: app_settings.git_unified_file_view,
      show_markdown_preview: false,
      show_terminal_sidebar: false,
      agent_review: AgentReviewComments::new(),
      binary_preview: None,
      svg_preview: cx.new(|_| SvgPreview::new()),
      branch_pr_lookup_context: None,
      branch_pr_lookup_result: None,
      branch_pr_lookup_loading: false,
      pending_open_action: None,
      pending_conflict_reveal_path: None,
      auth_state: AuthState::Unknown,
      auth_task: None,
      branch_pr_lookup_task: None,
      open_file_task: None,
      status_task: None,
      status_refresh_in_progress: false,
      history_task: None,
      history_files_task: None,
      history_open_file_task: None,
      branch_task: None,
      branch_refresh_in_progress: false,
      branch_pr_lookup_generation: 0,
      open_file_generation: 0,
      status_refresh_generation: 0,
      branch_refresh_generation: 0,
      poll_task: None,
      poll_window_active: true,
      commit_input,
      operation_error: None,
    };

    view.subscribe_to_file_list(cx);
    view.subscribe_to_commit_input(window, cx);
    view.subscribe_to_history_tree_focus(window, cx);
    view.subscribe_to_window_activation(window, cx);
    view.subscribe_to_svg_preview(cx);
    view.reload_status(cx);
    view.refresh_branches(cx);
    view.start_polling(cx);
    view.load_bearer_from_keychain(cx);
    AuthCallbackTarget::register_git_page(cx);
    GitPageHandle::register(cx);

    view
  }

  #[cfg(test)]
  fn new_for_test(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let git_page_weak = cx.entity().downgrade();
    let file_list =
      cx.new(|cx| ListState::new(GitFileListDelegate::new(git_page_weak), window, cx));
    let _ = file_list.read(cx).focus_handle(cx).tab_stop(true);
    let history_tree = cx.new(|cx| TreeState::new(cx));
    let commit_input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });

    let mut view = Self {
      focus_handle: cx.focus_handle(),
      history_tree_wrapper_focus: cx.focus_handle().tab_stop(true),
      api: ApiClient::new(),
      repo_dropdown_items: Vec::new(),
      branch_dropdown_items: Vec::new(),
      file_list,
      history_tree,
      window_handle: window.window_handle(),
      selected_repo: None,
      status_entries: Vec::new(),
      branch_status: None,
      has_head_commit: false,
      can_undo_last_commit: false,
      can_push: false,
      can_force_push: false,
      has_unpublished_branch_commits: false,
      unpublished_branch_check_key: None,
      unpublished_branch_checked_at: None,
      force_push_after_rebase: false,
      push_pull_in_progress: false,
      publish_branch_and_create_pr_in_progress: false,
      pro_push_hint_shown: false,
      fetch_in_progress: false,
      has_staged_changes: false,
      merge_in_progress: false,
      rebase_in_progress: false,
      sidebar_mode: GitSidebarMode::Changes,
      history_commits: Vec::new(),
      history_revision: None,
      history_loading: false,
      history_expanded_commit_oids: HashSet::new(),
      history_commit_files: HashMap::new(),
      history_commit_files_loading: HashSet::new(),
      pending_history_file_loads: HashSet::new(),
      history_opened_commit_file: None,
      history_rows_cache: Vec::new(),
      history_tree_nodes: HashMap::new(),
      selected_file: None,
      selected_file_source: None,
      selected_file_index_hint: None,
      select_first_file_after_restore: false,
      force_list_selection: false,
      editor: None,
      terminal_view: cx.new(|cx| TerminalView::new(None, cx)),
      interactive_rebase_todo_view: None,
      diff_view: DiffViewMode::Inline,
      hide_whitespace: false,
      git_unified_file_view: false,
      show_markdown_preview: false,
      show_terminal_sidebar: false,
      agent_review: AgentReviewComments::new(),
      binary_preview: None,
      svg_preview: cx.new(|_| SvgPreview::new()),
      branch_pr_lookup_context: None,
      branch_pr_lookup_result: None,
      branch_pr_lookup_loading: false,
      pending_open_action: None,
      pending_conflict_reveal_path: None,
      auth_state: AuthState::Unknown,
      auth_task: None,
      branch_pr_lookup_task: None,
      open_file_task: None,
      status_task: None,
      status_refresh_in_progress: false,
      history_task: None,
      history_files_task: None,
      history_open_file_task: None,
      branch_task: None,
      branch_refresh_in_progress: false,
      branch_pr_lookup_generation: 0,
      open_file_generation: 0,
      status_refresh_generation: 0,
      branch_refresh_generation: 0,
      poll_task: None,
      poll_window_active: true,
      commit_input,
      operation_error: None,
    };

    view.subscribe_to_file_list(cx);
    view.subscribe_to_commit_input(window, cx);
    view.subscribe_to_history_tree_focus(window, cx);
    view.subscribe_to_window_activation(window, cx);
    view.subscribe_to_svg_preview(cx);
    GitPageHandle::register(cx);
    view
  }

  fn handle_repo_select_confirm(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    self.set_selected_repo(repo_root, cx);
    self.ensure_page_shortcut_focus(cx);
  }

  fn open_repository_with_action(
    &mut self,
    repo_root: PathBuf,
    action: GitPageOpenAction,
    cx: &mut Context<Self>,
  ) {
    let conflict_resolution = Self::active_conflict_resolution_snapshot(&repo_root);
    self.set_selected_repo(repo_root.clone(), cx);
    self.pending_open_action = Some(action.clone());
    if let Some(conflict_resolution) = conflict_resolution {
      self.merge_in_progress = conflict_resolution.merge_in_progress;
      self.rebase_in_progress = conflict_resolution.rebase_in_progress;
    }

    match action {
      GitPageOpenAction::MergeBaseBranch { base_branch_name } => {
        self.start_merge_base_branch_action(repo_root, base_branch_name, cx);
      }
    }
  }

  fn active_conflict_resolution_snapshot(
    repo_root: &Path,
  ) -> Option<ActiveConflictResolutionSnapshot> {
    let merge_in_progress = is_merge_in_progress(repo_root).unwrap_or(false);
    let rebase_in_progress = is_rebase_in_progress(repo_root).unwrap_or(false);
    let conflicted_path = Self::first_conflicted_path(repo_root);

    (merge_in_progress || rebase_in_progress || conflicted_path.is_some()).then_some(
      ActiveConflictResolutionSnapshot {
        merge_in_progress,
        rebase_in_progress,
        conflicted_path,
      },
    )
  }

  fn open_action_loading_message(action: &GitPageOpenAction) -> &'static str {
    match action {
      GitPageOpenAction::MergeBaseBranch { .. } => "Opening conflict resolution...",
    }
  }

  fn refocus_page_shortcuts_after_dropdown_select(
    &self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let focus_handle = self.focus_handle.clone();
    window.focus(&focus_handle, cx);
    cx.on_next_frame(window, move |_, window, cx| {
      window.focus(&focus_handle, cx);
    });
  }

  fn repo_select_handler(&self, cx: &Context<Self>) -> RepoSelectHandler {
    let view = cx.entity();
    Rc::new(move |repo_root, window, cx| {
      view.update(cx, |this, cx| {
        if repo_root.as_os_str().is_empty() {
          this.start_open_repository(window, cx);
          return;
        }
        this.handle_repo_select_confirm(repo_root, cx);
        this.refocus_page_shortcuts_after_dropdown_select(window, cx);
      });
    })
  }

  fn handle_branch_select_confirm(&mut self, branch: BranchRef, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };

    self.ensure_page_shortcut_focus(cx);

    if Self::is_detached_branch_select_value(&branch) {
      return;
    }
    self.advance_status_refresh_generation();
    let editor = self.editor.clone();
    let branch_name = branch.name.clone();

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || switch_branch(&repo_root, &branch)).await;
      let _ = this.update(cx, |this, cx| match result {
        Ok(()) => {
          this.reload_status(cx);
          this.refresh_branches(cx);
          if let Some(editor) = editor.clone() {
            editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
          }
        }
        Err(error) => {
          this.push_branch_switch_error_notification(&branch_name, error.to_string().into(), cx);
        }
      });
    });

    self.branch_task = Some(task);
  }

  fn branch_select_handler(&self, cx: &Context<Self>) -> BranchSelectHandler {
    let view = cx.entity();
    Rc::new(move |branch, window, cx| {
      view.update(cx, |this, cx| {
        this.handle_branch_select_confirm(branch, cx);
        this.refocus_page_shortcuts_after_dropdown_select(window, cx);
      });
    })
  }

  fn subscribe_to_file_list(&mut self, cx: &mut Context<Self>) {
    cx.subscribe(
      &self.file_list,
      move |this, state, event: &ListEvent, cx| match event {
        ListEvent::Select(ix) | ListEvent::Confirm(ix) => {
          let row = state.read(cx).delegate().row_at(*ix);
          if let Some(row) = row {
            this.selected_file_index_hint = Some(*ix);
            this.open_status_file(row.entry.path.clone(), cx);
          }
        }
        ListEvent::Cancel => {}
      },
    )
    .detach();
  }

  fn subscribe_to_history_tree_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    cx.on_focus_in(
      &self.history_tree_wrapper_focus.clone(),
      window,
      |this, window, cx| {
        this.history_tree.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      },
    )
    .detach();
  }

  /// The preview renders on a background task; repaint the page when it lands.
  fn subscribe_to_svg_preview(&mut self, cx: &mut Context<Self>) {
    cx.observe(&self.svg_preview, |_, _, cx| cx.notify())
      .detach();
  }

  fn subscribe_to_window_activation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    cx.on_focus_in(&self.focus_handle.clone(), window, |this, _window, cx| {
      this.poll_window_active = true;
      if this.selected_repo.is_some() && !this.status_refresh_in_progress {
        this.reload_status(cx);
      }
    })
    .detach();
  }

  fn invalidate_open_file_task(&mut self) {
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    self.open_file_task = None;
  }

  fn set_selected_repo(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    if self.selected_repo.as_ref() == Some(&repo_root) {
      return;
    }

    let previous_repo = self.selected_repo.clone();
    self.selected_repo = Some(repo_root.clone());
    self.invalidate_open_file_task();
    self.selected_file = None;
    self.selected_file_source = None;
    self.select_first_file_after_restore = false;
    self.operation_error = None;
    self.editor = None;
    self.agent_review.clear();
    self.binary_preview = None;
    self.interactive_rebase_todo_view = None;
    self.merge_in_progress = false;
    self.rebase_in_progress = false;
    self.force_push_after_rebase = false;
    self.push_pull_in_progress = false;
    self.publish_branch_and_create_pr_in_progress = false;
    self.history_commits.clear();
    self.history_revision = None;
    self.history_loading = self.sidebar_mode == GitSidebarMode::History;
    self.history_expanded_commit_oids.clear();
    self.history_commit_files.clear();
    self.history_commit_files_loading.clear();
    self.pending_history_file_loads.clear();
    self.history_opened_commit_file = None;
    ActiveLocalRepoStore::set(cx, None);
    self.clear_branch_pr_lookup();
    ConfigStore::persist_recent_repository(&repo_root);

    self.reload_status(cx);
    self.refresh_branches(cx);
    self.refresh_repo_select(cx);
    self.sync_repo_select_with_path(&repo_root, cx);
    self.sync_sentry_git_context();
    let mut data = Map::new();
    if let Some(previous_repo) = previous_repo.as_deref() {
      let (repo_name, repo_hash) = sentry_context::sanitize_repo_path(previous_repo);
      data.insert("from_repo_name".into(), repo_name.into());
      data.insert("from_repo_hash".into(), repo_hash.into());
    }
    let (repo_name, repo_hash) = sentry_context::sanitize_repo_path(&repo_root);
    data.insert("to_repo_name".into(), repo_name.into());
    data.insert("to_repo_hash".into(), repo_hash.into());
    self.add_git_breadcrumb("Selected repository changed", data);
    cx.notify();
  }

  fn clear_selected_repo(&mut self, cx: &mut Context<Self>) {
    if self.selected_repo.is_none() {
      return;
    }
    self.selected_repo = None;
    self.invalidate_open_file_task();
    self.selected_file = None;
    self.selected_file_source = None;
    self.select_first_file_after_restore = false;
    self.operation_error = None;
    self.editor = None;
    self.agent_review.clear();
    self.binary_preview = None;
    self.interactive_rebase_todo_view = None;
    self.merge_in_progress = false;
    self.rebase_in_progress = false;
    self.force_push_after_rebase = false;
    self.push_pull_in_progress = false;
    self.publish_branch_and_create_pr_in_progress = false;
    self.status_entries.clear();
    self.branch_status = None;
    self.history_commits.clear();
    self.history_revision = None;
    self.history_loading = false;
    self.history_expanded_commit_oids.clear();
    self.history_commit_files.clear();
    self.history_commit_files_loading.clear();
    self.pending_history_file_loads.clear();
    self.history_opened_commit_file = None;
    ActiveLocalRepoStore::set(cx, None);
    self.clear_branch_pr_lookup();
    self.refresh_repo_select(cx);
    cx.notify();
  }

  fn refresh_repo_select(&mut self, _cx: &mut Context<Self>) {
    let selected_repo = self.selected_repo.clone();
    let mut recent = ConfigStore::load_recent_repositories();
    if let Some(selected_repo_path) = selected_repo.as_ref()
      && !recent.iter().any(|repo| &repo.path == selected_repo_path)
    {
      recent.insert(
        0,
        RecentRepository {
          path: selected_repo_path.clone(),
        },
      );
    }

    let items: Vec<RecentRepoItem> = recent
      .iter()
      .map(|repo| RecentRepoItem::new(repo, selected_repo.as_deref()))
      .collect();
    self.repo_dropdown_items = items;
  }

  fn sync_repo_select_with_path(&mut self, repo_root: &Path, _cx: &mut Context<Self>) {
    let repo_root = repo_root.to_path_buf();
    let mut recent = ConfigStore::load_recent_repositories();
    if !recent.iter().any(|repo| repo.path == repo_root) {
      recent.insert(
        0,
        RecentRepository {
          path: repo_root.clone(),
        },
      );
    }
    let items = recent
      .iter()
      .map(|repo| RecentRepoItem::new(repo, Some(repo_root.as_path())))
      .collect::<Vec<_>>();
    self.repo_dropdown_items = items;
  }

  fn clear_branch_select(&mut self, cx: &mut Context<Self>) {
    self.branch_dropdown_items.clear();
    cx.notify();
  }

  fn refresh_branches(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.branch_refresh_in_progress = false;
      self.clear_branch_select(cx);
      return;
    };

    self.branch_refresh_in_progress = true;
    self.branch_refresh_generation = self.branch_refresh_generation.wrapping_add(1);
    let refresh_generation = self.branch_refresh_generation;
    let requested_repo = repo_root.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        let branches = list_branches(&repo_root).ok()?;
        let current = current_branch_status(&repo_root).ok();
        let detached_label = if Self::is_detached_head(current.as_ref()) {
          detached_head_label(&repo_root).ok()
        } else {
          None
        };
        Some((branches, current, detached_label))
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        if !Self::should_apply_branch_refresh(
          this.selected_repo.as_deref(),
          requested_repo.as_path(),
          this.branch_refresh_generation,
          refresh_generation,
        ) {
          return;
        }
        this.branch_refresh_in_progress = false;
        this.branch_task = None;
        if let Some((branches, current, detached_label)) = result {
          let selected = Self::selected_branch_from_status(current.as_ref());
          let items =
            Self::branch_select_items(branches, selected.as_ref(), detached_label.as_deref());
          this.branch_dropdown_items = items;
        }
        cx.notify();
      });
    });

    self.branch_task = Some(task);
  }

  fn refresh_current_page(&mut self, cx: &mut Context<Self>) {
    self.reload_status(cx);
    self.refresh_branches(cx);
  }

  fn reload_status(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.invalidate_open_file_task();
      self.status_entries.clear();
      self.select_first_file_after_restore = false;
      if Self::should_refresh_file_list(self.sidebar_mode) {
        self.refresh_file_list(cx);
      }
      self.branch_status = None;
      self.has_head_commit = false;
      self.can_undo_last_commit = false;
      self.can_push = false;
      self.can_force_push = false;
      self.has_unpublished_branch_commits = false;
      self.unpublished_branch_check_key = None;
      self.unpublished_branch_checked_at = None;
      self.force_push_after_rebase = false;
      self.push_pull_in_progress = false;
      self.publish_branch_and_create_pr_in_progress = false;
      self.has_staged_changes = false;
      self.merge_in_progress = false;
      self.rebase_in_progress = false;
      self.operation_error = None;
      self.history_commits.clear();
      self.history_revision = None;
      self.history_loading = false;
      self.history_expanded_commit_oids.clear();
      self.history_commit_files.clear();
      self.history_commit_files_loading.clear();
      self.pending_history_file_loads.clear();
      self.history_opened_commit_file = None;
      self.refresh_history_list(cx);
      self.sync_sentry_git_context();
      ActiveLocalRepoStore::set(cx, None);
      self.clear_branch_pr_lookup();
      self.status_refresh_in_progress = false;
      cx.notify();
      return;
    };
    self.status_refresh_in_progress = true;
    let include_history = self.sidebar_mode == GitSidebarMode::History;
    if include_history && self.history_commits.is_empty() {
      self.history_loading = true;
      cx.notify();
    }
    let refresh_generation = self.advance_status_refresh_generation();

    let task = cx.spawn(async move |this, cx| {
      let requested_repo = repo_root.clone();
      let status = unblock(move || {
        let entries = list_repo_status(&repo_root).ok()?;
        let branch = current_branch_status(&repo_root).ok();
        let head_status = head_commit_status(&repo_root).ok();
        let unpublished_branch_commits = branch_has_unpublished_commits(&repo_root).ok()?;
        let merge_in_progress = is_merge_in_progress(&repo_root).unwrap_or(false);
        let rebase_in_progress = is_rebase_in_progress(&repo_root).unwrap_or(false);
        let rebase_commit_message = if rebase_in_progress {
          current_rebase_commit_message(&repo_root).ok().flatten()
        } else {
          None
        };
        let history = if include_history {
          list_commit_history(&repo_root, HISTORY_MAX_COMMITS).ok()
        } else {
          None
        };
        let history_revision = if include_history {
          current_history_revision(&repo_root).ok()
        } else {
          None
        };
        Some((
          entries,
          branch,
          head_status,
          unpublished_branch_commits,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          history,
          history_revision,
        ))
      })
      .await;
      let Some((
        entries,
        branch_status,
        head_status,
        unpublished_branch_commits,
        merge_in_progress,
        rebase_in_progress,
        rebase_commit_message,
        history,
        history_revision,
      )) = status
      else {
        let _ = this.update(cx, |this, cx| {
          if !Self::should_apply_status_refresh(
            this.selected_repo.as_deref(),
            requested_repo.as_path(),
            this.status_refresh_generation,
            refresh_generation,
          ) {
            return;
          }
          this.status_refresh_in_progress = false;
          this.status_task = None;
          this.invalidate_open_file_task();
          this.status_entries.clear();
          this.select_first_file_after_restore = false;
          this.branch_status = None;
          this.has_head_commit = false;
          this.can_undo_last_commit = false;
          this.can_push = false;
          this.can_force_push = false;
          this.has_unpublished_branch_commits = false;
          this.unpublished_branch_check_key = None;
          this.unpublished_branch_checked_at = None;
          this.force_push_after_rebase = false;
          this.push_pull_in_progress = false;
          this.publish_branch_and_create_pr_in_progress = false;
          this.has_staged_changes = false;
          this.merge_in_progress = false;
          this.rebase_in_progress = false;
          this.operation_error = None;
          this.selected_file = None;
          this.selected_file_source = None;
          this.editor = None;
          this.binary_preview = None;
          this.interactive_rebase_todo_view = None;
          this.history_opened_commit_file = None;
          this.clear_branch_select(cx);
          if include_history {
            this.history_commits.clear();
            this.history_revision = None;
            this.history_loading = false;
            this.history_expanded_commit_oids.clear();
            this.history_commit_files.clear();
            this.history_commit_files_loading.clear();
            this.pending_history_file_loads.clear();
            this.refresh_history_list(cx);
          } else if this.history_loading {
            this.history_loading = false;
          }
          if Self::should_refresh_file_list(this.sidebar_mode) {
            this.refresh_file_list(cx);
          }
          ActiveLocalRepoStore::set(cx, None);
          this.clear_branch_pr_lookup();
          cx.notify();
        });
        return;
      };

      let _ = this.update(cx, |this, cx| {
        if !Self::should_apply_status_refresh(
          this.selected_repo.as_deref(),
          requested_repo.as_path(),
          this.status_refresh_generation,
          refresh_generation,
        ) {
          return;
        }
        this.status_refresh_in_progress = false;
        this.status_task = None;
        let branch_changed = this.apply_status_snapshot(
          entries,
          branch_status,
          head_status,
          unpublished_branch_commits,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          true,
          cx,
        );
        if include_history {
          if let Some(history) = history {
            this.history_commits = history;
            this.sync_history_cache_with_commits();
            if let Some(history_revision) = history_revision {
              this.history_revision = Some(history_revision);
            }
            this.refresh_history_list(cx);
          }
          this.history_loading = false;
        }
        if branch_changed {
          this.refresh_branches(cx);
        }
        if Self::should_refresh_file_list(this.sidebar_mode) {
          this.refresh_file_list(cx);
        }
        if this.refresh_agent_review_comment_states_for_selected_file(cx) {
          this.sync_agent_review_comments_to_editor(cx);
        }
        cx.notify();
      });
    });

    self.status_task = Some(task);
  }

  fn start_polling(&mut self, cx: &mut Context<Self>) {
    if self.poll_task.is_some() {
      return;
    }

    self.poll_task = Some(cx.spawn(async move |this, cx| {
      loop {
        let poll_window_active = match this.update(cx, |this, _| this.poll_window_active) {
          Ok(window_active) => window_active,
          Err(_) => return,
        };
        cx.background_executor()
          .timer(Self::status_poll_interval(poll_window_active))
          .await;

        let window_handle = match this.update(cx, |this, _| this.window_handle) {
          Ok(window_handle) => window_handle,
          Err(_) => return,
        };
        let window_active = match window_handle.update(cx, |_, window, _| window.is_window_active())
        {
          Ok(window_active) => window_active,
          Err(_) => return,
        };

        let poll_state = match this.update(cx, |this, _| {
          this.poll_window_active = window_active;
          if !Self::should_poll_status(
            window_active,
            this.selected_repo.as_deref(),
            this.status_refresh_in_progress,
          ) {
            return None;
          }
          let repo_root = this
            .selected_repo
            .clone()
            .expect("selected repo is checked before polling");
          let now = Instant::now();
          let force_unpublished_branch_recheck =
            this.unpublished_branch_checked_at.is_none_or(|checked_at| {
              now.duration_since(checked_at) >= UNPUBLISHED_BRANCH_RECHECK_INTERVAL
            });
          Some((
            repo_root,
            this.sidebar_mode == GitSidebarMode::History,
            this.history_revision.clone(),
            this.history_commits.is_empty(),
            // Polling should not supersede an explicit refresh that is already in flight.
            this.current_status_refresh_generation(),
            this.unpublished_branch_check_key.clone(),
            this.has_unpublished_branch_commits,
            force_unpublished_branch_recheck,
            now,
          ))
        }) {
          Ok(Some(value)) => value,
          Ok(None) => continue,
          Err(_) => return,
        };
        let (
          repo_root,
          include_history,
          cached_history_revision,
          history_empty,
          refresh_generation,
          cached_unpublished_branch_key,
          cached_unpublished_branch_commits,
          force_unpublished_branch_recheck,
          unpublished_branch_checked_at,
        ) = poll_state;
        let requested_repo = repo_root.clone();

        let status = unblock(move || {
          let entries = list_repo_status(&repo_root).ok()?;
          let branch = current_branch_status(&repo_root).ok();
          let head_status = head_commit_status(&repo_root).ok();
          let (
            unpublished_branch_commits,
            unpublished_branch_check_key,
            unpublished_branch_checked,
          ) = Self::resolve_polled_unpublished_branch_commits(
            &repo_root,
            branch.as_ref(),
            cached_unpublished_branch_key.as_ref(),
            cached_unpublished_branch_commits,
            force_unpublished_branch_recheck,
          )?;
          let merge_in_progress = is_merge_in_progress(&repo_root).unwrap_or(false);
          let rebase_in_progress = is_rebase_in_progress(&repo_root).unwrap_or(false);
          let rebase_commit_message = if rebase_in_progress {
            current_rebase_commit_message(&repo_root).ok().flatten()
          } else {
            None
          };
          let polled_history_revision = if include_history {
            current_history_revision(&repo_root).ok()
          } else {
            None
          };
          let should_refresh_history = Self::should_refresh_history_for_poll(
            include_history,
            history_empty,
            cached_history_revision.as_ref(),
            polled_history_revision.as_ref(),
          );
          let history = if should_refresh_history {
            list_commit_history(&repo_root, HISTORY_MAX_COMMITS).ok()
          } else {
            None
          };
          Some((
            entries,
            branch,
            head_status,
            unpublished_branch_commits,
            merge_in_progress,
            rebase_in_progress,
            rebase_commit_message,
            polled_history_revision,
            should_refresh_history,
            history,
            unpublished_branch_check_key,
            unpublished_branch_checked,
          ))
        })
        .await;
        let Some((
          entries,
          branch_status,
          head_status,
          unpublished_branch_commits,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          polled_history_revision,
          should_refresh_history,
          history,
          unpublished_branch_check_key,
          unpublished_branch_checked,
        )) = status
        else {
          continue;
        };

        let _ = this.update(cx, |this, cx| {
          if !Self::should_apply_status_refresh(
            this.selected_repo.as_deref(),
            requested_repo.as_path(),
            this.status_refresh_generation,
            refresh_generation,
          ) {
            return;
          }
          let branch_changed = this.apply_status_snapshot(
            entries,
            branch_status,
            head_status,
            unpublished_branch_commits,
            merge_in_progress,
            rebase_in_progress,
            rebase_commit_message,
            false,
            cx,
          );
          if unpublished_branch_checked {
            this.unpublished_branch_checked_at = unpublished_branch_check_key
              .as_ref()
              .map(|_| unpublished_branch_checked_at);
          }
          this.unpublished_branch_check_key = unpublished_branch_check_key;
          if include_history {
            if let Some(history) = history {
              this.history_commits = history;
              this.sync_history_cache_with_commits();
              this.history_loading = false;
              if let Some(history_revision) = polled_history_revision {
                this.history_revision = Some(history_revision);
              }
              this.refresh_history_list(cx);
            } else if !should_refresh_history {
              if let Some(history_revision) = polled_history_revision {
                this.history_revision = Some(history_revision);
              }
            } else if this.history_loading {
              // Preserve last known history on transient failures.
              this.history_loading = false;
            }
          }
          if branch_changed {
            this.refresh_branches(cx);
          }
          if Self::should_refresh_file_list(this.sidebar_mode) {
            this.refresh_file_list(cx);
          }
          cx.notify();
        });
      }
    }));
  }

  #[cfg(test)]
  fn poll_once_for_test(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };

    let include_history = self.sidebar_mode == GitSidebarMode::History;
    let cached_history_revision = self.history_revision.clone();
    let history_empty = self.history_commits.is_empty();
    let requested_repo = repo_root.clone();
    let refresh_generation = self.current_status_refresh_generation();
    let cached_unpublished_branch_key = self.unpublished_branch_check_key.clone();
    let cached_unpublished_branch_commits = self.has_unpublished_branch_commits;
    let unpublished_branch_checked_at = Instant::now();
    let force_unpublished_branch_recheck =
      self.unpublished_branch_checked_at.is_none_or(|checked_at| {
        unpublished_branch_checked_at.duration_since(checked_at)
          >= UNPUBLISHED_BRANCH_RECHECK_INTERVAL
      });
    let task = cx.spawn(async move |this, cx| {
      let status = unblock(move || {
        let entries = list_repo_status(&repo_root).ok()?;
        let branch = current_branch_status(&repo_root).ok();
        let head_status = head_commit_status(&repo_root).ok();
        let (unpublished_branch_commits, unpublished_branch_check_key, unpublished_branch_checked) =
          Self::resolve_polled_unpublished_branch_commits(
            &repo_root,
            branch.as_ref(),
            cached_unpublished_branch_key.as_ref(),
            cached_unpublished_branch_commits,
            force_unpublished_branch_recheck,
          )?;
        let merge_in_progress = is_merge_in_progress(&repo_root).unwrap_or(false);
        let rebase_in_progress = is_rebase_in_progress(&repo_root).unwrap_or(false);
        let rebase_commit_message = if rebase_in_progress {
          current_rebase_commit_message(&repo_root).ok().flatten()
        } else {
          None
        };
        let polled_history_revision = if include_history {
          current_history_revision(&repo_root).ok()
        } else {
          None
        };
        let should_refresh_history = Self::should_refresh_history_for_poll(
          include_history,
          history_empty,
          cached_history_revision.as_ref(),
          polled_history_revision.as_ref(),
        );
        let history = if should_refresh_history {
          list_commit_history(&repo_root, HISTORY_MAX_COMMITS).ok()
        } else {
          None
        };
        Some((
          entries,
          branch,
          head_status,
          unpublished_branch_commits,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          polled_history_revision,
          should_refresh_history,
          history,
          unpublished_branch_check_key,
          unpublished_branch_checked,
        ))
      })
      .await;

      let Some((
        entries,
        branch_status,
        head_status,
        unpublished_branch_commits,
        merge_in_progress,
        rebase_in_progress,
        rebase_commit_message,
        polled_history_revision,
        should_refresh_history,
        history,
        unpublished_branch_check_key,
        unpublished_branch_checked,
      )) = status
      else {
        return;
      };

      let _ = this.update(cx, |this, cx| {
        if !Self::should_apply_status_refresh(
          this.selected_repo.as_deref(),
          requested_repo.as_path(),
          this.status_refresh_generation,
          refresh_generation,
        ) {
          return;
        }
        let branch_changed = this.apply_status_snapshot(
          entries,
          branch_status,
          head_status,
          unpublished_branch_commits,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          false,
          cx,
        );
        if unpublished_branch_checked {
          this.unpublished_branch_checked_at = unpublished_branch_check_key
            .as_ref()
            .map(|_| unpublished_branch_checked_at);
        }
        this.unpublished_branch_check_key = unpublished_branch_check_key;
        if include_history {
          if let Some(history) = history {
            this.history_commits = history;
            this.sync_history_cache_with_commits();
            this.history_loading = false;
            if let Some(history_revision) = polled_history_revision {
              this.history_revision = Some(history_revision);
            }
            this.refresh_history_list(cx);
          } else if !should_refresh_history {
            if let Some(history_revision) = polled_history_revision {
              this.history_revision = Some(history_revision);
            }
          } else if this.history_loading {
            this.history_loading = false;
          }
        }
        if branch_changed {
          this.refresh_branches(cx);
        }
        if Self::should_refresh_file_list(this.sidebar_mode) {
          this.refresh_file_list(cx);
        }
        cx.notify();
      });
    });

    self.status_task = Some(task);
  }

  fn show_branch_switcher_action(
    &mut self,
    _: &crate::ShowBranchSwitcher,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_repo.is_none() {
      return;
    }

    self.open_command_palette(window, cx, Some(CommandPaletteInitialScreen::SwitchBranch));
    cx.stop_propagation();
  }

  fn find_action(&mut self, action: &Find, window: &mut Window, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };

    editor.update(cx, |editor, cx| {
      editor::find(editor, action, window, cx);
    });
  }

  fn close_find_action(&mut self, action: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };

    editor.update(cx, |editor, cx| {
      editor::close_find(editor, action, window, cx);
    });
  }

  fn open_repository_action(
    &mut self,
    _: &OpenRepository,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.start_open_repository(window, cx);
  }

  fn start_open_repository(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
      files: false,
      directories: true,
      multiple: false,
      prompt: Some("Select a repository".into()),
    });

    cx.spawn(async move |this, cx| {
      let Ok(result) = receiver.await else {
        return;
      };

      match result {
        Ok(Some(paths)) => {
          if let Some(path) = paths.into_iter().next() {
            ConfigStore::persist_recent_repository(&path);
            let _ = this.update(cx, |view, cx| {
              view.set_selected_repo(path, cx);
            });
          }
        }
        Ok(None) => {}
        Err(_) => {}
      }
    })
    .detach();
  }

  fn open_git_history_sidebar_action(
    &mut self,
    _: &crate::OpenGitHistorySidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.set_sidebar_mode(GitSidebarMode::History, window, cx);
    cx.stop_propagation();
  }

  fn open_git_changes_sidebar_action(
    &mut self,
    _: &crate::OpenGitChangesSidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.set_sidebar_mode(GitSidebarMode::Changes, window, cx);
    cx.stop_propagation();
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

  fn toggle_hide_whitespace_action(
    &mut self,
    _: &crate::ToggleHideWhitespace,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_hide_whitespace(cx);
    cx.stop_propagation();
  }

  fn previous_annotation_action(
    &mut self,
    _: &crate::PreviousAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.navigate_annotation_in_editor(AnnotationDirection::Previous, cx);
    cx.stop_propagation();
  }

  fn next_annotation_action(
    &mut self,
    _: &crate::NextAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.navigate_annotation_in_editor(AnnotationDirection::Next, cx);
    cx.stop_propagation();
  }

  fn accept_both_conflict_action(
    &mut self,
    _: &crate::AcceptBothConflict,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    self.resolve_active_conflict_in_editor(&editor, ConflictResolution::Both, cx);
    cx.stop_propagation();
  }

  fn sync_editor_unmerged_state(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    if self.history_opened_commit_file.is_some() {
      editor.update(cx, |editor, cx| editor.set_is_unmerged(false, cx));
      return;
    }
    let is_unmerged = self
      .selected_file_entry()
      .is_some_and(|entry| entry.status == RepoStatusKind::Conflicted);
    editor.update(cx, |editor, cx| editor.set_is_unmerged(is_unmerged, cx));
  }

  fn resolve_active_conflict_in_editor(
    &self,
    editor: &Entity<Editor>,
    resolution: ConflictResolution,
    cx: &mut Context<Self>,
  ) -> bool {
    let file_status = if self.history_opened_commit_file.is_some() {
      None
    } else {
      self.selected_file_entry().map(|entry| entry.status)
    };
    if !matches!(file_status, Some(RepoStatusKind::Conflicted)) {
      return false;
    }
    editor.update(cx, |editor, cx| {
      let Some(state) = editor.conflict_navigation_state(cx) else {
        return false;
      };
      editor.resolve_conflict_region(state.active_start_line, resolution, cx);
      editor.save(cx);
      if let Some(next_state) = editor.conflict_navigation_state(cx) {
        editor.reveal_conflict_start_line(next_state.active_start_line, cx);
      }
      true
    })
  }

  fn resolve_all_conflicts_in_editor(
    &mut self,
    resolution: ConflictResolution,
    cx: &mut Context<Self>,
  ) {
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.resolve_all_conflicts(resolution, cx);
      });
    }
  }

  fn reveal_first_conflict_in_editor(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };

    editor.update(cx, |editor, cx| editor.reveal_first_conflict(cx));
    self.pending_conflict_reveal_path = None;
  }

  fn open_file_revealing_first_conflict(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    self.open_file_internal(rel_path, true, SelectedFileSource::StatusEntry, None, cx);
  }

  fn open_file(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    let selected_file_source = self.selected_file_source_for_open_path(&rel_path);
    self.open_file_internal(rel_path, false, selected_file_source, None, cx);
  }

  fn open_status_file(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    self.open_file_internal(rel_path, false, SelectedFileSource::StatusEntry, None, cx);
  }

  fn selected_file_source_for_open_path(&self, rel_path: &Path) -> SelectedFileSource {
    if self
      .status_entries
      .iter()
      .any(|entry| entry.path.as_path() == rel_path)
    {
      SelectedFileSource::StatusEntry
    } else {
      SelectedFileSource::ProjectFile
    }
  }

  fn open_file_internal(
    &mut self,
    rel_path: PathBuf,
    reveal_first_conflict: bool,
    selected_file_source: SelectedFileSource,
    reveal_line: Option<u32>,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    // Sync diff view from persisted setting
    let app_settings = crate::config::AppSettings::get(cx);
    let saved_mode = if app_settings.split_diff_view {
      DiffViewMode::Split
    } else {
      DiffViewMode::Inline
    };
    if self.diff_view != saved_mode {
      self.diff_view = saved_mode;
    }
    self.hide_whitespace = app_settings.hide_whitespace;
    // Agent line numbers are 1-based; the editor reveals by 0-based doc line.
    let reveal_doc_line = reveal_line.map(|line| line.saturating_sub(1) as usize);
    self.pending_conflict_reveal_path = reveal_first_conflict.then_some(rel_path.clone());
    if self.selected_file.as_ref() == Some(&rel_path) && self.history_opened_commit_file.is_none() {
      self.selected_file_source = Some(selected_file_source);
      if reveal_first_conflict {
        self.reveal_first_conflict_in_editor(cx);
      }
      if let Some(doc_line) = reveal_doc_line
        && let Some(editor) = self.editor.clone()
      {
        editor.update(cx, |editor, cx| editor.reveal_source_line(doc_line, cx));
      }
      return;
    }
    let is_markdown = is_markdown_path(&rel_path);
    if !is_markdown && !is_svg_path(&rel_path) {
      self.show_markdown_preview = false;
    }

    self.invalidate_open_file_task();
    let generation = self.open_file_generation;
    let had_history_file_selection = self.history_opened_commit_file.is_some();
    self.history_opened_commit_file = None;
    self.selected_file = Some(rel_path.clone());
    self.selected_file_source = Some(selected_file_source);
    self.sync_sentry_git_context();
    let mut data = Map::new();
    data.insert(
      "file".into(),
      rel_path.to_string_lossy().replace(['\n', '\r'], "").into(),
    );
    self.add_git_breadcrumb("Opened file in git page", data);
    self.editor = None;
    self.binary_preview = None;
    self.svg_preview.update(cx, |preview, _| preview.clear());
    self.force_list_selection = true;
    let opened_path = self.selected_file.clone();
    self.file_list.update(cx, |state, cx| {
      state.delegate_mut().set_opened_path(opened_path);
      cx.notify();
    });
    let selected_index = self.selected_file_index(cx);
    self.set_file_list_selected_index(selected_index, cx);
    if had_history_file_selection {
      self.refresh_history_list(cx);
    }

    let diff_view = self.effective_diff_view_for_path(&rel_path);
    let requested_repo = repo_root.clone();
    let requested_path = rel_path.clone();
    let file_path = requested_repo.join(&requested_path);
    let load_repo_root = requested_repo.clone();
    let load_file_path = file_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let loaded =
        unblock(move || Editor::load_file_for_editor(&load_repo_root, &load_file_path)).await;
      let _ = this.update(cx, move |this, cx| {
        if this.open_file_generation != generation {
          return;
        }
        if this.selected_repo.as_ref() != Some(&requested_repo) {
          return;
        }
        if this.selected_file.as_ref() != Some(&requested_path) {
          return;
        }
        if this.history_opened_commit_file.is_some() {
          return;
        }

        let editor_repo_root = requested_repo.clone();
        let editor_file_path = file_path.clone();
        let binary_preview =
          build_binary_preview(requested_path.as_path(), loaded.binary_bytes.clone());
        let should_reveal_first_conflict =
          this.pending_conflict_reveal_path.as_deref() == Some(requested_path.as_path());
        let editor = cx.new(move |cx| {
          Editor::new_with_loaded_file(editor_repo_root, editor_file_path, loaded, cx)
        });
        let hide_ws = this.hide_whitespace;
        let is_unmerged = this
          .selected_file_entry()
          .is_some_and(|entry| entry.status == RepoStatusKind::Conflicted);
        editor.update(cx, |editor, cx| {
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_ws, cx);
          editor.set_is_unmerged(is_unmerged, cx);
          if should_reveal_first_conflict {
            editor.reveal_first_conflict(cx);
          }
          if let Some(doc_line) = reveal_doc_line {
            editor.reveal_source_line(doc_line, cx);
          }
        });
        this.binary_preview = binary_preview;
        this.editor = Some(editor.clone());
        this.install_agent_review_handlers_for_editor(&editor, cx);
        this.sync_agent_review_comments_to_editor(cx);
        if should_reveal_first_conflict {
          this.pending_conflict_reveal_path = None;
        }
        cx.notify();
      });
    });
    self.open_file_task = Some(task);
    cx.notify();
  }

  fn clear_markdown_preview_if_not_previewable(&mut self, rel_path: &Path) {
    if !is_previewable_path(rel_path) {
      self.show_markdown_preview = false;
    }
  }

  fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
    if let Some(selected) = self.selected_file.as_ref()
      && self.split_disabled_for_path(selected)
    {
      return;
    }
    if self.show_markdown_preview {
      self.show_markdown_preview = false;
    }
    self.diff_view = match self.diff_view {
      DiffViewMode::Inline => DiffViewMode::Split,
      DiffViewMode::Split => DiffViewMode::Inline,
    };
    crate::config::AppSettings::update(cx, |s| {
      s.split_diff_view = self.diff_view == DiffViewMode::Split
    });
    self.sync_diff_view(cx);
    self.sync_sentry_git_context();
    let mut data = Map::new();
    data.insert("diff_view".into(), self.active_diff_view_tag().into());
    self.add_git_breadcrumb("Toggled git diff view", data);
    cx.notify();
  }

  fn toggle_hide_whitespace(&mut self, cx: &mut Context<Self>) {
    self.hide_whitespace = !self.hide_whitespace;
    if let Some(editor) = self.editor.as_ref() {
      let value = self.hide_whitespace;
      editor.update(cx, |editor, cx| {
        editor.set_ignore_whitespace(value, cx);
      });
    }
    cx.notify();
  }

  fn toggle_markdown_preview(&mut self, cx: &mut Context<Self>) {
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      self.show_markdown_preview = false;
      self.sync_diff_view(cx);
      self.sync_sentry_git_context();
      cx.notify();
      return;
    }

    self.show_markdown_preview = !self.show_markdown_preview;
    self.sync_diff_view(cx);
    self.sync_sentry_git_context();
    let mut data = Map::new();
    data.insert("enabled".into(), self.show_markdown_preview.into());
    self.add_git_breadcrumb("Toggled markdown preview", data);
    cx.notify();
  }

  fn toggle_sidebar_mode_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let next_mode = match self.sidebar_mode {
      GitSidebarMode::Changes => GitSidebarMode::History,
      GitSidebarMode::History => GitSidebarMode::Changes,
    };
    self.set_sidebar_mode(next_mode, window, cx);
  }

  fn focus_changes_sidebar_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.file_list.read(cx).selected_index().is_none() && !self.status_entries.is_empty() {
      self.file_list.update(cx, |state, cx| {
        state.set_selected_index(Some(IndexPath::new(0)), window, cx);
      });
    }

    self.file_list.update(cx, |state, cx| {
      state.focus(window, cx);
    });
  }

  fn focus_history_sidebar_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.history_tree.read(cx).selected_index().is_none()
      && let Some(first_row) = self.history_rows_cache.first()
    {
      let first_id = format!("history-commit:{}", first_row.commit.oid);
      self.history_tree.update(cx, |state, cx| {
        let item = TreeItem::new(first_id.clone(), first_id.clone());
        state.set_selected_item(Some(&item), cx);
        if let Some(ix) = state.selected_index() {
          state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
        }
      });
    }

    self.history_tree.update(cx, |state, cx| {
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
      state.focus(window, cx);
    });
  }

  fn focus_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    window.focus(&self.focus_handle, cx);
  }

  fn focus_editor_or_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(editor) = self.editor.clone() {
      let editor_focus_handle = editor.read(cx).focus_handle(cx);
      window.focus(&editor_focus_handle, cx);
      return;
    }

    self.focus_page(window, cx);
  }

  fn focus_terminal_sidebar_on_next_frame(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
    let terminal_view = self.terminal_view.clone();
    window.on_next_frame(move |window, cx| {
      let focus_handle = terminal_view.read(cx).focus_handle(cx);
      window.focus(&focus_handle, cx);
    });
  }

  fn toggle_terminal_sidebar_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.show_terminal_sidebar = !self.show_terminal_sidebar;
    if self.show_terminal_sidebar {
      crate::analytics::track(cx, "terminal_opened");
      self.focus_terminal_sidebar_on_next_frame(window, cx);
    } else {
      self.focus_editor_or_page(window, cx);
    }
    cx.notify();
  }

  fn toggle_terminal_sidebar_click(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_terminal_sidebar_visibility(window, cx);
  }

  fn toggle_terminal_sidebar_action(
    &mut self,
    _: &crate::ToggleTerminalSidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_terminal_sidebar_visibility(window, cx);
    cx.stop_propagation();
  }

  fn ensure_page_shortcut_focus(&self, cx: &mut Context<Self>) {
    let focus_handle = self.focus_handle.clone();
    let window_handle = self.window_handle;
    let _ = cx.update_window(window_handle, move |_, window, cx| {
      window.focus(&focus_handle, cx);
    });
  }

  fn focus_sidebar_on_next_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.sidebar_mode == GitSidebarMode::Changes {
      cx.on_next_frame(window, |this, window, cx| {
        if this.sidebar_mode == GitSidebarMode::Changes {
          this.focus_changes_sidebar_list(window, cx);
        }
      });
      return;
    }

    self.focus_history_sidebar_tree(window, cx);
    cx.on_next_frame(window, |this, window, cx| {
      if this.sidebar_mode == GitSidebarMode::History {
        this.focus_history_sidebar_tree(window, cx);
      }
    });
  }

  fn set_sidebar_mode(
    &mut self,
    mode: GitSidebarMode,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.sidebar_mode = mode;
    self.sync_sentry_git_context();
    let mut data = Map::new();
    data.insert(
      "sidebar_mode".into(),
      Self::sidebar_mode_tag(self.sidebar_mode).into(),
    );
    self.add_git_breadcrumb("Changed git sidebar mode", data);

    if self.sidebar_mode == GitSidebarMode::History {
      self.refresh_history(cx);
    } else {
      self.refresh_file_list(cx);
      cx.notify();
    }

    self.focus_sidebar_on_next_frame(window, cx);
  }

  fn status_color(kind: RepoStatusKind, theme: &gpui_component::Theme) -> gpui::Hsla {
    match kind {
      RepoStatusKind::Modified => theme.status_yellow(),
      RepoStatusKind::Added => theme.status_green(),
      RepoStatusKind::Deleted => theme.status_red(),
      RepoStatusKind::Renamed => theme.status_blue(),
      RepoStatusKind::TypeChange => theme.status_blue(),
      RepoStatusKind::Untracked => theme.status_green(),
      RepoStatusKind::Conflicted => theme.status_red(),
    }
  }

  fn status_tooltip(kind: RepoStatusKind) -> SharedString {
    match kind {
      RepoStatusKind::Modified => "Modified".into(),
      RepoStatusKind::Added => "Added".into(),
      RepoStatusKind::Deleted => "Deleted".into(),
      RepoStatusKind::Renamed => "Renamed".into(),
      RepoStatusKind::TypeChange => "Type changed".into(),
      RepoStatusKind::Untracked => "Untracked".into(),
      RepoStatusKind::Conflicted => "Conflicted".into(),
    }
  }

  fn selected_file_entry(&self) -> Option<&RepoStatusEntry> {
    let selected = self.selected_file.as_ref()?;
    self
      .status_entries
      .iter()
      .find(|entry| &entry.path == selected)
  }

  fn conflict_navigation_state_for(
    file_status: Option<RepoStatusKind>,
    editor: &Editor,
    cx: &App,
  ) -> Option<ConflictNavigationState> {
    matches!(file_status, Some(RepoStatusKind::Conflicted))
      .then(|| editor.conflict_navigation_state(cx))
      .flatten()
  }

  fn annotation_navigation_state_for(
    file_status: Option<RepoStatusKind>,
    editor: &Editor,
    cx: &App,
  ) -> Option<AnnotationNavigationState> {
    if let Some(state) = Self::conflict_navigation_state_for(file_status, editor, cx) {
      return Some(AnnotationNavigationState {
        active_index: state.active_index,
        total: state.total,
        kind: AnnotationKind::Conflict,
      });
    }
    editor
      .hunk_navigation_state(cx)
      .map(|state| AnnotationNavigationState {
        active_index: state.active_index,
        total: state.total,
        kind: AnnotationKind::Change,
      })
  }

  #[cfg(test)]
  fn editor_conflict_navigation_state(&self, cx: &App) -> Option<ConflictNavigationState> {
    let file_status = if self.history_opened_commit_file.is_some() {
      None
    } else {
      self.selected_file_entry().map(|entry| entry.status)
    };

    self.editor.as_ref().and_then(|editor| {
      editor.read_with(cx, |editor, cx| {
        Self::conflict_navigation_state_for(file_status, editor, cx)
      })
    })
  }

  #[cfg(test)]
  fn editor_annotation_navigation_state(&self, cx: &App) -> Option<AnnotationNavigationState> {
    let file_status = if self.history_opened_commit_file.is_some() {
      None
    } else {
      self.selected_file_entry().map(|entry| entry.status)
    };

    self.editor.as_ref().and_then(|editor| {
      editor.read_with(cx, |editor, cx| {
        Self::annotation_navigation_state_for(file_status, editor, cx)
      })
    })
  }

  fn can_navigate_annotations(state: Option<AnnotationNavigationState>) -> bool {
    state.is_some_and(|state| state.total > 1)
  }

  fn navigate_annotation_in_editor(
    &mut self,
    direction: AnnotationDirection,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let file_status = if self.history_opened_commit_file.is_some() {
      None
    } else {
      self.selected_file_entry().map(|entry| entry.status)
    };

    editor.update(cx, |editor, cx| {
      let use_conflict_nav = matches!(file_status, Some(RepoStatusKind::Conflicted))
        && editor.conflict_navigation_state(cx).is_some();
      if use_conflict_nav {
        editor.navigate_conflict(direction.conflict(), cx);
      } else {
        editor.navigate_hunk(direction.hunk(), cx);
      }
    });
  }

  fn selected_file_status(&self) -> Option<RepoStatusKind> {
    self.selected_file_entry().map(|entry| entry.status)
  }

  fn can_accept_all_conflicts(
    selected_status: Option<RepoStatusKind>,
    is_read_only: bool,
    has_unresolved_conflict_markers: bool,
  ) -> bool {
    matches!(selected_status, Some(RepoStatusKind::Conflicted))
      && !is_read_only
      && has_unresolved_conflict_markers
  }

  fn all_changes_staged(&self) -> bool {
    Self::should_show_unstage_all_command(&self.status_entries)
  }

  fn changed_files_count(entries: &[RepoStatusEntry]) -> usize {
    entries.len()
  }

  fn has_conflicted_entries(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| entry.status == RepoStatusKind::Conflicted)
  }

  fn has_untracked_entries(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| entry.status == RepoStatusKind::Untracked)
  }

  fn has_tracked_entries(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| entry.status != RepoStatusKind::Untracked)
  }

  fn should_show_stage_all_command(entries: &[RepoStatusEntry]) -> bool {
    Self::changed_files_count(entries) > 0 && !Self::all_entries_staged(entries)
  }

  fn should_show_unstage_all_command(entries: &[RepoStatusEntry]) -> bool {
    Self::all_entries_staged(entries)
  }

  fn build_history_rows(commits: &[HistoryCommitNode]) -> Vec<HistoryRenderRow> {
    commits
      .iter()
      .cloned()
      .map(HistoryRenderRow::from_commit)
      .collect()
  }

  fn history_change_kind_to_repo_status(kind: CommitFileChangeKind) -> RepoStatusKind {
    match kind {
      CommitFileChangeKind::Added => RepoStatusKind::Added,
      CommitFileChangeKind::Deleted => RepoStatusKind::Deleted,
      CommitFileChangeKind::Modified => RepoStatusKind::Modified,
      CommitFileChangeKind::Renamed => RepoStatusKind::Renamed,
      // RepoStatusKind does not have "Copied", closest visual semantics is renamed.
      CommitFileChangeKind::Copied => RepoStatusKind::Renamed,
      CommitFileChangeKind::Typechange => RepoStatusKind::TypeChange,
      CommitFileChangeKind::Conflicted => RepoStatusKind::Conflicted,
    }
  }
}

impl Focusable for GitPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::test_support::*;
  use super::*;
  use git2::Repository;
  use gpui::TestAppContext;
  use ui::CommandPaletteCommandId;

  use crate::api::UserRole;

  #[test]
  fn agent_path_strips_repo_root_when_absolute() {
    let root = Path::new("/home/u/proj");
    assert_eq!(
      agent_path_to_repo_relative(PathBuf::from("/home/u/proj/src/lib.rs"), Some(root)),
      PathBuf::from("src/lib.rs")
    );
  }

  #[test]
  fn agent_path_kept_when_outside_root_or_already_relative() {
    let root = Path::new("/home/u/proj");
    assert_eq!(
      agent_path_to_repo_relative(PathBuf::from("src/lib.rs"), Some(root)),
      PathBuf::from("src/lib.rs")
    );
    assert_eq!(
      agent_path_to_repo_relative(PathBuf::from("/other/x.rs"), Some(root)),
      PathBuf::from("/other/x.rs")
    );
    assert_eq!(
      agent_path_to_repo_relative(PathBuf::from("/abs/x.rs"), None),
      PathBuf::from("/abs/x.rs")
    );
  }

  #[test]
  fn open_action_repo_item_is_sentinel() {
    let action = RecentRepoItem::open_action();
    assert!(action.is_action);
    assert!(
      action.value().as_os_str().is_empty(),
      "empty path is the sentinel the repo select handler checks to open the picker"
    );
    assert!(
      action.matches("anything"),
      "action stays visible while searching"
    );
  }

  #[test]
  fn recent_repo_item_splits_prefix_and_name() {
    let repo = RecentRepository {
      path: PathBuf::from("/Users/example/workspace/reviu"),
    };
    let item = RecentRepoItem::new(&repo, Some(Path::new("/Users/example/workspace/reviu")));

    assert_eq!(item.path, PathBuf::from("/Users/example/workspace/reviu"));
    assert_eq!(item.prefix.as_ref(), "/Users/example/workspace/");
    assert_eq!(item.name.as_ref(), "reviu");
    assert!(item.is_selected);
  }

  #[test]
  fn git_refresh_helper_reports_loading_when_status_or_branch_refresh_is_running() {
    assert!(!git_refresh_in_progress(false, false));
    assert!(git_refresh_in_progress(true, false));
    assert!(git_refresh_in_progress(false, true));
    assert!(git_refresh_in_progress(true, true));
  }

  #[gpui::test]
  fn git_page_handle_refreshing_ignores_lingering_tasks(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    cx.update(|_, cx| {
      assert!(!GitPageHandle::is_refreshing(cx));
    });

    git_page.update_in(cx, |this, _window, cx| {
      this.status_task = Some(cx.spawn(async move |_, _| {}));
      this.branch_task = Some(cx.spawn(async move |_, _| {}));
      this.status_refresh_in_progress = false;
      this.branch_refresh_in_progress = false;
    });

    cx.update(|_, cx| {
      assert!(!GitPageHandle::is_refreshing(cx));
    });

    git_page.update_in(cx, |this, _window, _cx| {
      this.status_refresh_in_progress = true;
    });

    cx.update(|_, cx| {
      assert!(GitPageHandle::is_refreshing(cx));
    });
  }

  #[gpui::test]
  fn init_gpui_test_registers_required_globals(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    cx.update(|cx| {
      assert!(cx.has_global::<WorkspaceApi>());
      assert!(cx.has_global::<AuthStateStore>());
      assert!(cx.has_global::<ActiveLocalRepoStore>());
      assert_eq!(ActiveLocalRepoStore::get(cx), None);
    });
  }

  #[test]
  fn github_branch_context_from_active_repo_requires_repo_and_named_branch() {
    let ready = ActiveLocalRepo {
      repo_root: PathBuf::from("/tmp/repo"),
      github_owner: Some("acme".to_string()),
      github_repo: Some("widget".to_string()),
      current_branch: Some("feature/parser".to_string()),
      head_sha: Some("head".to_string()),
      has_uncommitted_changes: false,
    };
    assert_eq!(
      GitPage::github_branch_context_from_active_repo(&ready),
      Some(GithubBranchContext {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
        branch: "feature/parser".to_string(),
      })
    );

    let detached = ActiveLocalRepo {
      current_branch: Some("HEAD".to_string()),
      ..ready.clone()
    };
    assert_eq!(
      GitPage::github_branch_context_from_active_repo(&detached),
      None
    );

    let missing_remote = ActiveLocalRepo {
      github_owner: None,
      ..ready
    };
    assert_eq!(
      GitPage::github_branch_context_from_active_repo(&missing_remote),
      None
    );
  }

  #[test]
  fn branch_pr_button_state_prefers_open_existing_pull_request() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };
    let pull_request = make_branch_pull_request(42);

    assert_eq!(
      GitPage::branch_pr_button_state(
        Some(&context),
        true,
        true,
        false,
        false,
        Some(&pull_request)
      ),
      GitBranchPullRequestButtonState::OpenExisting {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
        number: 42,
      }
    );
  }

  #[test]
  fn branch_pr_button_state_shows_loading_only_when_in_app_lookup_is_available() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };

    assert_eq!(
      GitPage::branch_pr_button_state(Some(&context), true, true, false, true, None),
      GitBranchPullRequestButtonState::Checking
    );
    assert_eq!(
      GitPage::branch_pr_button_state(Some(&context), false, true, false, true, None),
      GitBranchPullRequestButtonState::LockedPro
    );
  }

  #[test]
  fn branch_pr_button_state_locks_branch_pull_request_button_without_github_access() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };
    let pull_request = make_branch_pull_request(42);

    assert_eq!(
      GitPage::branch_pr_button_state(
        Some(&context),
        false,
        true,
        false,
        false,
        Some(&pull_request),
      ),
      GitBranchPullRequestButtonState::LockedPro
    );
  }

  #[test]
  fn branch_pr_button_state_stays_hidden_without_github_branch_context() {
    assert_eq!(
      GitPage::branch_pr_button_state(None, false, true, false, false, None),
      GitBranchPullRequestButtonState::Hidden
    );
  }

  #[test]
  fn branch_pr_button_state_shows_create_when_branch_has_no_open_pull_request() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };

    assert_eq!(
      GitPage::branch_pr_button_state(Some(&context), true, true, false, false, None),
      GitBranchPullRequestButtonState::Create
    );
  }

  #[test]
  fn should_apply_created_pull_request_only_for_matching_branch_context() {
    let active_context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };
    let other_context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/other".to_string(),
    };

    assert!(GitPage::should_apply_created_pull_request(
      Some(&active_context),
      &active_context
    ));
    assert!(!GitPage::should_apply_created_pull_request(
      Some(&active_context),
      &other_context
    ));
    assert!(!GitPage::should_apply_created_pull_request(
      None,
      &active_context
    ));
  }

  #[test]
  fn branch_pr_button_state_shows_publish_and_create_for_unpublished_branch() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };

    assert_eq!(
      GitPage::branch_pr_button_state(Some(&context), true, false, true, false, None),
      GitBranchPullRequestButtonState::PublishAndCreate
    );
  }

  #[test]
  fn branch_pr_button_state_hides_publish_and_create_without_unique_branch_commits() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };

    assert_eq!(
      GitPage::branch_pr_button_state(Some(&context), true, false, false, false, None),
      GitBranchPullRequestButtonState::Hidden
    );
  }

  #[test]
  fn branch_has_github_upstream_requires_named_branch_with_upstream() {
    let published = make_branch_status("feature/parser", 0, 0, true);
    let local_only = make_branch_status("feature/parser", 0, 0, false);
    let detached = make_branch_status("HEAD", 0, 0, true);

    assert!(GitPage::branch_has_github_upstream(Some(&published)));
    assert!(!GitPage::branch_has_github_upstream(Some(&local_only)));
    assert!(!GitPage::branch_has_github_upstream(Some(&detached)));
    assert!(!GitPage::branch_has_github_upstream(None));
  }

  #[gpui::test]
  fn command_palette_create_pull_request_follows_branch_button_visibility_rules(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-command-palette-create-pr");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "initial\n", "initial");

    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
      ActiveLocalRepoStore::set(
        cx,
        Some(ActiveLocalRepo {
          repo_root: repo.path.clone(),
          github_owner: Some("acme".to_string()),
          github_repo: Some("widget".to_string()),
          current_branch: Some("main".to_string()),
          head_sha: Some("deadbeef".to_string()),
          has_uncommitted_changes: false,
        }),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let commands = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status("main", 0, 0, true));
      this.branch_pr_lookup_loading = false;
      this.branch_pr_lookup_result = None;
      this.build_command_palette_contents(1, cx).commands
    });
    assert!(
      commands
        .iter()
        .any(|command| command.id == CommandPaletteCommandId::CreatePullRequest)
    );
    assert!(
      !commands
        .iter()
        .any(|command| command.id == CommandPaletteCommandId::OpenPullRequest)
    );

    let commands_with_existing_pr = git_page.update(cx, |this, cx| {
      this.branch_pr_lookup_result = Some(make_branch_pull_request(42));
      this.build_command_palette_contents(1, cx).commands
    });
    assert!(
      !commands_with_existing_pr
        .iter()
        .any(|command| command.id == CommandPaletteCommandId::CreatePullRequest)
    );
    let open_pr_command = commands_with_existing_pr
      .iter()
      .find(|command| command.id == CommandPaletteCommandId::OpenPullRequest)
      .expect("existing branch PR should add an open command");
    assert_eq!(open_pr_command.name.as_ref(), "Open PR #42");
  }

  #[gpui::test]
  async fn refresh_branch_pr_lookup_skips_lookup_without_github_access(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::User))),
      );
      ActiveLocalRepoStore::set(
        cx,
        Some(ActiveLocalRepo {
          repo_root: PathBuf::from("/tmp/reviu-git-page-branch-pr-no-access"),
          github_owner: Some("acme".to_string()),
          github_repo: Some("widget".to_string()),
          current_branch: Some("feature/parser".to_string()),
          head_sha: Some("deadbeef".to_string()),
          has_uncommitted_changes: false,
        }),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.refresh_branch_pr_lookup(cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    git_page.read_with(cx, |this, cx| {
      assert_eq!(this.branch_pr_lookup_context, None);
      assert!(this.branch_pr_lookup_task.is_none());
      assert!(!this.branch_pr_lookup_loading);
      assert!(this.branch_pr_lookup_result.is_none());
      assert!(!AuthStateStore::has_github_access(cx));
    });
  }

  #[gpui::test]
  async fn refresh_branch_pr_lookup_skips_lookup_for_unpublished_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
      ActiveLocalRepoStore::set(
        cx,
        Some(ActiveLocalRepo {
          repo_root: PathBuf::from("/tmp/reviu-git-page-branch-pr-unpublished"),
          github_owner: Some("acme".to_string()),
          github_repo: Some("widget".to_string()),
          current_branch: Some("feature/parser".to_string()),
          head_sha: Some("deadbeef".to_string()),
          has_uncommitted_changes: false,
        }),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.branch_status = Some(make_branch_status("feature/parser", 0, 0, false));
      this.refresh_branch_pr_lookup(cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    git_page.read_with(cx, |this, cx| {
      assert_eq!(this.branch_pr_lookup_context, None);
      assert!(this.branch_pr_lookup_task.is_none());
      assert!(!this.branch_pr_lookup_loading);
      assert!(this.branch_pr_lookup_result.is_none());
      assert!(AuthStateStore::has_github_access(cx));
      assert!(!GitPage::branch_has_github_upstream(
        this.branch_status.as_ref()
      ));
    });
  }

  #[gpui::test]
  fn apply_created_pull_request_updates_branch_pr_lookup_for_matching_context(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo_root = PathBuf::from("/tmp/reviu-git-page-created-pr");
    let branch_context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };
    let pull_request = make_branch_pull_request(42);

    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
      ActiveLocalRepoStore::set(
        cx,
        Some(ActiveLocalRepo {
          repo_root: repo_root.clone(),
          github_owner: Some(branch_context.owner.clone()),
          github_repo: Some(branch_context.repo.clone()),
          current_branch: Some(branch_context.branch.clone()),
          head_sha: Some("deadbeef".to_string()),
          has_uncommitted_changes: false,
        }),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let create_pr_hidden = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo_root.clone());
      this.branch_status = Some(make_branch_status(
        branch_context.branch.as_str(),
        0,
        0,
        true,
      ));
      this.branch_pr_lookup_loading = true;
      this.apply_created_pull_request(&branch_context, &pull_request, cx);
      !this
        .build_command_palette_contents(1, cx)
        .commands
        .into_iter()
        .any(|command| command.id == CommandPaletteCommandId::CreatePullRequest)
    });

    assert!(create_pr_hidden);
    git_page.read_with(cx, |this, _cx| {
      assert_eq!(
        this.branch_pr_lookup_context.as_ref(),
        Some(&branch_context)
      );
      assert_eq!(
        this
          .branch_pr_lookup_result
          .as_ref()
          .map(|pull_request| pull_request.number),
        Some(pull_request.number)
      );
      assert!(!this.branch_pr_lookup_loading);
      assert!(this.branch_pr_lookup_task.is_none());
    });
  }

  #[gpui::test]
  fn apply_created_pull_request_ignores_stale_branch_context(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo_root = PathBuf::from("/tmp/reviu-git-page-created-pr-stale");
    let active_context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };
    let stale_context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/other".to_string(),
    };
    let pull_request = make_branch_pull_request(42);

    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
      ActiveLocalRepoStore::set(
        cx,
        Some(ActiveLocalRepo {
          repo_root: repo_root.clone(),
          github_owner: Some(active_context.owner.clone()),
          github_repo: Some(active_context.repo.clone()),
          current_branch: Some(active_context.branch.clone()),
          head_sha: Some("deadbeef".to_string()),
          has_uncommitted_changes: false,
        }),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo_root.clone());
      this.branch_status = Some(make_branch_status(
        active_context.branch.as_str(),
        0,
        0,
        true,
      ));
      this.branch_pr_lookup_loading = true;
      this.apply_created_pull_request(&stale_context, &pull_request, cx);
    });

    git_page.read_with(cx, |this, _cx| {
      assert!(this.branch_pr_lookup_context.is_none());
      assert!(this.branch_pr_lookup_result.is_none());
      assert!(this.branch_pr_lookup_loading);
      assert!(this.branch_pr_lookup_task.is_none());
    });
  }

  #[test]
  fn branch_name_changed_detects_name_transitions() {
    let main = make_branch_status("main", 0, 0, true);
    let feature = make_branch_status("feature", 0, 0, true);

    assert!(GitPage::branch_name_changed(None, Some(&main)));
    assert!(GitPage::branch_name_changed(Some(&main), Some(&feature)));
    assert!(GitPage::branch_name_changed(Some(&main), None));
    assert!(!GitPage::branch_name_changed(None, None));
    assert!(!GitPage::branch_name_changed(Some(&main), Some(&main)));
  }

  #[test]
  fn can_navigate_annotations_requires_multiple_annotations() {
    assert!(!GitPage::can_navigate_annotations(None));
    assert!(!GitPage::can_navigate_annotations(Some(
      AnnotationNavigationState {
        active_index: 0,
        total: 1,
        kind: AnnotationKind::Conflict,
      }
    )));
    assert!(GitPage::can_navigate_annotations(Some(
      AnnotationNavigationState {
        active_index: 0,
        total: 2,
        kind: AnnotationKind::Conflict,
      }
    )));
    assert!(GitPage::can_navigate_annotations(Some(
      AnnotationNavigationState {
        active_index: 0,
        total: 3,
        kind: AnnotationKind::Change,
      }
    )));
  }

  #[test]
  fn changed_files_count_matches_status_entries_len() {
    assert_eq!(GitPage::changed_files_count(&[]), 0);

    let entries = vec![
      make_status_entry("src/a.rs", RepoStage::Unstaged),
      make_status_entry("src/b.rs", RepoStage::Staged),
      make_status_entry("src/c.rs", RepoStage::PartiallyStaged),
    ];
    assert_eq!(GitPage::changed_files_count(&entries), 3);
  }

  #[test]
  fn has_tracked_entries_excludes_untracked_only_state() {
    let untracked_entries = vec![RepoStatusEntry {
      path: PathBuf::from("notes.txt"),
      old_path: None,
      status: RepoStatusKind::Untracked,
      stage: RepoStage::Unstaged,
    }];
    let tracked_entries = vec![
      RepoStatusEntry {
        path: PathBuf::from("notes.txt"),
        old_path: None,
        status: RepoStatusKind::Untracked,
        stage: RepoStage::Unstaged,
      },
      make_status_entry("src/main.rs", RepoStage::Unstaged),
    ];

    assert!(!GitPage::has_tracked_entries(&untracked_entries));
    assert!(GitPage::has_tracked_entries(&tracked_entries));
  }

  #[test]
  fn stash_command_flags_follow_untracked_only_rule() {
    let untracked_entries = vec![RepoStatusEntry {
      path: PathBuf::from("notes.txt"),
      old_path: None,
      status: RepoStatusKind::Untracked,
      stage: RepoStage::Unstaged,
    }];
    let tracked_entries = vec![make_status_entry("src/main.rs", RepoStage::Unstaged)];

    assert_eq!(GitPage::stash_command_flags(&[]), (false, false));
    assert_eq!(
      GitPage::stash_command_flags(&untracked_entries),
      (false, true)
    );
    assert_eq!(GitPage::stash_command_flags(&tracked_entries), (true, true));
  }

  #[gpui::test]
  fn focus_changes_sidebar_list_selects_first_entry_when_unselected(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.status_entries = vec![
        make_status_entry("src/main.rs", RepoStage::Unstaged),
        make_status_entry("src/lib.rs", RepoStage::Unstaged),
      ];
      this.refresh_file_list(cx);
      assert_eq!(this.file_list.read(cx).selected_index(), None);

      this.focus_changes_sidebar_list(window, cx);
      assert_eq!(
        this.file_list.read(cx).selected_index(),
        Some(IndexPath::new(0))
      );
    });
  }

  #[gpui::test]
  async fn repo_select_confirm_refocuses_page_shortcuts(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo_a = TempRepo::init("git-page-focus-after-repo-select-a");
    let repo_b = TempRepo::init("git-page-focus-after-repo-select-b");
    let _ = commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    let _ = commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo_a.path.clone());

      let external_focus = cx.focus_handle();
      window.focus(&external_focus, cx);
      assert!(!this.focus_handle.contains_focused(window, cx));
    });

    git_page.update(cx, |this, cx| {
      this.handle_repo_select_confirm(repo_b.path.clone(), cx);
    });

    let (has_page_focus, selected_repo) = git_page.update_in(cx, |this, window, cx| {
      (
        this.focus_handle.contains_focused(window, cx),
        this.selected_repo.clone(),
      )
    });
    assert!(has_page_focus);
    assert_eq!(selected_repo, Some(repo_b.path.clone()));

    await_git_page_background_tasks(git_page.clone(), cx).await;
  }

  #[gpui::test]
  fn branch_select_confirm_refocuses_page_shortcuts(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(PathBuf::from("/tmp/reviu-focus-select-branch"));

      let external_focus = cx.focus_handle();
      window.focus(&external_focus, cx);
      assert!(!this.focus_handle.contains_focused(window, cx));
    });

    git_page.update(cx, |this, cx| {
      this.handle_branch_select_confirm(GitPage::detached_branch_select_value(), cx);
    });

    let (has_page_focus, branch_task_is_none) = git_page.update_in(cx, |this, window, cx| {
      (
        this.focus_handle.contains_focused(window, cx),
        this.branch_task.is_none(),
      )
    });
    assert!(has_page_focus);
    assert!(branch_task_is_none);
  }

  #[gpui::test]
  async fn reload_status_refocuses_page_when_selected_file_disappears(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-focus-after-selection-clear");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.selected_file = Some(rel_path.to_path_buf());
      this.status_entries = vec![make_status_entry("README.md", RepoStage::Unstaged)];

      let external_focus = cx.focus_handle();
      window.focus(&external_focus, cx);
      assert!(!this.focus_handle.contains_focused(window, cx));
    });

    restore_file(&repo.path, rel_path).expect("restore file on disk");

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_file, has_page_focus) = git_page.update_in(cx, |this, window, cx| {
      (
        this.selected_file.clone(),
        this.focus_handle.contains_focused(window, cx),
      )
    });
    assert!(selected_file.is_none());
    assert!(has_page_focus);
  }

  #[test]
  fn selected_file_update_clears_missing_selection_without_history_file() {
    let update = GitPage::selected_file_update(
      Some(Path::new("src/missing.rs")),
      Some(SelectedFileSource::StatusEntry),
      &[make_status_entry("src/exists.rs", RepoStage::Unstaged)],
      false,
      true,
    );
    assert_eq!(
      update,
      SelectedFileUpdate {
        clear_selection: true,
        sync_diff_view: false,
      }
    );
  }

  #[test]
  fn selected_file_update_keeps_selection_and_syncs_when_present() {
    let update = GitPage::selected_file_update(
      Some(Path::new("src/main.rs")),
      Some(SelectedFileSource::StatusEntry),
      &[make_status_entry("src/main.rs", RepoStage::Unstaged)],
      false,
      true,
    );
    assert_eq!(
      update,
      SelectedFileUpdate {
        clear_selection: false,
        sync_diff_view: true,
      }
    );
  }

  #[test]
  fn selected_file_update_keeps_project_file_when_missing_from_status() {
    let update = GitPage::selected_file_update(
      Some(Path::new("src/main.rs")),
      Some(SelectedFileSource::ProjectFile),
      &[make_status_entry("src/other.rs", RepoStage::Unstaged)],
      false,
      true,
    );
    assert_eq!(
      update,
      SelectedFileUpdate {
        clear_selection: false,
        sync_diff_view: true,
      }
    );
  }

  #[test]
  fn selected_file_update_never_clears_when_history_file_is_open() {
    let update = GitPage::selected_file_update(
      Some(Path::new("src/main.rs")),
      Some(SelectedFileSource::StatusEntry),
      &[make_status_entry("src/other.rs", RepoStage::Unstaged)],
      true,
      true,
    );
    assert_eq!(update, SelectedFileUpdate::default());
  }

  #[test]
  fn selected_file_update_is_noop_without_selection() {
    let update = GitPage::selected_file_update(
      None,
      Some(SelectedFileSource::StatusEntry),
      &[make_status_entry("src/main.rs", RepoStage::Unstaged)],
      false,
      true,
    );
    assert_eq!(update, SelectedFileUpdate::default());
  }

  #[test]
  fn selected_branch_from_status_maps_detached_head_to_detached_select_value() {
    let detached = make_branch_status("HEAD", 0, 0, false);
    assert_eq!(
      GitPage::selected_branch_from_status(Some(&detached)),
      Some(GitPage::detached_branch_select_value())
    );
  }

  #[test]
  fn selected_branch_from_status_maps_named_head_to_local_branch() {
    let main = make_branch_status("main", 0, 0, true);
    assert_eq!(
      GitPage::selected_branch_from_status(Some(&main)),
      Some(BranchRef {
        name: "main".to_string(),
        kind: BranchKind::Local,
      })
    );
  }

  #[test]
  fn branch_select_items_marks_only_selected_branch() {
    let selected = BranchRef {
      name: "main".to_string(),
      kind: BranchKind::Local,
    };
    let items = GitPage::branch_select_items(
      vec![
        BranchRef {
          name: "main".to_string(),
          kind: BranchKind::Local,
        },
        BranchRef {
          name: "feature".to_string(),
          kind: BranchKind::Local,
        },
      ],
      Some(&selected),
      None,
    );

    assert_eq!(items.len(), 2);
    assert!(items[0].is_current);
    assert!(!items[1].is_current);
  }

  #[test]
  fn branch_select_items_includes_detached_head_entry_when_selected_is_detached() {
    let items = GitPage::branch_select_items(
      vec![BranchRef {
        name: "main".to_string(),
        kind: BranchKind::Local,
      }],
      Some(&GitPage::detached_branch_select_value()),
      Some("v1.0.0"),
    );

    assert_eq!(items.len(), 2);
    assert!(items[0].is_current);
    assert_eq!(items[0].label.as_ref(), "HEAD (v1.0.0)");
    assert!(GitPage::is_detached_branch_select_value(&items[0].branch));
    assert!(!items[1].is_current);
  }

  #[gpui::test]
  async fn open_file_loads_editor_asynchronously(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-open-file-async");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("update worktree file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_file(rel_path.to_path_buf(), cx);
      assert!(
        this.open_file_task.is_some(),
        "open file should schedule an async load task"
      );
      assert_eq!(this.selected_file, Some(rel_path.to_path_buf()));
      assert!(
        this.editor.is_none(),
        "editor should be created after async load"
      );
    });

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_file, is_read_only, contents) = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("editor should exist").read(cx);
      let document = editor.document().read(cx);
      (
        this.selected_file.clone(),
        editor.is_read_only,
        document.slice_to_string(0..document.len()),
      )
    });

    assert_eq!(selected_file, Some(rel_path.to_path_buf()));
    assert!(!is_read_only);
    assert_eq!(contents, "v2\n");
  }

  #[gpui::test]
  async fn reload_status_keeps_unchanged_project_file_open(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-clean-project-file-stays-open");
    let rel_path = Path::new("src/main.rs");
    std::fs::create_dir_all(repo.path.join("src")).expect("create source dir");
    let _ = commit_text_file(&repo.path, rel_path, "fn main() {}\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    git_page.read_with(cx, |this, _cx| {
      assert_eq!(this.selected_file.as_deref(), Some(rel_path));
      assert_eq!(
        this.selected_file_source,
        Some(SelectedFileSource::ProjectFile)
      );
      assert!(this.editor.is_some());
      assert!(this.status_entries.is_empty());
    });
  }

  #[gpui::test]
  async fn project_file_selection_uses_status_entry_after_file_changes(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-project-file-becomes-changed");
    let rel_path = Path::new("src/main.rs");
    std::fs::create_dir_all(repo.path.join("src")).expect("create source dir");
    let _ = commit_text_file(&repo.path, rel_path, "fn main() {}\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    std::fs::write(
      repo.path.join(rel_path),
      "fn main() { println!(\"changed\"); }\n",
    )
    .expect("modify project file");

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    git_page.read_with(cx, |this, _cx| {
      assert_eq!(this.selected_file.as_deref(), Some(rel_path));
      assert!(this.editor.is_some());
      assert_eq!(
        this.selected_file_entry().map(|entry| entry.status),
        Some(RepoStatusKind::Modified)
      );
    });
  }

  #[test]
  fn external_branch_switch_updates_branch_status_and_branch_select_model() {
    let repo = TempRepo::init("git-page-external-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let initial_status = current_branch_status(&repo.path).expect("read initial branch");
    let initial_branch_name = initial_status.name.clone();
    create_branch(&repo.path, "feature").expect("create feature branch");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    repo_handle
      .set_head("refs/heads/feature")
      .expect("set HEAD to feature");

    let switched_status = current_branch_status(&repo.path).expect("read switched branch");
    assert_eq!(switched_status.name, "feature");
    assert!(GitPage::branch_name_changed(
      Some(&initial_status),
      Some(&switched_status)
    ));

    let branches = list_branches(&repo.path).expect("list branches");
    let selected = GitPage::selected_branch_from_status(Some(&switched_status));
    let items = GitPage::branch_select_items(branches, selected.as_ref(), None);

    assert_eq!(items.iter().filter(|item| item.is_current).count(), 1);
    assert!(
      items
        .iter()
        .any(|item| item.branch.kind == BranchKind::Local
          && item.branch.name == "feature"
          && item.is_current)
    );
    if initial_branch_name != "feature" {
      assert!(!items.iter().any(|item| {
        item.branch.kind == BranchKind::Local
          && item.branch.name == initial_branch_name
          && item.is_current
      }));
    }
  }

  #[test]
  fn external_detached_head_selects_detached_entry_in_branch_select_model() {
    let repo = TempRepo::init("git-page-external-detach");
    let oid = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let initial_status = current_branch_status(&repo.path).expect("read initial branch");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    repo_handle.set_head_detached(oid).expect("detach HEAD");

    let detached_status = current_branch_status(&repo.path).expect("read detached status");
    assert_eq!(detached_status.name, "HEAD");
    assert!(GitPage::branch_name_changed(
      Some(&initial_status),
      Some(&detached_status)
    ));

    let branches = list_branches(&repo.path).expect("list branches");
    let selected = GitPage::selected_branch_from_status(Some(&detached_status));
    assert_eq!(selected, Some(GitPage::detached_branch_select_value()));
    let detached_label = detached_head_label(&repo.path).ok();
    let items =
      GitPage::branch_select_items(branches, selected.as_ref(), detached_label.as_deref());
    assert!(
      items
        .iter()
        .any(|item| { GitPage::is_detached_branch_select_value(&item.branch) && item.is_current }),
      "detached HEAD entry should be selected"
    );
  }

  #[gpui::test]
  async fn publish_branch_and_create_pr_action_publishes_branch_and_opens_dialog(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
    });

    let source = TempRepo::init("git-page-publish-pr-source");
    let remote = TempBareRepo::init("git-page-publish-pr-remote");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    source_repo
      .remote("github", "git@github.com:acme/widget.git")
      .expect("add github remote");

    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;

    let details_body = r#"{
      "name": "widget",
      "full_name": "acme/widget",
      "description": "A sample repository",
      "homepage": null,
      "language": "Rust",
      "default_branch": "main",
      "stargazers_count": 0,
      "forks_count": 0,
      "subscribers_count": 0,
      "open_issues_count": 0,
      "size": 1,
      "pushed_at": "2026-03-20T12:00:00Z",
      "html_url": "https://github.com/acme/widget",
      "owner": {
        "login": "acme",
        "avatar_url": "https://example.com/avatar.png"
      },
      "license": null
    }"#;
    let tree_body = r#"{
      "sha": "head123",
      "url": "https://api.github.com/repos/acme/widget/trees/head123",
      "tree": [],
      "truncated": false
    }"#;
    let (base_url, handle) = start_matching_response_server(vec![
      (
        format!("GET /github/repos/acme/widget/pr/branch?branch={branch_name}"),
        "200 OK".to_string(),
        r#"{"pullRequest":null}"#.to_string(),
      ),
      (
        "GET /github/repos/acme/widget HTTP/1.1".to_string(),
        "200 OK".to_string(),
        details_body.to_string(),
      ),
      (
        "GET /github/repos/acme/widget/trees/main?recursive=1 HTTP/1.1".to_string(),
        "200 OK".to_string(),
        tree_body.to_string(),
      ),
    ]);

    let mut mounted_git_page = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");

    let publish_task = git_page.update_in(cx, |this, _window, cx| {
      this.api = make_test_api_client(base_url.clone());
      this.selected_repo = Some(source.path.clone());
      this.branch_status = Some(current_branch_status(&source.path).expect("branch status"));
      this.has_unpublished_branch_commits = true;
      this.sync_active_local_repo(cx);
      this.publish_branch_and_create_pull_request_action(cx);
      assert!(this.push_pull_in_progress);
      assert!(this.publish_branch_and_create_pr_in_progress);
      this.status_task.take().expect("publish task")
    });
    publish_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    cx.cx.run_until_parked();
    cx.run_until_parked();
    cx.cx.run_until_parked();
    cx.run_until_parked();
    handle.join().expect("join server thread");

    let status = current_branch_status(&source.path).expect("status after publish");
    assert!(status.has_upstream);
    assert_eq!(
      remote_branch_oid(&remote.path, &branch_name),
      head_oid(&source.path)
    );
    assert!(!git_page.read_with(cx, |this, _| this.push_pull_in_progress));
    assert!(!git_page.read_with(cx, |this, _| this.publish_branch_and_create_pr_in_progress));
  }

  #[gpui::test]
  async fn poll_once_updates_branch_status_after_external_branch_switch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-poll-once-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      seed_repo_branch_state(this, &repo.path, cx);
    });

    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("read branch after switch")
        .name,
      "feature"
    );

    let status_task = git_page.update_in(cx, |this, _window, cx| {
      this.poll_once_for_test(cx);
      this.status_task.take().expect("poll status task")
    });
    status_task.await;
    let branch_name = git_page.read_with(cx, |this, _| {
      this
        .branch_status
        .as_ref()
        .map(|status| status.name.clone())
    });
    assert_eq!(branch_name.as_deref(), Some("feature"));
  }

  #[gpui::test]
  async fn poll_once_does_not_keep_manual_refresh_stuck_loading(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-poll-once-refresh-race");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      assert!(this.status_refresh_in_progress);
      this.status_task.take().expect("reload status task")
    });
    let poll_task = git_page.update_in(cx, |this, _window, cx| {
      this.poll_once_for_test(cx);
      this.status_task.take().expect("poll status task")
    });

    reload_task.await;
    poll_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    cx.update(|_, cx| {
      assert!(
        !GitPageHandle::is_refreshing(cx),
        "manual refresh should not stay stuck after poll runs"
      );
    });
  }

  #[gpui::test]
  async fn branch_select_switch_keeps_status_empty_when_target_branch_is_clean(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-branch-switch-clean-status");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "main\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature\n", "feature commit");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let initial_reload = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("initial reload task")
    });
    initial_reload.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let initial_entries = git_page.read_with(cx, |this, _| this.status_entries.clone());
    assert!(
      initial_entries.is_empty(),
      "base branch should start clean, got: {initial_entries:?}"
    );

    git_page.update_in(cx, |this, _window, cx| {
      this.handle_branch_select_confirm(
        BranchRef {
          name: "feature".to_string(),
          kind: BranchKind::Local,
        },
        cx,
      );
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (entries, branch_name, selected_branch) = git_page.read_with(cx, |this, _| {
      (
        this.status_entries.clone(),
        this
          .branch_status
          .as_ref()
          .map(|status| status.name.clone()),
        selected_branch_from_dropdown(this),
      )
    });

    assert!(
      entries.is_empty(),
      "feature branch should stay clean after switch, got: {entries:?}"
    );
    assert_eq!(branch_name.as_deref(), Some("feature"));
    assert_eq!(
      selected_branch,
      Some(BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      })
    );
  }

  #[gpui::test]
  async fn branch_select_switch_failure_shows_error_notification(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-branch-switch-failure-notification");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "main\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature\n", "feature commit");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    std::fs::write(repo.path.join(rel_path), "main local change\n").expect("write local change");

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_reload = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("initial reload task")
    });
    initial_reload.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    git_page.update_in(cx, |this, _window, cx| {
      this.handle_branch_select_confirm(
        BranchRef {
          name: "feature".to_string(),
          kind: BranchKind::Local,
        },
        cx,
      );
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (branch_name, selected_branch) = git_page.update_in(cx, |this, _window, _cx| {
      (
        this
          .branch_status
          .as_ref()
          .map(|status| status.name.clone()),
        selected_branch_from_dropdown(this),
      )
    });
    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });

    assert_eq!(
      current_branch_status(&repo.path)
        .expect("read branch after failed switch")
        .name,
      base_branch
    );
    assert_eq!(branch_name.as_deref(), Some(base_branch.as_str()));
    assert_eq!(
      selected_branch,
      Some(BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      })
    );
    assert_eq!(notification_count, 1);

    // Autohide is paused while the window is inactive; activate it first.
    root.update_in(cx, |_this, window, _cx| window.activate_window());
    cx.cx.run_until_parked();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.cx.run_until_parked();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count_after_autohide = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count_after_autohide, 0);
  }

  #[gpui::test]
  async fn poll_once_selects_detached_entry_on_external_detached_head(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-poll-once-detached");
    let oid = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      seed_repo_branch_state(this, &repo.path, cx);
    });

    Repository::open(&repo.path)
      .expect("open repo")
      .set_head_detached(oid)
      .expect("detach HEAD");

    let status_task = git_page.update_in(cx, |this, _window, cx| {
      this.poll_once_for_test(cx);
      this.status_task.take().expect("poll status task")
    });
    status_task.await;
    if let Some(branch_task) = git_page.update_in(cx, |this, _window, _| this.branch_task.take()) {
      branch_task.await;
    }

    let (branch_name, selected_branch) = git_page.read_with(cx, |this, _cx| {
      (
        this
          .branch_status
          .as_ref()
          .map(|status| status.name.clone()),
        selected_branch_from_dropdown(this),
      )
    });
    assert_eq!(branch_name.as_deref(), Some("HEAD"));
    assert_eq!(
      selected_branch,
      Some(GitPage::detached_branch_select_value())
    );
  }

  #[gpui::test]
  async fn refresh_branches_updates_branch_select_after_external_branch_switch(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-refresh-branches-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      seed_repo_branch_state(this, &repo.path, cx);
    });

    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("read branch after switch")
        .name,
      "feature"
    );

    let branch_task = git_page.update_in(cx, |this, _window, cx| {
      this.refresh_branches(cx);
      this.branch_task.take().expect("refresh branches task")
    });
    branch_task.await;

    let selected_branch = git_page.read_with(cx, |this, _cx| selected_branch_from_dropdown(this));
    assert_eq!(
      selected_branch,
      Some(BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      })
    );
  }

  #[gpui::test]
  async fn reload_status_clears_git_state_when_selected_repo_becomes_unavailable(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-reload-missing-repo");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      seed_repo_branch_state(this, &repo.path, cx);
      this.status_entries = vec![make_status_entry("README.md", RepoStage::Unstaged)];
      this.branch_status = Some(current_branch_status(&repo.path).expect("read branch status"));
      this.has_head_commit = true;
      this.can_undo_last_commit = true;
      this.can_push = true;
      this.can_force_push = true;
      this.force_push_after_rebase = true;
      this.has_staged_changes = true;
      this.selected_file = Some(rel_path.to_path_buf());
    });

    std::fs::remove_dir_all(&repo.path).expect("remove repo root");

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (
      status_entries_len,
      branch_status,
      has_head_commit,
      can_undo_last_commit,
      can_push,
      can_force_push,
      force_push_after_rebase,
      has_staged_changes,
      selected_file,
      selected_branch,
    ) = git_page.read_with(cx, |this, _cx| {
      (
        this.status_entries.len(),
        this.branch_status.clone(),
        this.has_head_commit,
        this.can_undo_last_commit,
        this.can_push,
        this.can_force_push,
        this.force_push_after_rebase,
        this.has_staged_changes,
        this.selected_file.clone(),
        selected_branch_from_dropdown(this),
      )
    });

    assert_eq!(status_entries_len, 0);
    assert!(branch_status.is_none());
    assert!(!has_head_commit);
    assert!(!can_undo_last_commit);
    assert!(!can_push);
    assert!(!can_force_push);
    assert!(!force_push_after_rebase);
    assert!(!has_staged_changes);
    assert!(selected_file.is_none());
    assert!(selected_branch.is_none());
  }

  #[test]
  fn should_apply_branch_refresh_rejects_stale_generation_or_repo_mismatch() {
    let repo = Path::new("/tmp/repo");
    let other_repo = Path::new("/tmp/other");

    assert!(GitPage::should_apply_branch_refresh(Some(repo), repo, 3, 3));
    assert!(!GitPage::should_apply_branch_refresh(
      Some(repo),
      repo,
      4,
      3
    ));
    assert!(!GitPage::should_apply_branch_refresh(
      Some(other_repo),
      repo,
      3,
      3
    ));
    assert!(!GitPage::should_apply_branch_refresh(None, repo, 3, 3));
  }

  #[test]
  fn should_apply_status_refresh_rejects_stale_generation_or_repo_mismatch() {
    let repo = Path::new("/tmp/repo");
    let other_repo = Path::new("/tmp/other");

    assert!(GitPage::should_apply_status_refresh(Some(repo), repo, 3, 3));
    assert!(!GitPage::should_apply_status_refresh(
      Some(repo),
      repo,
      4,
      3
    ));
    assert!(!GitPage::should_apply_status_refresh(
      Some(other_repo),
      repo,
      3,
      3
    ));
    assert!(!GitPage::should_apply_status_refresh(None, repo, 3, 3));
  }

  #[test]
  fn status_poll_interval_slows_down_for_inactive_window() {
    assert_eq!(
      GitPage::status_poll_interval(true),
      Duration::from_millis(STATUS_POLL_INTERVAL_MS)
    );
    assert_eq!(
      GitPage::status_poll_interval(false),
      INACTIVE_STATUS_POLL_INTERVAL
    );
  }

  #[test]
  fn should_poll_status_requires_active_window_repo_and_idle_refresh() {
    let repo = Path::new("/tmp/repo");

    assert!(GitPage::should_poll_status(true, Some(repo), false));
    assert!(!GitPage::should_poll_status(false, Some(repo), false));
    assert!(!GitPage::should_poll_status(true, None, false));
    assert!(!GitPage::should_poll_status(true, Some(repo), true));
  }

  #[test]
  fn unpublished_branch_check_derives_tracked_branch_from_ahead_count() {
    let repo = Path::new("/tmp/repo");
    let branch = make_branch_status("main", 2, 0, true);
    let (has_unpublished_commits, check_key, checked) =
      GitPage::resolve_polled_unpublished_branch_commits(repo, Some(&branch), None, false, false)
        .expect("tracked branch should not need the expensive unpublished check");

    assert!(has_unpublished_commits);
    assert!(checked);
    assert_eq!(
      check_key,
      Some(UnpublishedBranchCheckKey {
        repo_root: repo.to_path_buf(),
        branch_name: "main".to_string(),
        ahead: 2,
        behind: 0,
        has_upstream: true,
        head_sha: None,
      })
    );
  }

  #[test]
  fn unpublished_branch_check_reuses_cached_local_branch_key_until_forced_or_changed() {
    let repo = Path::new("/tmp/repo");
    let branch = make_branch_status("feature", 0, 0, false);
    let cached_key =
      GitPage::unpublished_branch_check_key(repo, &branch, Some("head-a".to_string()));
    let same_key = GitPage::unpublished_branch_check_key(repo, &branch, Some("head-a".to_string()));
    let changed_head_key =
      GitPage::unpublished_branch_check_key(repo, &branch, Some("head-b".to_string()));
    let changed_repo_key = GitPage::unpublished_branch_check_key(
      Path::new("/tmp/other-repo"),
      &branch,
      Some("head-a".to_string()),
    );

    assert!(!GitPage::should_recheck_unpublished_branch(
      &same_key,
      Some(&cached_key),
      false
    ));
    assert!(GitPage::should_recheck_unpublished_branch(
      &same_key,
      Some(&cached_key),
      true
    ));
    assert!(GitPage::should_recheck_unpublished_branch(
      &changed_head_key,
      Some(&cached_key),
      false
    ));
    assert!(GitPage::should_recheck_unpublished_branch(
      &changed_repo_key,
      Some(&cached_key),
      false
    ));

    let cached_headless_key = GitPage::unpublished_branch_check_key(repo, &branch, None);
    let (has_unpublished_commits, returned_key, checked) =
      GitPage::resolve_polled_unpublished_branch_commits(
        repo,
        Some(&branch),
        Some(&cached_headless_key),
        true,
        false,
      )
      .expect("unchanged key should reuse cached unpublished state");
    assert!(has_unpublished_commits);
    assert_eq!(returned_key, Some(cached_headless_key));
    assert!(!checked);
  }

  #[test]
  fn branch_refresh_guard_ignores_stale_result_after_repo_switch() {
    let repo_a = TempRepo::init("git-page-refresh-stale-a");
    commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    create_branch(&repo_a.path, "alpha").expect("create alpha branch");
    Repository::open(&repo_a.path)
      .expect("open repo a")
      .set_head("refs/heads/alpha")
      .expect("set HEAD to alpha");

    let repo_b = TempRepo::init("git-page-refresh-stale-b");
    commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    let repo_a_status = current_branch_status(&repo_a.path).expect("read repo a status");
    let repo_a_items = GitPage::branch_select_items(
      list_branches(&repo_a.path).expect("list repo a branches"),
      GitPage::selected_branch_from_status(Some(&repo_a_status)).as_ref(),
      None,
    );
    assert!(
      repo_a_items
        .iter()
        .any(|item| item.branch.name == "alpha" && item.is_current)
    );

    // Simulate two in-flight refreshes:
    // 1) old refresh requested for repo A at generation 1
    // 2) user switches repo, new refresh requested for repo B at generation 2
    let stale_request_generation = 1;
    let active_generation = 2;
    assert!(!GitPage::should_apply_branch_refresh(
      Some(repo_b.path.as_path()),
      repo_a.path.as_path(),
      active_generation,
      stale_request_generation
    ));
    assert!(GitPage::should_apply_branch_refresh(
      Some(repo_b.path.as_path()),
      repo_b.path.as_path(),
      active_generation,
      active_generation
    ));

    let repo_b_status = current_branch_status(&repo_b.path).expect("read repo b status");
    let repo_b_items = GitPage::branch_select_items(
      list_branches(&repo_b.path).expect("list repo b branches"),
      GitPage::selected_branch_from_status(Some(&repo_b_status)).as_ref(),
      None,
    );
    assert_eq!(
      repo_b_items.iter().filter(|item| item.is_current).count(),
      1
    );
    assert!(
      repo_b_items
        .iter()
        .any(|item| item.branch.name == repo_b_status.name && item.is_current)
    );
    assert!(
      !repo_b_items
        .iter()
        .any(|item| item.branch.name == "alpha" && item.is_current)
    );
  }

  #[test]
  fn should_refresh_editor_for_path_only_when_selected_matches() {
    let selected = Path::new("src/main.rs");
    let other = Path::new("src/lib.rs");

    assert!(GitPage::should_refresh_editor_for_path(
      Some(selected),
      selected
    ));
    assert!(!GitPage::should_refresh_editor_for_path(
      Some(selected),
      other
    ));
    assert!(!GitPage::should_refresh_editor_for_path(None, selected));
  }
}
