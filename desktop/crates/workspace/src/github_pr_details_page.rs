use std::{
  collections::{BTreeMap, HashMap, HashSet},
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
use git::{DiffKind, DiffSet, FileDiff, compute_buffer_diff};
use gpui::{
  App, Context, Corner, Entity, FocusHandle, Focusable, MouseButton, ParentElement, Render,
  RenderImage, SharedString, Styled, Task, Window, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, IndexPath, Selectable, Sizable as _, StyledExt,
  avatar::Avatar,
  button::{Button, ButtonVariant, ButtonVariants as _},
  clipboard::Clipboard,
  h_flex,
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  radio::{Radio, RadioGroup},
  scroll::ScrollableElement,
  skeleton::Skeleton,
  spinner::Spinner,
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
  SearchFileEntry, SearchFileHandler, StatusTag, StatusThemeExt, UiIconName, WindowExt,
  dropdown_select, file_icon_path_for_name_with_theme, h_resizable, parse_github_url_action,
  resizable_panel,
};

use crate::{
  ShowCommandPalette, ShowFileSearch,
  api::{
    ApiClient, ApiError, GithubIssueDetailsComment, GithubPullRequestCheckRun,
    GithubPullRequestChecksRollupState, GithubPullRequestChecksSummary, GithubPullRequestCommit,
    GithubPullRequestDescriptionUpdate, GithubPullRequestDetails, GithubPullRequestFile,
    GithubPullRequestIssueComment, GithubPullRequestIssueCommentUser,
    GithubPullRequestLegacyStatus, GithubPullRequestMergeMethod, GithubPullRequestMergeReadiness,
    GithubPullRequestMergeReadinessStatus, GithubPullRequestMergeResult, GithubPullRequestReview,
    GithubPullRequestReviewComment, GithubPullRequestReviewEvent, GithubPullRequestReviewState,
    GithubPullRequestWorkflowJob, GithubPullRequestWorkflowRun, GithubPullRequestWorkflowStep,
  },
  auth_state::{AuthState, AuthStateStore},
  date_format::{format_compact_datetime, format_long_date},
  file_preview::{is_markdown_path, is_svg_path},
  file_search_palette::open_file_search_palette as open_shared_file_search_palette,
  github_navigation::{
    SamePrGfmNavigation, open_repo_target, same_pr_gfm_navigation, should_open_externally,
  },
  github_page::GithubPageHandle,
  github_repo_page::GithubRepoPageHandle,
  github_shared, sentry_context,
  workspace::{WorkspaceApi, WorkspacePage, WorkspaceRoute},
};

const SIDEBAR_DEFAULT_WIDTH: f32 = 400.0;
const SIDEBAR_MIN_WIDTH: f32 = 250.0;
const SIDEBAR_MAX_WIDTH: f32 = 1500.0;
const DIFF_HEADER_HEIGHT: f32 = 40.0;
const PR_TAB_OVERVIEW_IX: usize = 0;
const PR_TAB_CHANGES_IX: usize = 1;
const PR_TAB_COMMITS_IX: usize = 2;
const PR_TAB_CHECKS_IX: usize = 3;
const PR_COMMIT_SELECT_WIDTH: f32 = 260.0;
const PR_COMMIT_SELECT_MENU_WIDTH: f32 = 420.0;
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

type CommitSelectHandler = Rc<dyn Fn(Option<String>, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverviewCommentKind {
  Issue,
  Review,
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
      Some((UiIconName::CircleSlash, theme.status_red()))
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
    .child(checks_rollup_state_label(state))
    .into_any_element()
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

fn overview_conversation_kind_label(kind: GithubPrOverviewConversationItemKind) -> &'static str {
  match kind {
    GithubPrOverviewConversationItemKind::IssueComment => "Comment",
    GithubPrOverviewConversationItemKind::Review => "Review",
    GithubPrOverviewConversationItemKind::ReviewComment => "Review comment",
  }
}

fn overview_stats_badge_labels(pr: &GithubPullRequestDetails) -> Vec<String> {
  vec![
    format!("Commits {}", pr.commits),
    format!("Additions +{}", pr.additions),
    format!("Deletions -{}", pr.deletions),
    format!("Files changed {}", pr.changed_files),
  ]
}

fn overview_stats_badges(
  pr: &GithubPullRequestDetails,
  theme: &gpui_component::Theme,
) -> Vec<gpui::AnyElement> {
  let labels = overview_stats_badge_labels(pr);
  let colors = [
    theme.status_blue(),
    theme.status_green(),
    theme.status_red(),
    theme.status_orange(),
  ];
  labels
    .into_iter()
    .zip(colors)
    .map(|(label, color)| StatusTag::new(color).child(label).into_any_element())
    .collect()
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
  rows: Vec<Rc<GithubPullRequestCommit>>,
  selected_index: Option<IndexPath>,
}

impl GithubPrCommitListDelegate {
  fn new() -> Self {
    Self {
      rows: Vec::new(),
      selected_index: None,
    }
  }

  fn set_rows(&mut self, commits: &[GithubPullRequestCommit], selected_commit_sha: Option<&str>) {
    self.rows = commits.iter().cloned().map(Rc::new).collect();

    self.selected_index = selected_commit_sha
      .and_then(|selected_sha| {
        self
          .rows
          .iter()
          .position(|commit| commit.sha == selected_sha)
          .map(IndexPath::new)
      })
      .or_else(|| (!self.rows.is_empty()).then_some(IndexPath::new(0)));
  }

  fn row_at(&self, ix: IndexPath) -> Option<Rc<GithubPullRequestCommit>> {
    self.rows.get(ix.row).cloned()
  }
}

impl ListDelegate for GithubPrCommitListDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.rows.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = cx.theme().clone();
    let commit = self.rows.get(ix.row)?;
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
      .map(format_compact_datetime)
      .unwrap_or_else(|| "—".into());

    Some(
      ListItem::new(ix)
        .selected(Some(ix) == self.selected_index)
        .w_full()
        .rounded(theme.radius)
        .px_3()
        .py_2()
        .child(
          v_flex()
            .gap_1()
            .child(
              h_flex()
                .items_center()
                .gap_2()
                .child(
                  Tag::secondary()
                    .small()
                    .rounded_full()
                    .text_color(theme.muted_foreground)
                    .child(short),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .overflow_hidden()
                    .text_ellipsis_start()
                    .child(subject),
                ),
            )
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!("{author} • {date_label}")),
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
      .child("No commits")
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
  file: Option<Rc<GithubPrFileDiff>>,
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

type FileTreeBuildResult = (
  Vec<TreeItem>,
  HashMap<String, Rc<GithubPrFileDiff>>,
  Option<usize>,
  Option<String>,
);

fn build_tree_items(files: &[Rc<GithubPrFileDiff>]) -> FileTreeBuildResult {
  fn insert_node(
    map: &mut BTreeMap<String, FileTreeNode>,
    parts: &[&str],
    prefix: &str,
    file: Rc<GithubPrFileDiff>,
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
      node.file = Some(file);
      return;
    }

    let node_path = node.path.clone();
    insert_node(&mut node.children, tail, &node_path, file);
  }

  let mut root: BTreeMap<String, FileTreeNode> = BTreeMap::new();
  let mut file_lookup: HashMap<String, Rc<GithubPrFileDiff>> = HashMap::new();

  for file in files {
    let path = file.path.as_ref();
    file_lookup.insert(path.to_string(), file.clone());
    let parts: Vec<&str> = path.split('/').collect();
    insert_node(&mut root, &parts, "", file.clone());
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
    .map(|node| build_tree_item(node, &mut order, &mut first_file_id))
    .collect::<Vec<_>>();

  let selected_index = first_file_id
    .as_ref()
    .and_then(|id| order.iter().position(|candidate| candidate == id));

  (items, file_lookup, selected_index, first_file_id)
}

fn build_tree_item(
  node: FileTreeNode,
  order: &mut Vec<String>,
  first_file_id: &mut Option<String>,
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
      .map(|child| build_tree_item(child, order, first_file_id))
      .collect::<Vec<_>>();
    item = item.children(children).expanded(true);
  }

  order.push(node.path.clone());
  if node.file.is_some() && first_file_id.is_none() {
    *first_file_id = Some(node.path.clone());
  }

  item
}

pub struct GithubPrDetailsPage {
  focus_handle: FocusHandle,
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
  file_lookup: HashMap<String, Rc<GithubPrFileDiff>>,
  file_contents: HashMap<String, GithubPrFileContents>,
  file_content_tasks: HashMap<String, Task<()>>,
  selected_file: Option<Rc<GithubPrFileDiff>>,
  selected_tree_id: Option<String>,
  diff_editor: Entity<Editor>,
  diff_view: DiffViewMode,
  show_markdown_preview: bool,
  description_markdown_state: MarkdownRenderState,
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

  fn show_with_back_target_and_open_target(
    owner: SharedString,
    repo: SharedString,
    number: u64,
    back_target: GithubPrBackTarget,
    open_target: GithubPrOpenTarget,
    cx: &mut App,
  ) {
    if !AuthStateStore::has_pro_access(cx) {
      WorkspaceRoute::open_billing(cx);
      cx.refresh_windows();
      return;
    }

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

    WorkspaceRoute::global_mut(cx).page = WorkspacePage::GithubPrDetails;
    cx.refresh_windows();
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

    let mut this = Self {
      focus_handle: cx.focus_handle(),
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
      file_lookup: HashMap::new(),
      file_contents: HashMap::new(),
      file_content_tasks: HashMap::new(),
      selected_file: None,
      selected_tree_id: None,
      diff_editor,
      diff_view: DiffViewMode::Inline,
      show_markdown_preview: false,
      description_markdown_state: MarkdownRenderState::new(),
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

  fn current_open_target(&self) -> GithubPrOpenTarget {
    GithubPrOpenTarget::new(self.active_tab_ix == PR_TAB_CHANGES_IX, None)
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

  fn reload_current_pull_request(&mut self, cx: &mut Context<Self>) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };
    let active_tab_ix = self.active_tab_ix;
    let open_target = self.current_open_target();
    self.load_pull_request(context.owner, context.repo, context.number, open_target, cx);
    self.active_tab_ix = active_tab_ix;
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
            if AuthStateStore::has_pro_access(cx) {
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
        this.refocus_page_shortcuts_after_dropdown_select(window, cx);
      });
    })
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

  fn select_commit_filter(&mut self, selected_commit_sha: Option<String>, cx: &mut Context<Self>) {
    if self.selected_commit_sha == selected_commit_sha {
      return;
    }
    self.selected_commit_sha = selected_commit_sha;
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
            if AuthStateStore::has_pro_access(cx) {
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
      return false;
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

    window.open_dialog(cx, move |dialog, _, _| {
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
        .build(dialog)
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

    window.open_dialog(cx, move |dialog, _, _| {
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
        .build(dialog)
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

    if ix == PR_TAB_CHANGES_IX {
      self.sync_tree_selection(cx);
      self.focus_changes_tree(window, cx);
      cx.on_next_frame(window, |this, window, cx| {
        if this.active_tab_ix == PR_TAB_CHANGES_IX {
          this.focus_changes_tree(window, cx);
        }
      });
      return;
    }

    if ix == PR_TAB_COMMITS_IX {
      cx.on_next_frame(window, |this, window, cx| {
        this.commits_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
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
    self.svg_preview = None;
    self.svg_preview_source = None;

    if let Some(file) = selected {
      self.ensure_diff_editor_for_path(file.path.as_ref(), cx);
      self.sync_diff_view(cx);
      self.sync_tree_selection(cx);
      let key = file.path.to_string();
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
    self
      .selected_file
      .as_ref()
      .is_some_and(|file| self.split_disabled_for_file(file))
  }

  fn selected_file_is_markdown(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|file| is_markdown_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  fn selected_file_is_svg(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|file| is_svg_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
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
      .filter(|comment| comment.path == file.path)
      .filter_map(|comment| {
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

        let line = resolved_line.and_then(|value| {
          if value > 0 {
            Some(value as usize)
          } else {
            None
          }
        })?;
        let side = match resolved_side {
          Some("LEFT") => ReviewCommentSide::Left,
          _ => ReviewCommentSide::Right,
        };

        let side_label = resolved_side.unwrap_or("");
        let line_label = {
          let line_label = if let Some(start) = comment.start_line
            && let Some(end) = comment.line
            && start != end
          {
            Some(
              format!("L{}-{} {}", start, end, side_label)
                .trim()
                .to_string(),
            )
          } else {
            comment
              .line
              .or(comment.start_line)
              .or(resolved_line)
              .map(|value| format!("L{} {}", value, side_label).trim().to_string())
          };
          line_label.map(|label| Arc::from(label.as_str()))
        };

        Some(ReviewComment {
          id: comment.id,
          in_reply_to_id: comment.in_reply_to_id,
          line: line.saturating_sub(1),
          side,
          author: Arc::from(comment.user.login.as_str()),
          avatar_url: comment.user.avatar_url.as_deref().map(Arc::from),
          line_label,
          body: Arc::from(comment.body.as_str()),
          created_at: Arc::from(format_long_date(&comment.created_at).to_string()),
        })
      })
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
      cx.background_spawn(
        async move { renderer.render_single_frame(svg_bytes.as_slice(), 1.0, true) },
      );

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
            let (items, lookup, selected_index, selected_id) = build_tree_items(&files);
            this.file_lookup = lookup;
            this.selected_tree_id = selected_id.clone();
            this.tree_state.update(cx, |state, cx| {
              state.set_items(items, cx);
              state.set_selected_index(selected_index, cx);
            });
            let selected = selected_id.and_then(|id| this.file_lookup.get(&id).cloned());
            this.set_selected_file(selected, cx);
            this.prefetch_overview_root_review_comment_files(cx);
            this.add_pr_breadcrumb("Load PR files succeeded", Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            this.files_loading = false;
            this.files_error = Some(error_message.clone().into());
            this.tree_state.update(cx, |state, cx| {
              state.set_items(Vec::new(), cx);
            });
            this.file_lookup.clear();
            this.file_contents.clear();
            this.file_content_tasks.clear();
            this.selected_tree_id = None;
            this.set_selected_file(None, cx);
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
      self.maybe_fetch_file_contents(file, cx);
    }
  }

  fn maybe_fetch_file_contents(&mut self, file: Rc<GithubPrFileDiff>, cx: &mut Context<Self>) {
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
    self.file_lookup.clear();
    self.file_contents.clear();
    self.file_content_tasks.clear();
    self.selected_tree_id = None;
    self.set_selected_file(None, cx);
    self.diff_view = DiffViewMode::Inline;
    self.show_markdown_preview = false;
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
            this.error = None;
            this.add_pr_breadcrumb("Load PR details succeeded", Map::new());
            let description_requests = this.description_code_reference_requests.clone();
            this.schedule_code_reference_fetches(description_requests.iter(), cx);
            this.sync_review_comments(cx);
            this.maybe_fetch_selected_file_contents(cx);
            this.prefetch_overview_root_review_comment_files(cx);
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
    match &self.back_target {
      GithubPrBackTarget::GithubHome => {
        if AuthStateStore::has_pro_access(cx) {
          GithubPageHandle::refresh(cx);
          WorkspaceRoute::open_github(cx);
        } else {
          WorkspaceRoute::open_billing(cx);
        }
        cx.refresh_windows();
      }
      GithubPrBackTarget::Repo { owner, repo } => {
        GithubRepoPageHandle::show(owner.clone(), repo.clone(), cx);
      }
    }
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
          .icon(UiIconName::GitMerge)
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
      .child(Tab::new().label("Changes"))
      .child(Tab::new().label("Commits"))
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
        .child(status_tag)
        .child(format!("#{}", pr.number));

      h_flex()
        .items_center()
        .gap_2()
        .child(back_button())
        .child(div().flex().items_center().gap_3().child(title).child(meta))
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
          .child(
            div()
              .debug_selector(|| "github-pr-merge-button".to_string())
              .child(self.render_merge_popover(&theme, cx)),
          )
          .child(
            div()
              .debug_selector(|| "github-pr-review-button".to_string())
              .child(self.render_review_popover(&theme, cx)),
          )
      });

    div()
      .px_3()
      .py_2()
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

  fn render_details_conversation_panel(
    &self,
    pr: &GithubPullRequestDetails,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let items =
      build_overview_conversation_items(&self.issue_comments, &self.reviews, &self.review_comments);
    let is_loading =
      self.issue_comments_loading || self.reviews_loading || self.review_comments_loading;
    let has_errors = self.issue_comments_error.is_some()
      || self.reviews_error.is_some()
      || self.review_comments_error.is_some();
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

    let conversation_content = if items.is_empty() && is_loading {
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
        )
        .into_any_element()
    } else if items.is_empty() {
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
        )
        .into_any_element()
    } else {
      v_flex()
        .gap_4()
        .children(items.into_iter().map(|item| {
          let timestamp = format_compact_datetime(&item.timestamp);
          let scope_id = overview_conversation_scope_id(pr.number, item.kind, item.id);
          let parent_item_id = item.id;
          let type_label = overview_conversation_kind_label(item.kind);
          let body = item.body.clone();
          let replies = item.replies.clone();
          let thread_comment_ids = item.thread_comment_ids.clone();
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
          let root_is_editing = overview_root_is_editing(editing_target, root_target);
          let root_is_reply_target = replying_target == Some(item.id);
          let root_is_last_review_message =
            allows_overview_review_reply_action(item.kind, &thread_comment_ids, item.id);
          let root_is_editable = root_target.is_some_and(|target| match target.kind {
            OverviewCommentKind::Issue => editable_issue_comment_ids.contains(&target.id),
            OverviewCommentKind::Review => editable_review_comment_ids.contains(&target.id),
          });
          let review_comment_preview =
            if item.kind == GithubPrOverviewConversationItemKind::ReviewComment {
              self.overview_root_review_comment_preview(item.id)
            } else {
              None
            };

          let markdown_options = MarkdownRenderOptions::with_on_link(link_handler.clone())
            .with_state(self.description_markdown_state.clone())
            .with_github_issue_reference_context(
              pr.repository.owner.as_str(),
              pr.repository.repo.as_str(),
            )
            .with_scope_id(scope_id);

          let root_edit_button = if root_is_editable && !overview_submission_in_flight {
            root_target.map(|target| {
              let page = pr_page.clone();
              div()
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                  cx.stop_propagation();
                })
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

          let root_delete_button = if root_is_editable && !overview_submission_in_flight {
            root_target.map(|target| {
              let page = pr_page.clone();
              div()
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                  cx.stop_propagation();
                })
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

          let root_reply_button = if root_is_last_review_message
            && replying_target.is_none()
            && !overview_submission_in_flight
          {
            let page = pr_page.clone();
            Some(
              div()
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                  cx.stop_propagation();
                })
                .child(
                  Button::new(format!("pr-overview-comment-reply-{}", item.id))
                    .ghost()
                    .xsmall()
                    .compact()
                    .icon(UiIconName::MessageCircleReply)
                    .tooltip("Reply")
                    .on_click(move |_, window, cx| {
                      cx.stop_propagation();
                      page.update(cx, |this, cx| {
                        this.start_overview_review_comment_reply(item.id, window, cx);
                      });
                    }),
                )
                .into_any_element(),
            )
          } else {
            None
          };
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
          } else if let Some(body) = body {
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

          let root_reply_composer = if root_is_reply_target {
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
                github_shared::normalize_non_empty_text(input_state.read(cx).value().as_str())
                  .is_some();
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

          v_flex()
            .id(format!(
              "pr-overview-conversation-{}-{}",
              conversation_source_priority(item.kind),
              item.id
            ))
            .border_1()
            .border_color(theme.border)
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
                        .child(Tag::secondary().small().rounded_full().child(type_label)),
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
                .when_some(item.review_state, |this, state| {
                  let label = review_state_display_label(state);
                  let icon_style = review_state_icon_style(state, &theme);
                  this.child(
                    h_flex()
                      .items_center()
                      .gap_1()
                      .when_some(icon_style, |this, (icon, color)| {
                        this.child(Icon::new(icon).size_3().text_color(color))
                      })
                      .child(label),
                  )
                })
                .child(root_body)
                .when_some(root_reply_composer, |this, composer| this.child(composer))
                .when(!replies.is_empty(), |this| {
                  this.child(
                    v_flex()
                      .gap_2()
                      .pl_3()
                      .border_l_1()
                      .border_color(theme.border)
                      .children(replies.into_iter().map(|reply| {
                        let reply_timestamp = format_compact_datetime(&reply.timestamp);
                        let reply_scope_id = scope_id
                          .wrapping_mul(1_000_003)
                          .wrapping_add(reply.id as usize);
                        let reply_target = OverviewCommentTarget {
                          kind: OverviewCommentKind::Review,
                          id: reply.id,
                        };
                        let reply_is_editing = editing_target == Some(reply_target);
                        let reply_is_reply_target = replying_target == Some(reply.id);
                        let reply_is_editable = editable_review_comment_ids.contains(&reply.id);
                        let reply_is_last_message = allows_overview_review_reply_action(
                          item.kind,
                          &thread_comment_ids,
                          reply.id,
                        );
                        let reply_markdown_options =
                          MarkdownRenderOptions::with_on_link(link_handler.clone())
                            .with_state(self.description_markdown_state.clone())
                            .with_github_issue_reference_context(
                              pr.repository.owner.as_str(),
                              pr.repository.repo.as_str(),
                            )
                            .with_scope_id(reply_scope_id);

                        let reply_edit_button =
                          if reply_is_editable && !overview_submission_in_flight {
                            let page = pr_page.clone();
                            Some(
                              div()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                  cx.stop_propagation();
                                })
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

                        let reply_delete_button =
                          if reply_is_editable && !overview_submission_in_flight {
                            let page = pr_page.clone();
                            Some(
                              div()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                  cx.stop_propagation();
                                })
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
                                        this.confirm_overview_comment_delete(
                                          reply_target,
                                          window,
                                          cx,
                                        );
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
                          Some(
                            div()
                              .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                              })
                              .child(
                                Button::new(format!("pr-overview-reply-action-{}", reply.id))
                                  .ghost()
                                  .xsmall()
                                  .compact()
                                  .icon(UiIconName::MessageCircleReply)
                                  .tooltip("Reply")
                                  .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    page.update(cx, |this, cx| {
                                      this
                                        .start_overview_review_comment_reply(reply.id, window, cx);
                                    });
                                  }),
                              )
                              .into_any_element(),
                          )
                        } else {
                          None
                        };

                        let reply_body = if reply_is_editing {
                          if self.overview_edit_submitting {
                            v_flex()
                              .gap_2()
                              .child(Spinner::new().small())
                              .child(
                                div()
                                  .text_xs()
                                  .text_color(theme.muted_foreground)
                                  .child("Saving..."),
                              )
                              .into_any_element()
                          } else if let Some(input_state) = self.overview_edit_input.clone() {
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
                                this.child(
                                  div().text_xs().text_color(theme.status_red()).child(error),
                                )
                              })
                              .child(
                                h_flex()
                                  .items_center()
                                  .justify_end()
                                  .gap_2()
                                  .child(
                                    Button::new(format!(
                                      "pr-overview-reply-edit-cancel-{}",
                                      reply.id
                                    ))
                                    .ghost()
                                    .xsmall()
                                    .compact()
                                    .label("Cancel")
                                    .on_click(
                                      move |_, _, cx| {
                                        page_for_cancel.update(cx, |this, cx| {
                                          this.cancel_overview_comment_edit(cx);
                                        });
                                      },
                                    ),
                                  )
                                  .child(
                                    Button::new(format!(
                                      "pr-overview-reply-edit-save-{}",
                                      reply.id
                                    ))
                                    .xsmall()
                                    .compact()
                                    .label("Save")
                                    .disabled(!can_save || overview_submission_in_flight)
                                    .on_click(
                                      move |_, _, cx| {
                                        page_for_save.update(cx, |this, cx| {
                                          this.submit_overview_comment_edit(cx);
                                        });
                                      },
                                    ),
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

                        let reply_reply_composer = if reply_is_reply_target {
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
                                  this.child(
                                    div().text_xs().text_color(theme.status_red()).child(error),
                                  )
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
                                      .on_click(
                                        move |_, _, cx| {
                                          page_for_cancel.update(cx, |this, cx| {
                                            this.cancel_overview_review_comment_reply(cx);
                                          });
                                        },
                                      ),
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
                                      .on_click(
                                        move |_, _, cx| {
                                          page_for_save.update(cx, |this, cx| {
                                            this.submit_overview_review_comment_reply(cx);
                                          });
                                        },
                                      ),
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
                            parent_item_id, reply.id
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
                                  )
                                  .child(Tag::secondary().small().rounded_full().child("Reply")),
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
        }))
        .into_any_element()
    };

    let errors = v_flex()
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
      .when(has_errors, |this| this.child(errors))
      .child(conversation_content)
      .child(
        v_flex()
          .gap_2()
          .pt_2()
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
                  let page = pr_page.clone();
                  move |_, window, cx| {
                    page.update(cx, |this, cx| {
                      this.submit_overview_issue_comment_create(window, cx);
                    });
                  }
                }),
            ),
          ),
      )
  }

  fn render_details(
    &self,
    pr: &GithubPullRequestDetails,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let repo_label = github_shared::repo_label(&pr.repository.owner, &pr.repository.repo);
    let pr_url = github_shared::pr_url(&pr.repository.owner, &pr.repository.repo, pr.number);
    let repo_owner = pr.repository.owner.clone();
    let repo_name = pr.repository.repo.clone();
    let updated_at = format_long_date(&pr.updated_at);
    let created_at = format_long_date(&pr.created_at);
    let merged_at = pr.merged_at.as_deref().map(format_long_date);

    let body = pr
      .body
      .clone()
      .filter(|value| !value.trim().is_empty())
      .unwrap_or_else(|| "No description provided.".to_string());

    let stats_badges = h_flex()
      .gap_2()
      .flex_wrap()
      .children(overview_stats_badges(pr, &theme));

    let labels_row = if pr.labels.is_empty() {
      None
    } else {
      Some(
        h_flex()
          .gap_1()
          .flex_wrap()
          .children(pr.labels.iter().map(|label| {
            Tag::secondary()
              .small()
              .rounded_full()
              .child(label.name.clone())
          })),
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
          .gap_6()
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
      .child(stats_badges)
      .when_some(labels_row, |this, labels| this.child(labels))
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
                  .with_github_issue_reference_context(
                    pr.repository.owner.as_str(),
                    pr.repository.repo.as_str(),
                  )
                  .with_scope_id(pr_description_scope_id(pr.number));
                if let Some(previews) = description_previews.clone() {
                  options = options.with_github_code_reference_previews(previews);
                }
                render_markdown(body.as_str(), &options, cx)
              })
              .into_any_element()
          }),
      )
      .child(self.render_details_conversation_panel(pr, cx));

    div()
      .id("github-pr-overview-scroll")
      .size_full()
      .overflow_y_scrollbar()
      .child(
        div()
          .pt_10()
          .pb_32()
          .child(content)
          .mx_auto()
          .max_w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH)),
      )
  }

  fn render_files_sidebar(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let count = self.file_lookup.len();
    let commit_options =
      Self::build_commit_dropdown_items(&self.commits, self.selected_commit_sha.as_deref());
    let on_commit_select = self.commit_select_handler(cx);

    if let Some(selected_id) = self
      .tree_state
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string())
      && Some(selected_id.as_str()) != self.selected_tree_id.as_deref()
      && let Some(file) = self.file_lookup.get(&selected_id).cloned()
    {
      self.selected_tree_id = Some(selected_id.clone());
      cx.on_next_frame(window, move |this, _, cx| {
        this.set_selected_file(Some(file), cx);
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

    let mut comment_counts = HashMap::new();
    if self.selected_commit_sha.is_none() && !self.review_comments.is_empty() {
      for comment in &self.review_comments {
        *comment_counts.entry(comment.path.clone()).or_insert(0) += 1;
      }
    }

    let list = if self.files_loading {
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
    } else if self.files_error.is_some() {
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
        .child("No files changed")
        .into_any_element()
    } else {
      let view = cx.entity();
      tree(
        &self.tree_state,
        move |ix, entry, _selected, _window, cx| {
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
            let mut row = ListItem::new(ix)
              .w_full()
              .rounded(theme.radius)
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
                      .text_ellipsis_start()
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

            if !is_folder && this.file_lookup.contains_key(item.id.as_ref()) {
              let id = item.id.clone();
              row = row.on_click(cx.listener(move |this, _, _, cx| {
                if let Some(file) = this.file_lookup.get(id.as_ref()).cloned() {
                  this.set_selected_file(Some(file), cx);
                }
              }));
            }

            row
          })
        },
      )
      .flex_1()
      .w_full()
      .into_any_element()
    };

    v_flex()
      .bg(theme.sidebar)
      .size_full()
      .child(header)
      .child(div().flex_1().min_h_0().child(list))
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
    let split_disabled = self.split_disabled_for_file(file) || preview_active;
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
    let include_github = matches!(AuthStateStore::get(cx), AuthState::Authenticated(_));
    let commands = CommandPaletteCommand::default_global_commands(
      CommandPalettePage::GithubPrDetails,
      include_github,
    );

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, _window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, cx)
      })
    });

    let config = CommandPaletteConfig::new(Vec::new(), commands, handler);
    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    let palette_for_dialog = palette.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      dialog
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
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match action {
      CommandPaletteAction::OpenGitPage => {
        WorkspaceRoute::global_mut(cx).page = WorkspacePage::Git;
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGithubPage => {
        if AuthStateStore::has_pro_access(cx) {
          GithubPageHandle::refresh(cx);
          WorkspaceRoute::open_github(cx);
        } else {
          WorkspaceRoute::open_billing(cx);
        }
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
        open_changes_tab,
        review_comment_id,
      } => {
        if !AuthStateStore::has_pro_access(cx) {
          WorkspaceRoute::open_billing(cx);
          cx.refresh_windows();
          return Ok(());
        }

        self.back_target = next_back_target_for_pr_palette(&self.back_target);
        self.load_pull_request(
          owner,
          repo,
          number,
          GithubPrOpenTarget {
            open_changes_tab,
            review_comment_id,
          },
          cx,
        );
        WorkspaceRoute::global_mut(cx).page = WorkspacePage::GithubPrDetails;
        cx.refresh_windows();
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
        WorkspaceRoute::open_settings(cx);
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenBillingPage => {
        WorkspaceRoute::open_billing(cx);
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenAboutPage => {
        WorkspaceRoute::open_about(cx);
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGitConfigPage => {
        WorkspaceRoute::open_git_config(cx);
        cx.refresh_windows();
        Ok(())
      }
      _ => Err("Command not available.".into()),
    }
  }

  fn open_file_search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.file_lookup.is_empty() {
      return;
    }

    let entries = self
      .file_lookup
      .values()
      .map(|file| {
        let path = PathBuf::from(file.path.as_ref());
        let label = file.path.as_ref().replace(['\n', '\r'], "");
        SearchFileEntry::new(path, label)
      })
      .collect::<Vec<_>>();

    let view = cx.entity();
    let handler: SearchFileHandler = Arc::new(move |path, window, cx| {
      view.update(cx, |view, cx| {
        view.select_file_from_palette(&path, cx);
      });

      let view_for_focus = view.clone();
      window.on_next_frame(move |window, cx| {
        let focus_handle = view_for_focus
          .read(cx)
          .diff_editor
          .read(cx)
          .focus_handle(cx);
        window.focus(&focus_handle, cx);
      });

      Ok(())
    });
    open_shared_file_search_palette(window, cx, entries, handler, true);
  }

  fn select_file_from_palette(&mut self, path: &Path, cx: &mut Context<Self>) {
    let key = path.to_string_lossy().to_string();
    let Some(file) = self.file_lookup.get(&key).cloned() else {
      return;
    };

    let tree_item = TreeItem::new(key.clone(), key.clone());
    self.tree_state.update(cx, |state, cx| {
      state.set_selected_item(Some(&tree_item), cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
    });

    self.set_selected_file(Some(file), cx);
  }

  fn sync_tree_selection(&mut self, cx: &mut Context<Self>) {
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
      .map(format_compact_datetime)
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
                    .map(format_compact_datetime)
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
    let time_label = format_compact_datetime(
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
      .map(format_compact_datetime)
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
    let time_label = format_compact_datetime(status.updated_at.as_str());

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
        .gap_4()
        .child(
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
            .when(checks.requires_up_to_date_branch, |this| {
              this.child(div().text_xs().text_color(theme.muted_foreground).child(
                "The base branch rules require this pull request to be up to date before merging.",
              ))
            }),
        )
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

  fn render_commits_tab(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();

    let content: gpui::AnyElement = if self.commits_loading {
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
            .child("Loading commits..."),
        )
        .into_any_element()
    } else if let Some(error) = self.commits_error.as_ref() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(error.clone())
        .into_any_element()
    } else if self.commits.is_empty() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("No commits")
        .into_any_element()
    } else {
      let commits_list = List::new(&self.commits_list)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .flex_1()
        .min_h_0()
        .p(px(8.));

      v_flex()
        .w_full()
        .max_w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH))
        .h_full()
        .min_h_0()
        .mx_auto()
        .py_4()
        .child(commits_list)
        .into_any_element()
    };

    div()
      .id("github-pr-commits-scroll")
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
    let is_markdown = self.selected_file_is_markdown();
    let is_svg = self.selected_file_is_svg();
    let preview_active = self.show_markdown_preview && (is_markdown || is_svg);

    let editor_content: gpui::AnyElement = if self.files_loading {
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
    } else if self.file_loading {
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
    } else if self.file_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.file_error.clone().unwrap_or_default())
        .into_any_element()
    } else if self.files_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.files_error.clone().unwrap_or_default())
        .into_any_element()
    } else if self.selected_file.is_some() {
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
        div()
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
          .into_any_element()
      } else {
        div()
          .flex_1()
          .min_h_0()
          .child(self.diff_editor.clone())
          .into_any_element()
      }
    } else {
      div()
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
      .child(editor_content);

    h_resizable("github-pr-changes")
      .child(
        resizable_panel()
          .size(px(SIDEBAR_DEFAULT_WIDTH))
          .size_range(px(SIDEBAR_MIN_WIDTH)..px(SIDEBAR_MAX_WIDTH))
          .child(self.render_files_sidebar(window, cx)),
      )
      .child(resizable_panel().child(editor_panel))
  }
}

impl Render for GithubPrDetailsPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let overview_inner: gpui::AnyElement = if let Some(pr) = self.pull_request.as_ref() {
      self.render_details(pr, cx).into_any_element()
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

    let commits_content = div()
      .id("commits-tab")
      .flex_1()
      .min_h_0()
      .child(self.render_commits_tab(window, cx))
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
    } else if self.active_tab_ix == PR_TAB_COMMITS_IX {
      commits_content
    } else if self.active_tab_ix == PR_TAB_CHECKS_IX {
      checks_content
    } else {
      overview_content
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GithubPrDetailsPage::show_command_palette_action))
      .on_action(cx.listener(GithubPrDetailsPage::show_file_search_action))
      .on_action(cx.listener(GithubPrDetailsPage::find_action))
      .on_action(cx.listener(GithubPrDetailsPage::close_find_action))
      .child(self.render_header(cx))
      .child(v_flex().flex_1().min_h_0().child(content))
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
  use gpui::TestAppContext;

  fn init_gpui_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
      gpui_component::init(cx);
      if !cx.has_global::<WorkspaceApi>() {
        cx.set_global(WorkspaceApi::new());
      }
      if !cx.has_global::<AuthStateStore>() {
        cx.set_global(AuthStateStore::default());
      }
    });
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

    assert!(cx.debug_bounds("github-pr-merge-button").is_none());
    assert!(cx.debug_bounds("github-pr-review-button").is_none());
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
  fn format_long_date_returns_long_month_for_iso_values() {
    assert_eq!(
      format_long_date("2026-02-15T12:34:56Z").as_ref(),
      "February 15, 2026"
    );
    assert_eq!(format_long_date("2026-02-15").as_ref(), "2026-02-15");
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
  fn overview_stats_badge_labels_exclude_comments_counts() {
    let pr = make_pr_details_for_stats();
    let labels = overview_stats_badge_labels(&pr);

    assert_eq!(labels.len(), 4);
    assert!(labels.iter().all(|label| !label.starts_with("Comments ")));
    assert!(
      labels
        .iter()
        .all(|label| !label.starts_with("Review comments "))
    );
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
