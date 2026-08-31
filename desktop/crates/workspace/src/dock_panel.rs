//! The right dock of the shell: changes, files, history, pull request, terminal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use editor::ReviewCommentCreateRequest;

use git::{
  HeadCommitStatus, RepoStage, RepoStatusEntry, commit_changes, current_branch_status,
  current_github_remote_repo, head_commit_status, is_merge_in_progress, is_rebase_in_progress,
  list_repo_status, list_repo_worktree_files, stage_all,
};
use gpui::{
  Anchor, AnyElement, AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, Render,
  SharedString, Task, WeakEntity, Window, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _, h_flex,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  menu::{DropdownMenu as _, PopupMenuItem},
  tree::{TreeEvent, TreeState, tree},
  v_flex,
};
use terminal::TerminalView;

use crate::changes_list::{ChangesList, ChangesListEvent, status_color};
use crate::file_tree::{
  build_path_tree_items_with_expansion, expanded_folder_paths_for_changed_files,
};
use crate::file_view::{file_dir_label, file_name_label, render_file_name_with_status};
use crate::history_list::{HistoryList, HistoryListEvent, history_change_kind_to_repo_status};
use crate::pro_promise::{ProPromiseSurface, render_pro_promise};
use crate::pull_request_refresh::{
  PullRequestRefresh, branch_switched_since_lookup, should_read_pull_request,
};
use crate::pull_request_review_comments::{
  FileCommentCounts, ReviewCommentWrite, comment_line, file_comment_counts,
  pending_review_comment_node_id, pending_review_id, pending_review_rows,
  review_comment_write_plan,
};
use crate::pull_request_review_submission::review_submission_target;
use crate::repo_state::{PaletteCommand, RepoState, push_flags, should_publish_branch};
use crate::review_list::{ReviewList, ReviewListEvent, ReviewSection};
use crate::review_submit_dialog::open_submit_review_dialog;

const DOCK_PANEL_TERMINAL_DEBUG_SELECTOR: &str = "dock-panel-terminal";
pub(crate) const DOCK_PANEL_HISTORY_DEBUG_SELECTOR: &str = "dock-panel-history";
pub(crate) const DOCK_PANEL_PR_CHECKS_DEBUG_SELECTOR: &str = "dock-panel-pr-checks";
pub(crate) const DOCK_PANEL_PR_MERGE_DEBUG_SELECTOR: &str = "dock-panel-pr-merge";
pub(crate) const DOCK_PANEL_PR_MERGE_METHOD_DEBUG_SELECTOR: &str = "dock-panel-pr-merge-method";
pub(crate) const DOCK_PANEL_PR_CHECKS_COUNTS_DEBUG_SELECTOR: &str = "dock-panel-pr-checks-counts";
pub(crate) const DOCK_PANEL_PR_REVIEW_DEBUG_SELECTOR: &str = "dock-panel-pr-review";
pub(crate) const DOCK_PANEL_PR_PENDING_COMMENTS_DEBUG_SELECTOR: &str =
  "dock-panel-pr-pending-comments";
pub(crate) const DOCK_PANEL_REVIEW_DEBUG_SELECTOR: &str = "dock-panel-review";
const DOCK_PANEL_COMMIT_DEBUG_SELECTOR: &str = "dock-panel-commit";
const DOCK_PANEL_COMMIT_MENU_DEBUG_SELECTOR: &str = "dock-panel-commit-menu";
const DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR: &str = "dock-panel-create-pr";
const DOCK_PANEL_PUBLISH_AND_CREATE_PR_DEBUG_SELECTOR: &str = "dock-panel-publish-and-create-pr";
const DOCK_PANEL_COMPARE_DEBUG_SELECTOR: &str = "dock-panel-compare-on-github";
const DOCK_PANEL_OPERATION_DEBUG_SELECTOR: &str = "dock-panel-operation";
const DOCK_PANEL_REFRESH_DEBUG_SELECTOR: &str = "dock-panel-refresh";
pub(crate) const DOCK_PANEL_ZOOM_DEBUG_SELECTOR: &str = "dock-panel-zoom";
pub(crate) const DOCK_PANEL_CHECKOUT_SELECTOR_DEBUG_SELECTOR: &str = "dock-panel-checkout-selector";
pub(crate) const DOCK_PANEL_CHECKOUT_FOLLOW_DEBUG_SELECTOR: &str = "dock-panel-checkout-follow";
use std::rc::Rc;

use crate::api::{
  GithubPullRequest, GithubPullRequestChecksRollupState, GithubPullRequestChecksSummary,
  GithubPullRequestMergeMethod, GithubPullRequestMergeReadiness, GithubPullRequestReview,
  GithubPullRequestReviewComment, GithubPullRequestReviewEvent,
};
use crate::auth_state::AuthStateStore;
use crate::github_navigation::{github_pull_request_url, open_compare_target};
use crate::github_shared::{pull_request_status_color, pull_request_status_label};
use crate::merge_dialog::{MergeConfirmedHandler, open_merge_dialog};
use crate::open_intent::OpenIntent;
use crate::pull_request_checks::{
  CheckRow, check_rows, check_state_sort_key, checks_state_counts, checks_summary_title,
  singular_or_plural,
};
use crate::pull_request_dialog::{
  GithubBranchContext, PullRequestCreatedHandler, open_create_pull_request_dialog,
};
use crate::pull_request_merge::{
  MergeAvailability, MergeRequest, merge_availability, merge_commit_defaults, merge_method_label,
};
use crate::pull_request_reviewers::{
  ReviewerRow, ReviewerStatus, reviewer_rows, reviewers_summary_title, submitted_review_status,
};
use crate::workspace::WorkspaceApi;
use gpui_component::avatar::Avatar;
use gpui_component::notification::Notification;
use gpui_component::scroll::ScrollableElement as _;
use ui::{
  Button, ButtonVariants as _, CommandPaletteCommand, ConfirmDialog, SelectableRowStyle,
  StatusThemeExt as _, Textarea, TextareaState, UiIconName, WindowExt as _, selectable_list_item,
};

#[derive(Clone, Debug)]
pub enum DockPanelEvent {
  OpenFile {
    path: PathBuf,
    intent: OpenIntent,
  },
  /// A file as it was in a commit, read-only.
  OpenCommitFile {
    commit_oid: String,
    path: PathBuf,
    intent: OpenIntent,
  },
  /// A commit landed: whoever shows the branch state has to refresh it.
  Committed,
  /// The rebase can move on: the host runs it, it owns the conflict flow.
  ContinueRebase,
  /// A command picked in the commit menu; the host owns running it.
  RunCommand(CommitMenuCommand),
  /// An unpublished branch: push it, then open the pull request form.
  PublishBranchAndCreatePullRequest(GithubBranchContext),
  /// The working tree was re-read: whoever shows a file has to look again.
  StatusRefreshed,
  /// The zoom button: the host owns the layout.
  ToggleZoom,
  /// A row of the review panel: the host owns the diff and the batch.
  OpenReviewComment {
    path: PathBuf,
    line: usize,
    intent: OpenIntent,
  },
  DeleteReviewComment {
    id: u64,
  },
  /// A comment already on GitHub cannot be taken back, so the host asks before
  /// this one goes. The same question the diff asks.
  DeletePullRequestReviewComment {
    id: u64,
  },
  SendReviewComment {
    id: u64,
  },
  /// A row of the pull request file list: the host owns the centre pane.
  OpenPullRequestFile {
    base_oid: String,
    head_oid: String,
    path: PathBuf,
    line: Option<usize>,
    intent: OpenIntent,
  },
  SendReview,
  DiscardReview,
  /// Finishing a pull request review needs a window the panel does not have
  /// where the request comes from.
  SubmitPullRequestReview,
  /// Discarding the review pending on GitHub asks the same window first.
  DiscardPullRequestReview,
  /// What GitHub holds on this pull request changed: whoever shows the comments
  /// has to look again.
  PullRequestReviewCommentsChanged,
  /// A checkout picked in the header selector; the host owns the pin.
  PinCheckout {
    path: PathBuf,
  },
  /// The pinned dock goes back to following the session's checkout.
  FollowSessionCheckout,
  /// A comment the diff is waiting on: the composer stays open on an error.
  PullRequestReviewCommentSubmitted {
    error: Option<Arc<str>>,
  },
}

impl gpui::EventEmitter<DockPanelEvent> for DockPanel {}

/// What the menu next to the commit button offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitMenuCommand {
  Amend,
  UndoLastCommit,
  Push,
  ForcePush,
}

impl CommitMenuCommand {
  fn label(self) -> &'static str {
    match self {
      Self::Amend => "Amend",
      Self::UndoLastCommit => "Undo last commit",
      Self::Push => "Push",
      Self::ForcePush => "Force push (with lease)",
    }
  }

  fn icon(self) -> IconName {
    match self {
      Self::Amend => IconName::Replace,
      Self::UndoLastCommit => IconName::Undo,
      Self::Push | Self::ForcePush => IconName::ArrowUp,
    }
  }

  fn rule(self) -> PaletteCommand {
    match self {
      Self::Amend => PaletteCommand::Amend,
      Self::UndoLastCommit => PaletteCommand::UndoLastCommit,
      Self::Push => PaletteCommand::Push,
      Self::ForcePush => PaletteCommand::ForcePush,
    }
  }
}

/// The files a pull request proposes, as rows the keyboard can walk.
struct PrFilesDelegate {
  panel: WeakEntity<DockPanel>,
  files: Vec<git::CommitChangedFile>,
  comment_counts: HashMap<PathBuf, FileCommentCounts>,
  selected_index: Option<IndexPath>,
}

impl PrFilesDelegate {
  fn new(panel: WeakEntity<DockPanel>) -> Self {
    Self {
      panel,
      files: Vec::new(),
      comment_counts: HashMap::new(),
      selected_index: None,
    }
  }

  fn file_at(&self, ix: IndexPath) -> Option<&git::CommitChangedFile> {
    self.files.get(ix.row)
  }
}

impl ListDelegate for PrFilesDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.files.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = cx.theme().clone();
    let file = self.file_at(ix)?;
    let path = file.path.clone();
    let status = history_change_kind_to_repo_status(file.kind);
    let old_path = file.old_path.clone();
    let comments = self.comment_counts.get(&path).copied();
    let selected = self
      .selected_index
      .map(|selected| selected.eq_row(ix))
      .unwrap_or(false);

    Some(
      selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
        .w_full()
        .px_2()
        .py_1()
        .debug_selector({
          let path = path.clone();
          move || format!("pr-file-{}", path.to_string_lossy())
        })
        .child(
          h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .child(render_file_name_with_status(
              &theme,
              Some(status),
              file_name_label(&path),
              old_path.as_deref().map(file_name_label),
            ))
            .child(
              div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .truncate()
                .child(file_dir_label(&path)),
            )
            .when_some(comments, |this, comments| {
              // Unsent comments call for the eye; published ones just say the
              // file has words on it.
              let color = if comments.pending > 0 {
                theme.primary
              } else {
                theme.muted_foreground
              };
              this.child(
                h_flex()
                  .flex_shrink_0()
                  .items_center()
                  .gap_0p5()
                  .text_xs()
                  .text_color(color)
                  .child(
                    Icon::new(UiIconName::MessageCircle)
                      .size_3()
                      .text_color(color),
                  )
                  .child(comments.total.to_string()),
              )
            })
            .child(
              div()
                .flex_shrink_0()
                .text_xs()
                .text_color(status_color(status, &theme))
                .child(status.short_code()),
            ),
        ),
    )
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    self.selected_index = ix;
    if let Some(path) = ix
      .and_then(|ix| self.file_at(ix))
      .map(|file| file.path.clone())
    {
      let _ = self
        .panel
        .update(cx, |panel, _| panel.pr_selected_file = Some(path));
    }
    cx.notify();
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockPanelTab {
  Changes,
  Review,
  Files,
  History,
  PullRequest,
  Terminal,
}

/// Exhaustive on the rollup state, so a new one has to pick its own colour
/// instead of silently reading as skipped.
fn check_state_icon(
  state: GithubPullRequestChecksRollupState,
  theme: &gpui_component::Theme,
) -> Icon {
  match state {
    GithubPullRequestChecksRollupState::Success => {
      Icon::new(UiIconName::CircleCheck).text_color(theme.status_green())
    }
    GithubPullRequestChecksRollupState::Failure => {
      Icon::new(IconName::CircleX).text_color(theme.status_red())
    }
    GithubPullRequestChecksRollupState::Pending => {
      Icon::new(UiIconName::CircleDot).text_color(theme.status_orange())
    }
    GithubPullRequestChecksRollupState::Skipped => {
      Icon::new(UiIconName::CircleSlash).text_color(theme.muted_foreground)
    }
  }
}

fn reviewer_status_icon(status: ReviewerStatus, theme: &gpui_component::Theme) -> Icon {
  match status {
    ReviewerStatus::Approved => Icon::new(UiIconName::CircleCheck).text_color(theme.status_green()),
    ReviewerStatus::ChangesRequested => Icon::new(IconName::CircleX).text_color(theme.status_red()),
    ReviewerStatus::Commented => {
      Icon::new(UiIconName::MessageCircle).text_color(theme.muted_foreground)
    }
    ReviewerStatus::Awaiting => Icon::new(UiIconName::CircleDot).text_color(theme.muted_foreground),
  }
}

/// Enough of the answer to read at a glance while the block is closed.
fn render_reviewer_avatars(reviewers: &[ReviewerRow], theme: &gpui_component::Theme) -> AnyElement {
  const SHOWN: usize = 3;
  let mut row = h_flex().flex_shrink_0().items_center().gap_1();
  for reviewer in reviewers.iter().take(SHOWN) {
    row = row.child(
      Avatar::new()
        .name(reviewer.login.clone())
        .when_some(reviewer.avatar_url.clone(), |this, url| this.src(url))
        .xsmall(),
    );
  }
  if reviewers.len() > SHOWN {
    row = row.child(
      div()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(format!("+{}", reviewers.len() - SHOWN)),
    );
  }
  row.into_any_element()
}

fn render_reviewer_row(reviewer: &ReviewerRow, theme: &gpui_component::Theme) -> AnyElement {
  h_flex()
    .id(SharedString::from(format!(
      "pr-reviewer-{}",
      reviewer.login
    )))
    .when_some(reviewer.latest_message.clone(), |this, message| {
      this.tooltip(move |window, cx| {
        gpui_component::tooltip::Tooltip::new(message.clone()).build(window, cx)
      })
    })
    .w_full()
    .items_center()
    .gap_2()
    .px_2()
    .py_1()
    .child(reviewer_status_icon(reviewer.status, theme).size_3())
    .child(
      Avatar::new()
        .name(reviewer.login.clone())
        .when_some(reviewer.avatar_url.clone(), |this, url| this.src(url))
        .xsmall(),
    )
    .child(
      div()
        .flex_1()
        .min_w_0()
        .text_xs()
        .text_color(theme.foreground)
        .truncate()
        .child(reviewer.login.clone()),
    )
    .child(
      div()
        .flex_shrink_0()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(reviewer.status.label()),
    )
    .into_any_element()
}

/// One check: its state, its name, and how long it took. Clicking opens it on
/// GitHub, which is where a failing run is actually read.
fn render_check_row(
  row: &CheckRow,
  theme: &gpui_component::Theme,
  _cx: &mut Context<DockPanel>,
) -> AnyElement {
  let open_url = row.open_url.clone();
  let clickable = open_url.is_some();

  h_flex()
    .id(gpui::SharedString::from(format!("pr-check-{}", row.id)))
    .w_full()
    .items_center()
    .gap_2()
    .px_2()
    .py_1()
    .rounded_sm()
    .when(clickable, |this| {
      this
        .hover(|this| this.bg(theme.accent))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
          if let Some(url) = open_url.clone() {
            cx.open_url(&url);
          }
        })
    })
    .child(check_state_icon(row.state, theme).size_3())
    .child(
      v_flex()
        .flex_1()
        .min_w_0()
        .child(
          div()
            .text_xs()
            .text_color(theme.foreground)
            .truncate()
            .child(row.title.clone()),
        )
        .when_some(row.status_label.clone(), |this, label| {
          this.child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .truncate()
              .child(label),
          )
        }),
    )
    .into_any_element()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PullRequestRange {
  pub base: String,
  pub head: String,
  pub base_ref: String,
  pub head_ref: String,
}

struct PullRequestRangeLoad {
  range: PullRequestRange,
  files: Vec<git::CommitChangedFile>,
  conversation: Option<PullRequestConversationLoad>,
  author_login: String,
  pull_request_node_id: String,
}

struct PullRequestConversationLoad {
  reviewers: Vec<ReviewerRow>,
  review_comments: Vec<GithubPullRequestReviewComment>,
}

/// The commits of a pull request may not be in the local object database yet.
/// A fetch is the whole precondition: no checkout, and no file content over the
/// network.
fn list_pull_request_files(
  repo_root: &std::path::Path,
  range: &PullRequestRange,
) -> anyhow::Result<Vec<git::CommitChangedFile>> {
  match git::list_range_changed_files(repo_root, &range.base, &range.head) {
    Ok(files) => Ok(files),
    Err(_) => {
      git::fetch(repo_root)?;
      git::list_range_changed_files(repo_root, &range.base, &range.head)
    }
  }
}

#[derive(Clone, Debug)]
pub(crate) enum BranchPrState {
  NoAccess,
  NoRemote,
  Loading,
  Missing(GithubBranchContext),
  Found(GithubBranchContext, Box<GithubPullRequest>),
}

/// Which pull request a state is about, if any.
fn pull_request_identity(state: &BranchPrState) -> Option<(String, String, u64)> {
  match state {
    BranchPrState::Found(context, pull_request) => Some((
      context.owner.clone(),
      context.repo.clone(),
      pull_request.number,
    )),
    _ => None,
  }
}

#[cfg(any(test, feature = "test-support"))]
fn driver_dock_tab(tab: DockPanelTab) -> &'static str {
  match tab {
    DockPanelTab::Changes => "changes",
    DockPanelTab::Review => "review",
    DockPanelTab::Files => "files",
    DockPanelTab::History => "history",
    DockPanelTab::PullRequest => "pull_request",
    DockPanelTab::Terminal => "terminal",
  }
}

#[cfg(any(test, feature = "test-support"))]
fn driver_pr_file(file: &git::CommitChangedFile) -> serde_json::Value {
  serde_json::json!({
    "path": file.path.display().to_string(),
    "old_path": file.old_path.as_ref().map(|path| path.display().to_string()),
    "kind": driver_pr_file_kind(file.kind),
  })
}

#[cfg(any(test, feature = "test-support"))]
fn driver_pr_file_kind(kind: git::CommitFileChangeKind) -> &'static str {
  match kind {
    git::CommitFileChangeKind::Added => "added",
    git::CommitFileChangeKind::Deleted => "deleted",
    git::CommitFileChangeKind::Modified => "modified",
    git::CommitFileChangeKind::Renamed => "renamed",
    git::CommitFileChangeKind::Copied => "copied",
    git::CommitFileChangeKind::Typechange => "typechange",
    git::CommitFileChangeKind::Conflicted => "conflicted",
  }
}

#[cfg(any(test, feature = "test-support"))]
fn driver_pr_review_comment(comment: &GithubPullRequestReviewComment) -> serde_json::Value {
  serde_json::json!({
    "id": comment.id,
    "path": comment.path,
    "line": comment.line.or(comment.original_line),
    "user_login": comment.user.login,
    "body": comment.body,
    "is_pending": comment.is_pending,
    "is_outdated": comment.is_outdated,
  })
}

#[cfg(any(test, feature = "test-support"))]
fn driver_review_panel_comment(
  comment: &crate::review_list::ReviewPanelComment,
) -> serde_json::Value {
  serde_json::json!({
    "id": comment.id,
    "path": comment.path.display().to_string(),
    "line": comment.line,
    "line_label": comment.line_label,
    "excerpt": comment.excerpt,
    "status": driver_review_row_status(comment.status),
    "sendable": comment.sendable,
  })
}

#[cfg(any(test, feature = "test-support"))]
fn driver_review_row_status(status: crate::review_list::ReviewRowStatus) -> &'static str {
  match status {
    crate::review_list::ReviewRowStatus::Draft => "draft",
    crate::review_list::ReviewRowStatus::Sent => "sent",
    crate::review_list::ReviewRowStatus::Pending => "pending",
    crate::review_list::ReviewRowStatus::Outdated => "outdated",
  }
}

/// One checkout of the shown session's repo, ready for the header selector.
#[derive(Clone, PartialEq)]
pub(crate) struct CheckoutOption {
  pub path: PathBuf,
  /// None on a detached HEAD.
  pub branch: Option<SharedString>,
  pub is_displayed: bool,
}

fn branch_pr_state_for_lookup(
  remote: Option<git::GithubRemoteRepo>,
  branch: Option<String>,
  fetch: impl FnOnce(&GithubBranchContext) -> anyhow::Result<Option<GithubPullRequest>>,
) -> BranchPrState {
  let Some(remote) = remote else {
    return BranchPrState::NoRemote;
  };
  let Some(branch) = branch else {
    return BranchPrState::NoRemote;
  };
  let context = GithubBranchContext {
    owner: remote.owner,
    repo: remote.repo,
    branch,
  };
  match fetch(&context) {
    Ok(Some(pull_request)) => BranchPrState::Found(context, Box::new(pull_request)),
    Ok(None) => BranchPrState::Missing(context),
    // Keep the tab usable on transient API errors: offer Create against the context.
    Err(_) => BranchPrState::Missing(context),
  }
}

pub struct DockPanel {
  focus_handle: FocusHandle,
  window_handle: AnyWindowHandle,
  repo_root: Option<PathBuf>,
  /// The checkouts the header selector offers; the host computes them.
  checkout_options: Vec<CheckoutOption>,
  /// The dock shows a checkout the active session does not work in.
  checkout_pinned: bool,
  status_entries: Vec<RepoStatusEntry>,
  merge_in_progress: bool,
  rebase_in_progress: bool,
  head_status: HeadCommitStatus,
  branch_status: Option<git::BranchStatus>,
  pub(crate) commit_input: Entity<TextareaState>,
  committing: bool,
  last_error: Option<SharedString>,
  active_tab: DockPanelTab,
  zoomed: bool,
  changes_list: Entity<ChangesList>,
  pub(crate) review_list: Entity<ReviewList>,
  pub(crate) history_list: Entity<HistoryList>,
  /// Spawned on the first visit to the tab: a shell per session is too much
  /// for someone who never opens it.
  terminal_view: Option<Entity<TerminalView>>,
  branch_pr: BranchPrState,
  /// The shas and the file list of the pull request on the current branch. The
  /// identity arrives first, this follows.
  pr_range: Option<PullRequestRange>,
  pr_files: Vec<git::CommitChangedFile>,
  pr_files_list: Entity<ListState<PrFilesDelegate>>,
  /// The tab was asked for before its files were there: focus them once they are.
  focus_pr_files_when_loaded: bool,
  /// The shell is mounted by the render that follows: it cannot take the focus
  /// before it exists.
  focus_terminal_when_rendered: bool,
  pr_files_loading: bool,
  pr_files_error: Option<SharedString>,
  /// When GitHub was last read for this branch's pull request.
  pr_fetched_at: Option<Instant>,
  /// The branch the lookup above answered for. A cache keyed on time alone
  /// survives a checkout, and the panel then shows the old branch's pull request.
  pr_branch: Option<String>,
  pr_selected_file: Option<PathBuf>,
  pr_checks: Option<GithubPullRequestChecksSummary>,
  /// What the viewer wrote on this pull request and has not submitted yet.
  /// GitHub owns them, so they are read back rather than stored here.
  pr_review_comments: Vec<GithubPullRequestReviewComment>,
  /// A link asked for a review comment the panel has not read yet; the load that
  /// follows is what takes the diff to it.
  awaited_review_comment: Option<u64>,
  /// Who opened it: an author reviews their own work with words only.
  pr_author_login: Option<String>,
  /// GraphQL names the pull request by node id, and starting a review needs it.
  pr_node_id: Option<String>,
  pr_reviewers: Vec<ReviewerRow>,
  /// The review the viewer just submitted: GitHub's reads can lag its writes,
  /// so this outranks a fetched Awaiting until the fetches catch up.
  submitted_review_overlay: Option<GithubPullRequestReview>,
  pr_checks_loading: bool,
  pr_merge_readiness: Option<GithubPullRequestMergeReadiness>,
  /// The viewer's remembered method for this repository, kept only while the
  /// repository allows it.
  selected_merge_method: Option<GithubPullRequestMergeMethod>,
  pr_checks_scroll: gpui::ScrollHandle,
  /// An asked-for refresh spins the button until everything it triggered has
  /// landed: the lookup, then the range and checks reads it fans out to.
  /// Rereads are deliberately silent otherwise, so the click needs its answer.
  pr_refresh_pending: u8,
  pr_merging: bool,
  _pr_merge_task: Option<Task<()>>,
  /// Collapsed by default: the file list is what you work in.
  pr_details_expanded: bool,
  _pr_range_task: Option<Task<()>>,
  _pr_checks_task: Option<Task<()>>,
  _pr_review_comments_task: Option<Task<()>>,
  files_tree_state: Entity<TreeState>,
  files_loaded: bool,
  /// The tab was opened before its tree existed: focus it as soon as it does.
  focus_files_tree_when_loaded: bool,
  files_loading: bool,
  pub(crate) _refresh_task: Option<Task<()>>,
  _commit_task: Option<Task<()>>,
  _pr_task: Option<Task<()>>,
  _files_task: Option<Task<()>>,
}

impl DockPanel {
  pub fn new(repo_root: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let commit_input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });
    let _ = commit_input
      .read(cx)
      .focus_handle(cx)
      .tab_stop(true)
      .tab_index(1);
    // cmd-enter from inside the input commits.
    cx.subscribe_in(
      &commit_input,
      window,
      |this, _state, event: &gpui_component::input::InputEvent, _window, cx| {
        if let gpui_component::input::InputEvent::PressEnter {
          secondary: true, ..
        } = event
        {
          this.commit(cx);
        }
      },
    )
    .detach();

    let split_sections = !crate::config::AppSettings::get(cx).git_unified_file_view;
    let changes_list = cx.new(|cx| ChangesList::new(repo_root.clone(), split_sections, window, cx));
    cx.subscribe_in(
      &changes_list,
      window,
      |this, _list, event: &ChangesListEvent, _window, cx| match event {
        ChangesListEvent::OpenFile { path, intent } => {
          cx.emit(DockPanelEvent::OpenFile {
            path: path.clone(),
            intent: *intent,
          });
        }
        ChangesListEvent::Changed => this.refresh(cx),
      },
    )
    .detach();

    // The unified/split file list is a setting: follow it without a restart.
    cx.observe_global::<crate::config::AppSettings>(|this, cx| {
      let split_sections = !crate::config::AppSettings::get(cx).git_unified_file_view;
      this
        .changes_list
        .update(cx, |list, cx| list.set_split_sections(split_sections, cx));
    })
    .detach();

    let panel = cx.entity().downgrade();
    let pr_files_list =
      cx.new(|cx| ListState::new(PrFilesDelegate::new(panel), window, cx).reset_on_cancel(false));
    let _ = pr_files_list
      .read(cx)
      .focus_handle(cx)
      .tab_stop(true)
      .tab_index(0);
    cx.subscribe(&pr_files_list, |this, state, event: &ListEvent, cx| {
      let (ix, intent) = match event {
        ListEvent::Select(ix) => (*ix, OpenIntent::Browse),
        ListEvent::Confirm(ix) => (*ix, OpenIntent::Open),
        _ => return,
      };
      let Some(range) = this.pr_range.clone() else {
        return;
      };
      let Some(path) = state
        .read(cx)
        .delegate()
        .file_at(ix)
        .map(|file| file.path.clone())
      else {
        return;
      };
      cx.emit(DockPanelEvent::OpenPullRequestFile {
        base_oid: range.base,
        head_oid: range.head,
        path,
        line: None,
        intent,
      });
    })
    .detach();

    // The worktree tree says where the user is and what they chose: a file row
    // shows as they walk it, and opens when they pick it.
    let files_tree_state = cx.new(|cx| TreeState::new(cx));
    let _ = files_tree_state
      .read(cx)
      .focus_handle(cx)
      .tab_stop(true)
      .tab_index(0);
    cx.subscribe(&files_tree_state, |_this, tree, event: &TreeEvent, cx| {
      let (id, intent) = match event {
        TreeEvent::Selected(id) => (id.clone(), OpenIntent::Browse),
        TreeEvent::Confirmed(id) => (id.clone(), OpenIntent::Open),
        TreeEvent::Expanded(_) | TreeEvent::Collapsed(_) => return,
      };
      // Walking onto a folder moves the selection and nothing else: a folder
      // has no contents to show.
      let state = tree.read(cx);
      let is_folder = state
        .index_of(&id)
        .and_then(|ix| state.entry(ix))
        .is_some_and(|entry| entry.is_folder());
      if is_folder {
        return;
      }
      cx.emit(DockPanelEvent::OpenFile {
        path: PathBuf::from(id.as_ref()),
        intent,
      });
    })
    .detach();

    let review_list = cx.new(|cx| ReviewList::new(window, cx));
    cx.subscribe(
      &review_list,
      |this, _list, event: &ReviewListEvent, cx| match event {
        ReviewListEvent::OpenComment {
          section,
          path,
          line,
          intent,
        } => match section {
          ReviewSection::Agent => cx.emit(DockPanelEvent::OpenReviewComment {
            path: path.clone(),
            line: *line,
            intent: *intent,
          }),
          // A pull request comment is about the range, not about the working
          // tree, which may hold something else entirely on that line.
          ReviewSection::PullRequest => {
            if let Some(range) = this.pr_range.as_ref() {
              cx.emit(DockPanelEvent::OpenPullRequestFile {
                base_oid: range.base.clone(),
                head_oid: range.head.clone(),
                path: path.clone(),
                line: Some(*line),
                intent: *intent,
              });
            }
          }
        },
        ReviewListEvent::DeleteComment { section, id } => match section {
          ReviewSection::Agent => cx.emit(DockPanelEvent::DeleteReviewComment { id: *id }),
          ReviewSection::PullRequest => {
            cx.emit(DockPanelEvent::DeletePullRequestReviewComment { id: *id })
          }
        },
        ReviewListEvent::SendComment { id } => {
          cx.emit(DockPanelEvent::SendReviewComment { id: *id });
        }
        ReviewListEvent::SubmitReview => cx.emit(DockPanelEvent::SubmitPullRequestReview),
        ReviewListEvent::DiscardPullRequestReview => {
          cx.emit(DockPanelEvent::DiscardPullRequestReview)
        }
        ReviewListEvent::SendReview => cx.emit(DockPanelEvent::SendReview),
        ReviewListEvent::DiscardReview => cx.emit(DockPanelEvent::DiscardReview),
      },
    )
    .detach();

    let history_list = cx.new(HistoryList::new);
    cx.subscribe(
      &history_list,
      |_this, _list, event: &HistoryListEvent, cx| match event {
        HistoryListEvent::OpenCommitFile {
          commit_oid,
          path,
          intent,
        } => {
          cx.emit(DockPanelEvent::OpenCommitFile {
            commit_oid: commit_oid.clone(),
            path: path.clone(),
            intent: *intent,
          });
        }
      },
    )
    .detach();

    let mut panel = Self {
      review_list,
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      repo_root,
      checkout_options: Vec::new(),
      checkout_pinned: false,
      status_entries: Vec::new(),
      merge_in_progress: false,
      rebase_in_progress: false,
      head_status: HeadCommitStatus::default(),
      branch_status: None,
      commit_input,
      committing: false,
      last_error: None,
      active_tab: DockPanelTab::Changes,
      zoomed: false,
      changes_list,
      history_list,
      terminal_view: None,
      branch_pr: BranchPrState::Loading,
      pr_range: None,
      pr_files: Vec::new(),
      pr_files_list,
      focus_pr_files_when_loaded: false,
      focus_terminal_when_rendered: false,
      pr_files_loading: false,
      pr_files_error: None,
      pr_fetched_at: None,
      pr_branch: None,
      pr_selected_file: None,
      pr_checks: None,
      pr_review_comments: Vec::new(),
      awaited_review_comment: None,
      pr_author_login: None,
      pr_node_id: None,
      pr_reviewers: Vec::new(),
      submitted_review_overlay: None,
      pr_checks_loading: false,
      pr_merge_readiness: None,
      selected_merge_method: None,
      pr_checks_scroll: gpui::ScrollHandle::new(),
      pr_refresh_pending: 0,
      pr_merging: false,
      _pr_merge_task: None,
      pr_details_expanded: false,
      _pr_range_task: None,
      _pr_checks_task: None,
      _pr_review_comments_task: None,
      files_tree_state,
      files_loaded: false,
      focus_files_tree_when_loaded: false,
      files_loading: false,
      _refresh_task: None,
      _commit_task: None,
      _pr_task: None,
      _files_task: None,
    };
    panel.refresh(cx);
    panel
  }

  fn load_worktree_files(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.repo_root.clone() else {
      return;
    };
    if self.files_loading {
      return;
    }
    self.files_loading = true;

    let task = cx.spawn(async move |this, cx| {
      let files = cx
        .background_spawn(async move { list_repo_worktree_files(&repo_root) })
        .await;
      let _ = this.update(cx, |this, cx| {
        this.files_loading = false;
        if let Ok(files) = files {
          let paths = files
            .iter()
            .map(|path| std::rc::Rc::new(path.to_string_lossy().into_owned()))
            .collect::<Vec<_>>();
          // Only the branches holding uncommitted work open by themselves: a
          // whole repository expanded is a wall of folders.
          let expanded = expanded_folder_paths_for_changed_files(
            this
              .status_entries
              .iter()
              .filter_map(|entry| entry.path.to_str()),
          );
          let (items, _, _, _) =
            build_path_tree_items_with_expansion(&paths, |path| path.as_str(), Some(&expanded));
          this.files_tree_state.update(cx, |state, cx| {
            state.set_items(items, cx);
          });
          this.files_loaded = true;
        }
        cx.notify();
      });
    });
    self._files_task = Some(task);
  }

  pub fn refresh(&mut self, cx: &mut Context<Self>) {
    self.refresh_all(PullRequestRefresh::IfStale, cx);
  }

  /// The refresh button: being asked is reason enough to read GitHub again.
  pub(crate) fn refresh_requested(&mut self, cx: &mut Context<Self>) {
    self.pr_refresh_pending = 1;
    self.refresh_all(PullRequestRefresh::Now, cx);
    cx.notify();
  }

  fn refresh_all(&mut self, refresh: PullRequestRefresh, cx: &mut Context<Self>) {
    self.refresh_status(cx);
    self.refresh_branch_pull_request(refresh, cx);
    if self.files_loaded {
      self.load_worktree_files(cx);
    }
  }

  /// The working tree alone: what a poll can afford to re-read, with no request
  /// to GitHub and no reload of the file tree.
  pub(crate) fn refresh_status(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.repo_root.clone() else {
      self.status_entries.clear();
      cx.notify();
      return;
    };

    let task = cx.spawn(async move |this, cx| {
      let (result, merge_in_progress, rebase_in_progress, head_status) = cx
        .background_spawn(async move {
          (
            list_repo_status(&repo_root),
            is_merge_in_progress(&repo_root).unwrap_or(false),
            is_rebase_in_progress(&repo_root).unwrap_or(false),
            head_commit_status(&repo_root).unwrap_or_default(),
          )
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        this.merge_in_progress = merge_in_progress;
        this.rebase_in_progress = rebase_in_progress;
        this.head_status = head_status;
        match result {
          Ok(entries) => {
            this.changes_list.update(cx, |list, cx| {
              list.set_entries(entries.clone(), cx);
            });
            this.status_entries = entries;
            this.last_error = None;
            cx.emit(DockPanelEvent::StatusRefreshed);
          }
          Err(error) => this.last_error = Some(format!("{error}").into()),
        }
        cx.notify();
      });
    });
    self._refresh_task = Some(task);
  }

  /// One poll tick: the working tree, plus the history when its tab is open and
  /// the repository actually moved.
  pub(crate) fn poll(&mut self, cx: &mut Context<Self>) {
    self.refresh_status(cx);
    if self.active_tab == DockPanelTab::History {
      self.history_list.update(cx, |list, cx| {
        list.refresh_if_repository_moved(cx);
      });
    }
  }

  pub(crate) fn refresh_branch_pull_request_state(&mut self, cx: &mut Context<Self>) {
    self.refresh_branch_pull_request(PullRequestRefresh::Now, cx);
  }

  fn refresh_branch_pull_request(&mut self, refresh: PullRequestRefresh, cx: &mut Context<Self>) {
    let Some(repo_root) = self.repo_root.clone() else {
      self.pr_refresh_pending = 0;
      return;
    };
    if !AuthStateStore::has_github_access(cx) {
      self.pr_refresh_pending = 0;
      self.pr_fetched_at = None;
      self.set_branch_pr(BranchPrState::NoAccess, cx);
      self
        .review_list
        .update(cx, |list, cx| list.set_pull_request_loading(false, cx));
      cx.notify();
      return;
    }
    if !should_read_pull_request(refresh, self.pr_fetched_at, cx.background_executor().now()) {
      self.pr_refresh_pending = 0;
      return;
    }

    // Rereading the same pull request must not blank the panel: what is on
    // screen stays until the answer replaces it.
    if !matches!(
      self.branch_pr,
      BranchPrState::Found(_, _) | BranchPrState::Missing(_)
    ) {
      self.set_branch_pr(BranchPrState::Loading, cx);
      if self.pr_review_comments.is_empty() {
        self
          .review_list
          .update(cx, |list, cx| list.set_pull_request_loading(true, cx));
      }
    }
    let api = WorkspaceApi::global(cx).api.clone();
    let task = cx.spawn(async move |this, cx| {
      let (branch, state) = cx
        .background_spawn(async move {
          let branch = current_branch_status(&repo_root)
            .ok()
            .map(|status| status.name);
          let state = branch_pr_state_for_lookup(
            current_github_remote_repo(&repo_root).ok().flatten(),
            branch.clone(),
            |context| {
              api.fetch_pull_request_for_branch(&context.owner, &context.repo, &context.branch)
            },
          );
          (branch, state)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        let refreshing = this.pr_refresh_pending > 0;
        this.pr_refresh_pending = 0;
        this.pr_fetched_at = Some(cx.background_executor().now());
        this.pr_branch = branch;
        this.apply_branch_pull_request(state, cx);
        // A found pull request fans out into the range and checks reads: the
        // button keeps spinning until both land.
        if refreshing && matches!(this.branch_pr, BranchPrState::Found(_, _)) {
          this.pr_refresh_pending = 2;
        }
      });
    });
    self._pr_task = Some(task);
  }

  /// What the lookup answered. Another pull request: the details of the last one
  /// answer for nothing here. The same one keeps its own until the reread lands.
  fn apply_branch_pull_request(&mut self, state: BranchPrState, cx: &mut Context<Self>) {
    let found_pull_request = matches!(state, BranchPrState::Found(_, _));
    if pull_request_identity(&state) != pull_request_identity(&self.branch_pr) {
      self.reset_pull_request_details(cx);
    }
    self.set_branch_pr(state, cx);
    // The comments ride the range load below: an empty review panel says
    // loading until they land; no pull request means nothing is coming.
    let loading = found_pull_request && self.pr_review_comments.is_empty();
    self
      .review_list
      .update(cx, |list, cx| list.set_pull_request_loading(loading, cx));
    if found_pull_request {
      self.load_pull_request_range(cx);
      self.load_pull_request_checks(cx);
    }
    cx.notify();
  }

  /// The shas a pull request spans, and what it changes between them.
  fn load_pull_request_range(&mut self, cx: &mut Context<Self>) {
    let (Some(repo_root), BranchPrState::Found(context, pull_request)) =
      (self.repo_root.clone(), &self.branch_pr)
    else {
      return;
    };
    let owner = context.owner.clone();
    let repo = context.repo.clone();
    let number = pull_request.number;
    let api = WorkspaceApi::global(cx).api.clone();
    let sentry_owner = owner.clone();
    let sentry_repo = repo.clone();

    self.pr_files_loading = self.pr_files.is_empty();
    self.pr_files_error = None;
    let task = cx.spawn(async move |this, cx| {
      let loaded = cx
        .background_spawn(async move {
          let details = api.fetch_pull_request_details(&owner, &repo, number)?;
          let range = PullRequestRange {
            base: details.merge_base_sha.clone(),
            head: details.head_sha.clone(),
            base_ref: details.base_ref_name.clone(),
            head_ref: details.head_ref_name.clone(),
          };
          let files = list_pull_request_files(&repo_root, &range)?;
          let conversation = api
            .fetch_pull_request_conversation(&owner, &repo, number)
            .ok();
          let pull_request_node_id = conversation
            .as_ref()
            .map(|conversation| conversation.pull_request.node_id.clone())
            .unwrap_or_else(|| details.node_id.clone());
          let conversation = conversation.map(|conversation| PullRequestConversationLoad {
            reviewers: reviewer_rows(
              &details.requested_reviewers,
              &conversation.reviews,
              &details.author.login,
            ),
            review_comments: conversation.review_comments,
          });
          anyhow::Ok(PullRequestRangeLoad {
            range,
            files,
            conversation,
            author_login: details.author.login.clone(),
            pull_request_node_id,
          })
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.pr_refresh_pending = this.pr_refresh_pending.saturating_sub(1);
        this.pr_files_loading = false;
        match loaded {
          Ok(loaded) => {
            this.apply_pull_request_range_load(loaded, cx);
            crate::sentry_context::sync_github_pr_context(
              &sentry_owner,
              &sentry_repo,
              number,
              None,
            );
          }
          Err(error) => {
            // A read that failed is not a read: the next open tries again.
            this.pr_fetched_at = None;
            this.pr_files_error = Some(format!("{error}").into());
            this
              .review_list
              .update(cx, |list, cx| list.set_pull_request_loading(false, cx));
          }
        }
        cx.notify();
      });
    });
    self._pr_range_task = Some(task);
  }

  fn apply_pull_request_range_load(
    &mut self,
    loaded: PullRequestRangeLoad,
    cx: &mut Context<Self>,
  ) {
    self.pr_range = Some(loaded.range);
    self.set_pr_files(loaded.files, cx);
    self.pr_author_login = Some(loaded.author_login);
    self.pr_node_id = Some(loaded.pull_request_node_id);
    if let Some(conversation) = loaded.conversation {
      self.pr_reviewers = conversation.reviewers;
      self.apply_submitted_review_overlay();
      self.set_pull_request_review_comments(conversation.review_comments, cx);
    } else {
      self
        .review_list
        .update(cx, |list, cx| list.set_pull_request_loading(false, cx));
    }
  }

  /// Everything the panel knows about one pull request. Another branch means
  /// another pull request, and stale checks read as this one's.
  fn reset_pull_request_details(&mut self, cx: &mut Context<Self>) {
    crate::sentry_context::clear_github_pr_context();
    self.awaited_review_comment = None;
    self.set_pull_request_review_comments(Vec::new(), cx);
    self.pr_author_login = None;
    self.pr_node_id = None;
    self.pr_range = None;
    self.set_pr_files(Vec::new(), cx);
    self.pr_files_error = None;
    self.pr_selected_file = None;
    self.pr_reviewers = Vec::new();
    self.submitted_review_overlay = None;
    self.pr_checks = None;
    self.pr_checks_loading = false;
    self.pr_merge_readiness = None;
    self.selected_merge_method = None;
    self.pr_merging = false;
  }

  fn load_pull_request_checks(&mut self, cx: &mut Context<Self>) {
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return;
    };
    let owner = context.owner.clone();
    let repo = context.repo.clone();
    let number = pull_request.number;
    let api = WorkspaceApi::global(cx).api.clone();

    self.pr_checks_loading = true;
    let task = cx.spawn(async move |this, cx| {
      // The CI state and the mergeability answer the same question, can this
      // land, so they arrive together.
      let loaded = cx
        .background_spawn(async move {
          (
            api.fetch_pull_request_checks(&owner, &repo, number),
            api.fetch_pull_request_merge_readiness(&owner, &repo, number),
          )
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        this.pr_refresh_pending = this.pr_refresh_pending.saturating_sub(1);
        this.pr_checks_loading = false;
        this.pr_checks = loaded.0.ok();
        this.pr_merge_readiness = loaded.1.ok();
        this.sync_merge_method_with_readiness();
        cx.notify();
      });
    });
    self._pr_checks_task = Some(task);
  }

  /// The remembered method survives a readiness reload only while the
  /// repository still allows it.
  fn sync_merge_method_with_readiness(&mut self) {
    let Some(readiness) = &self.pr_merge_readiness else {
      return;
    };
    if self.selected_merge_method.is_none()
      && let Some(key) = self.merge_method_store_key()
    {
      self.selected_merge_method = crate::config::ConfigStore::load_merge_method(&key);
    }
    if let Some(method) = self.selected_merge_method
      && !readiness.available_methods.contains(&method)
    {
      self.selected_merge_method = None;
    }
  }

  fn merge_method_store_key(&self) -> Option<String> {
    let BranchPrState::Found(context, _) = &self.branch_pr else {
      return None;
    };
    Some(format!("{}/{}", context.owner, context.repo))
  }

  fn choose_merge_method(&mut self, method: GithubPullRequestMergeMethod, cx: &mut Context<Self>) {
    self.selected_merge_method = Some(method);
    if let Some(key) = self.merge_method_store_key() {
      crate::config::ConfigStore::persist_merge_method(&key, method);
    }
    cx.notify();
  }

  fn set_pull_request_review_comments(
    &mut self,
    comments: Vec<GithubPullRequestReviewComment>,
    cx: &mut Context<Self>,
  ) {
    self
      .review_list
      .update(cx, |list, cx| list.set_pull_request_loading(false, cx));
    let rows = pending_review_rows(&comments);
    let counts = file_comment_counts(&comments);
    self.pr_review_comments = comments;
    self.review_list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::PullRequest, rows, cx);
    });
    self.pr_files_list.update(cx, |list, cx| {
      list.delegate_mut().comment_counts = counts;
      cx.notify();
    });
    if let Some(comment_id) = self.awaited_review_comment.take() {
      self.open_review_comment(comment_id, cx);
    }
    cx.emit(DockPanelEvent::PullRequestReviewCommentsChanged);
    // The panel counts them itself, above the file list.
    cx.notify();
  }

  #[cfg(test)]
  pub(crate) fn set_pull_request_range_for_test(&mut self, range: PullRequestRange) {
    self.pr_range = Some(range);
  }

  #[cfg(test)]
  pub(crate) fn set_pull_request_review_comments_for_test(
    &mut self,
    comments: Vec<GithubPullRequestReviewComment>,
    cx: &mut Context<Self>,
  ) {
    self.set_pull_request_review_comments(comments, cx);
  }

  /// A link named a review comment: take the diff to the lines it is about. A
  /// comment the panel has not read yet is answered by the load that follows.
  pub(crate) fn reveal_review_comment(&mut self, comment_id: u64, cx: &mut Context<Self>) {
    if !self.open_review_comment(comment_id, cx) {
      self.awaited_review_comment = Some(comment_id);
    }
  }

  fn open_review_comment(&self, comment_id: u64, cx: &mut Context<Self>) -> bool {
    let (Some(range), Some(comment)) = (
      self.pr_range.as_ref(),
      self
        .pr_review_comments
        .iter()
        .find(|comment| comment.id == comment_id),
    ) else {
      return false;
    };
    cx.emit(DockPanelEvent::OpenPullRequestFile {
      base_oid: range.base.clone(),
      head_oid: range.head.clone(),
      path: PathBuf::from(comment.path.as_str()),
      line: Some(comment_line(comment)),
      intent: OpenIntent::Open,
    });
    true
  }

  pub(crate) fn pull_request_review_comments(&self) -> &[GithubPullRequestReviewComment] {
    &self.pr_review_comments
  }

  pub(crate) fn pull_request_number(&self) -> Option<u64> {
    match &self.branch_pr {
      BranchPrState::Found(_, pull_request) => Some(pull_request.number),
      _ => None,
    }
  }

  #[cfg(any(test, feature = "test-support"))]
  pub(crate) fn branch_pull_request_state_for_driver(&self) -> serde_json::Value {
    match &self.branch_pr {
      BranchPrState::NoAccess => serde_json::json!({ "status": "no_access" }),
      BranchPrState::NoRemote => serde_json::json!({ "status": "no_remote" }),
      BranchPrState::Loading => serde_json::json!({ "status": "loading" }),
      BranchPrState::Missing(context) => serde_json::json!({
        "status": "missing",
        "owner": context.owner,
        "repo": context.repo,
        "branch": context.branch,
      }),
      BranchPrState::Found(context, pull_request) => serde_json::json!({
        "status": "found",
        "owner": context.owner,
        "repo": context.repo,
        "branch": context.branch,
        "number": pull_request.number,
        "title": pull_request.title,
      }),
    }
  }

  #[cfg(any(test, feature = "test-support"))]
  pub(crate) fn pull_request_panel_state_for_driver(&self) -> serde_json::Value {
    serde_json::json!({
      "active_tab": driver_dock_tab(self.active_tab),
      "files_loading": self.pr_files_loading,
      "files_error": self.pr_files_error.as_ref().map(|error| error.to_string()),
      "files": self.pr_files.iter().map(driver_pr_file).collect::<Vec<_>>(),
      "range": self.pr_range.as_ref().map(|range| serde_json::json!({
        "base": range.base.as_str(),
        "head": range.head.as_str(),
        "base_ref": range.base_ref.as_str(),
        "head_ref": range.head_ref.as_str(),
      })),
      "author_login": self.pr_author_login.as_deref(),
      "reviewers": self.pr_reviewers.len(),
      "review_comments": self.pr_review_comments.len(),
      "review_comment_details": self.pr_review_comments.iter().map(driver_pr_review_comment).collect::<Vec<_>>(),
      "checks_loading": self.pr_checks_loading,
      "has_checks": self.pr_checks.is_some(),
      "merge_readiness_loaded": self.pr_merge_readiness.is_some(),
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  pub(crate) fn review_panel_state_for_driver(&self, cx: &App) -> serde_json::Value {
    let list = self.review_list.read(cx);
    let agent_comments = list.comments(ReviewSection::Agent);
    let pull_request_comments = list.comments(ReviewSection::PullRequest);
    serde_json::json!({
      "active_tab": driver_dock_tab(self.active_tab),
      "agent_comments": agent_comments.iter().map(driver_review_panel_comment).collect::<Vec<_>>(),
      "pull_request_comments": pull_request_comments.iter().map(driver_review_panel_comment).collect::<Vec<_>>(),
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  pub(crate) fn pull_request_file_target_for_driver(
    &self,
    rel_path: Option<&std::path::Path>,
  ) -> Option<(String, String, PathBuf)> {
    let range = self.pr_range.as_ref()?;
    let file = match rel_path {
      Some(path) => self.pr_files.iter().find(|file| file.path == path)?,
      None => self.pr_files.first()?,
    };
    Some((range.base.clone(), range.head.clone(), file.path.clone()))
  }

  /// Whether a new comment would join something: GitHub refuses a standalone
  /// comment while an unsubmitted review is open.
  pub(crate) fn has_pending_pull_request_review(&self) -> bool {
    pending_review_id(&self.pr_review_comments).is_some()
  }

  /// Comments written but not submitted: work the rail's dot must point at.
  pub(crate) fn pending_pull_request_comment_count(&self) -> usize {
    self
      .pr_review_comments
      .iter()
      .filter(|comment| comment.is_pending)
      .count()
  }

  /// GitHub is the source of truth for a pending review, so what it holds is
  /// read again rather than guessed from what we just did to it.
  fn refresh_pull_request_review_comments(&mut self, cx: &mut Context<Self>) {
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return;
    };
    let owner = context.owner.clone();
    let repo = context.repo.clone();
    let number = pull_request.number;
    let api = WorkspaceApi::global(cx).api.clone();

    // Only an empty panel shows the loading state: a refresh over existing
    // rows must not blank them.
    self
      .review_list
      .update(cx, |list, cx| list.set_pull_request_loading(true, cx));
    let task = cx.spawn(async move |this, cx| {
      let loaded = cx
        .background_spawn(async move { api.fetch_pull_request_conversation(&owner, &repo, number) })
        .await;
      let _ = this.update(cx, |this, cx| {
        this
          .review_list
          .update(cx, |list, cx| list.set_pull_request_loading(false, cx));
        if let Ok(conversation) = loaded {
          this.set_pull_request_review_comments(conversation.review_comments, cx);
        }
        cx.notify();
      });
    });
    self._pr_review_comments_task = Some(task);
  }

  /// The review the API just answered with is applied to the reviewers block
  /// at once: the refetch that follows can read a GitHub that has not caught
  /// up with its own write yet.
  fn note_submitted_review(&mut self, review: GithubPullRequestReview, cx: &mut Context<Self>) {
    self.submitted_review_overlay = Some(review);
    self.apply_submitted_review_overlay();
    cx.notify();
  }

  /// Overrides a fetched Awaiting for the reviewer who just spoke; once a
  /// fetch carries their answer, the fetches have caught up and the overlay
  /// retires, so a later re-request can read as awaiting again.
  fn apply_submitted_review_overlay(&mut self) {
    let Some(review) = &self.submitted_review_overlay else {
      return;
    };
    let (Some(user), Some(status)) = (review.user.as_ref(), submitted_review_status(review.state))
    else {
      self.submitted_review_overlay = None;
      return;
    };
    let row = self.pr_reviewers.iter_mut().find(|row| {
      crate::github_shared::logins_match_case_insensitive(row.login.as_str(), user.login.as_str())
    });
    match row {
      Some(row) if row.status == ReviewerStatus::Awaiting => row.status = status,
      Some(_) => self.submitted_review_overlay = None,
      None => {
        // The author reviews nothing of their own; anyone else who was not
        // requested still gets their row, as the fetch will give it later.
        let viewer_is_author = self.pr_author_login.as_deref().is_some_and(|author| {
          crate::github_shared::logins_match_case_insensitive(author, user.login.as_str())
        });
        if viewer_is_author {
          self.submitted_review_overlay = None;
          return;
        }
        self.pr_reviewers.push(ReviewerRow {
          login: user.login.clone(),
          avatar_url: user.avatar_url.clone(),
          status,
          latest_message: review
            .body
            .as_deref()
            .map(str::trim)
            .filter(|body| !body.is_empty())
            .map(str::to_string),
        });
      }
    }
  }

  /// The decision and its message are asked for in a dialog: three choices and
  /// a paragraph do not fit a 350px column, and this is not a gesture to make
  /// halfway.
  pub(crate) fn submit_pull_request_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return;
    };
    let viewer_login = AuthStateStore::get(cx).github_login();
    let target = review_submission_target(
      context.owner.clone(),
      context.repo.clone(),
      pull_request.number,
      &self.pr_review_comments,
      viewer_login.as_deref(),
      self.pr_author_login.as_deref(),
    );
    let api = WorkspaceApi::global(cx).api.clone();
    let panel = cx.entity().downgrade();
    let on_submitted: crate::review_submit_dialog::ReviewSubmittedHandler =
      std::rc::Rc::new(move |review, cx| {
        let _ = panel.update(cx, |panel, cx| {
          panel.note_submitted_review(review.clone(), cx);
          panel.refresh_branch_pull_request(PullRequestRefresh::Now, cx)
        });
      });
    let window_handle = self.window_handle;

    open_submit_review_dialog(api, window_handle, target, on_submitted, window, cx);
  }

  /// Deleting the review pending on GitHub is confirmed first: nobody else has
  /// seen it, but it lives on their servers and its comments go with it.
  #[cfg(any(test, feature = "test-support"))]
  pub(crate) fn submit_pull_request_review_for_driver(
    &mut self,
    body: String,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return Err("No pull request is loaded.".into());
    };
    let review_id = pending_review_id(&self.pr_review_comments)
      .ok_or_else(|| SharedString::from("No pending pull request review is loaded."))?;
    let owner = context.owner.clone();
    let repo = context.repo.clone();
    let number = pull_request.number;
    let api = WorkspaceApi::global(cx).api.clone();
    let panel = cx.entity().downgrade();
    let task = cx.spawn(async move |_, cx| {
      let result = cx
        .background_spawn(async move {
          api.submit_pending_review(
            &owner,
            &repo,
            number,
            &review_id,
            GithubPullRequestReviewEvent::Comment,
            &body,
          )
        })
        .await;
      let _ = panel.update(cx, |panel, cx| match result {
        Ok(review) => {
          panel.note_submitted_review(review, cx);
          panel.refresh_branch_pull_request(PullRequestRefresh::Now, cx);
        }
        Err(error) => {
          let _ = cx.update_window(panel.window_handle, |_, window, cx| {
            window.push_notification(
              Notification::error(format!("Review submit failed: {error}")),
              cx,
            );
          });
        }
      });
    });
    self._pr_review_comments_task = Some(task);
    Ok(())
  }

  pub(crate) fn discard_pull_request_review(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(review_id) = pending_review_id(&self.pr_review_comments) else {
      return;
    };
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return;
    };
    let owner = context.owner.clone();
    let repo = context.repo.clone();
    let number = pull_request.number;
    let title: SharedString = "Discard this review?".into();
    let message: SharedString =
      format!("Delete your pending review on #{number} and its comments from GitHub.").into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let owner = owner.clone();
      let repo = repo.clone();
      let review_id = review_id.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Discard")
        .cancel_text("Cancel")
        .on_confirm(move |_, _, cx| {
          view.update(cx, |this, cx| {
            this.discard_pending_pull_request_review(
              owner.clone(),
              repo.clone(),
              number,
              review_id.clone(),
              cx,
            )
          });
          true
        })
        .build(alert)
    });
  }

  fn discard_pending_pull_request_review(
    &mut self,
    owner: String,
    repo: String,
    number: u64,
    review_id: String,
    cx: &mut Context<Self>,
  ) {
    let api = WorkspaceApi::global(cx).api.clone();
    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(
          async move { api.discard_pending_review(&owner, &repo, number, &review_id) },
        )
        .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              window.push_notification(Notification::info("Review discarded"), cx);
            });
            this.refresh_pull_request_review_comments(cx);
          }
          Err(error) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              window.push_notification(Notification::error(format!("Discard failed: {error}")), cx);
            });
          }
        }
        cx.notify();
      });
    });
    self._pr_review_comments_task = Some(task);
  }

  /// A new comment on the pull request: it joins the review being written, or
  /// goes out on its own when that is what was asked for.
  pub(crate) fn create_pull_request_review_comment(
    &mut self,
    request: ReviewCommentCreateRequest,
    path: PathBuf,
    cx: &mut Context<Self>,
  ) {
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return;
    };
    let owner = context.owner.clone();
    let repo = context.repo.clone();
    let number = pull_request.number;
    let api = WorkspaceApi::global(cx).api.clone();
    let thread_node_id = request.in_reply_to_id.and_then(|id| {
      self
        .pr_review_comments
        .iter()
        .find(|comment| comment.id == id)
        .map(|comment| comment.thread_id.clone())
        .filter(|thread_id| !thread_id.is_empty())
    });
    let plan = review_comment_write_plan(
      &request,
      path.as_path(),
      pending_review_id(&self.pr_review_comments),
      thread_node_id,
      self.pr_node_id.clone(),
      self.pr_range.as_ref().map(|range| range.head.clone()),
    );

    if let ReviewCommentWrite::Unavailable(reason) = plan {
      self.finish_review_comment_write(anyhow::Result::<()>::Err(anyhow::anyhow!(reason)), cx);
      return;
    }

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          match plan {
            ReviewCommentWrite::ReplyToReview {
              review_id,
              thread_node_id,
              body,
            } => api
              .reply_pending_review_thread(
                &owner,
                &repo,
                number,
                &review_id,
                &thread_node_id,
                &body,
              )
              .map(|_| ()),
            ReviewCommentWrite::Reply {
              in_reply_to_id,
              body,
            } => api
              .reply_pull_request_review_comment(&owner, &repo, number, in_reply_to_id, &body)
              .map(|_| ()),
            ReviewCommentWrite::AddToReview {
              pull_request_node_id,
              review_id,
              anchor,
              body,
            } => {
              let review_id = match review_id {
                Some(review_id) => review_id,
                None => {
                  api
                    .start_pending_review(&owner, &repo, number, &pull_request_node_id)?
                    .node_id
                }
              };
              api
                .add_pending_review_thread(
                  &owner,
                  &repo,
                  number,
                  &pull_request_node_id,
                  &review_id,
                  &anchor.path,
                  &body,
                  "LINE",
                  Some(anchor.line),
                  Some(anchor.side.as_str()),
                  anchor.start_line,
                  anchor.start_side.as_deref(),
                )
                .map(|_| ())
            }
            ReviewCommentWrite::SingleComment {
              head_sha,
              anchor,
              body,
            } => api
              .create_pull_request_review_comment(
                &owner,
                &repo,
                number,
                &anchor.path,
                &head_sha,
                anchor.line,
                &anchor.side,
                anchor.start_line,
                anchor.start_side.as_deref(),
                &body,
              )
              .map(|_| ()),
            ReviewCommentWrite::Unavailable(reason) => Err(anyhow::anyhow!(reason)),
          }
        })
        .await;

      let _ = this.update(cx, |this, cx| this.finish_review_comment_write(result, cx));
    });
    self._pr_review_comments_task = Some(task);
  }

  pub(crate) fn edit_pull_request_review_comment(
    &mut self,
    id: u64,
    body: Arc<str>,
    cx: &mut Context<Self>,
  ) {
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return;
    };
    let owner = context.owner.clone();
    let repo = context.repo.clone();
    let number = pull_request.number;
    let api = WorkspaceApi::global(cx).api.clone();
    let body = body.as_ref().to_string();
    // A comment nobody has seen yet is edited by node id, a published one by
    // its number.
    let pending_node_id = self
      .pr_review_comments
      .iter()
      .find(|comment| comment.id == id && comment.is_pending)
      .map(|comment| comment.node_id.clone())
      .filter(|node_id| !node_id.is_empty());

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          match pending_node_id {
            Some(node_id) => {
              api.update_pending_review_comment(&owner, &repo, number, &node_id, &body)
            }
            None => api
              .update_pull_request_review_comment(&owner, &repo, number, id, &body)
              .map(|_| ()),
          }
        })
        .await;

      let _ = this.update(cx, |this, cx| this.finish_review_comment_write(result, cx));
    });
    self._pr_review_comments_task = Some(task);
  }

  /// A comment of a review nobody has submitted: dropping it is between the
  /// viewer and their own draft, so it needs no confirmation.
  pub(crate) fn delete_pending_review_comment(&mut self, id: u64, cx: &mut Context<Self>) {
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return;
    };
    let owner = context.owner.clone();
    let repo = context.repo.clone();
    let number = pull_request.number;
    let api = WorkspaceApi::global(cx).api.clone();
    let pending_node_id = self
      .pr_review_comments
      .iter()
      .find(|comment| comment.id == id && comment.is_pending)
      .and_then(|_| pending_review_comment_node_id(&self.pr_review_comments, id));

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          match pending_node_id {
            Some(node_id) => api.delete_pending_review_comment(&owner, &repo, number, &node_id),
            None => api.delete_pull_request_review_comment(&owner, &repo, number, id),
          }
        })
        .await;

      let _ = this.update(cx, |this, cx| this.finish_review_comment_write(result, cx));
    });
    self._pr_review_comments_task = Some(task);
  }

  pub(crate) fn toggle_pull_request_review_thread(
    &mut self,
    thread_id: Arc<str>,
    currently_resolved: bool,
    cx: &mut Context<Self>,
  ) {
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return;
    };
    let owner = context.owner.clone();
    let repo = context.repo.clone();
    let number = pull_request.number;
    let api = WorkspaceApi::global(cx).api.clone();
    let thread_id = thread_id.as_ref().to_string();

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          if currently_resolved {
            api.unresolve_pull_request_review_thread(&owner, &repo, number, &thread_id)
          } else {
            api.resolve_pull_request_review_thread(&owner, &repo, number, &thread_id)
          }
        })
        .await;

      let _ = this.update(cx, |this, cx| this.finish_review_comment_write(result, cx));
    });
    self._pr_review_comments_task = Some(task);
  }

  /// One landing for every write: GitHub is asked what it holds now, and the
  /// diff is told whether its composer can close.
  fn finish_review_comment_write<T>(&mut self, result: anyhow::Result<T>, cx: &mut Context<Self>) {
    match result {
      Ok(_) => {
        self.refresh_pull_request_review_comments(cx);
        cx.emit(DockPanelEvent::PullRequestReviewCommentSubmitted { error: None });
      }
      Err(error) => {
        cx.emit(DockPanelEvent::PullRequestReviewCommentSubmitted {
          error: Some(Arc::from(error.to_string().as_str())),
        });
      }
    }
    cx.notify();
  }

  fn toggle_pull_request_details(&mut self, cx: &mut Context<Self>) {
    self.pr_details_expanded = !self.pr_details_expanded;
    cx.notify();
  }

  /// Git prepared a message for the operation in progress (merge, rebase).
  pub(crate) fn set_commit_message(
    &mut self,
    message: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self
      .commit_input
      .update(cx, |input, cx| input.set_value(message, window, cx));
  }

  pub(crate) fn commit_message(&self, cx: &App) -> String {
    self.commit_input.read(cx).value().to_string()
  }

  /// The host owns the branch; the panel needs it to know what its menu allows.
  pub(crate) fn set_checkout_selector(
    &mut self,
    options: Vec<CheckoutOption>,
    pinned: bool,
    cx: &mut Context<Self>,
  ) {
    if self.checkout_options == options && self.checkout_pinned == pinned {
      return;
    }
    self.checkout_options = options;
    self.checkout_pinned = pinned;
    cx.notify();
  }

  pub(crate) fn set_branch_status(
    &mut self,
    branch_status: Option<git::BranchStatus>,
    cx: &mut Context<Self>,
  ) {
    let switched = branch_switched_since_lookup(
      self.pr_fetched_at,
      self.pr_branch.as_deref(),
      branch_status.as_ref().map(|status| status.name.as_str()),
    );
    self.branch_status = branch_status;
    if switched {
      self.refresh_branch_pull_request(PullRequestRefresh::Now, cx);
    }
    cx.notify();
  }

  /// The palette offers the same thing as the Pull request tab, so the keyboard
  /// reaches the branch's pull request without going through the dock.
  pub(crate) fn branch_pull_request_command(&self) -> Option<CommandPaletteCommand> {
    match &self.branch_pr {
      BranchPrState::NoAccess | BranchPrState::NoRemote => None,
      BranchPrState::Loading => Some(
        CommandPaletteCommand::create_pull_request().disabled("Checking for an open pull request"),
      ),
      // Publishing is a push: it stays a deliberate click in the tab.
      BranchPrState::Missing(_) if self.branch_needs_publishing() => None,
      BranchPrState::Missing(_) => Some(CommandPaletteCommand::create_pull_request()),
      BranchPrState::Found(_, pull_request) => Some(CommandPaletteCommand::open_pull_request(
        pull_request.number,
      )),
    }
  }

  /// What the Pull request tab's own button does, for the palette to reuse.
  pub(crate) fn create_branch_pull_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let BranchPrState::Missing(context) = &self.branch_pr else {
      return;
    };
    if self.branch_needs_publishing() {
      return;
    }
    open_create_pull_request_dialog(
      WorkspaceApi::global(cx).api.clone(),
      self.window_handle,
      self.pr_created_handler(cx),
      context.clone(),
      window,
      cx,
    );
  }

  #[cfg(test)]
  pub(crate) fn set_branch_pull_request_state(
    &mut self,
    state: BranchPrState,
    cx: &mut Context<Self>,
  ) {
    self.set_branch_pr(state, cx);
    cx.notify();
  }

  #[cfg(test)]
  pub(crate) fn set_pull_request_lookup_for_test(&mut self, branch: &str, at: Instant) {
    self.pr_branch = Some(branch.to_string());
    self.pr_fetched_at = Some(at);
  }

  /// The one place the branch's pull request changes, so a link from outside
  /// always knows what this panel is showing.
  fn set_branch_pr(&mut self, state: BranchPrState, cx: &mut Context<Self>) {
    let identity = pull_request_identity(&state);
    self.branch_pr = state;
    crate::pull_request_surface::PullRequestSurfaceHandle::publish(identity, cx);
  }

  pub(crate) fn open_branch_pull_request(&self, cx: &mut Context<Self>) {
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return;
    };
    cx.open_url(&github_pull_request_url(
      &context.owner,
      &context.repo,
      pull_request.number,
      false,
      None,
    ));
  }

  /// GitHub cannot open a pull request for a branch its remote has never seen.
  fn branch_needs_publishing(&self) -> bool {
    should_publish_branch(
      self.branch_status.as_ref(),
      self.head_status.has_head_commit,
    )
  }

  fn repo_state<'a>(&'a self, commit_message: &'a str) -> RepoState<'a> {
    let branch_status = self.branch_status.as_ref();
    let (can_push, can_force_push) =
      push_flags(branch_status, self.head_status.has_head_commit, false);
    RepoState {
      has_repo: self.repo_root.is_some(),
      merge_in_progress: self.merge_in_progress,
      rebase_in_progress: self.rebase_in_progress,
      has_head_commit: self.head_status.has_head_commit,
      can_push,
      can_force_push,
      can_undo_last_commit: self.head_status.can_undo_last_commit,
      branch_status,
      status_entries: &self.status_entries,
      selected_entry: None,
      commit_message,
    }
  }

  pub(crate) fn changes_list(&self) -> Entity<ChangesList> {
    self.changes_list.clone()
  }

  pub(crate) fn head_status(&self) -> HeadCommitStatus {
    self.head_status
  }

  pub(crate) fn merge_in_progress(&self) -> bool {
    self.merge_in_progress
  }

  pub(crate) fn rebase_in_progress(&self) -> bool {
    self.rebase_in_progress
  }

  pub(crate) fn status_entries(&self) -> &[RepoStatusEntry] {
    &self.status_entries
  }

  pub(crate) fn repo_root(&self) -> Option<&Path> {
    self.repo_root.as_deref()
  }

  pub(crate) fn set_repo_root(&mut self, repo_root: Option<PathBuf>, cx: &mut Context<Self>) {
    // Another checkout means another pull request: what is on screen or still
    // in flight answers for the one we left, and the staleness window or a
    // same-named branch would keep it alive. Drop it all before moving.
    if self.repo_root != repo_root {
      self._pr_task = None;
      self._pr_range_task = None;
      self._pr_checks_task = None;
      self.pr_refresh_pending = 0;
      self.pr_fetched_at = None;
      self.pr_branch = None;
      self.reset_pull_request_details(cx);
      self.set_branch_pr(BranchPrState::Loading, cx);
    }
    self.repo_root = repo_root.clone();
    self.status_entries.clear();
    self.last_error = None;
    self.changes_list.update(cx, |list, cx| {
      list.set_repo_root(repo_root.clone(), cx);
    });
    self.history_list.update(cx, |list, cx| {
      list.set_repo_root(repo_root.clone(), cx);
    });
    if let Some(terminal) = self.terminal_view.clone() {
      terminal.update(cx, |terminal, cx| {
        terminal.set_working_directory(repo_root, cx);
      });
    }
  }

  /// The history is only worth loading once its tab is opened.
  fn refresh_history(&mut self, cx: &mut Context<Self>) {
    let repo_root = self.repo_root.clone();
    self.history_list.update(cx, |list, cx| {
      list.set_repo_root(repo_root, cx);
      if list.is_empty() {
        list.refresh(cx);
      }
    });
  }

  fn render_review_tab(&self) -> AnyElement {
    // No padding here: the list's own header and footer rules run edge to edge,
    // like the commit zone's does on the Changes tab.
    div()
      .id("dock-panel-review")
      .debug_selector(|| DOCK_PANEL_REVIEW_DEBUG_SELECTOR.to_string())
      .flex_1()
      .min_h_0()
      .min_w(px(0.0))
      .child(self.review_list.clone())
      .into_any_element()
  }

  fn render_history_tab(&self) -> AnyElement {
    div()
      .id("dock-panel-history")
      .debug_selector(|| DOCK_PANEL_HISTORY_DEBUG_SELECTOR.to_string())
      .flex_1()
      .min_h_0()
      .min_w(px(0.0))
      .px_1()
      .py_1()
      .child(self.history_list.clone())
      .into_any_element()
  }

  fn ensure_terminal(&mut self, cx: &mut Context<Self>) {
    if self.terminal_view.is_some() {
      return;
    }
    let working_directory = self.repo_root.clone();
    self.terminal_view = Some(cx.new(|cx| TerminalView::new(working_directory, cx)));
  }

  /// Shows the shell, never starts it: spawning a process while painting is the
  /// mistake this crate already made with the agent panel.
  fn render_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let Some(terminal) = self.terminal_view.clone() else {
      return div().into_any_element();
    };

    // Rendering is the first moment the shell exists to take the keyboard the
    // tab was opened for.
    if self.focus_terminal_when_rendered {
      self.focus_terminal_when_rendered = false;
      window.focus(&terminal.read(cx).focus_handle(cx), cx);
    }

    div()
      .id("dock-panel-terminal")
      .debug_selector(|| DOCK_PANEL_TERMINAL_DEBUG_SELECTOR.to_string())
      .flex_1()
      .min_h_0()
      .min_w(px(0.0))
      .px_2()
      .py_1()
      .child(terminal)
      .into_any_element()
  }

  fn has_staged_changes(&self) -> bool {
    self
      .status_entries
      .iter()
      .any(|entry| !matches!(entry.stage, RepoStage::Unstaged))
  }

  pub(crate) fn commit(&mut self, cx: &mut Context<Self>) {
    if self.committing {
      return;
    }
    let Some(repo_root) = self.repo_root.clone() else {
      return;
    };
    let message = self.commit_input.read(cx).value().to_string();
    if message.trim().is_empty() || self.status_entries.is_empty() {
      return;
    }
    let stage_all_needed = !self.has_staged_changes();
    self.committing = true;
    self.last_error = None;
    cx.notify();
    crate::analytics::track(cx, "commit_made");

    let window_handle = self.window_handle;
    let commit_input = self.commit_input.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          if stage_all_needed {
            stage_all(&repo_root)?;
          }
          commit_changes(&repo_root, &message)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.committing = false;
        match result {
          Ok(()) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            });
            cx.emit(DockPanelEvent::Committed);
          }
          Err(error) => this.last_error = Some(format!("{error}").into()),
        }
        this.refresh(cx);
      });
    });
    self._commit_task = Some(task);
  }

  fn render_commit_zone(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    // A rebase ends by continuing it, not by writing another commit message.
    let continuing_rebase = self.rebase_in_progress;
    let can_commit = if continuing_rebase {
      !crate::changes_list::has_conflicted_entries(&self.status_entries)
    } else {
      !self.committing
        && !self.status_entries.is_empty()
        && !self.commit_input.read(cx).value().trim().is_empty()
    };
    let commit_shortcut = crate::shortcuts::resolved_display_shortcut_keystroke_in(
      cx,
      window,
      crate::shortcuts::ShortcutId::CommitChanges,
    );

    v_flex()
      .gap_2()
      .p_2()
      .border_t_1()
      .border_color(theme.border)
      .when_some(self.last_error.clone(), |this, error| {
        this.child(div().text_xs().text_color(theme.status_red()).child(error))
      })
      .child(div().w_full().min_w_0().key_context("CommitInput").child({
        let commit_box = Textarea::new(&self.commit_input).w_full();
        commit_box.into_any_element()
      }))
      .when(self.merge_in_progress || continuing_rebase, |this| {
        let label = if continuing_rebase {
          "Rebase in progress"
        } else {
          "Merge in progress"
        };
        this.child(
          div()
            .debug_selector(|| DOCK_PANEL_OPERATION_DEBUG_SELECTOR.to_string())
            .text_xs()
            .text_color(theme.status_orange())
            .child(label),
        )
      })
      .child(h_flex().w_full().child(self.render_commit_button(
        continuing_rebase,
        can_commit,
        commit_shortcut,
        cx,
      )))
      .into_any_element()
  }

  /// The primary action, plus the menu of what else can be done to the last
  /// commit or the branch.
  fn render_commit_button(
    &self,
    continuing_rebase: bool,
    can_commit: bool,
    commit_shortcut: gpui::Keystroke,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let commit_message = self.commit_input.read(cx).value().to_string();
    let state = self.repo_state(&commit_message);
    let menu_items = [
      CommitMenuCommand::Amend,
      CommitMenuCommand::UndoLastCommit,
      CommitMenuCommand::Push,
      CommitMenuCommand::ForcePush,
    ]
    .map(|command| (command, state.allows(command.rule())));
    let menu_enabled = menu_items.iter().any(|(_, allowed)| *allowed);
    let view = cx.entity();

    h_flex()
      .w_full()
      .child(
        Button::new("dock-panel-commit")
          .tab_index(2)
          .label(if continuing_rebase {
            "Continue rebase"
          } else {
            "Commit"
          })
          .debug_selector(|| DOCK_PANEL_COMMIT_DEBUG_SELECTOR.to_string())
          .with_variant(gpui_component::button::ButtonVariant::Secondary)
          .outline()
          .small()
          .w_full()
          .child(gpui_component::kbd::Kbd::new(commit_shortcut).ml_1())
          .loading(self.committing)
          .disabled(!can_commit)
          .on_click(cx.listener(|this, _, _, cx| {
            if this.rebase_in_progress {
              cx.emit(DockPanelEvent::ContinueRebase);
              return;
            }
            this.commit(cx)
          }))
          .flex_1()
          .rounded_r_none(),
      )
      .child(
        Button::new("dock-panel-commit-menu")
          .tab_index(3)
          .icon(IconName::ChevronDown)
          .with_variant(gpui_component::button::ButtonVariant::Secondary)
          .outline()
          .small()
          .rounded_l_none()
          .border_l_0()
          .debug_selector(|| DOCK_PANEL_COMMIT_MENU_DEBUG_SELECTOR.to_string())
          .disabled(!menu_enabled)
          .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, _| {
            menu_items.iter().fold(menu, |menu, (command, allowed)| {
              let view = view.clone();
              let command = *command;
              menu.item(
                PopupMenuItem::new(command.label())
                  .icon(command.icon())
                  .disabled(!allowed)
                  .on_click(move |_, _, cx| {
                    view.update(cx, |_, cx| cx.emit(DockPanelEvent::RunCommand(command)));
                  }),
              )
            })
          }),
      )
      .into_any_element()
  }

  /// Opens a tab and gives it focus, loading what that tab needs the first time.
  pub(crate) fn open_tab(
    &mut self,
    target: DockPanelTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab != target {
      self.active_tab = target;
      match target {
        DockPanelTab::PullRequest => {
          self.refresh_branch_pull_request(PullRequestRefresh::IfStale, cx)
        }
        DockPanelTab::Terminal => self.ensure_terminal(cx),
        DockPanelTab::History => self.refresh_history(cx),
        DockPanelTab::Changes | DockPanelTab::Files | DockPanelTab::Review => {}
      }
    }
    match target {
      DockPanelTab::Changes => {
        if self.status_entries.is_empty() {
          // The empty state mounts no list; its handle would drop the focus.
          window.focus(&self.focus_handle, cx);
        } else {
          let list = self.changes_list.clone();
          list.update(cx, |list, cx| list.focus(window, cx));
        }
      }
      DockPanelTab::History => {
        let history = self.history_list.clone();
        history.update(cx, |history, cx| history.focus(window, cx));
      }
      DockPanelTab::Files => {
        if self.files_loaded {
          self
            .files_tree_state
            .update(cx, |tree, cx| tree.focus(window, cx));
        } else {
          self.focus_files_tree_when_loaded = true;
          window.focus(&self.focus_handle, cx);
        }
      }
      DockPanelTab::Review => {
        if self.review_list.read(cx).is_empty() {
          // The empty state mounts no list; its handle would drop the focus.
          window.focus(&self.focus_handle, cx);
        } else {
          let review = self.review_list.clone();
          review.update(cx, |review, cx| review.focus(window, cx));
        }
      }
      DockPanelTab::PullRequest => {
        if self.pr_files.is_empty() {
          // Nothing to walk yet: the list takes over when its files land.
          self.focus_pr_files_when_loaded = true;
          window.focus(&self.focus_handle, cx);
        } else {
          let list = self.pr_files_list.clone();
          list.update(cx, |list, cx| list.focus(window, cx));
        }
      }
      DockPanelTab::Terminal => {
        self.ensure_terminal(cx);
        self.focus_terminal_when_rendered = true;
        window.focus(&self.focus_handle, cx);
      }
    }
    cx.notify();
  }

  /// Whether the keyboard is already in the surface a tab's shortcut focuses.
  /// Being in the panel is not enough: the commit box is in the Changes tab, and
  /// its shortcut should reach the file list from there.
  #[cfg(test)]
  pub(crate) fn review_list(&self) -> &Entity<ReviewList> {
    &self.review_list
  }

  pub(crate) fn tab_has_focus(&self, tab: DockPanelTab, window: &Window, cx: &App) -> bool {
    // An empty surface mounts nothing to focus and the panel holds it instead;
    // the shortcut still has to be able to send the dock away.
    if self.focus_handle.is_focused(window) {
      return true;
    }
    match tab {
      DockPanelTab::Changes => self.changes_list.read(cx).is_focused(window, cx),
      DockPanelTab::Review => self.review_list.read(cx).is_focused(window, cx),
      DockPanelTab::Files => self
        .files_tree_state
        .read(cx)
        .focus_handle(cx)
        .contains_focused(window, cx),
      DockPanelTab::History => self.history_list.read(cx).tree_has_focus(window, cx),
      DockPanelTab::PullRequest => self
        .pr_files_list
        .read(cx)
        .focus_handle(cx)
        .contains_focused(window, cx),
      DockPanelTab::Terminal => self.terminal_view.as_ref().is_some_and(|terminal| {
        terminal
          .read(cx)
          .focus_handle(cx)
          .contains_focused(window, cx)
      }),
    }
  }

  /// Switches tab without taking the focus: for the panel following what the page
  /// is doing, rather than the user asking for it.
  pub(crate) fn select_tab(&mut self, target: DockPanelTab, cx: &mut Context<Self>) {
    if self.active_tab == target {
      return;
    }
    self.active_tab = target;
    cx.notify();
  }

  #[cfg(test)]
  pub(crate) fn has_terminal(&self) -> bool {
    self.terminal_view.is_some()
  }

  pub(crate) fn set_zoomed(&mut self, zoomed: bool, cx: &mut Context<Self>) {
    if self.zoomed != zoomed {
      self.zoomed = zoomed;
      cx.notify();
    }
  }

  pub(crate) fn active_tab(&self) -> DockPanelTab {
    self.active_tab
  }

  fn render_files_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();

    // Rendering is the first moment the tree exists to take the focus the tab
    // was opened for.
    if self.focus_files_tree_when_loaded && self.files_loaded {
      self.focus_files_tree_when_loaded = false;
      self
        .files_tree_state
        .update(cx, |tree, cx| tree.focus(window, cx));
    }

    if !self.files_loaded {
      if !self.files_loading {
        self.load_worktree_files(cx);
      }
      return v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading files..."),
        )
        .into_any_element();
    }

    let modified: std::collections::HashSet<PathBuf> = self
      .status_entries
      .iter()
      .map(|entry| entry.path.clone())
      .collect();

    div()
      .flex_1()
      .min_h_0()
      .py_1()
      .px_1()
      .child(tree(
        &self.files_tree_state,
        move |ix, entry, selected, _window, cx| {
          let theme = cx.theme().clone();
          let item = entry.item();
          let is_folder = entry.is_folder();
          let icon: AnyElement = if is_folder {
            Icon::new(if entry.is_expanded() {
              IconName::FolderOpen
            } else {
              IconName::Folder
            })
            .size_3()
            .text_color(theme.muted_foreground)
            .into_any_element()
          } else {
            ui::file_icon_path_for_name_with_theme(item.label.as_ref(), &theme)
              .map(|path| img(path).size(px(ui::FILE_ICON_SIZE_PX)).into_any_element())
              .unwrap_or_else(|| {
                Icon::new(IconName::File)
                  .size_3()
                  .text_color(theme.muted_foreground)
                  .into_any_element()
              })
          };
          let is_modified = !is_folder && modified.contains(&PathBuf::from(item.id.as_ref()));

          let indent = px(8.) + px(14.) * entry.depth();
          ui::selectable_list_item(ix, selected, ui::SelectableRowStyle::Inset, &theme)
            .w_full()
            .px_2()
            .pl(indent)
            .child(
              h_flex()
                .items_center()
                .gap_2()
                .child(icon)
                .child(
                  div()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_sm()
                    .child(item.label.clone()),
                )
                .when(is_modified, |this| {
                  this.child(
                    div()
                      .text_xs()
                      .font_weight(gpui::FontWeight::BOLD)
                      .text_color(theme.status_amber())
                      .child("M"),
                  )
                }),
            )
        },
      ))
      .into_any_element()
  }

  fn pr_created_handler(&self, cx: &mut Context<Self>) -> PullRequestCreatedHandler {
    let panel = cx.entity().downgrade();
    Rc::new(move |_context, _pull_request, cx| {
      let _ = panel.update(cx, |panel, cx| {
        panel.refresh_branch_pull_request(PullRequestRefresh::Now, cx);
      });
    })
  }

  fn render_pr_message(&self, text: &'static str, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    v_flex()
      .flex_1()
      .items_center()
      .justify_center()
      .gap_2()
      .px_4()
      .child(
        Icon::new(UiIconName::GitPullRequest)
          .size_4()
          .text_color(theme.muted_foreground),
      )
      .child(
        div()
          .text_sm()
          .text_center()
          .text_color(theme.muted_foreground)
          .child(text),
      )
      .into_any_element()
  }

  fn render_pr_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    match &self.branch_pr {
      // Nothing to show, so this is where Reviu says what it could show.
      BranchPrState::NoAccess => {
        let github_access = AuthStateStore::github_access_state(cx);
        match render_pro_promise(ProPromiseSurface::PullRequestPanel, github_access, cx) {
          Some(promise) => promise,
          None => self.render_pr_message(
            "Sign in with GitHub to link this branch to a pull request",
            cx,
          ),
        }
      }
      BranchPrState::NoRemote => self.render_pr_message("No GitHub remote on this repository", cx),
      BranchPrState::Loading => self.render_pr_message("Loading pull request...", cx),
      BranchPrState::Missing(context) if self.branch_needs_publishing() => {
        let context = context.clone();
        v_flex()
          .flex_1()
          .items_center()
          .justify_center()
          .gap_3()
          .px_4()
          .child(
            Icon::new(UiIconName::GitPullRequestArrow)
              .size_4()
              .text_color(theme.muted_foreground),
          )
          .child(
            div()
              .text_sm()
              .text_center()
              .text_color(theme.muted_foreground)
              .child(format!("{} is not on the remote yet", context.branch)),
          )
          .child(
            Button::new("dock-panel-publish-and-create-pr")
              .primary()
              .small()
              .label("Publish and create pull request")
              .debug_selector(|| DOCK_PANEL_PUBLISH_AND_CREATE_PR_DEBUG_SELECTOR.to_string())
              .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(DockPanelEvent::PublishBranchAndCreatePullRequest(
                  context.clone(),
                ));
              })),
          )
          .into_any_element()
      }
      BranchPrState::Missing(context) => {
        let context = context.clone();
        v_flex()
          .flex_1()
          .items_center()
          .justify_center()
          .gap_3()
          .px_4()
          .child(
            Icon::new(UiIconName::GitPullRequestArrow)
              .size_4()
              .text_color(theme.muted_foreground),
          )
          .child(
            div()
              .text_sm()
              .text_center()
              .text_color(theme.muted_foreground)
              .child(format!("No pull request for {}", context.branch)),
          )
          .child(
            Button::new("dock-panel-create-pr")
              .primary()
              .small()
              .label("Create pull request")
              .debug_selector(|| DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR.to_string())
              .on_click(cx.listener({
                let context = context.clone();
                move |this, _, window, cx| {
                  open_create_pull_request_dialog(
                    WorkspaceApi::global(cx).api.clone(),
                    this.window_handle,
                    this.pr_created_handler(cx),
                    context.clone(),
                    window,
                    cx,
                  );
                }
              })),
          )
          .child(
            // Reviewers, labels and projects live on github.com, not in our dialog.
            Button::new("dock-panel-compare-on-github")
              .ghost()
              .xsmall()
              .label("Open compare on GitHub")
              .debug_selector(|| DOCK_PANEL_COMPARE_DEBUG_SELECTOR.to_string())
              .on_click(move |_, _, cx| {
                open_compare_target(
                  context.owner.clone(),
                  context.repo.clone(),
                  context.branch.clone(),
                  cx,
                );
              }),
          )
          .into_any_element()
      }
      BranchPrState::Found(context, pull_request) => v_flex()
        .size_full()
        .min_h_0()
        .child(self.render_pr_identity(context, pull_request, cx))
        .children(self.render_pr_pending_comments(cx))
        .child(self.render_pr_checks(cx))
        .child(self.render_pr_files(window, cx))
        .into_any_element(),
    }
  }

  /// A comment written here lands in the Review tab, which is another tab: say
  /// so, and take them there.
  fn render_pr_pending_comments(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
    let theme = cx.theme().clone();
    let count = self
      .pr_review_comments
      .iter()
      .filter(|comment| comment.is_pending)
      .count();
    if count == 0 {
      return None;
    }

    Some(
      h_flex()
        .id("dock-panel-pr-pending-comments")
        .debug_selector(|| DOCK_PANEL_PR_PENDING_COMMENTS_DEBUG_SELECTOR.to_string())
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .border_b_1()
        .border_color(theme.border)
        .hover(|this| this.bg(theme.accent))
        .cursor_pointer()
        .on_click(cx.listener(|this, _, window, cx| {
          this.open_tab(DockPanelTab::Review, window, cx);
        }))
        .child(
          Icon::new(UiIconName::MessageCircle)
            .size_3()
            .text_color(theme.muted_foreground),
        )
        .child(
          div()
            .flex_1()
            .min_w_0()
            .text_xs()
            .text_color(theme.foreground)
            .child(format!(
              "{count} {} waiting in Review",
              singular_or_plural(count as u64, "comment", "comments")
            )),
        )
        .child(
          Icon::new(IconName::ChevronRight)
            .size_3()
            .text_color(theme.muted_foreground),
        )
        .into_any_element(),
    )
  }

  /// Pinned above the file list: what you are looking at stays visible while you
  /// work in the list below.
  fn render_pr_identity(
    &self,
    context: &GithubBranchContext,
    pull_request: &GithubPullRequest,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let status = pull_request.status();
    let number = pull_request.number;
    let owner = context.owner.clone();
    let repo = context.repo.clone();
    let branches = self
      .pr_range
      .as_ref()
      .map(|range| format!("{} into {}", range.head_ref, range.base_ref))
      .unwrap_or_else(|| context.branch.clone());

    v_flex()
      .flex_shrink_0()
      .gap_1()
      .p_3()
      .border_b_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_2()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(
                div()
                  .text_xs()
                  .font_weight(gpui::FontWeight::SEMIBOLD)
                  .text_color(pull_request_status_color(status, &theme))
                  .child(pull_request_status_label(status)),
              )
              .child(div().text_xs().text_color(theme.muted_foreground).child(
                if pull_request.comments_count > 0 {
                  format!("#{number} · {} comments", pull_request.comments_count)
                } else {
                  format!("#{number}")
                },
              )),
          )
          .child(
            Button::new("dock-panel-open-pr")
              .ghost()
              .xsmall()
              .compact()
              .icon(UiIconName::Globe)
              .tooltip("Open on GitHub")
              .on_click(cx.listener(move |_, _, _, cx| {
                cx.open_url(&github_pull_request_url(&owner, &repo, number, false, None));
              })),
          ),
      )
      .child(
        div()
          .text_sm()
          .text_color(theme.foreground)
          .child(pull_request.title.clone()),
      )
      .child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .truncate()
          .child(branches),
      )
      .into_any_element()
  }

  /// One line for the whole CI, expandable into one line per check. Collapsed by
  /// default: the file list below is what you came for. Always there even with
  /// nothing to report, because it carries what can be done to the pull request.
  fn render_pr_checks(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let checks = self
      .pr_checks
      .as_ref()
      .filter(|checks| checks.total_checks > 0 || !checks.missing_required_contexts.is_empty());
    let expanded = self.pr_details_expanded;
    let mut rows = checks.map(check_rows).unwrap_or_default();
    rows.sort_by_key(check_state_sort_key);

    let mut block = v_flex()
      .flex_shrink_0()
      .border_b_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .id("dock-panel-pr-checks-toggle")
          .debug_selector(|| DOCK_PANEL_PR_CHECKS_DEBUG_SELECTOR.to_string())
          .w_full()
          .items_center()
          .gap_2()
          .px_3()
          .py_2()
          .hover(|this| this.bg(theme.accent))
          .cursor_pointer()
          .on_click(cx.listener(|this, _, _, cx| this.toggle_pull_request_details(cx)))
          .child(Icon::new(if expanded {
            IconName::ChevronDown
          } else {
            IconName::ChevronRight
          }))
          .when_some(checks, |this, checks| {
            let counts = checks_state_counts(checks);
            this
              .child(check_state_icon(checks.overall_state, &theme).size_3())
              .child(
                div()
                  .flex_1()
                  .min_w_0()
                  .text_sm()
                  .text_color(theme.foreground)
                  .truncate()
                  .child(checks_summary_title(checks)),
              )
              .when(!counts.is_empty(), |this| {
                this.child(
                  h_flex()
                    .debug_selector(|| DOCK_PANEL_PR_CHECKS_COUNTS_DEBUG_SELECTOR.to_string())
                    .flex_shrink_0()
                    .items_center()
                    .gap_1p5()
                    .children(counts.into_iter().map(|(state, count)| {
                      h_flex()
                        .items_center()
                        .gap_0p5()
                        .child(check_state_icon(state, &theme).size_3())
                        .child(
                          div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(count.to_string()),
                        )
                    })),
                )
              })
          })
          .when(checks.is_none(), |this| {
            this.child(
              div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(if self.pr_checks_loading {
                  "Loading checks...".to_string()
                } else if self.pr_reviewers.is_empty() {
                  "No checks or reviewers".to_string()
                } else {
                  reviewers_summary_title(&self.pr_reviewers)
                }),
            )
          })
          // Closed, the avatars still say who has answered.
          .child(render_reviewer_avatars(&self.pr_reviewers, &theme)),
      );

    if !expanded {
      return block.into_any_element();
    }

    if checks.is_some() && !rows.is_empty() {
      // Six-or-so rows on screen, the rest behind a real scrollbar: a wide CI
      // must not push the file list out of the panel.
      let mut list = v_flex()
        .id("dock-panel-pr-checks-rows")
        .w_full()
        .max_h(px(300.))
        .overflow_y_scroll()
        .track_scroll(&self.pr_checks_scroll)
        .gap_0p5()
        .px_1()
        .pb_2();
      for row in rows {
        list = list.child(render_check_row(&row, &theme, cx));
      }
      block = block.child(
        div()
          .relative()
          .child(list)
          .vertical_scrollbar(&self.pr_checks_scroll),
      );
    }

    if !self.pr_reviewers.is_empty() {
      let mut list = v_flex().w_full().gap_0p5().px_1().pb_2();
      for reviewer in &self.pr_reviewers {
        list = list.child(render_reviewer_row(reviewer, &theme));
      }
      block = block.child(
        v_flex()
          .border_t_1()
          .border_color(theme.border)
          .child(
            div()
              .px_3()
              .py_1()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(reviewers_summary_title(&self.pr_reviewers)),
          )
          .child(list),
      );
    }

    block = block.child(self.render_pr_actions(cx));

    block.into_any_element()
  }

  /// Merging from a narrow column: the button names the method it will use, and
  /// a confirmation repeats it, because the target is small and the act is not
  /// undoable.
  /// What can be done to the pull request from here: say something about it,
  /// and land it.
  fn render_pr_actions(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let availability =
      merge_availability(self.pr_merge_readiness.as_ref(), self.selected_merge_method);
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return div().into_any_element();
    };
    let owner = context.owner.clone();
    let repo = context.repo.clone();
    let number = pull_request.number;

    let (label, message, request) = match &availability {
      MergeAvailability::Unknown => (
        "Merge",
        "Checking whether this can merge...".to_string(),
        None,
      ),
      MergeAvailability::Blocked(reason) => ("Merge", reason.clone(), None),
      MergeAvailability::Ready { method, head_sha } => (
        merge_method_label(*method),
        "Ready to merge".to_string(),
        Some(MergeRequest {
          owner,
          repo,
          number,
          method: *method,
          head_sha: head_sha.clone(),
        }),
      ),
    };
    // Methods the repository forbids are not offered at all.
    let other_methods: Vec<GithubPullRequestMergeMethod> = match &availability {
      MergeAvailability::Ready { method, .. } => self
        .pr_merge_readiness
        .as_ref()
        .map(|readiness| {
          readiness
            .available_methods
            .iter()
            .copied()
            .filter(|other| other != method)
            .collect()
        })
        .unwrap_or_default(),
      _ => Vec::new(),
    };
    let view = cx.entity();

    h_flex()
      .w_full()
      .items_center()
      .justify_between()
      .gap_2()
      .px_3()
      .py_2()
      .border_t_1()
      .border_color(theme.border)
      .child(
        div()
          .flex_1()
          .min_w_0()
          .text_xs()
          .text_color(theme.muted_foreground)
          .truncate()
          .child(message),
      )
      .child(
        Button::new("dock-panel-pr-review")
          .debug_selector(|| DOCK_PANEL_PR_REVIEW_DEBUG_SELECTOR.to_string())
          .outline()
          .small()
          .compact()
          .label("Review")
          .tooltip("Approve, comment or request changes")
          .on_click(cx.listener(|this, _, window, cx| this.submit_pull_request_review(window, cx))),
      )
      .child(
        h_flex()
          .child(
            Button::new("dock-panel-pr-merge")
              .debug_selector(|| DOCK_PANEL_PR_MERGE_DEBUG_SELECTOR.to_string())
              .primary()
              .small()
              .compact()
              .label(label)
              .disabled(request.is_none() || self.pr_merging)
              .on_click(cx.listener(move |this, _, window, cx| {
                let Some(request) = request.clone() else {
                  return;
                };
                this.confirm_merge_pull_request(request, window, cx);
              }))
              .when(!other_methods.is_empty(), |this| this.rounded_r_none()),
          )
          .when(!other_methods.is_empty(), |this| {
            this.child(
              Button::new("dock-panel-pr-merge-method")
                .debug_selector(|| DOCK_PANEL_PR_MERGE_METHOD_DEBUG_SELECTOR.to_string())
                .icon(IconName::ChevronDown)
                .primary()
                .small()
                .compact()
                .rounded_l_none()
                .border_l_0()
                .disabled(self.pr_merging)
                .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, _| {
                  other_methods.iter().fold(menu, |menu, method| {
                    let view = view.clone();
                    let method = *method;
                    menu.item(PopupMenuItem::new(merge_method_label(method)).on_click(
                      move |_, _, cx| {
                        view.update(cx, |this, cx| this.choose_merge_method(method, cx));
                      },
                    ))
                  })
                }),
            )
          }),
      )
      .into_any_element()
  }

  fn confirm_merge_pull_request(
    &mut self,
    request: MergeRequest,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let defaults = merge_commit_defaults(self.pr_merge_readiness.as_ref(), request.method);
    let view = cx.entity().downgrade();
    let number = request.number;
    let method = request.method;
    let on_confirmed: MergeConfirmedHandler = std::rc::Rc::new(move |title, message, cx| {
      let request = request.clone();
      let _ = view.update(cx, |this, cx| {
        this.merge_pull_request(request, title, message, cx)
      });
    });

    open_merge_dialog(number, method, defaults, on_confirmed, window, cx);
  }

  fn merge_pull_request(
    &mut self,
    request: MergeRequest,
    commit_title: Option<String>,
    commit_message: Option<String>,
    cx: &mut Context<Self>,
  ) {
    let api = WorkspaceApi::global(cx).api.clone();
    let window_handle = self.window_handle;
    let number = request.number;

    // GitHub remembers the last method used per repository; so do we.
    if let Some(key) = self.merge_method_store_key() {
      crate::config::ConfigStore::persist_merge_method(&key, request.method);
    }
    self.pr_merging = true;
    cx.notify();
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          api.merge_pull_request(
            &request.owner,
            &request.repo,
            request.number,
            request.method,
            &request.head_sha,
            commit_title.as_deref(),
            commit_message.as_deref(),
          )
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.pr_merging = false;
        match result {
          Ok(_) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              window.push_notification(Notification::info(format!("Merged #{number}")), cx);
            });
            // The branch is behind its own reality now: everything reloads.
            this.refresh_branch_pull_request(PullRequestRefresh::Now, cx);
          }
          Err(error) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              window.push_notification(Notification::error(format!("Merge failed: {error}")), cx);
            });
          }
        }
        cx.notify();
      });
    });
    self._pr_merge_task = Some(task);
  }

  /// What the branch proposes against its base: the committed changes, which is
  /// a different question from the working tree of the Changes tab.
  /// The rows and the list that walks them are the same thing: they are set
  /// together or they drift.
  pub(crate) fn set_pr_files(
    &mut self,
    files: Vec<git::CommitChangedFile>,
    cx: &mut Context<Self>,
  ) {
    self.pr_files = files.clone();
    self.pr_files_list.update(cx, |list, cx| {
      list.delegate_mut().files = files;
      cx.notify();
    });
  }

  fn render_pr_files(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    if let Some(error) = self.pr_files_error.clone() {
      return self.render_pr_files_message(error, cx);
    }
    if self.pr_files_loading {
      return self.render_pr_files_message("Loading changed files...".into(), cx);
    }
    if self.pr_files.is_empty() {
      return self.render_pr_files_message("This pull request changes nothing".into(), cx);
    }

    // Rendering is the first moment the list exists to take the focus the tab
    // was opened for.
    if self.focus_pr_files_when_loaded {
      self.focus_pr_files_when_loaded = false;
      let list = self.pr_files_list.clone();
      list.update(cx, |list, cx| list.focus(window, cx));
    }

    div()
      .id("dock-panel-pr-files")
      .flex_1()
      .min_h_0()
      .px_1()
      .py_1()
      .child(List::new(&self.pr_files_list).w_full().min_h_0())
      .into_any_element()
  }

  fn render_pr_files_message(&self, message: SharedString, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    v_flex()
      .flex_1()
      .min_h_0()
      .items_center()
      .justify_center()
      .p_4()
      .child(
        div()
          .text_sm()
          .text_center()
          .text_color(theme.muted_foreground)
          .child(message),
      )
      .into_any_element()
  }

  fn render_empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    v_flex()
      .flex_1()
      .items_center()
      .justify_center()
      .gap_2()
      .child(
        Icon::new(UiIconName::CircleCheck)
          .size_4()
          .text_color(theme.muted_foreground),
      )
      .child(div().text_sm().text_color(theme.muted_foreground).child(
        if self.repo_root.is_some() {
          "No changes"
        } else {
          "No repository"
        },
      ))
      .into_any_element()
  }

  /// The dropdown listing the repo's checkouts; picking one pins the dock to
  /// it until the session takes back over.
  fn render_checkout_selector(&self, cx: &mut Context<Self>) -> AnyElement {
    let pinned = self.checkout_pinned;
    let displayed_branch = self
      .checkout_options
      .iter()
      .find(|option| option.is_displayed)
      .and_then(|option| option.branch.clone())
      .or_else(|| {
        self
          .branch_status
          .as_ref()
          .map(|status| SharedString::from(status.name.clone()))
      })
      .unwrap_or_else(|| SharedString::from("checkout"));
    let options = self.checkout_options.clone();
    let view = cx.entity();

    let mut selector = Button::new("dock-panel-checkout-selector")
      .debug_selector(|| DOCK_PANEL_CHECKOUT_SELECTOR_DEBUG_SELECTOR.to_string())
      .icon(UiIconName::GitBranch)
      // A child instead of a label: the button's own label never shrinks,
      // this one takes the room the header has and ellipsizes past it.
      .child(div().min_w(px(0.)).truncate().child(displayed_branch))
      .min_w(px(0.))
      .overflow_hidden()
      .flex_shrink(1.)
      .compact()
      .small()
      .tooltip(if pinned {
        "The dock is pinned to this checkout"
      } else {
        "Show another checkout"
      });
    if pinned {
      let view = cx.entity();
      selector = selector.child(
        div()
          .id("dock-panel-checkout-follow")
          .debug_selector(|| DOCK_PANEL_CHECKOUT_FOLLOW_DEBUG_SELECTOR.to_string())
          .flex_none()
          .cursor_pointer()
          .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            view.update(cx, |_, cx| cx.emit(DockPanelEvent::FollowSessionCheckout));
          })
          .child(Icon::new(IconName::Close).size_3()),
      );
    }
    let selector = if pinned {
      selector.primary()
    } else {
      selector.ghost()
    };
    let selector = selector.dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
      let menu = if pinned {
        let view = view.clone();
        menu.item(
          PopupMenuItem::new("Follow the session")
            .icon(UiIconName::Pin)
            .on_click(move |_, _, cx| {
              view.update(cx, |_, cx| cx.emit(DockPanelEvent::FollowSessionCheckout));
            }),
        )
      } else {
        menu
      };
      options.iter().fold(menu, |menu, option| {
        let view = view.clone();
        let path = option.path.clone();
        let label = option
          .branch
          .as_ref()
          .map(|branch| branch.to_string())
          .unwrap_or_else(|| {
            option
              .path
              .file_name()
              .map(|name| name.to_string_lossy().into_owned())
              .unwrap_or_else(|| option.path.display().to_string())
          });
        let mut item = PopupMenuItem::new(label).on_click(move |_, _, cx| {
          view.update(cx, |_, cx| {
            cx.emit(DockPanelEvent::PinCheckout { path: path.clone() });
          });
        });
        if option.is_displayed {
          item = item.icon(UiIconName::Check);
        }
        menu.item(item)
      })
    });

    selector.into_any_element()
  }
}

impl Render for DockPanel {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let header = h_flex()
      .h(px(40.))
      .min_h(px(40.))
      .max_h(px(40.))
      .flex_shrink_0()
      .items_center()
      .justify_between()
      .px_2()
      .border_b_1()
      .border_color(theme.border)
      .child(
        div()
          .flex_1()
          .min_w(px(0.0))
          .overflow_hidden()
          .truncate()
          .text_xs()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.muted_foreground)
          .child(match self.active_tab {
            DockPanelTab::Changes => "Changes",
            DockPanelTab::Review => "Review",
            DockPanelTab::Files => "Files",
            DockPanelTab::History => "History",
            DockPanelTab::PullRequest => "Pull request",
            DockPanelTab::Terminal => "Terminal",
          }),
      )
      .child(
        h_flex()
          .min_w(px(0.0))
          .items_center()
          .gap_1()
          .when(
            self.checkout_options.len() > 1 || self.checkout_pinned,
            |this| this.child(self.render_checkout_selector(cx)),
          )
          .when(self.active_tab != DockPanelTab::Terminal, |this| {
            this.child(
              Button::new("dock-panel-refresh")
                .debug_selector(|| DOCK_PANEL_REFRESH_DEBUG_SELECTOR.to_string())
                .icon(UiIconName::RefreshCw)
                .ghost()
                .compact()
                .small()
                .tooltip("Refresh")
                .loading(self.pr_refresh_pending > 0)
                .loading_icon(gpui_component::Icon::new(UiIconName::RefreshCw))
                .on_click(cx.listener(|this, _, _, cx| this.refresh_requested(cx))),
            )
          })
          .child(
            Button::new("dock-panel-zoom")
              .debug_selector(|| DOCK_PANEL_ZOOM_DEBUG_SELECTOR.to_string())
              .icon(if self.zoomed {
                UiIconName::Minimize2
              } else {
                UiIconName::Maximize2
              })
              .ghost()
              .compact()
              .small()
              .tooltip(if self.zoomed { "Restore" } else { "Expand" })
              .on_click(cx.listener(|_, _, _, cx| cx.emit(DockPanelEvent::ToggleZoom))),
          ),
      );

    let body = match self.active_tab {
      DockPanelTab::Files => self.render_files_tab(_window, cx),
      DockPanelTab::Changes => {
        if self.status_entries.is_empty() {
          self.render_empty_state(cx)
        } else {
          div()
            .id("dock-panel-file-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py_1()
            .px_1()
            .child(self.changes_list.clone())
            .into_any_element()
        }
      }
      DockPanelTab::PullRequest => self.render_pr_tab(_window, cx),
      DockPanelTab::Review => self.render_review_tab(),
      DockPanelTab::History => self.render_history_tab(),
      DockPanelTab::Terminal => self.render_terminal_tab(_window, cx),
    };

    let mut panel = v_flex()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.sidebar)
      .track_focus(&self.focus_handle)
      // One stop of the window, and its own order inside: the surface first,
      // then what the tab holds besides it.
      .tab_group()
      .key_context(crate::shortcuts::DOCK_PANEL_CONTEXT)
      .on_action(cx.listener(|this, _: &crate::CommitChanges, _, cx| this.commit(cx)))
      .child(header)
      .child(body);
    if self.active_tab == DockPanelTab::Changes {
      panel = panel.child(self.render_commit_zone(_window, cx));
    }
    panel
  }
}

impl Focusable for DockPanel {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use git::RepoStatusKind;
  use git2::Repository;
  use gpui::TestAppContext;
  use std::path::Path;
  use std::sync::Arc;
  use std::sync::atomic::{AtomicBool, Ordering};
  use ui::CommandPaletteCommandId;

  #[gpui::test]
  async fn a_poll_re_reads_the_working_tree_without_calling_github(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-poll-local");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |panel, _| {
      // As if the Files tab had been opened once already.
      panel.files_loaded = true;
      panel._files_task.take();
      panel.branch_pr = BranchPrState::Loading;
    });

    std::fs::write(repo.path.join("README.md"), "v2\n").expect("edit outside Reviu");
    panel.update(cx, |panel, cx| panel.poll(cx));
    await_refresh(&panel, cx).await;

    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.status_entries().len(), 1, "the poll saw the edit");
      assert!(
        matches!(panel.branch_pr, BranchPrState::Loading),
        "a poll asks GitHub nothing"
      );
      assert!(
        panel._files_task.is_none(),
        "a poll does not rebuild the file tree"
      );
    });

    // An explicit refresh still does both.
    panel.update(cx, |panel, cx| panel.refresh(cx));
    await_refresh(&panel, cx).await;
    panel.read_with(cx, |panel, _| {
      assert!(!matches!(panel.branch_pr, BranchPrState::Loading));
      assert!(panel._files_task.is_some());
    });
  }

  #[gpui::test]
  async fn a_poll_touches_the_history_only_when_its_tab_is_open(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-poll-history");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    // Open the history once, so it already knows the repository, then leave it.
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::History, window, cx)
    });
    cx.run_until_parked();
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::Changes, window, cx)
    });
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
      panel.history_list.update(cx, |list, _| {
        list._poll_task.take();
        list._history_task.take();
      })
    });

    panel.update(cx, |panel, cx| panel.poll(cx));
    await_refresh(&panel, cx).await;
    panel.read_with(cx, |panel, cx| {
      assert!(
        panel.history_list.read(cx)._poll_task.is_none(),
        "the history costs nothing while its tab is closed"
      );
    });

    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::History, window, cx)
    });
    cx.run_until_parked();

    panel.update(cx, |panel, cx| panel.poll(cx));
    await_refresh(&panel, cx).await;
    panel.read_with(cx, |panel, cx| {
      assert!(
        panel.history_list.read(cx)._poll_task.is_some(),
        "an open history tab follows the repository"
      );
    });
  }

  #[gpui::test]
  async fn an_unpublished_branch_is_offered_a_push_before_the_pull_request(
    cx: &mut TestAppContext,
  ) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-publish-and-create-pr");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature".to_string(),
    };
    // Opening the tab refreshes the lookup, so the state is set afterwards.
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::PullRequest, window, cx)
    });
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
      panel.branch_pr = BranchPrState::Missing(context.clone());
      panel.set_branch_status(
        Some(git::BranchStatus {
          name: "feature".to_string(),
          ahead: 1,
          behind: 0,
          has_upstream: false,
        }),
        cx,
      );
    });
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(DOCK_PANEL_PUBLISH_AND_CREATE_PR_DEBUG_SELECTOR)
        .is_some(),
      "a branch the remote never saw is published first"
    );
    assert!(
      cx.debug_bounds(DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR)
        .is_none(),
      "GitHub cannot open a pull request for it yet"
    );

    // The event carries the branch the form will target.
    let published = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let recorder = published.clone();
    cx.update(|_, cx| {
      cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
        if let DockPanelEvent::PublishBranchAndCreatePullRequest(context) = event {
          recorder.borrow_mut().push(context.branch.clone());
        }
      })
      .detach();
    });
    let button = cx
      .debug_bounds(DOCK_PANEL_PUBLISH_AND_CREATE_PR_DEBUG_SELECTOR)
      .expect("publish button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(published.borrow().as_slice(), ["feature".to_string()]);

    // Even called directly, the form refuses a branch the remote never saw.
    panel.update_in(cx, |panel, window, cx| {
      panel.create_branch_pull_request(window, cx)
    });
    cx.run_until_parked();
    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));

    // Once it has an upstream, the plain Create button comes back.
    panel.update(cx, |panel, cx| {
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
    cx.run_until_parked();
    assert!(
      cx.debug_bounds(DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR)
        .is_some(),
      "a published branch goes straight to the form"
    );
    assert!(
      cx.debug_bounds(DOCK_PANEL_PUBLISH_AND_CREATE_PR_DEBUG_SELECTOR)
        .is_none()
    );
  }

  #[gpui::test]
  async fn the_palette_mirrors_what_the_pull_request_tab_offers(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-pr-palette");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature".to_string(),
    };
    let published = git::BranchStatus {
      name: "feature".to_string(),
      ahead: 0,
      behind: 0,
      has_upstream: true,
    };

    let command_id = |panel: &DockPanel| {
      panel
        .branch_pull_request_command()
        .map(|command| (command.id, command.disabled_reason.is_some()))
    };

    // Without GitHub, or without a GitHub remote, the palette says nothing.
    panel.update(cx, |panel, cx| {
      panel.branch_pr = BranchPrState::NoAccess;
      panel.set_branch_status(Some(published.clone()), cx);
    });
    panel.read_with(cx, |panel, _| assert_eq!(command_id(panel), None));
    panel.update(cx, |panel, _| panel.branch_pr = BranchPrState::NoRemote);
    panel.read_with(cx, |panel, _| assert_eq!(command_id(panel), None));

    // While the lookup runs, the command shows up but cannot be run.
    panel.update(cx, |panel, _| panel.branch_pr = BranchPrState::Loading);
    panel.read_with(cx, |panel, _| {
      assert_eq!(
        command_id(panel),
        Some((CommandPaletteCommandId::CreatePullRequest, true))
      );
    });

    // No pull request on a published branch: the form is one keystroke away.
    panel.update(cx, |panel, _| {
      panel.branch_pr = BranchPrState::Missing(context.clone())
    });
    panel.read_with(cx, |panel, _| {
      assert_eq!(
        command_id(panel),
        Some((CommandPaletteCommandId::CreatePullRequest, false))
      );
    });

    // Unpublished: publishing is a push, it stays a deliberate click in the tab.
    panel.update(cx, |panel, cx| {
      panel.set_branch_status(
        Some(git::BranchStatus {
          name: "feature".to_string(),
          ahead: 1,
          behind: 0,
          has_upstream: false,
        }),
        cx,
      );
    });
    panel.read_with(cx, |panel, _| assert_eq!(command_id(panel), None));

    // An existing pull request: the palette opens that one.
    panel.update(cx, |panel, cx| {
      panel.set_branch_status(Some(published), cx);
      panel.branch_pr = BranchPrState::Found(context, Box::new(test_pull_request()));
    });
    panel.read_with(cx, |panel, _| {
      assert_eq!(
        command_id(panel),
        Some((CommandPaletteCommandId::OpenPullRequest, false))
      );
      let label = panel
        .branch_pull_request_command()
        .expect("command")
        .name
        .to_string();
      assert!(label.contains("42"), "the command names the pull request");
    });
  }

  #[gpui::test]
  async fn opening_the_files_tab_hands_the_keyboard_to_its_tree(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-files-keyboard");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    // The tab is asked for before its tree exists: the files load off the main
    // thread and the focus follows on the render that mounts them.
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::Files, window, cx)
    });
    let files = panel.update(cx, |panel, _| panel._files_task.take());
    if let Some(files) = files {
      files.await;
    }
    cx.run_until_parked();

    let before = panel.read_with(cx, |panel, cx| {
      panel.files_tree_state.read(cx).selected_index()
    });
    cx.simulate_keystrokes("down");
    let after = panel.read_with(cx, |panel, cx| {
      panel.files_tree_state.read(cx).selected_index()
    });
    assert_ne!(before, after, "the arrow keys reach the file tree");
  }

  #[gpui::test]
  async fn opening_the_review_tab_hands_the_keyboard_to_its_rows(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-review-keyboard");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    let comments =
      crate::review_list::review_panel_comments(&[crate::agent_review::LocalAgentReviewComment {
        id: 1,
        in_reply_to_id: None,
        path: PathBuf::from("a.txt"),
        line: 1,
        side: editor::ReviewCommentSide::Right,
        start_line: None,
        start_side: None,
        body: std::sync::Arc::from("look here"),
        original_start_line: Some(2),
        original_lines: Vec::new(),
        state: crate::agent_review::LocalAgentReviewCommentState::Draft,
      }]);
    panel.update(cx, |panel, cx| {
      panel.review_list.update(cx, |list, cx| {
        list.set_comments(crate::review_list::ReviewSection::Agent, comments, cx)
      });
    });
    cx.run_until_parked();

    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::Review, window, cx)
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("down");
    panel.read_with(cx, |panel, cx| {
      assert!(
        panel
          .review_list
          .read(cx)
          .keyboard_selected_row(cx)
          .is_some(),
        "the arrow keys reach the review rows"
      );
    });
  }

  #[gpui::test]
  async fn walking_the_file_tree_shows_files_and_enter_opens_them(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-files-intent");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::Files, window, cx)
    });
    let files = panel.update(cx, |panel, _| panel._files_task.take());
    if let Some(files) = files {
      files.await;
    }
    cx.run_until_parked();

    let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
        if let DockPanelEvent::OpenFile { path, intent } = event {
          seen.borrow_mut().push((path.clone(), *intent));
        }
      })
      .detach();
    });

    // The first down lands on the second row: the tree starts with nothing
    // selected and counts from there.
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(
      opened.borrow().as_slice(),
      &[(PathBuf::from("b.txt"), OpenIntent::Browse)],
      "walking the tree shows the file, it does not choose it"
    );

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
      opened.borrow().last(),
      Some(&(PathBuf::from("b.txt"), OpenIntent::Open)),
      "Enter chooses it"
    );
  }

  #[gpui::test]
  async fn walking_onto_a_folder_opens_no_editor(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-files-folder");
    commit_text_file(&repo.path, Path::new("one/a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("two/b.txt"), "v1\n", "second");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::Files, window, cx)
    });
    let files = panel.update(cx, |panel, _| panel._files_task.take());
    if let Some(files) = files {
      files.await;
    }
    cx.run_until_parked();

    let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
        if let DockPanelEvent::OpenFile { path, intent } = event {
          seen.borrow_mut().push((path.clone(), *intent));
        }
      })
      .detach();
    });

    // Both rows are folders, and the first down lands on the second of them.
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert!(
      opened.borrow().is_empty(),
      "a folder has no contents to show, got {:?}",
      opened.borrow()
    );

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(
      opened.borrow().is_empty(),
      "Enter on a folder unfolds it, it does not open an editor"
    );

    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(
      opened.borrow().as_slice(),
      &[(PathBuf::from("two/b.txt"), OpenIntent::Browse)],
      "the file inside it still shows"
    );
  }

  async fn await_refresh(panel: &Entity<DockPanel>, cx: &mut gpui::VisualTestContext) {
    let task = panel.update(cx, |panel, _| panel._refresh_task.take());
    if let Some(task) = task {
      task.await;
    }
    cx.run_until_parked();
  }

  fn add_dock_panel_window(
    repo_root: Option<PathBuf>,
    cx: &mut TestAppContext,
  ) -> (Entity<DockPanel>, &mut gpui::VisualTestContext) {
    cx.update(|cx| {
      // Painting a panel needs the theme, and some of these tests paint.
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
    let mut mounted: Option<Entity<DockPanel>> = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let panel = cx.new(|cx| DockPanel::new(repo_root.clone(), window, cx));
      mounted = Some(panel.clone());
      gpui_component::Root::new(panel, window, cx)
    });
    (mounted.expect("dock panel"), cx)
  }

  fn test_remote() -> git::GithubRemoteRepo {
    git::GithubRemoteRepo {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
    }
  }

  fn test_pull_request() -> GithubPullRequest {
    serde_json::from_value(serde_json::json!({
      "number": 42,
      "title": "Add widgets",
      "state": "open",
      "draft": false,
      "updated_at": "2026-08-13T00:00:00Z",
      "labels": [],
      "repository": { "owner": "acme", "repo": "widget" }
    }))
    .expect("build test pull request")
  }

  fn test_pull_request_numbered(number: u64) -> GithubPullRequest {
    let mut pull_request = test_pull_request();
    pull_request.number = number;
    pull_request
  }

  fn test_branch_context() -> GithubBranchContext {
    GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature".to_string(),
    }
  }

  fn a_comment_on(path: &str) -> GithubPullRequestReviewComment {
    crate::pull_request_review_comments::pending_comment_fixture(1, path, Some(2), "here")
  }

  fn pull_request_panel(
    cx: &mut TestAppContext,
    files: Vec<git::CommitChangedFile>,
  ) -> (Entity<DockPanel>, &mut gpui::VisualTestContext) {
    let (panel, cx) = add_dock_panel_window(Some(PathBuf::from("/repo")), cx);
    panel.update(cx, |panel, cx| {
      panel.active_tab = DockPanelTab::PullRequest;
      panel.branch_pr = BranchPrState::Found(
        GithubBranchContext {
          owner: "acme".to_string(),
          repo: "widget".to_string(),
          branch: "feature".to_string(),
        },
        Box::new(test_pull_request()),
      );
      panel.pr_range = Some(PullRequestRange {
        base: "b".repeat(40),
        head: "h".repeat(40),
        base_ref: "main".to_string(),
        head_ref: "feature".to_string(),
      });
      panel.set_pr_files(files, cx);
      panel.pr_files_loading = false;
      cx.notify();
    });
    cx.run_until_parked();
    (panel, cx)
  }

  fn changed_file(path: &str, kind: git::CommitFileChangeKind) -> git::CommitChangedFile {
    git::CommitChangedFile {
      path: PathBuf::from(path),
      old_path: None,
      kind,
    }
  }

  #[gpui::test]
  async fn the_checks_block_opens_on_demand_and_costs_one_line_closed(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(
      cx,
      vec![changed_file(
        "src/main.rs",
        git::CommitFileChangeKind::Modified,
      )],
    );
    panel.update(cx, |panel, cx| {
      panel.pr_checks = Some(crate::pull_request_checks::checks_summary_fixture());
      cx.notify();
    });
    cx.run_until_parked();

    // Closed: the rollup line only, and the file list keeps its room.
    let closed = cx
      .debug_bounds(DOCK_PANEL_PR_CHECKS_DEBUG_SELECTOR)
      .expect("checks rollup bounds");
    assert!(cx.debug_bounds("pr-file-src/main.rs").is_some());

    cx.simulate_click(closed.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| assert!(panel.pr_details_expanded));
    // The missing required context of the fixture becomes a row of its own.
    assert!(
      cx.debug_bounds("pr-file-src/main.rs").is_some(),
      "the file list stays reachable while the checks are open"
    );
  }

  #[gpui::test]
  async fn a_pull_request_without_checks_or_reviewers_still_offers_its_actions(
    cx: &mut TestAppContext,
  ) {
    let (_panel, cx) = pull_request_panel(cx, Vec::new());

    // Nothing to report is not nothing to do: reviewing and merging live here.
    assert!(
      cx.debug_bounds(DOCK_PANEL_PR_CHECKS_DEBUG_SELECTOR)
        .is_some()
    );
    assert!(
      cx.debug_bounds(DOCK_PANEL_PR_MERGE_DEBUG_SELECTOR)
        .is_none(),
      "the actions stay behind the closed block"
    );
  }

  #[gpui::test]
  async fn another_branch_leaves_none_of_the_previous_pull_request_behind(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(
      cx,
      vec![changed_file(
        "src/main.rs",
        git::CommitFileChangeKind::Modified,
      )],
    );
    panel.update(cx, |panel, cx| {
      panel.pr_checks = Some(crate::pull_request_checks::checks_summary_fixture());
      panel.pr_reviewers = vec![ReviewerRow {
        login: "ada".to_string(),
        avatar_url: None,
        status: ReviewerStatus::Approved,
        latest_message: None,
      }];
      panel.pr_selected_file = Some(PathBuf::from("src/main.rs"));
      panel.set_pull_request_review_comments(
        vec![
          crate::pull_request_review_comments::pending_comment_fixture(
            1,
            "src/main.rs",
            Some(12),
            "rename this",
          ),
        ],
        cx,
      );
      cx.notify();
    });
    cx.run_until_parked();

    panel.update(cx, |panel, cx| panel.reset_pull_request_details(cx));

    // Nothing of the previous pull request may read as this one's.
    panel.read_with(cx, |panel, _| {
      assert!(panel.pr_checks.is_none());
      assert!(panel.pr_range.is_none());
      assert!(panel.pr_files.is_empty());
      assert!(panel.pr_reviewers.is_empty());
      assert!(panel.pr_selected_file.is_none());
      assert!(panel.pr_files_error.is_none());
      assert!(!panel.pr_checks_loading);
      assert!(panel.pr_review_comments.is_empty());
    });
    panel.read_with(cx, |panel, cx| {
      assert!(
        panel
          .review_list
          .read(cx)
          .comments(ReviewSection::PullRequest)
          .is_empty()
      );
    });
  }

  #[gpui::test]
  async fn what_the_review_still_owes_github_shows_in_the_review_panel(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());

    panel.update(cx, |panel, cx| {
      let mut published = crate::pull_request_review_comments::pending_comment_fixture(
        2,
        "src/other.rs",
        Some(3),
        "already submitted",
      );
      published.is_pending = false;
      panel.set_pull_request_review_comments(
        vec![
          crate::pull_request_review_comments::pending_comment_fixture(
            1,
            "src/main.rs",
            Some(12),
            "rename this",
          ),
          published,
        ],
        cx,
      );
    });
    cx.run_until_parked();

    panel.read_with(cx, |panel, cx| {
      let rows = panel
        .review_list
        .read(cx)
        .comments(ReviewSection::PullRequest)
        .to_vec();
      // Only what is not submitted yet: the rest lives on GitHub already.
      assert_eq!(rows.len(), 1);
      assert_eq!(rows[0].excerpt, "rename this");
      assert_eq!(rows[0].line, 12);
      // The panel's own batch is untouched by the pull request's.
      assert!(
        panel
          .review_list
          .read(cx)
          .comments(ReviewSection::Agent)
          .is_empty()
      );
    });
  }

  #[gpui::test]
  async fn a_panel_with_no_github_says_what_it_would_be_for(cx: &mut TestAppContext) {
    let (panel, cx) = add_dock_panel_window(Some(PathBuf::from("/repo")), cx);
    panel.update(cx, |panel, cx| {
      panel.active_tab = DockPanelTab::PullRequest;
      panel.branch_pr = BranchPrState::NoAccess;
      cx.notify();
    });
    cx.run_until_parked();

    // Signed out: the invitation is to sign in.
    assert!(
      cx.debug_bounds("pro-promise-pull_request_panel").is_some(),
      "an empty panel is a wasted surface"
    );

    // Signed in without a subscription is a different pitch, same surface.
    cx.update(|_, cx| {
      AuthStateStore::set(cx, crate::auth_state::signed_in_without_subscription());
    });
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(cx.debug_bounds("pro-promise-pull_request_panel").is_some());
  }

  #[gpui::test]
  async fn comments_written_here_say_where_they_went(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    assert!(
      cx.debug_bounds(DOCK_PANEL_PR_PENDING_COMMENTS_DEBUG_SELECTOR)
        .is_none(),
      "nothing waiting, nothing to say"
    );

    panel.update(cx, |panel, cx| {
      panel.set_pull_request_review_comments(
        vec![
          crate::pull_request_review_comments::pending_comment_fixture(
            1,
            "src/main.rs",
            Some(12),
            "rename this",
          ),
        ],
        cx,
      );
    });
    cx.run_until_parked();

    let row = cx
      .debug_bounds(DOCK_PANEL_PR_PENDING_COMMENTS_DEBUG_SELECTOR)
      .expect("pending comments row");
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    // The comment landed in another tab: the row is the way there.
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.active_tab(), DockPanelTab::Review);
    });
  }

  #[gpui::test]
  async fn finishing_a_review_asks_for_the_decision(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.set_pull_request_review_comments(
        vec![
          crate::pull_request_review_comments::pending_comment_fixture(
            1,
            "src/main.rs",
            Some(12),
            "rename this",
          ),
        ],
        cx,
      );
    });
    cx.run_until_parked();

    let asked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observer = {
      let asked = asked.clone();
      cx.update(|_, cx| {
        cx.subscribe(&panel, move |_, event: &DockPanelEvent, _| {
          if matches!(event, DockPanelEvent::SubmitPullRequestReview) {
            asked.store(true, std::sync::atomic::Ordering::SeqCst);
          }
        })
      })
    };

    let review_list = panel.read_with(cx, |panel, _| panel.review_list.clone());
    review_list.update(cx, |_, cx| cx.emit(ReviewListEvent::SubmitReview));
    cx.run_until_parked();
    drop(observer);

    // Three choices and a paragraph: the panel column has room for neither, so
    // the host is asked for a dialog.
    assert!(asked.load(std::sync::atomic::Ordering::SeqCst));
  }

  #[gpui::test]
  async fn discarding_the_pending_review_reaches_the_host(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());

    let asked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observer = {
      let asked = asked.clone();
      cx.update(|_, cx| {
        cx.subscribe(&panel, move |_, event: &DockPanelEvent, _| {
          if matches!(event, DockPanelEvent::DiscardPullRequestReview) {
            asked.store(true, std::sync::atomic::Ordering::SeqCst);
          }
        })
      })
    };

    let review_list = panel.read_with(cx, |panel, _| panel.review_list.clone());
    review_list.update(cx, |_, cx| {
      cx.emit(ReviewListEvent::DiscardPullRequestReview)
    });
    cx.run_until_parked();
    drop(observer);

    assert!(asked.load(std::sync::atomic::Ordering::SeqCst));
  }

  #[gpui::test]
  async fn discarding_a_pull_request_review_asks_first(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.set_pull_request_review_comments(
        vec![
          crate::pull_request_review_comments::pending_comment_fixture(
            1,
            "src/main.rs",
            Some(12),
            "rename this",
          ),
        ],
        cx,
      );
    });
    cx.run_until_parked();

    panel.update_in(cx, |panel, window, cx| {
      panel.discard_pull_request_review(window, cx)
    });
    cx.run_until_parked();

    // The review lives on GitHub's servers: nothing is deleted unasked.
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
  }

  #[gpui::test]
  async fn nothing_pending_leaves_nothing_to_discard(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());

    panel.update_in(cx, |panel, window, cx| {
      panel.discard_pull_request_review(window, cx)
    });
    cx.run_until_parked();

    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
  }

  #[gpui::test]
  async fn approving_needs_no_pending_comment(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.pr_details_expanded = true;
      cx.notify();
    });
    cx.run_until_parked();

    // Nothing pending, so the Review panel shows no section: the pull request
    // block is where an approval starts.
    let button = cx
      .debug_bounds(DOCK_PANEL_PR_REVIEW_DEBUG_SELECTOR)
      .expect("review button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
  }

  #[gpui::test]
  async fn a_pull_request_comment_opens_the_range_on_its_line(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.set_pull_request_review_comments(
        vec![
          crate::pull_request_review_comments::pending_comment_fixture(
            1,
            "src/main.rs",
            Some(12),
            "rename this",
          ),
        ],
        cx,
      );
    });
    cx.run_until_parked();

    let opened = std::sync::Arc::new(std::sync::Mutex::new(
      None::<(String, PathBuf, Option<usize>)>,
    ));
    let observer = {
      let opened = opened.clone();
      cx.update(|_, cx| {
        cx.subscribe(&panel, move |_, event: &DockPanelEvent, _| {
          if let DockPanelEvent::OpenPullRequestFile {
            head_oid,
            path,
            line,
            ..
          } = event
          {
            *opened.lock().expect("lock") = Some((head_oid.clone(), path.clone(), *line));
          }
        })
      })
    };

    let review_list = panel.read_with(cx, |panel, _| panel.review_list.clone());
    review_list.update(cx, |_, cx| {
      cx.emit(ReviewListEvent::OpenComment {
        section: ReviewSection::PullRequest,
        path: PathBuf::from("src/main.rs"),
        line: 12,
        intent: OpenIntent::Open,
      });
    });
    cx.run_until_parked();
    drop(observer);

    // The range, not the working tree: that line may hold something else there.
    assert_eq!(
      opened.lock().expect("lock").clone(),
      Some(("h".repeat(40), PathBuf::from("src/main.rs"), Some(12)))
    );
  }

  #[gpui::test]
  async fn the_merge_button_waits_for_an_answer_before_offering_anything(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.pr_reviewers = vec![ReviewerRow {
        login: "ada".to_string(),
        avatar_url: None,
        status: ReviewerStatus::Approved,
        latest_message: None,
      }];
      panel.pr_details_expanded = true;
      cx.notify();
    });
    cx.run_until_parked();

    // No readiness yet: the row is there, and the button cannot be pressed.
    assert!(
      cx.debug_bounds(DOCK_PANEL_PR_MERGE_DEBUG_SELECTOR)
        .is_some()
    );
    panel.read_with(cx, |panel, _| {
      assert_eq!(
        merge_availability(panel.pr_merge_readiness.as_ref(), None),
        MergeAvailability::Unknown
      );
    });
  }

  #[gpui::test]
  async fn the_merge_row_stays_closed_with_the_block(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.pr_reviewers = vec![ReviewerRow {
        login: "ada".to_string(),
        avatar_url: None,
        status: ReviewerStatus::Approved,
        latest_message: None,
      }];
      cx.notify();
    });
    cx.run_until_parked();

    // Merging is not a one-click-away act from a collapsed block.
    assert!(
      cx.debug_bounds(DOCK_PANEL_PR_MERGE_DEBUG_SELECTOR)
        .is_none()
    );
  }

  fn ready_merge_readiness(
    methods: Vec<GithubPullRequestMergeMethod>,
  ) -> GithubPullRequestMergeReadiness {
    GithubPullRequestMergeReadiness {
      status: crate::api::GithubPullRequestMergeReadinessStatus::Ready,
      message: "This pull request is ready to merge.".to_string(),
      current_head_sha: "h".repeat(40),
      default_method: methods.first().copied(),
      available_methods: methods,
      can_merge_now: true,
      viewer_can_merge: true,
      mergeable_state: None,
      rebaseable: None,
      commit_defaults: None,
    }
  }

  #[gpui::test]
  async fn the_chosen_method_is_remembered_and_falls_back_when_forbidden(cx: &mut TestAppContext) {
    let db_path = std::env::temp_dir().join(format!(
      "reviu-dock-merge-method-{}.sqlite",
      std::process::id()
    ));
    let _ = std::fs::remove_file(&db_path);
    crate::config::ConfigStore::set_test_db_path(Some(db_path.clone()));

    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.pr_merge_readiness = Some(ready_merge_readiness(vec![
        GithubPullRequestMergeMethod::Squash,
        GithubPullRequestMergeMethod::Merge,
      ]));
      panel.sync_merge_method_with_readiness();
      panel.choose_merge_method(GithubPullRequestMergeMethod::Merge, cx);
    });
    panel.read_with(cx, |panel, _| {
      assert!(matches!(
        merge_availability(
          panel.pr_merge_readiness.as_ref(),
          panel.selected_merge_method
        ),
        MergeAvailability::Ready {
          method: GithubPullRequestMergeMethod::Merge,
          ..
        }
      ));
    });

    // A fresh readiness read starts with no selection: the store remembers.
    panel.update(cx, |panel, _| {
      panel.selected_merge_method = None;
      panel.sync_merge_method_with_readiness();
    });
    panel.read_with(cx, |panel, _| {
      assert_eq!(
        panel.selected_merge_method,
        Some(GithubPullRequestMergeMethod::Merge)
      );
    });

    // The repository stopped allowing it: the choice falls back to the default.
    panel.update(cx, |panel, _| {
      panel.pr_merge_readiness = Some(ready_merge_readiness(vec![
        GithubPullRequestMergeMethod::Squash,
      ]));
      panel.sync_merge_method_with_readiness();
    });
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.selected_merge_method, None);
      assert!(matches!(
        merge_availability(
          panel.pr_merge_readiness.as_ref(),
          panel.selected_merge_method
        ),
        MergeAvailability::Ready {
          method: GithubPullRequestMergeMethod::Squash,
          ..
        }
      ));
    });

    let _ = std::fs::remove_file(&db_path);
    crate::config::ConfigStore::set_test_db_path(None);
  }

  #[gpui::test]
  async fn only_the_allowed_methods_reach_the_selector(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.pr_details_expanded = true;
      panel.pr_merge_readiness = Some(ready_merge_readiness(vec![
        GithubPullRequestMergeMethod::Squash,
      ]));
      cx.notify();
    });
    cx.run_until_parked();

    // One allowed method: nothing to choose, so no chevron at all.
    assert!(
      cx.debug_bounds(DOCK_PANEL_PR_MERGE_METHOD_DEBUG_SELECTOR)
        .is_none()
    );

    panel.update(cx, |panel, cx| {
      panel.pr_merge_readiness = Some(ready_merge_readiness(vec![
        GithubPullRequestMergeMethod::Squash,
        GithubPullRequestMergeMethod::Rebase,
      ]));
      cx.notify();
    });
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(DOCK_PANEL_PR_MERGE_METHOD_DEBUG_SELECTOR)
        .is_some()
    );
  }

  fn submitted_review(login: &str, state: &str) -> GithubPullRequestReview {
    serde_json::from_value(serde_json::json!({
      "id": 7,
      "user": { "login": login, "avatar_url": null },
      "state": state,
      "submitted_at": "2026-08-26T10:00:00Z",
      "body": null,
      "html_url": "https://github.com/acme/widget/pull/42",
    }))
    .expect("build submitted review")
  }

  fn awaiting_row(login: &str) -> ReviewerRow {
    ReviewerRow {
      login: login.to_string(),
      avatar_url: None,
      status: ReviewerStatus::Awaiting,
      latest_message: None,
    }
  }

  #[gpui::test]
  async fn a_submitted_review_outlives_the_stale_reads_that_follow(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.pr_reviewers = vec![awaiting_row("octocat")];
      panel.note_submitted_review(submitted_review("octocat", "APPROVED"), cx);
    });
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.pr_reviewers[0].status, ReviewerStatus::Approved);
    });

    // GitHub answers the refetch with a read it has not caught up on: the
    // fetched Awaiting must not undo what its own mutation just answered.
    panel.update(cx, |panel, _| {
      panel.pr_reviewers = vec![awaiting_row("octocat")];
      panel.apply_submitted_review_overlay();
    });
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.pr_reviewers[0].status, ReviewerStatus::Approved);
    });

    // A fetch that carries the answer retires the overlay: the next Awaiting
    // is a genuine re-request, not staleness.
    panel.update(cx, |panel, _| {
      panel.pr_reviewers = vec![ReviewerRow {
        login: "octocat".to_string(),
        avatar_url: None,
        status: ReviewerStatus::Approved,
        latest_message: None,
      }];
      panel.apply_submitted_review_overlay();
      panel.pr_reviewers = vec![awaiting_row("octocat")];
      panel.apply_submitted_review_overlay();
    });
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.pr_reviewers[0].status, ReviewerStatus::Awaiting);
    });
  }

  #[gpui::test]
  async fn the_file_rows_learn_their_comment_counts_with_the_comments(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(
      cx,
      vec![changed_file(
        "src/main.rs",
        git::CommitFileChangeKind::Modified,
      )],
    );
    panel.update(cx, |panel, cx| {
      panel.set_pull_request_review_comments(
        vec![
          crate::pull_request_review_comments::pending_comment_fixture(
            1,
            "src/main.rs",
            Some(12),
            "rename this",
          ),
        ],
        cx,
      );
    });

    panel.read_with(cx, |panel, cx| {
      let counts = panel
        .pr_files_list
        .read(cx)
        .delegate()
        .comment_counts
        .clone();
      assert_eq!(
        counts.get(Path::new("src/main.rs")),
        Some(&crate::pull_request_review_comments::FileCommentCounts {
          total: 1,
          pending: 1
        })
      );
    });
  }

  #[gpui::test]
  async fn a_review_from_someone_not_requested_gets_their_row_at_once(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.pr_author_login = Some("author".to_string());
      panel.note_submitted_review(submitted_review("octocat", "CHANGES_REQUESTED"), cx);
    });
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.pr_reviewers.len(), 1);
      assert_eq!(panel.pr_reviewers[0].login, "octocat");
      assert_eq!(
        panel.pr_reviewers[0].status,
        ReviewerStatus::ChangesRequested
      );
    });
  }

  #[gpui::test]
  async fn the_author_commenting_on_their_own_pull_request_gets_no_row(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.pr_author_login = Some("octocat".to_string());
      panel.note_submitted_review(submitted_review("octocat", "COMMENTED"), cx);
    });
    panel.read_with(cx, |panel, _| {
      assert!(panel.pr_reviewers.is_empty());
      assert!(panel.submitted_review_overlay.is_none());
    });
  }

  #[gpui::test]
  async fn merging_asks_before_calling_github(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    let request = MergeRequest {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      number: 42,
      method: GithubPullRequestMergeMethod::Squash,
      head_sha: "h".repeat(40),
    };
    panel.update_in(cx, |panel, window, cx| {
      panel.confirm_merge_pull_request(request, window, cx)
    });
    cx.run_until_parked();

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
  }

  #[gpui::test]
  async fn the_check_counts_show_without_opening_the_block(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.pr_checks = Some(crate::pull_request_checks::checks_summary_fixture());
      cx.notify();
    });
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| {
      assert!(!panel.pr_details_expanded, "the block starts closed");
    });
    assert!(
      cx.debug_bounds(DOCK_PANEL_PR_CHECKS_COUNTS_DEBUG_SELECTOR)
        .is_some(),
      "the split of the checks reads without a click"
    );
  }

  #[gpui::test]
  async fn the_checks_say_they_are_loading_instead_of_vanishing(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.pr_checks = None;
      panel.pr_checks_loading = true;
      cx.notify();
    });
    cx.run_until_parked();

    // The block holds its line, so the file list below does not jump.
    assert!(
      cx.debug_bounds(DOCK_PANEL_PR_CHECKS_DEBUG_SELECTOR)
        .is_some()
    );
  }

  #[gpui::test]
  async fn reviewers_alone_are_enough_to_show_the_block(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());
    panel.update(cx, |panel, cx| {
      panel.pr_reviewers = vec![ReviewerRow {
        login: "ada".to_string(),
        avatar_url: None,
        status: ReviewerStatus::Awaiting,
        latest_message: None,
      }];
      cx.notify();
    });
    cx.run_until_parked();

    // No CI on this pull request, but there is still an answer to wait for.
    assert!(
      cx.debug_bounds(DOCK_PANEL_PR_CHECKS_DEBUG_SELECTOR)
        .is_some()
    );
  }

  #[gpui::test]
  async fn the_pull_request_panel_lists_what_the_branch_proposes(cx: &mut TestAppContext) {
    let (_panel, cx) = pull_request_panel(
      cx,
      vec![
        changed_file("src/main.rs", git::CommitFileChangeKind::Modified),
        changed_file("docs/gone.md", git::CommitFileChangeKind::Deleted),
      ],
    );

    // A deleted file has no working-tree copy, and it still belongs to the list.
    assert!(cx.debug_bounds("pr-file-src/main.rs").is_some());
    assert!(cx.debug_bounds("pr-file-docs/gone.md").is_some());
  }

  #[gpui::test]
  async fn clicking_a_pull_request_file_asks_the_host_to_open_the_range(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(
      cx,
      vec![changed_file(
        "src/main.rs",
        git::CommitFileChangeKind::Modified,
      )],
    );

    let opened = std::sync::Arc::new(std::sync::Mutex::new(None::<(String, String, PathBuf)>));
    let observer = {
      let opened = opened.clone();
      cx.update(|_, cx| {
        cx.subscribe(&panel, move |_, event: &DockPanelEvent, _| {
          if let DockPanelEvent::OpenPullRequestFile {
            base_oid,
            head_oid,
            path,
            ..
          } = event
          {
            *opened.lock().expect("lock") =
              Some((base_oid.clone(), head_oid.clone(), path.clone()));
          }
        })
      })
    };

    let row = cx
      .debug_bounds("pr-file-src/main.rs")
      .expect("file row bounds");
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    drop(observer);

    assert_eq!(
      opened.lock().expect("lock").clone(),
      Some(("b".repeat(40), "h".repeat(40), PathBuf::from("src/main.rs"))),
      "the row carries the range, so the centre can load the snapshot"
    );
  }

  #[gpui::test]
  async fn walking_the_pull_request_files_shows_them_and_enter_opens_them(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(
      cx,
      vec![
        changed_file("src/main.rs", git::CommitFileChangeKind::Modified),
        changed_file("docs/gone.md", git::CommitFileChangeKind::Deleted),
      ],
    );

    let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
        if let DockPanelEvent::OpenPullRequestFile { path, intent, .. } = event {
          seen.borrow_mut().push((path.clone(), *intent));
        }
      })
      .detach();
    });

    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::PullRequest, window, cx)
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(
      opened.borrow().as_slice(),
      &[(PathBuf::from("src/main.rs"), OpenIntent::Browse)],
      "walking the list shows the file it proposes"
    );

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
      opened.borrow().last(),
      Some(&(PathBuf::from("src/main.rs"), OpenIntent::Open)),
      "Enter chooses it"
    );
  }

  #[gpui::test]
  async fn a_link_takes_the_diff_to_the_review_comment_it_names(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(
      cx,
      vec![changed_file(
        "src/main.rs",
        git::CommitFileChangeKind::Modified,
      )],
    );
    panel.update(cx, |panel, cx| {
      panel.set_pull_request_review_comments_for_test(
        vec![
          crate::pull_request_review_comments::pending_comment_fixture(
            9,
            "src/main.rs",
            Some(12),
            "here",
          ),
        ],
        cx,
      );
    });
    cx.run_until_parked();

    let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
        if let DockPanelEvent::OpenPullRequestFile {
          path, line, intent, ..
        } = event
        {
          seen.borrow_mut().push((path.clone(), *line, *intent));
        }
      })
      .detach();
    });

    panel.update(cx, |panel, cx| panel.reveal_review_comment(9, cx));
    cx.run_until_parked();

    assert_eq!(
      opened.borrow().as_slice(),
      &[(PathBuf::from("src/main.rs"), Some(12), OpenIntent::Open)],
      "the link asked for the comment, so the diff opens on its lines"
    );
  }

  #[gpui::test]
  async fn switching_checkout_drops_the_previous_pull_requests_review(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(
      cx,
      vec![changed_file(
        "src/main.rs",
        git::CommitFileChangeKind::Modified,
      )],
    );
    panel.update(cx, |panel, cx| {
      // A fresh lookup: without the reset, the staleness window would happily
      // reuse it for the next checkout.
      panel.pr_fetched_at = Some(cx.background_executor().now());
      panel.set_pull_request_review_comments_for_test(
        vec![
          crate::pull_request_review_comments::pending_comment_fixture(
            9,
            "src/main.rs",
            Some(12),
            "from the repo we left",
          ),
        ],
        cx,
      );
    });
    cx.run_until_parked();

    // The same checkout again: nothing moves.
    panel.update(cx, |panel, cx| {
      panel.set_repo_root(Some(PathBuf::from("/repo")), cx)
    });
    panel.read_with(cx, |panel, cx| {
      assert_eq!(
        panel
          .review_list
          .read(cx)
          .comments(ReviewSection::PullRequest)
          .len(),
        1,
        "the same checkout keeps its review"
      );
    });

    panel.update(cx, |panel, cx| {
      panel.set_repo_root(Some(PathBuf::from("/other")), cx)
    });
    panel.read_with(cx, |panel, cx| {
      assert!(
        panel
          .review_list
          .read(cx)
          .comments(ReviewSection::PullRequest)
          .is_empty(),
        "the review of the checkout we left is gone at once"
      );
      assert!(
        matches!(panel.branch_pr, BranchPrState::Loading),
        "the old pull request no longer answers for the new checkout"
      );
      assert!(
        panel.pr_fetched_at.is_none(),
        "the next refresh may not reuse the old lookup"
      );
    });
  }

  #[gpui::test]
  async fn a_review_comment_not_read_yet_waits_for_the_load(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(
      cx,
      vec![changed_file(
        "src/main.rs",
        git::CommitFileChangeKind::Modified,
      )],
    );

    let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
        if let DockPanelEvent::OpenPullRequestFile { path, line, .. } = event {
          seen.borrow_mut().push((path.clone(), *line));
        }
      })
      .detach();
    });

    panel.update(cx, |panel, cx| panel.reveal_review_comment(9, cx));
    cx.run_until_parked();
    assert!(
      opened.borrow().is_empty(),
      "nothing to open while the comments are still coming"
    );

    panel.update(cx, |panel, cx| {
      panel.set_pull_request_review_comments_for_test(
        vec![
          crate::pull_request_review_comments::pending_comment_fixture(
            9,
            "src/main.rs",
            Some(4),
            "here",
          ),
        ],
        cx,
      );
    });
    cx.run_until_parked();

    assert_eq!(
      opened.borrow().as_slice(),
      &[(PathBuf::from("src/main.rs"), Some(4))],
      "the load that reads the comment answers the link"
    );
  }

  #[gpui::test]
  async fn the_pull_request_files_take_the_keyboard_when_they_land(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());

    // The tab is asked for while the files are still coming.
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::PullRequest, window, cx)
    });
    cx.run_until_parked();

    panel.update(cx, |panel, cx| {
      panel.set_pr_files(
        vec![changed_file(
          "src/main.rs",
          git::CommitFileChangeKind::Modified,
        )],
        cx,
      )
    });
    cx.run_until_parked();

    let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
        if let DockPanelEvent::OpenPullRequestFile { path, .. } = event {
          seen.borrow_mut().push(path.clone());
        }
      })
      .detach();
    });

    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(
      opened.borrow().as_slice(),
      &[PathBuf::from("src/main.rs")],
      "the list took the keyboard the tab was opened for"
    );
  }

  #[gpui::test]
  async fn tab_stays_in_the_terminal(cx: &mut TestAppContext) {
    cx.update(|cx| {
      gpui_component::init(cx);
      // The terminal's claim on tab lives with the app bindings.
      crate::shortcuts::install_workspace_shortcuts(cx);
    });
    let repo = TempRepo::init("dock-terminal-tab");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::Terminal, window, cx)
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    panel.update_in(cx, |panel, window, cx| {
      let terminal = panel.terminal_view.clone().expect("terminal");
      assert!(
        terminal.read(cx).focus_handle(cx).is_focused(window),
        "tab belongs to the shell, it must not walk the focus out"
      );
    });
  }

  #[gpui::test]
  async fn opening_the_terminal_hands_it_the_keyboard(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-terminal-focus");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::Terminal, window, cx)
    });
    cx.run_until_parked();

    panel.update_in(cx, |panel, window, cx| {
      let terminal = panel.terminal_view.clone().expect("terminal");
      assert!(
        terminal.read(cx).focus_handle(cx).is_focused(window),
        "the shell takes the keyboard, without a click"
      );
    });
  }

  #[test]
  fn branch_pr_state_requires_remote_and_branch() {
    let no_remote = branch_pr_state_for_lookup(None, Some("main".to_string()), |_| {
      panic!("fetch must not run without a remote")
    });
    assert!(matches!(no_remote, BranchPrState::NoRemote));

    let no_branch = branch_pr_state_for_lookup(Some(test_remote()), None, |_| {
      panic!("fetch must not run without a branch")
    });
    assert!(matches!(no_branch, BranchPrState::NoRemote));
  }

  #[test]
  fn branch_pr_state_maps_lookup_results() {
    let found = branch_pr_state_for_lookup(
      Some(test_remote()),
      Some("feature/x".to_string()),
      |context| {
        assert_eq!(context.owner, "acme");
        assert_eq!(context.repo, "widget");
        assert_eq!(context.branch, "feature/x");
        Ok(Some(test_pull_request()))
      },
    );
    match found {
      BranchPrState::Found(context, pull_request) => {
        assert_eq!(context.branch, "feature/x");
        assert_eq!(pull_request.number, 42);
      }
      other => panic!("expected Found, got {other:?}"),
    }

    let missing =
      branch_pr_state_for_lookup(Some(test_remote()), Some("feature/x".to_string()), |_| {
        Ok(None)
      });
    assert!(matches!(missing, BranchPrState::Missing(_)));

    // API errors degrade to Missing so the tab still offers Create against the context.
    let errored =
      branch_pr_state_for_lookup(Some(test_remote()), Some("feature/x".to_string()), |_| {
        Err(anyhow::anyhow!("network down"))
      });
    match errored {
      BranchPrState::Missing(context) => assert_eq!(context.branch, "feature/x"),
      other => panic!("expected Missing, got {other:?}"),
    }
  }

  #[gpui::test]
  async fn branch_pr_requires_github_access(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-pr-gate");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    cx.run_until_parked();

    // Default auth state is Unknown: no GitHub access, no lookup attempted.
    panel.read_with(cx, |panel, _| {
      assert!(matches!(panel.branch_pr, BranchPrState::NoAccess));
    });
  }

  #[gpui::test]
  async fn a_checkout_rereads_the_branch_pull_request(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-pr-branch-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    // A pull request read a moment ago, for the branch we are about to leave.
    panel.update(cx, |panel, cx| {
      panel.set_branch_pull_request_state(
        BranchPrState::Missing(GithubBranchContext {
          owner: "acme".to_string(),
          repo: "widget".to_string(),
          branch: "main".to_string(),
        }),
        cx,
      );
      panel.set_pull_request_lookup_for_test("main", cx.background_executor().now());
    });

    // Staying put reads nothing again: the staleness window still governs.
    panel.update(cx, |panel, cx| {
      panel.set_branch_status(
        Some(git::BranchStatus {
          name: "main".to_string(),
          ahead: 0,
          behind: 0,
          has_upstream: true,
        }),
        cx,
      );
    });
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
      assert!(
        matches!(panel.branch_pr, BranchPrState::Missing(_)),
        "the same branch keeps what was read for it"
      );
    });

    panel.update(cx, |panel, cx| {
      panel.set_branch_status(
        Some(git::BranchStatus {
          name: "feature".to_string(),
          ahead: 0,
          behind: 0,
          has_upstream: false,
        }),
        cx,
      );
    });
    cx.run_until_parked();

    // Without GitHub access the reread lands on NoAccess, which is enough to
    // show the checkout did not leave the previous branch's answer up.
    panel.read_with(cx, |panel, _| {
      assert!(
        matches!(panel.branch_pr, BranchPrState::NoAccess),
        "a checkout rereads instead of keeping the branch we left"
      );
      assert!(panel.pr_fetched_at.is_none());
    });
  }

  #[gpui::test]
  async fn refresh_lists_working_tree_changes(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-refresh");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.status_entries.len(), 1);
      assert_eq!(panel.status_entries[0].path, PathBuf::from("README.md"));
      assert_eq!(panel.status_entries[0].status, RepoStatusKind::Modified);
    });
  }

  #[gpui::test]
  async fn a_merge_in_progress_still_ends_with_a_commit(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-merge");
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
    let _ = git::merge_branch(&repo.path, &feature);
    std::fs::write(repo.path.join("a.txt"), "resolved\n").expect("resolve conflict");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| {
      assert!(panel.merge_in_progress());
      assert!(!panel.rebase_in_progress(), "a merge is not a rebase");
    });
    assert!(
      cx.debug_bounds(DOCK_PANEL_OPERATION_DEBUG_SELECTOR)
        .is_some(),
      "the panel says a merge is running"
    );

    // Unlike a rebase, the button still commits: that is how a merge ends.
    panel.update_in(cx, |panel, window, cx| {
      panel.set_commit_message("Merge branch 'feature'", window, cx)
    });
    let button = cx
      .debug_bounds(DOCK_PANEL_COMMIT_DEBUG_SELECTOR)
      .expect("commit button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    let commit = panel.update(cx, |panel, _| panel._commit_task.take());
    if let Some(task) = commit {
      task.await;
    }
    cx.run_until_parked();

    assert!(!git::is_merge_in_progress(&repo.path).expect("merge state"));
  }

  #[gpui::test]
  async fn a_branch_without_a_pull_request_offers_both_ways_to_open_one(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-create-pr");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.update(cx, |panel, cx| {
      panel.branch_pr = BranchPrState::Missing(GithubBranchContext {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
        branch: "feature".to_string(),
      });
      panel.active_tab = DockPanelTab::PullRequest;
      cx.notify();
    });
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR)
        .is_some(),
      "the dialog stays the default path"
    );
    assert!(
      cx.debug_bounds(DOCK_PANEL_COMPARE_DEBUG_SELECTOR).is_some(),
      "github.com covers what the dialog does not"
    );

    // A branch that already has a pull request offers neither.
    panel.update(cx, |panel, cx| {
      panel.branch_pr = BranchPrState::NoRemote;
      cx.notify();
    });
    cx.run_until_parked();
    assert!(
      cx.debug_bounds(DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR)
        .is_none()
    );
    assert!(cx.debug_bounds(DOCK_PANEL_COMPARE_DEBUG_SELECTOR).is_none());
  }

  #[gpui::test]
  async fn the_commit_menu_offers_what_the_repository_allows(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-commit-menu");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(DOCK_PANEL_COMMIT_MENU_DEBUG_SELECTOR)
        .is_some(),
      "the commit button carries its menu"
    );

    // Two commits and no branch handed over yet: amend and undo, no push.
    panel.read_with(cx, |panel, _| {
      let state = panel.repo_state("");
      assert!(state.allows(PaletteCommand::Amend));
      assert!(state.allows(PaletteCommand::UndoLastCommit));
      assert!(
        !state.allows(PaletteCommand::Push),
        "the panel knows no branch yet"
      );
    });

    // The host hands the branch over: publishing becomes possible.
    panel.update(cx, |panel, cx| {
      panel.set_branch_status(
        Some(git::BranchStatus {
          name: "main".to_string(),
          ahead: 0,
          behind: 0,
          has_upstream: false,
        }),
        cx,
      )
    });
    panel.read_with(cx, |panel, _| {
      assert!(
        panel.repo_state("").allows(PaletteCommand::Push),
        "an unpublished branch is pushed to publish it"
      );
    });

    // The menu asks the host to run, it never runs the command itself.
    let asked = Arc::new(AtomicBool::new(false));
    let observer = {
      let asked = asked.clone();
      cx.update(|_, cx| {
        cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
          if matches!(event, DockPanelEvent::RunCommand(CommitMenuCommand::Amend)) {
            asked.store(true, Ordering::SeqCst);
          }
        })
      })
    };
    panel.update(cx, |_, cx| {
      cx.emit(DockPanelEvent::RunCommand(CommitMenuCommand::Amend))
    });
    cx.run_until_parked();
    assert!(asked.load(Ordering::SeqCst));
    drop(observer);
  }

  #[gpui::test]
  async fn a_rebase_in_progress_replaces_the_commit_button(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-rebase");
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
    git::switch_branch(&repo.path, &feature).expect("switch to feature");
    let _ = git::rebase_branch(&repo.path, &base);

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| assert!(panel.rebase_in_progress()));
    assert!(
      cx.debug_bounds(DOCK_PANEL_OPERATION_DEBUG_SELECTOR)
        .is_some(),
      "the panel says a rebase is running"
    );

    // The button now continues the rebase; the host runs it.
    let asked = Arc::new(AtomicBool::new(false));
    let observer = {
      let asked = asked.clone();
      cx.update(|_, cx| {
        cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
          if matches!(event, DockPanelEvent::ContinueRebase) {
            asked.store(true, Ordering::SeqCst);
          }
        })
      })
    };

    // Conflicted: the button is there but does nothing yet.
    let button = cx
      .debug_bounds(DOCK_PANEL_COMMIT_DEBUG_SELECTOR)
      .expect("commit button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(!asked.load(Ordering::SeqCst));

    // Resolved and staged: the same button asks to continue.
    std::fs::write(repo.path.join("a.txt"), "resolved\n").expect("resolve conflict");
    git::stage_all(&repo.path).expect("stage the resolution");
    await_refresh(&panel, cx).await;
    panel.update(cx, |panel, cx| {
      panel.refresh(cx);
    });
    await_refresh(&panel, cx).await;
    let button = cx
      .debug_bounds(DOCK_PANEL_COMMIT_DEBUG_SELECTOR)
      .expect("commit button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(asked.load(Ordering::SeqCst));

    // Nothing was committed behind our back.
    assert!(git::is_rebase_in_progress(&repo.path).expect("rebase state"));
    drop(observer);
  }

  #[gpui::test]
  async fn commit_stages_and_commits_all_changes(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-commit");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.update_in(cx, |panel, window, cx| {
      panel.commit_input.update(cx, |input, cx| {
        input.set_value("feat: update readme", window, cx)
      });
    });
    // The shell refreshes its branch counters on this event.
    let committed = Arc::new(AtomicBool::new(false));
    let observer = {
      let committed = committed.clone();
      cx.update(|_, cx| {
        cx.subscribe(&panel, move |_, event: &DockPanelEvent, _| {
          if matches!(event, DockPanelEvent::Committed) {
            committed.store(true, Ordering::Relaxed);
          }
        })
      })
    };

    panel.update(cx, |panel, cx| panel.commit(cx));

    let commit_task = panel.update(cx, |panel, _| {
      panel._commit_task.take().expect("commit task")
    });
    commit_task.await;
    await_refresh(&panel, cx).await;
    drop(observer);
    assert!(
      committed.load(Ordering::Relaxed),
      "expected a Committed event"
    );

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(
      head.summary().expect("read summary"),
      Some("feat: update readme")
    );
    panel.read_with(cx, |panel, cx| {
      assert!(panel.status_entries.is_empty());
      assert!(panel.commit_input.read(cx).value().is_empty());
    });
  }

  #[gpui::test]
  async fn commit_requires_message_and_changes(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-commit-guards");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    // Clean tree: commit is a no-op even with a message.
    panel.update_in(cx, |panel, window, cx| {
      panel
        .commit_input
        .update(cx, |input, cx| input.set_value("feat: nothing", window, cx));
    });
    panel.update(cx, |panel, cx| panel.commit(cx));
    panel.read_with(cx, |panel, _| assert!(panel._commit_task.is_none()));

    // Dirty tree but empty message: also a no-op.
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    panel.update(cx, |panel, cx| panel.refresh(cx));
    await_refresh(&panel, cx).await;
    panel.update_in(cx, |panel, window, cx| {
      panel
        .commit_input
        .update(cx, |input, cx| input.set_value("   ", window, cx));
    });
    panel.update(cx, |panel, cx| panel.commit(cx));
    panel.read_with(cx, |panel, _| assert!(panel._commit_task.is_none()));
  }

  #[gpui::test]
  async fn the_terminal_starts_only_when_its_tab_is_opened(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-terminal-lazy");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.read_with(cx, |panel, _| {
      // No shell for someone who never opens the tab.
      assert!(panel.terminal_view.is_none());
      assert!(panel.active_tab != DockPanelTab::Terminal);
    });

    panel.update(cx, |panel, cx| {
      panel.active_tab = DockPanelTab::Terminal;
      panel.ensure_terminal(cx);
      cx.notify();
    });
    cx.run_until_parked();

    panel.read_with(cx, |panel, cx| {
      let terminal = panel.terminal_view.as_ref().expect("terminal view");
      assert_eq!(
        terminal.read(cx).working_directory(),
        Some(repo.path.as_path())
      );
    });
    assert!(
      cx.debug_bounds(DOCK_PANEL_TERMINAL_DEBUG_SELECTOR)
        .is_some(),
      "the terminal tab should be painted"
    );
  }

  #[gpui::test]
  async fn switching_repository_moves_a_running_terminal(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-terminal-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("dock-panel-terminal-switch-other");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.update(cx, |panel, cx| panel.ensure_terminal(cx));
    panel.update(cx, |panel, cx| {
      panel.set_repo_root(Some(other.path.clone()), cx)
    });
    cx.run_until_parked();

    panel.read_with(cx, |panel, cx| {
      let terminal = panel.terminal_view.as_ref().expect("terminal view");
      assert_eq!(
        terminal.read(cx).working_directory(),
        Some(other.path.as_path())
      );
    });
  }

  #[gpui::test]
  async fn the_history_tab_lists_the_commits_and_opens_one_of_their_files(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-history-tab");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    // The tabs live in the page's rail now; the panel exposes open_tab.
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::History, window, cx)
    });
    let history = panel.read_with(cx, |panel, _| panel.history_list.clone());
    let load = history.update(cx, |list, _| list._history_task.take());
    if let Some(task) = load {
      task.await;
    }
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.active_tab, DockPanelTab::History);
    });
    assert!(
      cx.debug_bounds(crate::history_list::HISTORY_LIST_DEBUG_SELECTOR)
        .is_some(),
      "the history tab shows the commit tree"
    );

    // The panel forwards the file of a commit to whoever hosts it.
    let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
        if let DockPanelEvent::OpenCommitFile {
          commit_oid, path, ..
        } = event
        {
          seen.borrow_mut().push((commit_oid.clone(), path.clone()));
        }
      })
      .detach();
    });

    let head = git::current_head_sha(&repo.path)
      .expect("head sha")
      .expect("head sha");
    history.update(cx, |list, cx| {
      list.open_commit_file(
        head.clone(),
        PathBuf::from("README.md"),
        OpenIntent::Open,
        cx,
      )
    });
    cx.run_until_parked();

    assert_eq!(
      opened.borrow().as_slice(),
      &[(head, PathBuf::from("README.md"))]
    );
  }

  #[gpui::test]
  async fn clicking_the_terminal_tab_opens_a_shell(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-terminal-click");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(DOCK_PANEL_REFRESH_DEBUG_SELECTOR).is_some(),
      "the refresh button belongs to the review tabs"
    );

    // The tabs live in the page's rail now; the panel exposes open_tab.
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::Terminal, window, cx)
    });
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.active_tab, DockPanelTab::Terminal);
      assert!(panel.terminal_view.is_some());
    });
    assert!(
      cx.debug_bounds(DOCK_PANEL_TERMINAL_DEBUG_SELECTOR)
        .is_some()
    );
    assert!(
      cx.debug_bounds(DOCK_PANEL_REFRESH_DEBUG_SELECTOR).is_none(),
      "nothing to refresh on the terminal tab"
    );
  }

  #[gpui::test]
  async fn reopening_the_terminal_tab_keeps_the_same_shell(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-terminal-reuse");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    let first = panel.update(cx, |panel, cx| {
      panel.ensure_terminal(cx);
      panel.terminal_view.clone().expect("terminal view")
    });
    let second = panel.update(cx, |panel, cx| {
      panel.ensure_terminal(cx);
      panel.terminal_view.clone().expect("terminal view")
    });

    // A second shell would leak a process on every visit to the tab.
    assert_eq!(first.entity_id(), second.entity_id());
  }

  #[gpui::test]
  async fn staging_from_the_changes_list_refreshes_the_panel(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let repo = TempRepo::init("dock-panel-changes-list");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    panel.read_with(cx, |panel, cx| {
      assert_eq!(panel.status_entries.len(), 1);
      assert_eq!(panel.status_entries[0].stage, RepoStage::Unstaged);
      // The panel feeds the shared list.
      assert_eq!(panel.changes_list.read(cx).entries().len(), 1);
    });

    let button = cx
      .debug_bounds("changes-stage-0-0")
      .expect("stage button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    let action = panel.update(cx, |panel, cx| {
      panel
        .changes_list
        .update(cx, |list, _| list._action_task.take().expect("stage task"))
    });
    action.await;
    cx.run_until_parked();
    await_refresh(&panel, cx).await;

    panel.read_with(cx, |panel, _| {
      assert!(
        panel
          .status_entries
          .iter()
          .all(|entry| entry.stage != RepoStage::Unstaged),
        "the panel should have refreshed after the staging action"
      );
    });
  }

  /// A pull request read a moment ago, with a comment of its own on screen.
  fn a_read_pull_request(
    cx: &mut TestAppContext,
  ) -> (Entity<DockPanel>, &mut gpui::VisualTestContext) {
    let (panel, cx) = pull_request_panel(
      cx,
      vec![changed_file(
        "src/main.rs",
        git::CommitFileChangeKind::Modified,
      )],
    );
    cx.update(|_, cx| {
      AuthStateStore::set(cx, crate::auth_state::signed_in_with_github_access());
    });
    panel.update(cx, |panel, cx| {
      panel.pr_fetched_at = Some(cx.background_executor().now());
      panel.set_pull_request_review_comments(vec![a_comment_on("src/main.rs")], cx);
    });
    cx.run_until_parked();
    (panel, cx)
  }

  /// The repository of these tests has no GitHub remote, so any read of the
  /// pull request loses it: what is still there is what was never reread.
  fn still_holds_its_pull_request(
    panel: &Entity<DockPanel>,
    cx: &mut gpui::VisualTestContext,
  ) -> bool {
    panel.read_with(cx, |panel, _| {
      matches!(panel.branch_pr, BranchPrState::Found(_, _))
        && panel.pull_request_review_comments().len() == 1
        && !panel.pr_files.is_empty()
        && !panel.pr_files_loading
    })
  }

  #[gpui::test]
  async fn reopening_the_pull_request_tab_shows_what_it_already_holds(cx: &mut TestAppContext) {
    let (panel, cx) = a_read_pull_request(cx);

    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::Changes, window, cx);
      panel.open_tab(DockPanelTab::PullRequest, window, cx);
    });
    cx.run_until_parked();

    assert!(
      still_holds_its_pull_request(&panel, cx),
      "a read a moment old is the read this open needed"
    );
  }

  #[gpui::test]
  async fn the_pull_request_tab_reads_again_once_its_read_is_old(cx: &mut TestAppContext) {
    let (panel, cx) = a_read_pull_request(cx);

    cx.executor()
      .advance_clock(std::time::Duration::from_secs(120));
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::Changes, window, cx);
      panel.open_tab(DockPanelTab::PullRequest, window, cx);
    });
    cx.run_until_parked();

    assert!(
      !still_holds_its_pull_request(&panel, cx),
      "an old read is worth spending a request on"
    );
  }

  #[gpui::test]
  async fn the_refresh_button_reads_however_recent_the_last_read_was(cx: &mut TestAppContext) {
    let (panel, cx) = a_read_pull_request(cx);

    panel.update(cx, |panel, cx| panel.refresh(cx));
    cx.run_until_parked();
    assert!(
      still_holds_its_pull_request(&panel, cx),
      "a save is not a reason to read GitHub again"
    );

    panel.update(cx, |panel, cx| panel.refresh_requested(cx));
    cx.run_until_parked();
    assert!(
      !still_holds_its_pull_request(&panel, cx),
      "being asked is a reason"
    );
  }

  #[gpui::test]
  async fn the_refresh_button_answers_the_click_until_the_read_lands(cx: &mut TestAppContext) {
    let (panel, cx) = a_read_pull_request(cx);

    panel.update(cx, |panel, cx| panel.refresh_requested(cx));
    panel.read_with(cx, |panel, _| {
      assert!(
        panel.pr_refresh_pending > 0,
        "the click has to show as taken"
      );
    });

    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
      assert_eq!(
        panel.pr_refresh_pending, 0,
        "the reads landed, the button rests"
      );
    });
  }

  #[gpui::test]
  async fn a_reread_of_the_same_pull_request_keeps_its_comments(cx: &mut TestAppContext) {
    let (panel, cx) = a_read_pull_request(cx);

    panel.update(cx, |panel, cx| {
      let same = BranchPrState::Found(
        test_branch_context(),
        Box::new(test_pull_request_numbered(42)),
      );
      panel.apply_branch_pull_request(same, cx);
    });
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.pull_request_review_comments().len(), 1);
    });

    panel.update(cx, |panel, cx| {
      let another = BranchPrState::Found(
        test_branch_context(),
        Box::new(test_pull_request_numbered(43)),
      );
      panel.apply_branch_pull_request(another, cx);
    });
    panel.read_with(cx, |panel, _| {
      assert!(
        panel.pull_request_review_comments().is_empty(),
        "the comments of #42 say nothing about #43"
      );
    });
  }

  #[gpui::test]
  async fn a_range_reread_without_conversation_keeps_its_comments(cx: &mut TestAppContext) {
    let (panel, cx) = a_read_pull_request(cx);

    panel.update(cx, |panel, cx| {
      panel.apply_pull_request_range_load(
        PullRequestRangeLoad {
          range: PullRequestRange {
            base: "b".repeat(40),
            head: "h".repeat(40),
            base_ref: "main".to_string(),
            head_ref: "feature".to_string(),
          },
          files: vec![changed_file(
            "src/main.rs",
            git::CommitFileChangeKind::Modified,
          )],
          conversation: None,
          author_login: "octocat".to_string(),
          pull_request_node_id: "PR_kwDOExample".to_string(),
        },
        cx,
      );
    });

    panel.read_with(cx, |panel, cx| {
      assert_eq!(panel.pull_request_review_comments().len(), 1);
      assert_eq!(panel.pr_node_id.as_deref(), Some("PR_kwDOExample"));
      assert_eq!(
        panel
          .pr_files_list
          .read(cx)
          .delegate()
          .comment_counts
          .get(Path::new("src/main.rs"))
          .map(|counts| counts.total),
        Some(1)
      );
    });
  }

  /// The Review tab used to pad its rows on top of the padding every list item
  /// already pays, so its comments stood taller than every other list of the
  /// dock.
  #[gpui::test]
  async fn a_review_row_is_as_tall_as_a_row_of_the_pull_request(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(
      cx,
      vec![changed_file(
        "src/main.rs",
        git::CommitFileChangeKind::Modified,
      )],
    );
    let pull_request_row = cx
      .debug_bounds("pr-file-src/main.rs")
      .expect("pull request file bounds");

    panel.update(cx, |panel, cx| {
      panel.active_tab = DockPanelTab::Review;
      panel.review_list.update(cx, |list, cx| {
        list.set_comments(
          crate::review_list::ReviewSection::Agent,
          vec![crate::review_list::ReviewPanelComment {
            id: 1,
            section: crate::review_list::ReviewSection::Agent,
            path: PathBuf::from("src/main.rs"),
            line: 2,
            line_label: "L2".to_string(),
            excerpt: "here".to_string(),
            status: crate::review_list::ReviewRowStatus::Draft,
            sendable: true,
          }],
          cx,
        );
      });
      cx.notify();
    });
    cx.run_until_parked();

    let review_row = cx
      .debug_bounds("review-comment-agent-1")
      .expect("review comment bounds");

    assert_eq!(review_row.size.height, pull_request_row.size.height);
  }

  /// The corbeille of a row used to call GitHub straight away, so a comment the
  /// list still believed pending could go out of a published review without a
  /// word. It asks the host now, like the diff does.
  #[gpui::test]
  async fn deleting_a_pull_request_comment_from_a_row_asks_the_host_first(cx: &mut TestAppContext) {
    let (panel, cx) = pull_request_panel(cx, Vec::new());

    let asked = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = asked.clone();
    cx.update(|_, cx| {
      cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
        if let DockPanelEvent::DeletePullRequestReviewComment { id } = event {
          seen.borrow_mut().push(*id);
        }
      })
      .detach();
    });

    panel.update(cx, |panel, cx| {
      panel.review_list.update(cx, |_, cx| {
        cx.emit(crate::review_list::ReviewListEvent::DeleteComment {
          section: crate::review_list::ReviewSection::PullRequest,
          id: 7,
        });
      });
    });
    cx.run_until_parked();

    assert_eq!(asked.borrow().as_slice(), &[7]);
  }
}
