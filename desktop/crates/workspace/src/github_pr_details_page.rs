use std::{
  collections::{BTreeSet, HashMap, HashSet},
  path::{Path, PathBuf},
  rc::Rc,
  sync::Arc,
};

use editor::{
  CloseFind, DiffViewMode, Editor, Find, HunkNavigationDirection, ReviewComment,
  ReviewCommentCancelHandler, ReviewCommentCodeReferencePreview, ReviewCommentCreateHandler,
  ReviewCommentCreateRequest, ReviewCommentDeleteHandler, ReviewCommentEditHandler,
  ReviewCommentImageUploadHandler, ReviewCommentLinkHandler, ReviewCommentMode,
  ReviewCommentPreviewRenderer, ReviewCommentResolveHandler, ReviewCommentSide,
  ReviewCommentSuggestionActionFactory,
};
use gfm_markdown_viewer::{
  GithubBlobLineReference, LinkAction, MarkdownRenderOptions, MarkdownRenderState,
  SuggestionActionContext, SuggestionContext, extract_github_blob_line_references, render_markdown,
};
use git::{
  DiffKind, DiffSet, FileDiff, GitStore, RepoStatusKind, compute_buffer_diff, create_stash,
  current_branch_status, current_github_remote_repo, current_head_sha, default_stash_message,
  is_merge_in_progress, is_rebase_in_progress, list_repo_head_files, list_repo_status,
  search_repo_head_contents, switch_to_branch_name, sync_current_branch_to_head,
};
use gpui::{
  Anchor, AnyElement, AnyWindowHandle, App, ClipboardItem, Context, Entity, ExternalPaths,
  FocusHandle, Focusable, Hsla, Image, InteractiveElement, Keystroke, MouseButton, ObjectFit,
  ParentElement, PathBuilder, Render, SharedString, Styled, Task, Window, canvas, deferred, div,
  img, point, prelude::*, px, white,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, Sizable as _, StyledExt,
  avatar::Avatar,
  button::{Button, ButtonVariant, ButtonVariants as _},
  clipboard::Clipboard,
  collapsible::Collapsible,
  h_flex,
  input::InputEvent,
  kbd::Kbd,
  menu::{DropdownMenu as _, PopupMenuItem},
  notification::Notification,
  radio::{Radio, RadioGroup},
  scroll::ScrollableElement,
  select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
  skeleton::Skeleton,
  spinner::Spinner,
  switch::Switch,
  tab::{Tab, TabBar},
  tag::Tag,
  text::TextView,
  tooltip::Tooltip,
  tree::{TreeItem, TreeState, tree},
  v_flex,
};
use sentry::protocol::{Map, Value};

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, ConfirmDialog, FILE_ICON_SIZE_PX, Input, InputState,
  MarkdownComposer, Popover, ScrollAxes, SearchFileEntry, SearchFileHandler, SelectableRowStyle,
  StatusThemeExt, Textarea, TextareaState, UiIconName, WindowExt,
  file_icon_path_for_name_with_theme, h_resizable, parse_github_url_action, resizable_panel,
  restrict_scroll_to_wheel_axis, scrollable_node, selectable_list_item,
};

use crate::diff_toolbar::{DiffToolbar, NavigationControl, SplitControl, ToggleControl};
use crate::diff_view_policy::{DiffViewInputs, effective_diff_view};
use crate::file_tree::{
  FileTreeBuildResult, build_path_tree_items, build_path_tree_items_with_expansion,
  expanded_folder_paths_for_changed_files,
};
use crate::file_view::render_file_title_with_status;
use crate::review_destination::{GithubReviewHandlers, ReviewDestination, configure_review};
use crate::svg_preview::SvgPreview;
use crate::{
  ShowCommandPalette, ShowFileSearch,
  active_local_repo::{ActiveLocalRepo, ActiveLocalRepoStore},
  api::{
    ApiClient, ApiError, GithubIssueReferenceTargetKind, GithubPullRequestChecksRollupState,
    GithubPullRequestChecksSummary, GithubPullRequestCommit, GithubPullRequestConversation,
    GithubPullRequestDetails, GithubPullRequestFile, GithubPullRequestFilterOptionLabel,
    GithubPullRequestFilterOptionUser, GithubPullRequestIssueComment, GithubPullRequestLabel,
    GithubPullRequestMergeMethod, GithubPullRequestMergeReadiness,
    GithubPullRequestMergeReadinessStatus, GithubPullRequestMergeResult, GithubPullRequestReview,
    GithubPullRequestReviewComment, GithubPullRequestReviewEvent, GithubPullRequestReviewState,
    GithubPullRequestState, GithubRepository, GithubRepositoryBranch,
  },
  auth_state::{AuthState, AuthStateStore},
  config::{AppSettings, ConfigStore},
  date_format::format_relative_time,
  file_preview::{
    FilePreviewKind, file_preview_kind, is_markdown_path, is_svg_path, raster_image_from_bytes,
    should_show_unsupported_binary_placeholder,
  },
  file_search_palette::open_file_search_palette as open_shared_file_search_palette,
  github_navigation::{
    SamePrGfmNavigation, open_commit_target, open_pr_target, open_profile_target, open_repo_target,
    same_pr_gfm_navigation, should_open_externally,
  },
  github_shared,
  navigation::NavigationHistory,
  sentry_context,
  session_page::SessionPageHandle,
  workspace::WorkspaceApi,
};

const SIDEBAR_DEFAULT_WIDTH: f32 = 400.0;
const SIDEBAR_MIN_WIDTH: f32 = 350.0;
const SIDEBAR_MAX_WIDTH: f32 = 1500.0;
const DIFF_HEADER_HEIGHT: f32 = 40.0;
const PR_CHANGE_COUNTER_DEBUG_SELECTOR: &str = "pr-change-counter";
const PR_WHITESPACE_TOGGLE_DEBUG_SELECTOR: &str = "pr-whitespace-toggle";
const PR_DIFF_VIEW_TOGGLE_DEBUG_SELECTOR: &str = "pr-diff-toggle";
const PR_PREVIEW_TOGGLE_DEBUG_SELECTOR: &str = "pr-markdown-preview";
const PR_TAB_OVERVIEW_IX: usize = 0;
const PR_TAB_CHANGES_IX: usize = 1;
const OVERVIEW_CHECKS_SCROLL_GUARD_ID: u64 = 0xCEDC_2025_C8EC_0001;

fn should_refresh_pr_overview_data(active_tab_ix: usize) -> bool {
  active_tab_ix == PR_TAB_OVERVIEW_IX
}

fn should_refresh_pr_changes_data(active_tab_ix: usize) -> bool {
  active_tab_ix == PR_TAB_CHANGES_IX
}

fn pr_refresh_in_progress(
  active_tab_ix: usize,
  merge_readiness_loading: bool,
  issue_comments_loading: bool,
  reviews_loading: bool,
  review_comments_loading: bool,
  commits_loading: bool,
  files_loading: bool,
  file_loading: bool,
  checks_loading: bool,
) -> bool {
  if merge_readiness_loading {
    return true;
  }

  if commits_loading || checks_loading {
    return true;
  }

  if should_refresh_pr_overview_data(active_tab_ix) {
    return issue_comments_loading || reviews_loading || review_comments_loading;
  }

  if should_refresh_pr_changes_data(active_tab_ix) {
    return review_comments_loading || files_loading || file_loading;
  }

  false
}

#[derive(Clone)]
struct PrTargetBranchSelectItem {
  branch: String,
  label: SharedString,
}

impl PrTargetBranchSelectItem {
  fn new(branch: String) -> Self {
    let label: SharedString = branch.clone().into();
    Self { branch, label }
  }
}

impl SelectItem for PrTargetBranchSelectItem {
  type Value = String;

  fn title(&self) -> SharedString {
    self.label.clone()
  }

  fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
    div()
      .w_full()
      .overflow_hidden()
      .text_ellipsis()
      .child(self.label.clone())
  }

  fn value(&self) -> &Self::Value {
    &self.branch
  }

  fn matches(&self, query: &str) -> bool {
    self.label.to_lowercase().contains(&query.to_lowercase())
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GithubPrRouteTarget {
  owner: String,
  repo: String,
  number: u64,
  tab_ix: usize,
}

fn github_pr_route_target_from_pathname(pathname: &str) -> Option<GithubPrRouteTarget> {
  let mut segments = pathname.trim_start_matches('/').split('/');
  if segments.next()? != "github" {
    return None;
  }
  let owner = segments.next()?.to_string();
  let repo = segments.next()?.to_string();
  if segments.next()? != "pull" {
    return None;
  }
  let number = segments.next()?.parse().ok()?;
  let tab_ix = match segments.next() {
    Some("changes") => PR_TAB_CHANGES_IX,
    Some(_) | None => PR_TAB_OVERVIEW_IX,
  };

  if segments.next().is_some() {
    return None;
  }

  Some(GithubPrRouteTarget {
    owner,
    repo,
    number,
    tab_ix,
  })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TabNavigationDirection {
  Previous,
  Next,
}
const PR_MERGE_POPOVER_WIDTH: f32 = 520.0;
const PR_MERGE_MESSAGE_INPUT_HEIGHT_PX: f32 = 100.0;
const PR_REVIEW_POPOVER_WIDTH: f32 = 500.0;
const PR_REVIEW_INPUT_HEIGHT_PX: f32 = 100.0;
const GITHUB_PR_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR: &str =
  "github-pr-markdown-preview-editor-pane";
const GITHUB_PR_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR: &str =
  "github-pr-markdown-preview-render-pane";
const GITHUB_PR_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR: &str = "github-pr-binary-preview-render-pane";

struct GithubPrStatusActionNotificationId;

#[derive(Clone)]
enum GithubPrBinaryPreview {
  RasterImage(Arc<Image>),
  UnsupportedBinary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubPrStatusAction {
  ReadyForReview,
  ConvertToDraft,
}

impl GithubPrStatusAction {
  fn button_label(self) -> &'static str {
    match self {
      GithubPrStatusAction::ReadyForReview => "Ready for review",
      GithubPrStatusAction::ConvertToDraft => "Convert to draft",
    }
  }

  fn success_breadcrumb(self) -> &'static str {
    match self {
      GithubPrStatusAction::ReadyForReview => "Mark pull request ready for review succeeded",
      GithubPrStatusAction::ConvertToDraft => "Convert pull request to draft succeeded",
    }
  }

  fn failure_breadcrumb(self) -> &'static str {
    match self {
      GithubPrStatusAction::ReadyForReview => "Mark pull request ready for review failed",
      GithubPrStatusAction::ConvertToDraft => "Convert pull request to draft failed",
    }
  }

  fn error_title(self) -> &'static str {
    match self {
      GithubPrStatusAction::ReadyForReview => "Ready for review failed",
      GithubPrStatusAction::ConvertToDraft => "Convert to draft failed",
    }
  }

  fn sentry_operation(self) -> &'static str {
    match self {
      GithubPrStatusAction::ReadyForReview => "github.pr.ready_for_review",
      GithubPrStatusAction::ConvertToDraft => "github.pr.convert_to_draft",
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SuggestedChangeCommitTarget {
  comment_id: u64,
  author_login: Arc<str>,
  path: Arc<str>,
  original_start_line: usize,
  original_lines: Vec<String>,
  suggested_lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubPrFileStatus {
  Added,
  Modified,
  Deleted,
  Renamed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewCommentNavigationDirection {
  Previous,
  Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommitNavigationDirection {
  Previous,
  Next,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GithubPrReviewDecision {
  #[default]
  Comment,
  Approve,
  RequestChanges,
}

#[derive(Clone, Debug)]
struct GithubPrFileDiff {
  path: SharedString,
  old_path: Option<SharedString>,
  status: GithubPrFileStatus,
}

#[derive(Clone, Debug)]
struct GithubPrLocalProjectFile {
  path: SharedString,
}

fn map_file_status(status: &str) -> GithubPrFileStatus {
  match status.trim().to_ascii_lowercase().as_str() {
    "added" => GithubPrFileStatus::Added,
    "removed" | "deleted" => GithubPrFileStatus::Deleted,
    "renamed" => GithubPrFileStatus::Renamed,
    _ => GithubPrFileStatus::Modified,
  }
}

fn files_from_api(files: Vec<GithubPullRequestFile>) -> Vec<Rc<GithubPrFileDiff>> {
  files
    .into_iter()
    .map(|file| {
      let status = map_file_status(file.status.as_str());
      let path = if file.filename.is_empty() {
        "unknown".to_string()
      } else {
        file.filename
      };
      let old_path = if status == GithubPrFileStatus::Renamed {
        file.previous_filename
      } else {
        None
      };

      Rc::new(GithubPrFileDiff {
        path: path.into(),
        old_path: old_path.map(Into::into),
        status,
      })
    })
    .collect()
}

fn commit_sort_timestamp(commit: &GithubPullRequestCommit) -> &str {
  commit
    .committed_at
    .as_deref()
    .or(commit.authored_at.as_deref())
    .unwrap_or("")
}

fn sort_commits_desc(commits: &mut [GithubPullRequestCommit]) {
  commits.sort_by(|a, b| commit_sort_timestamp(b).cmp(commit_sort_timestamp(a)));
}

fn positive_line_number(value: Option<i64>) -> Option<usize> {
  value.and_then(|value| (value > 0).then_some(value as usize))
}

fn resolve_review_comment_display_anchor(
  comment: &GithubPullRequestReviewComment,
  comments_by_id: &HashMap<u64, &GithubPullRequestReviewComment>,
) -> Option<(usize, ReviewCommentSide, Option<i64>)> {
  let mut resolved_line = comment.line.or(comment.start_line);
  let mut resolved_side = comment.side.as_deref().or(comment.start_side.as_deref());
  let mut current = Some(comment);
  for _ in 0..32 {
    if resolved_line.is_some() && resolved_side.is_some() {
      break;
    }

    let Some(parent_id) = current.and_then(|value| value.in_reply_to_id) else {
      break;
    };

    current = comments_by_id.get(&parent_id).copied();
    let Some(parent) = current else {
      break;
    };

    if resolved_line.is_none() {
      resolved_line = parent.line.or(parent.start_line);
    }
    if resolved_side.is_none() {
      resolved_side = parent.side.as_deref().or(parent.start_side.as_deref());
    }
  }

  let line = positive_line_number(resolved_line)?.saturating_sub(1);
  let side = match resolved_side {
    Some("LEFT") => ReviewCommentSide::Left,
    _ => ReviewCommentSide::Right,
  };

  Some((line, side, resolved_line))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewerStatus {
  Awaiting,
  Approved,
  Commented,
  ChangesRequested,
}

fn reviewer_status_for_login(
  reviews: &[GithubPullRequestReview],
  login: &str,
  requested_reviewers: &[GithubPullRequestFilterOptionUser],
) -> ReviewerStatus {
  let mut latest_approved: Option<&str> = None;
  let mut latest_changes: Option<&str> = None;
  let mut latest_comment: Option<&str> = None;

  for review in reviews {
    let Some(user) = review.user.as_ref() else {
      continue;
    };
    if !github_shared::logins_match_case_insensitive(user.login.as_str(), login) {
      continue;
    }
    let Some(submitted_at) = review.submitted_at.as_deref() else {
      continue;
    };
    match review.state {
      GithubPullRequestReviewState::Approved => {
        if latest_approved.is_none_or(|ts| submitted_at > ts) {
          latest_approved = Some(submitted_at);
        }
      }
      GithubPullRequestReviewState::RequestChanges => {
        if latest_changes.is_none_or(|ts| submitted_at > ts) {
          latest_changes = Some(submitted_at);
        }
      }
      GithubPullRequestReviewState::Commented => {
        if latest_comment.is_none_or(|ts| submitted_at > ts) {
          latest_comment = Some(submitted_at);
        }
      }
      GithubPullRequestReviewState::Dismissed | GithubPullRequestReviewState::Pending => {}
    }
  }

  let is_requested = requested_reviewers
    .iter()
    .any(|r| r.login.eq_ignore_ascii_case(login));

  let review_status = match (latest_approved, latest_changes, latest_comment) {
    (Some(approved), Some(changes), _) => {
      if approved > changes {
        ReviewerStatus::Approved
      } else {
        ReviewerStatus::ChangesRequested
      }
    }
    (_, Some(_), _) => ReviewerStatus::ChangesRequested,
    (Some(_), None, _) => ReviewerStatus::Approved,
    (None, None, Some(_)) => ReviewerStatus::Commented,
    _ => return ReviewerStatus::Awaiting,
  };

  // If the reviewer was re-requested after their last review, show as awaiting.
  if is_requested {
    return ReviewerStatus::Awaiting;
  }

  review_status
}

#[derive(Clone, Copy, Debug)]
struct OverviewChecksSummarySlice {
  value: f32,
  color: Hsla,
}

#[derive(Clone, Copy, Debug)]
struct OverviewChecksSummaryCap {
  left: f32,
  top: f32,
  size: f32,
  color: Hsla,
}

#[derive(Clone, Copy, Debug)]
struct OverviewChecksSummarySegment {
  start_angle: f32,
  end_angle: f32,
  color: Hsla,
}

const OVERVIEW_CHECKS_SUMMARY_RING_SIZE: f32 = 36.0;
const OVERVIEW_CHECKS_SUMMARY_RING_RADIUS: f32 = 14.0;
const OVERVIEW_CHECKS_SUMMARY_RING_STROKE_WIDTH: f32 = 5.0;
const OVERVIEW_CHECKS_SUMMARY_RING_GAP_ANGLE: f32 = 0.5;

#[derive(Clone, Debug, PartialEq, Eq)]
enum OverviewPrAlertKind {
  Conflicts,
  OutOfDate,
  Blocked,
}

#[derive(Clone, Debug)]
struct OverviewReviewStatusInfo {
  title: &'static str,
  message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OverviewConflictsKind {
  NoConflicts,
  Conflicts,
  OutOfDate,
  Merged,
}

#[derive(Clone, Debug)]
struct OverviewConflictsInfo {
  kind: OverviewConflictsKind,
  title: &'static str,
  message: String,
}

fn labels_contains(labels: &[GithubPullRequestLabel], candidate: &str) -> bool {
  labels
    .iter()
    .any(|label| label.name.eq_ignore_ascii_case(candidate))
}

fn remove_label(labels: &mut Vec<GithubPullRequestLabel>, name: &str) {
  labels.retain(|label| !label.name.eq_ignore_ascii_case(name));
}

fn file_for_review_comment_path(
  file_lookup: &HashMap<String, Rc<GithubPrFileDiff>>,
  path: &str,
) -> Option<Rc<GithubPrFileDiff>> {
  if let Some(file) = file_lookup.get(path) {
    return Some(file.clone());
  }

  file_lookup
    .values()
    .find(|file| {
      file
        .old_path
        .as_ref()
        .is_some_and(|old_path| old_path.as_ref() == path)
    })
    .cloned()
}

#[derive(Clone, Debug, Default)]
struct GithubPrFileContents {
  base: Option<String>,
  head: Option<String>,
}

#[derive(Default)]
struct GithubPrTreeSearchResult {
  matches: HashSet<String>,
  updated_file_contents: HashMap<String, GithubPrFileContents>,
  error: Option<String>,
}

#[derive(Clone, Debug)]
struct GithubPrDiffRefs {
  base_owner: String,
  base_repo: String,
  base_sha: String,
  head_owner: String,
  head_repo: String,
  head_sha: String,
}

fn build_tree_items(files: &[Rc<GithubPrFileDiff>]) -> FileTreeBuildResult<GithubPrFileDiff> {
  build_path_tree_items(files, |file| file.path.as_ref())
}

pub struct GithubPrDetailsPage {
  focus_handle: FocusHandle,
  window_handle: AnyWindowHandle,
  api: ApiClient,
  details_task: Option<Task<()>>,
  files_task: Option<Task<()>>,
  files_loading: bool,
  files_error: Option<SharedString>,
  files_request_generation: u64,
  commits_task: Option<Task<()>>,
  commits_loading: bool,
  commits_error: Option<SharedString>,
  commits: Vec<GithubPullRequestCommit>,
  commit_lookup: HashMap<String, GithubPullRequestCommit>,
  selected_commit_sha: Option<String>,
  checks_task: Option<Task<()>>,
  checks_loading: bool,
  checks_error: Option<SharedString>,
  checks: Option<GithubPullRequestChecksSummary>,
  merge_readiness_task: Option<Task<()>>,
  merge_readiness_loading: bool,
  merge_readiness_error: Option<SharedString>,
  merge_readiness: Option<GithubPullRequestMergeReadiness>,
  merge_popover_open: bool,
  merge_method_focus_handle: FocusHandle,
  merge_form_reset_pending: bool,
  merge_method: GithubPullRequestMergeMethod,
  merge_commit_title_input: Entity<InputState>,
  merge_commit_message_input: Entity<TextareaState>,
  merge_submit_task: Option<Task<()>>,
  merge_submit_loading: bool,
  merge_submit_error: Option<SharedString>,
  status_action_task: Option<Task<()>>,
  status_action_loading: bool,
  update_branch_task: Option<Task<()>>,
  update_branch_loading: bool,
  target_branch_select: Entity<SelectState<SearchableVec<PrTargetBranchSelectItem>>>,
  target_branch_task: Option<Task<()>>,
  target_branch_loading: bool,
  target_branch_error: Option<SharedString>,
  target_branch_request_generation: u64,
  target_branch_update_task: Option<Task<()>>,
  target_branch_update_loading: bool,
  target_branch_update_error: Option<SharedString>,
  review_input: Entity<TextareaState>,
  review_decision: GithubPrReviewDecision,
  review_popover_open: bool,
  review_form_reset_pending: bool,
  review_preview_open: bool,
  submit_review_task: Option<Task<()>>,
  submit_review_loading: bool,
  submit_review_error: Option<SharedString>,
  conversation_task: Option<Task<()>>,
  reaction_task: Option<Task<()>>,
  reaction_error: Option<(String, SharedString)>,
  issue_comments_task: Option<Task<()>>,
  issue_comments_loading: bool,
  issue_comments_error: Option<SharedString>,
  issue_comments: Vec<GithubPullRequestIssueComment>,
  review_people_options_task: Option<Task<()>>,
  review_people_options_loading: bool,
  review_people_options_error: Option<SharedString>,
  review_people_options: Vec<GithubPullRequestFilterOptionUser>,
  label_options: Vec<GithubPullRequestFilterOptionLabel>,
  label_options_loading: bool,
  label_options_error: Option<SharedString>,
  assignee_input: Entity<InputState>,
  requested_reviewer_input: Entity<InputState>,
  label_input: Entity<InputState>,
  assignee_popover_open: bool,
  reviewer_popover_open: bool,
  label_popover_open: bool,
  people_mutation_task: Option<Task<()>>,
  people_mutation_loading: bool,
  people_mutation_error: Option<SharedString>,
  label_mutation_task: Option<Task<()>>,
  label_mutation_loading: bool,
  label_mutation_error: Option<SharedString>,
  reviews_task: Option<Task<()>>,
  reviews_loading: bool,
  reviews_error: Option<SharedString>,
  reviews: Vec<GithubPullRequestReview>,
  review_comments_task: Option<Task<()>>,
  review_comments_loading: bool,
  review_comments_error: Option<SharedString>,
  review_comments: Vec<GithubPullRequestReviewComment>,
  // Viewer's unsubmitted pending review (GraphQL node ids), when one exists on this PR.
  pending_review_id: Option<String>,
  pending_review_pull_request_id: Option<String>,
  suggested_change_commit_target: Option<SuggestedChangeCommitTarget>,
  suggested_change_commit_title_input: Entity<InputState>,
  suggested_change_commit_message_input: Entity<TextareaState>,
  suggested_change_include_co_author: bool,
  suggested_change_commit_loading: bool,
  suggested_change_commit_error: Option<SharedString>,
  suggested_change_commit_task: Option<Task<()>>,
  stale_suggested_change_comment_ids: HashSet<u64>,
  resolve_thread_in_flight: HashSet<String>,
  resolve_thread_tasks: HashMap<String, Task<()>>,
  resolve_thread_errors: HashMap<String, SharedString>,
  expanded_resolved_threads: HashSet<u64>,
  selected_file_review_comment_ids: Vec<u64>,
  active_review_comment_id: Option<u64>,
  review_comment_handlers_enabled: bool,
  description_code_reference_requests: Vec<GithubBlobLineReference>,
  review_comment_code_reference_cache: HashMap<String, Option<ReviewCommentCodeReferencePreview>>,
  review_comment_code_reference_tasks: HashMap<String, Task<()>>,
  pending_review_comment_link_comment_id: Option<u64>,
  pr_description_editing: bool,
  pr_description_initial_body: Option<String>,
  pr_description_submitting: bool,
  pr_description_error: Option<SharedString>,
  file_loading: bool,
  file_error: Option<SharedString>,
  tree_state: Entity<TreeState>,
  tree_search_input: Entity<InputState>,
  tree_search_query: String,
  tree_search_matches: Option<HashSet<String>>,
  tree_search_task: Option<Task<()>>,
  tree_search_loading: bool,
  tree_search_error: Option<SharedString>,
  tree_search_generation: u64,
  tree_search_reset_pending: bool,
  show_local_project_files: bool,
  hide_whitespace: bool,
  saved_pr_selected_tree_id: Option<String>,
  file_lookup: HashMap<String, Rc<GithubPrFileDiff>>,
  resolved_local_repo: Option<ActiveLocalRepo>,
  resolved_local_repo_scan_complete: bool,
  resolved_local_repo_task: Option<Task<()>>,
  resolved_local_repo_generation: u64,
  local_project_lookup: HashMap<String, Rc<GithubPrLocalProjectFile>>,
  local_project_loaded_repo_root: Option<PathBuf>,
  local_project_tree_loading: bool,
  local_project_tree_error: Option<SharedString>,
  local_project_files_task: Option<Task<()>>,
  local_branch_switch_task: Option<Task<()>>,
  local_branch_switch_loading: bool,
  local_branch_switch_error: Option<SharedString>,
  local_project_update_task: Option<Task<()>>,
  local_project_update_loading: bool,
  local_project_update_error: Option<SharedString>,
  local_project_open_file_task: Option<Task<()>>,
  local_project_open_file_generation: u64,
  file_contents: HashMap<String, GithubPrFileContents>,
  file_content_tasks: HashMap<String, Task<()>>,
  file_asset_previews: HashMap<String, GithubPrBinaryPreview>,
  file_asset_tasks: HashMap<String, Task<()>>,
  selected_file: Option<Rc<GithubPrFileDiff>>,
  selected_tree_id: Option<String>,
  selected_local_project_file: Option<Rc<GithubPrLocalProjectFile>>,
  selected_local_project_tree_id: Option<String>,
  diff_editor: Entity<Editor>,
  diff_view: DiffViewMode,
  show_markdown_preview: bool,
  binary_preview: Option<GithubPrBinaryPreview>,
  description_markdown_state: MarkdownRenderState,
  syntax_highlight_cache: Arc<gfm_markdown_viewer::SyntaxHighlightCache>,
  overview_checks_open: bool,
  overview_checks_scroll_handle: gpui::ScrollHandle,
  svg_preview: Entity<SvgPreview>,
  active_tab_ix: usize,
  current_pr_context: Option<CurrentPrContext>,
  pull_request: Option<GithubPullRequestDetails>,
  error: Option<SharedString>,
}

#[derive(Clone, Debug)]
struct CurrentPrContext {
  owner: String,
  repo: String,
  number: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GithubPrLocalProjectAvailability {
  Hidden,
  NeedsBranchSwitch {
    repo_root: PathBuf,
    current_branch: Option<String>,
    has_uncommitted_changes: bool,
  },
  Ready {
    repo_root: PathBuf,
  },
  NeedsUpdate {
    repo_root: PathBuf,
  },
  Dirty {
    repo_root: PathBuf,
  },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GithubPrLocalProjectPostAction {
  EnsurePrHeadThenMergeBaseInWorkspace { base_branch_name: String },
  MergeBaseInWorkspace { base_branch_name: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GithubPrOpenTarget {
  open_changes_tab: bool,
  review_comment_id: Option<u64>,
}

impl GithubPrOpenTarget {
  pub(crate) fn new(open_changes_tab: bool, review_comment_id: Option<u64>) -> Self {
    Self {
      open_changes_tab,
      review_comment_id,
    }
  }

  fn tab_ix(self) -> usize {
    if self.open_changes_tab || self.review_comment_id.is_some() {
      PR_TAB_CHANGES_IX
    } else {
      PR_TAB_OVERVIEW_IX
    }
  }
}

#[derive(Clone, Default)]
pub struct GithubPrDetailsPageHandle {
  page: Option<gpui::WeakEntity<GithubPrDetailsPage>>,
}

impl gpui::Global for GithubPrDetailsPageHandle {}

impl GithubPrDetailsPageHandle {
  pub fn register(cx: &mut Context<GithubPrDetailsPage>) {
    cx.set_global(Self {
      page: Some(cx.entity().downgrade()),
    });
  }

  pub fn show_with_open_target(
    owner: SharedString,
    repo: SharedString,
    number: u64,
    open_changes_tab: bool,
    review_comment_id: Option<u64>,
    cx: &mut App,
  ) {
    Self::show_with_open_target_inner(
      owner,
      repo,
      number,
      GithubPrOpenTarget::new(open_changes_tab, review_comment_id),
      cx,
    );
  }

  pub fn refresh(cx: &mut App) {
    let Some(weak) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.page.clone())
    else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.refresh_current_page(cx));
  }

  pub fn is_refreshing(cx: &App) -> bool {
    let Some(weak) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.page.clone())
    else {
      return false;
    };

    weak
      .read_with(cx, |this, _cx| {
        pr_refresh_in_progress(
          this.active_tab_ix,
          this.merge_readiness_loading,
          this.issue_comments_loading,
          this.reviews_loading,
          this.review_comments_loading,
          this.commits_loading,
          this.files_loading,
          this.file_loading,
          this.checks_loading,
        )
      })
      .unwrap_or(false)
  }

  fn show_with_open_target_inner(
    owner: SharedString,
    repo: SharedString,
    number: u64,
    open_target: GithubPrOpenTarget,
    cx: &mut App,
  ) {
    // Opening a pull request is one keystroke away from anywhere; the page it
    // needs may not be mounted, which is not a crash.
    let Some(weak) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.page.clone())
    else {
      return;
    };

    let owner_string = owner.to_string();
    let repo_string = repo.to_string();
    let window_handle = weak.read_with(cx, |this, _| this.window_handle).ok();
    let _ = weak.update(cx, |this, cx| {
      this.load_pull_request(owner_string, repo_string, number, open_target, cx);
    });

    if let Some(handle) = window_handle {
      let _ = cx.update_window(handle, |_, window, cx| {
        window.close_sheet(cx);
      });
    }

    NavigationHistory::navigate(crate::navigation::build_pr_path(&owner, &repo, number), cx);
  }
}

fn local_repo_snapshot(
  repo_root: &Path,
  current_branch_override: Option<&str>,
) -> Option<ActiveLocalRepo> {
  let github_remote = current_github_remote_repo(repo_root).ok().flatten();
  let current_branch = current_branch_override.map(str::to_string).or_else(|| {
    current_branch_status(repo_root)
      .ok()
      .map(|status| status.name)
  });
  let head_sha = current_head_sha(repo_root).ok().flatten();
  let has_uncommitted_changes = list_repo_status(repo_root)
    .map(|entries| !entries.is_empty())
    .unwrap_or(false);

  Some(ActiveLocalRepo {
    repo_root: repo_root.to_path_buf(),
    github_owner: github_remote.as_ref().map(|remote| remote.owner.clone()),
    github_repo: github_remote.as_ref().map(|remote| remote.repo.clone()),
    current_branch,
    head_sha,
    has_uncommitted_changes,
  })
}

#[cfg(test)]
pub(crate) mod test_support;

mod actions;
mod changes;
mod local_project;
mod merge;
mod people;
mod render_changes;
mod render_overview;
mod review;

impl GithubPrDetailsPage {
  fn sync_route(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !cx.has_global::<gpui_router::RouterState>() {
      return;
    }

    let Some(current_context) = self.current_pr_context.as_ref() else {
      return;
    };

    let pathname = NavigationHistory::current_pathname(cx);
    let Some(route_target) = github_pr_route_target_from_pathname(&pathname) else {
      return;
    };

    let same_pr = current_context
      .owner
      .eq_ignore_ascii_case(&route_target.owner)
      && current_context
        .repo
        .eq_ignore_ascii_case(&route_target.repo)
      && current_context.number == route_target.number;

    if !same_pr {
      self.load_pull_request(
        route_target.owner,
        route_target.repo,
        route_target.number,
        GithubPrOpenTarget::new(route_target.tab_ix == PR_TAB_CHANGES_IX, None),
        cx,
      );
      return;
    }

    if self.active_tab_ix != route_target.tab_ix {
      self.set_active_tab_inner(route_target.tab_ix, window, cx, false);
    }
  }

  fn sentry_pr_data(&self) -> Map<String, Value> {
    let mut data = Map::new();
    if let Some(context) = self.current_pr_context.as_ref() {
      data.insert("owner".into(), context.owner.clone().into());
      data.insert("repo".into(), context.repo.clone().into());
      data.insert("number".into(), context.number.into());
    }
    if let Some(selected_file) = self.selected_file.as_ref() {
      data.insert(
        "selected_file".into(),
        selected_file.path.to_string().into(),
      );
    }
    if let Some(selected_commit_sha) = self.selected_commit_sha.as_ref() {
      data.insert(
        "selected_commit_sha".into(),
        selected_commit_sha.clone().into(),
      );
    }
    data.insert("active_tab".into(), self.active_tab_ix.into());
    data
  }

  fn add_pr_breadcrumb(&self, message: &str, mut data: Map<String, Value>) {
    let base = self.sentry_pr_data();
    for (key, value) in base {
      data.entry(key).or_insert(value);
    }
    sentry_context::add_breadcrumb("github.pr", message, data);
  }

  fn sync_sentry_pr_context(&self) {
    let Some(context) = self.current_pr_context.as_ref() else {
      return;
    };
    sentry_context::sync_github_pr_context(
      context.owner.as_str(),
      context.repo.as_str(),
      context.number,
      self.selected_file.as_ref().map(|file| file.path.as_ref()),
      Some(self.active_tab_ix),
      self.selected_commit_sha.as_deref(),
    );
  }

  fn record_pr_error(&self, operation: &'static str, error: &str, mut data: Map<String, Value>) {
    data.insert("error".into(), error.to_string().into());
    let base = self.sentry_pr_data();
    for (key, value) in base {
      data.entry(key).or_insert(value);
    }
    if github_shared::is_unauthorized_error_message(error) {
      sentry_context::record_expected_error(operation, "unauthorized", data);
      return;
    }

    let io_error = std::io::Error::other(error.to_string());
    sentry_context::capture_unexpected_error(operation, &io_error, data);
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    GithubPrDetailsPageHandle::register(cx);

    let tree_state = cx.new(|cx| TreeState::new(cx));
    let diff_editor = Self::build_detached_diff_editor("__reviu_github_pr_placeholder__.diff", cx);
    let merge_commit_title_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Commit title (optional)"));
    let merge_commit_message_input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .rows(5)
        .placeholder("Commit message (optional)")
    });
    let review_input =
      cx.new(|cx| TextareaState::new(window, cx).placeholder("Add an overall review comment..."));
    let suggested_change_commit_title_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Apply suggestion from code review"));
    let suggested_change_commit_message_input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .rows(4)
        .placeholder("Commit message (optional)")
    });
    let assignee_input = cx.new(|cx| InputState::new(window, cx).placeholder("Assign a user..."));
    let requested_reviewer_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Request review..."));
    let label_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add a label..."));
    let target_branch_select = cx.new(|cx| {
      SelectState::new(
        SearchableVec::new(Vec::<PrTargetBranchSelectItem>::new()),
        None,
        window,
        cx,
      )
      .searchable(true)
    });
    let tree_search_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Search in file contents..."));

    let mut this = Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      api: WorkspaceApi::global(cx).api.clone(),
      details_task: None,
      files_task: None,
      files_loading: false,
      files_error: None,
      files_request_generation: 0,
      commits_task: None,
      commits_loading: false,
      commits_error: None,
      commits: Vec::new(),
      commit_lookup: HashMap::new(),
      selected_commit_sha: None,
      checks_task: None,
      checks_loading: false,
      checks_error: None,
      checks: None,
      merge_readiness_task: None,
      merge_readiness_loading: false,
      merge_readiness_error: None,
      merge_readiness: None,
      merge_popover_open: false,
      merge_method_focus_handle: cx.focus_handle(),
      merge_form_reset_pending: true,
      merge_method: GithubPullRequestMergeMethod::Merge,
      merge_commit_title_input,
      merge_commit_message_input,
      merge_submit_task: None,
      merge_submit_loading: false,
      merge_submit_error: None,
      status_action_task: None,
      status_action_loading: false,
      update_branch_task: None,
      update_branch_loading: false,
      target_branch_select,
      target_branch_task: None,
      target_branch_loading: false,
      target_branch_error: None,
      target_branch_request_generation: 0,
      target_branch_update_task: None,
      target_branch_update_loading: false,
      target_branch_update_error: None,
      review_input,
      review_decision: GithubPrReviewDecision::default(),
      review_popover_open: false,
      review_form_reset_pending: false,
      review_preview_open: false,
      submit_review_task: None,
      submit_review_loading: false,
      submit_review_error: None,
      conversation_task: None,
      reaction_task: None,
      reaction_error: None,
      issue_comments_task: None,
      issue_comments_loading: false,
      issue_comments_error: None,
      issue_comments: Vec::new(),
      review_people_options_task: None,
      review_people_options_loading: false,
      review_people_options_error: None,
      review_people_options: Vec::new(),
      label_options: Vec::new(),
      label_options_loading: false,
      label_options_error: None,
      assignee_input,
      requested_reviewer_input,
      label_input,
      assignee_popover_open: false,
      reviewer_popover_open: false,
      label_popover_open: false,
      people_mutation_task: None,
      people_mutation_loading: false,
      people_mutation_error: None,
      label_mutation_task: None,
      label_mutation_loading: false,
      label_mutation_error: None,
      reviews_task: None,
      reviews_loading: false,
      reviews_error: None,
      reviews: Vec::new(),
      review_comments_task: None,
      review_comments_loading: false,
      review_comments_error: None,
      review_comments: Vec::new(),
      pending_review_id: None,
      pending_review_pull_request_id: None,
      suggested_change_commit_target: None,
      suggested_change_commit_title_input,
      suggested_change_commit_message_input,
      suggested_change_include_co_author: true,
      suggested_change_commit_loading: false,
      suggested_change_commit_error: None,
      suggested_change_commit_task: None,
      stale_suggested_change_comment_ids: HashSet::new(),
      resolve_thread_in_flight: HashSet::new(),
      resolve_thread_tasks: HashMap::new(),
      resolve_thread_errors: HashMap::new(),
      expanded_resolved_threads: HashSet::new(),
      selected_file_review_comment_ids: Vec::new(),
      active_review_comment_id: None,
      review_comment_handlers_enabled: true,
      description_code_reference_requests: Vec::new(),
      review_comment_code_reference_cache: HashMap::new(),
      review_comment_code_reference_tasks: HashMap::new(),
      pending_review_comment_link_comment_id: None,
      pr_description_editing: false,
      pr_description_initial_body: None,
      pr_description_submitting: false,
      pr_description_error: None,
      file_loading: false,
      file_error: None,
      tree_state,
      tree_search_input,
      tree_search_query: String::new(),
      tree_search_matches: None,
      tree_search_task: None,
      tree_search_loading: false,
      tree_search_error: None,
      tree_search_generation: 0,
      tree_search_reset_pending: false,
      show_local_project_files: false,
      hide_whitespace: AppSettings::get(cx).hide_whitespace,
      saved_pr_selected_tree_id: None,
      file_lookup: HashMap::new(),
      resolved_local_repo: None,
      resolved_local_repo_scan_complete: false,
      resolved_local_repo_task: None,
      resolved_local_repo_generation: 0,
      local_project_lookup: HashMap::new(),
      local_project_loaded_repo_root: None,
      local_project_tree_loading: false,
      local_project_tree_error: None,
      local_project_files_task: None,
      local_branch_switch_task: None,
      local_branch_switch_loading: false,
      local_branch_switch_error: None,
      local_project_update_task: None,
      local_project_update_loading: false,
      local_project_update_error: None,
      local_project_open_file_task: None,
      local_project_open_file_generation: 0,
      file_contents: HashMap::new(),
      file_content_tasks: HashMap::new(),
      file_asset_previews: HashMap::new(),
      file_asset_tasks: HashMap::new(),
      selected_file: None,
      selected_tree_id: None,
      selected_local_project_file: None,
      selected_local_project_tree_id: None,
      diff_editor,
      diff_view: if AppSettings::get(cx).split_diff_view {
        DiffViewMode::Split
      } else {
        DiffViewMode::Inline
      },
      show_markdown_preview: false,
      binary_preview: None,
      description_markdown_state: MarkdownRenderState::new(),
      syntax_highlight_cache: Arc::new(gfm_markdown_viewer::SyntaxHighlightCache::new()),
      overview_checks_open: false,
      overview_checks_scroll_handle: gpui::ScrollHandle::new(),
      svg_preview: cx.new(|_| SvgPreview::new()),
      active_tab_ix: 0,
      current_pr_context: None,
      pull_request: None,
      error: None,
    };
    this.install_diff_editor_review_comment_handlers(cx);
    this.subscribe_to_tree_search_input(window, cx);
    this.subscribe_to_svg_preview(cx);
    this.subscribe_to_assignee_input(window, cx);
    this.subscribe_to_requested_reviewer_input(window, cx);
    this.subscribe_to_label_input(window, cx);
    this.subscribe_to_target_branch_select(cx);
    this.subscribe_to_review_input(window, cx);
    this.subscribe_to_merge_commit_inputs(window, cx);
    this
  }

  fn current_github_login(cx: &App) -> Option<String> {
    match AuthStateStore::get(cx) {
      AuthState::Authenticated(user) => user.github_login.or_else(|| {
        let fallback = user.name.trim();
        if fallback.is_empty() {
          None
        } else {
          Some(fallback.to_string())
        }
      }),
      _ => None,
    }
  }

  fn current_open_target(&self) -> GithubPrOpenTarget {
    GithubPrOpenTarget::new(self.active_tab_ix == PR_TAB_CHANGES_IX, None)
  }

  fn refresh_current_page(&mut self, cx: &mut Context<Self>) {
    if self.current_pr_context.is_none() {
      return;
    }

    self.refresh_pull_request_details_for_current_context(cx);
    self.reload_merge_readiness_for_current_pull_request(cx);
    self.refresh_commits_for_current_pull_request(cx);
    self.refresh_checks_for_current_pull_request(cx);

    if should_refresh_pr_overview_data(self.active_tab_ix) {
      self.refresh_pull_request_conversation_for_current_pull_request(true, cx);
      self.refresh_review_people_options_for_current_context(cx);
    }

    if should_refresh_pr_changes_data(self.active_tab_ix) {
      self.saved_pr_selected_tree_id = self.current_selected_tree_path();
      self.refresh_pull_request_conversation_for_current_pull_request(false, cx);
      self.reload_files_for_current_pull_request(cx);
    }
  }

  fn refresh_pull_request_details_for_current_context(&mut self, cx: &mut Context<Self>) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };

    self.error = None;
    let details_api = self.api.clone();
    let details_owner = context.owner.clone();
    let details_repo = context.repo.clone();
    let number = context.number;
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          details_api.fetch_pull_request_details(&details_owner, &details_repo, number)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(pull_request) => {
            this.description_code_reference_requests =
              Self::description_code_reference_requests_for_pull_request(&pull_request);
            this.pull_request = Some(pull_request);
            this.resolved_local_repo = None;
            this.resolved_local_repo_scan_complete = false;
            this.resolved_local_repo_task = None;
            this.error = None;
            this.add_pr_breadcrumb("Refresh PR details succeeded", Map::new());
            this.refresh_target_branch_options(cx);
            let description_requests = this.description_code_reference_requests.clone();
            this.schedule_code_reference_fetches(description_requests.iter(), cx);
            this.sync_review_comments(cx);
            this.maybe_fetch_selected_file_contents(cx);
            this.prefetch_overview_root_review_comment_files(cx);
            this.refresh_resolved_local_repo_match(cx);
            if this.tree_search_query_normalized().is_some() {
              this.refresh_tree_text_search(cx);
            }
          }
          Err(error) => {
            let error_message = error.to_string();
            this.pull_request = None;
            this.description_code_reference_requests.clear();
            this.error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Refresh PR details failed", Map::new());
            this.record_pr_error(
              "github.pr.details.refresh",
              error_message.as_str(),
              Map::new(),
            );
            this.sync_review_comments(cx);
          }
        }
        this.details_task = None;
        cx.notify();
      });
    });

    self.details_task = Some(task);
  }

  fn apply_pull_request_conversation(
    &mut self,
    conversation: GithubPullRequestConversation,
    cx: &mut Context<Self>,
  ) {
    let pull_request_node_id = conversation.pull_request.node_id.clone();
    self.issue_comments = conversation.issue_comments;
    self.reviews = conversation.reviews;
    self.review_comments = conversation.review_comments;
    self.stale_suggested_change_comment_ids.clear();
    self.issue_comments_loading = false;
    self.issue_comments_error = None;
    self.reviews_loading = false;
    self.reviews_error = None;
    self.review_comments_loading = false;
    self.review_comments_error = None;
    // Drafts are marked (is_pending) by the backend from each comment's review state;
    // derive the pending-review node id from any draft, keep the PR node id for starting one.
    self.pending_review_id = self
      .review_comments
      .iter()
      .find(|comment| comment.is_pending)
      .and_then(|comment| comment.pull_request_review_node_id.clone());
    self.pending_review_pull_request_id = Some(pull_request_node_id);
    self.sync_review_comments(cx);
    self.prefetch_overview_root_review_comment_files(cx);
  }

  fn refresh_pull_request_conversation_for_current_pull_request(
    &mut self,
    include_overview_loading: bool,
    cx: &mut Context<Self>,
  ) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };

    self.fetch_pull_request_conversation_for_context(
      context.owner,
      context.repo,
      context.number,
      include_overview_loading,
      "Refresh",
      cx,
    );
  }

  fn fetch_pull_request_conversation_for_context(
    &mut self,
    owner: String,
    repo: String,
    number: u64,
    include_overview_loading: bool,
    operation_label: &'static str,
    cx: &mut Context<Self>,
  ) {
    if include_overview_loading {
      self.issue_comments_loading = true;
      self.issue_comments_error = None;
      self.reviews_loading = true;
      self.reviews_error = None;
    }
    self.review_comments_loading = true;
    self.review_comments_error = None;

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move { api.fetch_pull_request_conversation(&owner, &repo, number) })
        .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(conversation) => {
            this.apply_pull_request_conversation(conversation, cx);
            let message = format!("{operation_label} PR conversation succeeded");
            this.add_pr_breadcrumb(message.as_str(), Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            if include_overview_loading {
              this.issue_comments_loading = false;
              this.issue_comments_error = Some(error_message.clone().into());
              this.reviews_loading = false;
              this.reviews_error = Some(error_message.clone().into());
            }
            this.review_comments_loading = false;
            this.review_comments_error = Some(error_message.clone().into());
            let message = format!("{operation_label} PR conversation failed");
            this.add_pr_breadcrumb(message.as_str(), Map::new());
            this.record_pr_error("github.pr.conversation", error_message.as_str(), Map::new());
            this.sync_review_comments(cx);
          }
        }
        this.conversation_task = None;
        cx.notify();
      });
    });

    self.conversation_task = Some(task);
  }

  fn reload_current_pull_request(&mut self, cx: &mut Context<Self>) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };
    let active_tab_ix = self.active_tab_ix;
    let open_target = self.current_open_target();
    self.load_pull_request(context.owner, context.repo, context.number, open_target, cx);
    self.active_tab_ix = active_tab_ix;
  }

  fn refresh_commits_for_current_pull_request(&mut self, cx: &mut Context<Self>) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };

    self.commits_loading = true;
    self.commits_error = None;

    let api = self.api.clone();
    let owner = context.owner;
    let repo = context.repo;
    let number = context.number;
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move { api.fetch_pull_request_commits(&owner, &repo, number) })
        .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(mut commits) => {
            sort_commits_desc(commits.as_mut_slice());
            this.commit_lookup = commits
              .iter()
              .cloned()
              .map(|commit| (commit.sha.clone(), commit))
              .collect();
            this.commits = commits;
            let selected_commit_cleared = this
              .selected_commit_sha
              .clone()
              .is_some_and(|selected_sha| !this.commit_lookup.contains_key(&selected_sha));
            if selected_commit_cleared {
              this.selected_commit_sha = None;
            }
            this.commits_loading = false;
            this.commits_error = None;
            if selected_commit_cleared {
              this.saved_pr_selected_tree_id = this.current_selected_tree_path();
              this.reload_files_for_current_pull_request(cx);
            }
          }
          Err(error) => {
            let error_message = error.to_string();
            this.commits_loading = false;
            this.commits_error = Some(error_message.clone().into());
            this.commits.clear();
            this.commit_lookup.clear();
            this.selected_commit_sha = None;
            this.add_pr_breadcrumb("Refresh PR commits failed", Map::new());
            this.record_pr_error(
              "github.pr.commits.refresh",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        this.commits_task = None;
        cx.notify();
      });
    });

    self.commits_task = Some(task);
  }

  fn refresh_checks_for_current_pull_request(&mut self, cx: &mut Context<Self>) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };

    self.checks_loading = true;
    self.checks_error = None;

    let api = self.api.clone();
    let owner = context.owner;
    let repo = context.repo;
    let number = context.number;
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move { api.fetch_pull_request_checks(&owner, &repo, number) })
        .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(checks) => {
            this.checks = Some(checks);
            this.checks_loading = false;
            this.checks_error = None;
            this.add_pr_breadcrumb("Refresh PR checks succeeded", Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            this.checks = None;
            this.checks_loading = false;
            this.checks_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Refresh PR checks failed", Map::new());
            this.record_pr_error(
              "github.pr.checks.refresh",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        this.checks_task = None;
        cx.notify();
      });
    });

    self.checks_task = Some(task);
  }

  /// The preview renders on a background task; repaint the page when it lands.
  fn subscribe_to_svg_preview(&mut self, cx: &mut Context<Self>) {
    cx.observe(&self.svg_preview, |_, _, cx| cx.notify())
      .detach();
  }

  fn subscribe_to_tree_search_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    cx.subscribe_in(
      &self.tree_search_input,
      window,
      |this, state, event: &InputEvent, _window, cx| {
        if let InputEvent::Change = event {
          this.set_tree_search_query(state.read(cx).value().to_string(), cx);
        }
      },
    )
    .detach();
  }

  fn subscribe_to_target_branch_select(&mut self, cx: &mut Context<Self>) {
    cx.subscribe(
      &self.target_branch_select,
      |this, _state, event: &SelectEvent<SearchableVec<PrTargetBranchSelectItem>>, cx| {
        if let SelectEvent::Confirm(Some(branch)) = event {
          this.target_branch_update_error = None;
          this.submit_target_branch_update(branch.clone(), cx);
        }
      },
    )
    .detach();
  }

  fn set_tree_search_query(&mut self, query: String, cx: &mut Context<Self>) {
    if self.tree_search_query == query {
      return;
    }

    self.tree_search_query = query;
    self.refresh_tree_text_search(cx);
  }

  fn load_pull_request(
    &mut self,
    owner: String,
    repo: String,
    number: u64,
    open_target: GithubPrOpenTarget,
    cx: &mut Context<Self>,
  ) {
    self.active_tab_ix = open_target.tab_ix();
    self.current_pr_context = Some(CurrentPrContext {
      owner: owner.clone(),
      repo: repo.clone(),
      number,
    });
    self.sync_sentry_pr_context();
    self.add_pr_breadcrumb("Load pull request started", Map::new());
    crate::analytics::track(cx, "github_pr_viewed");
    self.error = None;
    self.pull_request = None;
    self.files_loading = true;
    self.files_error = None;
    self.files_request_generation = 0;
    self.commits_loading = true;
    self.commits_error = None;
    self.commits.clear();
    self.commit_lookup.clear();
    self.selected_commit_sha = None;
    self.checks_task = None;
    self.checks_loading = true;
    self.checks_error = None;
    self.checks = None;
    self.overview_checks_open = false;
    self.merge_readiness_task = None;
    self.merge_readiness_loading = true;
    self.merge_readiness_error = None;
    self.merge_readiness = None;
    self.merge_popover_open = false;
    self.merge_submit_task = None;
    self.merge_submit_loading = false;
    self.status_action_task = None;
    self.status_action_loading = false;
    self.update_branch_task = None;
    self.update_branch_loading = false;
    self.target_branch_task = None;
    self.target_branch_loading = false;
    self.target_branch_error = None;
    self.target_branch_request_generation = self.target_branch_request_generation.wrapping_add(1);
    self.target_branch_update_task = None;
    self.target_branch_update_loading = false;
    self.target_branch_update_error = None;
    self.set_target_branch_select_items(Vec::new(), None, cx);
    self.mark_merge_form_reset_pending();
    self.review_popover_open = false;
    self.mark_review_form_reset_pending();
    self.submit_review_task = None;
    self.conversation_task = None;
    self.reaction_task = None;
    self.reaction_error = None;
    self.issue_comments_task = None;
    self.reviews_task = None;
    self.review_comments_task = None;
    self.submit_review_loading = false;
    self.review_people_options_task = None;
    self.review_people_options_loading = true;
    self.review_people_options_error = None;
    self.review_people_options.clear();
    self.label_options.clear();
    self.label_options_loading = true;
    self.label_options_error = None;
    self.people_mutation_task = None;
    self.people_mutation_loading = false;
    self.people_mutation_error = None;
    self.label_mutation_task = None;
    self.label_mutation_loading = false;
    self.label_mutation_error = None;
    self.assignee_popover_open = false;
    self.reviewer_popover_open = false;
    self.label_popover_open = false;
    self.issue_comments_loading = true;
    self.issue_comments_error = None;
    self.issue_comments.clear();
    self.reviews_loading = true;
    self.reviews_error = None;
    self.reviews.clear();
    self.review_comments_loading = true;
    self.review_comments_error = None;
    self.review_comments.clear();
    self.selected_file_review_comment_ids.clear();
    self.active_review_comment_id = None;
    self.description_code_reference_requests.clear();
    self.review_comment_code_reference_cache.clear();
    self.review_comment_code_reference_tasks.clear();
    self.pending_review_comment_link_comment_id = open_target.review_comment_id;
    self.pr_description_editing = false;
    self.pr_description_initial_body = None;
    self.pr_description_submitting = false;
    self.pr_description_error = None;
    self.file_loading = false;
    self.file_error = None;
    self.tree_state.update(cx, |state, cx| {
      state.set_items(Vec::new(), cx);
    });
    self.tree_search_query.clear();
    self.tree_search_matches = None;
    self.tree_search_task = None;
    self.tree_search_loading = false;
    self.tree_search_error = None;
    self.tree_search_generation = 0;
    self.tree_search_reset_pending = true;
    self.show_local_project_files = false;
    self.saved_pr_selected_tree_id = None;
    self.file_lookup.clear();
    self.resolved_local_repo = None;
    self.resolved_local_repo_scan_complete = false;
    self.resolved_local_repo_task = None;
    self.resolved_local_repo_generation = self.resolved_local_repo_generation.wrapping_add(1);
    self.local_project_lookup.clear();
    self.local_project_loaded_repo_root = None;
    self.local_project_tree_loading = false;
    self.local_project_tree_error = None;
    self.local_project_files_task = None;
    self.local_branch_switch_task = None;
    self.local_branch_switch_loading = false;
    self.local_branch_switch_error = None;
    self.local_project_update_task = None;
    self.local_project_update_loading = false;
    self.local_project_update_error = None;
    self.local_project_open_file_task = None;
    self.local_project_open_file_generation = 0;
    self.file_contents.clear();
    self.file_content_tasks.clear();
    self.file_asset_previews.clear();
    self.file_asset_tasks.clear();
    self.selected_tree_id = None;
    self.selected_local_project_file = None;
    self.selected_local_project_tree_id = None;
    self.set_selected_file(None, cx);
    self.diff_view = DiffViewMode::Inline;
    self.show_markdown_preview = false;
    self.binary_preview = None;
    self.svg_preview.update(cx, |preview, _| preview.clear());
    self.diff_editor.update(cx, |editor, cx| {
      editor.document().update(cx, |doc, cx| {
        doc.replace_all("", cx);
      });
      editor.is_read_only = true;
    });
    let window_handle = self.window_handle;
    let _ = cx.update_window(window_handle, |_, window, cx| {
      self
        .assignee_input
        .update(cx, |input, cx| input.set_value("", window, cx));
      self
        .requested_reviewer_input
        .update(cx, |input, cx| input.set_value("", window, cx));
      self
        .label_input
        .update(cx, |input, cx| input.set_value("", window, cx));
    });

    let details_api = self.api.clone();
    let details_owner = owner.clone();
    let details_repo = repo.clone();
    let details_task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          details_api.fetch_pull_request_details(&details_owner, &details_repo, number)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(pull_request) => {
            this.description_code_reference_requests =
              Self::description_code_reference_requests_for_pull_request(&pull_request);
            this.pull_request = Some(pull_request);
            this.resolved_local_repo = None;
            this.resolved_local_repo_scan_complete = false;
            this.resolved_local_repo_task = None;
            this.error = None;
            this.add_pr_breadcrumb("Load PR details succeeded", Map::new());
            this.refresh_target_branch_options(cx);
            let description_requests = this.description_code_reference_requests.clone();
            this.schedule_code_reference_fetches(description_requests.iter(), cx);
            this.sync_review_comments(cx);
            this.maybe_fetch_selected_file_contents(cx);
            this.prefetch_overview_root_review_comment_files(cx);
            this.refresh_resolved_local_repo_match(cx);
            if this.tree_search_query_normalized().is_some() {
              this.refresh_tree_text_search(cx);
            }
          }
          Err(error) => {
            let error_message = error.to_string();
            this.pull_request = None;
            this.description_code_reference_requests.clear();
            this.error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Load PR details failed", Map::new());
            this.record_pr_error("github.pr.details", error_message.as_str(), Map::new());
            this.sync_review_comments(cx);
          }
        }
        cx.notify();
      });
    });

    self.refresh_review_people_options_for_current_context(cx);
    self.fetch_pull_request_conversation_for_context(
      owner.clone(),
      repo.clone(),
      number,
      true,
      "Load",
      cx,
    );

    let commits_api = self.api.clone();
    let commits_owner = owner.clone();
    let commits_repo = repo.clone();
    let commits_task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          commits_api.fetch_pull_request_commits(&commits_owner, &commits_repo, number)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(mut commits) => {
            sort_commits_desc(commits.as_mut_slice());
            this.commit_lookup = commits
              .iter()
              .cloned()
              .map(|commit| (commit.sha.clone(), commit))
              .collect();
            this.commits = commits;
            if let Some(selected_sha) = this.selected_commit_sha.clone()
              && !this.commit_lookup.contains_key(&selected_sha)
            {
              this.selected_commit_sha = None;
            }
            this.commits_loading = false;
            this.commits_error = None;
          }
          Err(error) => {
            let error_message = error.to_string();
            this.commits_loading = false;
            this.commits_error = Some(error_message.clone().into());
            this.commits.clear();
            this.commit_lookup.clear();
            this.selected_commit_sha = None;
            this.add_pr_breadcrumb("Load PR commits failed", Map::new());
            this.record_pr_error("github.pr.commits", error_message.as_str(), Map::new());
          }
        }
        cx.notify();
      });
    });

    let checks_api = self.api.clone();
    let checks_owner = owner.clone();
    let checks_repo = repo.clone();
    let checks_task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          checks_api.fetch_pull_request_checks(&checks_owner, &checks_repo, number)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(checks) => {
            this.checks = Some(checks);
            this.checks_loading = false;
            this.checks_error = None;
            this.add_pr_breadcrumb("Load PR checks succeeded", Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            this.checks = None;
            this.checks_loading = false;
            this.checks_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Load PR checks failed", Map::new());
            this.record_pr_error("github.pr.checks", error_message.as_str(), Map::new());
          }
        }
        cx.notify();
      });
    });

    self.details_task = Some(details_task);
    self.commits_task = Some(commits_task);
    self.checks_task = Some(checks_task);
    self.fetch_merge_readiness_for_context(owner.clone(), repo.clone(), number, cx);
    self.fetch_pull_request_files_for_context(owner, repo, number, cx);
  }
}

impl Render for GithubPrDetailsPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    self.sync_route(window, cx);
    self.maybe_refresh_resolved_local_repo_match(window, cx);

    // Poll syntax highlight cache, if background highlights completed, schedule re-render
    if self.syntax_highlight_cache.take_new_highlights() {
      cx.notify();
    } else if self.syntax_highlight_cache.has_pending() {
      // Background highlights still in progress, check again next frame
      cx.on_next_frame(window, |this, _window, cx| {
        if this.syntax_highlight_cache.take_new_highlights() {
          cx.notify();
        }
      });
    }

    let overview_inner: gpui::AnyElement = if let Some(pr) = self.pull_request.clone() {
      self.render_details(&pr, cx).into_any_element()
    } else if self.error.is_some() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.error.clone().unwrap_or_default())
        .into_any_element()
    } else {
      self.render_overview_skeleton(&theme)
    };

    let overview_content = div()
      .id("overview-tab")
      .flex_1()
      .min_h_0()
      .child(overview_inner)
      .into_any_element();

    let changes_content = div()
      .id("changes-tab")
      .flex_1()
      .min_h_0()
      .child(self.render_changes_tab(window, cx))
      .into_any_element();

    let content = if self.active_tab_ix == PR_TAB_OVERVIEW_IX {
      overview_content
    } else if self.active_tab_ix == PR_TAB_CHANGES_IX {
      changes_content
    } else {
      overview_content
    };

    let right_panel = v_flex()
      .debug_selector(|| "github-pr-details-main-panel".to_string())
      .size_full()
      .overflow_hidden()
      .child(self.render_header(window, cx))
      .child(v_flex().flex_1().min_h_0().child(content));

    div()
      .size_full()
      .flex()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GithubPrDetailsPage::show_command_palette_action))
      .on_action(cx.listener(GithubPrDetailsPage::switch_to_pr_branch_action))
      .on_action(cx.listener(GithubPrDetailsPage::previous_annotation_action))
      .on_action(cx.listener(GithubPrDetailsPage::next_annotation_action))
      .on_action(cx.listener(GithubPrDetailsPage::previous_review_comment_action))
      .on_action(cx.listener(GithubPrDetailsPage::next_review_comment_action))
      .on_action(cx.listener(GithubPrDetailsPage::toggle_diff_view_action))
      .on_action(cx.listener(GithubPrDetailsPage::toggle_commit_by_commit_action))
      .on_action(cx.listener(GithubPrDetailsPage::previous_pr_commit_action))
      .on_action(cx.listener(GithubPrDetailsPage::next_pr_commit_action))
      .on_action(cx.listener(GithubPrDetailsPage::toggle_hide_whitespace_action))
      .on_action(cx.listener(GithubPrDetailsPage::focus_file_tree_action))
      .on_action(cx.listener(GithubPrDetailsPage::comment_hunk_action))
      .on_action(cx.listener(GithubPrDetailsPage::previous_page_tab_action))
      .on_action(cx.listener(GithubPrDetailsPage::next_page_tab_action))
      .on_action(cx.listener(GithubPrDetailsPage::show_file_search_action))
      .on_action(cx.listener(GithubPrDetailsPage::find_action))
      .on_action(cx.listener(GithubPrDetailsPage::close_find_action))
      .child(right_panel)
  }
}

impl Focusable for GithubPrDetailsPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::test_support::*;
  use super::*;

  use gpui::TestAppContext;

  #[gpui::test]
  fn pr_header_shows_branch_switch_on_overview_but_search_only_on_changes(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(
        cx,
        Some(make_active_local_repo_for_branch("main", "head", true)),
      );
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.active_tab_ix = PR_TAB_OVERVIEW_IX;
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });
    cx.run_until_parked();

    let availability = page.read_with(cx, |this, cx| this.local_project_availability(cx));
    assert!(matches!(
      availability,
      GithubPrLocalProjectAvailability::NeedsBranchSwitch {
        has_uncommitted_changes: true,
        ..
      }
    ));

    let controls_bounds = cx
      .debug_bounds("github-pr-local-project-controls")
      .expect("local project controls should render on overview")
      .size;
    assert!(controls_bounds.width > gpui::px(0.0));
    assert!(controls_bounds.height > gpui::px(0.0));
    assert!(cx.debug_bounds("github-pr-file-contents-search").is_none());

    page.update_in(cx, |this, _window, cx| {
      this.active_tab_ix = PR_TAB_CHANGES_IX;
      cx.notify();
    });
    cx.run_until_parked();

    assert!(
      cx.debug_bounds("github-pr-local-project-controls")
        .is_some()
    );
    assert!(cx.debug_bounds("github-pr-file-contents-search").is_some());
  }

  #[gpui::test]
  fn overview_updated_row_renders_inline_change_stats(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

    cx.run_until_parked();
    let created_updated_separator_bounds = cx
      .debug_bounds("github-pr-overview-created-updated-separator")
      .expect("created and updated separator bounds")
      .size;
    let separator_bounds = cx
      .debug_bounds("github-pr-overview-updated-change-stats-separator")
      .expect("updated row separator bounds")
      .size;
    let stats_bounds = cx
      .debug_bounds("github-pr-overview-updated-change-stats")
      .expect("updated row change stats bounds")
      .size;

    assert!(created_updated_separator_bounds.width > gpui::px(0.0));
    assert!(created_updated_separator_bounds.height > gpui::px(0.0));
    assert!(separator_bounds.width > gpui::px(0.0));
    assert!(separator_bounds.height > gpui::px(0.0));
    assert!(stats_bounds.width > gpui::px(0.0));
    assert!(stats_bounds.height > gpui::px(0.0));
  }

  #[gpui::test]
  fn overview_offers_github_instead_of_the_conversation(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

    cx.run_until_parked();

    assert!(
      cx.debug_bounds("github-pr-overview-open-on-github")
        .is_some(),
      "overview should link out to GitHub"
    );
  }

  #[test]
  fn pr_refresh_helpers_match_active_tab() {
    assert!(should_refresh_pr_overview_data(PR_TAB_OVERVIEW_IX));
    assert!(!should_refresh_pr_overview_data(PR_TAB_CHANGES_IX));

    assert!(should_refresh_pr_changes_data(PR_TAB_CHANGES_IX));
    assert!(!should_refresh_pr_changes_data(PR_TAB_OVERVIEW_IX));

    assert!(pr_refresh_in_progress(
      PR_TAB_OVERVIEW_IX,
      false,
      true,
      false,
      false,
      false,
      false,
      false,
      false,
    ));
    assert!(pr_refresh_in_progress(
      PR_TAB_CHANGES_IX,
      false,
      false,
      false,
      true,
      true,
      false,
      false,
      false,
    ));
    assert!(pr_refresh_in_progress(
      PR_TAB_OVERVIEW_IX,
      true,
      false,
      false,
      false,
      false,
      false,
      false,
      false,
    ));
    assert!(pr_refresh_in_progress(
      PR_TAB_OVERVIEW_IX,
      false,
      false,
      false,
      false,
      true,
      false,
      false,
      false,
    ));
  }

  async fn assert_refresh_current_page_starts_commits_and_checks_for_tab(
    active_tab_ix: usize,
    cx: &mut TestAppContext,
  ) {
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    let (
      commits_loading,
      commits_task_present,
      checks_loading,
      checks_task_present,
      issue_comments_loading,
      reviews_loading,
      review_comments_loading,
      files_loading,
    ) = page.update_in(cx, |this, _window, cx| {
      this.api = make_test_api_client("http://127.0.0.1:1");
      this.current_pr_context = Some(CurrentPrContext {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
        number: 42,
      });
      this.active_tab_ix = active_tab_ix;
      this.commits_loading = false;
      this.checks_loading = false;
      this.issue_comments_loading = false;
      this.reviews_loading = false;
      this.review_comments_loading = false;
      this.files_loading = false;
      this.file_loading = false;
      this.commits_task = None;
      this.checks_task = None;
      this.conversation_task = None;
      this.reaction_task = None;
      this.issue_comments_task = None;
      this.reviews_task = None;
      this.review_comments_task = None;
      this.files_task = None;

      this.refresh_current_page(cx);

      (
        this.commits_loading,
        this.commits_task.is_some(),
        this.checks_loading,
        this.checks_task.is_some(),
        this.issue_comments_loading,
        this.reviews_loading,
        this.review_comments_loading,
        this.files_loading,
      )
    });

    assert!(
      commits_loading,
      "tab {active_tab_ix} should refresh commits"
    );
    assert!(
      commits_task_present,
      "tab {active_tab_ix} should schedule commit refresh"
    );
    assert!(checks_loading, "tab {active_tab_ix} should refresh checks");
    assert!(
      checks_task_present,
      "tab {active_tab_ix} should schedule checks refresh"
    );

    match active_tab_ix {
      PR_TAB_OVERVIEW_IX => {
        assert!(issue_comments_loading);
        assert!(reviews_loading);
        assert!(review_comments_loading);
        assert!(!files_loading);
      }
      PR_TAB_CHANGES_IX => {
        assert!(!issue_comments_loading);
        assert!(!reviews_loading);
        assert!(review_comments_loading);
        assert!(files_loading);
      }
      _ => unreachable!(),
    }

    let tasks = page.update_in(cx, |this, _window, _cx| {
      let mut tasks = Vec::new();
      if let Some(task) = this.details_task.take() {
        tasks.push(task);
      }
      if let Some(task) = this.merge_readiness_task.take() {
        tasks.push(task);
      }
      if let Some(task) = this.commits_task.take() {
        tasks.push(task);
      }
      if let Some(task) = this.checks_task.take() {
        tasks.push(task);
      }
      if let Some(task) = this.conversation_task.take() {
        tasks.push(task);
      }
      if let Some(task) = this.reaction_task.take() {
        tasks.push(task);
      }
      if let Some(task) = this.issue_comments_task.take() {
        tasks.push(task);
      }
      if let Some(task) = this.reviews_task.take() {
        tasks.push(task);
      }
      if let Some(task) = this.review_comments_task.take() {
        tasks.push(task);
      }
      if let Some(task) = this.files_task.take() {
        tasks.push(task);
      }
      tasks
    });

    for task in tasks {
      task.await;
    }
  }

  #[gpui::test]
  async fn refresh_current_page_starts_commits_and_checks_for_all_tabs(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    assert_refresh_current_page_starts_commits_and_checks_for_tab(PR_TAB_OVERVIEW_IX, cx).await;
    assert_refresh_current_page_starts_commits_and_checks_for_tab(PR_TAB_CHANGES_IX, cx).await;
  }

  #[gpui::test]
  fn pr_details_handle_refreshing_ignores_details_task_reuse(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    cx.update(|_, cx| {
      assert!(!GithubPrDetailsPageHandle::is_refreshing(cx));
    });

    page.update_in(cx, |this, _window, cx| {
      this.details_task = Some(cx.spawn(async move |_, _| {}));
      this.merge_readiness_loading = false;
      this.issue_comments_loading = false;
      this.reviews_loading = false;
      this.review_comments_loading = false;
      this.commits_loading = false;
      this.files_loading = false;
      this.file_loading = false;
      this.checks_loading = false;
    });

    cx.update(|_, cx| {
      assert!(!GithubPrDetailsPageHandle::is_refreshing(cx));
    });

    page.update_in(cx, |this, _window, _cx| {
      this.merge_readiness_loading = true;
    });

    cx.update(|_, cx| {
      assert!(GithubPrDetailsPageHandle::is_refreshing(cx));
    });
  }

  #[test]
  fn map_file_status_handles_known_and_unknown_values() {
    assert_eq!(map_file_status("added"), GithubPrFileStatus::Added);
    assert_eq!(map_file_status("removed"), GithubPrFileStatus::Deleted);
    assert_eq!(map_file_status("deleted"), GithubPrFileStatus::Deleted);
    assert_eq!(map_file_status("renamed"), GithubPrFileStatus::Renamed);
    assert_eq!(map_file_status("changed"), GithubPrFileStatus::Modified);
    assert_eq!(map_file_status("ADDED"), GithubPrFileStatus::Added);
    assert_eq!(map_file_status(" deleted "), GithubPrFileStatus::Deleted);
  }

  #[test]
  fn commit_subject_uses_first_non_empty_line() {
    assert_eq!(
      github_shared::commit_subject("\n\nfeat: add filter\n\nbody details"),
      "feat: add filter"
    );
    assert_eq!(github_shared::commit_subject(""), "No commit message");
  }

  #[test]
  fn sort_commits_desc_orders_newest_first() {
    let mut commits = vec![
      make_api_commit(
        "aaaaaaa111111111111111111111111111111111",
        "older",
        Some("2026-02-20T10:00:00Z"),
        Some("p1"),
      ),
      make_api_commit(
        "bbbbbbb222222222222222222222222222222222",
        "newer",
        Some("2026-02-25T10:00:00Z"),
        Some("p2"),
      ),
    ];

    sort_commits_desc(commits.as_mut_slice());
    assert_eq!(commits[0].message, "newer");
    assert_eq!(commits[1].message, "older");
  }

  #[test]
  fn markdown_path_detection_is_case_insensitive_and_extension_based() {
    assert!(is_markdown_path(Path::new("README.md")));
    assert!(is_markdown_path(Path::new("docs/GUIDE.MD")));
    assert!(is_markdown_path(Path::new("notes.markdown")));
    assert!(is_markdown_path(Path::new("post.MdX")));

    assert!(!is_markdown_path(Path::new("README")));
    assert!(!is_markdown_path(Path::new("icon.svg")));
    assert!(!is_markdown_path(Path::new("note.md.txt")));
  }

  #[test]
  fn svg_path_detection_is_case_insensitive_and_extension_based() {
    assert!(is_svg_path(Path::new("icon.svg")));
    assert!(is_svg_path(Path::new("ICON.SVG")));

    assert!(!is_svg_path(Path::new("icon.svgz")));
    assert!(!is_svg_path(Path::new("README.md")));
    assert!(!is_svg_path(Path::new("icon")));
  }

  #[test]
  fn files_from_api_sets_unknown_filename_and_rename_old_path() {
    let files = files_from_api(vec![
      make_api_file("", "added", None),
      make_api_file("src/new.rs", "renamed", Some("src/old.rs")),
      make_api_file("src/main.rs", "modified", Some("src/very_old.rs")),
    ]);

    assert_eq!(files[0].path.as_ref(), "unknown");
    assert_eq!(files[0].old_path, None);

    assert_eq!(files[1].status, GithubPrFileStatus::Renamed);
    assert_eq!(files[1].path.as_ref(), "src/new.rs");
    assert_eq!(
      files[1].old_path.as_ref().map(|v| v.as_ref()),
      Some("src/old.rs")
    );

    assert_eq!(files[2].status, GithubPrFileStatus::Modified);
    assert_eq!(files[2].old_path, None);
  }

  #[test]
  fn build_tree_items_prefers_folder_and_selects_first_file() {
    let files = files_from_api(vec![
      make_api_file("README.md", "modified", None),
      make_api_file("src/lib.rs", "modified", None),
      make_api_file("src/main.rs", "modified", None),
    ]);

    let (items, lookup, selected_index, selected_id) = build_tree_items(&files);

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].label.as_ref(), "src");
    assert_eq!(items[0].children.len(), 2);
    assert_eq!(items[0].children[0].label.as_ref(), "lib.rs");
    assert_eq!(items[0].children[1].label.as_ref(), "main.rs");
    assert_eq!(items[1].label.as_ref(), "README.md");

    assert_eq!(selected_id.as_deref(), Some("src/lib.rs"));
    assert_eq!(selected_index, Some(0));
    assert!(lookup.contains_key("src/lib.rs"));
    assert!(lookup.contains_key("README.md"));
  }

  #[test]
  fn extract_github_blob_line_references_reads_markdown_link_syntax() {
    let body = "[compose](https://github.com/acme/widget/blob/main/docker-compose.yml#L7)";
    let references = extract_github_blob_line_references(body);
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].start_line, 7);
    assert_eq!(references[0].end_line, 7);
    assert_eq!(references[0].path, "docker-compose.yml");
  }

  #[test]
  fn github_pr_open_target_defaults_to_overview_tab() {
    let target = GithubPrOpenTarget::default();
    assert_eq!(target.tab_ix(), 0);
    assert_eq!(target.review_comment_id, None);
  }

  #[test]
  fn github_pr_open_target_new_respects_open_changes_flag() {
    let target = GithubPrOpenTarget::new(true, None);
    assert_eq!(target.tab_ix(), 1);
  }

  #[test]
  fn github_pr_open_target_new_routes_review_comment_links_to_changes_tab() {
    let target = GithubPrOpenTarget::new(false, Some(42));
    assert_eq!(target.tab_ix(), 1);
  }

  #[test]
  fn github_pr_route_target_parses_overview_route() {
    assert_eq!(
      github_pr_route_target_from_pathname("/github/acme/widget/pull/42"),
      Some(GithubPrRouteTarget {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
        number: 42,
        tab_ix: PR_TAB_OVERVIEW_IX,
      })
    );
  }

  #[test]
  fn github_pr_route_target_parses_changes_route() {
    assert_eq!(
      github_pr_route_target_from_pathname("/github/acme/widget/pull/42/changes"),
      Some(GithubPrRouteTarget {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
        number: 42,
        tab_ix: PR_TAB_CHANGES_IX,
      })
    );
  }

  #[test]
  fn github_pr_route_target_ignores_non_pr_routes() {
    assert_eq!(
      github_pr_route_target_from_pathname("/github/acme/widget/issues/42"),
      None
    );
  }
}
