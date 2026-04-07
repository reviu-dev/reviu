use std::{
  collections::{BTreeMap, BTreeSet, HashMap, HashSet},
  path::{Path, PathBuf},
  rc::Rc,
  sync::Arc,
};

use editor::{
  CloseFind, DiffViewMode, Editor, Find, ReviewComment, ReviewCommentCodeReferencePreview,
  ReviewCommentCreateHandler, ReviewCommentCreateRequest, ReviewCommentDeleteHandler,
  ReviewCommentEditHandler, ReviewCommentLinkHandler, ReviewCommentSide,
};
use gfm_markdown_viewer::{
  GithubBlobLineReference, GithubCodeReferencePreview, LinkAction, MarkdownRenderOptions,
  MarkdownRenderState, extract_github_blob_line_references,
  render_github_code_reference_preview_card, render_markdown,
};
use git::{
  DiffKind, DiffSet, FileDiff, GitStore, RepoStatusKind, compute_buffer_diff, create_stash,
  current_branch_status, current_github_remote_repo, current_head_sha, default_stash_message,
  is_merge_in_progress, is_rebase_in_progress, list_repo_head_files, list_repo_status,
  search_repo_head_contents, switch_to_branch_name, sync_current_branch_to_head,
};
use gpui::{
  AnyElement, AnyWindowHandle, App, Context, Corner, Entity, FocusHandle, Focusable, Hsla, Image,
  ListAlignment, ListState as GpuiListState, MouseButton, ObjectFit, ParentElement, Render,
  RenderImage, SharedString, Styled, Task, Window, div, img, list, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, IndexPath, Selectable, Sizable as _, StyledExt,
  avatar::Avatar,
  button::{Button, ButtonVariant, ButtonVariants as _},
  clipboard::Clipboard,
  h_flex,
  input::InputEvent,
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  notification::Notification,
  radio::{Radio, RadioGroup},
  scroll::ScrollableElement,
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
use smol::unblock;

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, ConfirmDialog, DETAILS_PAGE_CONTAINER_MAX_WIDTH,
  DropdownSelectConfig, DropdownSelectItem, FILE_ICON_SIZE_PX, Input, InputState, Popover,
  SearchFileEntry, SearchFileHandler, SelectableRowStyle, StatusTag, StatusThemeExt, UiIconName,
  WindowExt, dropdown_select, file_icon_path_for_name_with_theme, h_resizable,
  parse_github_url_action, resizable_panel, selectable_list_item,
};

use crate::{
  ShowCommandPalette, ShowFileSearch,
  active_local_repo::{ActiveLocalRepo, ActiveLocalRepoStore},
  api::{
    ApiClient, ApiError, GithubIssueDetailsComment, GithubPullRequestCheckRun,
    GithubPullRequestChecksRollupState, GithubPullRequestChecksSummary, GithubPullRequestCommit,
    GithubPullRequestDescriptionUpdate, GithubPullRequestDetails, GithubPullRequestFile,
    GithubPullRequestIssueComment, GithubPullRequestIssueCommentUser,
    GithubPullRequestLegacyStatus, GithubPullRequestMergeMethod, GithubPullRequestMergeReadiness,
    GithubPullRequestMergeReadinessStatus, GithubPullRequestMergeResult, GithubPullRequestReview,
    GithubPullRequestReviewComment, GithubPullRequestReviewEvent, GithubPullRequestReviewState,
    GithubPullRequestState, GithubPullRequestWorkflowJob, GithubPullRequestWorkflowRun,
    GithubPullRequestWorkflowStep, GithubRepository,
  },
  auth_state::{AuthState, AuthStateStore},
  config::{AppSettings, ConfigStore},
  date_format::format_relative_time,
  file_preview::{
    FilePreviewKind, file_preview_kind, is_markdown_path, is_svg_path, raster_image_from_bytes,
    should_show_unsupported_binary_placeholder,
  },
  file_search_palette::open_file_search_palette as open_shared_file_search_palette,
  git_page::GitPageHandle,
  github_navigation::{
    SamePrGfmNavigation, open_repo_target, same_pr_gfm_navigation, should_open_externally,
  },
  github_page::GithubPageHandle,
  github_repo_page::GithubRepoPageHandle,
  github_shared,
  navigation::NavigationHistory,
  sentry_context,
  workspace::WorkspaceApi,
};

const SIDEBAR_DEFAULT_WIDTH: f32 = 400.0;
const SIDEBAR_MIN_WIDTH: f32 = 350.0;
const SIDEBAR_MAX_WIDTH: f32 = 1500.0;
const DIFF_HEADER_HEIGHT: f32 = 40.0;
const PR_TAB_OVERVIEW_IX: usize = 0;
const PR_TAB_CHANGES_IX: usize = 1;
const PR_TAB_CHECKS_IX: usize = 2;

fn should_refresh_pr_overview_data(active_tab_ix: usize) -> bool {
  active_tab_ix == PR_TAB_OVERVIEW_IX
}

fn should_refresh_pr_changes_data(active_tab_ix: usize) -> bool {
  active_tab_ix == PR_TAB_CHANGES_IX
}

fn should_refresh_pr_checks_data(active_tab_ix: usize) -> bool {
  active_tab_ix == PR_TAB_CHECKS_IX
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

  if should_refresh_pr_overview_data(active_tab_ix) {
    return issue_comments_loading || reviews_loading || review_comments_loading;
  }

  if should_refresh_pr_changes_data(active_tab_ix) {
    return commits_loading || review_comments_loading || files_loading || file_loading;
  }

  if should_refresh_pr_checks_data(active_tab_ix) {
    return checks_loading;
  }

  false
}

fn pr_tab_url_segment(tab_ix: usize) -> &'static str {
  match tab_ix {
    PR_TAB_CHANGES_IX => "changes",
    PR_TAB_CHECKS_IX => "checks",
    _ => "", // overview = no suffix
  }
}

fn adjacent_pr_tab_ix(current: usize, direction: TabNavigationDirection) -> usize {
  const PR_TAB_COUNT: usize = 3;

  match direction {
    TabNavigationDirection::Previous => (current + PR_TAB_COUNT - 1) % PR_TAB_COUNT,
    TabNavigationDirection::Next => (current + 1) % PR_TAB_COUNT,
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubPrLeftSidebarKind {
  Files,
  Context,
}

fn left_sidebar_kind_for_tab(active_tab_ix: usize) -> GithubPrLeftSidebarKind {
  if active_tab_ix == PR_TAB_CHANGES_IX {
    GithubPrLeftSidebarKind::Files
  } else {
    GithubPrLeftSidebarKind::Context
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TabNavigationDirection {
  Previous,
  Next,
}
const PR_COMMIT_SELECT_WIDTH: f32 = 260.0;
const PR_COMMIT_SELECT_MENU_WIDTH: f32 = 320.0;
const PR_MERGE_POPOVER_WIDTH: f32 = 520.0;
const PR_MERGE_MESSAGE_INPUT_HEIGHT_PX: f32 = 100.0;
const PR_REVIEW_POPOVER_WIDTH: f32 = 500.0;
const PR_REVIEW_INPUT_HEIGHT_PX: f32 = 100.0;
const OVERVIEW_COMMENT_INPUT_HEIGHT_PX: f32 = 100.0;
const OVERVIEW_DESCRIPTION_INPUT_HEIGHT_PX: f32 = 500.0;
const GITHUB_PR_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR: &str =
  "github-pr-markdown-preview-editor-pane";
const GITHUB_PR_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR: &str =
  "github-pr-markdown-preview-render-pane";
const GITHUB_PR_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR: &str = "github-pr-binary-preview-render-pane";

type CommitSelectHandler = Rc<dyn Fn(Option<String>, &mut Window, &mut App)>;

struct GithubPrStatusActionNotificationId;

#[derive(Clone)]
enum GithubPrBinaryPreview {
  RasterImage(Arc<Image>),
  UnsupportedBinary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverviewCommentKind {
  Issue,
  Review,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OverviewCommentTarget {
  kind: OverviewCommentKind,
  id: u64,
}

enum OverviewCommentUpdateResult {
  Issue(GithubPullRequestIssueComment),
  Review(Box<GithubPullRequestReviewComment>),
}

impl OverviewCommentUpdateResult {
  fn review(comment: GithubPullRequestReviewComment) -> Self {
    Self::Review(Box::new(comment))
  }
}

fn pr_description_scope_id(pr_number: u64) -> usize {
  (pr_number as usize).wrapping_mul(1_000_003).wrapping_add(1)
}

fn code_reference_requests_from_markdown(markdown: &str) -> Vec<GithubBlobLineReference> {
  extract_github_blob_line_references(markdown)
}

fn gfm_preview_from_review_preview(
  preview: &ReviewCommentCodeReferencePreview,
) -> GithubCodeReferencePreview {
  GithubCodeReferencePreview {
    url: preview.url.clone(),
    repo: preview.repo.clone(),
    path: preview.path.clone(),
    reference: preview.reference.clone(),
    start_line: preview.start_line,
    end_line: preview.end_line,
    snippets: preview.snippets.clone(),
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubPrFileStatus {
  Added,
  Modified,
  Deleted,
  Renamed,
}

fn status_letter(status: GithubPrFileStatus) -> &'static str {
  match status {
    GithubPrFileStatus::Added => "A",
    GithubPrFileStatus::Modified => "M",
    GithubPrFileStatus::Deleted => "D",
    GithubPrFileStatus::Renamed => "R",
  }
}

fn status_color(status: GithubPrFileStatus, theme: &gpui_component::Theme) -> gpui::Hsla {
  match status {
    GithubPrFileStatus::Modified => theme.status_orange(),
    GithubPrFileStatus::Added => theme.status_green(),
    GithubPrFileStatus::Deleted => theme.status_red(),
    GithubPrFileStatus::Renamed => theme.info,
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewCommentNavigationDirection {
  Previous,
  Next,
}

fn next_review_comment_navigation_index(
  comment_ids: &[u64],
  active_comment_id: Option<u64>,
  direction: ReviewCommentNavigationDirection,
) -> Option<usize> {
  if comment_ids.is_empty() {
    return None;
  }

  let active_index =
    active_comment_id.and_then(|id| comment_ids.iter().position(|value| *value == id));

  Some(match direction {
    ReviewCommentNavigationDirection::Next => active_index
      .map(|ix| (ix + 1) % comment_ids.len())
      .unwrap_or(0),
    ReviewCommentNavigationDirection::Previous => active_index
      .map(|ix| {
        if ix == 0 {
          comment_ids.len() - 1
        } else {
          ix - 1
        }
      })
      .unwrap_or(comment_ids.len() - 1),
  })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GithubPrReviewDecision {
  #[default]
  Comment,
  Approve,
  RequestChanges,
}

fn merge_method_label(method: GithubPullRequestMergeMethod) -> &'static str {
  match method {
    GithubPullRequestMergeMethod::Merge => "Create a merge commit",
    GithubPullRequestMergeMethod::Squash => "Squash and merge",
    GithubPullRequestMergeMethod::Rebase => "Rebase and merge",
  }
}

fn merge_method_supports_commit_message(method: GithubPullRequestMergeMethod) -> bool {
  !matches!(method, GithubPullRequestMergeMethod::Rebase)
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

fn commit_subject(message: &str) -> String {
  message
    .lines()
    .map(str::trim)
    .find(|line| !line.is_empty())
    .unwrap_or("No commit message")
    .to_string()
}

fn pull_request_commit_matches(commit: &GithubPullRequestCommit, query: &str) -> bool {
  if query.is_empty() {
    return true;
  }

  let q = query.to_lowercase();
  commit.sha.to_lowercase().contains(&q)
    || github_shared::short_sha(&commit.sha)
      .to_lowercase()
      .contains(&q)
    || commit.message.to_lowercase().contains(&q)
    || commit
      .author
      .as_ref()
      .is_some_and(|author| author.login.to_lowercase().contains(&q))
    || commit
      .committer
      .as_ref()
      .is_some_and(|committer| committer.login.to_lowercase().contains(&q))
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

fn resolve_diff_shas_for_context(
  merge_base_sha: &str,
  base_sha: &str,
  head_sha: &str,
  selected_commit_sha: Option<&str>,
  selected_parent_sha: Option<&str>,
) -> Option<(String, String)> {
  if let Some(selected_commit_sha) = selected_commit_sha {
    let selected_commit_sha = selected_commit_sha.trim();
    if selected_commit_sha.is_empty() {
      return None;
    }
    let base_sha = selected_parent_sha
      .map(str::trim)
      .filter(|sha| !sha.is_empty())
      .unwrap_or_else(|| base_sha.trim());
    if base_sha.is_empty() {
      return None;
    }
    return Some((base_sha.to_string(), selected_commit_sha.to_string()));
  }

  let base_sha = if !merge_base_sha.trim().is_empty() {
    merge_base_sha.trim()
  } else {
    base_sha.trim()
  };
  let head_sha = head_sha.trim();
  if base_sha.is_empty() || head_sha.is_empty() {
    return None;
  }

  Some((base_sha.to_string(), head_sha.to_string()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewCommentPreviewSide {
  Left,
  Right,
}

fn positive_line_number(value: Option<i64>) -> Option<usize> {
  value.and_then(|value| (value > 0).then_some(value as usize))
}

fn normalize_line_range(start: Option<i64>, end: Option<i64>) -> Option<(usize, usize)> {
  let start = positive_line_number(start);
  let end = positive_line_number(end);
  let (start, end) = match (start, end) {
    (Some(start), Some(end)) => (start, end),
    (Some(start), None) => (start, start),
    (None, Some(end)) => (end, end),
    (None, None) => return None,
  };

  Some(if start <= end {
    (start, end)
  } else {
    (end, start)
  })
}

fn review_comment_preview_line_range(
  comment: &GithubPullRequestReviewComment,
) -> Option<(usize, usize)> {
  normalize_line_range(
    comment.start_line.or(comment.line),
    comment.line.or(comment.start_line),
  )
  .or_else(|| {
    normalize_line_range(
      comment.original_start_line.or(comment.original_line),
      comment.original_line.or(comment.original_start_line),
    )
  })
}

fn review_comment_preview_side(
  comment: &GithubPullRequestReviewComment,
) -> ReviewCommentPreviewSide {
  let side = comment
    .side
    .as_deref()
    .or(comment.start_side.as_deref())
    .unwrap_or("RIGHT");
  if side.eq_ignore_ascii_case("LEFT") {
    ReviewCommentPreviewSide::Left
  } else {
    ReviewCommentPreviewSide::Right
  }
}

fn review_comment_targets_file(
  comment: &GithubPullRequestReviewComment,
  file: &GithubPrFileDiff,
) -> bool {
  comment.path == file.path
    || file
      .old_path
      .as_ref()
      .is_some_and(|old_path| old_path.as_ref() == comment.path)
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

fn review_comment_to_editor_comment(
  comment: &GithubPullRequestReviewComment,
  comments_by_id: &HashMap<u64, &GithubPullRequestReviewComment>,
) -> Option<ReviewComment> {
  let (line, side, resolved_line) = resolve_review_comment_display_anchor(comment, comments_by_id)?;

  let line_label = {
    let line_label = if let Some(start) = comment.start_line
      && let Some(end) = comment.line
      && start != end
    {
      Some(format!("L{}-{}", start, end))
    } else {
      comment
        .line
        .or(comment.start_line)
        .or(resolved_line)
        .map(|value| format!("L{}", value))
    };
    line_label.map(|label| Arc::from(label.as_str()))
  };

  Some(ReviewComment {
    id: comment.id,
    in_reply_to_id: comment.in_reply_to_id,
    line,
    side,
    author: Arc::from(comment.user.login.as_str()),
    avatar_url: comment.user.avatar_url.as_deref().map(Arc::from),
    line_label,
    body: Arc::from(comment.body.as_str()),
    created_at: Arc::from(format_relative_time(&comment.created_at).to_string()),
  })
}

fn visible_review_comment_counts_by_path(
  file_lookup: &HashMap<String, Rc<GithubPrFileDiff>>,
  review_comments: &[GithubPullRequestReviewComment],
) -> HashMap<String, usize> {
  if file_lookup.is_empty() || review_comments.is_empty() {
    return HashMap::new();
  }

  let comments_by_id: HashMap<u64, &GithubPullRequestReviewComment> = review_comments
    .iter()
    .map(|comment| (comment.id, comment))
    .collect();
  let mut counts = HashMap::new();

  for comment in review_comments {
    let Some(file) = file_for_review_comment_path(file_lookup, comment.path.as_str()) else {
      continue;
    };
    if resolve_review_comment_display_anchor(comment, &comments_by_id).is_none() {
      continue;
    }
    *counts.entry(file.path.to_string()).or_insert(0) += 1;
  }

  counts
}

fn github_blob_url(
  owner: &str,
  repo: &str,
  reference: &str,
  path: &str,
  start_line: usize,
  end_line: usize,
) -> String {
  if start_line == end_line {
    format!("https://github.com/{owner}/{repo}/blob/{reference}/{path}#L{start_line}")
  } else {
    format!("https://github.com/{owner}/{repo}/blob/{reference}/{path}#L{start_line}-L{end_line}")
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum GithubPrOverviewConversationItemKind {
  IssueComment,
  Review,
  ReviewComment,
}

#[derive(Clone, Debug)]
struct GithubPrOverviewConversationItem {
  kind: GithubPrOverviewConversationItemKind,
  id: u64,
  timestamp: String,
  author_login: String,
  author_avatar_url: Option<String>,
  body: Option<String>,
  review_state: Option<GithubPullRequestReviewState>,
  replies: Vec<GithubPrOverviewConversationReply>,
  thread_comment_ids: Vec<u64>,
}

#[derive(Clone, Debug)]
struct GithubPrOverviewConversationReply {
  id: u64,
  timestamp: String,
  author_login: String,
  author_avatar_url: Option<String>,
  body: String,
}

fn review_state_display_label(state: GithubPullRequestReviewState) -> &'static str {
  match state {
    GithubPullRequestReviewState::Commented => "Commented",
    GithubPullRequestReviewState::Approved => "Approved",
    GithubPullRequestReviewState::RequestChanges => "Changes requested",
    GithubPullRequestReviewState::Dismissed => "Dismissed",
    GithubPullRequestReviewState::Pending => "Pending",
  }
}

fn review_state_icon_style(
  state: GithubPullRequestReviewState,
  theme: &gpui_component::Theme,
) -> Option<(UiIconName, gpui::Hsla)> {
  match state {
    GithubPullRequestReviewState::Commented => None,
    GithubPullRequestReviewState::Approved => Some((UiIconName::CircleCheck, theme.status_green())),
    GithubPullRequestReviewState::RequestChanges => {
      Some((UiIconName::FileDiff, theme.status_red()))
    }
    GithubPullRequestReviewState::Dismissed => {
      Some((UiIconName::CircleSlash, theme.status_orange()))
    }
    GithubPullRequestReviewState::Pending => None,
  }
}

fn checks_rollup_state_label(state: GithubPullRequestChecksRollupState) -> &'static str {
  match state {
    GithubPullRequestChecksRollupState::Success => "Passing",
    GithubPullRequestChecksRollupState::Pending => "Pending",
    GithubPullRequestChecksRollupState::Failure => "Failing",
  }
}

fn checks_rollup_state_color(
  state: GithubPullRequestChecksRollupState,
  theme: &gpui_component::Theme,
) -> gpui::Hsla {
  match state {
    GithubPullRequestChecksRollupState::Success => theme.status_green(),
    GithubPullRequestChecksRollupState::Pending => theme.status_orange(),
    GithubPullRequestChecksRollupState::Failure => theme.status_red(),
  }
}

fn render_checks_state_badge(
  state: GithubPullRequestChecksRollupState,
  theme: &gpui_component::Theme,
) -> gpui::AnyElement {
  let color = checks_rollup_state_color(state, theme);
  StatusTag::new(color)
    .outline()
    .child(checks_rollup_state_label(state))
    .into_any_element()
}

fn render_checks_summary_card(
  checks: &GithubPullRequestChecksSummary,
  theme: &gpui_component::Theme,
  trailing_header_content: Option<gpui::AnyElement>,
) -> gpui::AnyElement {
  v_flex()
    .debug_selector(|| "github-pr-checks-summary-card".to_string())
    .gap_3()
    .border_1()
    .border_color(theme.border)
    .rounded(theme.radius)
    .p_3()
    .child(
      h_flex()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
          div()
            .text_sm()
            .font_medium()
            .text_color(theme.foreground)
            .child("Checks summary"),
        )
        .child(
          h_flex()
            .items_center()
            .gap_2()
            .child(render_checks_state_badge(checks.overall_state, theme))
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Overall"),
            )
            .child(render_checks_state_badge(checks.required_state, theme))
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Required"),
            )
            .when_some(trailing_header_content, |this, content| this.child(content)),
        ),
    )
    .child(
      h_flex()
        .items_center()
        .gap_4()
        .flex_wrap()
        .child(
          div()
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Total checks"),
            )
            .child(
              div()
                .text_sm()
                .font_medium()
                .text_color(theme.foreground)
                .child(checks.total_checks.to_string()),
            ),
        )
        .child(
          div()
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Passing"),
            )
            .child(
              div()
                .text_sm()
                .font_medium()
                .text_color(theme.status_green())
                .child(checks.successful_checks.to_string()),
            ),
        )
        .child(
          div()
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Failing"),
            )
            .child(
              div()
                .text_sm()
                .font_medium()
                .text_color(theme.status_red())
                .child(checks.failed_checks.to_string()),
            ),
        )
        .child(
          div()
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Pending"),
            )
            .child(
              div()
                .text_sm()
                .font_medium()
                .text_color(theme.status_orange())
                .child(checks.pending_checks.to_string()),
            ),
        ),
    )
    .child(
      div()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(format!(
          "{} required contexts • {} passed • {} failing • {} pending",
          checks.required_checks_total,
          checks.required_checks_passed,
          checks.required_checks_failed,
          checks.required_checks_pending,
        )),
    )
    .when(checks.requires_up_to_date_branch, |this| {
      this.child(
        div().text_xs().text_color(theme.muted_foreground).child(
          "The base branch rules require this pull request to be up to date before merging.",
        ),
      )
    })
    .into_any_element()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OverviewPrAlertKind {
  Conflicts,
  OutOfDate,
  Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OverviewPrAlertContent {
  id: &'static str,
  kind: OverviewPrAlertKind,
  title: &'static str,
  message: String,
}

fn overview_pr_alert_content(
  merge_readiness: Option<&GithubPullRequestMergeReadiness>,
  checks: Option<&GithubPullRequestChecksSummary>,
) -> Option<OverviewPrAlertContent> {
  if let Some(readiness) = merge_readiness {
    match readiness
      .mergeable_state
      .as_deref()
      .map(str::trim)
      .map(str::to_ascii_lowercase)
      .as_deref()
    {
      Some("dirty") => {
        return Some(OverviewPrAlertContent {
          id: "github-pr-overview-conflicts-alert",
          kind: OverviewPrAlertKind::Conflicts,
          title: "Merge conflicts detected",
          message: readiness.message.clone(),
        });
      }
      Some("behind") => {
        return Some(OverviewPrAlertContent {
          id: "github-pr-overview-out-of-date-alert",
          kind: OverviewPrAlertKind::OutOfDate,
          title: "Branch is out of date",
          message: readiness.message.clone(),
        });
      }
      _ => {}
    }

    if matches!(
      readiness.status,
      GithubPullRequestMergeReadinessStatus::Blocked
    ) {
      return Some(OverviewPrAlertContent {
        id: "github-pr-overview-merge-blocked-alert",
        kind: OverviewPrAlertKind::Blocked,
        title: "Merge is blocked",
        message: readiness.message.clone(),
      });
    }
  }

  if checks.is_some_and(|checks| checks.requires_up_to_date_branch) {
    return Some(OverviewPrAlertContent {
      id: "github-pr-overview-out-of-date-alert",
      kind: OverviewPrAlertKind::OutOfDate,
      title: "Branch is out of date",
      message: "The base branch rules require this pull request to be up to date before merging."
        .to_string(),
    });
  }

  None
}

fn conversation_source_priority(kind: GithubPrOverviewConversationItemKind) -> u8 {
  match kind {
    GithubPrOverviewConversationItemKind::IssueComment => 0,
    GithubPrOverviewConversationItemKind::Review => 1,
    GithubPrOverviewConversationItemKind::ReviewComment => 2,
  }
}

fn resolve_review_comment_thread_root_id(
  comment: &GithubPullRequestReviewComment,
  comments_by_id: &HashMap<u64, &GithubPullRequestReviewComment>,
) -> u64 {
  let mut root_id = comment.id;
  let mut parent = comment.in_reply_to_id;
  for _ in 0..64 {
    let Some(parent_id) = parent else {
      break;
    };
    if parent_id == root_id {
      break;
    }
    root_id = parent_id;
    parent = comments_by_id
      .get(&parent_id)
      .and_then(|value| value.in_reply_to_id);
  }
  if comments_by_id.contains_key(&root_id) {
    root_id
  } else {
    comment.id
  }
}

fn overview_root_review_comment_ids(
  review_comments: &[GithubPullRequestReviewComment],
) -> Vec<u64> {
  let comments_by_id: HashMap<u64, &GithubPullRequestReviewComment> = review_comments
    .iter()
    .map(|comment| (comment.id, comment))
    .collect();
  let mut root_ids = Vec::new();
  let mut seen = HashSet::new();

  for comment in review_comments {
    let root_id = resolve_review_comment_thread_root_id(comment, &comments_by_id);
    if seen.insert(root_id) {
      root_ids.push(root_id);
    }
  }

  root_ids
}

fn build_overview_conversation_items(
  issue_comments: &[GithubPullRequestIssueComment],
  reviews: &[GithubPullRequestReview],
  review_comments: &[GithubPullRequestReviewComment],
) -> Vec<GithubPrOverviewConversationItem> {
  let mut items = Vec::new();

  items.extend(issue_comments.iter().map(|comment| {
    let body = comment.body.trim();
    GithubPrOverviewConversationItem {
      kind: GithubPrOverviewConversationItemKind::IssueComment,
      id: comment.id,
      timestamp: comment.created_at.clone(),
      author_login: comment
        .user
        .as_ref()
        .map(|user| user.login.clone())
        .unwrap_or_else(|| "unknown".to_string()),
      author_avatar_url: comment
        .user
        .as_ref()
        .and_then(|user| user.avatar_url.clone()),
      body: Some(if body.is_empty() {
        "No comment body.".to_string()
      } else {
        body.to_string()
      }),
      review_state: None,
      replies: Vec::new(),
      thread_comment_ids: Vec::new(),
    }
  }));

  items.extend(reviews.iter().filter_map(|review| {
    let submitted_at = review.submitted_at.as_ref()?;
    let body = review
      .body
      .as_deref()
      .map(str::trim)
      .filter(|value| !value.is_empty())
      .map(ToString::to_string);

    // Skip empty "COMMENTED" reviews — these are auto-created envelopes
    // for inline review comments that already appear as ReviewComment items.
    if matches!(review.state, GithubPullRequestReviewState::Commented) && body.is_none() {
      return None;
    }

    Some(GithubPrOverviewConversationItem {
      kind: GithubPrOverviewConversationItemKind::Review,
      id: review.id,
      timestamp: submitted_at.clone(),
      author_login: review
        .user
        .as_ref()
        .map(|user| user.login.clone())
        .unwrap_or_else(|| "unknown".to_string()),
      author_avatar_url: review
        .user
        .as_ref()
        .and_then(|user| user.avatar_url.clone()),
      body,
      review_state: (!matches!(review.state, GithubPullRequestReviewState::Commented))
        .then_some(review.state),
      replies: Vec::new(),
      thread_comment_ids: Vec::new(),
    })
  }));

  let comments_by_id: HashMap<u64, &GithubPullRequestReviewComment> = review_comments
    .iter()
    .map(|comment| (comment.id, comment))
    .collect();
  let mut threads: HashMap<u64, Vec<&GithubPullRequestReviewComment>> = HashMap::new();
  for comment in review_comments {
    let root_id = resolve_review_comment_thread_root_id(comment, &comments_by_id);
    threads.entry(root_id).or_default().push(comment);
  }

  for mut thread_comments in threads.into_values() {
    thread_comments.sort_by(|a, b| {
      a.created_at
        .cmp(&b.created_at)
        .then_with(|| a.id.cmp(&b.id))
    });

    let root_comment = thread_comments
      .iter()
      .find(|comment| comment.in_reply_to_id.is_none())
      .copied()
      .or_else(|| thread_comments.first().copied());
    let Some(root_comment) = root_comment else {
      continue;
    };

    let body = root_comment.body.trim();
    let replies = thread_comments
      .iter()
      .copied()
      .filter(|comment| comment.id != root_comment.id)
      .map(|comment| GithubPrOverviewConversationReply {
        id: comment.id,
        timestamp: comment.created_at.clone(),
        author_login: comment.user.login.clone(),
        author_avatar_url: comment.user.avatar_url.clone(),
        body: {
          let body = comment.body.trim();
          if body.is_empty() {
            "No comment body.".to_string()
          } else {
            body.to_string()
          }
        },
      })
      .collect();
    let thread_comment_ids = thread_comments.iter().map(|comment| comment.id).collect();

    items.push(GithubPrOverviewConversationItem {
      kind: GithubPrOverviewConversationItemKind::ReviewComment,
      id: root_comment.id,
      timestamp: root_comment.created_at.clone(),
      author_login: root_comment.user.login.clone(),
      author_avatar_url: root_comment.user.avatar_url.clone(),
      body: Some(if body.is_empty() {
        "No comment body.".to_string()
      } else {
        body.to_string()
      }),
      review_state: None,
      replies,
      thread_comment_ids,
    });
  }

  items.sort_by(|a, b| {
    a.timestamp
      .cmp(&b.timestamp)
      .then_with(|| conversation_source_priority(a.kind).cmp(&conversation_source_priority(b.kind)))
      .then_with(|| a.id.cmp(&b.id))
  });

  items
}

fn pull_request_issue_comment_from_issue_details_comment(
  comment: GithubIssueDetailsComment,
) -> GithubPullRequestIssueComment {
  GithubPullRequestIssueComment {
    id: comment.id,
    body: comment.body.unwrap_or_default(),
    created_at: comment.created_at,
    updated_at: comment.updated_at,
    user: comment.user.map(|user| GithubPullRequestIssueCommentUser {
      login: user.login,
      avatar_url: user.avatar_url,
    }),
  }
}

fn next_overview_comment_body(raw_value: &str, initial_value: &str) -> Option<String> {
  let next_body = github_shared::normalize_non_empty_text(raw_value)?;
  let initial_body = initial_value.trim();
  if next_body == initial_body {
    None
  } else {
    Some(next_body)
  }
}

fn next_pr_description_body(raw_value: &str, initial_value: &str) -> Option<String> {
  github_shared::next_trimmed_text_update(raw_value, initial_value)
}

fn apply_pull_request_description_update_local(
  pull_request: &mut GithubPullRequestDetails,
  update: GithubPullRequestDescriptionUpdate,
) {
  pull_request.body = update.body;
  pull_request.updated_at = update.updated_at;
}

fn review_comment_owned_by_login(comment: &GithubPullRequestReviewComment, login: &str) -> bool {
  github_shared::logins_match_case_insensitive(comment.user.login.as_str(), login)
}

fn issue_comment_owned_by_login(comment: &GithubPullRequestIssueComment, login: &str) -> bool {
  comment
    .user
    .as_ref()
    .is_some_and(|user| github_shared::logins_match_case_insensitive(user.login.as_str(), login))
}

fn upsert_issue_comment_local(
  comments: &mut Vec<GithubPullRequestIssueComment>,
  comment: GithubPullRequestIssueComment,
) {
  if let Some(existing) = comments
    .iter_mut()
    .find(|existing| existing.id == comment.id)
  {
    *existing = comment;
    return;
  }
  comments.push(comment);
}

fn upsert_review_comment_local(
  comments: &mut Vec<GithubPullRequestReviewComment>,
  comment: GithubPullRequestReviewComment,
) {
  if let Some(existing) = comments
    .iter_mut()
    .find(|existing| existing.id == comment.id)
  {
    *existing = comment;
    return;
  }
  comments.push(comment);
}

fn upsert_review_local(
  reviews: &mut Vec<GithubPullRequestReview>,
  review: GithubPullRequestReview,
) {
  if let Some(existing) = reviews.iter_mut().find(|existing| existing.id == review.id) {
    *existing = review;
    return;
  }

  reviews.push(review);
}

fn remove_issue_comment_local(
  comments: &mut Vec<GithubPullRequestIssueComment>,
  comment_id: u64,
) -> Option<(usize, GithubPullRequestIssueComment)> {
  let (index, removed) = comments
    .iter()
    .enumerate()
    .find(|(_, comment)| comment.id == comment_id)
    .map(|(index, comment)| (index, comment.clone()))?;
  comments.remove(index);
  Some((index, removed))
}

fn restore_issue_comment_local(
  comments: &mut Vec<GithubPullRequestIssueComment>,
  index: usize,
  comment: GithubPullRequestIssueComment,
) {
  let insert_index = index.min(comments.len());
  comments.insert(insert_index, comment);
}

fn remove_review_comment_local(
  comments: &mut Vec<GithubPullRequestReviewComment>,
  comment_id: u64,
) -> Option<(usize, GithubPullRequestReviewComment)> {
  let (index, removed) = comments
    .iter()
    .enumerate()
    .find(|(_, comment)| comment.id == comment_id)
    .map(|(index, comment)| (index, comment.clone()))?;
  comments.remove(index);
  Some((index, removed))
}

fn restore_review_comment_local(
  comments: &mut Vec<GithubPullRequestReviewComment>,
  index: usize,
  comment: GithubPullRequestReviewComment,
) {
  let insert_index = index.min(comments.len());
  comments.insert(insert_index, comment);
}

fn is_last_review_thread_message(thread_comment_ids: &[u64], comment_id: u64) -> bool {
  thread_comment_ids.last().copied() == Some(comment_id)
}

fn allows_overview_review_reply_action(
  kind: GithubPrOverviewConversationItemKind,
  thread_comment_ids: &[u64],
  comment_id: u64,
) -> bool {
  kind == GithubPrOverviewConversationItemKind::ReviewComment
    && is_last_review_thread_message(thread_comment_ids, comment_id)
}

fn overview_root_is_editing(
  editing_target: Option<OverviewCommentTarget>,
  root_target: Option<OverviewCommentTarget>,
) -> bool {
  root_target.is_some() && editing_target == root_target
}

fn overview_conversation_scope_id(
  pr_number: u64,
  kind: GithubPrOverviewConversationItemKind,
  id: u64,
) -> usize {
  let kind_part = match kind {
    GithubPrOverviewConversationItemKind::IssueComment => 1usize,
    GithubPrOverviewConversationItemKind::Review => 2usize,
    GithubPrOverviewConversationItemKind::ReviewComment => 3usize,
  };
  (pr_number as usize)
    .wrapping_mul(1_000_003)
    .wrapping_add(kind_part.wrapping_mul(10_007))
    .wrapping_add(id as usize)
}

fn overview_change_stat_labels(pr: &GithubPullRequestDetails) -> [String; 2] {
  [format!("+{}", pr.additions), format!("-{}", pr.deletions)]
}

fn pr_changes_tab_count_label(changed_files: u64) -> SharedString {
  changed_files.to_string().into()
}

fn overview_change_stats(
  pr: &GithubPullRequestDetails,
  theme: &gpui_component::Theme,
) -> Vec<gpui::AnyElement> {
  let [additions, deletions] = overview_change_stat_labels(pr);
  vec![
    div()
      .text_sm()
      .font_medium()
      .text_color(theme.status_green())
      .child(additions)
      .into_any_element(),
    div()
      .text_sm()
      .font_medium()
      .text_color(theme.status_red())
      .child(deletions)
      .into_any_element(),
  ]
}

#[derive(Clone)]
struct GithubPrCommitSelectItem {
  sha: Option<String>,
  label: SharedString,
  search_text: String,
  is_selected: bool,
}

impl GithubPrCommitSelectItem {
  fn all_changes(is_selected: bool) -> Self {
    Self {
      sha: None,
      label: "All changes".into(),
      search_text: "all changes".to_string(),
      is_selected,
    }
  }

  fn for_commit(commit: &GithubPullRequestCommit, is_selected: bool) -> Self {
    let short = github_shared::short_sha(&commit.sha);
    let subject = commit_subject(&commit.message);
    let author = commit
      .author
      .as_ref()
      .map(|user| user.login.as_str())
      .unwrap_or("unknown");
    let search_text = format!("{short} {subject} {} {author}", commit.sha);
    Self {
      sha: Some(commit.sha.clone()),
      label: subject.into(),
      search_text: search_text.to_lowercase(),
      is_selected,
    }
  }
}

impl DropdownSelectItem for GithubPrCommitSelectItem {
  type Value = Option<String>;

  fn value(&self) -> &Self::Value {
    &self.sha
  }

  fn selected(&self) -> bool {
    self.is_selected
  }

  fn matches(&self, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
      return true;
    }

    self.search_text.contains(query.as_str())
  }

  fn render_item(&self, _window: &mut Window, _cx: &mut App) -> gpui::AnyElement {
    h_flex()
      .max_w(px(PR_COMMIT_SELECT_MENU_WIDTH - 40.0))
      .min_w_0()
      .text_sm()
      .child(
        div()
          .min_w_0()
          .overflow_hidden()
          .text_ellipsis()
          .child(self.label.clone()),
      )
      .into_any_element()
  }

  fn render_selected(&self, _window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    h_flex()
      .w_full()
      .min_w_0()
      .text_sm()
      .text_color(cx.theme().foreground)
      .child(
        div()
          .min_w_0()
          .text_ellipsis()
          .overflow_hidden()
          .child(self.label.clone()),
      )
      .into_any_element()
  }
}

struct GithubPrCommitListDelegate {
  all_rows: Vec<Rc<GithubPullRequestCommit>>,
  matched_rows: Vec<Rc<GithubPullRequestCommit>>,
  selected_index: Option<IndexPath>,
  selected_commit_sha: Option<SharedString>,
  query: SharedString,
}

impl GithubPrCommitListDelegate {
  fn new() -> Self {
    Self {
      all_rows: Vec::new(),
      matched_rows: Vec::new(),
      selected_index: None,
      selected_commit_sha: None,
      query: "".into(),
    }
  }

  fn set_rows(&mut self, commits: &[GithubPullRequestCommit], selected_commit_sha: Option<&str>) {
    self.all_rows = commits.iter().cloned().map(Rc::new).collect();
    self.selected_commit_sha = selected_commit_sha.map(|sha| sha.to_string().into());
    self.prepare(self.query.clone());
  }

  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();
    let q = self.query.as_ref();

    self.matched_rows = self
      .all_rows
      .iter()
      .filter(|commit| pull_request_commit_matches(commit, q))
      .cloned()
      .collect();

    self.selected_index = self
      .selected_commit_sha
      .as_ref()
      .and_then(|selected_sha| {
        self
          .matched_rows
          .iter()
          .position(|commit| commit.sha == selected_sha.as_ref())
          .map(IndexPath::new)
      })
      .or_else(|| (!self.matched_rows.is_empty()).then_some(IndexPath::new(0)));
  }

  fn row_at(&self, ix: IndexPath) -> Option<Rc<GithubPullRequestCommit>> {
    self.matched_rows.get(ix.row).cloned()
  }
}

impl ListDelegate for GithubPrCommitListDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.matched_rows.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = cx.theme().clone();
    let commit = self.matched_rows.get(ix.row)?;
    let subject = commit_subject(&commit.message);
    let short = github_shared::short_sha(&commit.sha);
    let author = commit
      .author
      .as_ref()
      .or(commit.committer.as_ref())
      .map(|user| user.login.as_str())
      .unwrap_or("unknown");
    let date_label = commit
      .committed_at
      .as_deref()
      .or(commit.authored_at.as_deref())
      .map(format_relative_time)
      .unwrap_or_else(|| "—".into());

    Some(
      selectable_list_item(
        ix,
        Some(ix) == self.selected_index,
        SelectableRowStyle::Inset,
        &theme,
      )
      .w_full()
      .px_3()
      .py_2()
      .child(
        v_flex()
          .gap_1()
          .child(
            div()
              .text_sm()
              .text_color(theme.foreground)
              .overflow_hidden()
              .text_ellipsis()
              .child(subject),
          )
          .child(
            h_flex()
              .gap_1()
              .items_center()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(
                Tag::secondary()
                  .small()
                  .rounded_full()
                  .text_color(theme.muted_foreground)
                  .child(short),
              )
              .child(author.to_string())
              .child("·")
              .child(date_label),
          ),
      ),
    )
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    v_flex()
      .size_full()
      .items_center()
      .justify_center()
      .text_sm()
      .text_color(cx.theme().muted_foreground)
      .child(if self.query.is_empty() {
        "No commits"
      } else {
        "No matching commits"
      })
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    self.selected_index = ix;
    cx.notify();
  }

  fn perform_search(
    &mut self,
    query: &str,
    _: &mut Window,
    _: &mut Context<ListState<Self>>,
  ) -> Task<()> {
    self.prepare(query.to_owned());
    Task::ready(())
  }
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

#[derive(Default)]
struct FileTreeNode {
  name: String,
  path: String,
  children: BTreeMap<String, FileTreeNode>,
  file: Option<()>,
}

impl FileTreeNode {
  fn new(name: String, path: String) -> Self {
    Self {
      name,
      path,
      children: BTreeMap::new(),
      file: None,
    }
  }

  fn is_folder(&self) -> bool {
    !self.children.is_empty()
  }
}

type FileTreeBuildResult<T> = (
  Vec<TreeItem>,
  HashMap<String, Rc<T>>,
  Option<usize>,
  Option<String>,
);

fn build_path_tree_items_with_expansion<T, F>(
  files: &[Rc<T>],
  path_for: F,
  expanded_folder_paths: Option<&HashSet<String>>,
) -> FileTreeBuildResult<T>
where
  F: Fn(&T) -> &str,
{
  fn insert_node(
    map: &mut BTreeMap<String, FileTreeNode>,
    parts: &[&str],
    prefix: &str,
    has_file: bool,
  ) {
    let Some((head, tail)) = parts.split_first() else {
      return;
    };

    let path = if prefix.is_empty() {
      head.to_string()
    } else {
      format!("{}/{}", prefix, head)
    };

    let node = map
      .entry(head.to_string())
      .or_insert_with(|| FileTreeNode::new(head.to_string(), path.clone()));

    if tail.is_empty() {
      if has_file {
        node.file = Some(());
      }
      return;
    }

    let node_path = node.path.clone();
    insert_node(&mut node.children, tail, &node_path, has_file);
  }

  let mut root: BTreeMap<String, FileTreeNode> = BTreeMap::new();
  let mut file_lookup: HashMap<String, Rc<T>> = HashMap::new();

  for file in files {
    let path = path_for(file.as_ref());
    file_lookup.insert(path.to_string(), file.clone());
    let parts: Vec<&str> = path.split('/').collect();
    insert_node(&mut root, &parts, "", true);
  }

  let mut order = Vec::new();
  let mut first_file_id: Option<String> = None;

  let mut root_nodes: Vec<FileTreeNode> = root.into_values().collect();
  root_nodes.sort_by(|a, b| {
    b.is_folder()
      .cmp(&a.is_folder())
      .then_with(|| a.name.cmp(&b.name))
  });

  let items = root_nodes
    .into_iter()
    .map(|node| build_tree_item(node, &mut order, &mut first_file_id, expanded_folder_paths))
    .collect::<Vec<_>>();

  let selected_index = first_file_id
    .as_ref()
    .and_then(|id| order.iter().position(|candidate| candidate == id));

  (items, file_lookup, selected_index, first_file_id)
}

fn build_path_tree_items<T, F>(files: &[Rc<T>], path_for: F) -> FileTreeBuildResult<T>
where
  F: Fn(&T) -> &str,
{
  build_path_tree_items_with_expansion(files, path_for, None)
}

fn build_tree_items(files: &[Rc<GithubPrFileDiff>]) -> FileTreeBuildResult<GithubPrFileDiff> {
  build_path_tree_items(files, |file| file.path.as_ref())
}

fn build_local_project_tree_items(
  files: &[Rc<GithubPrLocalProjectFile>],
) -> FileTreeBuildResult<GithubPrLocalProjectFile> {
  build_path_tree_items(files, |file| file.path.as_ref())
}

fn expanded_folder_paths_for_changed_files<'a, I>(paths: I) -> HashSet<String>
where
  I: IntoIterator<Item = &'a str>,
{
  let mut expanded = HashSet::new();
  for path in paths {
    let mut prefix = String::new();
    let parts = path.split('/').collect::<Vec<_>>();
    for folder in parts.iter().take(parts.len().saturating_sub(1)) {
      if prefix.is_empty() {
        prefix.push_str(folder);
      } else {
        prefix.push('/');
        prefix.push_str(folder);
      }
      expanded.insert(prefix.clone());
    }
  }
  expanded
}

fn build_tree_items_from_paths(
  paths: &[String],
  expanded_folder_paths: Option<&HashSet<String>>,
) -> (Vec<TreeItem>, Option<usize>, Option<String>) {
  let files = paths
    .iter()
    .map(|path| {
      Rc::new(GithubPrLocalProjectFile {
        path: path.clone().into(),
      })
    })
    .collect::<Vec<_>>();
  let (items, _, selected_index, selected_id) =
    build_path_tree_items_with_expansion(&files, |file| file.path.as_ref(), expanded_folder_paths);
  (items, selected_index, selected_id)
}

fn build_search_file_entry(path: &str) -> SearchFileEntry {
  let label = path.replace(['\n', '\r'], "");
  SearchFileEntry::new(PathBuf::from(label.clone()), label)
}

fn searchable_text_from_pr_file_contents<'a>(
  file: &GithubPrFileDiff,
  contents: &'a GithubPrFileContents,
) -> Option<&'a str> {
  match file.status {
    GithubPrFileStatus::Deleted => contents.base.as_deref().or(contents.head.as_deref()),
    _ => contents.head.as_deref().or(contents.base.as_deref()),
  }
}

fn fetch_pr_file_search_contents(
  api: &ApiClient,
  diff_refs: &GithubPrDiffRefs,
  file: &GithubPrFileDiff,
) -> GithubPrFileContents {
  match file.status {
    GithubPrFileStatus::Deleted => GithubPrFileContents {
      base: file
        .old_path
        .as_ref()
        .or(Some(&file.path))
        .and_then(|path| {
          api
            .fetch_github_file_content(
              &diff_refs.base_owner,
              &diff_refs.base_repo,
              path.as_ref(),
              &diff_refs.base_sha,
            )
            .ok()
            .flatten()
        }),
      head: None,
    },
    _ => GithubPrFileContents {
      base: None,
      head: api
        .fetch_github_file_content(
          &diff_refs.head_owner,
          &diff_refs.head_repo,
          file.path.as_ref(),
          &diff_refs.head_sha,
        )
        .ok()
        .flatten(),
    },
  }
}

fn perform_tree_text_search(
  query: &str,
  scope_paths: &[String],
  pr_files: &HashMap<String, GithubPrFileDiff>,
  cached_file_contents: &HashMap<String, GithubPrFileContents>,
  diff_refs: Option<&GithubPrDiffRefs>,
  api: &ApiClient,
  local_repo_root: Option<&Path>,
) -> GithubPrTreeSearchResult {
  let mut result = GithubPrTreeSearchResult::default();
  let mut local_scope_paths = Vec::new();

  for path in scope_paths {
    if let Some(file) = pr_files.get(path) {
      let contents = if let Some(contents) = cached_file_contents.get(path).cloned() {
        contents
      } else if let Some(diff_refs) = diff_refs {
        let fetched = fetch_pr_file_search_contents(api, diff_refs, file);
        result
          .updated_file_contents
          .insert(path.clone(), fetched.clone());
        fetched
      } else {
        continue;
      };

      if searchable_text_from_pr_file_contents(file, &contents)
        .is_some_and(|contents| contents.to_lowercase().contains(query))
      {
        result.matches.insert(path.clone());
      }
      continue;
    }

    if local_repo_root.is_some() {
      local_scope_paths.push(PathBuf::from(path));
    }
  }

  if let Some(local_repo_root) = local_repo_root
    && !local_scope_paths.is_empty()
  {
    match search_repo_head_contents(local_repo_root, &local_scope_paths, query) {
      Ok(matches) => {
        result.matches.extend(
          matches
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned()),
        );
      }
      Err(error) => {
        result.error = Some(format!("Failed to search local files: {error}"));
      }
    }
  }

  result
}

fn build_tree_item(
  node: FileTreeNode,
  order: &mut Vec<String>,
  first_file_id: &mut Option<String>,
  expanded_folder_paths: Option<&HashSet<String>>,
) -> TreeItem {
  let mut child_nodes: Vec<FileTreeNode> = node.children.into_values().collect();
  child_nodes.sort_by(|a, b| {
    b.is_folder()
      .cmp(&a.is_folder())
      .then_with(|| a.name.cmp(&b.name))
  });

  let mut item = TreeItem::new(node.path.clone(), node.name.clone());
  if !child_nodes.is_empty() {
    let children = child_nodes
      .into_iter()
      .map(|child| build_tree_item(child, order, first_file_id, expanded_folder_paths))
      .collect::<Vec<_>>();
    let is_expanded = expanded_folder_paths
      .map(|paths| paths.contains(&node.path))
      .unwrap_or(true);
    item = item.children(children).expanded(is_expanded);
  }

  order.push(node.path.clone());
  if node.file.is_some() && first_file_id.is_none() {
    *first_file_id = Some(node.path.clone());
  }

  item
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
  commits_list: Entity<ListState<GithubPrCommitListDelegate>>,
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
  merge_form_reset_pending: bool,
  merge_method: GithubPullRequestMergeMethod,
  merge_commit_title_input: Entity<InputState>,
  merge_commit_message_input: Entity<InputState>,
  merge_submit_task: Option<Task<()>>,
  merge_submit_loading: bool,
  merge_submit_error: Option<SharedString>,
  status_action_task: Option<Task<()>>,
  status_action_loading: bool,
  review_input: Entity<InputState>,
  review_decision: GithubPrReviewDecision,
  review_popover_open: bool,
  review_form_reset_pending: bool,
  submit_review_task: Option<Task<()>>,
  submit_review_loading: bool,
  submit_review_error: Option<SharedString>,
  issue_comments_task: Option<Task<()>>,
  issue_comments_loading: bool,
  issue_comments_error: Option<SharedString>,
  issue_comments: Vec<GithubPullRequestIssueComment>,
  overview_issue_comment_input: Entity<InputState>,
  overview_issue_comment_submitting: bool,
  overview_issue_comment_error: Option<SharedString>,
  reviews_task: Option<Task<()>>,
  reviews_loading: bool,
  reviews_error: Option<SharedString>,
  reviews: Vec<GithubPullRequestReview>,
  review_comments_task: Option<Task<()>>,
  review_comments_loading: bool,
  review_comments_error: Option<SharedString>,
  review_comments: Vec<GithubPullRequestReviewComment>,
  overview_edit_input: Option<Entity<InputState>>,
  overview_edit_target: Option<OverviewCommentTarget>,
  overview_edit_initial_body: Option<String>,
  overview_edit_submitting: bool,
  overview_edit_error: Option<SharedString>,
  overview_reply_input: Option<Entity<InputState>>,
  overview_reply_target_comment_id: Option<u64>,
  overview_reply_submitting: bool,
  overview_reply_error: Option<SharedString>,
  selected_file_review_comment_ids: Vec<u64>,
  active_review_comment_id: Option<u64>,
  review_comment_handlers_enabled: bool,
  description_code_reference_requests: Vec<GithubBlobLineReference>,
  review_comment_code_reference_cache: HashMap<String, Option<ReviewCommentCodeReferencePreview>>,
  review_comment_code_reference_tasks: HashMap<String, Task<()>>,
  pending_review_comment_link_comment_id: Option<u64>,
  pr_description_edit_input: Option<Entity<InputState>>,
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
  overview_conversation_items: Vec<GithubPrOverviewConversationItem>,
  overview_list: GpuiListState,
  overview_list_count: usize,
  svg_preview: Option<Result<Arc<RenderImage>, SharedString>>,
  svg_preview_source: Option<SharedString>,
  svg_preview_task: Option<Task<()>>,
  active_tab_ix: usize,
  current_pr_context: Option<CurrentPrContext>,
  back_target: GithubPrBackTarget,
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
  EnsurePrHeadThenOpenGitPageMergeBase { base_branch_name: String },
  OpenGitPageMergeBase { base_branch_name: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GithubPrBackTarget {
  GithubHome,
  Repo {
    owner: SharedString,
    repo: SharedString,
  },
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

fn resolve_pr_back_target(owner: SharedString, repo: SharedString) -> GithubPrBackTarget {
  if owner.as_ref().trim().is_empty() || repo.as_ref().trim().is_empty() {
    GithubPrBackTarget::GithubHome
  } else {
    GithubPrBackTarget::Repo { owner, repo }
  }
}

fn local_repo_matches_pull_request(
  pull_request: &GithubPullRequestDetails,
  local_repo: &ActiveLocalRepo,
) -> bool {
  let source_repo = GithubPrDetailsPage::pr_source_repository(pull_request);
  let Some(local_owner) = local_repo.github_owner.as_deref() else {
    return false;
  };
  let Some(local_name) = local_repo.github_repo.as_deref() else {
    return false;
  };

  local_owner.eq_ignore_ascii_case(source_repo.owner.as_str())
    && local_name.eq_ignore_ascii_case(source_repo.repo.as_str())
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

fn local_repo_has_active_conflict_resolution(repo_root: &Path) -> bool {
  if is_merge_in_progress(repo_root).unwrap_or(false)
    || is_rebase_in_progress(repo_root).unwrap_or(false)
  {
    return true;
  }

  list_repo_status(repo_root)
    .map(|entries| {
      entries
        .iter()
        .any(|entry| entry.status == RepoStatusKind::Conflicted)
    })
    .unwrap_or(false)
}

fn should_prepare_local_branch_before_opening_git_page(
  repo_root: &Path,
  has_uncommitted_changes: bool,
) -> bool {
  has_uncommitted_changes && !local_repo_has_active_conflict_resolution(repo_root)
}

fn find_matching_recent_local_repo(
  pull_request: &GithubPullRequestDetails,
  excluded_repo_root: Option<&Path>,
) -> Option<ActiveLocalRepo> {
  ConfigStore::load_recent_repositories()
    .into_iter()
    .filter(|repo| {
      excluded_repo_root
        .map(|excluded| excluded != repo.path.as_path())
        .unwrap_or(true)
    })
    .filter(|repo| repo.path.is_dir())
    .find_map(|repo| {
      let snapshot = local_repo_snapshot(repo.path.as_path(), None)?;
      local_repo_matches_pull_request(pull_request, &snapshot).then_some(snapshot)
    })
}

fn next_back_target_for_pr_palette(back_target: &GithubPrBackTarget) -> GithubPrBackTarget {
  match back_target {
    GithubPrBackTarget::GithubHome => GithubPrBackTarget::GithubHome,
    GithubPrBackTarget::Repo { owner, repo } => resolve_pr_back_target(owner.clone(), repo.clone()),
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

  pub fn show(owner: SharedString, repo: SharedString, number: u64, cx: &mut App) {
    Self::show_with_back_target_and_open_target(
      owner,
      repo,
      number,
      GithubPrBackTarget::GithubHome,
      GithubPrOpenTarget::default(),
      cx,
    );
  }

  pub fn show_with_open_target(
    owner: SharedString,
    repo: SharedString,
    number: u64,
    open_changes_tab: bool,
    review_comment_id: Option<u64>,
    cx: &mut App,
  ) {
    Self::show_with_back_target_and_open_target(
      owner,
      repo,
      number,
      GithubPrBackTarget::GithubHome,
      GithubPrOpenTarget::new(open_changes_tab, review_comment_id),
      cx,
    );
  }

  pub fn show_with_repo_return(
    owner: SharedString,
    repo: SharedString,
    number: u64,
    return_owner: SharedString,
    return_repo: SharedString,
    cx: &mut App,
  ) {
    Self::show_with_back_target_and_open_target(
      owner,
      repo,
      number,
      resolve_pr_back_target(return_owner, return_repo),
      GithubPrOpenTarget::default(),
      cx,
    );
  }

  pub(crate) fn show_with_repo_return_open_target(
    owner: SharedString,
    repo: SharedString,
    number: u64,
    return_owner: SharedString,
    return_repo: SharedString,
    open_target: GithubPrOpenTarget,
    cx: &mut App,
  ) {
    Self::show_with_back_target_and_open_target(
      owner,
      repo,
      number,
      resolve_pr_back_target(return_owner, return_repo),
      open_target,
      cx,
    );
  }

  pub fn refresh(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().page.clone() else {
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

  fn show_with_back_target_and_open_target(
    owner: SharedString,
    repo: SharedString,
    number: u64,
    back_target: GithubPrBackTarget,
    open_target: GithubPrOpenTarget,
    cx: &mut App,
  ) {
    let Some(weak) = cx.global::<Self>().page.clone() else {
      return;
    };

    let owner_string = owner.to_string();
    let repo_string = repo.to_string();
    let back_target_value = back_target.clone();
    let open_target_value = open_target;
    let _ = weak.update(cx, |this, cx| {
      this.back_target = back_target_value.clone();
      this.load_pull_request(owner_string, repo_string, number, open_target_value, cx);
    });

    NavigationHistory::navigate(crate::navigation::build_pr_path(&owner, &repo, number), cx);
  }
}

impl GithubPrDetailsPage {
  fn build_detached_diff_editor(
    path: impl Into<PathBuf>,
    cx: &mut Context<Self>,
  ) -> Entity<Editor> {
    let editor_path = path.into();
    let load_root = PathBuf::from(".");
    let load_path = PathBuf::from(".reviu-github-pr-preview").join(&editor_path);
    let loaded = Editor::load_file_for_editor(&load_root, &load_path);
    let detached_root = PathBuf::from(".reviu-github-pr-editor-root");

    cx.new(move |cx| {
      let mut editor = Editor::new_with_loaded_file(detached_root, editor_path, loaded, cx);
      editor.is_read_only = true;
      editor
    })
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
    let commits_list = cx.new(|cx| ListState::new(GithubPrCommitListDelegate::new(), window, cx));
    let diff_editor = Self::build_detached_diff_editor("__reviu_github_pr_placeholder__.diff", cx);
    let merge_commit_title_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Commit title (optional)"));
    let merge_commit_message_input = cx.new(|cx| {
      InputState::new(window, cx)
        .multi_line(true)
        .rows(5)
        .placeholder("Commit message (optional)")
    });
    let review_input = cx.new(|cx| {
      InputState::new(window, cx)
        .multi_line(true)
        .placeholder("Add an overall review comment...")
    });
    let overview_issue_comment_input = cx.new(|cx| {
      InputState::new(window, cx)
        .multi_line(true)
        .rows(6)
        .placeholder("Add comment...")
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
      commits_list,
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
      merge_form_reset_pending: true,
      merge_method: GithubPullRequestMergeMethod::Merge,
      merge_commit_title_input,
      merge_commit_message_input,
      merge_submit_task: None,
      merge_submit_loading: false,
      merge_submit_error: None,
      status_action_task: None,
      status_action_loading: false,
      review_input,
      review_decision: GithubPrReviewDecision::default(),
      review_popover_open: false,
      review_form_reset_pending: false,
      submit_review_task: None,
      submit_review_loading: false,
      submit_review_error: None,
      issue_comments_task: None,
      issue_comments_loading: false,
      issue_comments_error: None,
      issue_comments: Vec::new(),
      overview_issue_comment_input,
      overview_issue_comment_submitting: false,
      overview_issue_comment_error: None,
      reviews_task: None,
      reviews_loading: false,
      reviews_error: None,
      reviews: Vec::new(),
      review_comments_task: None,
      review_comments_loading: false,
      review_comments_error: None,
      review_comments: Vec::new(),
      overview_edit_input: None,
      overview_edit_target: None,
      overview_edit_initial_body: None,
      overview_edit_submitting: false,
      overview_edit_error: None,
      overview_reply_input: None,
      overview_reply_target_comment_id: None,
      overview_reply_submitting: false,
      overview_reply_error: None,
      selected_file_review_comment_ids: Vec::new(),
      active_review_comment_id: None,
      review_comment_handlers_enabled: true,
      description_code_reference_requests: Vec::new(),
      review_comment_code_reference_cache: HashMap::new(),
      review_comment_code_reference_tasks: HashMap::new(),
      pending_review_comment_link_comment_id: None,
      pr_description_edit_input: None,
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
      overview_conversation_items: Vec::new(),
      overview_list: GpuiListState::new(0, ListAlignment::Top, px(300.)),
      overview_list_count: 0,
      svg_preview: None,
      svg_preview_source: None,
      svg_preview_task: None,
      active_tab_ix: 0,
      current_pr_context: None,
      back_target: GithubPrBackTarget::GithubHome,
      pull_request: None,
      error: None,
    };
    this.install_diff_editor_review_comment_handlers(cx);
    this.sync_commits_list(cx);
    this.subscribe_to_commits_list(window, cx);
    this.subscribe_to_tree_search_input(window, cx);
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

  fn pr_source_repository(pull_request: &GithubPullRequestDetails) -> &GithubRepository {
    pull_request
      .head_repository
      .as_ref()
      .unwrap_or(&pull_request.repository)
  }

  fn active_local_repo_for_pull_request(&self, cx: &App) -> Option<ActiveLocalRepo> {
    let pull_request = self.pull_request.as_ref()?;
    let local_repo = ActiveLocalRepoStore::get(cx)?;
    local_repo_matches_pull_request(pull_request, &local_repo).then_some(local_repo)
  }

  fn effective_local_repo_for_pull_request(&self, cx: &App) -> Option<ActiveLocalRepo> {
    self
      .active_local_repo_for_pull_request(cx)
      .or_else(|| self.resolved_local_repo.clone())
  }

  fn local_project_availability_for_repo(
    pull_request: &GithubPullRequestDetails,
    local_repo: ActiveLocalRepo,
  ) -> GithubPrLocalProjectAvailability {
    if local_repo.current_branch.as_deref() != Some(pull_request.head_ref_name.as_str()) {
      return GithubPrLocalProjectAvailability::NeedsBranchSwitch {
        repo_root: local_repo.repo_root,
        current_branch: local_repo.current_branch,
        has_uncommitted_changes: local_repo.has_uncommitted_changes,
      };
    }

    let Some(local_head_sha) = local_repo.head_sha.as_deref() else {
      return GithubPrLocalProjectAvailability::Hidden;
    };
    if local_head_sha == pull_request.head_sha {
      return GithubPrLocalProjectAvailability::Ready {
        repo_root: local_repo.repo_root,
      };
    }

    if local_repo.has_uncommitted_changes {
      GithubPrLocalProjectAvailability::Dirty {
        repo_root: local_repo.repo_root,
      }
    } else {
      GithubPrLocalProjectAvailability::NeedsUpdate {
        repo_root: local_repo.repo_root,
      }
    }
  }

  fn maybe_refresh_resolved_local_repo_match(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_commit_sha.is_some() || self.pull_request.is_none() {
      return;
    }

    if self.active_local_repo_for_pull_request(cx).is_some()
      || self.resolved_local_repo_scan_complete
      || self.resolved_local_repo_task.is_some()
    {
      return;
    }

    cx.defer_in(window, |this, _window, cx| {
      this.refresh_resolved_local_repo_match(cx);
    });
  }

  fn refresh_resolved_local_repo_match(&mut self, cx: &mut Context<Self>) {
    if self.resolved_local_repo_task.is_some() {
      return;
    }

    let Some(pull_request) = self.pull_request.clone() else {
      self.resolved_local_repo = None;
      self.resolved_local_repo_task = None;
      self.resolved_local_repo_scan_complete = false;
      return;
    };

    if self.active_local_repo_for_pull_request(cx).is_some() {
      self.resolved_local_repo = None;
      self.resolved_local_repo_task = None;
      self.resolved_local_repo_scan_complete = false;
      return;
    }

    self.resolved_local_repo_generation = self.resolved_local_repo_generation.wrapping_add(1);
    let generation = self.resolved_local_repo_generation;
    self.resolved_local_repo = None;
    self.resolved_local_repo_scan_complete = false;

    let excluded_repo_root = ActiveLocalRepoStore::get(cx).map(|repo| repo.repo_root);
    let task = cx.spawn(async move |this, cx| {
      let pull_request_for_scan = pull_request.clone();
      let excluded_repo_root_for_scan = excluded_repo_root.clone();
      let snapshot = unblock(move || {
        find_matching_recent_local_repo(
          &pull_request_for_scan,
          excluded_repo_root_for_scan.as_deref(),
        )
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        if this.resolved_local_repo_generation != generation {
          return;
        }

        this.resolved_local_repo_task = None;
        this.resolved_local_repo_scan_complete = true;
        this.resolved_local_repo = snapshot;

        cx.notify();
      });
    });

    self.resolved_local_repo_task = Some(task);
  }

  fn sync_active_local_repo_store_snapshot(
    &self,
    snapshot: &ActiveLocalRepo,
    cx: &mut Context<Self>,
  ) {
    if ActiveLocalRepoStore::get(cx)
      .as_ref()
      .map(|repo| repo.repo_root.as_path())
      == Some(snapshot.repo_root.as_path())
    {
      ActiveLocalRepoStore::set(cx, Some(snapshot.clone()));
    }
  }

  fn sync_resolved_local_repo_snapshot(&mut self, snapshot: &ActiveLocalRepo) {
    let Some(pull_request) = self.pull_request.as_ref() else {
      self.resolved_local_repo = None;
      self.resolved_local_repo_scan_complete = false;
      return;
    };

    if local_repo_matches_pull_request(pull_request, snapshot) {
      self.resolved_local_repo = Some(snapshot.clone());
      self.resolved_local_repo_scan_complete = true;
    } else {
      self.resolved_local_repo = None;
      self.resolved_local_repo_scan_complete = false;
    }
  }

  fn local_project_availability(&self, cx: &App) -> GithubPrLocalProjectAvailability {
    if self.selected_commit_sha.is_some() {
      return GithubPrLocalProjectAvailability::Hidden;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return GithubPrLocalProjectAvailability::Hidden;
    };
    let Some(local_repo) = self.effective_local_repo_for_pull_request(cx) else {
      return GithubPrLocalProjectAvailability::Hidden;
    };

    Self::local_project_availability_for_repo(pull_request, local_repo)
  }

  fn effective_local_repo_has_uncommitted_changes(&self, cx: &App) -> bool {
    self
      .effective_local_repo_for_pull_request(cx)
      .is_some_and(|repo| repo.has_uncommitted_changes)
  }

  fn local_project_mode_active(&self, cx: &App) -> bool {
    self.show_local_project_files
      && matches!(
        self.local_project_availability(cx),
        GithubPrLocalProjectAvailability::Ready { .. }
      )
  }

  fn tree_search_query_normalized(&self) -> Option<String> {
    let query = self.tree_search_query.trim();
    (!query.is_empty()).then(|| query.to_lowercase())
  }

  fn search_scope_paths(&self, cx: &App) -> Vec<String> {
    if self.local_project_mode_active(cx) {
      let mut paths = BTreeSet::new();
      paths.extend(self.local_project_lookup.keys().cloned());
      paths.extend(self.file_lookup.keys().cloned());
      return paths.into_iter().collect();
    }

    let mut paths = self.file_lookup.keys().cloned().collect::<Vec<_>>();
    paths.sort();
    paths
  }

  fn visible_tree_paths(&self, cx: &App) -> Vec<String> {
    let mut paths = self.search_scope_paths(cx);
    if self.tree_search_query_normalized().is_some()
      && let Some(matches) = self.tree_search_matches.as_ref()
    {
      paths.retain(|path| matches.contains(path));
    }
    paths
  }

  fn active_file_count(&self, cx: &App) -> usize {
    self.visible_tree_paths(cx).len()
  }

  fn active_file_search_entries(&self, cx: &App) -> Vec<SearchFileEntry> {
    let mut entries = self
      .visible_tree_paths(cx)
      .into_iter()
      .map(|path| build_search_file_entry(path.as_str()))
      .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.label.as_ref().cmp(b.label.as_ref()));
    entries
  }

  fn current_selected_tree_path(&self) -> Option<String> {
    self
      .selected_file
      .as_ref()
      .map(|file| file.path.to_string())
      .or_else(|| {
        self
          .selected_local_project_file
          .as_ref()
          .map(|file| file.path.to_string())
      })
  }

  fn select_visible_tree_path(&mut self, path: &str, cx: &mut Context<Self>) {
    if let Some(file) = self.file_lookup.get(path).cloned() {
      self.set_selected_file(Some(file), cx);
      return;
    }

    if let Some(file) = self.local_project_lookup.get(path).cloned() {
      self.set_selected_local_project_file(Some(file), cx);
      return;
    }

    if self.selected_file.is_some() {
      self.set_selected_file(None, cx);
    } else if self.selected_local_project_file.is_some() {
      self.set_selected_local_project_file(None, cx);
    }
  }

  fn set_tree_items_with_selection(
    &mut self,
    items: Vec<TreeItem>,
    preferred_id: Option<String>,
    fallback_index: Option<usize>,
    cx: &mut Context<Self>,
  ) -> Option<String> {
    let mut resolved_id = None;
    self.tree_state.update(cx, |state, cx| {
      state.set_items(items, cx);

      if let Some(preferred_id) = preferred_id.as_ref() {
        let tree_item = TreeItem::new(preferred_id.clone(), preferred_id.clone());
        state.set_selected_item(Some(&tree_item), cx);
        if let Some(ix) = state.selected_index() {
          state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
          resolved_id = Some(preferred_id.clone());
          return;
        }
      }

      state.set_selected_index(fallback_index, cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
        resolved_id = state
          .selected_entry()
          .map(|entry| entry.item().id.to_string());
      }
    });
    resolved_id
  }

  fn sync_changes_tree_state(&mut self, cx: &mut Context<Self>) {
    let visible_paths = self.visible_tree_paths(cx);
    let expanded_folder_paths = self.local_project_mode_active(cx).then(|| {
      expanded_folder_paths_for_changed_files(self.file_lookup.keys().map(|path| path.as_str()))
    });
    let (items, fallback_index, fallback_id) =
      build_tree_items_from_paths(&visible_paths, expanded_folder_paths.as_ref());
    let preferred_id = self
      .saved_pr_selected_tree_id
      .clone()
      .or_else(|| self.current_selected_tree_path())
      .filter(|id| visible_paths.contains(id))
      .or(fallback_id);
    let resolved_id = self.set_tree_items_with_selection(items, preferred_id, fallback_index, cx);
    self.saved_pr_selected_tree_id = None;
    match resolved_id {
      Some(path) => self.select_visible_tree_path(path.as_str(), cx),
      None => {
        self.selected_tree_id = None;
        self.selected_local_project_tree_id = None;
        if self.selected_file.is_some() {
          self.set_selected_file(None, cx);
        } else if self.selected_local_project_file.is_some() {
          self.set_selected_local_project_file(None, cx);
        }
      }
    }
  }

  fn sync_local_project_tree_state(&mut self, cx: &mut Context<Self>) {
    self.sync_changes_tree_state(cx);
  }

  fn maybe_load_local_project_files_if_needed(&mut self, repo_root: &Path, cx: &mut Context<Self>) {
    if self.local_project_loaded_repo_root.as_deref() == Some(repo_root) {
      if self.local_project_tree_loading || self.local_project_files_task.is_some() {
        return;
      }
      if !self.local_project_lookup.is_empty() || self.local_project_tree_error.is_some() {
        self.sync_local_project_tree_state(cx);
        return;
      }
    }

    self.load_local_project_files(repo_root.to_path_buf(), cx);
  }

  fn load_local_project_files(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    if self.local_project_loaded_repo_root.as_ref() == Some(&repo_root)
      && (self.local_project_tree_loading || self.local_project_files_task.is_some())
    {
      return;
    }

    self.local_project_loaded_repo_root = Some(repo_root.clone());
    self.local_project_tree_loading = true;
    self.local_project_tree_error = None;
    self.local_project_files_task = None;
    self.local_project_lookup.clear();
    self.selected_local_project_file = None;
    self.selected_local_project_tree_id = None;
    self.local_project_open_file_task = None;
    self.local_project_open_file_generation =
      self.local_project_open_file_generation.wrapping_add(1);

    if self.show_local_project_files {
      self.tree_state.update(cx, |state, cx| {
        state.set_items(Vec::new(), cx);
        state.set_selected_index(None, cx);
      });
    }

    let requested_repo_root = repo_root.clone();
    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_load = requested_repo_root.clone();
      let result = unblock(move || list_repo_head_files(&repo_root_for_load)).await;

      let _ = this.update(cx, |this, cx| {
        if this.local_project_loaded_repo_root.as_ref() != Some(&requested_repo_root) {
          return;
        }

        this.local_project_files_task = None;
        this.local_project_tree_loading = false;

        match result {
          Ok(paths) => {
            let files = paths
              .into_iter()
              .map(|path| {
                Rc::new(GithubPrLocalProjectFile {
                  path: path.to_string_lossy().replace(['\n', '\r'], "").into(),
                })
              })
              .collect::<Vec<_>>();
            let (_, lookup, _, _) = build_local_project_tree_items(&files);
            this.local_project_lookup = lookup;
            this.local_project_tree_error = None;
            this.refresh_tree_text_search(cx);
          }
          Err(error) => {
            this.local_project_lookup.clear();
            this.local_project_tree_error = Some(error.to_string().into());
            if this.local_project_mode_active(cx)
              && this.selected_local_project_file.is_some()
              && this.selected_file.is_none()
            {
              this.set_selected_local_project_file(None, cx);
            }
            this.refresh_tree_text_search(cx);
          }
        }

        cx.notify();
      });
    });

    self.local_project_files_task = Some(task);
    cx.notify();
  }

  fn sync_local_project_tree_selection(&mut self, cx: &mut Context<Self>) {
    let Some(file) = self.selected_local_project_file.as_ref() else {
      return;
    };

    let key = file.path.as_ref().to_string();
    let tree_item = TreeItem::new(key.clone(), key.clone());
    self.tree_state.update(cx, |state, cx| {
      state.set_selected_item(Some(&tree_item), cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
    });
  }

  fn load_local_project_snapshot_into_diff_editor(
    &mut self,
    file_path: PathBuf,
    contents: String,
    cx: &mut Context<Self>,
  ) {
    self.diff_editor = Self::build_detached_diff_editor(file_path, cx);
    self.diff_editor.update(cx, |editor, cx| {
      editor.load_readonly_snapshot(contents, None, cx);
      editor.reset_after_replace();
      editor.reset_selection(cx);
    });
  }

  fn set_selected_local_project_file(
    &mut self,
    selected: Option<Rc<GithubPrLocalProjectFile>>,
    cx: &mut Context<Self>,
  ) {
    let current_id = self
      .selected_local_project_file
      .as_ref()
      .map(|file| file.path.clone());
    let next_id = selected.as_ref().map(|file| file.path.clone());
    if current_id == next_id {
      return;
    }

    self.selected_local_project_file = selected.clone();
    self.selected_local_project_tree_id = selected.as_ref().map(|file| file.path.to_string());
    self.selected_file = None;
    self.selected_tree_id = None;
    self.active_review_comment_id = None;
    self.selected_file_review_comment_ids.clear();
    self.sync_sentry_pr_context();
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      self.show_markdown_preview = false;
    }
    self.binary_preview = None;
    self.svg_preview = None;
    self.svg_preview_source = None;
    self.file_error = None;
    self.local_project_open_file_generation =
      self.local_project_open_file_generation.wrapping_add(1);
    self.local_project_open_file_task = None;

    let Some(file) = selected else {
      self.file_loading = false;
      self.clear_diff_editor(cx);
      self.sync_diff_view(cx);
      self.sync_review_comments(cx);
      cx.notify();
      return;
    };

    let Some(repo_root) = self.local_project_loaded_repo_root.clone() else {
      self.file_loading = false;
      self.file_error = Some("Local project unavailable".into());
      self.clear_diff_editor(cx);
      self.sync_review_comments(cx);
      cx.notify();
      return;
    };

    self.sync_local_project_tree_selection(cx);
    self.file_loading = true;
    self.file_error = None;
    self.clear_diff_editor(cx);

    let generation = self.local_project_open_file_generation;
    let requested_repo_root = repo_root.clone();
    let requested_rel_path = PathBuf::from(file.path.as_ref());
    let requested_key = file.path.to_string();
    let requested_absolute_path = requested_repo_root.join(&requested_rel_path);
    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_load = requested_repo_root.clone();
      let absolute_path_for_load = requested_absolute_path.clone();
      let rel_path_for_load = requested_rel_path.clone();
      let (snapshot_contents, binary_bytes) = unblock(move || {
        let loaded = Editor::load_file_for_editor(&repo_root_for_load, &absolute_path_for_load);
        let git_store = GitStore::new(repo_root_for_load.clone());
        let head_contents = git_store
          .load_bases(rel_path_for_load.as_path())
          .ok()
          .and_then(|bases| bases.head);
        let head_binary_bytes = git_store
          .load_binary_bases(rel_path_for_load.as_path())
          .ok()
          .and_then(|bases| bases.head);
        (
          head_contents.unwrap_or(loaded.content),
          head_binary_bytes.or(loaded.binary_bytes),
        )
      })
      .await;

      let _ = this.update(cx, move |this, cx| {
        if this.local_project_open_file_generation != generation {
          return;
        }
        if this.local_project_loaded_repo_root.as_ref() != Some(&requested_repo_root) {
          return;
        }
        if this
          .selected_local_project_file
          .as_ref()
          .map(|file| file.path.as_ref())
          != Some(requested_key.as_str())
        {
          return;
        }

        this.load_local_project_snapshot_into_diff_editor(
          requested_rel_path.clone(),
          snapshot_contents,
          cx,
        );
        this.binary_preview =
          Self::build_binary_preview(requested_rel_path.as_path(), binary_bytes.clone());
        this.file_loading = false;
        this.file_error = None;
        this.sync_diff_view(cx);
        cx.notify();
      });
    });
    self.local_project_open_file_task = Some(task);
    self.sync_review_comments(cx);
    cx.notify();
  }

  fn selected_local_project_file_is_markdown(&self) -> bool {
    self
      .selected_local_project_file
      .as_ref()
      .map(|file| is_markdown_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  fn selected_local_project_file_is_svg(&self) -> bool {
    self
      .selected_local_project_file
      .as_ref()
      .map(|file| is_svg_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  fn set_show_local_project_files(&mut self, enabled: bool, cx: &mut Context<Self>) {
    if self.show_local_project_files == enabled {
      return;
    }

    let previous_selection = self.current_selected_tree_path();

    if enabled {
      let GithubPrLocalProjectAvailability::Ready { repo_root } =
        self.local_project_availability(cx)
      else {
        return;
      };
      self.saved_pr_selected_tree_id = previous_selection;
      self.show_local_project_files = true;
      self.local_project_update_error = None;
      self.maybe_load_local_project_files_if_needed(repo_root.as_path(), cx);
      if !self.local_project_tree_loading {
        self.refresh_tree_text_search(cx);
      }
      cx.notify();
      return;
    }

    self.saved_pr_selected_tree_id = previous_selection;
    self.show_local_project_files = false;
    self.local_project_open_file_task = None;
    self.local_project_open_file_generation =
      self.local_project_open_file_generation.wrapping_add(1);
    self.file_loading = false;
    self.file_error = None;
    self.binary_preview = None;
    self.refresh_tree_text_search(cx);
    cx.notify();
  }

  fn confirm_switch_local_branch_with_stash(
    &mut self,
    post_action: Option<GithubPrLocalProjectPostAction>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };
    let branch_name = pull_request.head_ref_name.clone();
    let title: SharedString = "Stash changes before switching branches?".into();
    let message: SharedString = format!(
      "Create a stash with tracked and untracked files, then switch to {}?",
      branch_name
    )
    .into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let post_action = post_action.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Stash and switch")
        .cancel_text("Cancel")
        .on_confirm(move |_, window, cx| {
          let post_action = post_action.clone();
          view.update(cx, |this, cx| {
            this.switch_local_branch_to_pr_branch(true, post_action, window, cx);
          });
          true
        })
        .build(alert)
    });
  }

  fn confirm_prepare_local_branch_for_git_page_with_stash(
    &mut self,
    repo_root: PathBuf,
    post_action: GithubPrLocalProjectPostAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let title: SharedString = "Stash changes before opening Git page?".into();
    let message: SharedString =
      "Create a stash with tracked and untracked files, then prepare this PR branch in the Git page?"
        .into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let repo_root = repo_root.clone();
      let post_action = post_action.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Stash and open Git page")
        .cancel_text("Cancel")
        .on_confirm(move |_, window, cx| {
          let repo_root = repo_root.clone();
          let post_action = post_action.clone();
          view.update(cx, |this, cx| {
            let _ = window;
            this.start_sync_local_branch_to_pr_head(repo_root, true, Some(post_action), cx);
          });
          true
        })
        .build(alert)
    });
  }

  fn execute_local_project_post_action(
    &mut self,
    post_action: GithubPrLocalProjectPostAction,
    repo_root: PathBuf,
    cx: &mut Context<Self>,
  ) {
    match post_action {
      GithubPrLocalProjectPostAction::EnsurePrHeadThenOpenGitPageMergeBase { base_branch_name } => {
        let current_head = current_head_sha(&repo_root).ok().flatten();
        let pr_head = self.pull_request.as_ref().map(|pr| pr.head_sha.as_str());

        if current_head.as_deref() == pr_head {
          GitPageHandle::show_repository_and_merge_base(repo_root, base_branch_name, cx);
        } else {
          self.start_sync_local_branch_to_pr_head(
            repo_root,
            false,
            Some(GithubPrLocalProjectPostAction::OpenGitPageMergeBase { base_branch_name }),
            cx,
          );
        }
      }
      GithubPrLocalProjectPostAction::OpenGitPageMergeBase { base_branch_name } => {
        GitPageHandle::show_repository_and_merge_base(repo_root, base_branch_name, cx);
      }
    }
  }

  fn prompt_or_switch_local_branch_to_pr_branch(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.local_branch_switch_loading {
      return;
    }

    let GithubPrLocalProjectAvailability::NeedsBranchSwitch {
      has_uncommitted_changes,
      ..
    } = self.local_project_availability(cx)
    else {
      return;
    };

    if has_uncommitted_changes {
      self.confirm_switch_local_branch_with_stash(None, window, cx);
    } else {
      self.switch_local_branch_to_pr_branch(false, None, window, cx);
    }
  }

  fn start_switch_local_branch_to_pr_branch(
    &mut self,
    repo_root: PathBuf,
    stash_before_switch: bool,
    post_action: Option<GithubPrLocalProjectPostAction>,
    cx: &mut Context<Self>,
  ) {
    if self.local_branch_switch_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let branch_name = pull_request.head_ref_name.clone();
    self.local_branch_switch_loading = true;
    self.local_branch_switch_error = None;
    self.local_project_update_error = None;
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_action = repo_root.clone();
      let branch_name_for_action = branch_name.clone();
      let result = unblock(move || {
        if stash_before_switch {
          let stash_message = default_stash_message(&repo_root_for_action).ok();
          create_stash(&repo_root_for_action, true, stash_message.as_deref())?;
        }
        switch_to_branch_name(&repo_root_for_action, &branch_name_for_action)?;
        Ok::<_, anyhow::Error>(local_repo_snapshot(
          &repo_root_for_action,
          Some(branch_name_for_action.as_str()),
        ))
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.local_branch_switch_task = None;
        this.local_branch_switch_loading = false;

        match result {
          Ok(snapshot) => {
            this.local_branch_switch_error = None;
            if let Some(snapshot) = snapshot {
              this.sync_active_local_repo_store_snapshot(&snapshot, cx);
              this.sync_resolved_local_repo_snapshot(&snapshot);
            }
            this.load_local_project_files(repo_root.clone(), cx);
            if let Some(post_action) = post_action.clone() {
              this.execute_local_project_post_action(post_action, repo_root.clone(), cx);
            }
          }
          Err(error) => {
            this.local_branch_switch_error = Some(error.to_string().into());
          }
        }

        cx.notify();
      });
    });

    self.local_branch_switch_task = Some(task);
  }

  fn switch_local_branch_to_pr_branch(
    &mut self,
    stash_before_switch: bool,
    post_action: Option<GithubPrLocalProjectPostAction>,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let GithubPrLocalProjectAvailability::NeedsBranchSwitch { repo_root, .. } =
      self.local_project_availability(cx)
    else {
      return;
    };

    self.start_switch_local_branch_to_pr_branch(repo_root, stash_before_switch, post_action, cx);
  }

  fn start_sync_local_branch_to_pr_head(
    &mut self,
    repo_root: PathBuf,
    stash_before_update: bool,
    post_action: Option<GithubPrLocalProjectPostAction>,
    cx: &mut Context<Self>,
  ) {
    if self.local_project_update_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let branch_name = pull_request.head_ref_name.clone();
    let target_head_sha = pull_request.head_sha.clone();
    self.local_project_update_loading = true;
    self.local_project_update_error = None;
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_update = repo_root.clone();
      let branch_name_for_update = branch_name.clone();
      let target_head_sha_for_update = target_head_sha.clone();
      let result = unblock(move || {
        if stash_before_update {
          let stash_message = default_stash_message(&repo_root_for_update).ok();
          create_stash(&repo_root_for_update, true, stash_message.as_deref())?;
        }
        sync_current_branch_to_head(
          &repo_root_for_update,
          &branch_name_for_update,
          &target_head_sha_for_update,
        )?;
        Ok::<_, anyhow::Error>(local_repo_snapshot(
          &repo_root_for_update,
          Some(branch_name_for_update.as_str()),
        ))
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.local_project_update_task = None;
        this.local_project_update_loading = false;

        match result {
          Ok(snapshot) => {
            this.local_project_update_error = None;
            if let Some(snapshot) = snapshot {
              this.sync_active_local_repo_store_snapshot(&snapshot, cx);
              this.sync_resolved_local_repo_snapshot(&snapshot);
            }
            this.load_local_project_files(repo_root.clone(), cx);
            if let Some(post_action) = post_action.clone() {
              this.execute_local_project_post_action(post_action, repo_root.clone(), cx);
            }
          }
          Err(error) => {
            this.local_project_update_error = Some(error.to_string().into());
          }
        }

        cx.notify();
      });
    });

    self.local_project_update_task = Some(task);
  }

  fn update_local_branch_to_pr_head(
    &mut self,
    stash_before_update: bool,
    post_action: Option<GithubPrLocalProjectPostAction>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let repo_root = match self.local_project_availability(cx) {
      GithubPrLocalProjectAvailability::NeedsUpdate { repo_root }
      | GithubPrLocalProjectAvailability::Dirty { repo_root }
      | GithubPrLocalProjectAvailability::Ready { repo_root } => repo_root,
      _ => return,
    };

    let _ = window;
    self.start_sync_local_branch_to_pr_head(repo_root, stash_before_update, post_action, cx);
  }

  fn local_project_command_palette_commands(
    availability: &GithubPrLocalProjectAvailability,
  ) -> Vec<CommandPaletteCommand> {
    if matches!(
      availability,
      GithubPrLocalProjectAvailability::NeedsBranchSwitch { .. }
    ) {
      vec![CommandPaletteCommand::switch_to_pr_branch()]
    } else {
      Vec::new()
    }
  }

  fn command_palette_commands(&self, cx: &App) -> Vec<CommandPaletteCommand> {
    let include_github = AuthStateStore::has_github_access(cx);
    let availability = self.local_project_availability(cx);
    let mut commands = Self::local_project_command_palette_commands(&availability);
    if matches!(availability, GithubPrLocalProjectAvailability::Ready { .. }) {
      commands.push(CommandPaletteCommand::toggle_unchanged_files(
        self.show_local_project_files,
      ));
    }
    commands.extend(CommandPaletteCommand::default_global_commands(
      CommandPalettePage::GithubPrDetails,
      include_github,
    ));
    commands
  }

  fn mark_merge_form_reset_pending(&mut self) {
    self.merge_form_reset_pending = true;
    self.merge_submit_error = None;
  }

  fn sync_merge_method_with_readiness(&mut self) {
    let Some(readiness) = self.merge_readiness.as_ref() else {
      return;
    };

    let method_available = readiness
      .available_methods
      .iter()
      .any(|method| *method == self.merge_method);

    if !method_available {
      self.merge_method = readiness
        .default_method
        .or_else(|| readiness.available_methods.first().copied())
        .unwrap_or(GithubPullRequestMergeMethod::Merge);
    }
  }

  fn reset_merge_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.merge_form_reset_pending = false;
    self.sync_merge_method_with_readiness();
    self.merge_submit_error = None;
    self.merge_commit_title_input.update(cx, |input, cx| {
      input.set_value("", window, cx);
    });
    self.merge_commit_message_input.update(cx, |input, cx| {
      input.set_value("", window, cx);
    });
  }

  fn selected_merge_method(&self) -> Option<GithubPullRequestMergeMethod> {
    self.merge_readiness.as_ref().and_then(|readiness| {
      readiness
        .available_methods
        .iter()
        .any(|method| *method == self.merge_method)
        .then_some(self.merge_method)
    })
  }

  fn overview_pr_alert_action_label(
    &self,
    content: &OverviewPrAlertContent,
    cx: &App,
  ) -> Option<&'static str> {
    if matches!(
      self.local_project_availability(cx),
      GithubPrLocalProjectAvailability::Hidden
    ) {
      return None;
    }

    match content.kind {
      OverviewPrAlertKind::Conflicts => Some("Resolve in Git page"),
      OverviewPrAlertKind::OutOfDate => Some("Update in Git page"),
      OverviewPrAlertKind::Blocked => None,
    }
  }

  fn open_overview_pr_alert_in_git_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(content) =
      overview_pr_alert_content(self.merge_readiness.as_ref(), self.checks.as_ref())
    else {
      return;
    };
    if matches!(content.kind, OverviewPrAlertKind::Blocked) {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let post_action = GithubPrLocalProjectPostAction::OpenGitPageMergeBase {
      base_branch_name: pull_request.base_ref_name.clone(),
    };
    let has_uncommitted_changes = self.effective_local_repo_has_uncommitted_changes(cx);

    match self.local_project_availability(cx) {
      GithubPrLocalProjectAvailability::Hidden => {}
      GithubPrLocalProjectAvailability::NeedsBranchSwitch {
        has_uncommitted_changes,
        ..
      } => {
        let post_action = Some(
          GithubPrLocalProjectPostAction::EnsurePrHeadThenOpenGitPageMergeBase {
            base_branch_name: pull_request.base_ref_name.clone(),
          },
        );
        if has_uncommitted_changes {
          self.confirm_switch_local_branch_with_stash(post_action, window, cx);
        } else {
          self.switch_local_branch_to_pr_branch(false, post_action, window, cx);
        }
      }
      GithubPrLocalProjectAvailability::Ready { repo_root } => {
        if should_prepare_local_branch_before_opening_git_page(
          repo_root.as_path(),
          has_uncommitted_changes,
        ) {
          self.confirm_prepare_local_branch_for_git_page_with_stash(
            repo_root,
            post_action,
            window,
            cx,
          );
        } else {
          self.execute_local_project_post_action(post_action, repo_root, cx);
        }
      }
      GithubPrLocalProjectAvailability::NeedsUpdate { .. } => {
        self.update_local_branch_to_pr_head(false, Some(post_action), window, cx);
      }
      GithubPrLocalProjectAvailability::Dirty { repo_root } => {
        if should_prepare_local_branch_before_opening_git_page(repo_root.as_path(), true) {
          self.confirm_prepare_local_branch_for_git_page_with_stash(
            repo_root,
            post_action,
            window,
            cx,
          );
        } else {
          self.execute_local_project_post_action(post_action, repo_root, cx);
        }
      }
    }
  }

  fn render_overview_pr_alert(&self, cx: &Context<Self>) -> Option<AnyElement> {
    let content = overview_pr_alert_content(self.merge_readiness.as_ref(), self.checks.as_ref())?;
    let action_label = self.overview_pr_alert_action_label(&content, cx);
    let theme = cx.theme();
    let view = cx.entity();

    Some(
      div()
        .flex()
        .id(content.id)
        .debug_selector(|| content.id.to_string())
        .w_full()
        .items_start()
        .gap_3()
        .px_4()
        .py_3()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.warning.opacity(0.3))
        .bg(theme.warning.opacity(0.08))
        .text_color(theme.warning)
        .child(
          h_flex()
            .flex_1()
            .items_start()
            .gap_3()
            .child(Icon::new(IconName::TriangleAlert).mt(px(3.0)))
            .child(
              v_flex()
                .flex_1()
                .text_sm()
                .gap_1()
                .child(
                  h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(div().font_semibold().child(content.title))
                    .when_some(action_label, |this, label| {
                      this.child(
                        Button::new("github-pr-overview-alert-action")
                          .small()
                          .primary()
                          .with_variant(ButtonVariant::Secondary)
                          .label(label)
                          .disabled(
                            self.local_branch_switch_loading || self.local_project_update_loading,
                          )
                          .on_click(move |_, window, cx| {
                            view.update(cx, |this, cx| {
                              this.open_overview_pr_alert_in_git_page(window, cx);
                            });
                          }),
                      )
                    }),
                )
                .child(div().child(content.message)),
            ),
        )
        .into_any_element(),
    )
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

    if should_refresh_pr_overview_data(self.active_tab_ix) {
      self.refresh_issue_comments_for_current_pull_request(cx);
      self.refresh_reviews_for_current_pull_request(true, cx);
      self.refresh_review_comments_for_current_pull_request(cx);
    }

    if should_refresh_pr_changes_data(self.active_tab_ix) {
      self.saved_pr_selected_tree_id = self.current_selected_tree_path();
      self.refresh_commits_for_current_pull_request(cx);
      self.refresh_review_comments_for_current_pull_request(cx);
      self.reload_files_for_current_pull_request(cx);
    }

    if should_refresh_pr_checks_data(self.active_tab_ix) {
      self.refresh_checks_for_current_pull_request(cx);
    }
  }

  fn is_pull_request_merged(&self) -> bool {
    self
      .pull_request
      .as_ref()
      .is_some_and(|pull_request| pull_request.merged_at.is_some())
  }

  fn reload_merge_readiness_for_current_pull_request(&mut self, cx: &mut Context<Self>) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };
    self.fetch_merge_readiness_for_context(context.owner, context.repo, context.number, cx);
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
      let result = unblock(move || {
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

  fn refresh_reviews_for_current_pull_request(
    &mut self,
    set_loading: bool,
    cx: &mut Context<Self>,
  ) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };

    if set_loading {
      self.reviews_loading = true;
      self.reviews_error = None;
    }

    let api = self.api.clone();
    let owner = context.owner;
    let repo = context.repo;
    let number = context.number;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_pull_request_reviews(&owner, &repo, number)).await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(reviews) => {
            this.reviews = reviews;
            this.reviews_loading = false;
            this.reviews_error = None;
            this.add_pr_breadcrumb("Refresh PR reviews succeeded", Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            this.reviews_loading = false;
            this.reviews_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Refresh PR reviews failed", Map::new());
            this.record_pr_error(
              "github.pr.reviews.refresh",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        cx.notify();
      });
    });

    self.reviews_task = Some(task);
  }

  fn refresh_issue_comments_for_current_pull_request(&mut self, cx: &mut Context<Self>) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };

    self.issue_comments_loading = true;
    self.issue_comments_error = None;

    let api = self.api.clone();
    let owner = context.owner;
    let repo = context.repo;
    let number = context.number;
    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.fetch_pull_request_issue_comments(&owner, &repo, number)).await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(comments) => {
            this.issue_comments = comments;
            this.issue_comments_loading = false;
            this.issue_comments_error = None;
            this.add_pr_breadcrumb("Refresh PR issue comments succeeded", Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            this.issue_comments_loading = false;
            this.issue_comments_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Refresh PR issue comments failed", Map::new());
            this.record_pr_error(
              "github.pr.issue_comments.refresh",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        this.issue_comments_task = None;
        cx.notify();
      });
    });

    self.issue_comments_task = Some(task);
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

  fn refresh_review_comments_for_current_pull_request(&mut self, cx: &mut Context<Self>) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };

    self.review_comments_loading = true;
    self.review_comments_error = None;

    let api = self.api.clone();
    let owner = context.owner;
    let repo = context.repo;
    let number = context.number;
    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.fetch_pull_request_review_comments(&owner, &repo, number)).await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(comments) => {
            this.review_comments = comments;
            this.review_comments_loading = false;
            this.review_comments_error = None;
            this.add_pr_breadcrumb("Refresh PR comments succeeded", Map::new());
            this.sync_review_comments(cx);
            this.prefetch_overview_root_review_comment_files(cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            this.review_comments_loading = false;
            this.review_comments_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Refresh PR comments failed", Map::new());
            this.record_pr_error(
              "github.pr.comments.refresh",
              error_message.as_str(),
              Map::new(),
            );
            this.sync_review_comments(cx);
          }
        }
        this.review_comments_task = None;
        cx.notify();
      });
    });

    self.review_comments_task = Some(task);
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
      let result = unblock(move || api.fetch_pull_request_commits(&owner, &repo, number)).await;

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
            this.sync_commits_list(cx);
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
            this.sync_commits_list(cx);
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
      let result = unblock(move || api.fetch_pull_request_checks(&owner, &repo, number)).await;

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

  fn fetch_merge_readiness_for_context(
    &mut self,
    owner: String,
    repo: String,
    number: u64,
    cx: &mut Context<Self>,
  ) {
    self.merge_readiness_loading = true;
    self.merge_readiness_error = None;
    self.merge_readiness = None;
    let merge_api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || merge_api.fetch_pull_request_merge_readiness(&owner, &repo, number)).await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(readiness) => {
            this.merge_readiness_loading = false;
            this.merge_readiness_error = None;
            this.merge_readiness = Some(readiness);
            this.sync_merge_method_with_readiness();
            this.add_pr_breadcrumb("Load PR merge readiness succeeded", Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            this.merge_readiness_loading = false;
            this.merge_readiness_error = Some(error_message.clone().into());
            this.merge_readiness = None;
            this.add_pr_breadcrumb("Load PR merge readiness failed", Map::new());
            this.record_pr_error(
              "github.pr.merge_readiness",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        this.merge_readiness_task = None;
        cx.notify();
      });
    });
    self.merge_readiness_task = Some(task);
  }

  fn submit_pull_request_merge(&mut self, _: &mut Window, cx: &mut Context<Self>) {
    if self.merge_submit_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      self.merge_submit_error = Some("No pull request selected".into());
      return;
    };

    let Some(readiness) = self.merge_readiness.as_ref() else {
      self.merge_submit_error = Some("Merge readiness is not available yet.".into());
      return;
    };

    let Some(method) = self.selected_merge_method() else {
      self.merge_submit_error = Some("No merge method is available.".into());
      return;
    };

    if !readiness.can_merge_now {
      self.merge_submit_error = Some(readiness.message.clone().into());
      return;
    }

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let expected_head_sha = readiness.current_head_sha.clone();
    let commit_title = self.merge_commit_title_input.read(cx).value().to_string();
    let commit_message = self.merge_commit_message_input.read(cx).value().to_string();
    let api = self.api.clone();
    self.merge_submit_loading = true;
    self.merge_submit_error = None;

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        api.merge_pull_request(
          &owner,
          &repo,
          number,
          method,
          &expected_head_sha,
          Some(commit_title.as_str()),
          Some(commit_message.as_str()),
        )
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.merge_submit_loading = false;
        match result {
          Ok(GithubPullRequestMergeResult { merged: true, .. }) => {
            this.merge_popover_open = false;
            this.mark_merge_form_reset_pending();
            this.add_pr_breadcrumb("Merge pull request succeeded", Map::new());
            this.reload_current_pull_request(cx);
            if AuthStateStore::has_github_access(cx) {
              GithubPageHandle::refresh(cx);
            }
            cx.refresh_windows();
          }
          Ok(result) => {
            this.merge_submit_error = Some(result.message.into());
          }
          Err(error) => {
            let should_reload_merge_readiness = error
              .downcast_ref::<ApiError>()
              .and_then(ApiError::status_code_u16)
              .is_some_and(|status| status == 405 || status == 409);
            let error_message = error.to_string();
            this.merge_submit_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Merge pull request failed", Map::new());
            this.record_pr_error("github.pr.merge", error_message.as_str(), Map::new());
            if should_reload_merge_readiness {
              this.reload_merge_readiness_for_current_pull_request(cx);
            }
          }
        }
        cx.notify();
      });
    });

    self.merge_submit_task = Some(task);
  }

  fn is_current_user_pr_author(&self, cx: &App) -> bool {
    let Some(pull_request) = self.pull_request.as_ref() else {
      return false;
    };
    let Some(login) = Self::current_github_login(cx) else {
      return false;
    };

    pull_request
      .author
      .login
      .eq_ignore_ascii_case(login.as_str())
  }

  fn pull_request_status_action(&self) -> Option<GithubPrStatusAction> {
    let pull_request = self.pull_request.as_ref()?;
    if !matches!(pull_request.state, GithubPullRequestState::Open) {
      return None;
    }

    Some(if pull_request.draft {
      GithubPrStatusAction::ReadyForReview
    } else {
      GithubPrStatusAction::ConvertToDraft
    })
  }

  fn push_pr_status_action_error_notification(
    &self,
    title: impl Into<SharedString>,
    error: SharedString,
    cx: &mut Context<Self>,
  ) {
    let title = title.into();
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      window.push_notification(
        Notification::error(error)
          .id::<GithubPrStatusActionNotificationId>()
          .title(title),
        cx,
      );
    });
  }

  fn local_draft_merge_readiness(&self) -> Option<GithubPullRequestMergeReadiness> {
    let pull_request = self.pull_request.as_ref()?;
    let existing = self.merge_readiness.as_ref();

    Some(GithubPullRequestMergeReadiness {
      status: GithubPullRequestMergeReadinessStatus::Draft,
      message: "This pull request is still marked as a draft.".to_string(),
      current_head_sha: pull_request.head_sha.clone(),
      available_methods: Vec::new(),
      default_method: None,
      can_merge_now: false,
      viewer_can_merge: existing
        .map(|readiness| readiness.viewer_can_merge)
        .unwrap_or(true),
      mergeable_state: Some("draft".to_string()),
      rebaseable: existing.and_then(|readiness| readiness.rebaseable),
      auto_merge_enabled: existing
        .map(|readiness| readiness.auto_merge_enabled)
        .unwrap_or(false),
    })
  }

  fn apply_pull_request_status_action_success(
    &mut self,
    action: GithubPrStatusAction,
    cx: &mut Context<Self>,
  ) {
    let Some(pull_request) = self.pull_request.as_mut() else {
      return;
    };

    pull_request.draft = matches!(action, GithubPrStatusAction::ConvertToDraft);

    match action {
      GithubPrStatusAction::ReadyForReview => {
        self.merge_popover_open = false;
        self.mark_merge_form_reset_pending();
        self.reload_merge_readiness_for_current_pull_request(cx);
      }
      GithubPrStatusAction::ConvertToDraft => {
        self.merge_popover_open = false;
        self.mark_merge_form_reset_pending();
        self.merge_readiness_loading = false;
        self.merge_readiness_error = None;
        self.merge_readiness_task = None;
        self.merge_readiness = self.local_draft_merge_readiness();
        self.sync_merge_method_with_readiness();
      }
    }
  }

  fn submit_pull_request_status_action(
    &mut self,
    action: GithubPrStatusAction,
    cx: &mut Context<Self>,
  ) {
    if self.status_action_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let pull_request_id = pull_request.node_id.clone();
    let api = self.api.clone();
    self.status_action_loading = true;

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || match action {
        GithubPrStatusAction::ReadyForReview => {
          api.mark_pull_request_ready_for_review(&owner, &repo, number, &pull_request_id)
        }
        GithubPrStatusAction::ConvertToDraft => {
          api.convert_pull_request_to_draft(&owner, &repo, number, &pull_request_id)
        }
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.status_action_loading = false;

        match result {
          Ok(()) => {
            this.add_pr_breadcrumb(action.success_breadcrumb(), Map::new());
            this.apply_pull_request_status_action_success(action, cx);
            cx.refresh_windows();
          }
          Err(error) => {
            let error_message = error.to_string();
            this.add_pr_breadcrumb(action.failure_breadcrumb(), Map::new());
            this.record_pr_error(
              action.sentry_operation(),
              error_message.as_str(),
              Map::new(),
            );
            this.push_pr_status_action_error_notification(
              action.error_title(),
              error_message.into(),
              cx,
            );
          }
        }

        cx.notify();
      });
    });

    self.status_action_task = Some(task);
    cx.notify();
  }

  fn editable_review_comment_ids(&self, cx: &App) -> HashSet<u64> {
    let Some(login) = Self::current_github_login(cx) else {
      return HashSet::new();
    };

    self
      .review_comments
      .iter()
      .filter(|comment| review_comment_owned_by_login(comment, &login))
      .map(|comment| comment.id)
      .collect()
  }

  fn editable_issue_comment_ids(&self, cx: &App) -> HashSet<u64> {
    let Some(login) = Self::current_github_login(cx) else {
      return HashSet::new();
    };

    self
      .issue_comments
      .iter()
      .filter_map(|comment| issue_comment_owned_by_login(comment, &login).then_some(comment.id))
      .collect()
  }

  fn ensure_pr_description_edit_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<InputState> {
    if let Some(input) = self.pr_description_edit_input.as_ref() {
      return input.clone();
    }

    let input = cx.new(|cx| {
      InputState::new(window, cx)
        .multi_line(true)
        .rows(6)
        .placeholder("Edit description...")
    });
    self.pr_description_edit_input = Some(input.clone());
    input
  }

  fn clear_pr_description_edit_state(&mut self) {
    self.pr_description_editing = false;
    self.pr_description_initial_body = None;
    self.pr_description_submitting = false;
    self.pr_description_error = None;
  }

  fn start_pr_description_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.pr_description_submitting || self.overview_comment_submission_in_flight() {
      return;
    }
    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let initial_body = pull_request.body.clone().unwrap_or_default();
    let input = self.ensure_pr_description_edit_input(window, cx);
    let initial_body_for_input = initial_body.clone();
    input.update(cx, |state, cx| {
      state.set_value(initial_body_for_input.clone(), window, cx);
    });

    let input_for_focus = input.clone();
    window.on_next_frame(move |window, cx| {
      input_for_focus.update(cx, |state, cx| {
        state.focus(window, cx);
      });
    });

    self.pr_description_editing = true;
    self.pr_description_initial_body = Some(initial_body);
    self.pr_description_error = None;
    cx.notify();
  }

  fn cancel_pr_description_edit(&mut self, cx: &mut Context<Self>) {
    if self.pr_description_submitting || !self.pr_description_editing {
      return;
    }
    self.clear_pr_description_edit_state();
    cx.notify();
  }

  fn submit_pr_description_edit(&mut self, cx: &mut Context<Self>) {
    if self.pr_description_submitting
      || !self.pr_description_editing
      || self.overview_comment_submission_in_flight()
    {
      return;
    }
    let Some((owner, repo, number)) = self.pull_request.as_ref().map(|pull_request| {
      (
        pull_request.repository.owner.clone(),
        pull_request.repository.repo.clone(),
        pull_request.number,
      )
    }) else {
      self.pr_description_error = Some("No pull request selected".into());
      cx.notify();
      return;
    };
    let Some(input) = self.pr_description_edit_input.as_ref() else {
      return;
    };
    let initial_body = self
      .pr_description_initial_body
      .as_deref()
      .unwrap_or_default()
      .to_string();
    let raw_value = input.read(cx).value().to_string();
    let Some(next_body) = next_pr_description_body(raw_value.as_str(), initial_body.as_str())
    else {
      self.clear_pr_description_edit_state();
      cx.notify();
      return;
    };

    self.pr_description_submitting = true;
    self.pr_description_error = None;
    cx.notify();

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        api.update_pull_request_description(&owner, &repo, number, next_body.as_str())
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.pr_description_submitting = false;
        match result {
          Ok(update) => {
            if let Some(pull_request) = this.pull_request.as_mut() {
              apply_pull_request_description_update_local(pull_request, update);
            }
            if let Some(pull_request) = this.pull_request.as_ref() {
              this.description_code_reference_requests =
                Self::description_code_reference_requests_for_pull_request(pull_request);
              let description_requests = this.description_code_reference_requests.clone();
              this.schedule_code_reference_fetches(description_requests.iter(), cx);
            }
            this.clear_pr_description_edit_state();
          }
          Err(error) => {
            let error_message = error.to_string();
            this.pr_description_error = Some(error_message.clone().into());
            this.record_pr_error(
              "github.pr.description.update",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        cx.notify();
      });
    });
    self.details_task = Some(task);
  }

  fn overview_comment_submission_in_flight(&self) -> bool {
    self.overview_issue_comment_submitting
      || self.overview_edit_submitting
      || self.overview_reply_submitting
      || self.pr_description_submitting
  }

  fn overview_comment_body_for_target(&self, target: OverviewCommentTarget) -> Option<String> {
    match target.kind {
      OverviewCommentKind::Issue => self
        .issue_comments
        .iter()
        .find(|comment| comment.id == target.id)
        .map(|comment| comment.body.clone()),
      OverviewCommentKind::Review => self
        .review_comments
        .iter()
        .find(|comment| comment.id == target.id)
        .map(|comment| comment.body.clone()),
    }
  }

  fn ensure_overview_edit_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<InputState> {
    if let Some(input) = self.overview_edit_input.as_ref() {
      return input.clone();
    }

    let input = cx.new(|cx| {
      InputState::new(window, cx)
        .multi_line(true)
        .rows(6)
        .placeholder("Edit comment...")
    });
    self.overview_edit_input = Some(input.clone());
    input
  }

  fn ensure_overview_reply_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<InputState> {
    if let Some(input) = self.overview_reply_input.as_ref() {
      return input.clone();
    }

    let input = cx.new(|cx| {
      InputState::new(window, cx)
        .multi_line(true)
        .rows(6)
        .placeholder("Reply to review comment...")
    });
    self.overview_reply_input = Some(input.clone());
    input
  }

  fn clear_overview_edit_state(&mut self) {
    self.overview_edit_target = None;
    self.overview_edit_initial_body = None;
    self.overview_edit_error = None;
    self.overview_edit_submitting = false;
  }

  fn clear_overview_reply_state(&mut self) {
    self.overview_reply_target_comment_id = None;
    self.overview_reply_error = None;
    self.overview_reply_submitting = false;
  }

  fn start_overview_comment_edit(
    &mut self,
    target: OverviewCommentTarget,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.overview_comment_submission_in_flight() {
      return;
    }

    let Some(initial_body) = self.overview_comment_body_for_target(target) else {
      return;
    };

    self.clear_overview_reply_state();
    let input = self.ensure_overview_edit_input(window, cx);
    let initial_body_for_input = initial_body.clone();
    input.update(cx, |state, cx| {
      state.set_value(initial_body_for_input.clone(), window, cx);
    });

    let input_for_focus = input.clone();
    window.on_next_frame(move |window, cx| {
      input_for_focus.update(cx, |state, cx| {
        state.focus(window, cx);
      });
    });

    self.overview_edit_target = Some(target);
    self.overview_edit_initial_body = Some(initial_body);
    self.overview_edit_error = None;
    cx.notify();
  }

  fn cancel_overview_comment_edit(&mut self, cx: &mut Context<Self>) {
    if self.overview_edit_submitting || self.overview_edit_target.is_none() {
      return;
    }
    self.clear_overview_edit_state();
    cx.notify();
  }

  fn start_overview_review_comment_reply(
    &mut self,
    comment_id: u64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.overview_comment_submission_in_flight() {
      return;
    }
    if !self
      .review_comments
      .iter()
      .any(|comment| comment.id == comment_id)
    {
      return;
    }

    let input = self.ensure_overview_reply_input(window, cx);
    input.update(cx, |state, cx| {
      state.set_value("", window, cx);
    });

    let input_for_focus = input.clone();
    window.on_next_frame(move |window, cx| {
      input_for_focus.update(cx, |state, cx| {
        state.focus(window, cx);
      });
    });

    self.overview_edit_target = None;
    self.overview_edit_initial_body = None;
    self.overview_edit_error = None;
    self.overview_reply_target_comment_id = Some(comment_id);
    self.overview_reply_error = None;
    cx.notify();
  }

  fn cancel_overview_review_comment_reply(&mut self, cx: &mut Context<Self>) {
    if self.overview_reply_submitting || self.overview_reply_target_comment_id.is_none() {
      return;
    }
    self.clear_overview_reply_state();
    cx.notify();
  }

  fn sync_commits_list(&mut self, cx: &mut Context<Self>) {
    let commits = self.commits.clone();
    let selected_commit_sha = self.selected_commit_sha.clone();
    self.commits_list.update(cx, |state, cx| {
      state
        .delegate_mut()
        .set_rows(&commits, selected_commit_sha.as_deref());
      cx.notify();
    });
  }

  fn subscribe_to_commits_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    cx.subscribe_in(
      &self.commits_list,
      window,
      |this, state, event: &ListEvent, window, cx| {
        if let ListEvent::Confirm(ix) = event {
          let commit = state.read(cx).delegate().row_at(*ix);
          if let Some(commit) = commit {
            this.select_commit_filter(Some(commit.sha.clone()), cx);
            this.set_active_tab(PR_TAB_CHANGES_IX, window, cx);
          }
        }
      },
    )
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

  fn set_tree_search_query(&mut self, query: String, cx: &mut Context<Self>) {
    if self.tree_search_query == query {
      return;
    }

    self.tree_search_query = query;
    self.refresh_tree_text_search(cx);
  }

  fn refresh_tree_text_search(&mut self, cx: &mut Context<Self>) {
    self.tree_search_generation = self.tree_search_generation.wrapping_add(1);
    let generation = self.tree_search_generation;
    self.tree_search_task = None;
    self.tree_search_error = None;

    let Some(query) = self.tree_search_query_normalized() else {
      self.tree_search_loading = false;
      self.tree_search_matches = None;
      self.sync_changes_tree_state(cx);
      cx.notify();
      return;
    };

    let scope_paths = self.search_scope_paths(cx);
    if scope_paths.is_empty() {
      self.tree_search_loading = false;
      self.tree_search_matches = Some(HashSet::new());
      self.sync_changes_tree_state(cx);
      cx.notify();
      return;
    }

    let pr_files = scope_paths
      .iter()
      .filter_map(|path| {
        self
          .file_lookup
          .get(path)
          .map(|file| (path.clone(), file.as_ref().clone()))
      })
      .collect::<HashMap<_, _>>();
    let cached_file_contents = self.file_contents.clone();
    let diff_refs = self.resolve_diff_refs();
    let api = self.api.clone();
    let local_repo_root = self
      .local_project_mode_active(cx)
      .then(|| self.local_project_loaded_repo_root.clone())
      .flatten();
    let previous_matches = self.tree_search_matches.clone();

    self.tree_search_loading = true;
    if let Some(previous_matches) = previous_matches {
      self.tree_search_matches = Some(previous_matches);
    }
    self.sync_changes_tree_state(cx);
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        perform_tree_text_search(
          &query,
          &scope_paths,
          &pr_files,
          &cached_file_contents,
          diff_refs.as_ref(),
          &api,
          local_repo_root.as_deref(),
        )
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        if generation != this.tree_search_generation {
          return;
        }

        this.tree_search_task = None;
        this.tree_search_loading = false;
        this.tree_search_error = result.error.map(Into::into);
        for (path, contents) in result.updated_file_contents {
          this.file_contents.entry(path).or_insert(contents);
        }
        this.tree_search_matches = Some(result.matches);
        this.sync_changes_tree_state(cx);
        cx.notify();
      });
    });

    self.tree_search_task = Some(task);
  }

  fn build_commit_dropdown_items(
    commits: &[GithubPullRequestCommit],
    selected_commit_sha: Option<&str>,
  ) -> Vec<GithubPrCommitSelectItem> {
    let selected_commit_sha = selected_commit_sha
      .map(str::trim)
      .filter(|sha| !sha.is_empty());
    let mut items = vec![GithubPrCommitSelectItem::all_changes(
      selected_commit_sha.is_none(),
    )];
    items.extend(commits.iter().map(|commit| {
      GithubPrCommitSelectItem::for_commit(commit, selected_commit_sha == Some(commit.sha.as_str()))
    }));
    items
  }

  fn commit_select_handler(&self, cx: &Context<Self>) -> CommitSelectHandler {
    let view = cx.entity();
    Rc::new(move |selected_commit_sha, window, cx| {
      view.update(cx, |this, cx| {
        this.select_commit_filter(selected_commit_sha.clone(), cx);
        this.refocus_page_shortcuts(window, cx);
      });
    })
  }

  fn refocus_page_shortcuts(&self, window: &mut Window, cx: &mut Context<Self>) {
    let focus_handle = self.focus_handle.clone();
    window.focus(&focus_handle, cx);
    cx.on_next_frame(window, move |_, window, cx| {
      window.focus(&focus_handle, cx);
    });
  }

  fn select_commit_filter(&mut self, selected_commit_sha: Option<String>, cx: &mut Context<Self>) {
    if self.selected_commit_sha == selected_commit_sha {
      return;
    }
    let should_disable_local_project =
      selected_commit_sha.is_some() && self.show_local_project_files;
    self.selected_commit_sha = selected_commit_sha;
    if should_disable_local_project {
      self.set_show_local_project_files(false, cx);
    }
    self.sync_sentry_pr_context();
    self.sync_commits_list(cx);
    self.reload_files_for_current_pull_request(cx);
  }

  fn review_decision_to_api_event(
    decision: GithubPrReviewDecision,
  ) -> GithubPullRequestReviewEvent {
    match decision {
      GithubPrReviewDecision::Comment => GithubPullRequestReviewEvent::Comment,
      GithubPrReviewDecision::Approve => GithubPullRequestReviewEvent::Approve,
      GithubPrReviewDecision::RequestChanges => GithubPullRequestReviewEvent::RequestChanges,
    }
  }

  fn review_decision_requires_body(decision: GithubPrReviewDecision) -> bool {
    matches!(
      decision,
      GithubPrReviewDecision::Comment | GithubPrReviewDecision::RequestChanges
    )
  }

  fn review_decision_from_index(index: usize) -> GithubPrReviewDecision {
    match index {
      1 => GithubPrReviewDecision::Approve,
      2 => GithubPrReviewDecision::RequestChanges,
      _ => GithubPrReviewDecision::Comment,
    }
  }

  fn review_decision_index(decision: GithubPrReviewDecision) -> usize {
    match decision {
      GithubPrReviewDecision::Comment => 0,
      GithubPrReviewDecision::Approve => 1,
      GithubPrReviewDecision::RequestChanges => 2,
    }
  }

  fn validate_review_submission(
    decision: GithubPrReviewDecision,
    body: &str,
  ) -> Option<SharedString> {
    if Self::review_decision_requires_body(decision) && body.trim().is_empty() {
      return Some("A review comment is required for this review type".into());
    }

    None
  }

  fn mark_review_form_reset_pending(&mut self) {
    self.review_form_reset_pending = true;
    self.review_decision = GithubPrReviewDecision::Comment;
    self.submit_review_error = None;
  }

  fn reset_review_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.review_form_reset_pending = false;
    self.review_decision = GithubPrReviewDecision::Comment;
    self.submit_review_error = None;
    self.review_input.update(cx, |input, cx| {
      input.set_value("", window, cx);
    });
  }

  fn focus_review_input(&self, window: &mut Window) {
    let review_input = self.review_input.clone();
    window.on_next_frame(move |window, cx| {
      review_input.update(cx, |input, cx| {
        input.focus(window, cx);
      });
    });
  }

  fn submit_pull_request_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.submit_review_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      self.submit_review_error = Some("No pull request selected".into());
      cx.notify();
      return;
    };

    let body = self.review_input.read(cx).value().to_string();
    let decision = self.review_decision;
    let author_restricted_decision =
      self.is_current_user_pr_author(cx) && !matches!(decision, GithubPrReviewDecision::Comment);
    if author_restricted_decision {
      self.submit_review_error = Some(
        "Pull request authors cannot approve or request changes on their own pull requests.".into(),
      );
      cx.notify();
      return;
    }
    if let Some(error) = Self::validate_review_submission(decision, body.as_str()) {
      self.submit_review_error = Some(error);
      cx.notify();
      return;
    }

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let event = Self::review_decision_to_api_event(decision);
    let api = self.api.clone();

    self.submit_review_loading = true;
    self.submit_review_error = None;
    cx.notify();

    let task = cx.spawn_in(window, async move |this, cx| {
      let result =
        unblock(move || api.submit_pull_request_review(&owner, &repo, number, event, &body)).await;

      let _ = this.update_in(cx, |this, window, cx| {
        this.submit_review_loading = false;

        match result {
          Ok(review) => {
            this.review_popover_open = false;
            this.reset_review_form(window, cx);
            upsert_review_local(&mut this.reviews, review);
            this.refresh_reviews_for_current_pull_request(false, cx);
            this.add_pr_breadcrumb("Submit PR review succeeded", Map::new());
            if AuthStateStore::has_github_access(cx) {
              GithubPageHandle::refresh(cx);
            }
            cx.refresh_windows();
          }
          Err(error) => {
            let error_message = error.to_string();
            this.submit_review_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Submit PR review failed", Map::new());
            this.record_pr_error(
              "github.pr.review.submit",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        cx.notify();
      });
    });

    self.submit_review_task = Some(task);
  }

  fn install_diff_editor_review_comment_handlers(&mut self, cx: &mut Context<Self>) {
    let view = cx.entity().downgrade();
    self.diff_editor.update(cx, |editor, cx| {
      let handler: ReviewCommentEditHandler = Arc::new({
        let view = view.clone();
        move |comment_id, body, _window, cx| {
          let _ = view.update(cx, |this, cx| {
            this.submit_review_comment_edit(comment_id, body.as_ref().to_string(), cx);
          });
        }
      });
      editor.set_review_comment_edit_handler(Some(handler), cx);

      let create_handler: ReviewCommentCreateHandler = Arc::new({
        let view = view.clone();
        move |request, _window, cx| {
          let _ = view.update(cx, |this, cx| {
            this.submit_review_comment_create(request, cx);
          });
        }
      });
      editor.set_review_comment_create_handler(Some(create_handler), cx);

      let delete_handler: ReviewCommentDeleteHandler = Arc::new({
        let view = view.clone();
        move |comment_id, window, cx| {
          let _ = view.update(cx, |this, cx| {
            this.confirm_review_comment_delete(comment_id, window, cx);
          });
        }
      });
      editor.set_review_comment_delete_handler(Some(delete_handler), cx);

      let link_handler: ReviewCommentLinkHandler = Arc::new({
        let view = view.clone();
        move |url, window, cx| {
          view
            .update(cx, |this, cx| this.handle_gfm_link(url, window, cx))
            .unwrap_or(false)
        }
      });
      editor.set_review_comment_link_handler(Some(link_handler), cx);
    });
  }

  fn sync_review_comment_handlers(&mut self, cx: &mut Context<Self>) {
    let should_enable = self.selected_commit_sha.is_none();
    if self.review_comment_handlers_enabled == should_enable {
      return;
    }

    self.review_comment_handlers_enabled = should_enable;
    if should_enable {
      self.install_diff_editor_review_comment_handlers(cx);
      return;
    }

    self.diff_editor.update(cx, |editor, cx| {
      editor.set_review_comment_edit_handler(None, cx);
      editor.set_review_comment_delete_handler(None, cx);
      editor.set_review_comment_create_handler(None, cx);
      editor.set_review_comment_pr_number(None, cx);
      editor.set_editable_review_comment_ids(std::iter::empty::<u64>(), cx);
      editor.set_review_comments(Vec::new(), cx);
      editor.set_review_comment_code_reference_previews(HashMap::new(), cx);
    });
  }

  fn handle_gfm_link(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
    if should_open_externally(window) {
      return github_shared::try_open_github_asset_url(url, &self.api, cx);
    }

    if github_shared::try_open_github_asset_url(url, &self.api, cx) {
      return true;
    }

    let Some(action) = parse_github_url_action(url) else {
      return false;
    };

    match action {
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
        open_changes_tab: _,
        review_comment_id,
      } => {
        let same_target = self.current_pr_context.as_ref().is_some_and(|context| {
          context.number == number
            && context.owner.eq_ignore_ascii_case(&owner)
            && context.repo.eq_ignore_ascii_case(&repo)
        });

        if same_target {
          match same_pr_gfm_navigation(self.active_tab_ix, review_comment_id) {
            SamePrGfmNavigation::ShowOverview { switch_to_overview } => {
              if switch_to_overview {
                self.set_active_tab(PR_TAB_OVERVIEW_IX, window, cx);
              }
              return true;
            }
            SamePrGfmNavigation::ScrollComment { switch_to_changes } => {
              if switch_to_changes {
                self.set_active_tab(PR_TAB_CHANGES_IX, window, cx);
              }
            }
          }
          return review_comment_id.is_some_and(|comment_id| {
            self.handle_review_comment_link_target(number, comment_id, cx)
          });
        }

        self.back_target = next_back_target_for_pr_palette(&self.back_target);
        self.load_pull_request(
          owner,
          repo,
          number,
          GithubPrOpenTarget {
            open_changes_tab: review_comment_id.is_some(),
            review_comment_id,
          },
          cx,
        );
        true
      }
      CommandPaletteAction::OpenGithubRepoDetails {
        owner,
        repo,
        tab,
        issue_number,
        issue_comment_id,
      } => {
        open_repo_target(owner, repo, tab, issue_number, issue_comment_id, cx);
        true
      }
      _ => false,
    }
  }

  fn handle_review_comment_link_target(
    &mut self,
    pr_number: u64,
    comment_id: u64,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(pull_request) = self.pull_request.as_ref() else {
      return false;
    };
    if pull_request.number != pr_number {
      return false;
    }

    self.pending_review_comment_link_comment_id = Some(comment_id);
    self.resolve_pending_review_comment_link(cx);

    if self.pending_review_comment_link_comment_id == Some(comment_id)
      && !self.review_comments_loading
      && !self.files_loading
      && !self.file_loading
      && !self.file_lookup.is_empty()
    {
      self.pending_review_comment_link_comment_id = None;
      return false;
    }

    true
  }

  fn file_for_review_comment_path(&self, path: &str) -> Option<Rc<GithubPrFileDiff>> {
    file_for_review_comment_path(&self.file_lookup, path)
  }

  fn try_scroll_to_pending_review_comment(&mut self, cx: &mut Context<Self>) -> bool {
    let Some(comment_id) = self.pending_review_comment_link_comment_id else {
      return false;
    };

    let did_scroll = self.diff_editor.update(cx, |editor, cx| {
      editor.scroll_to_review_comment(comment_id, editor.measured_editor_line_height(), cx)
    });

    if did_scroll {
      self.pending_review_comment_link_comment_id = None;
      self.active_review_comment_id = Some(comment_id);
    }

    did_scroll
  }

  fn resolve_pending_review_comment_link(&mut self, cx: &mut Context<Self>) {
    let Some(comment_id) = self.pending_review_comment_link_comment_id else {
      return;
    };

    let Some(comment_path) = self
      .review_comments
      .iter()
      .find(|comment| comment.id == comment_id)
      .map(|comment| comment.path.clone())
    else {
      return;
    };

    let Some(target_file) = self.file_for_review_comment_path(comment_path.as_str()) else {
      return;
    };

    let selected_matches_target = self
      .selected_file
      .as_ref()
      .is_some_and(|file| file.path == target_file.path);

    if !selected_matches_target {
      self.set_selected_file(Some(target_file), cx);
      return;
    }

    let _ = self.try_scroll_to_pending_review_comment(cx);
  }

  fn navigate_review_comment(
    &mut self,
    direction: ReviewCommentNavigationDirection,
    cx: &mut Context<Self>,
  ) {
    let Some(index) = next_review_comment_navigation_index(
      &self.selected_file_review_comment_ids,
      self.active_review_comment_id,
      direction,
    ) else {
      return;
    };
    let Some(comment_id) = self.selected_file_review_comment_ids.get(index).copied() else {
      return;
    };

    let did_scroll = self.diff_editor.update(cx, |editor, cx| {
      editor.scroll_to_review_comment(comment_id, editor.measured_editor_line_height(), cx)
    });
    if !did_scroll {
      self.pending_review_comment_link_comment_id = Some(comment_id);
      self.resolve_pending_review_comment_link(cx);
    }
    self.active_review_comment_id = Some(comment_id);
    cx.notify();
  }

  fn submit_review_comment_edit(&mut self, comment_id: u64, body: String, cx: &mut Context<Self>) {
    if self.selected_commit_sha.is_some() {
      let message = Arc::<str>::from("Review comments are disabled for commit-level diffs");
      self.review_comments_error = Some(message.to_string().into());
      self.diff_editor.update(cx, |editor, cx| {
        editor.finish_review_comment_edit_submission(comment_id, Some(message.clone()), cx);
      });
      cx.notify();
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      self.review_comments_error = Some("No pull request selected".into());
      self.diff_editor.update(cx, |editor, cx| {
        editor.finish_review_comment_edit_submission(
          comment_id,
          Some(Arc::from("No pull request selected")),
          cx,
        );
      });
      cx.notify();
      return;
    };

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        api.update_pull_request_review_comment(&owner, &repo, number, comment_id, &body)
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        let mut error_message: Option<Arc<str>> = None;
        match result {
          Ok(updated_comment) => {
            if let Some(existing) = this
              .review_comments
              .iter_mut()
              .find(|comment| comment.id == updated_comment.id)
            {
              *existing = updated_comment;
            } else {
              this.review_comments.push(updated_comment);
            }
            this.review_comments_error = None;
            this.sync_review_comments(cx);
          }
          Err(error) => {
            let error_message_text = error.to_string();
            this.review_comments_error = Some(error_message_text.clone().into());
            this.add_pr_breadcrumb("Update review comment failed", Map::new());
            this.record_pr_error(
              "github.pr.review_comment.update",
              error_message_text.as_str(),
              Map::new(),
            );
            error_message = Some(Arc::from(error_message_text));
          }
        }
        this.diff_editor.update(cx, |editor, cx| {
          editor.finish_review_comment_edit_submission(comment_id, error_message, cx);
        });
        cx.notify();
      });
    });
    self.review_comments_task = Some(task);
  }

  fn submit_review_comment_create(
    &mut self,
    request: ReviewCommentCreateRequest,
    cx: &mut Context<Self>,
  ) {
    if self.selected_commit_sha.is_some() {
      let message = Arc::<str>::from("Review comments are disabled for commit-level diffs");
      self.review_comments_error = Some(message.to_string().into());
      self.diff_editor.update(cx, |editor, cx| {
        editor.finish_review_comment_create_submission(Some(message.clone()), cx);
      });
      cx.notify();
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      self.review_comments_error = Some("No pull request selected".into());
      self.diff_editor.update(cx, |editor, cx| {
        editor
          .finish_review_comment_create_submission(Some(Arc::from("No pull request selected")), cx);
      });
      cx.notify();
      return;
    };
    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let in_reply_to_id = request.in_reply_to_id;
    let line_comment_payload = if in_reply_to_id.is_none() {
      let Some(selected_file) = self.selected_file.as_ref() else {
        self.review_comments_error = Some("No selected file".into());
        self.diff_editor.update(cx, |editor, cx| {
          editor.finish_review_comment_create_submission(Some(Arc::from("No selected file")), cx);
        });
        cx.notify();
        return;
      };

      let side = match request.side {
        ReviewCommentSide::Left => "LEFT".to_string(),
        ReviewCommentSide::Right => "RIGHT".to_string(),
      };
      let start_side = request.start_side.map(|value| match value {
        ReviewCommentSide::Left => "LEFT".to_string(),
        ReviewCommentSide::Right => "RIGHT".to_string(),
      });
      let line = request.line.saturating_add(1) as u64;
      let start_line = request
        .start_line
        .map(|value| value.saturating_add(1) as u64);

      Some((
        selected_file.path.to_string(),
        pull_request.head_sha.clone(),
        line,
        side,
        start_line,
        start_side,
      ))
    } else {
      None
    };
    let body = request.body.as_ref().to_string();
    let api = self.api.clone();

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        if let Some(in_reply_to_id) = in_reply_to_id {
          api.reply_pull_request_review_comment(&owner, &repo, number, in_reply_to_id, &body)
        } else {
          let (path, commit_id, line, side, start_line, start_side) = line_comment_payload
            .expect("line comment payload should exist when creating a top-level comment");
          api.create_pull_request_review_comment(
            &owner,
            &repo,
            number,
            &path,
            &commit_id,
            line,
            &side,
            start_line,
            start_side.as_deref(),
            &body,
          )
        }
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        let mut error_message: Option<Arc<str>> = None;
        match result {
          Ok(created_comment) => {
            if let Some(existing) = this
              .review_comments
              .iter_mut()
              .find(|comment| comment.id == created_comment.id)
            {
              *existing = created_comment;
            } else {
              this.review_comments.push(created_comment);
            }
            this.review_comments_error = None;
            this.sync_review_comments(cx);
          }
          Err(error) => {
            let error_message_text = error.to_string();
            this.review_comments_error = Some(error_message_text.clone().into());
            this.add_pr_breadcrumb("Create review comment failed", Map::new());
            this.record_pr_error(
              "github.pr.review_comment.create",
              error_message_text.as_str(),
              Map::new(),
            );
            error_message = Some(Arc::from(error_message_text));
          }
        }
        this.diff_editor.update(cx, |editor, cx| {
          editor.finish_review_comment_create_submission(error_message, cx);
        });
        cx.notify();
      });
    });
    self.review_comments_task = Some(task);
  }

  fn confirm_review_comment_delete(
    &mut self,
    comment_id: u64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_commit_sha.is_some() {
      self.review_comments_error =
        Some("Review comments are disabled for commit-level diffs".into());
      cx.notify();
      return;
    }

    if !self.editable_review_comment_ids(cx).contains(&comment_id) {
      return;
    }

    let title: SharedString = "Delete comment?".into();
    let message: SharedString = "This review comment will be permanently deleted.".into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Delete")
        .cancel_text("Cancel")
        .destructive()
        .on_confirm(move |_, _, cx| {
          view.update(cx, |this, cx| {
            this.submit_review_comment_delete(comment_id, cx);
          });
          true
        })
        .build(alert)
    });
  }

  fn submit_review_comment_delete(&mut self, comment_id: u64, cx: &mut Context<Self>) {
    if self.selected_commit_sha.is_some() {
      self.review_comments_error =
        Some("Review comments are disabled for commit-level diffs".into());
      self.diff_editor.update(cx, |editor, cx| {
        editor.finish_review_comment_delete_submission(comment_id, cx);
      });
      cx.notify();
      return;
    }

    let Some((owner, repo, number)) = self.pull_request.as_ref().map(|pull_request| {
      (
        pull_request.repository.owner.clone(),
        pull_request.repository.repo.clone(),
        pull_request.number,
      )
    }) else {
      self.review_comments_error = Some("No pull request selected".into());
      self.diff_editor.update(cx, |editor, cx| {
        editor.finish_review_comment_delete_submission(comment_id, cx);
      });
      cx.notify();
      return;
    };
    let Some((removed_index, removed_comment)) = self
      .review_comments
      .iter()
      .enumerate()
      .find(|(_, comment)| comment.id == comment_id)
      .map(|(index, comment)| (index, comment.clone()))
    else {
      return;
    };

    self.diff_editor.update(cx, |editor, cx| {
      editor.start_review_comment_delete_submission(comment_id, cx);
    });
    self.review_comments.remove(removed_index);
    self.review_comments_error = None;
    self.sync_review_comments(cx);
    cx.notify();

    let api = self.api.clone();

    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.delete_pull_request_review_comment(&owner, &repo, number, comment_id))
          .await;

      let _ = this.update(cx, |this, cx| {
        if let Err(error) = result {
          if !this
            .review_comments
            .iter()
            .any(|comment| comment.id == removed_comment.id)
          {
            let insert_index = removed_index.min(this.review_comments.len());
            this
              .review_comments
              .insert(insert_index, removed_comment.clone());
          }
          let error_message_text = error.to_string();
          this.review_comments_error = Some(error_message_text.clone().into());
          this.add_pr_breadcrumb("Delete review comment failed", Map::new());
          this.record_pr_error(
            "github.pr.review_comment.delete",
            error_message_text.as_str(),
            Map::new(),
          );
          this.sync_review_comments(cx);
        } else {
          this.review_comments_error = None;
        }

        this.diff_editor.update(cx, |editor, cx| {
          editor.finish_review_comment_delete_submission(comment_id, cx);
        });
        cx.notify();
      });
    });
    self.review_comments_task = Some(task);
  }

  fn upsert_issue_comment(&mut self, comment: GithubPullRequestIssueComment) {
    upsert_issue_comment_local(&mut self.issue_comments, comment);
  }

  fn upsert_review_comment(&mut self, comment: GithubPullRequestReviewComment) {
    upsert_review_comment_local(&mut self.review_comments, comment);
  }

  fn submit_overview_issue_comment_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.overview_comment_submission_in_flight() || self.overview_issue_comment_submitting {
      return;
    }
    let Some((owner, repo, issue_number)) = self.pull_request.as_ref().map(|pull_request| {
      (
        pull_request.repository.owner.clone(),
        pull_request.repository.repo.clone(),
        pull_request.number,
      )
    }) else {
      self.overview_issue_comment_error = Some("No pull request selected".into());
      cx.notify();
      return;
    };

    let raw_body = self
      .overview_issue_comment_input
      .read(cx)
      .value()
      .to_string();
    let Some(body) = github_shared::normalize_non_empty_text(raw_body.as_str()) else {
      return;
    };

    self.overview_issue_comment_submitting = true;
    self.overview_issue_comment_error = None;
    cx.notify();

    let api = self.api.clone();
    let task = cx.spawn_in(window, async move |this, cx| {
      let result =
        unblock(move || api.create_issue_comment(&owner, &repo, issue_number, body.as_str())).await;
      let _ = this.update_in(cx, |this, window, cx| {
        this.overview_issue_comment_submitting = false;
        match result {
          Ok(comment) => {
            let mapped = pull_request_issue_comment_from_issue_details_comment(comment);
            this.upsert_issue_comment(mapped);
            this.overview_issue_comment_error = None;
            this.overview_issue_comment_input.update(cx, |input, cx| {
              input.set_value("", window, cx);
            });
          }
          Err(error) => {
            let error_message = error.to_string();
            this.overview_issue_comment_error = Some(error_message.clone().into());
            this.record_pr_error(
              "github.pr.issue_comment.create",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        cx.notify();
      });
    });
    self.issue_comments_task = Some(task);
  }

  fn submit_overview_comment_edit(&mut self, cx: &mut Context<Self>) {
    if self.overview_comment_submission_in_flight()
      || self.overview_edit_submitting
      || self.overview_edit_target.is_none()
    {
      return;
    }
    let Some((owner, repo, issue_number)) = self.pull_request.as_ref().map(|pull_request| {
      (
        pull_request.repository.owner.clone(),
        pull_request.repository.repo.clone(),
        pull_request.number,
      )
    }) else {
      self.overview_edit_error = Some("No pull request selected".into());
      cx.notify();
      return;
    };
    let Some(target) = self.overview_edit_target else {
      return;
    };
    let Some(initial_body) = self.overview_edit_initial_body.clone() else {
      return;
    };
    let Some(input) = self.overview_edit_input.as_ref() else {
      return;
    };
    let raw_body = input.read(cx).value().to_string();
    let Some(next_body) = next_overview_comment_body(raw_body.as_str(), initial_body.as_str())
    else {
      self.clear_overview_edit_state();
      cx.notify();
      return;
    };

    self.overview_edit_submitting = true;
    self.overview_edit_error = None;
    cx.notify();

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || match target.kind {
        OverviewCommentKind::Issue => api
          .update_issue_comment(&owner, &repo, issue_number, target.id, next_body.as_str())
          .map(pull_request_issue_comment_from_issue_details_comment)
          .map(OverviewCommentUpdateResult::Issue),
        OverviewCommentKind::Review => api
          .update_pull_request_review_comment(
            &owner,
            &repo,
            issue_number,
            target.id,
            next_body.as_str(),
          )
          .map(OverviewCommentUpdateResult::review),
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.overview_edit_submitting = false;
        match result {
          Ok(OverviewCommentUpdateResult::Issue(comment)) => {
            this.upsert_issue_comment(comment);
            this.clear_overview_edit_state();
          }
          Ok(OverviewCommentUpdateResult::Review(comment)) => {
            this.upsert_review_comment(*comment);
            this.sync_review_comments(cx);
            this.clear_overview_edit_state();
          }
          Err(error) => {
            let error_message = error.to_string();
            this.overview_edit_error = Some(error_message.clone().into());
            let operation = match target.kind {
              OverviewCommentKind::Issue => "github.pr.issue_comment.update",
              OverviewCommentKind::Review => "github.pr.review_comment.update_overview",
            };
            this.record_pr_error(operation, error_message.as_str(), Map::new());
          }
        }
        cx.notify();
      });
    });

    match target.kind {
      OverviewCommentKind::Issue => self.issue_comments_task = Some(task),
      OverviewCommentKind::Review => self.review_comments_task = Some(task),
    }
  }

  fn submit_overview_review_comment_reply(&mut self, cx: &mut Context<Self>) {
    if self.overview_comment_submission_in_flight()
      || self.overview_reply_submitting
      || self.overview_reply_target_comment_id.is_none()
    {
      return;
    }

    let Some((owner, repo, number)) = self.pull_request.as_ref().map(|pull_request| {
      (
        pull_request.repository.owner.clone(),
        pull_request.repository.repo.clone(),
        pull_request.number,
      )
    }) else {
      self.overview_reply_error = Some("No pull request selected".into());
      cx.notify();
      return;
    };
    let Some(in_reply_to_id) = self.overview_reply_target_comment_id else {
      return;
    };
    let Some(input) = self.overview_reply_input.as_ref() else {
      return;
    };
    let raw_body = input.read(cx).value().to_string();
    let Some(body) = github_shared::normalize_non_empty_text(raw_body.as_str()) else {
      return;
    };

    self.overview_reply_submitting = true;
    self.overview_reply_error = None;
    cx.notify();

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        api.reply_pull_request_review_comment(&owner, &repo, number, in_reply_to_id, body.as_str())
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.overview_reply_submitting = false;
        match result {
          Ok(comment) => {
            this.upsert_review_comment(comment);
            this.sync_review_comments(cx);
            this.clear_overview_reply_state();
          }
          Err(error) => {
            let error_message = error.to_string();
            this.overview_reply_error = Some(error_message.clone().into());
            this.record_pr_error(
              "github.pr.review_comment.reply_overview",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        cx.notify();
      });
    });
    self.review_comments_task = Some(task);
  }

  fn confirm_overview_comment_delete(
    &mut self,
    target: OverviewCommentTarget,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.overview_comment_submission_in_flight() {
      return;
    }
    let title: SharedString = "Delete comment?".into();
    let message: SharedString = "This comment will be permanently deleted.".into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Delete")
        .cancel_text("Cancel")
        .destructive()
        .on_confirm(move |_, _, cx| {
          view.update(cx, |this, cx| {
            this.submit_overview_comment_delete(target, cx);
          });
          true
        })
        .build(alert)
    });
  }

  fn submit_overview_comment_delete(
    &mut self,
    target: OverviewCommentTarget,
    cx: &mut Context<Self>,
  ) {
    if self.overview_comment_submission_in_flight() {
      return;
    }
    let Some((owner, repo, issue_number)) = self.pull_request.as_ref().map(|pull_request| {
      (
        pull_request.repository.owner.clone(),
        pull_request.repository.repo.clone(),
        pull_request.number,
      )
    }) else {
      return;
    };

    match target.kind {
      OverviewCommentKind::Issue => {
        let Some((removed_index, removed_comment)) =
          remove_issue_comment_local(&mut self.issue_comments, target.id)
        else {
          return;
        };
        self.overview_issue_comment_submitting = true;
        self.overview_issue_comment_error = None;
        cx.notify();

        let api = self.api.clone();
        let task = cx.spawn(async move |this, cx| {
          let result =
            unblock(move || api.delete_issue_comment(&owner, &repo, issue_number, target.id)).await;
          let _ = this.update(cx, |this, cx| {
            this.overview_issue_comment_submitting = false;
            if let Err(error) = result {
              restore_issue_comment_local(
                &mut this.issue_comments,
                removed_index,
                removed_comment.clone(),
              );
              let error_message = error.to_string();
              this.overview_issue_comment_error = Some(error_message.clone().into());
              this.record_pr_error(
                "github.pr.issue_comment.delete",
                error_message.as_str(),
                Map::new(),
              );
            } else {
              this.overview_issue_comment_error = None;
            }
            cx.notify();
          });
        });
        self.issue_comments_task = Some(task);
      }
      OverviewCommentKind::Review => {
        let Some((removed_index, removed_comment)) =
          remove_review_comment_local(&mut self.review_comments, target.id)
        else {
          return;
        };
        self.overview_edit_submitting = true;
        self.overview_edit_error = None;
        self.sync_review_comments(cx);
        cx.notify();

        let api = self.api.clone();
        let task = cx.spawn(async move |this, cx| {
          let result = unblock(move || {
            api.delete_pull_request_review_comment(&owner, &repo, issue_number, target.id)
          })
          .await;
          let _ = this.update(cx, |this, cx| {
            this.overview_edit_submitting = false;
            if let Err(error) = result {
              restore_review_comment_local(
                &mut this.review_comments,
                removed_index,
                removed_comment.clone(),
              );
              let error_message = error.to_string();
              this.overview_edit_error = Some(error_message.clone().into());
              this.record_pr_error(
                "github.pr.review_comment.delete_overview",
                error_message.as_str(),
                Map::new(),
              );
            } else if this
              .overview_edit_target
              .is_some_and(|edit_target| edit_target.id == target.id)
            {
              this.clear_overview_edit_state();
            }
            this.sync_review_comments(cx);
            cx.notify();
          });
        });
        self.review_comments_task = Some(task);
      }
    }
  }

  fn set_active_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
    self.active_tab_ix = ix;
    self.sync_sentry_pr_context();
    let mut data = Map::new();
    data.insert("active_tab".into(), ix.into());
    self.add_pr_breadcrumb("Changed PR tab", data);
    cx.notify();

    // Sync URL with active tab
    if let Some(ctx) = &self.current_pr_context {
      let tab_segment = pr_tab_url_segment(ix);
      let path = if tab_segment.is_empty() {
        crate::navigation::build_pr_path(&ctx.owner, &ctx.repo, ctx.number)
      } else {
        crate::navigation::build_pr_tab_path(&ctx.owner, &ctx.repo, ctx.number, tab_segment)
      };
      NavigationHistory::navigate_replace(path, cx);
    }

    if ix == PR_TAB_CHANGES_IX {
      let saved = crate::config::AppSettings::get(cx).split_diff_view;
      let saved_mode = if saved {
        DiffViewMode::Split
      } else {
        DiffViewMode::Inline
      };
      if self.diff_view != saved_mode {
        self.diff_view = saved_mode;
        self.sync_diff_view(cx);
      }
      self.sync_tree_selection(cx);
      self.focus_changes_tree(window, cx);
      cx.on_next_frame(window, |this, window, cx| {
        if this.active_tab_ix == PR_TAB_CHANGES_IX {
          this.focus_changes_tree(window, cx);
        }
      });
    }
  }

  fn focus_changes_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.tree_state.update(cx, |state, cx| {
      state.focus(window, cx);
    });
  }

  fn set_selected_file(&mut self, selected: Option<Rc<GithubPrFileDiff>>, cx: &mut Context<Self>) {
    let current_id = self.selected_file.as_ref().map(|file| file.path.clone());
    let next_id = selected.as_ref().map(|file| file.path.clone());
    if current_id == next_id {
      return;
    }

    self.selected_file = selected.clone();
    self.selected_tree_id = selected.as_ref().map(|file| file.path.to_string());
    self.selected_local_project_file = None;
    self.selected_local_project_tree_id = None;
    self.local_project_open_file_task = None;
    self.local_project_open_file_generation =
      self.local_project_open_file_generation.wrapping_add(1);
    self.active_review_comment_id = None;
    self.selected_file_review_comment_ids.clear();
    self.sync_sentry_pr_context();
    let mut data = Map::new();
    if let Some(file) = self.selected_file.as_ref() {
      data.insert("selected_file".into(), file.path.to_string().into());
    }
    self.add_pr_breadcrumb("Selected PR file changed", data);
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      self.show_markdown_preview = false;
    }
    self.binary_preview = None;
    self.svg_preview = None;
    self.svg_preview_source = None;

    if let Some(file) = selected {
      self.ensure_diff_editor_for_path(file.path.as_ref(), cx);
      self.sync_diff_view(cx);
      self.sync_tree_selection(cx);
      let key = file.path.to_string();
      let path = Path::new(file.path.as_ref());
      match file_preview_kind(path) {
        Some(FilePreviewKind::RasterImage(_)) => {
          self.file_error = None;
          self.clear_diff_editor(cx);
          if let Some(preview) = self.file_asset_previews.get(&key).cloned() {
            self.binary_preview = Some(preview);
            self.file_loading = false;
          } else {
            self.file_loading = true;
            self.maybe_fetch_file_asset(file, cx);
          }
        }
        Some(FilePreviewKind::UnsupportedBinary) => {
          self.file_loading = false;
          self.file_error = None;
          self.binary_preview = Some(GithubPrBinaryPreview::UnsupportedBinary);
          self.clear_diff_editor(cx);
        }
        _ => {
          let cached = self.file_contents.contains_key(&key);
          let in_flight = self.file_content_tasks.contains_key(&key);
          let _ = (cached, in_flight);
          if let Some(contents) = self.file_contents.get(&key).cloned() {
            if contents.base.is_none() && contents.head.is_none() {
              self.file_loading = false;
              self.file_error = Some("File contents unavailable".into());
              self.clear_diff_editor(cx);
            } else {
              self.file_loading = false;
              self.file_error = None;
              self.apply_full_diff(&file, &contents, cx);
            }
          } else {
            self.file_loading = true;
            self.file_error = None;
            self.clear_diff_editor(cx);
            self.maybe_fetch_file_contents(file, cx);
          }
        }
      }
    } else {
      self.file_loading = false;
      self.file_error = None;
      self.clear_diff_editor(cx);
    }

    self.sync_review_comments(cx);
    cx.notify();
  }

  fn ensure_diff_editor_for_path(&mut self, path: &str, cx: &mut Context<Self>) {
    let desired_path = PathBuf::from(path);
    let mut current_path = None;
    self.diff_editor.update(cx, |editor, _| {
      current_path = Some(editor.workdir_path.clone());
    });
    if current_path.as_ref() == Some(&desired_path) {
      return;
    }

    self.diff_editor = Self::build_detached_diff_editor(desired_path, cx);
    self.install_diff_editor_review_comment_handlers(cx);
  }

  fn clear_diff_editor(&mut self, cx: &mut Context<Self>) {
    self.diff_editor.update(cx, |editor, cx| {
      editor.document().update(cx, |doc, cx| {
        doc.replace_all("", cx);
      });
      editor.reset_after_replace();
      editor.reset_selection(cx);
      editor.set_diffs(None, cx);
      editor.is_read_only = true;
    });
  }

  fn split_disabled_for_file(&self, file: &GithubPrFileDiff) -> bool {
    matches!(
      file.status,
      GithubPrFileStatus::Added | GithubPrFileStatus::Deleted
    )
  }

  fn split_disabled_for_selected_file(&self) -> bool {
    if self.binary_preview.is_some() {
      return true;
    }

    if self.show_local_project_files && self.selected_local_project_file.is_some() {
      return true;
    }

    self
      .selected_file
      .as_ref()
      .is_some_and(|file| self.split_disabled_for_file(file))
  }

  fn selected_file_is_markdown(&self) -> bool {
    if self.show_local_project_files && self.selected_local_project_file.is_some() {
      return self.selected_local_project_file_is_markdown();
    }

    self
      .selected_file
      .as_ref()
      .map(|file| is_markdown_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  fn selected_file_is_svg(&self) -> bool {
    if self.show_local_project_files && self.selected_local_project_file.is_some() {
      return self.selected_local_project_file_is_svg();
    }

    self
      .selected_file
      .as_ref()
      .map(|file| is_svg_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  fn build_binary_preview(
    path: &Path,
    binary_bytes: Option<Vec<u8>>,
  ) -> Option<GithubPrBinaryPreview> {
    if let Some(bytes) = binary_bytes {
      if let Some(image) = raster_image_from_bytes(path, bytes.clone()) {
        return Some(GithubPrBinaryPreview::RasterImage(image));
      }
      if should_show_unsupported_binary_placeholder(path, Some(bytes.as_slice())) {
        return Some(GithubPrBinaryPreview::UnsupportedBinary);
      }
      return None;
    }

    if matches!(
      file_preview_kind(path),
      Some(FilePreviewKind::UnsupportedBinary)
    ) {
      Some(GithubPrBinaryPreview::UnsupportedBinary)
    } else {
      None
    }
  }

  fn render_binary_preview_content(
    &self,
    preview: &GithubPrBinaryPreview,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();

    match preview {
      GithubPrBinaryPreview::RasterImage(image) => {
        let loading_color = theme.muted_foreground;
        let error_color = theme.status_red();
        let image_el = img(image.clone())
          .max_w_full()
          .max_h_full()
          .object_fit(ObjectFit::Contain)
          .with_loading(move || {
            div()
              .text_sm()
              .text_color(loading_color)
              .child("Rendering image preview...")
              .into_any_element()
          })
          .with_fallback(move || {
            div()
              .text_sm()
              .text_color(error_color)
              .child("Unable to render image preview")
              .into_any_element()
          });

        div()
          .flex_1()
          .min_h_0()
          .min_w(px(0.0))
          .overflow_hidden()
          .bg(theme.background)
          .debug_selector(|| GITHUB_PR_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR.to_string())
          .child(
            div().relative().size_full().child(
              div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .p_4()
                .flex()
                .items_center()
                .justify_center()
                .child(image_el),
            ),
          )
          .into_any_element()
      }
      GithubPrBinaryPreview::UnsupportedBinary => div()
        .flex_1()
        .min_h_0()
        .min_w(px(0.0))
        .bg(theme.background)
        .debug_selector(|| GITHUB_PR_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR.to_string())
        .child(
          v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
              Icon::new(IconName::File)
                .size_6()
                .text_color(theme.muted_foreground),
            )
            .child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Binary file preview is not available."),
            ),
        )
        .into_any_element(),
    }
  }

  fn review_comments_for_selected_file(&self) -> Vec<ReviewComment> {
    let Some(file) = self.selected_file.as_ref() else {
      return Vec::new();
    };

    let comments_for_file: Vec<&GithubPullRequestReviewComment> = self
      .review_comments
      .iter()
      .filter(|comment| comment.path == file.path)
      .collect();
    let comments_by_id: HashMap<u64, &GithubPullRequestReviewComment> = comments_for_file
      .iter()
      .map(|comment| (comment.id, *comment))
      .collect();

    self
      .review_comments
      .iter()
      .filter(|comment| review_comment_targets_file(comment, file))
      .filter_map(|comment| review_comment_to_editor_comment(comment, &comments_by_id))
      .collect()
  }

  fn review_comment_code_reference_requests_for_comments(
    &self,
    comments: &[ReviewComment],
  ) -> HashMap<u64, Vec<GithubBlobLineReference>> {
    comments
      .iter()
      .filter_map(|comment| {
        let references = code_reference_requests_from_markdown(comment.body.as_ref());
        if references.is_empty() {
          None
        } else {
          Some((comment.id, references))
        }
      })
      .collect()
  }

  fn description_code_reference_requests_for_pull_request(
    pull_request: &GithubPullRequestDetails,
  ) -> Vec<GithubBlobLineReference> {
    pull_request
      .body
      .as_deref()
      .map(code_reference_requests_from_markdown)
      .unwrap_or_default()
  }

  fn prefetch_overview_root_review_comment_files(&mut self, cx: &mut Context<Self>) {
    if self.review_comments.is_empty() || self.file_lookup.is_empty() {
      return;
    }

    let mut seen_paths = HashSet::new();
    for root_id in overview_root_review_comment_ids(&self.review_comments) {
      let Some(comment) = self
        .review_comments
        .iter()
        .find(|comment| comment.id == root_id)
      else {
        continue;
      };
      let Some(file) = self.file_for_review_comment_path(comment.path.as_str()) else {
        continue;
      };
      let canonical_path = file.path.to_string();
      if !seen_paths.insert(canonical_path) {
        continue;
      }
      self.maybe_fetch_file_contents(file, cx);
    }
  }

  fn overview_root_review_comment_preview(
    &self,
    comment_id: u64,
  ) -> Option<GithubCodeReferencePreview> {
    let comments_by_id: HashMap<u64, &GithubPullRequestReviewComment> = self
      .review_comments
      .iter()
      .map(|comment| (comment.id, comment))
      .collect();
    let comment = comments_by_id.get(&comment_id).copied()?;
    let root_id = resolve_review_comment_thread_root_id(comment, &comments_by_id);
    if root_id != comment_id {
      return None;
    }

    let file = self.file_for_review_comment_path(comment.path.as_str())?;
    let contents = self.file_contents.get(file.path.as_ref())?;
    let (start_line, end_line) = review_comment_preview_line_range(comment)?;
    let diff_refs = self.resolve_diff_refs()?;

    let preferred_source = match review_comment_preview_side(comment) {
      ReviewCommentPreviewSide::Left => contents.base.as_ref().map(|content| {
        (
          content.as_str(),
          diff_refs.base_owner.clone(),
          diff_refs.base_repo.clone(),
          diff_refs.base_sha.clone(),
        )
      }),
      ReviewCommentPreviewSide::Right => contents.head.as_ref().map(|content| {
        (
          content.as_str(),
          diff_refs.head_owner.clone(),
          diff_refs.head_repo.clone(),
          diff_refs.head_sha.clone(),
        )
      }),
    };

    let fallback_source = match review_comment_preview_side(comment) {
      ReviewCommentPreviewSide::Left => contents.head.as_ref().map(|content| {
        (
          content.as_str(),
          diff_refs.head_owner.clone(),
          diff_refs.head_repo.clone(),
          diff_refs.head_sha.clone(),
        )
      }),
      ReviewCommentPreviewSide::Right => contents.base.as_ref().map(|content| {
        (
          content.as_str(),
          diff_refs.base_owner.clone(),
          diff_refs.base_repo.clone(),
          diff_refs.base_sha.clone(),
        )
      }),
    };

    let (content, owner, repo, reference) = preferred_source.or(fallback_source)?;
    let snippets = github_shared::line_snippets_from_content(content, start_line, end_line)?;
    let actual_end_line = start_line.saturating_add(snippets.len().saturating_sub(1));
    let url = github_blob_url(
      owner.as_str(),
      repo.as_str(),
      reference.as_str(),
      comment.path.as_str(),
      start_line,
      actual_end_line,
    );

    Some(GithubCodeReferencePreview {
      url: Arc::<str>::from(url),
      repo: Arc::<str>::from(github_shared::repo_label(owner.as_str(), repo.as_str())),
      path: Arc::<str>::from(comment.path.as_str()),
      reference: Arc::<str>::from(reference),
      start_line,
      end_line: actual_end_line,
      snippets: snippets.into_iter().map(Arc::<str>::from).collect(),
    })
  }

  fn cached_review_comment_code_reference_previews(
    &self,
    requests: &HashMap<u64, Vec<GithubBlobLineReference>>,
  ) -> HashMap<u64, Vec<ReviewCommentCodeReferencePreview>> {
    requests
      .iter()
      .filter_map(|(comment_id, references)| {
        let previews: Vec<ReviewCommentCodeReferencePreview> = references
          .iter()
          .filter_map(|reference| {
            self
              .review_comment_code_reference_cache
              .get(&reference.url)
              .and_then(|preview| preview.clone())
          })
          .collect();
        if previews.is_empty() {
          None
        } else {
          Some((*comment_id, previews))
        }
      })
      .collect()
  }

  fn cached_github_code_reference_previews_for_requests(
    &self,
    requests: &[GithubBlobLineReference],
  ) -> Option<Arc<HashMap<Arc<str>, GithubCodeReferencePreview>>> {
    let previews: HashMap<Arc<str>, GithubCodeReferencePreview> = requests
      .iter()
      .filter_map(|reference| {
        self
          .review_comment_code_reference_cache
          .get(&reference.url)
          .and_then(|preview| preview.as_ref())
          .map(gfm_preview_from_review_preview)
          .map(|preview| (preview.url.clone(), preview))
      })
      .collect();

    if previews.is_empty() {
      None
    } else {
      Some(Arc::new(previews))
    }
  }

  fn schedule_code_reference_fetches<'a, I>(&mut self, references: I, cx: &mut Context<Self>)
  where
    I: IntoIterator<Item = &'a GithubBlobLineReference>,
  {
    for reference in references {
      if self
        .review_comment_code_reference_cache
        .contains_key(&reference.url)
        || self
          .review_comment_code_reference_tasks
          .contains_key(&reference.url)
      {
        continue;
      }

      let cache_key = reference.url.clone();
      let api = self.api.clone();
      let owner = reference.owner.clone();
      let repo = reference.repo.clone();
      let path = reference.path.clone();
      let revision = reference.reference.clone();
      let start_line = reference.start_line;
      let end_line = reference.end_line;
      let repo_label = github_shared::repo_label(&owner, &repo);
      let url = Arc::<str>::from(reference.url.as_str());
      let path_arc = Arc::<str>::from(path.as_str());
      let reference_arc = Arc::<str>::from(revision.as_str());
      let repo_arc = Arc::<str>::from(repo_label.as_str());

      let task = cx.spawn(async move |this, cx| {
        let result =
          unblock(move || api.fetch_github_file_content(&owner, &repo, &path, &revision)).await;

        let preview = match result {
          Ok(Some(content)) => github_shared::line_snippets_from_content(
            &content, start_line, end_line,
          )
          .map(|snippets| {
            let actual_end_line = start_line.saturating_add(snippets.len().saturating_sub(1));
            ReviewCommentCodeReferencePreview {
              url: url.clone(),
              repo: repo_arc.clone(),
              path: path_arc.clone(),
              reference: reference_arc.clone(),
              start_line,
              end_line: actual_end_line,
              snippets: snippets.into_iter().map(Arc::<str>::from).collect(),
            }
          }),
          _ => None,
        };

        let _ = this.update(cx, |this, cx| {
          this
            .review_comment_code_reference_cache
            .insert(cache_key.clone(), preview);
          this.review_comment_code_reference_tasks.remove(&cache_key);
          this.sync_review_comments(cx);
          cx.notify();
        });
      });

      self
        .review_comment_code_reference_tasks
        .insert(reference.url.clone(), task);
    }
  }

  fn sync_review_comments(&mut self, cx: &mut Context<Self>) {
    self.sync_review_comment_handlers(cx);
    if self.selected_commit_sha.is_some() {
      self.selected_file_review_comment_ids.clear();
      self.active_review_comment_id = None;
      self.diff_editor.update(cx, |editor, cx| {
        editor.set_review_comment_pr_number(None, cx);
        editor.set_editable_review_comment_ids(std::iter::empty::<u64>(), cx);
        editor.set_review_comments(Vec::new(), cx);
        editor.set_review_comment_code_reference_previews(HashMap::new(), cx);
      });
      self.pending_review_comment_link_comment_id = None;
      return;
    }

    let comments = self.review_comments_for_selected_file();
    self.selected_file_review_comment_ids = comments.iter().map(|comment| comment.id).collect();
    if self
      .active_review_comment_id
      .is_some_and(|id| !self.selected_file_review_comment_ids.contains(&id))
    {
      self.active_review_comment_id = None;
    }
    let preview_requests = self.review_comment_code_reference_requests_for_comments(&comments);
    let preview_map = self.cached_review_comment_code_reference_previews(&preview_requests);
    let pr_number = self.pull_request.as_ref().map(|pr| pr.number);
    let editable_comment_ids = self.editable_review_comment_ids(cx);
    self.diff_editor.update(cx, move |editor, cx| {
      editor.set_review_comment_pr_number(pr_number, cx);
      editor.set_editable_review_comment_ids(editable_comment_ids.iter().copied(), cx);
      editor.set_review_comments(comments, cx);
      editor.set_review_comment_code_reference_previews(preview_map, cx);
    });
    self.schedule_code_reference_fetches(
      preview_requests.values().flat_map(|items| items.iter()),
      cx,
    );
    self.resolve_pending_review_comment_link(cx);
  }

  fn effective_diff_view(&self) -> DiffViewMode {
    if self.show_markdown_preview
      && (self.selected_file_is_markdown() || self.selected_file_is_svg())
    {
      return DiffViewMode::Inline;
    }

    if self.split_disabled_for_selected_file() {
      return DiffViewMode::Inline;
    }

    self.diff_view
  }

  fn sync_diff_view(&mut self, cx: &mut Context<Self>) {
    let diff_view = self.effective_diff_view();
    self
      .diff_editor
      .update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
  }

  fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
    if self.split_disabled_for_selected_file() {
      return;
    }
    if self.show_markdown_preview
      && (self.selected_file_is_markdown() || self.selected_file_is_svg())
    {
      return;
    }

    self.diff_view = match self.diff_view {
      DiffViewMode::Inline => DiffViewMode::Split,
      DiffViewMode::Split => DiffViewMode::Inline,
    };
    AppSettings::update(cx, |s| {
      s.split_diff_view = self.diff_view == DiffViewMode::Split
    });
    self.sync_diff_view(cx);
    cx.notify();
  }

  fn toggle_markdown_preview(&mut self, cx: &mut Context<Self>) {
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      return;
    }

    self.show_markdown_preview = !self.show_markdown_preview;
    self.sync_diff_view(cx);
    cx.notify();
  }

  fn update_svg_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.show_markdown_preview || !self.selected_file_is_svg() {
      return;
    }

    let document = self.diff_editor.read(cx).document().read(cx);
    let svg_source = document.slice_to_string(0..document.len());
    let svg_source: SharedString = svg_source.into();

    if self.svg_preview_source.as_ref() == Some(&svg_source) {
      return;
    }

    self.svg_preview_source = Some(svg_source.clone());
    let renderer = cx.svg_renderer();
    let svg_bytes = svg_source.as_ref().as_bytes().to_vec();
    let background =
      cx.background_spawn(async move { renderer.render_single_frame(svg_bytes.as_slice(), 1.0) });

    let task = cx.spawn_in(window, async move |this, cx| {
      let result = background.await;
      let _ = this.update_in(cx, |this, window, cx| {
        if let Some(Ok(image)) = this.svg_preview.take() {
          let _ = window.drop_image(image);
        }
        this.svg_preview = Some(result.map_err(|err| err.to_string().into()));
        cx.notify();
      });
    });

    self.svg_preview_task = Some(task);
  }

  fn apply_full_diff(
    &mut self,
    file: &GithubPrFileDiff,
    contents: &GithubPrFileContents,
    cx: &mut Context<Self>,
  ) {
    self.file_loading = false;
    self.file_error = None;
    let head = contents.head.as_deref().unwrap_or("");
    let base = contents.base.as_deref();
    let _ = (
      file.path.clone(),
      contents.base.as_ref().map(|value| value.len()),
      head.len(),
    );
    let diff = compute_buffer_diff(
      DiffKind::Uncommitted,
      base,
      head,
      Path::new(file.path.as_ref()),
    )
    .ok();
    let Some(diff) = diff else {
      self.file_error = Some("Unable to compute diff".into());
      self.file_loading = false;
      return;
    };
    let diff_set = Some(DiffSet {
      uncommitted: diff,
      unstaged: FileDiff {
        kind: DiffKind::Unstaged,
        hunks: Vec::new(),
      },
      staged: FileDiff {
        kind: DiffKind::Staged,
        hunks: Vec::new(),
      },
    });

    self.diff_editor.update(cx, |editor, cx| {
      let _ = (
        editor.document().read(cx).len(),
        editor.document().read(cx).len_lines(),
      );
      editor.document().update(cx, |doc, cx| {
        doc.replace_all(head, cx);
      });
      editor.reset_after_replace();
      let _ = (
        editor.document().read(cx).len(),
        editor.document().read(cx).len_lines(),
      );
      editor.reset_selection(cx);
      editor.set_diffs(diff_set, cx);
      editor.is_read_only = true;
    });
    self.sync_diff_view(cx);
    self.resolve_pending_review_comment_link(cx);
  }

  fn selected_commit(&self) -> Option<&GithubPullRequestCommit> {
    self
      .selected_commit_sha
      .as_ref()
      .and_then(|sha| self.commit_lookup.get(sha))
  }

  fn resolve_diff_refs(&self) -> Option<GithubPrDiffRefs> {
    let pull_request = self.pull_request.as_ref()?;
    let base_owner = pull_request.repository.owner.clone();
    let base_repo = pull_request.repository.repo.clone();
    let head_owner = pull_request
      .head_repository
      .as_ref()
      .map(|repo| repo.owner.clone())
      .unwrap_or_else(|| base_owner.clone());
    let head_repo = pull_request
      .head_repository
      .as_ref()
      .map(|repo| repo.repo.clone())
      .unwrap_or_else(|| base_repo.clone());
    let selected_commit = self.selected_commit();
    let (resolved_base_sha, resolved_head_sha) = resolve_diff_shas_for_context(
      pull_request.merge_base_sha.as_str(),
      pull_request.base_sha.as_str(),
      pull_request.head_sha.as_str(),
      selected_commit.map(|commit| commit.sha.as_str()),
      selected_commit.and_then(|commit| commit.parent_sha.as_deref()),
    )?;

    if selected_commit.is_some() {
      return Some(GithubPrDiffRefs {
        base_owner: head_owner.clone(),
        base_repo: head_repo.clone(),
        base_sha: resolved_base_sha,
        head_owner,
        head_repo,
        head_sha: resolved_head_sha,
      });
    }

    Some(GithubPrDiffRefs {
      base_owner,
      base_repo,
      base_sha: resolved_base_sha,
      head_owner,
      head_repo,
      head_sha: resolved_head_sha,
    })
  }

  fn reset_files_state(&mut self, cx: &mut Context<Self>) {
    self.file_loading = false;
    self.file_error = None;
    self.files_error = None;
    self.tree_state.update(cx, |state, cx| {
      state.set_items(Vec::new(), cx);
    });
    self.file_lookup.clear();
    self.file_contents.clear();
    self.file_content_tasks.clear();
    self.file_asset_previews.clear();
    self.file_asset_tasks.clear();
    self.binary_preview = None;
    self.selected_tree_id = None;
    self.set_selected_file(None, cx);
    self.sync_review_comments(cx);
  }

  fn reload_files_for_current_pull_request(&mut self, cx: &mut Context<Self>) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };
    self.files_loading = true;
    self.reset_files_state(cx);
    self.fetch_pull_request_files_for_context(context.owner, context.repo, context.number, cx);
    cx.notify();
  }

  fn fetch_pull_request_files_for_context(
    &mut self,
    owner: String,
    repo: String,
    number: u64,
    cx: &mut Context<Self>,
  ) {
    self.files_request_generation = self.files_request_generation.wrapping_add(1);
    let generation = self.files_request_generation;
    let files_api = self.api.clone();
    let commit_sha = self.selected_commit_sha.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        files_api.fetch_pull_request_files(&owner, &repo, number, commit_sha.as_deref())
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        if generation != this.files_request_generation {
          return;
        }
        match result {
          Ok(files) => {
            this.files_loading = false;
            this.files_error = None;
            let files = files_from_api(files);
            let (_, lookup, _, _) = build_tree_items(&files);
            this.file_lookup = lookup;
            this.refresh_tree_text_search(cx);
            this.prefetch_overview_root_review_comment_files(cx);
            this.add_pr_breadcrumb("Load PR files succeeded", Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            this.files_loading = false;
            this.files_error = Some(error_message.clone().into());
            this.file_lookup.clear();
            this.file_contents.clear();
            this.file_content_tasks.clear();
            this.refresh_tree_text_search(cx);
            this.add_pr_breadcrumb("Load PR files failed", Map::new());
            this.record_pr_error("github.pr.files", error_message.as_str(), Map::new());
          }
        }
        cx.notify();
      });
    });
    self.files_task = Some(task);
  }

  fn maybe_fetch_selected_file_contents(&mut self, cx: &mut Context<Self>) {
    if let Some(file) = self.selected_file.clone() {
      match file_preview_kind(Path::new(file.path.as_ref())) {
        Some(FilePreviewKind::RasterImage(_)) => self.maybe_fetch_file_asset(file, cx),
        Some(FilePreviewKind::UnsupportedBinary) => {}
        _ => self.maybe_fetch_file_contents(file, cx),
      }
    }
  }

  fn maybe_fetch_file_asset(&mut self, file: Rc<GithubPrFileDiff>, cx: &mut Context<Self>) {
    let key = file.path.to_string();
    let key_for_task = key.clone();
    if self.file_asset_previews.contains_key(&key) || self.file_asset_tasks.contains_key(&key) {
      return;
    }

    let Some(diff_refs) = self.resolve_diff_refs() else {
      return;
    };
    let (owner, repo, reference, preview_path) = match file.status {
      GithubPrFileStatus::Deleted => (
        diff_refs.base_owner,
        diff_refs.base_repo,
        diff_refs.base_sha,
        file
          .old_path
          .as_ref()
          .map(|path| path.to_string())
          .unwrap_or_else(|| file.path.to_string()),
      ),
      _ => (
        diff_refs.head_owner,
        diff_refs.head_repo,
        diff_refs.head_sha,
        file.path.to_string(),
      ),
    };

    let api = self.api.clone();
    let preview_path_for_request = preview_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let asset_result = unblock(move || {
        api.fetch_github_file_asset(&owner, &repo, &preview_path_for_request, &reference)
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.file_asset_tasks.remove(&key_for_task);
        let is_selected_file = this.selected_tree_id.as_deref() == Some(key_for_task.as_str());

        match asset_result {
          Ok(Some(bytes)) => {
            let preview = Self::build_binary_preview(Path::new(preview_path.as_str()), Some(bytes));
            let Some(preview) = preview else {
              if is_selected_file {
                this.file_loading = false;
                this.file_error = Some("Unable to render file preview".into());
              }
              cx.notify();
              return;
            };
            this
              .file_asset_previews
              .insert(key_for_task.clone(), preview.clone());
            if is_selected_file {
              this.binary_preview = Some(preview);
              this.file_loading = false;
              this.file_error = None;
            }
          }
          Ok(None) => {
            if is_selected_file {
              this.file_loading = false;
              this.file_error = Some("File preview unavailable".into());
            }
          }
          Err(error) => {
            if is_selected_file {
              this.file_loading = false;
              this.file_error = Some(error.to_string().into());
            }
          }
        }
        cx.notify();
      });
    });

    self.file_asset_tasks.insert(key, task);
  }

  fn maybe_fetch_file_contents(&mut self, file: Rc<GithubPrFileDiff>, cx: &mut Context<Self>) {
    match file_preview_kind(Path::new(file.path.as_ref())) {
      Some(FilePreviewKind::RasterImage(_)) | Some(FilePreviewKind::UnsupportedBinary) => return,
      _ => {}
    }

    let key = file.path.to_string();
    let key_for_task = key.clone();
    if self.file_contents.contains_key(&key) || self.file_content_tasks.contains_key(&key) {
      return;
    }

    let Some(diff_refs) = self.resolve_diff_refs() else {
      return;
    };
    let base_owner = diff_refs.base_owner;
    let base_repo = diff_refs.base_repo;
    let base_sha = diff_refs.base_sha;
    let head_owner = diff_refs.head_owner;
    let head_repo = diff_refs.head_repo;
    let head_sha = diff_refs.head_sha;

    let base_path = match file.status {
      GithubPrFileStatus::Added => None,
      GithubPrFileStatus::Renamed => file
        .old_path
        .as_ref()
        .map(|path| path.to_string())
        .or_else(|| Some(file.path.to_string())),
      _ => Some(file.path.to_string()),
    };
    let head_path = match file.status {
      GithubPrFileStatus::Deleted => None,
      _ => Some(file.path.to_string()),
    };

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let base_result = if let Some(path) = base_path.clone() {
        let api = api.clone();
        let owner = base_owner.clone();
        let repo = base_repo.clone();
        let base_sha = base_sha.clone();
        unblock(move || api.fetch_github_file_content(&owner, &repo, &path, &base_sha)).await
      } else {
        Ok(None)
      };

      let head_result = if let Some(path) = head_path.clone() {
        let api = api.clone();
        let owner = head_owner.clone();
        let repo = head_repo.clone();
        let head_sha = head_sha.clone();
        unblock(move || api.fetch_github_file_content(&owner, &repo, &path, &head_sha)).await
      } else {
        Ok(None)
      };

      let _ = this.update(cx, |this, cx| {
        this.file_content_tasks.remove(&key_for_task);
        let is_selected_file = this.selected_tree_id.as_deref() == Some(key_for_task.as_str());
        let (base, head) = match (base_result, head_result) {
          (Ok(base), Ok(head)) => (base, head),
          _ => {
            if is_selected_file {
              this.file_loading = false;
              this.file_error = Some("Failed to load file contents".into());
            }
            cx.notify();
            return;
          }
        };

        if base.is_none() && head.is_none() {
          if is_selected_file {
            this.file_loading = false;
            this.file_error = Some("File contents unavailable".into());
          }
          this
            .file_contents
            .insert(key_for_task.clone(), GithubPrFileContents { base, head });
          cx.notify();
          return;
        }

        this
          .file_contents
          .insert(key_for_task.clone(), GithubPrFileContents { base, head });

        if is_selected_file
          && let Some(file) = this.file_lookup.get(&key_for_task).cloned()
          && let Some(contents) = this.file_contents.get(&key_for_task).cloned()
        {
          this.apply_full_diff(&file, &contents, cx);
        }
        cx.notify();
      });
    });

    self.file_content_tasks.insert(key, task);
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
    self.merge_readiness_task = None;
    self.merge_readiness_loading = true;
    self.merge_readiness_error = None;
    self.merge_readiness = None;
    self.merge_popover_open = false;
    self.merge_submit_task = None;
    self.merge_submit_loading = false;
    self.status_action_task = None;
    self.status_action_loading = false;
    self.mark_merge_form_reset_pending();
    self.review_popover_open = false;
    self.mark_review_form_reset_pending();
    self.submit_review_task = None;
    self.issue_comments_task = None;
    self.reviews_task = None;
    self.submit_review_loading = false;
    self.sync_commits_list(cx);
    self.issue_comments_loading = true;
    self.issue_comments_error = None;
    self.issue_comments.clear();
    self.overview_issue_comment_submitting = false;
    self.overview_issue_comment_error = None;
    self.reviews_loading = true;
    self.reviews_error = None;
    self.reviews.clear();
    self.review_comments_loading = true;
    self.review_comments_error = None;
    self.review_comments.clear();
    self.overview_edit_target = None;
    self.overview_edit_initial_body = None;
    self.overview_edit_submitting = false;
    self.overview_edit_error = None;
    self.overview_reply_target_comment_id = None;
    self.overview_reply_submitting = false;
    self.overview_reply_error = None;
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
    self.svg_preview = None;
    self.svg_preview_source = None;
    self.diff_editor.update(cx, |editor, cx| {
      editor.document().update(cx, |doc, cx| {
        doc.replace_all("", cx);
      });
      editor.is_read_only = true;
    });

    let details_api = self.api.clone();
    let details_owner = owner.clone();
    let details_repo = repo.clone();
    let details_task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
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

    let issue_comments_api = self.api.clone();
    let issue_comments_owner = owner.clone();
    let issue_comments_repo = repo.clone();
    let issue_comments_task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        issue_comments_api.fetch_pull_request_issue_comments(
          &issue_comments_owner,
          &issue_comments_repo,
          number,
        )
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(comments) => {
            this.issue_comments = comments;
            this.issue_comments_loading = false;
            this.issue_comments_error = None;
            this.add_pr_breadcrumb("Load PR issue comments succeeded", Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            this.issue_comments_loading = false;
            this.issue_comments_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Load PR issue comments failed", Map::new());
            this.record_pr_error(
              "github.pr.issue_comments",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        cx.notify();
      });
    });

    let reviews_api = self.api.clone();
    let reviews_owner = owner.clone();
    let reviews_repo = repo.clone();
    let reviews_task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        reviews_api.fetch_pull_request_reviews(&reviews_owner, &reviews_repo, number)
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(reviews) => {
            this.reviews = reviews;
            this.reviews_loading = false;
            this.reviews_error = None;
            this.add_pr_breadcrumb("Load PR reviews succeeded", Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            this.reviews_loading = false;
            this.reviews_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Load PR reviews failed", Map::new());
            this.record_pr_error("github.pr.reviews", error_message.as_str(), Map::new());
          }
        }
        cx.notify();
      });
    });

    let comments_api = self.api.clone();
    let comments_owner = owner.clone();
    let comments_repo = repo.clone();
    let review_comments_task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        comments_api.fetch_pull_request_review_comments(&comments_owner, &comments_repo, number)
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(comments) => {
            this.review_comments = comments;
            this.review_comments_loading = false;
            this.review_comments_error = None;
            this.add_pr_breadcrumb("Load PR comments succeeded", Map::new());
            this.sync_review_comments(cx);
            this.prefetch_overview_root_review_comment_files(cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            this.review_comments_loading = false;
            this.review_comments_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Load PR comments failed", Map::new());
            this.record_pr_error("github.pr.comments", error_message.as_str(), Map::new());
            this.sync_review_comments(cx);
          }
        }
        cx.notify();
      });
    });

    let commits_api = self.api.clone();
    let commits_owner = owner.clone();
    let commits_repo = repo.clone();
    let commits_task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
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
            this.sync_commits_list(cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            this.commits_loading = false;
            this.commits_error = Some(error_message.clone().into());
            this.commits.clear();
            this.commit_lookup.clear();
            this.selected_commit_sha = None;
            this.sync_commits_list(cx);
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
      let result =
        unblock(move || checks_api.fetch_pull_request_checks(&checks_owner, &checks_repo, number))
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
    self.issue_comments_task = Some(issue_comments_task);
    self.reviews_task = Some(reviews_task);
    self.review_comments_task = Some(review_comments_task);
    self.commits_task = Some(commits_task);
    self.checks_task = Some(checks_task);
    self.fetch_merge_readiness_for_context(owner.clone(), repo.clone(), number, cx);
    self.fetch_pull_request_files_for_context(owner, repo, number, cx);
  }

  fn navigate_back(&self, cx: &mut Context<Self>) {
    NavigationHistory::navigate_back(cx);
  }

  fn render_merge_popover(
    &mut self,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let merge_readiness = self.merge_readiness.clone();
    let merge_status = merge_readiness
      .as_ref()
      .map(|readiness| readiness.status)
      .unwrap_or(GithubPullRequestMergeReadinessStatus::Checking);
    let available_methods = merge_readiness
      .as_ref()
      .map(|readiness| readiness.available_methods.clone())
      .unwrap_or_default();
    let selected_method = self.selected_merge_method();
    let selected_method_index = selected_method.and_then(|method| {
      available_methods
        .iter()
        .position(|candidate| *candidate == method)
    });
    let can_submit_merge = self.merge_readiness.as_ref().is_some_and(|readiness| {
      readiness.can_merge_now
        && readiness
          .available_methods
          .iter()
          .any(|method| Some(*method) == selected_method)
    });
    let show_commit_fields = selected_method.is_some_and(merge_method_supports_commit_message);
    let merge_button_disabled = self.pull_request.is_none();
    let merge_message = self
      .merge_submit_error
      .clone()
      .or_else(|| self.merge_readiness_error.clone())
      .or_else(|| {
        merge_readiness
          .as_ref()
          .map(|readiness| readiness.message.clone().into())
      });

    Popover::new("pr-merge-popover")
      .anchor(Corner::TopRight)
      .w(px(PR_MERGE_POPOVER_WIDTH))
      .open(self.merge_popover_open)
      .on_open_change(cx.listener(|this, open, window, cx| {
        this.merge_popover_open = *open;
        if *open && this.merge_form_reset_pending {
          this.reset_merge_form(window, cx);
        }
        cx.notify();
      }))
      .trigger(
        Button::new("pr-merge-button")
          .label("Merge")
          .with_variant(ButtonVariant::Secondary)
          .outline()
          .icon(IconName::ChevronDown)
          .small()
          .disabled(merge_button_disabled),
      )
      .child(
        v_flex()
          .id("pr-merge-popover-content")
          .w_full()
          .gap_3()
          .child(
            div()
              .text_sm()
              .font_medium()
              .text_color(theme.foreground)
              .child("Merge pull request"),
          )
          .when(self.merge_readiness_loading && self.merge_readiness.is_none(), |this| {
            this.child(
              h_flex()
                .items_center()
                .gap_2()
                .child(Spinner::new().small())
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Checking merge readiness..."),
                ),
            )
          })
          .when(
            !available_methods.is_empty()
              && matches!(
                merge_status,
                GithubPullRequestMergeReadinessStatus::Ready
                  | GithubPullRequestMergeReadinessStatus::Blocked
              ),
            |this| {
              let methods_for_click = available_methods.clone();
              let mut group = RadioGroup::vertical("pr-merge-method-group")
                .selected_index(selected_method_index)
                .on_click(cx.listener(move |this, index: &usize, _, cx| {
                  if let Some(method) = methods_for_click.get(*index).copied() {
                    this.merge_method = method;
                    this.merge_submit_error = None;
                    cx.notify();
                  }
                }));

              for method in &available_methods {
                let id = match method {
                  GithubPullRequestMergeMethod::Merge => "pr-merge-method-merge",
                  GithubPullRequestMergeMethod::Squash => "pr-merge-method-squash",
                  GithubPullRequestMergeMethod::Rebase => "pr-merge-method-rebase",
                };
                group = group.child(Radio::new(id).label(merge_method_label(*method)));
              }

              this
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Choose how GitHub should merge this pull request."),
                )
                .child(group)
            },
          )
          .when(show_commit_fields, |this| {
            this
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child("Leave the fields empty to let GitHub generate the default commit title and message."),
              )
              .child(
                div()
                  .w_full()
                  .debug_selector(|| "github-pr-merge-title-input".to_string())
                  .child(Input::new(&self.merge_commit_title_input).w_full()),
              )
              .child(
                div()
                  .w_full()
                  .debug_selector(|| "github-pr-merge-message-input".to_string())
                  .child(
                    Input::new(&self.merge_commit_message_input)
                      .w_full()
                      .h(px(PR_MERGE_MESSAGE_INPUT_HEIGHT_PX)),
                  ),
              )
          })
          .when_some(merge_message, |this, message| {
            let color = if self.merge_submit_error.is_some() || self.merge_readiness_error.is_some()
            {
              theme.status_red()
            } else if matches!(merge_status, GithubPullRequestMergeReadinessStatus::Ready) {
              theme.muted_foreground
            } else {
              theme.status_orange()
            };

            this.child(
              div()
                .text_xs()
                .text_color(color)
                .child(message),
            )
          })
          .child(
            h_flex()
              .items_center()
              .justify_end()
              .gap_2()
              .child(
                Button::new("pr-merge-cancel")
                  .ghost()
                  .small()
                  .label("Cancel")
                  .disabled(self.merge_submit_loading)
                  .on_click(cx.listener(|this, _, window, cx| {
                    this.merge_popover_open = false;
                    this.reset_merge_form(window, cx);
                    cx.notify();
                  })),
              )
              .child(
                Button::new("pr-merge-submit")
                  .primary()
                  .small()
                  .label("Merge pull request")
                  .loading(self.merge_submit_loading)
                  .disabled(!can_submit_merge)
                  .on_click(cx.listener(|this, _, window, cx| {
                    this.submit_pull_request_merge(window, cx);
                  })),
              ),
          ),
      )
      .into_any_element()
  }

  fn render_review_popover(
    &mut self,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let author_cannot_approve_tooltip =
      "Pull request authors cannot approve their own pull requests.".to_string();
    let author_cannot_request_changes_tooltip =
      "Pull request authors cannot request changes on their own pull requests.".to_string();
    let is_current_user_pr_author = self.is_current_user_pr_author(cx);
    let review_body = self.review_input.read(cx).value().to_string();
    let submit_review_disabled = self.submit_review_loading
      || self.pull_request.is_none()
      || (is_current_user_pr_author
        && !matches!(self.review_decision, GithubPrReviewDecision::Comment))
      || Self::validate_review_submission(self.review_decision, review_body.as_str()).is_some();
    let review_decision_index = Self::review_decision_index(self.review_decision);
    let review_button_disabled = self.pull_request.is_none();

    Popover::new("pr-review-popover")
      .anchor(Corner::TopRight)
      .w(px(PR_REVIEW_POPOVER_WIDTH))
      .open(self.review_popover_open)
      .on_open_change(cx.listener(|this, open, window, cx| {
        this.review_popover_open = *open;
        if *open {
          if this.review_form_reset_pending {
            this.reset_review_form(window, cx);
          }
          if this.is_current_user_pr_author(cx)
            && !matches!(this.review_decision, GithubPrReviewDecision::Comment)
          {
            this.review_decision = GithubPrReviewDecision::Comment;
          }
          this.focus_review_input(window);
        }
        cx.notify();
      }))
      .trigger(
        Button::new("pr-review-button")
          .label("Review")
          .with_variant(ButtonVariant::Secondary)
          .outline()
          .icon(IconName::ChevronDown)
          .small()
          .disabled(review_button_disabled),
      )
      .child(
        v_flex()
          .id("pr-review-popover-content")
          .w_full()
          .gap_3()
          .child(
            div()
              .text_sm()
              .font_medium()
              .text_color(theme.foreground)
              .child("Submit review"),
          )
          .child(
            div().w_full().child(
              Input::new(&self.review_input)
                .w_full()
                .h(px(PR_REVIEW_INPUT_HEIGHT_PX)),
            ),
          )
          .child(
            RadioGroup::vertical("pr-review-decision-group")
              .selected_index(Some(review_decision_index))
              .on_click(cx.listener(|this, index: &usize, _, cx| {
                let next_decision = Self::review_decision_from_index(*index);
                if this.is_current_user_pr_author(cx)
                  && !matches!(next_decision, GithubPrReviewDecision::Comment)
                {
                  this.review_decision = GithubPrReviewDecision::Comment;
                  this.submit_review_error = Some(
                    "Pull request authors cannot approve or request changes on their own pull requests."
                      .into(),
                  );
                  cx.notify();
                  return;
                }
                this.review_decision = next_decision;
                this.submit_review_error = None;
                cx.notify();
              }))
              .child(Radio::new("pr-review-decision-comment").label("Comment"))
              .child(
                Radio::new("pr-review-decision-approve")
                  .label("Approve")
                  .disabled(is_current_user_pr_author)
                  .when(is_current_user_pr_author, |this| {
                    this.tooltip({
                      let tooltip = author_cannot_approve_tooltip.clone();
                      move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)
                    })
                  }),
              )
              .child(
                Radio::new("pr-review-decision-request-changes")
                  .label("Request changes")
                  .disabled(is_current_user_pr_author)
                  .when(is_current_user_pr_author, |this| {
                    this.tooltip({
                      let tooltip = author_cannot_request_changes_tooltip.clone();
                      move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)
                    })
                  }),
              ),
          )
          .when_some(self.submit_review_error.clone(), |this, error| {
            this.child(
              div()
                .text_xs()
                .text_color(theme.status_red())
                .overflow_hidden()
                .text_ellipsis_start()
                .child(error),
            )
          })
          .child(
            h_flex()
              .items_center()
              .justify_end()
              .gap_2()
              .child(
                Button::new("pr-review-cancel")
                  .ghost()
                  .small()
                  .label("Cancel")
                  .disabled(self.submit_review_loading)
                  .on_click(cx.listener(|this, _, window, cx| {
                    this.review_popover_open = false;
                    this.reset_review_form(window, cx);
                    cx.notify();
                  })),
              )
              .child(
                Button::new("pr-review-submit")
                  .primary()
                  .small()
                  .label("Submit review")
                  .loading(self.submit_review_loading)
                  .disabled(submit_review_disabled)
                  .on_click(cx.listener(|this, _, window, cx| {
                    this.submit_pull_request_review(window, cx);
                  })),
              ),
          ),
      )
      .into_any_element()
  }

  fn render_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let changes_tab = if let Some(pull_request) = self.pull_request.as_ref() {
      Tab::new().child(
        h_flex().items_center().gap_2().child("Changes").child(
          div()
            .debug_selector(|| "github-pr-changes-tab-count".to_string())
            .child(
              Tag::secondary()
                .small()
                .rounded_full()
                .child(pr_changes_tab_count_label(pull_request.changed_files)),
            ),
        ),
      )
    } else {
      Tab::new().label("Changes")
    };

    let checks_tab = if let Some(checks) = self.checks.as_ref() {
      Tab::new().child(
        h_flex()
          .debug_selector(|| "github-pr-checks-tab-status".to_string())
          .items_center()
          .gap_2()
          .child("Checks")
          .child(render_checks_state_badge(checks.overall_state, &theme)),
      )
    } else {
      Tab::new().label("Checks")
    };

    let tab_bar = TabBar::new("pr-details-tabs")
      .w_full()
      .segmented()
      .selected_index(self.active_tab_ix)
      .on_click(cx.listener(|this, ix: &usize, window, cx| {
        this.set_active_tab(*ix, window, cx);
      }))
      .child(Tab::new().label("Overview"))
      .child(changes_tab)
      .child(checks_tab);

    let back_button = || {
      Button::new("pr-back")
        .icon(IconName::ArrowLeft)
        .ghost()
        .compact()
        .on_click(cx.listener(|this, _, _, cx| {
          this.navigate_back(cx);
        }))
    };

    let left_area = if let Some(pr) = self.pull_request.as_ref() {
      let status_tag = github_shared::pull_request_status_tag(pr.status(), &theme);

      let title = div()
        .min_w_0()
        .text_sm()
        .font_medium()
        .text_color(theme.foreground)
        .overflow_hidden()
        .text_ellipsis_start()
        .child(pr.title.clone());

      let meta = h_flex()
        .items_center()
        .gap_2()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(format!("#{}", pr.number))
        .child(status_tag);

      h_flex()
        .items_center()
        .gap_2()
        .child(back_button())
        .child(div().flex().items_center().gap_2().child(title).child(meta))
    } else {
      let title_skeleton = Skeleton::new().w(px(220.)).h_4().rounded_md();
      let meta_skeleton = Skeleton::new().w(px(110.)).h_4().rounded_md().secondary();
      h_flex().items_center().gap_2().child(back_button()).child(
        div()
          .flex()
          .items_center()
          .gap_3()
          .child(title_skeleton)
          .child(meta_skeleton),
      )
    };
    let right_area = h_flex()
      .items_center()
      .gap_2()
      .when(!self.is_pull_request_merged(), |this| {
        this
          .when_some(self.pull_request_status_action(), |this, action| {
            this.child(
              div()
                .debug_selector(|| "github-pr-status-action-button".to_string())
                .child(
                  Button::new("pr-status-action-button")
                    .label(action.button_label())
                    .small()
                    .outline()
                    .loading(self.status_action_loading)
                    .disabled(self.status_action_loading)
                    .on_click(cx.listener(move |this, _, _, cx| {
                      this.submit_pull_request_status_action(action, cx);
                    })),
                ),
            )
          })
          .when(
            !self
              .pull_request
              .as_ref()
              .is_some_and(|pull_request| pull_request.draft),
            |this| {
              this.child(
                div()
                  .debug_selector(|| "github-pr-merge-button".to_string())
                  .child(self.render_merge_popover(&theme, cx)),
              )
            },
          )
          .child(
            div()
              .debug_selector(|| "github-pr-review-button".to_string())
              .child(self.render_review_popover(&theme, cx)),
          )
      });

    div()
      .px_3()
      .pt_2()
      .pb_3()
      .flex()
      .flex_col()
      .gap_1()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(
        div()
          .flex()
          .items_center()
          .justify_between()
          .child(left_area)
          .child(right_area),
      )
      .child(tab_bar)
  }

  fn render_context_sidebar_checks_summary(
    &self,
    show_view_checks_button: bool,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let theme = cx.theme().clone();

    if self.checks_loading && self.checks.is_none() {
      return v_flex()
        .gap_3()
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .p_3()
        .child(
          h_flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
              div()
                .text_sm()
                .font_medium()
                .text_color(theme.foreground)
                .child("Checks summary"),
            )
            .child(Spinner::new().small()),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading checks..."),
        )
        .into_any_element();
    }

    if let Some(error) = self.checks_error.as_ref() {
      return v_flex()
        .gap_3()
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .p_3()
        .child(
          div()
            .text_sm()
            .font_medium()
            .text_color(theme.foreground)
            .child("Checks summary"),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme.status_red())
            .child(error.clone()),
        )
        .into_any_element();
    }

    if let Some(checks) = self.checks.as_ref() {
      let view_checks_button = if show_view_checks_button {
        let view = cx.entity();
        Some(
          Button::new("pr-overview-view-checks")
            .ghost()
            .xsmall()
            .label("View check details")
            .on_click(move |_, window, cx| {
              view.update(cx, |this, cx| {
                this.set_active_tab(PR_TAB_CHECKS_IX, window, cx);
              });
            }),
        )
      } else {
        None
      };

      return v_flex()
        .gap_3()
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .p_3()
        .child(
          v_flex()
            .gap_2()
            .child(
              h_flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                  div()
                    .text_sm()
                    .font_medium()
                    .text_color(theme.foreground)
                    .child("Checks summary"),
                )
                .when_some(view_checks_button, |this, button| this.child(button)),
            )
            .child(
              h_flex()
                .items_center()
                .gap_2()
                .flex_wrap()
                .child(render_checks_state_badge(checks.overall_state, &theme))
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Overall"),
                )
                .child(render_checks_state_badge(checks.required_state, &theme))
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Required"),
                ),
            ),
        )
        .child(
          h_flex()
            .items_center()
            .gap_4()
            .flex_wrap()
            .child(
              div()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Total checks"),
                )
                .child(
                  div()
                    .text_sm()
                    .font_medium()
                    .text_color(theme.foreground)
                    .child(checks.total_checks.to_string()),
                ),
            )
            .child(
              div()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Passing"),
                )
                .child(
                  div()
                    .text_sm()
                    .font_medium()
                    .text_color(theme.status_green())
                    .child(checks.successful_checks.to_string()),
                ),
            )
            .child(
              div()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Failing"),
                )
                .child(
                  div()
                    .text_sm()
                    .font_medium()
                    .text_color(theme.status_red())
                    .child(checks.failed_checks.to_string()),
                ),
            )
            .child(
              div()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Pending"),
                )
                .child(
                  div()
                    .text_sm()
                    .font_medium()
                    .text_color(theme.status_orange())
                    .child(checks.pending_checks.to_string()),
                ),
            ),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(format!(
              "{} required contexts • {} passed • {} failing • {} pending",
              checks.required_checks_total,
              checks.required_checks_passed,
              checks.required_checks_failed,
              checks.required_checks_pending,
            )),
        )
        .into_any_element();
    }

    v_flex()
      .gap_3()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .p_3()
      .child(
        div()
          .text_sm()
          .font_medium()
          .text_color(theme.foreground)
          .child("Checks summary"),
      )
      .child(
        div()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child("Checks are unavailable for this pull request."),
      )
      .into_any_element()
  }

  fn render_context_sidebar_commits(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
    let theme = cx.theme().clone();

    if self.commits_loading {
      return v_flex()
        .flex_1()
        .min_h_0()
        .gap_3()
        .child(
          div()
            .text_sm()
            .font_medium()
            .text_color(theme.foreground)
            .child("Commits"),
        )
        .child(
          v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap_2()
            .child(Spinner::new().small())
            .child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Loading commits..."),
            ),
        )
        .into_any_element();
    }

    if let Some(error) = self.commits_error.as_ref() {
      return v_flex()
        .flex_1()
        .min_h_0()
        .gap_3()
        .child(
          div()
            .text_sm()
            .font_medium()
            .text_color(theme.foreground)
            .child("Commits"),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme.status_red())
            .child(error.clone()),
        )
        .into_any_element();
    }

    let commits_list = List::new(&self.commits_list)
      .search_placeholder("Search commits...")
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .size_full()
      .p(px(8.));

    v_flex()
      .flex_1()
      .min_h_0()
      .gap_3()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_2()
          .child(
            div()
              .text_sm()
              .font_medium()
              .text_color(theme.foreground)
              .child("Commits"),
          )
          .child(
            Tag::secondary()
              .small()
              .rounded_full()
              .child(self.commits.len().to_string()),
          ),
      )
      .child(div().flex_1().min_h_0().child(commits_list))
      .into_any_element()
  }

  fn render_context_sidebar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
    let theme = cx.theme().clone();

    v_flex()
      .id("github-pr-context-sidebar")
      .size_full()
      .bg(theme.sidebar)
      .gap_3()
      .p_3()
      .child(self.render_context_sidebar_checks_summary(self.active_tab_ix != PR_TAB_CHECKS_IX, cx))
      .child(self.render_context_sidebar_commits(cx))
      .into_any_element()
  }

  fn render_overview_add_comment_section(
    &self,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    v_flex()
      .gap_2()
      .pt_2()
      .pb_32()
      .child(
        div()
          .text_sm()
          .font_medium()
          .text_color(theme.foreground)
          .child("Add comment"),
      )
      .child(div().w_full().child(
        Input::new(&self.overview_issue_comment_input).h(px(OVERVIEW_COMMENT_INPUT_HEIGHT_PX)),
      ))
      .when_some(self.overview_issue_comment_error.clone(), |this, error| {
        this.child(div().text_xs().text_color(theme.status_red()).child(error))
      })
      .child(
        h_flex().items_center().justify_end().gap_2().child(
          Button::new("pr-overview-issue-comment-save")
            .xsmall()
            .compact()
            .label("Comment")
            .disabled(
              self.overview_issue_comment_submitting
                || self.overview_comment_submission_in_flight()
                || github_shared::normalize_non_empty_text(
                  self.overview_issue_comment_input.read(cx).value().as_str(),
                )
                .is_none(),
            )
            .on_click({
              let page = cx.entity().clone();
              move |_, window, cx| {
                page.update(cx, |this, cx| {
                  this.submit_overview_issue_comment_create(window, cx);
                });
              }
            }),
        ),
      )
      .into_any_element()
  }

  fn render_overview_conversation_item(
    &self,
    ix: usize,
    pr_number: u64,
    pr_owner: SharedString,
    pr_repo: SharedString,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let Some(item) = self.overview_conversation_items.get(ix) else {
      return div().into_any_element();
    };

    let theme = cx.theme().clone();
    let timestamp = format_relative_time(&item.timestamp);
    let scope_id = overview_conversation_scope_id(pr_number, item.kind, item.id);
    let editable_issue_comment_ids = self.editable_issue_comment_ids(cx);
    let editable_review_comment_ids = self.editable_review_comment_ids(cx);
    let overview_submission_in_flight = self.overview_comment_submission_in_flight();
    let editing_target = self.overview_edit_target;
    let replying_target = self.overview_reply_target_comment_id;

    let pr_page = cx.entity().clone();
    let pr_page_for_links = pr_page.clone();
    let link_handler = Arc::new(move |url: &str, window: &mut Window, cx: &mut App| {
      let handled = pr_page_for_links.update(cx, |this, cx| this.handle_gfm_link(url, window, cx));
      if handled {
        LinkAction::Handled
      } else {
        LinkAction::Open
      }
    });

    let markdown_options = MarkdownRenderOptions::with_on_link(link_handler.clone())
      .with_state(self.description_markdown_state.clone())
      .with_syntax_cache(self.syntax_highlight_cache.clone())
      .with_asset_url_resolver(github_shared::make_asset_url_resolver(&self.api))
      .with_github_issue_reference_context(pr_owner.as_ref(), pr_repo.as_ref())
      .with_scope_id(scope_id)
      .with_hardbreaks();

    // Determine root comment target and editability
    let root_target = match item.kind {
      GithubPrOverviewConversationItemKind::IssueComment => Some(OverviewCommentTarget {
        kind: OverviewCommentKind::Issue,
        id: item.id,
      }),
      GithubPrOverviewConversationItemKind::ReviewComment => Some(OverviewCommentTarget {
        kind: OverviewCommentKind::Review,
        id: item.id,
      }),
      GithubPrOverviewConversationItemKind::Review => None,
    };
    let root_is_editable = root_target.is_some_and(|target| match target.kind {
      OverviewCommentKind::Issue => editable_issue_comment_ids.contains(&target.id),
      OverviewCommentKind::Review => editable_review_comment_ids.contains(&target.id),
    });
    let root_is_editing = overview_root_is_editing(editing_target, root_target);
    let root_is_last_review_message =
      allows_overview_review_reply_action(item.kind, &item.thread_comment_ids, item.id);

    let review_comment_preview = if item.kind == GithubPrOverviewConversationItemKind::ReviewComment
    {
      self.overview_root_review_comment_preview(item.id)
    } else {
      None
    };

    // Root edit button
    let root_edit_button = if root_is_editable && !overview_submission_in_flight {
      root_target.map(|target| {
        let page = pr_page.clone();
        div()
          .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
          .child(
            Button::new(format!("pr-overview-comment-edit-{}", target.id))
              .ghost()
              .xsmall()
              .compact()
              .icon(UiIconName::SquarePen)
              .tooltip("Edit comment")
              .on_click(move |_, window, cx| {
                cx.stop_propagation();
                page.update(cx, |this, cx| {
                  this.start_overview_comment_edit(target, window, cx);
                });
              }),
          )
          .into_any_element()
      })
    } else {
      None
    };

    // Root delete button
    let root_delete_button = if root_is_editable && !overview_submission_in_flight {
      root_target.map(|target| {
        let page = pr_page.clone();
        div()
          .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
          .child(
            Button::new(format!("pr-overview-comment-delete-{}", target.id))
              .ghost()
              .xsmall()
              .compact()
              .icon(IconName::Delete)
              .tooltip("Delete comment")
              .on_click(move |_, window, cx| {
                cx.stop_propagation();
                page.update(cx, |this, cx| {
                  this.confirm_overview_comment_delete(target, window, cx);
                });
              }),
          )
          .into_any_element()
      })
    } else {
      None
    };

    // Root reply button
    let root_reply_button =
      if root_is_last_review_message && replying_target.is_none() && !overview_submission_in_flight
      {
        let page = pr_page.clone();
        let item_id = item.id;
        Some(
          div()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
              Button::new(format!("pr-overview-comment-reply-{}", item_id))
                .ghost()
                .xsmall()
                .compact()
                .icon(UiIconName::MessageCircleReply)
                .tooltip("Reply")
                .on_click(move |_, window, cx| {
                  cx.stop_propagation();
                  page.update(cx, |this, cx| {
                    this.start_overview_review_comment_reply(item_id, window, cx);
                  });
                }),
            )
            .into_any_element(),
        )
      } else {
        None
      };

    // Root body (edit mode or markdown)
    let root_body = if root_is_editing {
      if let Some(input_state) = self.overview_edit_input.clone() {
        let can_save = self
          .overview_edit_initial_body
          .as_deref()
          .and_then(|initial| {
            let raw_value = input_state.read(cx).value().to_string();
            next_overview_comment_body(raw_value.as_str(), initial)
          })
          .is_some();
        let page_for_cancel = pr_page.clone();
        let page_for_save = pr_page.clone();
        v_flex()
          .gap_2()
          .child(
            div().w_full().child(
              Input::new(&input_state)
                .disabled(self.overview_edit_submitting)
                .h(px(OVERVIEW_COMMENT_INPUT_HEIGHT_PX)),
            ),
          )
          .when_some(self.overview_edit_error.clone(), |this, error| {
            this.child(div().text_xs().text_color(theme.status_red()).child(error))
          })
          .child(
            h_flex()
              .items_center()
              .justify_end()
              .gap_2()
              .child(
                Button::new(format!("pr-overview-edit-cancel-{}", item.id))
                  .ghost()
                  .xsmall()
                  .compact()
                  .label("Cancel")
                  .on_click(move |_, _, cx| {
                    page_for_cancel.update(cx, |this, cx| {
                      this.cancel_overview_comment_edit(cx);
                    });
                  }),
              )
              .child(
                Button::new(format!("pr-overview-edit-save-{}", item.id))
                  .xsmall()
                  .compact()
                  .label("Save")
                  .disabled(!can_save || overview_submission_in_flight)
                  .on_click(move |_, _, cx| {
                    page_for_save.update(cx, |this, cx| {
                      this.submit_overview_comment_edit(cx);
                    });
                  }),
              ),
          )
          .into_any_element()
      } else {
        div().into_any_element()
      }
    } else if let Some(body) = &item.body {
      div()
        .w_full()
        .min_w_0()
        .child(render_markdown(body.as_str(), &markdown_options, cx))
        .into_any_element()
    } else {
      div()
        .w_full()
        .min_w_0()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("No comment body.")
        .into_any_element()
    };

    // Reply composer for root
    let root_reply_composer = if replying_target == Some(item.id) {
      if self.overview_reply_submitting {
        Some(
          v_flex()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(theme.border)
            .child(Spinner::new().small())
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Replying..."),
            )
            .into_any_element(),
        )
      } else if let Some(input_state) = self.overview_reply_input.clone() {
        let can_save =
          github_shared::normalize_non_empty_text(input_state.read(cx).value().as_str()).is_some();
        let page_for_cancel = pr_page.clone();
        let page_for_save = pr_page.clone();
        Some(
          v_flex()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(theme.border)
            .child(
              div()
                .w_full()
                .child(Input::new(&input_state).h(px(OVERVIEW_COMMENT_INPUT_HEIGHT_PX))),
            )
            .when_some(self.overview_reply_error.clone(), |this, error| {
              this.child(div().text_xs().text_color(theme.status_red()).child(error))
            })
            .child(
              h_flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(
                  Button::new(format!("pr-overview-reply-cancel-{}", item.id))
                    .ghost()
                    .xsmall()
                    .compact()
                    .label("Cancel")
                    .on_click(move |_, _, cx| {
                      page_for_cancel.update(cx, |this, cx| {
                        this.cancel_overview_review_comment_reply(cx);
                      });
                    }),
                )
                .child(
                  Button::new(format!("pr-overview-reply-save-{}", item.id))
                    .xsmall()
                    .compact()
                    .label("Save")
                    .disabled(!can_save || overview_submission_in_flight)
                    .on_click(move |_, _, cx| {
                      page_for_save.update(cx, |this, cx| {
                        this.submit_overview_review_comment_reply(cx);
                      });
                    }),
                ),
            )
            .into_any_element(),
        )
      } else {
        None
      }
    } else {
      None
    };

    let replies = &item.replies;
    let thread_comment_ids = item.thread_comment_ids.clone();

    v_flex()
      .id(format!(
        "pr-overview-conversation-{}-{}",
        conversation_source_priority(item.kind),
        item.id
      ))
      .border_1()
      .border_color(match item.review_state {
        Some(GithubPullRequestReviewState::Approved) => theme.status_green(),
        Some(GithubPullRequestReviewState::RequestChanges) => theme.status_red(),
        _ => theme.border,
      })
      .rounded(theme.radius)
      .when_some(review_comment_preview, |this, preview| {
        this.child(
          render_github_code_reference_preview_card(&preview, cx)
            .my_0()
            .border_b_1()
            .border_t_0()
            .border_x_0()
            .rounded_none(),
        )
      })
      .child(
        v_flex()
          .gap_2()
          .p_3()
          .when_some(item.review_state, |this, state| {
            let label = review_state_display_label(state);
            let icon_style = review_state_icon_style(state, &theme);
            this.child(
              h_flex()
                .items_center()
                .gap_1()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .when_some(icon_style, |this, (icon, color)| {
                  this.child(Icon::new(icon).size_4().text_color(color))
                })
                .child(label),
            )
          })
          .child(
            h_flex()
              .items_center()
              .justify_between()
              .gap_2()
              .child(
                h_flex()
                  .items_center()
                  .gap_2()
                  .flex_wrap()
                  .child(
                    Avatar::new()
                      .name(item.author_login.clone())
                      .when_some(item.author_avatar_url.clone(), |this, url| this.src(url))
                      .small(),
                  )
                  .child(
                    div()
                      .text_sm()
                      .text_color(theme.foreground)
                      .child(item.author_login.clone()),
                  )
                  .child(
                    div()
                      .text_xs()
                      .text_color(theme.muted_foreground)
                      .child(timestamp),
                  )
                  .when(
                    item.kind == GithubPrOverviewConversationItemKind::ReviewComment,
                    |this| {
                      this.child(
                        Icon::new(UiIconName::ScanEye)
                          .size_3()
                          .text_color(theme.muted_foreground),
                      )
                    },
                  ),
              )
              .child(
                h_flex()
                  .items_center()
                  .gap_1()
                  .when_some(root_edit_button, |this, button| this.child(button))
                  .when_some(root_delete_button, |this, button| this.child(button))
                  .when_some(root_reply_button, |this, button| this.child(button)),
              ),
          )
          .child(root_body)
          .when_some(root_reply_composer, |this, composer| this.child(composer))
          .when(!replies.is_empty(), |this| {
            this.child(
              v_flex()
                .gap_2()
                .pl_3()
                .border_l_1()
                .border_color(theme.border)
                .children(replies.iter().map(|reply| {
                  let reply_timestamp = format_relative_time(&reply.timestamp);
                  let reply_scope_id = scope_id
                    .wrapping_mul(1_000_003)
                    .wrapping_add(reply.id as usize);
                  let reply_markdown_options =
                    MarkdownRenderOptions::with_on_link(link_handler.clone())
                      .with_state(self.description_markdown_state.clone())
                      .with_syntax_cache(self.syntax_highlight_cache.clone())
                      .with_asset_url_resolver(github_shared::make_asset_url_resolver(&self.api))
                      .with_github_issue_reference_context(pr_owner.as_ref(), pr_repo.as_ref())
                      .with_scope_id(reply_scope_id)
                      .with_hardbreaks();

                  let reply_target = OverviewCommentTarget {
                    kind: OverviewCommentKind::Review,
                    id: reply.id,
                  };
                  let reply_is_editable = editable_review_comment_ids.contains(&reply.id);
                  let reply_is_last_message =
                    allows_overview_review_reply_action(item.kind, &thread_comment_ids, reply.id);

                  // Reply action buttons
                  let reply_edit_button = if reply_is_editable && !overview_submission_in_flight {
                    let page = pr_page.clone();
                    Some(
                      div()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                          Button::new(format!("pr-overview-reply-edit-{}", reply.id))
                            .ghost()
                            .xsmall()
                            .compact()
                            .icon(UiIconName::SquarePen)
                            .tooltip("Edit comment")
                            .on_click(move |_, window, cx| {
                              cx.stop_propagation();
                              page.update(cx, |this, cx| {
                                this.start_overview_comment_edit(reply_target, window, cx);
                              });
                            }),
                        )
                        .into_any_element(),
                    )
                  } else {
                    None
                  };

                  let reply_delete_button = if reply_is_editable && !overview_submission_in_flight {
                    let page = pr_page.clone();
                    Some(
                      div()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                          Button::new(format!("pr-overview-reply-delete-{}", reply.id))
                            .ghost()
                            .xsmall()
                            .compact()
                            .icon(IconName::Delete)
                            .tooltip("Delete comment")
                            .on_click(move |_, window, cx| {
                              cx.stop_propagation();
                              page.update(cx, |this, cx| {
                                this.confirm_overview_comment_delete(reply_target, window, cx);
                              });
                            }),
                        )
                        .into_any_element(),
                    )
                  } else {
                    None
                  };

                  let reply_reply_button = if reply_is_last_message
                    && replying_target.is_none()
                    && !overview_submission_in_flight
                  {
                    let page = pr_page.clone();
                    let reply_id = reply.id;
                    Some(
                      div()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                          Button::new(format!("pr-overview-reply-action-{}", reply_id))
                            .ghost()
                            .xsmall()
                            .compact()
                            .icon(UiIconName::MessageCircleReply)
                            .tooltip("Reply")
                            .on_click(move |_, window, cx| {
                              cx.stop_propagation();
                              page.update(cx, |this, cx| {
                                this.start_overview_review_comment_reply(reply_id, window, cx);
                              });
                            }),
                        )
                        .into_any_element(),
                    )
                  } else {
                    None
                  };

                  // Reply body (edit mode or markdown)
                  let reply_is_editing = editing_target == Some(reply_target);
                  let reply_body =
                    if reply_is_editing {
                      if let Some(input_state) = self.overview_edit_input.clone() {
                        let can_save = self
                          .overview_edit_initial_body
                          .as_deref()
                          .and_then(|initial| {
                            let raw_value = input_state.read(cx).value().to_string();
                            next_overview_comment_body(raw_value.as_str(), initial)
                          })
                          .is_some();
                        let page_for_cancel = pr_page.clone();
                        let page_for_save = pr_page.clone();
                        v_flex()
                          .gap_2()
                          .child(div().w_full().child(
                            Input::new(&input_state).h(px(OVERVIEW_COMMENT_INPUT_HEIGHT_PX)),
                          ))
                          .when_some(self.overview_edit_error.clone(), |this, error| {
                            this.child(div().text_xs().text_color(theme.status_red()).child(error))
                          })
                          .child(
                            h_flex()
                              .items_center()
                              .justify_end()
                              .gap_2()
                              .child(
                                Button::new(format!("pr-overview-reply-edit-cancel-{}", reply.id))
                                  .ghost()
                                  .xsmall()
                                  .compact()
                                  .label("Cancel")
                                  .on_click(move |_, _, cx| {
                                    page_for_cancel.update(cx, |this, cx| {
                                      this.cancel_overview_comment_edit(cx);
                                    });
                                  }),
                              )
                              .child(
                                Button::new(format!("pr-overview-reply-edit-save-{}", reply.id))
                                  .xsmall()
                                  .compact()
                                  .label("Save")
                                  .disabled(!can_save || overview_submission_in_flight)
                                  .on_click(move |_, _, cx| {
                                    page_for_save.update(cx, |this, cx| {
                                      this.submit_overview_comment_edit(cx);
                                    });
                                  }),
                              ),
                          )
                          .into_any_element()
                      } else {
                        div().into_any_element()
                      }
                    } else {
                      div()
                        .w_full()
                        .min_w_0()
                        .child(render_markdown(
                          reply.body.as_str(),
                          &reply_markdown_options,
                          cx,
                        ))
                        .into_any_element()
                    };

                  // Reply reply composer
                  let reply_reply_composer = if replying_target == Some(reply.id) {
                    if self.overview_reply_submitting {
                      Some(
                        v_flex()
                          .gap_2()
                          .pt_2()
                          .border_t_1()
                          .border_color(theme.border)
                          .child(Spinner::new().small())
                          .child(
                            div()
                              .text_xs()
                              .text_color(theme.muted_foreground)
                              .child("Replying..."),
                          )
                          .into_any_element(),
                      )
                    } else if let Some(input_state) = self.overview_reply_input.clone() {
                      let can_save = github_shared::normalize_non_empty_text(
                        input_state.read(cx).value().as_str(),
                      )
                      .is_some();
                      let page_for_cancel = pr_page.clone();
                      let page_for_save = pr_page.clone();
                      Some(
                        v_flex()
                          .gap_2()
                          .pt_2()
                          .border_t_1()
                          .border_color(theme.border)
                          .child(div().w_full().child(
                            Input::new(&input_state).h(px(OVERVIEW_COMMENT_INPUT_HEIGHT_PX)),
                          ))
                          .when_some(self.overview_reply_error.clone(), |this, error| {
                            this.child(div().text_xs().text_color(theme.status_red()).child(error))
                          })
                          .child(
                            h_flex()
                              .items_center()
                              .justify_end()
                              .gap_2()
                              .child(
                                Button::new(format!(
                                  "pr-overview-reply-composer-cancel-{}",
                                  reply.id
                                ))
                                .ghost()
                                .xsmall()
                                .compact()
                                .label("Cancel")
                                .on_click(move |_, _, cx| {
                                  page_for_cancel.update(cx, |this, cx| {
                                    this.cancel_overview_review_comment_reply(cx);
                                  });
                                }),
                              )
                              .child(
                                Button::new(format!(
                                  "pr-overview-reply-composer-save-{}",
                                  reply.id
                                ))
                                .xsmall()
                                .compact()
                                .label("Save")
                                .disabled(!can_save || overview_submission_in_flight)
                                .on_click(move |_, _, cx| {
                                  page_for_save.update(cx, |this, cx| {
                                    this.submit_overview_review_comment_reply(cx);
                                  });
                                }),
                              ),
                          )
                          .into_any_element(),
                      )
                    } else {
                      None
                    }
                  } else {
                    None
                  };

                  v_flex()
                    .id(format!(
                      "pr-overview-conversation-reply-{}-{}",
                      item.id, reply.id
                    ))
                    .gap_1()
                    .child(
                      h_flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                          h_flex()
                            .items_center()
                            .gap_2()
                            .flex_wrap()
                            .child(
                              Avatar::new()
                                .name(reply.author_login.clone())
                                .when_some(reply.author_avatar_url.clone(), |this, url| {
                                  this.src(url)
                                })
                                .small(),
                            )
                            .child(
                              div()
                                .text_sm()
                                .text_color(theme.foreground)
                                .child(reply.author_login.clone()),
                            )
                            .child(
                              div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(reply_timestamp),
                            ),
                        )
                        .child(
                          h_flex()
                            .items_center()
                            .gap_1()
                            .when_some(reply_edit_button, |this, button| this.child(button))
                            .when_some(reply_delete_button, |this, button| this.child(button))
                            .when_some(reply_reply_button, |this, button| this.child(button)),
                        ),
                    )
                    .child(reply_body)
                    .when_some(reply_reply_composer, |this, composer| this.child(composer))
                    .into_any_element()
                })),
            )
          }),
      )
      .into_any_element()
  }

  fn render_details(
    &mut self,
    pr: &GithubPullRequestDetails,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let repo_label = github_shared::repo_label(&pr.repository.owner, &pr.repository.repo);
    let pr_url = github_shared::pr_url(&pr.repository.owner, &pr.repository.repo, pr.number);
    let repo_owner = pr.repository.owner.clone();
    let repo_name = pr.repository.repo.clone();
    let updated_at = format_relative_time(&pr.updated_at);
    let created_at = format_relative_time(&pr.created_at);
    let merged_at = pr.merged_at.as_deref().map(format_relative_time);

    let body = pr
      .body
      .clone()
      .filter(|value| !value.trim().is_empty())
      .unwrap_or_else(|| "No description provided.".to_string());

    let labels_row = if pr.labels.is_empty() {
      None
    } else {
      Some(
        h_flex()
          .debug_selector(|| "github-pr-overview-labels".to_string())
          .gap_1()
          .flex_wrap()
          .children(
            pr.labels
              .iter()
              .map(|label| github_shared::github_label_tag(label, &theme)),
          ),
      )
    };

    let pr_page = cx.entity().clone();
    let pr_page_for_links = pr_page.clone();
    let description_link_handler = Arc::new(move |url: &str, window: &mut Window, cx: &mut App| {
      let handled = pr_page_for_links.update(cx, |this, cx| this.handle_gfm_link(url, window, cx));
      if handled {
        LinkAction::Handled
      } else {
        LinkAction::Open
      }
    });
    let description_previews = self.cached_github_code_reference_previews_for_requests(
      &self.description_code_reference_requests,
    );
    let overview_pr_alert = self.render_overview_pr_alert(cx);

    let content = v_flex()
      .w_full()
      .gap_4()
      .child(
        h_flex()
          .gap_2()
          .items_center()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(
                Avatar::new()
                  .name(pr.author.login.clone())
                  .when_some(pr.author.avatar_url.clone(), |this, url| this.src(url))
                  .small(),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.foreground)
                  .child(pr.author.login.clone()),
              ),
          )
          .child(
            Button::new("open-pr-repo-details")
              .ghost()
              .small()
              .compact()
              .label(repo_label)
              .on_click(move |_, _, cx| {
                GithubRepoPageHandle::show(repo_owner.clone().into(), repo_name.clone().into(), cx);
              }),
          )
          .child(
            Button::new("open-pr-on-github")
              .icon(IconName::ExternalLink)
              .ghost()
              .small()
              .label("View on GitHub")
              .compact()
              .on_click({
                let pr_url = pr_url.clone();
                move |_, _, cx| {
                  cx.open_url(&pr_url);
                }
              }),
          ),
      )
      .child(
        h_flex()
          .gap_2()
          .flex_wrap()
          .items_center()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(
                div()
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .child("Created"),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.foreground)
                  .child(created_at),
              ),
          )
          .child(
            div()
              .debug_selector(|| "github-pr-overview-created-updated-separator".to_string())
              .text_sm()
              .text_color(theme.muted_foreground)
              .child("•"),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(
                div()
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .child("Updated"),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.foreground)
                  .child(updated_at),
              )
              .child(
                div()
                  .debug_selector(|| {
                    "github-pr-overview-updated-change-stats-separator".to_string()
                  })
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .child("•"),
              )
              .child(
                h_flex()
                  .debug_selector(|| "github-pr-overview-updated-change-stats".to_string())
                  .items_center()
                  .gap_2()
                  .children(overview_change_stats(pr, &theme)),
              ),
          )
          .when_some(merged_at.clone(), |this, merged| {
            this.child(
              h_flex()
                .items_center()
                .gap_2()
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Merged"),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(merged.to_string()),
                ),
            )
          }),
      )
      .child(
        h_flex().items_center().gap_2().child(
          h_flex()
            .items_center()
            .gap_2()
            .child(
              h_flex()
                .items_center()
                .gap_1()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Source"),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(pr.head_ref_name.clone()),
                )
                .child(Clipboard::new("copy-pr-branch-source").value(pr.head_ref_name.clone())),
            )
            .child(
              Icon::new(IconName::ArrowRight)
                .size_3()
                .text_color(theme.muted_foreground),
            )
            .child(
              h_flex()
                .items_center()
                .gap_1()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Target"),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(pr.base_ref_name.clone()),
                )
                .child(Clipboard::new("copy-pr-branch-target").value(pr.base_ref_name.clone())),
            ),
        ),
      )
      .when_some(labels_row, |this, labels| this.child(labels))
      .when_some(overview_pr_alert, |this, alert| this.child(alert))
      .child(
        v_flex()
          .gap_2()
          .child(
            h_flex()
              .items_center()
              .justify_between()
              .gap_2()
              .child(
                div()
                  .text_sm()
                  .font_medium()
                  .text_color(theme.foreground)
                  .child("Description"),
              )
              .child(
                Button::new(format!("pr-description-edit-{}", pr.number))
                  .ghost()
                  .xsmall()
                  .compact()
                  .icon(UiIconName::SquarePen)
                  .tooltip("Edit description")
                  .disabled(
                    self.pr_description_editing || self.overview_comment_submission_in_flight(),
                  )
                  .on_click({
                    let page = pr_page.clone();
                    move |_, window, cx| {
                      page.update(cx, |this, cx| {
                        this.start_pr_description_edit(window, cx);
                      });
                    }
                  }),
              ),
          )
          .child(if self.pr_description_editing {
            if let Some(input_state) = self.pr_description_edit_input.clone() {
              let can_save = self
                .pr_description_initial_body
                .as_deref()
                .and_then(|initial| {
                  let raw_value = input_state.read(cx).value().to_string();
                  next_pr_description_body(raw_value.as_str(), initial)
                })
                .is_some();
              let page_for_cancel = pr_page.clone();
              let page_for_save = pr_page.clone();
              v_flex()
                .gap_2()
                .child(
                  div().w_full().child(
                    Input::new(&input_state)
                      .disabled(self.pr_description_submitting)
                      .h(px(OVERVIEW_DESCRIPTION_INPUT_HEIGHT_PX)),
                  ),
                )
                .when_some(self.pr_description_error.clone(), |this, error| {
                  this.child(div().text_xs().text_color(theme.status_red()).child(error))
                })
                .child(
                  h_flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(
                      Button::new(format!("pr-description-edit-cancel-{}", pr.number))
                        .ghost()
                        .xsmall()
                        .compact()
                        .label("Cancel")
                        .disabled(self.pr_description_submitting)
                        .on_click(move |_, _, cx| {
                          page_for_cancel.update(cx, |this, cx| {
                            this.cancel_pr_description_edit(cx);
                          });
                        }),
                    )
                    .child(
                      Button::new(format!("pr-description-edit-save-{}", pr.number))
                        .xsmall()
                        .compact()
                        .label("Save")
                        .disabled(!can_save || self.overview_comment_submission_in_flight())
                        .on_click(move |_, _, cx| {
                          page_for_save.update(cx, |this, cx| {
                            this.submit_pr_description_edit(cx);
                          });
                        }),
                    ),
                )
                .into_any_element()
            } else {
              div().into_any_element()
            }
          } else {
            div()
              .border_1()
              .border_color(theme.border)
              .rounded(theme.radius)
              .p_3()
              .child({
                let mut options = MarkdownRenderOptions::with_on_link(description_link_handler)
                  .with_state(self.description_markdown_state.clone())
                  .with_syntax_cache(self.syntax_highlight_cache.clone())
                  .with_asset_url_resolver(github_shared::make_asset_url_resolver(&self.api))
                  .with_github_issue_reference_context(
                    pr.repository.owner.as_str(),
                    pr.repository.repo.as_str(),
                  )
                  .with_scope_id(pr_description_scope_id(pr.number))
                  .with_hardbreaks();
                if let Some(previews) = description_previews.clone() {
                  options = options.with_github_code_reference_previews(previews);
                }
                render_markdown(body.as_str(), &options, cx)
              })
              .into_any_element()
          }),
      );

    // Build conversation items and update list state
    let conversation_items =
      build_overview_conversation_items(&self.issue_comments, &self.reviews, &self.review_comments);
    self.overview_conversation_items = conversation_items;

    // Also include conversation header in item 0
    let is_conv_loading =
      self.issue_comments_loading || self.reviews_loading || self.review_comments_loading;
    let has_conv_errors = self.issue_comments_error.is_some()
      || self.reviews_error.is_some()
      || self.review_comments_error.is_some();
    let conv_errors = v_flex()
      .gap_1()
      .when_some(self.issue_comments_error.clone(), |this, error| {
        this.child(
          div()
            .text_xs()
            .text_color(theme.status_red())
            .child(format!("Issue comments: {error}")),
        )
      })
      .when_some(self.reviews_error.clone(), |this, error| {
        this.child(
          div()
            .text_xs()
            .text_color(theme.status_red())
            .child(format!("Reviews: {error}")),
        )
      })
      .when_some(self.review_comments_error.clone(), |this, error| {
        this.child(
          div()
            .text_xs()
            .text_color(theme.status_red())
            .child(format!("Review comments: {error}")),
        )
      });

    let header_el = content
      .child(
        v_flex()
          .w_full()
          .gap_2()
          .child(
            div()
              .text_sm()
              .font_medium()
              .text_color(theme.foreground)
              .child("Conversation"),
          )
          .when(has_conv_errors, |this| this.child(conv_errors))
          .when(
            self.overview_conversation_items.is_empty() && is_conv_loading,
            |this| {
              this.child(
                v_flex()
                  .w_full()
                  .items_center()
                  .justify_center()
                  .gap_2()
                  .py_6()
                  .child(Spinner::new().small())
                  .child(
                    div()
                      .text_sm()
                      .text_color(theme.muted_foreground)
                      .child("Loading conversation..."),
                  ),
              )
            },
          )
          .when(
            self.overview_conversation_items.is_empty() && !is_conv_loading,
            |this| {
              this.child(
                v_flex()
                  .w_full()
                  .items_center()
                  .justify_center()
                  .py_6()
                  .child(
                    div()
                      .text_sm()
                      .text_color(theme.muted_foreground)
                      .child("No comments yet"),
                  ),
              )
            },
          ),
      )
      .into_any_element();

    // List layout: item 0 = header/description, items 1..N = comments, item N+1 = add comment
    let conversation_count = self.overview_conversation_items.len();
    let total_items = 1 + conversation_count + 1;
    // Only reset when item count changes to preserve cached heights
    if self.overview_list_count != total_items {
      self.overview_list_count = total_items;
      self.overview_list.reset(total_items);
    }

    let pr_number = pr.number;
    let pr_owner: SharedString = pr.repository.owner.clone().into();
    let pr_repo: SharedString = pr.repository.repo.clone().into();
    let entity = cx.entity().clone();
    let max_w = px(1100.);
    let header = std::cell::RefCell::new(Some(header_el));

    list(self.overview_list.clone(), move |ix, _window, cx| {
      entity.update(cx, |this, cx| {
        let theme = cx.theme().clone();
        let el = if ix == 0 {
          // Header + description + conversation header
          header
            .borrow_mut()
            .take()
            .unwrap_or_else(|| div().into_any_element())
        } else if ix <= conversation_count {
          // Conversation item
          this.render_overview_conversation_item(
            ix - 1,
            pr_number,
            pr_owner.clone(),
            pr_repo.clone(),
            cx,
          )
        } else {
          // Add comment section
          this.render_overview_add_comment_section(&theme, cx)
        };

        div()
          .px_10()
          .pb_4()
          .child(
            div()
              .mx_auto()
              .max_w(max_w)
              .pt(if ix == 0 { px(40.) } else { px(0.) })
              .child(el),
          )
          .into_any_element()
      })
    })
    .size_full()
  }

  fn render_files_sidebar(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let local_project_mode = self.local_project_mode_active(cx);
    let local_project_availability = self.local_project_availability(cx);
    let count = self.active_file_count(cx);
    let commit_options =
      Self::build_commit_dropdown_items(&self.commits, self.selected_commit_sha.as_deref());
    let on_commit_select = self.commit_select_handler(cx);
    let tree_search_active = self.tree_search_query_normalized().is_some();

    if self.tree_search_reset_pending {
      let tree_search_input = self.tree_search_input.clone();
      cx.on_next_frame(window, move |this, window, cx| {
        if this.tree_search_reset_pending {
          tree_search_input.update(cx, |input, cx| input.set_value("", window, cx));
          this.tree_search_reset_pending = false;
          cx.notify();
        }
      });
    }

    if let Some(selected_id) = self
      .tree_state
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string())
      && Some(selected_id.as_str()) != self.current_selected_tree_path().as_deref()
    {
      cx.on_next_frame(window, move |this, _, cx| {
        this.select_visible_tree_path(selected_id.as_str(), cx);
      });
    }

    let header = h_flex()
      .pl_3()
      .items_center()
      .justify_between()
      .h(px(DIFF_HEADER_HEIGHT))
      .border_b_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(div().text_sm().text_color(theme.foreground).child("Files"))
          .child(
            Tag::secondary()
              .small()
              .rounded_full()
              .child(count.to_string()),
          ),
      )
      .child(
        div()
          .border_l_1()
          .border_color(theme.border)
          .child(dropdown_select(
            DropdownSelectConfig::new("github-pr-commit-select")
              .placeholder("All changes")
              .search_placeholder("Search commits...")
              .options(commit_options)
              .width(px(PR_COMMIT_SELECT_WIDTH))
              .menu_width(px(PR_COMMIT_SELECT_MENU_WIDTH))
              .trigger_height(px(DIFF_HEADER_HEIGHT - 1.))
              .disabled(self.commits_loading || self.commits_error.is_some())
              .on_select(on_commit_select),
          )),
      );

    let local_project_controls = if matches!(
      local_project_availability,
      GithubPrLocalProjectAvailability::Hidden
    ) {
      None
    } else {
      let (status_text, status_color): (Option<String>, Hsla) = match &local_project_availability {
        GithubPrLocalProjectAvailability::NeedsBranchSwitch {
          current_branch: Some(current_branch),
          has_uncommitted_changes,
          ..
        } => (
          if *has_uncommitted_changes {
            Some(format!(
              "Local changes detected on {}. Stash before switching to this PR branch.",
              current_branch
            ))
          } else {
            Some(format!(
              "Current branch is {}. Switch to this PR branch to browse unchanged files.",
              current_branch
            ))
          },
          theme.status_orange(),
        ),
        GithubPrLocalProjectAvailability::NeedsBranchSwitch {
          current_branch: None,
          has_uncommitted_changes,
          ..
        } => (
          if *has_uncommitted_changes {
            Some("Local changes detected. Stash before switching to this PR branch.".to_string())
          } else {
            Some("Local repo is not on this PR branch.".to_string())
          },
          theme.status_orange(),
        ),
        GithubPrLocalProjectAvailability::Ready { .. } => (None, theme.muted_foreground),
        GithubPrLocalProjectAvailability::NeedsUpdate { .. } => (
          Some("Local branch is not at this PR head".to_string()),
          theme.status_orange(),
        ),
        GithubPrLocalProjectAvailability::Dirty { .. } => (
          Some("Local branch is not at this PR head and has local changes".to_string()),
          theme.status_orange(),
        ),
        GithubPrLocalProjectAvailability::Hidden => (None, theme.muted_foreground),
      };
      let can_toggle_local_project = matches!(
        local_project_availability,
        GithubPrLocalProjectAvailability::Ready { .. }
      );
      let view = cx.entity();
      let switch_branch_button = if matches!(
        local_project_availability,
        GithubPrLocalProjectAvailability::NeedsBranchSwitch { .. }
      ) {
        Some(
          Button::new("github-pr-local-project-switch-branch")
            .label(if self.local_branch_switch_loading {
              "Switching..."
            } else {
              "Switch to PR branch"
            })
            .xsmall()
            .ghost()
            .disabled(self.local_branch_switch_loading || self.local_project_update_loading)
            .on_click(move |_, window, cx| {
              view.update(cx, |this, cx| {
                this.prompt_or_switch_local_branch_to_pr_branch(window, cx);
              });
            }),
        )
      } else {
        None
      };
      let view = cx.entity();
      let update_button = if matches!(
        local_project_availability,
        GithubPrLocalProjectAvailability::NeedsUpdate { .. }
      ) {
        Some(
          Button::new("github-pr-local-project-update")
            .label(if self.local_project_update_loading {
              "Updating..."
            } else {
              "Update to PR head"
            })
            .xsmall()
            .ghost()
            .disabled(self.local_project_update_loading)
            .on_click(move |_, window, cx| {
              view.update(cx, |this, cx| {
                this.update_local_branch_to_pr_head(false, None, window, cx);
              });
            }),
        )
      } else {
        None
      };

      Some(
        v_flex()
          .gap_1()
          .justify_center()
          .min_h(px(DIFF_HEADER_HEIGHT))
          .px_3()
          .py_2()
          .border_b_1()
          .border_color(theme.border)
          .child(
            h_flex()
              .items_center()
              .justify_between()
              .gap_2()
              .child(
                Switch::new("github-pr-local-project-switch")
                  .label("Show unchanged files")
                  .small()
                  .checked(local_project_mode)
                  .disabled(
                    !can_toggle_local_project
                      || self.local_project_update_loading
                      || self.local_branch_switch_loading,
                  )
                  .on_click(cx.listener(move |this, checked, _, cx| {
                    this.set_show_local_project_files(*checked, cx);
                  })),
              )
              .when_some(switch_branch_button, |this, button| this.child(button))
              .when_some(update_button, |this, button| this.child(button)),
          )
          .when_some(status_text, |this, status_text| {
            this.child(div().text_xs().text_color(status_color).child(status_text))
          })
          .when_some(self.local_branch_switch_error.clone(), |this, error| {
            this.child(div().text_xs().text_color(theme.status_red()).child(error))
          })
          .when_some(self.local_project_update_error.clone(), |this, error| {
            this.child(div().text_xs().text_color(theme.status_red()).child(error))
          }),
      )
    };

    let search_controls = {
      v_flex()
        .gap_1()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(theme.border)
        .child(
          div()
            .relative()
            .child(Input::new(&self.tree_search_input).w_full().pr(px(28.0)))
            .when(tree_search_active && self.tree_search_loading, |this| {
              this.child(
                h_flex()
                  .absolute()
                  .top_0()
                  .right_2()
                  .bottom_0()
                  .items_center()
                  .child(Spinner::new().small()),
              )
            }),
        )
        .when_some(self.tree_search_error.clone(), |this, message| {
          this.child(
            div()
              .text_xs()
              .text_color(theme.status_red())
              .child(message),
          )
        })
    };

    let comment_counts = if self.selected_commit_sha.is_none() && !self.review_comments.is_empty() {
      visible_review_comment_counts_by_path(&self.file_lookup, &self.review_comments)
    } else {
      HashMap::new()
    };

    let list = if local_project_mode && self.local_project_tree_loading {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading project files..."),
        )
        .into_any_element()
    } else if local_project_mode && self.local_project_tree_error.is_some() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.local_project_tree_error.clone().unwrap_or_default())
        .into_any_element()
    } else if !local_project_mode && self.files_loading {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading files..."),
        )
        .into_any_element()
    } else if !local_project_mode && self.files_error.is_some() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.files_error.clone().unwrap_or_default())
        .into_any_element()
    } else if count == 0 {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(if tree_search_active {
          "No matching files"
        } else if local_project_mode {
          "No project files found"
        } else {
          "No files changed"
        })
        .into_any_element()
    } else {
      let view = cx.entity();
      tree(&self.tree_state, move |ix, entry, selected, _window, cx| {
        view.update(cx, |this, cx| {
          let theme = cx.theme().clone();
          let item = entry.item();
          let is_folder = entry.is_folder();
          let status = if is_folder {
            None
          } else {
            this
              .file_lookup
              .get(item.id.as_ref())
              .map(|file| file.status)
          };
          let status_letter = status.map(status_letter).unwrap_or("");
          let status_color = status
            .map(|status| status_color(status, &theme))
            .unwrap_or(theme.muted_foreground);
          let comment_count = if is_folder {
            0
          } else {
            comment_counts.get(item.id.as_ref()).copied().unwrap_or(0)
          };
          let icon = if is_folder {
            if entry.is_expanded() {
              Icon::new(IconName::FolderOpen)
            } else {
              Icon::new(IconName::Folder)
            }
            .size_3()
            .text_color(theme.muted_foreground)
            .into_any_element()
          } else {
            file_icon_path_for_name_with_theme(item.label.as_ref(), &theme)
              .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
              .unwrap_or_else(|| {
                Icon::new(IconName::File)
                  .size_3()
                  .text_color(theme.muted_foreground)
                  .into_any_element()
              })
          };

          let indent = px(12.) + px(15.) * entry.depth();
          let mut row = selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
            .w_full()
            .px_2()
            .pl(indent)
            .child(
              h_flex()
                .items_center()
                .gap_2()
                .when(!is_folder, |this| {
                  this.child(
                    div()
                      .w(px(15.))
                      .text_xs()
                      .text_color(status_color)
                      .child(status_letter),
                  )
                })
                .child(icon)
                .child(
                  div()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(item.label.clone()),
                )
                .when(comment_count > 0, |this| {
                  this.child(
                    h_flex()
                      .items_center()
                      .gap_1()
                      .text_xs()
                      .pr_2()
                      .text_color(theme.muted_foreground)
                      .child(
                        Icon::new(UiIconName::MessageCircle)
                          .size_3()
                          .text_color(theme.muted_foreground),
                      )
                      .child(comment_count.to_string()),
                  )
                }),
            );

          if !is_folder {
            let id = item.id.clone();
            row = row.on_click(cx.listener(move |this, _, _, cx| {
              this.select_visible_tree_path(id.as_ref(), cx);
            }));
          }

          row
        })
      })
      .flex_1()
      .w_full()
      .into_any_element()
    };

    v_flex()
      .bg(theme.sidebar)
      .size_full()
      .child(header)
      .child(search_controls)
      .when_some(local_project_controls, |this, controls| {
        this.child(controls)
      })
      .child(
        div()
          .px_1()
          .flex_1()
          .min_h_0()
          .key_context(crate::shortcuts::GITHUB_PR_CHANGES_TREE_CONTEXT)
          .child(list),
      )
  }

  fn render_left_sidebar(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    match left_sidebar_kind_for_tab(self.active_tab_ix) {
      GithubPrLeftSidebarKind::Files => self.render_files_sidebar(window, cx).into_any_element(),
      GithubPrLeftSidebarKind::Context => self.render_context_sidebar(cx),
    }
  }

  fn render_diff_header(
    &self,
    file: &GithubPrFileDiff,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let path = Path::new(file.path.as_ref());
    let file_name = path
      .file_name()
      .and_then(|name| name.to_str())
      .unwrap_or(file.path.as_ref())
      .to_string();
    let dir_path = path
      .parent()
      .and_then(|parent| parent.to_str())
      .unwrap_or("")
      .to_string();
    let icon = file_icon_path_for_name_with_theme(&file_name, &theme)
      .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
      .unwrap_or_else(|| {
        Icon::new(IconName::File)
          .size_3()
          .text_color(theme.muted_foreground)
          .into_any_element()
      });

    let status_letter = status_letter(file.status);
    let status_color = status_color(file.status, &theme);
    let is_markdown = is_markdown_path(path);
    let is_svg = is_svg_path(path);
    let preview_active = (is_markdown || is_svg) && self.show_markdown_preview;
    let file_loading = self.file_loading;
    let split_disabled = self.split_disabled_for_selected_file() || preview_active;
    let has_review_comments = !self.selected_file_review_comment_ids.is_empty();
    let (toggle_label, toggle_icon) = if split_disabled {
      ("Split", IconName::PanelLeft)
    } else {
      match self.diff_view {
        DiffViewMode::Inline => ("Split", IconName::PanelLeft),
        DiffViewMode::Split => ("Inline", IconName::PanelLeftClose),
      }
    };
    let view = cx.entity();
    let previous_comment_button = Button::new("pr-review-comment-prev")
      .icon(IconName::ArrowUp)
      .xsmall()
      .ghost()
      .compact()
      .tooltip("Previous comment")
      .disabled(!has_review_comments || file_loading)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.navigate_review_comment(ReviewCommentNavigationDirection::Previous, cx);
        });
      });
    let view = cx.entity();
    let next_comment_button = Button::new("pr-review-comment-next")
      .icon(IconName::ArrowDown)
      .xsmall()
      .ghost()
      .compact()
      .tooltip("Next comment")
      .disabled(!has_review_comments || file_loading)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.navigate_review_comment(ReviewCommentNavigationDirection::Next, cx);
        });
      });
    let view = cx.entity();
    let toggle_button = Button::new("pr-diff-toggle")
      .label(toggle_label)
      .icon(toggle_icon)
      .xsmall()
      .ghost()
      .disabled(split_disabled || file_loading)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_diff_view(cx);
        });
      });
    let view = cx.entity();
    let preview_button = Button::new("pr-markdown-preview")
      .label("Preview")
      .icon(if preview_active {
        IconName::EyeOff
      } else {
        IconName::Eye
      })
      .xsmall()
      .ghost()
      .selected(preview_active)
      .disabled(file_loading)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_markdown_preview(cx);
        });
      });

    div()
      .h(px(DIFF_HEADER_HEIGHT))
      .bg(theme.sidebar)
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .border_b_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(
            div()
              .w(px(15.))
              .text_xs()
              .text_color(status_color)
              .child(status_letter),
          )
          .child(icon)
          .child({
            let mut label = Label::new(file_name);
            if !dir_path.is_empty() {
              label = label.secondary(format!("- {}", dir_path));
            }
            label.truncate()
          }),
      )
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(previous_comment_button)
          .child(next_comment_button)
          .child(toggle_button)
          .when(is_markdown || is_svg, |this| this.child(preview_button)),
      )
  }

  fn render_local_project_file_header(
    &self,
    file: &GithubPrLocalProjectFile,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let path = Path::new(file.path.as_ref());
    let file_name = path
      .file_name()
      .and_then(|name| name.to_str())
      .unwrap_or(file.path.as_ref())
      .to_string();
    let dir_path = path
      .parent()
      .and_then(|parent| parent.to_str())
      .unwrap_or("")
      .to_string();
    let icon = file_icon_path_for_name_with_theme(&file_name, &theme)
      .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
      .unwrap_or_else(|| {
        Icon::new(IconName::File)
          .size_3()
          .text_color(theme.muted_foreground)
          .into_any_element()
      });

    let is_markdown = is_markdown_path(path);
    let is_svg = is_svg_path(path);
    let preview_active = (is_markdown || is_svg) && self.show_markdown_preview;
    let view = cx.entity();
    let preview_button = Button::new("pr-local-project-preview")
      .label("Preview")
      .icon(if preview_active {
        IconName::EyeOff
      } else {
        IconName::Eye
      })
      .xsmall()
      .ghost()
      .selected(preview_active)
      .disabled(self.file_loading)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_markdown_preview(cx);
        });
      });

    div()
      .h(px(DIFF_HEADER_HEIGHT))
      .bg(theme.sidebar)
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .border_b_1()
      .border_color(theme.border)
      .child(h_flex().items_center().gap_2().child(icon).child({
        let mut label = Label::new(file_name);
        if !dir_path.is_empty() {
          label = label.secondary(format!("- {}", dir_path));
        }
        label.truncate()
      }))
      .when(is_markdown || is_svg, |this| {
        this.child(h_flex().items_center().gap_2().child(preview_button))
      })
  }

  fn render_selected_editor_content(
    &mut self,
    is_markdown: bool,
    is_svg: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    if let Some(binary_preview) = self.binary_preview.as_ref() {
      return self.render_binary_preview_content(binary_preview, cx);
    }

    let theme = cx.theme().clone();
    let preview_active = self.show_markdown_preview && (is_markdown || is_svg);

    if preview_active {
      let preview_panel = if is_svg {
        self.update_svg_preview(window, cx);
        let preview = match self.svg_preview.clone() {
          Some(Ok(image)) => img(image).max_w_full().max_h_full().into_any_element(),
          Some(Err(error)) => div()
            .text_sm()
            .text_color(theme.status_red())
            .child(error)
            .into_any_element(),
          None => div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Rendering SVG preview...")
            .into_any_element(),
        };
        div()
          .flex_1()
          .min_h_0()
          .min_w(px(0.0))
          .bg(theme.background)
          .child(
            div()
              .flex_1()
              .min_h_0()
              .min_w(px(0.0))
              .p_4()
              .items_center()
              .justify_center()
              .child(preview),
          )
          .into_any_element()
      } else {
        let markdown = self.diff_editor.read(cx).document().read(cx);
        let markdown = markdown.slice_to_string(0..markdown.len());
        div()
          .flex_1()
          .min_h_0()
          .min_w(px(0.0))
          .bg(theme.background)
          .child(
            div().size_full().pb_4().px_4().child(
              TextView::markdown("github-pr-markdown-preview-text", markdown)
                .size_full()
                .selectable(true)
                .scrollable(true),
            ),
          )
          .into_any_element()
      };
      return div()
        .flex_1()
        .min_h_0()
        .child(
          h_resizable("github-pr-markdown-preview")
            .child(
              resizable_panel().child(
                div()
                  .size_full()
                  .min_w(px(0.0))
                  .min_h_0()
                  .flex()
                  .flex_col()
                  .debug_selector(|| GITHUB_PR_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR.to_string())
                  .child(self.diff_editor.clone()),
              ),
            )
            .child(
              resizable_panel().child(
                div()
                  .size_full()
                  .min_w(px(0.0))
                  .min_h_0()
                  .flex()
                  .flex_col()
                  .debug_selector(|| GITHUB_PR_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR.to_string())
                  .child(preview_panel),
              ),
            ),
        )
        .into_any_element();
    }

    div()
      .flex_1()
      .min_h_0()
      .child(self.diff_editor.clone())
      .into_any_element()
  }

  fn show_file_search_action(
    &mut self,
    _: &ShowFileSearch,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.open_file_search_palette(window, cx);
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn switch_to_pr_branch_action(
    &mut self,
    _: &crate::SwitchToPrBranch,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.prompt_or_switch_local_branch_to_pr_branch(window, cx);
    cx.stop_propagation();
  }

  fn previous_annotation_action(
    &mut self,
    _: &crate::PreviousAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.navigate_review_comment(ReviewCommentNavigationDirection::Previous, cx);
    cx.stop_propagation();
  }

  fn next_annotation_action(
    &mut self,
    _: &crate::NextAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.navigate_review_comment(ReviewCommentNavigationDirection::Next, cx);
    cx.stop_propagation();
  }

  fn toggle_diff_view_action(
    &mut self,
    _: &crate::ToggleDiffView,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.toggle_diff_view(cx);
    cx.stop_propagation();
  }

  fn previous_page_tab_action(
    &mut self,
    _: &crate::PreviousPageTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let next_ix = adjacent_pr_tab_ix(self.active_tab_ix, TabNavigationDirection::Previous);
    self.set_active_tab(next_ix, window, cx);
    cx.stop_propagation();
  }

  fn next_page_tab_action(
    &mut self,
    _: &crate::NextPageTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let next_ix = adjacent_pr_tab_ix(self.active_tab_ix, TabNavigationDirection::Next);
    self.set_active_tab(next_ix, window, cx);
    cx.stop_propagation();
  }

  fn find_action(&mut self, action: &Find, window: &mut Window, cx: &mut Context<Self>) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.diff_editor.update(cx, |editor, cx| {
      editor::find(editor, action, window, cx);
    });
  }

  fn close_find_action(&mut self, action: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.diff_editor.update(cx, |editor, cx| {
      editor::close_find(editor, action, window, cx);
    });
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let commands = self.command_palette_commands(cx);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let config = CommandPaletteConfig::new(Vec::new(), commands, handler);
    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    let palette_for_dialog = palette.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      dialog
        .on_ok(|_, _, _| false)
        .p_0()
        .border_0()
        .min_h_0()
        .overlay_closable(true)
        .keyboard(true)
        .close_button(false)
        .child(palette_for_dialog.clone())
    });
  }

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match action {
      CommandPaletteAction::SwitchToPrBranch => {
        cx.defer_in(window, |this, window, cx| {
          this.prompt_or_switch_local_branch_to_pr_branch(window, cx);
        });
        Ok(())
      }
      CommandPaletteAction::OpenGitPage => {
        NavigationHistory::navigate("/git", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPage => {
        GithubPageHandle::refresh(cx);
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
        self.back_target = next_back_target_for_pr_palette(&self.back_target);
        self.load_pull_request(
          owner.clone(),
          repo.clone(),
          number,
          GithubPrOpenTarget {
            open_changes_tab,
            review_comment_id,
          },
          cx,
        );
        NavigationHistory::navigate(crate::navigation::build_pr_path(&owner, &repo, number), cx);
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
      CommandPaletteAction::ToggleUnchangedFiles => {
        let next = !self.show_local_project_files;
        self.set_show_local_project_files(next, cx);
        Ok(())
      }
      _ => Err("Command not available.".into()),
    }
  }

  fn open_file_search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let entries = self.active_file_search_entries(cx);
    if entries.is_empty() {
      return;
    }

    let view = cx.entity();
    let handler: SearchFileHandler = Arc::new(move |path, window, cx| {
      view.update(cx, |view, cx| {
        view.select_file_from_palette(&path, cx);
        view.refocus_page_shortcuts(window, cx);
      });

      Ok(())
    });
    open_shared_file_search_palette(window, cx, entries, handler, true);
  }

  fn select_file_from_palette(&mut self, path: &Path, cx: &mut Context<Self>) {
    let key = path.to_string_lossy().to_string();

    let tree_item = TreeItem::new(key.clone(), key.clone());
    self.tree_state.update(cx, |state, cx| {
      state.set_selected_item(Some(&tree_item), cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
    });

    self.select_visible_tree_path(key.as_str(), cx);
  }

  fn sync_tree_selection(&mut self, cx: &mut Context<Self>) {
    if self.show_local_project_files && self.selected_local_project_file.is_some() {
      self.sync_local_project_tree_selection(cx);
      return;
    }

    let Some(file) = self.selected_file.as_ref() else {
      return;
    };

    let key = file.path.as_ref().to_string();
    let tree_item = TreeItem::new(key.clone(), key.clone());
    self.tree_state.update(cx, |state, cx| {
      state.set_selected_item(Some(&tree_item), cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
    });
  }

  fn render_checks_open_url_button(
    &self,
    id: String,
    label: &'static str,
    url: Option<String>,
    _cx: &mut Context<Self>,
  ) -> Option<gpui::AnyElement> {
    let url = url
      .map(|value| value.trim().to_string())
      .filter(|value| !value.is_empty())?;

    Some(
      Button::new(id)
        .ghost()
        .xsmall()
        .compact()
        .label(label)
        .on_click(move |_, _, cx| {
          cx.open_url(&url);
        })
        .into_any_element(),
    )
  }

  fn render_workflow_job_card(
    &self,
    _run_id: u64,
    job: GithubPullRequestWorkflowJob,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let open_button = self.render_checks_open_url_button(
      format!("pr-checks-job-open-{}", job.id),
      "Open",
      job.html_url.clone(),
      cx,
    );
    let time_label = job
      .started_at
      .as_deref()
      .map(format_relative_time)
      .unwrap_or_else(|| "Unknown time".into());

    v_flex()
      .gap_2()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .p_2()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_2()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .flex_wrap()
              .child(
                div()
                  .text_sm()
                  .font_medium()
                  .text_color(theme.foreground)
                  .child(job.name.clone()),
              )
              .when(job.required, |this| {
                this.child(Tag::secondary().small().rounded_full().child("Required"))
              })
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(time_label),
              ),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(render_checks_state_badge(job.state, theme))
              .when_some(open_button, |this, button| this.child(button)),
          ),
      )
      .when(!job.steps.is_empty(), |this| {
        this.child(
          v_flex()
            .gap_1()
            .pl_3()
            .border_l_1()
            .border_color(theme.border)
            .children(
              job
                .steps
                .into_iter()
                .map(|step: GithubPullRequestWorkflowStep| {
                  let step_time = step
                    .started_at
                    .as_deref()
                    .map(format_relative_time)
                    .unwrap_or_else(|| "Unknown time".into());
                  h_flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                      h_flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .child(
                          div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("#{}", step.number)),
                        )
                        .child(
                          div()
                            .min_w_0()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child(step.name),
                        )
                        .child(
                          div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(step_time),
                        ),
                    )
                    .child(render_checks_state_badge(step.state, theme))
                    .into_any_element()
                }),
            ),
        )
      })
      .into_any_element()
  }

  fn render_workflow_run_card(
    &self,
    run: GithubPullRequestWorkflowRun,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let title = run
      .name
      .clone()
      .filter(|value| !value.trim().is_empty())
      .unwrap_or_else(|| "Workflow run".to_string());
    let open_button = self.render_checks_open_url_button(
      format!("pr-checks-run-open-{}", run.id),
      "Open run",
      run.html_url.clone(),
      cx,
    );
    let meta = format!(
      "Run #{}{} • {}",
      run.run_number,
      run
        .run_attempt
        .map(|attempt| format!(" attempt {}", attempt))
        .unwrap_or_default(),
      run.event
    );
    let time_label = format_relative_time(
      run
        .run_started_at
        .as_deref()
        .unwrap_or(run.created_at.as_str()),
    );

    v_flex()
      .gap_3()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .p_3()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_2()
          .child(
            v_flex()
              .gap_1()
              .min_w_0()
              .child(
                div()
                  .text_sm()
                  .font_medium()
                  .text_color(theme.foreground)
                  .child(title),
              )
              .when_some(run.display_title.clone(), |this, display_title| {
                this.child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(display_title),
                )
              })
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(format!("{} • {}", meta, time_label)),
              ),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(render_checks_state_badge(run.state, theme))
              .when_some(open_button, |this, button| this.child(button)),
          ),
      )
      .when(run.jobs.is_empty(), |this| {
        this.child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("No job details were returned for this workflow run."),
        )
      })
      .when(!run.jobs.is_empty(), |this| {
        this.child(
          v_flex().gap_2().children(
            run
              .jobs
              .into_iter()
              .map(|job| self.render_workflow_job_card(run.id, job, theme, cx)),
          ),
        )
      })
      .into_any_element()
  }

  fn render_check_run_card(
    &self,
    check: GithubPullRequestCheckRun,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let open_button = self.render_checks_open_url_button(
      format!("pr-checks-check-run-open-{}", check.id),
      "Open",
      check.details_url.clone().or(check.html_url.clone()),
      cx,
    );
    let time_label = check
      .started_at
      .as_deref()
      .map(format_relative_time)
      .unwrap_or_else(|| "Unknown time".into());

    v_flex()
      .gap_2()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .p_3()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_2()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .flex_wrap()
              .child(
                div()
                  .text_sm()
                  .font_medium()
                  .text_color(theme.foreground)
                  .child(check.name.clone()),
              )
              .when(check.required, |this| {
                this.child(Tag::secondary().small().rounded_full().child("Required"))
              })
              .when_some(check.app_name.clone(), |this, app_name| {
                this.child(Tag::secondary().small().rounded_full().child(app_name))
              })
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(time_label),
              ),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(render_checks_state_badge(check.state, theme))
              .when_some(open_button, |this, button| this.child(button)),
          ),
      )
      .when_some(check.summary.clone(), |this, summary| {
        this.child(div().text_sm().text_color(theme.foreground).child(summary))
      })
      .when_some(check.text.clone(), |this, text| {
        this.child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(text),
        )
      })
      .when(check.annotations_count > 0, |this| {
        this.child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("{} annotations reported", check.annotations_count)),
        )
      })
      .into_any_element()
  }

  fn render_legacy_status_card(
    &self,
    status: GithubPullRequestLegacyStatus,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let open_button = self.render_checks_open_url_button(
      format!("pr-checks-legacy-status-open-{}", status.id),
      "Open",
      status.target_url.clone(),
      cx,
    );
    let time_label = format_relative_time(status.updated_at.as_str());

    v_flex()
      .gap_2()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .p_3()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_2()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .flex_wrap()
              .child(
                div()
                  .text_sm()
                  .font_medium()
                  .text_color(theme.foreground)
                  .child(status.context.clone()),
              )
              .when(status.required, |this| {
                this.child(Tag::secondary().small().rounded_full().child("Required"))
              })
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(time_label),
              ),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(render_checks_state_badge(status.state, theme))
              .when_some(open_button, |this, button| this.child(button)),
          ),
      )
      .when_some(status.description.clone(), |this, description| {
        this.child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(description),
        )
      })
      .into_any_element()
  }

  fn render_checks_tab(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();

    let content: gpui::AnyElement = if self.checks_loading && self.checks.is_none() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading checks..."),
        )
        .into_any_element()
    } else if let Some(error) = self.checks_error.as_ref() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(error.clone())
        .into_any_element()
    } else if let Some(checks) = self.checks.clone() {
      let missing_required_contexts = checks.missing_required_contexts.clone();
      let has_missing_required_contexts = !missing_required_contexts.is_empty();
      let actions_runs = checks.actions_runs.clone();
      let has_actions_runs = !actions_runs.is_empty();
      let other_checks = checks.other_checks.clone();
      let has_other_checks = !other_checks.is_empty();
      let legacy_statuses = checks.legacy_statuses.clone();
      let has_legacy_statuses = !legacy_statuses.is_empty();

      v_flex()
        .w_full()
        .max_w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH))
        .mx_auto()
        .py_4()
        .px_10()
        .gap_4()
        .child(render_checks_summary_card(&checks, &theme, None))
        .when(has_missing_required_contexts, |this| {
          this.child(
            v_flex()
              .gap_2()
              .border_1()
              .border_color(theme.border)
              .rounded(theme.radius)
              .p_3()
              .child(
                div()
                  .text_sm()
                  .font_medium()
                  .text_color(theme.foreground)
                  .child("Required checks still waiting to report"),
              )
              .children(missing_required_contexts.into_iter().map(|context| {
                h_flex()
                  .items_center()
                  .justify_between()
                  .gap_2()
                  .child(div().text_sm().text_color(theme.foreground).child(context))
                  .child(render_checks_state_badge(
                    GithubPullRequestChecksRollupState::Pending,
                    &theme,
                  ))
                  .into_any_element()
              })),
          )
        })
        .when(has_actions_runs, |this| {
          this.child(
            v_flex()
              .gap_3()
              .child(
                div()
                  .text_sm()
                  .font_medium()
                  .text_color(theme.foreground)
                  .child("GitHub Actions"),
              )
              .children(
                actions_runs
                  .into_iter()
                  .map(|run| self.render_workflow_run_card(run, &theme, cx)),
              ),
          )
        })
        .when(has_other_checks, |this| {
          this.child(
            v_flex()
              .gap_3()
              .child(
                div()
                  .text_sm()
                  .font_medium()
                  .text_color(theme.foreground)
                  .child("Other checks"),
              )
              .children(
                other_checks
                  .into_iter()
                  .map(|check| self.render_check_run_card(check, &theme, cx)),
              ),
          )
        })
        .when(has_legacy_statuses, |this| {
          this.child(
            v_flex()
              .gap_3()
              .child(
                div()
                  .text_sm()
                  .font_medium()
                  .text_color(theme.foreground)
                  .child("Legacy statuses"),
              )
              .children(
                legacy_statuses
                  .into_iter()
                  .map(|status| self.render_legacy_status_card(status, &theme, cx)),
              ),
          )
        })
        .when(
          !has_actions_runs
            && !has_other_checks
            && !has_legacy_statuses
            && !has_missing_required_contexts,
          |this| {
            this.child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("GitHub has not reported any checks for this pull request yet."),
            )
          },
        )
        .into_any_element()
    } else {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("Checks are unavailable for this pull request.")
        .into_any_element()
    };

    div()
      .id("github-pr-checks-scroll")
      .size_full()
      .overflow_y_scrollbar()
      .child(content)
  }

  fn render_changes_tab(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let local_project_mode = self.local_project_mode_active(cx);
    let is_markdown = self.selected_file_is_markdown();
    let is_svg = self.selected_file_is_svg();
    let editor_content: gpui::AnyElement = if self.file_loading {
      if self.selected_local_project_file.is_some() && self.selected_file.is_none() {
        v_flex()
          .flex_1()
          .items_center()
          .justify_center()
          .gap_2()
          .child(Spinner::new().small())
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child("Loading local file..."),
          )
          .into_any_element()
      } else {
        v_flex()
          .flex_1()
          .items_center()
          .justify_center()
          .gap_2()
          .child(Spinner::new().small())
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child("Loading file contents..."),
          )
          .into_any_element()
      }
    } else if self.file_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.file_error.clone().unwrap_or_default())
        .into_any_element()
    } else if self.selected_file.is_some() || self.selected_local_project_file.is_some() {
      self.render_selected_editor_content(is_markdown, is_svg, window, cx)
    } else if local_project_mode && self.local_project_tree_loading {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading project files..."),
        )
        .into_any_element()
    } else if local_project_mode && self.local_project_tree_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.local_project_tree_error.clone().unwrap_or_default())
        .into_any_element()
    } else if !local_project_mode && self.files_loading {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading diff..."),
        )
        .into_any_element()
    } else if !local_project_mode && self.files_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.files_error.clone().unwrap_or_default())
        .into_any_element()
    } else if local_project_mode {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("Select a file to view it")
        .into_any_element()
    } else {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("Select a file to view diff")
        .into_any_element()
    };

    let editor_panel = v_flex()
      .size_full()
      .overflow_hidden()
      .when_some(self.selected_file.as_ref(), |this, file| {
        this.child(self.render_diff_header(file, cx))
      })
      .when(
        self.selected_file.is_none() && self.selected_local_project_file.is_some(),
        |this| {
          this.when_some(self.selected_local_project_file.as_ref(), |this, file| {
            this.child(self.render_local_project_file_header(file, cx))
          })
        },
      )
      .child(editor_content);

    editor_panel
  }
}

impl Render for GithubPrDetailsPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    self.maybe_refresh_resolved_local_repo_match(window, cx);

    // Poll syntax highlight cache — if background highlights completed, schedule re-render
    if self.syntax_highlight_cache.take_new_highlights() {
      cx.notify();
    } else if self.syntax_highlight_cache.has_pending() {
      // Background highlights still in progress — check again next frame
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
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading pull request details..."),
        )
        .into_any_element()
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

    let checks_content = div()
      .id("checks-tab")
      .flex_1()
      .min_h_0()
      .child(self.render_checks_tab(window, cx))
      .into_any_element();

    let content = if self.active_tab_ix == PR_TAB_OVERVIEW_IX {
      overview_content
    } else if self.active_tab_ix == PR_TAB_CHANGES_IX {
      changes_content
    } else if self.active_tab_ix >= PR_TAB_CHECKS_IX {
      checks_content
    } else {
      overview_content
    };

    let right_panel = v_flex()
      .size_full()
      .overflow_hidden()
      .child(self.render_header(cx))
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
      .on_action(cx.listener(GithubPrDetailsPage::toggle_diff_view_action))
      .on_action(cx.listener(GithubPrDetailsPage::previous_page_tab_action))
      .on_action(cx.listener(GithubPrDetailsPage::next_page_tab_action))
      .on_action(cx.listener(GithubPrDetailsPage::show_file_search_action))
      .on_action(cx.listener(GithubPrDetailsPage::find_action))
      .on_action(cx.listener(GithubPrDetailsPage::close_find_action))
      .child(
        h_resizable("github-pr-layout")
          .child(
            resizable_panel()
              .size(px(SIDEBAR_DEFAULT_WIDTH))
              .size_range(px(SIDEBAR_MIN_WIDTH)..px(SIDEBAR_MAX_WIDTH))
              .child(self.render_left_sidebar(window, cx)),
          )
          .child(resizable_panel().child(right_panel)),
      )
  }
}

impl Focusable for GithubPrDetailsPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::api::{
    GithubPullRequestChecksRollupState, GithubPullRequestChecksSummary, GithubPullRequestCommit,
    GithubPullRequestCommitUser, GithubPullRequestDescriptionUpdate, GithubPullRequestDetails,
    GithubPullRequestFile, GithubPullRequestIssueComment, GithubPullRequestIssueCommentUser,
    GithubPullRequestMergeMethod, GithubPullRequestMergeReadiness,
    GithubPullRequestMergeReadinessStatus, GithubPullRequestReview, GithubPullRequestReviewComment,
    GithubPullRequestReviewCommentUser, GithubPullRequestReviewEvent, GithubPullRequestReviewState,
    GithubPullRequestState, GithubRepository,
  };
  use crate::workspace::WorkspaceApi;
  use git::{BranchKind, BranchRef, merge_branch};
  use git2::{BranchType, Repository, Signature};
  use gpui::TestAppContext;
  use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{SystemTime, UNIX_EPOCH},
  };

  fn init_gpui_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
      gpui_component::init(cx);
      if !cx.has_global::<WorkspaceApi>() {
        cx.set_global(WorkspaceApi::new());
      }
      if !cx.has_global::<AuthStateStore>() {
        cx.set_global(AuthStateStore::default());
      }
      if !cx.has_global::<ActiveLocalRepoStore>() {
        cx.set_global(ActiveLocalRepoStore::default());
      }
      if !cx.has_global::<AppSettings>() {
        cx.set_global(AppSettings::default());
      }
      ActiveLocalRepoStore::set(cx, None);
    });
  }

  fn unique_test_db_path(label: &str) -> PathBuf {
    static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
      "reviu-pr-details-{label}-{}-{id}.sqlite",
      std::process::id()
    ))
  }

  fn make_test_api_client(base_url: impl Into<String>) -> ApiClient {
    ApiClient::new_with_base_url(base_url)
  }

  fn start_response_server(responses: Vec<(String, String)>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let address = format!("http://{}", listener.local_addr().expect("local addr"));

    let handle = thread::spawn(move || {
      for (status, body) in responses {
        let (mut stream, _) = listener.accept().expect("accept connection");
        let mut request_buffer = [0u8; 4096];
        let _ = stream.read(&mut request_buffer).expect("read request");

        let response = format!(
          "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
          body.as_bytes().len(),
          body,
        );
        stream
          .write_all(response.as_bytes())
          .expect("write response");
        stream.flush().expect("flush response");
      }
    });

    (address, handle)
  }

  fn start_single_response_server(status: &str, body: &str) -> (String, thread::JoinHandle<()>) {
    start_response_server(vec![(status.to_string(), body.to_string())])
  }

  fn make_active_local_repo(head_sha: &str, has_uncommitted_changes: bool) -> ActiveLocalRepo {
    make_active_local_repo_for_branch("feature", head_sha, has_uncommitted_changes)
  }

  fn make_active_local_repo_for_branch(
    current_branch: &str,
    head_sha: &str,
    has_uncommitted_changes: bool,
  ) -> ActiveLocalRepo {
    ActiveLocalRepo {
      repo_root: PathBuf::from("/tmp/reviu-tests/acme-widget"),
      github_owner: Some("acme".to_string()),
      github_repo: Some("widget".to_string()),
      current_branch: Some(current_branch.to_string()),
      head_sha: Some(head_sha.to_string()),
      has_uncommitted_changes,
    }
  }

  fn commit_local_project_file(repo_root: &Path, rel_path: &Path, contents: &str, message: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::write(repo_root.join(rel_path), contents).expect("write project file");

    let mut index = repo.index().expect("open git index");
    index.add_path(rel_path).expect("stage project file");
    index.write().expect("write git index");
    let tree_id = index.write_tree().expect("write git tree");
    let tree = repo.find_tree(tree_id).expect("find git tree");
    let signature = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());

    match parent {
      Some(parent) => {
        repo
          .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&parent],
          )
          .expect("commit with parent");
      }
      None => {
        repo
          .commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
          .expect("initial commit");
      }
    }
  }

  fn create_local_repo_with_github_remote(
    owner: &str,
    repo_name: &str,
    current_branch: &str,
    additional_branches: &[&str],
  ) -> (PathBuf, ActiveLocalRepo) {
    let unique = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system time after unix epoch")
      .as_nanos();
    let repo_root = std::env::temp_dir().join(format!(
      "reviu-pr-details-local-repo-{}-{unique}",
      std::process::id()
    ));

    std::fs::create_dir_all(repo_root.join("src")).expect("create repo directories");
    Repository::init(&repo_root).expect("init local repo");
    commit_local_project_file(
      &repo_root,
      Path::new("src/main.rs"),
      "fn main() {}\n",
      "initial",
    );

    let repo = Repository::open(&repo_root).expect("open local repo");
    repo
      .remote(
        "origin",
        format!("https://github.com/{owner}/{repo_name}.git").as_str(),
      )
      .expect("create origin remote");
    {
      let head_commit = repo
        .head()
        .expect("repo head")
        .peel_to_commit()
        .expect("head commit");

      for branch_name in additional_branches
        .iter()
        .copied()
        .chain(std::iter::once(current_branch))
      {
        if repo.find_branch(branch_name, BranchType::Local).is_err() {
          repo
            .branch(branch_name, &head_commit, false)
            .expect("create local branch");
        }
      }
    }
    drop(repo);

    switch_to_branch_name(&repo_root, current_branch).expect("switch to current branch");
    let snapshot =
      local_repo_snapshot(&repo_root, None).expect("snapshot local repo with GitHub remote");
    (repo_root, snapshot)
  }

  fn make_pr_details_for_local_repo(
    head_sha: &str,
    head_ref_name: &str,
  ) -> GithubPullRequestDetails {
    let mut pull_request = make_pr_details_for_stats();
    pull_request.head_sha = head_sha.to_string();
    pull_request.head_ref_name = head_ref_name.to_string();
    pull_request
  }

  fn make_api_file(
    filename: &str,
    status: &str,
    previous_filename: Option<&str>,
  ) -> GithubPullRequestFile {
    GithubPullRequestFile {
      filename: filename.to_string(),
      status: status.to_string(),
      patch: None,
      previous_filename: previous_filename.map(str::to_string),
    }
  }

  fn make_api_commit(
    sha: &str,
    message: &str,
    committed_at: Option<&str>,
    parent_sha: Option<&str>,
  ) -> GithubPullRequestCommit {
    GithubPullRequestCommit {
      sha: sha.to_string(),
      message: message.to_string(),
      authored_at: committed_at.map(str::to_string),
      committed_at: committed_at.map(str::to_string),
      parent_sha: parent_sha.map(str::to_string),
      author: Some(GithubPullRequestCommitUser {
        login: "octocat".to_string(),
        avatar_url: None,
      }),
      committer: Some(GithubPullRequestCommitUser {
        login: "octocat".to_string(),
        avatar_url: None,
      }),
    }
  }

  fn make_merge_readiness(
    status: GithubPullRequestMergeReadinessStatus,
    methods: Vec<GithubPullRequestMergeMethod>,
  ) -> GithubPullRequestMergeReadiness {
    GithubPullRequestMergeReadiness {
      status,
      message: match status {
        GithubPullRequestMergeReadinessStatus::Ready => {
          "This pull request is ready to merge.".to_string()
        }
        GithubPullRequestMergeReadinessStatus::Blocked => {
          "This pull request is blocked by required checks.".to_string()
        }
        GithubPullRequestMergeReadinessStatus::Checking => {
          "GitHub is still computing whether this pull request can be merged.".to_string()
        }
        GithubPullRequestMergeReadinessStatus::Forbidden => {
          "You do not have permission to merge this pull request.".to_string()
        }
        GithubPullRequestMergeReadinessStatus::Draft => {
          "This pull request is still marked as a draft.".to_string()
        }
        GithubPullRequestMergeReadinessStatus::Closed => "This pull request is closed.".to_string(),
        GithubPullRequestMergeReadinessStatus::Merged => {
          "This pull request has already been merged.".to_string()
        }
      },
      current_head_sha: "head123".to_string(),
      default_method: methods.first().copied(),
      can_merge_now: status == GithubPullRequestMergeReadinessStatus::Ready && !methods.is_empty(),
      viewer_can_merge: true,
      mergeable_state: Some("clean".to_string()),
      rebaseable: Some(true),
      auto_merge_enabled: false,
      available_methods: methods,
    }
  }

  fn make_merge_readiness_with_state(
    status: GithubPullRequestMergeReadinessStatus,
    mergeable_state: Option<&str>,
    message: &str,
  ) -> GithubPullRequestMergeReadiness {
    let mut readiness = make_merge_readiness(status, vec![GithubPullRequestMergeMethod::Merge]);
    readiness.mergeable_state = mergeable_state.map(ToString::to_string);
    readiness.message = message.to_string();
    readiness
  }

  fn make_checks_summary() -> GithubPullRequestChecksSummary {
    GithubPullRequestChecksSummary {
      head_sha: "head123".to_string(),
      overall_state: GithubPullRequestChecksRollupState::Failure,
      required_state: GithubPullRequestChecksRollupState::Pending,
      total_checks: 4,
      successful_checks: 2,
      failed_checks: 1,
      pending_checks: 1,
      required_checks_total: 3,
      required_checks_passed: 1,
      required_checks_failed: 1,
      required_checks_pending: 1,
      required_contexts: vec![
        "build".to_string(),
        "lint".to_string(),
        "deploy".to_string(),
      ],
      missing_required_contexts: vec!["deploy".to_string()],
      requires_up_to_date_branch: true,
      actions_runs: Vec::new(),
      other_checks: Vec::new(),
      legacy_statuses: Vec::new(),
    }
  }

  #[test]
  fn overview_pr_alert_content_returns_conflicts_for_dirty_mergeable_state() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("dirty"),
      "This pull request has merge conflicts that must be resolved before it can be merged.",
    );

    let alert = overview_pr_alert_content(Some(&readiness), None).expect("alert");

    assert_eq!(alert.id, "github-pr-overview-conflicts-alert");
    assert_eq!(alert.title, "Merge conflicts detected");
  }

  #[test]
  fn overview_pr_alert_content_returns_out_of_date_for_behind_mergeable_state() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("behind"),
      "This pull request branch is out of date with the base branch.",
    );

    let alert = overview_pr_alert_content(Some(&readiness), None).expect("alert");

    assert_eq!(alert.id, "github-pr-overview-out-of-date-alert");
    assert_eq!(alert.title, "Branch is out of date");
  }

  #[test]
  fn overview_pr_alert_content_falls_back_to_checks_requirement() {
    let alert = overview_pr_alert_content(None, Some(&make_checks_summary())).expect("alert");

    assert_eq!(alert.id, "github-pr-overview-out-of-date-alert");
    assert_eq!(alert.title, "Branch is out of date");
  }

  #[test]
  fn overview_pr_alert_content_returns_none_when_pr_is_ready() {
    let mut checks = make_checks_summary();
    checks.requires_up_to_date_branch = false;
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );

    assert!(overview_pr_alert_content(Some(&readiness), Some(&checks)).is_none());
  }

  #[gpui::test]
  fn set_active_tab_changes_focuses_file_tree(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    let files = files_from_api(vec![
      make_api_file("src/main.rs", "modified", None),
      make_api_file("src/lib.rs", "modified", None),
    ]);
    let (items, lookup, selected_index, selected_id) = build_tree_items(&files);

    page.update_in(cx, |this, window, cx| {
      this.file_lookup = lookup;
      this.selected_tree_id = selected_id.clone();
      this.selected_file = selected_id
        .as_ref()
        .and_then(|id| this.file_lookup.get(id).cloned());
      this.tree_state.update(cx, |state, cx| {
        state.set_items(items, cx);
        state.set_selected_index(selected_index, cx);
      });

      let external_focus = cx.focus_handle();
      let page_focus = this.focus_handle.clone();
      window.focus(&external_focus, cx);

      this.set_active_tab(PR_TAB_CHANGES_IX, window, cx);

      let focused = window.focused(cx).expect("changes tree should take focus");
      assert_ne!(focused, external_focus);
      assert_ne!(focused, page_focus);
    });
  }

  #[gpui::test]
  fn changes_markdown_preview_keeps_editor_and_preview_panes_visible(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let markdown = "# Preview\n\nPR markdown preview should stay visible.\n";
    let file = files_from_api(vec![make_api_file("README.md", "modified", None)])
      .into_iter()
      .next()
      .expect("markdown file");
    let file_key = file.path.to_string();
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.active_tab_ix = PR_TAB_CHANGES_IX;
      this.files_loading = false;
      this.file_loading = false;
      this.files_error = None;
      this.file_error = None;
      this.file_lookup.insert(file_key.clone(), file.clone());
      this.file_contents.insert(
        file_key.clone(),
        GithubPrFileContents {
          base: Some(String::new()),
          head: Some(markdown.to_string()),
        },
      );
      this.set_selected_file(Some(file.clone()), cx);
      this.show_markdown_preview = true;
      this.sync_diff_view(cx);
      cx.notify();
    });

    let editor_bounds = cx
      .debug_bounds(GITHUB_PR_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR)
      .expect("pr preview editor pane bounds")
      .size;
    let preview_bounds = cx
      .debug_bounds(GITHUB_PR_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR)
      .expect("pr preview render pane bounds")
      .size;

    assert!(editor_bounds.width > gpui::px(0.0));
    assert!(editor_bounds.height > gpui::px(0.0));
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
  }

  #[gpui::test]
  async fn changes_raster_image_preview_renders_without_source_editor(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    let (base_url, handle) = start_single_response_server(
      "200 OK",
      r#"{"contentBase64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP0cAAAAASUVORK5CYII="}"#,
    );
    let file = files_from_api(vec![make_api_file("image.png", "modified", None)])
      .into_iter()
      .next()
      .expect("image file");
    let file_key = file.path.to_string();
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    let file_asset_task = page.update_in(cx, |this, _window, cx| {
      this.active_tab_ix = PR_TAB_CHANGES_IX;
      this.api = make_test_api_client(base_url.clone());
      this.pull_request = Some(make_pr_details_for_stats());
      this.files_loading = false;
      this.file_loading = false;
      this.file_lookup.insert(file_key.clone(), file.clone());
      this.set_selected_file(Some(file.clone()), cx);
      this
        .file_asset_tasks
        .remove(file_key.as_str())
        .expect("file asset task should exist")
    });
    file_asset_task.await;
    handle.join().expect("join server thread");

    let is_raster_preview = page.read_with(cx, |this, _cx| {
      matches!(
        this.binary_preview,
        Some(GithubPrBinaryPreview::RasterImage(_))
      )
    });
    let preview_bounds = cx
      .debug_bounds(GITHUB_PR_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR)
      .expect("binary preview render pane bounds")
      .size;

    assert!(is_raster_preview);
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
    assert!(
      cx.debug_bounds(GITHUB_PR_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR)
        .is_none()
    );
    assert!(
      cx.debug_bounds(GITHUB_PR_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR)
        .is_none()
    );
  }

  #[gpui::test]
  async fn selected_raster_image_fetches_asset_when_pr_details_arrive_late(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    let (base_url, handle) = start_single_response_server(
      "200 OK",
      r#"{"contentBase64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP0cAAAAASUVORK5CYII="}"#,
    );
    let file = files_from_api(vec![make_api_file("image.png", "modified", None)])
      .into_iter()
      .next()
      .expect("image file");
    let file_key = file.path.to_string();
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.active_tab_ix = PR_TAB_CHANGES_IX;
      this.api = make_test_api_client(base_url.clone());
      this.file_lookup.insert(file_key.clone(), file.clone());
      this.set_selected_file(Some(file.clone()), cx);

      assert!(this.file_loading);
      assert!(this.file_content_tasks.is_empty());
      assert!(this.file_asset_tasks.is_empty());

      this.pull_request = Some(make_pr_details_for_stats());
      this.maybe_fetch_selected_file_contents(cx);

      assert!(this.file_content_tasks.is_empty());
      assert!(this.file_asset_tasks.contains_key(file_key.as_str()));
    });

    let file_asset_task = page.update_in(cx, |this, _window, _cx| {
      this
        .file_asset_tasks
        .remove(file_key.as_str())
        .expect("file asset task should exist")
    });
    file_asset_task.await;
    handle.join().expect("join server thread");

    let is_raster_preview = page.read_with(cx, |this, _cx| {
      matches!(
        this.binary_preview,
        Some(GithubPrBinaryPreview::RasterImage(_))
      )
    });
    let preview_bounds = cx
      .debug_bounds(GITHUB_PR_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR)
      .expect("binary preview render pane bounds")
      .size;

    assert!(is_raster_preview);
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
  }

  #[gpui::test]
  fn changes_unsupported_binary_preview_shows_placeholder(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let file = files_from_api(vec![make_api_file("slides.pdf", "modified", None)])
      .into_iter()
      .next()
      .expect("pdf file");
    let file_key = file.path.to_string();
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.active_tab_ix = PR_TAB_CHANGES_IX;
      this.pull_request = Some(make_pr_details_for_stats());
      this.files_loading = false;
      this.file_loading = false;
      this.file_lookup.insert(file_key, file.clone());
      this.set_selected_file(Some(file.clone()), cx);
      cx.notify();
    });

    let is_placeholder = page.read_with(cx, |this, _cx| {
      matches!(
        this.binary_preview,
        Some(GithubPrBinaryPreview::UnsupportedBinary)
      )
    });
    let preview_bounds = cx
      .debug_bounds(GITHUB_PR_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR)
      .expect("binary preview placeholder bounds")
      .size;

    assert!(is_placeholder);
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
  }

  #[gpui::test]
  fn merge_button_renders_for_loaded_pull_request(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      this.merge_readiness = Some(make_merge_readiness(
        GithubPullRequestMergeReadinessStatus::Ready,
        vec![GithubPullRequestMergeMethod::Merge],
      ));
      cx.notify();
    });

    let button_bounds = cx
      .debug_bounds("github-pr-merge-button")
      .expect("merge button bounds")
      .size;
    assert!(button_bounds.width > gpui::px(0.0));
    assert!(button_bounds.height > gpui::px(0.0));
  }

  #[gpui::test]
  fn changes_tab_renders_changed_files_count_tag(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

    let count_bounds = cx
      .debug_bounds("github-pr-changes-tab-count")
      .expect("changes tab count bounds")
      .size;
    assert!(count_bounds.width > gpui::px(0.0));
    assert!(count_bounds.height > gpui::px(0.0));
  }

  #[gpui::test]
  fn overview_updated_row_renders_inline_change_stats(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

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
  fn overview_conflicts_alert_is_built_when_pr_has_merge_conflicts(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      this.merge_readiness = Some(make_merge_readiness_with_state(
        GithubPullRequestMergeReadinessStatus::Blocked,
        Some("dirty"),
        "This pull request has merge conflicts that must be resolved before it can be merged.",
      ));
      cx.notify();
    });

    let has_alert = page.update_in(cx, |this, _window, cx| {
      this.render_overview_pr_alert(cx).is_some()
    });
    assert!(has_alert);
  }

  #[gpui::test]
  fn overview_conflicts_alert_exposes_git_page_action_when_local_repo_is_available(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(cx, Some(make_active_local_repo("head", false)));
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      this.merge_readiness = Some(make_merge_readiness_with_state(
        GithubPullRequestMergeReadinessStatus::Blocked,
        Some("dirty"),
        "This pull request has merge conflicts that must be resolved before it can be merged.",
      ));
      cx.notify();
    });

    let action_label = page.read_with(cx, |this, cx| {
      let content =
        overview_pr_alert_content(this.merge_readiness.as_ref(), this.checks.as_ref()).unwrap();
      this.overview_pr_alert_action_label(&content, cx)
    });
    assert_eq!(action_label, Some("Resolve in Git page"));
  }

  #[gpui::test]
  fn pull_request_status_action_matches_open_draft_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, _cx| {
      this.pull_request = Some(make_pr_details_for_stats());
    });
    let open_action = page.read_with(cx, |this, _cx| this.pull_request_status_action());
    assert_eq!(open_action, Some(GithubPrStatusAction::ConvertToDraft));

    page.update_in(cx, |this, _window, _cx| {
      let mut draft_pull_request = make_pr_details_for_stats();
      draft_pull_request.draft = true;
      this.pull_request = Some(draft_pull_request);
    });
    let draft_action = page.read_with(cx, |this, _cx| this.pull_request_status_action());
    assert_eq!(draft_action, Some(GithubPrStatusAction::ReadyForReview));

    page.update_in(cx, |this, _window, _cx| {
      let mut closed_pull_request = make_pr_details_for_stats();
      closed_pull_request.state = GithubPullRequestState::Closed;
      this.pull_request = Some(closed_pull_request);
    });
    let closed_action = page.read_with(cx, |this, _cx| this.pull_request_status_action());
    assert_eq!(closed_action, None);
  }

  #[gpui::test]
  fn status_action_button_renders_for_loaded_pull_request(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

    let button_bounds = cx
      .debug_bounds("github-pr-status-action-button")
      .expect("status action button bounds")
      .size;
    assert!(button_bounds.width > gpui::px(0.0));
    assert!(button_bounds.height > gpui::px(0.0));
  }

  #[gpui::test]
  fn refocus_page_shortcuts_focuses_page_container(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, window, cx| {
      let external_focus = cx.focus_handle();
      let page_focus = this.focus_handle.clone();
      window.focus(&external_focus, cx);

      this.refocus_page_shortcuts(window, cx);

      let focused = window.focused(cx).expect("page should take focus");
      assert_eq!(focused, page_focus);
    });
  }

  #[gpui::test]
  fn checks_summary_renders_when_checks_tab_is_active(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      this.active_tab_ix = PR_TAB_CHECKS_IX;
      this.checks = Some(make_checks_summary());
      cx.notify();
    });

    let summary_bounds = cx
      .debug_bounds("github-pr-checks-summary-card")
      .expect("checks summary bounds")
      .size;
    assert!(summary_bounds.width > gpui::px(0.0));
    assert!(summary_bounds.height > gpui::px(0.0));
  }

  #[gpui::test]
  fn checks_tab_renders_overall_status_badge_in_header(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      this.checks = Some(make_checks_summary());
      cx.notify();
    });

    let overall_status_bounds = cx
      .debug_bounds("github-pr-checks-tab-status")
      .expect("checks tab status bounds")
      .size;
    assert!(overall_status_bounds.width > gpui::px(0.0));
    assert!(overall_status_bounds.height > gpui::px(0.0));
  }

  #[gpui::test]
  fn merge_and_review_buttons_do_not_render_for_merged_pull_request(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      let mut pull_request = make_pr_details_for_stats();
      pull_request.merged_at = Some("2026-03-19T21:20:00Z".to_string());
      this.pull_request = Some(pull_request);
      this.merge_readiness = Some(make_merge_readiness(
        GithubPullRequestMergeReadinessStatus::Merged,
        vec![],
      ));
      cx.notify();
    });

    assert!(cx.debug_bounds("github-pr-status-action-button").is_none());
    assert!(cx.debug_bounds("github-pr-merge-button").is_none());
    assert!(cx.debug_bounds("github-pr-review-button").is_none());
  }

  #[gpui::test]
  fn merge_button_does_not_render_for_draft_pull_request(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      let mut pull_request = make_pr_details_for_stats();
      pull_request.draft = true;
      this.pull_request = Some(pull_request);
      this.merge_readiness = Some(make_merge_readiness(
        GithubPullRequestMergeReadinessStatus::Draft,
        vec![],
      ));
      cx.notify();
    });

    assert!(cx.debug_bounds("github-pr-status-action-button").is_some());
    assert!(cx.debug_bounds("github-pr-merge-button").is_none());
    assert!(cx.debug_bounds("github-pr-review-button").is_some());
  }

  #[gpui::test]
  async fn draft_status_action_failure_shows_error_notification(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    let (base_url, handle) = start_single_response_server(
      "403 FORBIDDEN",
      r#"{"error":"You cannot change this pull request status."}"#,
    );

    let mut mounted_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let page = cx.new(|cx| GithubPrDetailsPage::new(window, cx));
      mounted_page = Some(page.clone());
      gpui_component::Root::new(page, window, cx)
    });
    let page = mounted_page.expect("pr details page");

    page.update_in(cx, |this, _window, cx| {
      let mut pull_request = make_pr_details_for_stats();
      pull_request.draft = true;
      this.api = make_test_api_client(base_url.clone());
      this.pull_request = Some(pull_request);
      cx.notify();
    });

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    let task = page.update_in(cx, |this, _window, cx| {
      this.submit_pull_request_status_action(GithubPrStatusAction::ReadyForReview, cx);
      this
        .status_action_task
        .take()
        .expect("status action task should exist")
    });
    task.await;
    handle.join().expect("join server thread");

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
    let loading = page.read_with(cx, |this, _cx| this.status_action_loading);
    assert!(!loading);
  }

  #[gpui::test]
  async fn convert_to_draft_success_updates_local_state_without_reloading_pr(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    let (base_url, handle) = start_single_response_server("204 NO CONTENT", "");
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.api = make_test_api_client(base_url.clone());
      this.pull_request = Some(make_pr_details_for_stats());
      this.merge_readiness = Some(make_merge_readiness(
        GithubPullRequestMergeReadinessStatus::Ready,
        vec![GithubPullRequestMergeMethod::Merge],
      ));
      cx.notify();
    });

    let task = page.update_in(cx, |this, _window, cx| {
      this.submit_pull_request_status_action(GithubPrStatusAction::ConvertToDraft, cx);
      this
        .status_action_task
        .take()
        .expect("status action task should exist")
    });
    task.await;
    handle.join().expect("join server thread");

    let (draft, merge_status, details_task_present, merge_readiness_task_present, loading) = page
      .read_with(cx, |this, _cx| {
        (
          this
            .pull_request
            .as_ref()
            .map(|pull_request| pull_request.draft),
          this
            .merge_readiness
            .as_ref()
            .map(|readiness| readiness.status),
          this.details_task.is_some(),
          this.merge_readiness_task.is_some(),
          this.status_action_loading,
        )
      });

    assert_eq!(draft, Some(true));
    assert_eq!(
      merge_status,
      Some(GithubPullRequestMergeReadinessStatus::Draft)
    );
    assert!(!details_task_present);
    assert!(!merge_readiness_task_present);
    assert!(!loading);
  }

  #[gpui::test]
  async fn ready_for_review_success_only_reloads_merge_readiness(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    let merge_readiness_body = r#"{
      "mergeReadiness": {
        "status": "ready",
        "message": "This pull request is ready to merge.",
        "current_head_sha": "head123",
        "available_methods": ["merge"],
        "default_method": "merge",
        "can_merge_now": true,
        "viewer_can_merge": true,
        "mergeable_state": "clean",
        "rebaseable": true,
        "auto_merge_enabled": false
      }
    }"#;
    let (base_url, handle) = start_response_server(vec![
      ("204 NO CONTENT".to_string(), String::new()),
      ("200 OK".to_string(), merge_readiness_body.to_string()),
    ]);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      let mut pull_request = make_pr_details_for_stats();
      pull_request.draft = true;
      this.api = make_test_api_client(base_url.clone());
      this.current_pr_context = Some(CurrentPrContext {
        owner: pull_request.repository.owner.clone(),
        repo: pull_request.repository.repo.clone(),
        number: pull_request.number,
      });
      this.pull_request = Some(pull_request);
      this.merge_readiness = Some(make_merge_readiness(
        GithubPullRequestMergeReadinessStatus::Draft,
        Vec::new(),
      ));
      cx.notify();
    });

    let task = page.update_in(cx, |this, _window, cx| {
      this.submit_pull_request_status_action(GithubPrStatusAction::ReadyForReview, cx);
      this
        .status_action_task
        .take()
        .expect("status action task should exist")
    });
    task.await;

    let merge_task = page.update_in(cx, |this, _window, _cx| {
      assert_eq!(
        this
          .pull_request
          .as_ref()
          .map(|pull_request| pull_request.draft),
        Some(false)
      );
      assert!(this.details_task.is_none());
      this.merge_readiness_task.take()
    });
    if let Some(task) = merge_task {
      task.await;
    }
    handle.join().expect("join server thread");

    let (draft, merge_status, details_task_present, loading, error) =
      page.read_with(cx, |this, _cx| {
        (
          this
            .pull_request
            .as_ref()
            .map(|pull_request| pull_request.draft),
          this
            .merge_readiness
            .as_ref()
            .map(|readiness| readiness.status),
          this.details_task.is_some(),
          this.merge_readiness_loading,
          this.merge_readiness_error.clone(),
        )
      });

    assert_eq!(draft, Some(false));
    assert_eq!(
      merge_status,
      Some(GithubPullRequestMergeReadinessStatus::Ready)
    );
    assert!(!details_task_present);
    assert!(!loading);
    assert!(error.is_none());
  }

  #[gpui::test]
  fn local_project_availability_is_ready_when_repo_branch_and_sha_match(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(cx, Some(make_active_local_repo("head", false)));
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

    let availability = page.read_with(cx, |this, cx| this.local_project_availability(cx));
    assert!(matches!(
      availability,
      GithubPrLocalProjectAvailability::Ready { .. }
    ));
  }

  #[test]
  fn pr_refresh_helpers_match_active_tab() {
    assert!(should_refresh_pr_overview_data(PR_TAB_OVERVIEW_IX));
    assert!(!should_refresh_pr_overview_data(PR_TAB_CHANGES_IX));
    assert!(!should_refresh_pr_overview_data(PR_TAB_CHECKS_IX));

    assert!(should_refresh_pr_changes_data(PR_TAB_CHANGES_IX));
    assert!(!should_refresh_pr_changes_data(PR_TAB_OVERVIEW_IX));
    assert!(!should_refresh_pr_changes_data(PR_TAB_CHECKS_IX));

    assert!(should_refresh_pr_checks_data(PR_TAB_CHECKS_IX));
    assert!(!should_refresh_pr_checks_data(PR_TAB_OVERVIEW_IX));
    assert!(!should_refresh_pr_checks_data(PR_TAB_CHANGES_IX));

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
      PR_TAB_CHECKS_IX,
      false,
      false,
      false,
      false,
      false,
      false,
      false,
      true,
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
    assert!(!pr_refresh_in_progress(
      PR_TAB_CHECKS_IX,
      false,
      false,
      false,
      false,
      false,
      false,
      false,
      false,
    ));
  }

  #[test]
  fn adjacent_pr_tab_ix_wraps_in_both_directions() {
    assert_eq!(
      adjacent_pr_tab_ix(PR_TAB_OVERVIEW_IX, TabNavigationDirection::Previous),
      PR_TAB_CHECKS_IX
    );
    assert_eq!(
      adjacent_pr_tab_ix(PR_TAB_CHECKS_IX, TabNavigationDirection::Next),
      PR_TAB_OVERVIEW_IX
    );
    assert_eq!(
      adjacent_pr_tab_ix(PR_TAB_CHANGES_IX, TabNavigationDirection::Previous),
      PR_TAB_OVERVIEW_IX
    );
    assert_eq!(
      adjacent_pr_tab_ix(PR_TAB_CHANGES_IX, TabNavigationDirection::Next),
      PR_TAB_CHECKS_IX
    );
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

  #[gpui::test]
  fn local_project_availability_requires_pr_head_sha_match_when_clean(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(cx, Some(make_active_local_repo("stale-head", false)));
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

    let availability = page.read_with(cx, |this, cx| this.local_project_availability(cx));
    assert!(matches!(
      availability,
      GithubPrLocalProjectAvailability::NeedsUpdate { .. }
    ));
  }

  #[gpui::test]
  fn local_project_availability_reports_dirty_when_sha_mismatches_and_worktree_is_dirty(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(cx, Some(make_active_local_repo("stale-head", true)));
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

    let availability = page.read_with(cx, |this, cx| this.local_project_availability(cx));
    assert!(matches!(
      availability,
      GithubPrLocalProjectAvailability::Dirty { .. }
    ));
  }

  #[gpui::test]
  fn local_project_availability_requires_branch_switch_when_current_branch_differs(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(
        cx,
        Some(make_active_local_repo_for_branch("main", "head", false)),
      );
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

    let availability = page.read_with(cx, |this, cx| this.local_project_availability(cx));
    assert!(matches!(
      availability,
      GithubPrLocalProjectAvailability::NeedsBranchSwitch {
        current_branch: Some(ref branch),
        has_uncommitted_changes: false,
        ..
      } if branch == "main"
    ));
  }

  #[gpui::test]
  fn local_project_availability_requires_branch_switch_and_preserves_dirty_state(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(
        cx,
        Some(make_active_local_repo_for_branch("main", "head", true)),
      );
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

    let availability = page.read_with(cx, |this, cx| this.local_project_availability(cx));
    assert!(matches!(
      availability,
      GithubPrLocalProjectAvailability::NeedsBranchSwitch {
        current_branch: Some(ref branch),
        has_uncommitted_changes: true,
        ..
      } if branch == "main"
    ));
  }

  #[test]
  fn local_project_command_palette_commands_include_switch_to_pr_branch_only_when_available() {
    let commands = GithubPrDetailsPage::local_project_command_palette_commands(
      &GithubPrLocalProjectAvailability::NeedsBranchSwitch {
        repo_root: PathBuf::from("/tmp/reviu-repo"),
        current_branch: Some("main".to_string()),
        has_uncommitted_changes: true,
      },
    );
    assert_eq!(commands.len(), 1);
    assert_eq!(
      commands[0].id,
      ui::CommandPaletteCommandId::SwitchToPrBranch
    );

    assert!(
      GithubPrDetailsPage::local_project_command_palette_commands(
        &GithubPrLocalProjectAvailability::Hidden
      )
      .is_empty()
    );
    assert!(
      GithubPrDetailsPage::local_project_command_palette_commands(
        &GithubPrLocalProjectAvailability::Ready {
          repo_root: PathBuf::from("/tmp/reviu-repo"),
        }
      )
      .is_empty()
    );
    assert!(
      GithubPrDetailsPage::local_project_command_palette_commands(
        &GithubPrLocalProjectAvailability::NeedsUpdate {
          repo_root: PathBuf::from("/tmp/reviu-repo"),
        }
      )
      .is_empty()
    );
    assert!(
      GithubPrDetailsPage::local_project_command_palette_commands(
        &GithubPrLocalProjectAvailability::Dirty {
          repo_root: PathBuf::from("/tmp/reviu-repo"),
        }
      )
      .is_empty()
    );
  }

  #[gpui::test]
  fn command_palette_commands_prepend_switch_to_pr_branch_when_available(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(
        cx,
        Some(make_active_local_repo_for_branch("main", "head", false)),
      );
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

    let commands = page.read_with(cx, |this, cx| this.command_palette_commands(cx));
    assert_eq!(
      commands.first().map(|command| command.id),
      Some(ui::CommandPaletteCommandId::SwitchToPrBranch)
    );
  }

  #[test]
  fn find_matching_recent_local_repo_returns_recent_repo_matching_pr_source() {
    let db_path = unique_test_db_path("recent-repo-match");
    let _ = std::fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path.clone()));

    let (repo_root, snapshot) =
      create_local_repo_with_github_remote("acme", "widget", "feature", &[]);
    ConfigStore::persist_recent_repository(&repo_root);

    let matched =
      find_matching_recent_local_repo(&make_pr_details_for_stats(), None).expect("matching repo");
    assert_eq!(matched.repo_root, repo_root);
    assert_eq!(matched.github_owner.as_deref(), Some("acme"));
    assert_eq!(matched.github_repo.as_deref(), Some("widget"));
    assert_eq!(matched.current_branch, snapshot.current_branch);

    ConfigStore::set_test_db_path(None);
    let _ = std::fs::remove_file(&db_path);
    std::fs::remove_dir_all(&repo_root).ok();
  }

  #[gpui::test]
  fn local_project_availability_uses_resolved_recent_repo_when_active_repo_is_missing(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let (repo_root, snapshot) =
      create_local_repo_with_github_remote("acme", "widget", "main", &["feature"]);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_local_repo(
        snapshot.head_sha.as_deref().expect("snapshot head sha"),
        "feature",
      ));
      this.resolved_local_repo = Some(snapshot.clone());
      this.resolved_local_repo_scan_complete = true;
      cx.notify();
    });

    let availability = page.read_with(cx, |this, cx| this.local_project_availability(cx));
    assert!(matches!(
      availability,
      GithubPrLocalProjectAvailability::NeedsBranchSwitch {
        repo_root: ref resolved_repo_root,
        current_branch: Some(ref branch),
        has_uncommitted_changes: false,
      } if resolved_repo_root == &repo_root && branch == "main"
    ));

    std::fs::remove_dir_all(&repo_root).ok();
  }

  #[gpui::test]
  fn command_palette_commands_prepend_switch_when_resolved_recent_repo_needs_branch_switch(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let (repo_root, snapshot) =
      create_local_repo_with_github_remote("acme", "widget", "main", &["feature"]);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_local_repo(
        snapshot.head_sha.as_deref().expect("snapshot head sha"),
        "feature",
      ));
      this.resolved_local_repo = Some(snapshot.clone());
      this.resolved_local_repo_scan_complete = true;
      cx.notify();
    });

    let commands = page.read_with(cx, |this, cx| this.command_palette_commands(cx));
    assert_eq!(
      commands.first().map(|command| command.id),
      Some(ui::CommandPaletteCommandId::SwitchToPrBranch)
    );

    std::fs::remove_dir_all(&repo_root).ok();
  }

  #[gpui::test]
  async fn switch_to_pr_branch_palette_action_defers_before_switching_branch(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();

    let unique = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system time after unix epoch")
      .as_nanos();
    let repo_root = std::env::temp_dir().join(format!("reviu-pr-switch-palette-{unique}"));
    std::fs::create_dir_all(repo_root.join("src")).expect("create repo directories");
    Repository::init(&repo_root).expect("init repo");
    commit_local_project_file(
      &repo_root,
      Path::new("src/main.rs"),
      "fn main() {}\n",
      "initial",
    );

    let repo = Repository::open(&repo_root).expect("open repo");
    let head_commit = repo
      .head()
      .expect("repo head")
      .peel_to_commit()
      .expect("head commit");
    let current_branch = repo
      .head()
      .expect("repo head")
      .shorthand()
      .expect("current branch shorthand")
      .to_string();
    repo
      .branch("feature", &head_commit, false)
      .expect("create feature branch");
    let head_sha = head_commit.id().to_string();

    cx.update(|cx| {
      ActiveLocalRepoStore::set(
        cx,
        Some(ActiveLocalRepo {
          repo_root: repo_root.clone(),
          github_owner: Some("acme".to_string()),
          github_repo: Some("widget".to_string()),
          current_branch: Some(current_branch),
          head_sha: Some(head_sha.clone()),
          has_uncommitted_changes: false,
        }),
      );
    });

    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      cx.notify();
    });

    let (action_result, switch_task_is_scheduled_immediately) =
      page.update_in(cx, |this, window, cx| {
        let result =
          this.handle_command_palette_action(CommandPaletteAction::SwitchToPrBranch, window, cx);
        let switch_task_is_scheduled_immediately = this.local_branch_switch_task.is_some();
        (result, switch_task_is_scheduled_immediately)
      });
    assert!(action_result.is_ok());
    assert!(!switch_task_is_scheduled_immediately);

    cx.run_until_parked();

    let switch_task = page.update_in(cx, |this, _window, _cx| {
      assert!(
        this.local_branch_switch_loading,
        "branch switch should start after deferred command runs"
      );
      this
        .local_branch_switch_task
        .take()
        .expect("branch switch task should be present after deferred command runs")
    });
    switch_task.await;

    let switched_branch = Repository::open(&repo_root)
      .expect("reopen repo")
      .head()
      .expect("repo head after switch")
      .shorthand()
      .expect("branch shorthand after switch")
      .to_string();
    assert_eq!(switched_branch, "feature");

    let store_branch = page.read_with(cx, |_, cx| {
      ActiveLocalRepoStore::get(cx).and_then(|repo| repo.current_branch)
    });
    assert_eq!(store_branch.as_deref(), Some("feature"));

    let (switch_loading, switch_error) = page.read_with(cx, |this, _cx| {
      (
        this.local_branch_switch_loading,
        this.local_branch_switch_error.clone(),
      )
    });
    assert!(!switch_loading);
    assert!(switch_error.is_none());

    std::fs::remove_dir_all(&repo_root).ok();
  }

  #[gpui::test]
  fn overview_conflicts_action_returns_to_git_page_when_conflict_resolution_is_already_active(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/github/acme/widget/pull/42", cx);
    });

    let (repo_root, _) =
      create_local_repo_with_github_remote("acme", "widget", "feature", &["main"]);
    let rel_path = Path::new("src/main.rs");
    commit_local_project_file(
      &repo_root,
      rel_path,
      "fn main() {\n  println!(\"feature\");\n}\n",
      "feature change",
    );
    switch_to_branch_name(&repo_root, "main").expect("switch to main");
    commit_local_project_file(
      &repo_root,
      rel_path,
      "fn main() {\n  println!(\"main\");\n}\n",
      "main change",
    );
    switch_to_branch_name(&repo_root, "feature").expect("switch back to feature");

    let merge_result = merge_branch(
      &repo_root,
      &BranchRef {
        name: "main".to_string(),
        kind: BranchKind::Local,
      },
    );
    assert!(merge_result.is_err(), "merge should stop on conflicts");
    assert!(local_repo_has_active_conflict_resolution(&repo_root));

    let snapshot = local_repo_snapshot(&repo_root, None).expect("snapshot conflicted repo");
    let head_sha = snapshot
      .head_sha
      .clone()
      .expect("feature branch head should stay available");
    assert!(snapshot.has_uncommitted_changes);

    cx.update(|cx| {
      ActiveLocalRepoStore::set(cx, Some(snapshot));
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_local_repo(&head_sha, "feature"));
      this.merge_readiness = Some(make_merge_readiness_with_state(
        GithubPullRequestMergeReadinessStatus::Blocked,
        Some("dirty"),
        "This pull request has merge conflicts that must be resolved before it can be merged.",
      ));
      cx.notify();
    });

    page.update_in(cx, |this, window, cx| {
      this.open_overview_pr_alert_in_git_page(window, cx);
    });

    cx.update(|_, cx| {
      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/git");
    });

    std::fs::remove_dir_all(&repo_root).ok();
  }

  #[gpui::test]
  fn active_file_search_entries_include_pr_and_unchanged_local_files_when_mode_is_active(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(cx, Some(make_active_local_repo("head", false)));
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      this.show_local_project_files = true;
      this.file_lookup.insert(
        "src/pr_only.rs".to_string(),
        Rc::new(GithubPrFileDiff {
          path: "src/pr_only.rs".into(),
          old_path: None,
          status: GithubPrFileStatus::Modified,
        }),
      );
      this.local_project_lookup.insert(
        "src/local.rs".to_string(),
        Rc::new(GithubPrLocalProjectFile {
          path: "src/local.rs".into(),
        }),
      );
      this.local_project_lookup.insert(
        "README.md".to_string(),
        Rc::new(GithubPrLocalProjectFile {
          path: "README.md".into(),
        }),
      );
      cx.notify();
    });

    let labels = page.read_with(cx, |this, cx| {
      this
        .active_file_search_entries(cx)
        .into_iter()
        .map(|entry| entry.label.to_string())
        .collect::<Vec<_>>()
    });
    assert_eq!(
      labels,
      vec![
        "README.md".to_string(),
        "src/local.rs".to_string(),
        "src/pr_only.rs".to_string(),
      ]
    );
  }

  #[gpui::test]
  fn active_file_search_entries_hide_unchanged_local_files_when_mode_is_inactive(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.file_lookup.insert(
        "src/pr_only.rs".to_string(),
        Rc::new(GithubPrFileDiff {
          path: "src/pr_only.rs".into(),
          old_path: None,
          status: GithubPrFileStatus::Modified,
        }),
      );
      this.local_project_lookup.insert(
        "src/local.rs".to_string(),
        Rc::new(GithubPrLocalProjectFile {
          path: "src/local.rs".into(),
        }),
      );
      cx.notify();
    });

    let labels = page.read_with(cx, |this, cx| {
      this
        .active_file_search_entries(cx)
        .into_iter()
        .map(|entry| entry.label.to_string())
        .collect::<Vec<_>>()
    });
    assert_eq!(labels, vec!["src/pr_only.rs".to_string()]);
  }

  #[gpui::test]
  fn active_file_search_entries_follow_tree_text_search_matches(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(cx, Some(make_active_local_repo("head", false)));
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      this.show_local_project_files = true;
      this.tree_search_query = "needle".to_string();
      this.tree_search_matches = Some(HashSet::from([
        "README.md".to_string(),
        "src/pr_only.rs".to_string(),
      ]));
      this.file_lookup.insert(
        "src/pr_only.rs".to_string(),
        Rc::new(GithubPrFileDiff {
          path: "src/pr_only.rs".into(),
          old_path: None,
          status: GithubPrFileStatus::Modified,
        }),
      );
      this.local_project_lookup.insert(
        "src/local.rs".to_string(),
        Rc::new(GithubPrLocalProjectFile {
          path: "src/local.rs".into(),
        }),
      );
      this.local_project_lookup.insert(
        "README.md".to_string(),
        Rc::new(GithubPrLocalProjectFile {
          path: "README.md".into(),
        }),
      );
      cx.notify();
    });

    let labels = page.read_with(cx, |this, cx| {
      this
        .active_file_search_entries(cx)
        .into_iter()
        .map(|entry| entry.label.to_string())
        .collect::<Vec<_>>()
    });
    assert_eq!(
      labels,
      vec!["README.md".to_string(), "src/pr_only.rs".to_string(),]
    );
  }

  #[gpui::test]
  async fn refresh_tree_text_search_keeps_previous_matches_visible_while_loading(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.file_lookup.insert(
        "src/one.rs".to_string(),
        Rc::new(GithubPrFileDiff {
          path: "src/one.rs".into(),
          old_path: None,
          status: GithubPrFileStatus::Modified,
        }),
      );
      this.file_lookup.insert(
        "src/two.rs".to_string(),
        Rc::new(GithubPrFileDiff {
          path: "src/two.rs".into(),
          old_path: None,
          status: GithubPrFileStatus::Modified,
        }),
      );
      this.tree_search_query = "needle".to_string();
      this.tree_search_matches = Some(HashSet::from(["src/one.rs".to_string()]));

      this.refresh_tree_text_search(cx);

      assert!(this.tree_search_loading);
      assert_eq!(
        this.tree_search_matches,
        Some(HashSet::from(["src/one.rs".to_string()]))
      );
    });

    let search_task = page.update_in(cx, |this, _window, _cx| this.tree_search_task.take());
    if let Some(task) = search_task {
      task.await;
    }
  }

  #[test]
  fn perform_tree_text_search_matches_changed_pr_files_from_cached_contents() {
    let api = WorkspaceApi::new().api;
    let scope_paths = vec!["src/pr_only.rs".to_string(), "src/other.rs".to_string()];
    let pr_only = GithubPrFileDiff {
      path: "src/pr_only.rs".into(),
      old_path: None,
      status: GithubPrFileStatus::Modified,
    };
    let other = GithubPrFileDiff {
      path: "src/other.rs".into(),
      old_path: None,
      status: GithubPrFileStatus::Modified,
    };
    let pr_files = HashMap::from([
      ("src/pr_only.rs".to_string(), pr_only),
      ("src/other.rs".to_string(), other),
    ]);
    let cached_file_contents = HashMap::from([
      (
        "src/pr_only.rs".to_string(),
        GithubPrFileContents {
          base: Some("before\n".to_string()),
          head: Some("needle appears here\n".to_string()),
        },
      ),
      (
        "src/other.rs".to_string(),
        GithubPrFileContents {
          base: Some("before\n".to_string()),
          head: Some("different content\n".to_string()),
        },
      ),
    ]);

    let result = perform_tree_text_search(
      "needle",
      &scope_paths,
      &pr_files,
      &cached_file_contents,
      None,
      &api,
      None,
    );

    assert_eq!(
      result.matches,
      HashSet::from(["src/pr_only.rs".to_string(),])
    );
    assert!(result.updated_file_contents.is_empty());
  }

  #[test]
  fn perform_tree_text_search_matches_local_head_contents_and_skips_untracked() {
    let unique = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system time after unix epoch")
      .as_nanos();
    let repo_root = std::env::temp_dir().join(format!("reviu-pr-search-local-{unique}"));
    Repository::init(&repo_root).expect("init local project git repo");
    commit_local_project_file(
      &repo_root,
      Path::new("tracked.txt"),
      "needle in head\n",
      "initial tracked",
    );
    std::fs::write(repo_root.join("tracked.txt"), "worktree without match\n")
      .expect("write local worktree change");
    std::fs::create_dir_all(repo_root.join("scratch")).expect("create scratch dir");
    std::fs::write(
      repo_root.join("scratch/tmp.txt"),
      "needle only in untracked\n",
    )
    .expect("write untracked file");

    let api = WorkspaceApi::new().api;
    let result = perform_tree_text_search(
      "needle",
      &["tracked.txt".to_string(), "scratch/tmp.txt".to_string()],
      &HashMap::new(),
      &HashMap::new(),
      None,
      &api,
      Some(repo_root.as_path()),
    );

    assert_eq!(result.matches, HashSet::from(["tracked.txt".to_string(),]));
    assert!(result.updated_file_contents.is_empty());
    std::fs::remove_dir_all(&repo_root).ok();
  }

  #[gpui::test]
  fn selecting_visible_changed_file_prefers_pr_diff_over_local_copy(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(cx, Some(make_active_local_repo("head", false)));
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      this.show_local_project_files = true;
      this.file_lookup.insert(
        "src/shared.rs".to_string(),
        Rc::new(GithubPrFileDiff {
          path: "src/shared.rs".into(),
          old_path: None,
          status: GithubPrFileStatus::Modified,
        }),
      );
      this.local_project_lookup.insert(
        "src/shared.rs".to_string(),
        Rc::new(GithubPrLocalProjectFile {
          path: "src/shared.rs".into(),
        }),
      );
      this.file_contents.insert(
        "src/shared.rs".to_string(),
        GithubPrFileContents {
          base: Some("old\n".to_string()),
          head: Some("new\n".to_string()),
        },
      );
      this.select_visible_tree_path("src/shared.rs", cx);
    });

    let (selected_file, selected_local_project_file) = page.read_with(cx, |this, _cx| {
      (
        this
          .selected_file
          .as_ref()
          .map(|file| file.path.to_string()),
        this
          .selected_local_project_file
          .as_ref()
          .map(|file| file.path.to_string()),
      )
    });
    assert_eq!(selected_file.as_deref(), Some("src/shared.rs"));
    assert!(selected_local_project_file.is_none());
  }

  #[gpui::test]
  async fn loading_local_project_files_keeps_selected_pr_diff_visible(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      let file = Rc::new(GithubPrFileDiff {
        path: "src/main.rs".into(),
        old_path: None,
        status: GithubPrFileStatus::Modified,
      });
      this.show_local_project_files = true;
      this.file_lookup.insert(file.path.to_string(), file.clone());
      this.file_contents.insert(
        file.path.to_string(),
        GithubPrFileContents {
          base: Some("old contents\n".to_string()),
          head: Some("new contents\n".to_string()),
        },
      );
      this.set_selected_file(Some(file), cx);
      this.load_local_project_files(PathBuf::from("/tmp/reviu-tests/non-repo"), cx);
    });

    let after = page.read_with(cx, |this, cx| {
      let editor = this.diff_editor.read(cx);
      let document = editor.document().read(cx);
      document.slice_to_string(0..document.len())
    });
    assert_eq!(after, "new contents\n");

    let files_task = page.update_in(cx, |this, _window, _cx| {
      this.local_project_files_task.take()
    });
    if let Some(task) = files_task {
      task.await;
    }
  }

  #[gpui::test]
  async fn selecting_local_project_file_uses_detached_readonly_snapshot(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.executor().allow_parking();

    let unique = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system time after unix epoch")
      .as_nanos();
    let repo_root = std::env::temp_dir().join(format!("reviu-pr-local-project-{unique}"));
    let file_path = repo_root.join("src/local.rs");
    std::fs::create_dir_all(
      file_path
        .parent()
        .expect("local project file should have parent directory"),
    )
    .expect("create local project directory");
    Repository::init(&repo_root).expect("init local project git repo");
    commit_local_project_file(
      &repo_root,
      Path::new("src/local.rs"),
      "fn clean() {}\n",
      "initial",
    );
    std::fs::write(&file_path, "fn local_change() {}\n").expect("write local worktree change");

    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    let open_task = page.update_in(cx, |this, _window, cx| {
      this.local_project_loaded_repo_root = Some(repo_root.clone());
      this.set_selected_local_project_file(
        Some(Rc::new(GithubPrLocalProjectFile {
          path: "src/local.rs".into(),
        })),
        cx,
      );
      this
        .local_project_open_file_task
        .take()
        .expect("local project open task should exist")
    });
    open_task.await;

    let (repo_file_is_none, git_store_is_none, is_read_only, diffs_is_none, workdir_path, contents) =
      page.read_with(cx, |this, cx| {
        let editor = this.diff_editor.read(cx);
        let document = editor.document().read(cx);
        (
          editor.repo_file.is_none(),
          editor.git_store.is_none(),
          editor.is_read_only,
          editor.diffs.is_none(),
          editor.workdir_path.clone(),
          document.slice_to_string(0..document.len()),
        )
      });

    assert!(repo_file_is_none);
    assert!(git_store_is_none);
    assert!(is_read_only);
    assert!(diffs_is_none);
    assert_eq!(workdir_path, PathBuf::from("src/local.rs"));
    assert_eq!(contents, "fn clean() {}\n");

    std::fs::remove_dir_all(&repo_root).ok();
  }

  #[gpui::test]
  fn selecting_a_commit_disables_local_project_mode(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.update(|cx| {
      ActiveLocalRepoStore::set(cx, Some(make_active_local_repo("head", false)));
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      let file = Rc::new(GithubPrFileDiff {
        path: "src/main.rs".into(),
        old_path: None,
        status: GithubPrFileStatus::Modified,
      });
      this.file_lookup.insert(file.path.to_string(), file);
      this.show_local_project_files = true;
      this.selected_local_project_file = Some(Rc::new(GithubPrLocalProjectFile {
        path: "src/local.rs".into(),
      }));
      this.selected_local_project_tree_id = Some("src/local.rs".to_string());
      this.select_commit_filter(Some("commit-sha".to_string()), cx);
    });

    let (selected_commit_sha, show_local_project_files, selected_local_project_file) = page
      .read_with(cx, |this, _cx| {
        (
          this.selected_commit_sha.clone(),
          this.show_local_project_files,
          this.selected_local_project_file.clone(),
        )
      });
    assert_eq!(selected_commit_sha.as_deref(), Some("commit-sha"));
    assert!(!show_local_project_files);
    assert!(selected_local_project_file.is_none());
  }

  fn make_issue_comment(id: u64, created_at: &str, body: &str) -> GithubPullRequestIssueComment {
    GithubPullRequestIssueComment {
      id,
      body: body.to_string(),
      created_at: created_at.to_string(),
      updated_at: created_at.to_string(),
      user: Some(GithubPullRequestIssueCommentUser {
        login: "octocat".to_string(),
        avatar_url: None,
      }),
    }
  }

  fn make_review(
    id: u64,
    submitted_at: Option<&str>,
    state: GithubPullRequestReviewState,
    body: Option<&str>,
  ) -> GithubPullRequestReview {
    GithubPullRequestReview {
      id,
      body: body.map(str::to_string),
      state: state,
      submitted_at: submitted_at.map(str::to_string),
      commit_id: Some("1111111111111111111111111111111111111111".to_string()),
      html_url: "https://github.com/acme/widget/pull/42#pullrequestreview-1".to_string(),
      user: Some(crate::api::GithubPullRequestReviewUser {
        login: "reviewer".to_string(),
        avatar_url: None,
      }),
    }
  }

  fn make_review_comment(
    id: u64,
    created_at: &str,
    in_reply_to_id: Option<u64>,
  ) -> GithubPullRequestReviewComment {
    GithubPullRequestReviewComment {
      id,
      pull_request_review_id: Some(12),
      diff_hunk: "@@ -1 +1 @@".to_string(),
      path: "src/main.rs".to_string(),
      position: Some(1),
      original_position: Some(1),
      commit_id: "head123".to_string(),
      original_commit_id: "base123".to_string(),
      in_reply_to_id,
      user: GithubPullRequestReviewCommentUser {
        login: "octocat".to_string(),
        avatar_url: None,
      },
      body: "Looks good".to_string(),
      created_at: created_at.to_string(),
      updated_at: created_at.to_string(),
      start_line: None,
      original_start_line: None,
      start_side: None,
      line: Some(1),
      original_line: Some(1),
      side: Some("RIGHT".to_string()),
    }
  }

  #[test]
  fn overview_comment_update_result_review_preserves_comment() {
    let comment = make_review_comment(42, "2026-02-28T10:00:00Z", None);

    let result = OverviewCommentUpdateResult::review(comment);

    match result {
      OverviewCommentUpdateResult::Review(review) => {
        assert_eq!(review.id, 42);
        assert_eq!(review.path, "src/main.rs");
      }
      OverviewCommentUpdateResult::Issue(_) => panic!("expected review variant"),
    }
  }

  #[test]
  fn review_comment_preview_line_range_prefers_primary_fields() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.start_line = Some(8);
    comment.line = Some(11);
    comment.original_start_line = Some(2);
    comment.original_line = Some(4);

    assert_eq!(review_comment_preview_line_range(&comment), Some((8, 11)));
  }

  #[test]
  fn review_comment_preview_line_range_falls_back_to_original_fields() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.start_line = None;
    comment.line = None;
    comment.original_start_line = Some(14);
    comment.original_line = Some(16);

    assert_eq!(review_comment_preview_line_range(&comment), Some((14, 16)));
  }

  #[test]
  fn review_comment_preview_line_range_normalizes_order_and_rejects_non_positive_values() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.start_line = Some(21);
    comment.line = Some(19);
    assert_eq!(review_comment_preview_line_range(&comment), Some((19, 21)));

    comment.start_line = Some(0);
    comment.line = Some(-2);
    comment.original_start_line = Some(0);
    comment.original_line = Some(-1);
    assert_eq!(review_comment_preview_line_range(&comment), None);
  }

  #[test]
  fn review_comment_preview_side_explicit_left_maps_to_left() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.side = Some("LEFT".to_string());
    assert_eq!(
      review_comment_preview_side(&comment),
      ReviewCommentPreviewSide::Left
    );
  }

  #[test]
  fn review_comment_preview_side_unknown_value_defaults_to_right() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.side = Some("DIAGONAL".to_string());
    assert_eq!(
      review_comment_preview_side(&comment),
      ReviewCommentPreviewSide::Right
    );
  }

  #[test]
  fn review_comment_preview_side_missing_value_defaults_to_right() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.side = None;
    comment.start_side = None;
    assert_eq!(
      review_comment_preview_side(&comment),
      ReviewCommentPreviewSide::Right
    );
  }

  #[test]
  fn review_comment_targets_file_matches_renamed_old_path() {
    let file = GithubPrFileDiff {
      path: "src/new.rs".into(),
      old_path: Some("src/old.rs".into()),
      status: GithubPrFileStatus::Renamed,
    };
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.path = "src/old.rs".to_string();

    assert!(review_comment_targets_file(&comment, &file));
  }

  #[test]
  fn review_comment_to_editor_comment_returns_none_without_current_anchor() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.line = None;
    comment.start_line = None;
    comment.original_line = Some(4);
    comment.original_start_line = Some(4);

    let comments_by_id = HashMap::from([(comment.id, &comment)]);

    assert!(review_comment_to_editor_comment(&comment, &comments_by_id).is_none());
  }

  #[test]
  fn visible_review_comment_counts_by_path_ignores_unanchored_comments_and_maps_renames() {
    let files = files_from_api(vec![
      make_api_file("src/main.rs", "modified", None),
      make_api_file("src/new.rs", "renamed", Some("src/old.rs")),
    ]);
    let lookup: HashMap<String, Rc<GithubPrFileDiff>> = files
      .into_iter()
      .map(|file| (file.path.as_ref().to_string(), file))
      .collect();

    let mut renamed_comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    renamed_comment.path = "src/old.rs".to_string();
    renamed_comment.line = Some(3);

    let mut outdated_comment = make_review_comment(2, "2026-02-28T10:01:00Z", None);
    outdated_comment.path = "src/main.rs".to_string();
    outdated_comment.line = None;
    outdated_comment.start_line = None;
    outdated_comment.original_line = Some(7);
    outdated_comment.original_start_line = Some(7);

    let counts =
      visible_review_comment_counts_by_path(&lookup, &[renamed_comment, outdated_comment]);

    assert_eq!(counts.get("src/new.rs"), Some(&1));
    assert!(counts.get("src/main.rs").is_none());
  }

  #[test]
  fn review_comment_owned_by_login_is_case_insensitive() {
    let comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    assert!(review_comment_owned_by_login(&comment, "OCTOCAT"));
    assert!(!review_comment_owned_by_login(&comment, "hubot"));
  }

  #[test]
  fn issue_comment_owned_by_login_requires_user_and_matches_case_insensitively() {
    let mut comment = make_issue_comment(1, "2026-02-28T10:00:00Z", "Body");
    assert!(issue_comment_owned_by_login(&comment, "OCTOCAT"));
    comment.user = None;
    assert!(!issue_comment_owned_by_login(&comment, "octocat"));
  }

  #[test]
  fn next_pr_description_body_returns_none_when_value_is_unchanged_after_trim() {
    assert_eq!(
      next_pr_description_body("  Existing description  ", "Existing description"),
      None
    );
  }

  #[test]
  fn apply_pull_request_description_update_local_updates_body_and_updated_at() {
    let mut pull_request = make_pr_details_for_stats();
    let update = GithubPullRequestDescriptionUpdate {
      number: pull_request.number,
      body: Some("Updated description".to_string()),
      updated_at: "2026-03-01T11:10:00Z".to_string(),
    };

    apply_pull_request_description_update_local(&mut pull_request, update);
    assert_eq!(pull_request.body.as_deref(), Some("Updated description"));
    assert_eq!(pull_request.updated_at, "2026-03-01T11:10:00Z");
  }

  #[test]
  fn description_code_reference_requests_for_pull_request_recomputes_after_description_update() {
    let mut pull_request = make_pr_details_for_stats();
    pull_request.body =
      Some("[old](https://github.com/acme/widget/blob/main/src/old.rs#L2-L3)".to_string());
    let update = GithubPullRequestDescriptionUpdate {
      number: pull_request.number,
      body: Some("[new](https://github.com/acme/widget/blob/main/src/new.rs#L8-L9)".to_string()),
      updated_at: "2026-03-01T11:30:00Z".to_string(),
    };

    apply_pull_request_description_update_local(&mut pull_request, update);
    let requests =
      GithubPrDetailsPage::description_code_reference_requests_for_pull_request(&pull_request);

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "src/new.rs");
    assert_eq!(requests[0].start_line, 8);
    assert_eq!(requests[0].end_line, 9);
  }

  #[test]
  fn overview_root_review_comment_ids_collapses_threads_to_root_only() {
    let review_comments = vec![
      make_review_comment(1, "2026-02-28T10:00:00Z", None),
      make_review_comment(2, "2026-02-28T10:01:00Z", Some(1)),
      make_review_comment(3, "2026-02-28T10:02:00Z", Some(2)),
    ];

    let roots = overview_root_review_comment_ids(&review_comments);
    assert_eq!(roots, vec![1]);
    assert!(!roots.contains(&2));
    assert!(!roots.contains(&3));
  }

  #[test]
  fn overview_root_review_comment_ids_keeps_distinct_thread_roots() {
    let review_comments = vec![
      make_review_comment(1, "2026-02-28T10:00:00Z", None),
      make_review_comment(2, "2026-02-28T10:01:00Z", Some(1)),
      make_review_comment(10, "2026-02-28T10:02:00Z", None),
      make_review_comment(11, "2026-02-28T10:03:00Z", Some(10)),
    ];

    let roots = overview_root_review_comment_ids(&review_comments);
    assert_eq!(roots, vec![1, 10]);
  }

  #[test]
  fn overview_root_review_comment_ids_uses_orphan_reply_as_its_own_root() {
    let review_comments = vec![make_review_comment(7, "2026-02-28T10:00:00Z", Some(999))];
    let roots = overview_root_review_comment_ids(&review_comments);
    assert_eq!(roots, vec![7]);
  }

  #[test]
  fn allows_overview_review_reply_action_requires_review_kind_and_last_message() {
    let thread_ids = vec![10, 11, 12];
    assert!(allows_overview_review_reply_action(
      GithubPrOverviewConversationItemKind::ReviewComment,
      &thread_ids,
      12
    ));
    assert!(!allows_overview_review_reply_action(
      GithubPrOverviewConversationItemKind::ReviewComment,
      &thread_ids,
      11
    ));
    assert!(!allows_overview_review_reply_action(
      GithubPrOverviewConversationItemKind::IssueComment,
      &thread_ids,
      12
    ));
  }

  #[test]
  fn overview_root_is_editing_requires_a_real_root_target() {
    assert!(!overview_root_is_editing(None, None));
    assert!(!overview_root_is_editing(
      Some(OverviewCommentTarget {
        kind: OverviewCommentKind::Issue,
        id: 1,
      }),
      None,
    ));
    assert!(overview_root_is_editing(
      Some(OverviewCommentTarget {
        kind: OverviewCommentKind::Issue,
        id: 1,
      }),
      Some(OverviewCommentTarget {
        kind: OverviewCommentKind::Issue,
        id: 1,
      }),
    ));
  }

  #[test]
  fn checks_rollup_state_label_covers_all_variants() {
    assert_eq!(
      checks_rollup_state_label(GithubPullRequestChecksRollupState::Success),
      "Passing"
    );
    assert_eq!(
      checks_rollup_state_label(GithubPullRequestChecksRollupState::Pending),
      "Pending"
    );
    assert_eq!(
      checks_rollup_state_label(GithubPullRequestChecksRollupState::Failure),
      "Failing"
    );
  }

  #[test]
  fn upsert_issue_comment_local_updates_existing_and_appends_missing() {
    let mut comments = vec![make_issue_comment(1, "2026-02-28T10:00:00Z", "Initial")];
    let mut updated = make_issue_comment(1, "2026-02-28T10:01:00Z", "Updated");
    updated.updated_at = "2026-02-28T10:05:00Z".to_string();
    upsert_issue_comment_local(&mut comments, updated.clone());
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, updated.body);

    upsert_issue_comment_local(
      &mut comments,
      make_issue_comment(2, "2026-02-28T10:02:00Z", "Another"),
    );
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[1].id, 2);
  }

  #[test]
  fn upsert_review_comment_local_updates_existing_and_appends_missing() {
    let mut comments = vec![make_review_comment(1, "2026-02-28T10:00:00Z", None)];
    let mut updated = make_review_comment(1, "2026-02-28T10:01:00Z", None);
    updated.body = "Updated review comment".to_string();
    upsert_review_comment_local(&mut comments, updated.clone());
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, updated.body);

    upsert_review_comment_local(
      &mut comments,
      make_review_comment(2, "2026-02-28T10:02:00Z", Some(1)),
    );
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[1].id, 2);
  }

  #[test]
  fn remove_and_restore_issue_comment_local_supports_delete_rollback() {
    let mut comments = vec![
      make_issue_comment(1, "2026-02-28T10:00:00Z", "First"),
      make_issue_comment(2, "2026-02-28T10:01:00Z", "Second"),
    ];
    let (index, removed) = remove_issue_comment_local(&mut comments, 1).expect("removed");
    assert_eq!(index, 0);
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].id, 2);

    restore_issue_comment_local(&mut comments, index, removed);
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0].id, 1);
    assert_eq!(comments[1].id, 2);
  }

  #[test]
  fn remove_and_restore_review_comment_local_supports_delete_rollback() {
    let mut comments = vec![
      make_review_comment(1, "2026-02-28T10:00:00Z", None),
      make_review_comment(2, "2026-02-28T10:01:00Z", Some(1)),
    ];
    let (index, removed) = remove_review_comment_local(&mut comments, 1).expect("removed");
    assert_eq!(index, 0);
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].id, 2);

    restore_review_comment_local(&mut comments, index, removed);
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0].id, 1);
    assert_eq!(comments[1].id, 2);
  }

  #[test]
  fn github_blob_url_formats_single_line_anchor() {
    assert_eq!(
      github_blob_url("acme", "widget", "main", "src/main.rs", 7, 7),
      "https://github.com/acme/widget/blob/main/src/main.rs#L7"
    );
  }

  #[test]
  fn github_blob_url_formats_multi_line_anchor() {
    assert_eq!(
      github_blob_url("acme", "widget", "main", "src/main.rs", 7, 9),
      "https://github.com/acme/widget/blob/main/src/main.rs#L7-L9"
    );
  }

  fn make_pr_details_for_stats() -> GithubPullRequestDetails {
    GithubPullRequestDetails {
      node_id: "PR_kwDOExample".to_string(),
      number: 42,
      title: "Example PR".to_string(),
      state: GithubPullRequestState::Open,
      draft: false,
      created_at: "2026-02-28T10:00:00Z".to_string(),
      updated_at: "2026-02-28T10:00:00Z".to_string(),
      merged_at: None,
      merge_base_sha: "base".to_string(),
      base_sha: "base".to_string(),
      head_sha: "head".to_string(),
      base_ref_name: "main".to_string(),
      head_ref_name: "feature".to_string(),
      body: Some("Body".to_string()),
      author: crate::api::GithubPullRequestAuthor {
        login: "author".to_string(),
        avatar_url: None,
        is_bot: false,
      },
      comments: 10,
      review_comments: 11,
      commits: 3,
      additions: 20,
      deletions: 4,
      changed_files: 2,
      labels: Vec::new(),
      repository: GithubRepository {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
      },
      head_repository: Some(GithubRepository {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
      }),
    }
  }

  #[test]
  fn status_letter_covers_all_file_statuses() {
    assert_eq!(status_letter(GithubPrFileStatus::Added), "A");
    assert_eq!(status_letter(GithubPrFileStatus::Modified), "M");
    assert_eq!(status_letter(GithubPrFileStatus::Deleted), "D");
    assert_eq!(status_letter(GithubPrFileStatus::Renamed), "R");
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
      commit_subject("\n\nfeat: add filter\n\nbody details"),
      "feat: add filter"
    );
    assert_eq!(commit_subject(""), "No commit message");
  }

  #[test]
  fn review_decision_to_api_event_maps_all_variants() {
    assert_eq!(
      GithubPrDetailsPage::review_decision_to_api_event(GithubPrReviewDecision::Comment),
      GithubPullRequestReviewEvent::Comment
    );
    assert_eq!(
      GithubPrDetailsPage::review_decision_to_api_event(GithubPrReviewDecision::Approve),
      GithubPullRequestReviewEvent::Approve
    );
    assert_eq!(
      GithubPrDetailsPage::review_decision_to_api_event(GithubPrReviewDecision::RequestChanges),
      GithubPullRequestReviewEvent::RequestChanges
    );
  }

  #[test]
  fn validate_review_submission_requires_body_for_comment_and_request_changes() {
    assert!(
      GithubPrDetailsPage::validate_review_submission(GithubPrReviewDecision::Comment, "   ")
        .is_some()
    );
    assert!(
      GithubPrDetailsPage::validate_review_submission(GithubPrReviewDecision::RequestChanges, "")
        .is_some()
    );
  }

  #[test]
  fn validate_review_submission_allows_empty_body_for_approve() {
    assert!(
      GithubPrDetailsPage::validate_review_submission(GithubPrReviewDecision::Approve, "   ")
        .is_none()
    );
  }

  #[test]
  fn review_decision_defaults_to_comment() {
    assert_eq!(
      GithubPrReviewDecision::default(),
      GithubPrReviewDecision::Comment
    );
  }

  #[test]
  fn merge_method_label_covers_all_variants() {
    assert_eq!(
      merge_method_label(GithubPullRequestMergeMethod::Merge),
      "Create a merge commit"
    );
    assert_eq!(
      merge_method_label(GithubPullRequestMergeMethod::Squash),
      "Squash and merge"
    );
    assert_eq!(
      merge_method_label(GithubPullRequestMergeMethod::Rebase),
      "Rebase and merge"
    );
  }

  #[test]
  fn merge_method_supports_commit_message_hides_fields_for_rebase_only() {
    assert!(merge_method_supports_commit_message(
      GithubPullRequestMergeMethod::Merge
    ));
    assert!(merge_method_supports_commit_message(
      GithubPullRequestMergeMethod::Squash
    ));
    assert!(!merge_method_supports_commit_message(
      GithubPullRequestMergeMethod::Rebase
    ));
  }

  #[test]
  fn review_state_display_label_supports_commented_reviews() {
    assert_eq!(
      review_state_display_label(GithubPullRequestReviewState::Commented),
      "Commented"
    );
  }

  #[test]
  fn build_overview_conversation_items_hides_state_label_for_commented_reviews() {
    let reviews = vec![make_review(
      2,
      Some("2026-02-28T10:00:00Z"),
      GithubPullRequestReviewState::Commented,
      Some("Looks good to me"),
    )];

    let items = build_overview_conversation_items(&[], &reviews, &[]);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].body.as_deref(), Some("Looks good to me"));
    assert_eq!(items[0].review_state, None);
  }

  #[test]
  fn build_overview_conversation_items_orders_oldest_to_newest_across_sources() {
    let issue_comments = vec![make_issue_comment(
      1,
      "2026-02-28T11:00:00Z",
      "Issue comment",
    )];
    let reviews = vec![make_review(
      2,
      Some("2026-02-28T10:00:00Z"),
      GithubPullRequestReviewState::Approved,
      Some("Approved"),
    )];
    let review_comments = vec![make_review_comment(3, "2026-02-28T12:00:00Z", None)];

    let items = build_overview_conversation_items(&issue_comments, &reviews, &review_comments);
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].kind, GithubPrOverviewConversationItemKind::Review);
    assert_eq!(
      items[1].kind,
      GithubPrOverviewConversationItemKind::IssueComment
    );
    assert_eq!(
      items[2].kind,
      GithubPrOverviewConversationItemKind::ReviewComment
    );
  }

  #[test]
  fn build_overview_conversation_items_excludes_reviews_without_submitted_at() {
    let reviews = vec![
      make_review(
        2,
        Some("2026-02-28T10:00:00Z"),
        GithubPullRequestReviewState::Approved,
        Some("Posted"),
      ),
      make_review(
        3,
        None,
        GithubPullRequestReviewState::RequestChanges,
        Some("Waiting"),
      ),
    ];

    let items = build_overview_conversation_items(&[], &reviews, &[]);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, 2);
    assert_eq!(items[0].kind, GithubPrOverviewConversationItemKind::Review);
  }

  #[test]
  fn build_overview_conversation_items_keeps_review_comment_replies() {
    let review_comments = vec![
      make_review_comment(1, "2026-02-28T10:00:00Z", None),
      make_review_comment(2, "2026-02-28T10:01:00Z", Some(1)),
    ];

    let items = build_overview_conversation_items(&[], &[], &review_comments);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, 1);
    assert_eq!(items[0].replies.len(), 1);
    assert_eq!(items[0].replies[0].id, 2);
  }

  #[test]
  fn build_overview_conversation_items_groups_nested_reply_chains_on_root_comment() {
    let review_comments = vec![
      make_review_comment(1, "2026-02-28T10:00:00Z", None),
      make_review_comment(2, "2026-02-28T10:01:00Z", Some(1)),
      make_review_comment(3, "2026-02-28T10:02:00Z", Some(2)),
    ];

    let items = build_overview_conversation_items(&[], &[], &review_comments);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, 1);
    let reply_ids = items[0]
      .replies
      .iter()
      .map(|reply| reply.id)
      .collect::<Vec<_>>();
    assert_eq!(reply_ids, vec![2, 3]);
  }

  #[test]
  fn build_overview_conversation_items_does_not_render_review_body_when_missing() {
    let reviews = vec![make_review(
      1,
      Some("2026-02-28T10:00:00Z"),
      GithubPullRequestReviewState::RequestChanges,
      None,
    )];

    let items = build_overview_conversation_items(&[], &reviews, &[]);
    assert_eq!(items.len(), 1);
    assert!(items[0].body.is_none());
  }

  #[test]
  fn overview_change_stat_labels_are_compact() {
    let pr = make_pr_details_for_stats();
    let labels = overview_change_stat_labels(&pr);

    assert_eq!(labels, ["+20".to_string(), "-4".to_string()]);
  }

  #[test]
  fn build_commit_dropdown_items_includes_all_changes_first() {
    let commits = vec![make_api_commit(
      "1111111111111111111111111111111111111111",
      "feat: add filter",
      Some("2026-02-26T10:00:00Z"),
      Some("0000000000000000000000000000000000000000"),
    )];
    let items = GithubPrDetailsPage::build_commit_dropdown_items(&commits, None);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].value(), &None);
    assert!(items[0].selected());
    assert_eq!(
      items[1].value().as_deref(),
      Some("1111111111111111111111111111111111111111")
    );
    assert!(!items[1].selected());
  }

  #[test]
  fn build_commit_dropdown_items_marks_selected_commit() {
    let commits = vec![
      make_api_commit(
        "1111111111111111111111111111111111111111",
        "feat: add filter",
        Some("2026-02-26T10:00:00Z"),
        Some("0000000000000000000000000000000000000000"),
      ),
      make_api_commit(
        "2222222222222222222222222222222222222222",
        "fix: adjust theme colors",
        Some("2026-02-26T11:00:00Z"),
        Some("1111111111111111111111111111111111111111"),
      ),
    ];

    let items = GithubPrDetailsPage::build_commit_dropdown_items(
      &commits,
      Some("2222222222222222222222222222222222222222"),
    );

    assert_eq!(items.len(), 3);
    assert!(!items[0].selected());
    assert!(!items[1].selected());
    assert!(items[2].selected());
    assert_eq!(
      items[2].value().as_deref(),
      Some("2222222222222222222222222222222222222222")
    );
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
  fn commit_list_delegate_selects_matching_commit_sha() {
    let commits = vec![
      make_api_commit(
        "aaaaaaa111111111111111111111111111111111",
        "first",
        Some("2026-02-20T10:00:00Z"),
        Some("p1"),
      ),
      make_api_commit(
        "bbbbbbb222222222222222222222222222222222",
        "second",
        Some("2026-02-21T10:00:00Z"),
        Some("p2"),
      ),
    ];

    let mut delegate = GithubPrCommitListDelegate::new();
    delegate.set_rows(&commits, Some("bbbbbbb222222222222222222222222222222222"));

    assert_eq!(delegate.selected_index, Some(IndexPath::new(1)));
    assert_eq!(
      delegate
        .row_at(IndexPath::new(1))
        .map(|commit| commit.sha.clone()),
      Some("bbbbbbb222222222222222222222222222222222".to_string())
    );
  }

  #[test]
  fn commit_list_delegate_defaults_to_first_when_no_selected_commit() {
    let commits = vec![
      make_api_commit(
        "aaaaaaa111111111111111111111111111111111",
        "first",
        Some("2026-02-20T10:00:00Z"),
        Some("p1"),
      ),
      make_api_commit(
        "bbbbbbb222222222222222222222222222222222",
        "second",
        Some("2026-02-21T10:00:00Z"),
        Some("p2"),
      ),
    ];

    let mut delegate = GithubPrCommitListDelegate::new();
    delegate.set_rows(&commits, None);

    assert_eq!(delegate.selected_index, Some(IndexPath::new(0)));
  }

  #[test]
  fn commit_list_delegate_search_filters_by_message_sha_and_author() {
    let commits = vec![
      make_api_commit(
        "aaaaaaa111111111111111111111111111111111",
        "Fix parser regression",
        Some("2026-02-20T10:00:00Z"),
        Some("p1"),
      ),
      make_api_commit(
        "bbbbbbb222222222222222222222222222222222",
        "Refactor toolbar",
        Some("2026-02-21T10:00:00Z"),
        Some("p2"),
      ),
    ];

    let mut delegate = GithubPrCommitListDelegate::new();
    delegate.set_rows(&commits, None);

    delegate.prepare("parser");
    assert_eq!(delegate.matched_rows.len(), 1);
    assert_eq!(delegate.matched_rows[0].sha, commits[0].sha);

    delegate.prepare("bbbbbbb");
    assert_eq!(delegate.matched_rows.len(), 1);
    assert_eq!(delegate.matched_rows[0].sha, commits[1].sha);

    delegate.prepare("octocat");
    assert_eq!(delegate.matched_rows.len(), 2);
  }

  #[test]
  fn resolve_diff_shas_for_context_uses_commit_parent_when_selected() {
    let resolved = resolve_diff_shas_for_context(
      "merge123",
      "base123",
      "head123",
      Some("commit999"),
      Some("parent888"),
    );
    assert_eq!(
      resolved,
      Some(("parent888".to_string(), "commit999".to_string()))
    );
  }

  #[test]
  fn resolve_diff_shas_for_context_uses_merge_base_when_no_commit_selected() {
    let resolved = resolve_diff_shas_for_context("merge123", "base123", "head123", None, None);
    assert_eq!(
      resolved,
      Some(("merge123".to_string(), "head123".to_string()))
    );
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
  fn file_for_review_comment_path_prefers_direct_match() {
    let files = files_from_api(vec![make_api_file("src/main.rs", "modified", None)]);
    let lookup: HashMap<String, Rc<GithubPrFileDiff>> = files
      .into_iter()
      .map(|file| (file.path.as_ref().to_string(), file))
      .collect();

    let resolved = file_for_review_comment_path(&lookup, "src/main.rs");
    assert_eq!(
      resolved.as_ref().map(|file| file.path.as_ref()),
      Some("src/main.rs")
    );
  }

  #[test]
  fn file_for_review_comment_path_falls_back_to_renamed_old_path() {
    let files = files_from_api(vec![make_api_file(
      "src/new.rs",
      "renamed",
      Some("src/old.rs"),
    )]);
    let lookup: HashMap<String, Rc<GithubPrFileDiff>> = files
      .into_iter()
      .map(|file| (file.path.as_ref().to_string(), file))
      .collect();

    let resolved = file_for_review_comment_path(&lookup, "src/old.rs");
    assert_eq!(
      resolved.as_ref().map(|file| file.path.as_ref()),
      Some("src/new.rs")
    );
    assert!(file_for_review_comment_path(&lookup, "missing.rs").is_none());
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
  fn build_tree_items_empty_input_has_no_selection() {
    let (items, lookup, selected_index, selected_id) = build_tree_items(&[]);
    assert!(items.is_empty());
    assert!(lookup.is_empty());
    assert_eq!(selected_index, None);
    assert_eq!(selected_id, None);
  }

  #[test]
  fn left_sidebar_kind_routes_changes_to_files_and_other_tabs_to_context() {
    assert_eq!(
      left_sidebar_kind_for_tab(PR_TAB_OVERVIEW_IX),
      GithubPrLeftSidebarKind::Context
    );
    assert_eq!(
      left_sidebar_kind_for_tab(PR_TAB_CHANGES_IX),
      GithubPrLeftSidebarKind::Files
    );
    assert_eq!(
      left_sidebar_kind_for_tab(PR_TAB_CHECKS_IX),
      GithubPrLeftSidebarKind::Context
    );
  }

  #[test]
  fn build_tree_items_from_paths_expands_only_folders_with_changed_files() {
    let paths = vec![
      "src/changed.rs".to_string(),
      "src/nested/also_changed.rs".to_string(),
      "tests/helper.rs".to_string(),
      "README.md".to_string(),
    ];
    let expanded =
      expanded_folder_paths_for_changed_files(["src/changed.rs", "src/nested/also_changed.rs"]);

    let (items, selected_index, selected_id) = build_tree_items_from_paths(&paths, Some(&expanded));

    assert_eq!(items.len(), 3);
    assert_eq!(items[0].label.as_ref(), "src");
    assert!(items[0].is_expanded());
    assert_eq!(items[0].children[0].label.as_ref(), "nested");
    assert!(items[0].children[0].is_expanded());
    assert_eq!(items[1].label.as_ref(), "tests");
    assert!(!items[1].is_expanded());
    assert_eq!(items[2].label.as_ref(), "README.md");
    assert_eq!(selected_id.as_deref(), Some("src/nested/also_changed.rs"));
    assert_eq!(selected_index, Some(0));
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
  fn code_reference_requests_from_markdown_extracts_blob_links() {
    let body = "[compose](https://github.com/acme/widget/blob/main/docker-compose.yml#L7)";
    let references = code_reference_requests_from_markdown(body);
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].owner, "acme");
    assert_eq!(references[0].repo, "widget");
  }

  #[test]
  fn gfm_preview_from_review_preview_preserves_fields() {
    let preview = ReviewCommentCodeReferencePreview {
      url: Arc::from("https://github.com/acme/widget/blob/main/src/lib.rs#L1-L2"),
      repo: Arc::from("acme/widget"),
      path: Arc::from("src/lib.rs"),
      reference: Arc::from("main"),
      start_line: 1,
      end_line: 2,
      snippets: vec![Arc::from("fn main() {}")],
    };

    let converted = gfm_preview_from_review_preview(&preview);
    assert_eq!(converted.url, preview.url);
    assert_eq!(converted.repo, preview.repo);
    assert_eq!(converted.path, preview.path);
    assert_eq!(converted.reference, preview.reference);
    assert_eq!(converted.start_line, preview.start_line);
    assert_eq!(converted.end_line, preview.end_line);
    assert_eq!(converted.snippets, preview.snippets);
  }

  #[test]
  fn next_review_comment_navigation_index_handles_empty_list() {
    assert_eq!(
      next_review_comment_navigation_index(&[], None, ReviewCommentNavigationDirection::Next),
      None
    );
  }

  #[test]
  fn next_review_comment_navigation_index_uses_first_or_last_without_active_selection() {
    let comment_ids = [11, 22, 33];
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        None,
        ReviewCommentNavigationDirection::Next
      ),
      Some(0)
    );
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        None,
        ReviewCommentNavigationDirection::Previous
      ),
      Some(2)
    );
  }

  #[test]
  fn next_review_comment_navigation_index_wraps_in_both_directions() {
    let comment_ids = [11, 22, 33];
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        Some(33),
        ReviewCommentNavigationDirection::Next
      ),
      Some(0)
    );
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        Some(11),
        ReviewCommentNavigationDirection::Previous
      ),
      Some(2)
    );
  }

  #[test]
  fn next_review_comment_navigation_index_falls_back_when_active_comment_is_missing() {
    let comment_ids = [11, 22, 33];
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        Some(99),
        ReviewCommentNavigationDirection::Next
      ),
      Some(0)
    );
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        Some(99),
        ReviewCommentNavigationDirection::Previous
      ),
      Some(2)
    );
  }

  #[test]
  fn resolve_pr_back_target_defaults_to_github_home_when_repo_is_empty() {
    let target = resolve_pr_back_target("".into(), "".into());
    assert_eq!(target, GithubPrBackTarget::GithubHome);
  }

  #[test]
  fn resolve_pr_back_target_uses_repo_when_owner_and_repo_are_present() {
    let target = resolve_pr_back_target("acme".into(), "widget".into());
    assert_eq!(
      target,
      GithubPrBackTarget::Repo {
        owner: "acme".into(),
        repo: "widget".into(),
      }
    );
  }

  #[test]
  fn next_back_target_for_pr_palette_preserves_github_home() {
    let target = next_back_target_for_pr_palette(&GithubPrBackTarget::GithubHome);
    assert_eq!(target, GithubPrBackTarget::GithubHome);
  }

  #[test]
  fn next_back_target_for_pr_palette_preserves_repo_target() {
    let target = next_back_target_for_pr_palette(&GithubPrBackTarget::Repo {
      owner: "acme".into(),
      repo: "widget".into(),
    });
    assert_eq!(
      target,
      GithubPrBackTarget::Repo {
        owner: "acme".into(),
        repo: "widget".into(),
      }
    );
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
}
