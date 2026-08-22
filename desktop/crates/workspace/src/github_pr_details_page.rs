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
  ReviewCommentResolveHandler, ReviewCommentSide, ReviewCommentSuggestionActionFactory,
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
  date_format::{format_relative_time, parse_rfc3339},
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

fn pr_tab_url_segment(tab_ix: usize) -> &'static str {
  match tab_ix {
    PR_TAB_CHANGES_IX => "changes",
    _ => "", // overview = no suffix
  }
}

fn sorted_branch_names_for_target_selector(
  branches: Vec<GithubRepositoryBranch>,
  current_base: &str,
  head_ref: &str,
) -> Vec<String> {
  let mut names = branches
    .into_iter()
    .map(|branch| branch.name)
    .filter(|name| !name.trim().is_empty())
    .filter(|name| !name.eq_ignore_ascii_case(head_ref))
    .collect::<Vec<_>>();
  if !current_base.trim().is_empty()
    && !current_base.eq_ignore_ascii_case(head_ref)
    && !names
      .iter()
      .any(|name| name.eq_ignore_ascii_case(current_base))
  {
    names.push(current_base.to_string());
  }
  names.sort_by_key(|name| name.to_ascii_lowercase());
  names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
  names
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

fn build_target_branch_select_items(
  branches: Vec<GithubRepositoryBranch>,
  current_base: &str,
  head_ref: &str,
) -> Vec<PrTargetBranchSelectItem> {
  sorted_branch_names_for_target_selector(branches, current_base, head_ref)
    .into_iter()
    .map(PrTargetBranchSelectItem::new)
    .collect()
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

fn adjacent_pr_tab_ix(current: usize, direction: TabNavigationDirection) -> usize {
  const PR_TAB_COUNT: usize = 2;

  match direction {
    TabNavigationDirection::Previous => (current + PR_TAB_COUNT - 1) % PR_TAB_COUNT,
    TabNavigationDirection::Next => (current + 1) % PR_TAB_COUNT,
  }
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

fn render_image_preview_status_message(
  message: impl Into<SharedString>,
  color: Hsla,
) -> AnyElement {
  div()
    .w(px(280.0))
    .max_w_full()
    .px_3()
    .text_sm()
    .text_center()
    .whitespace_normal()
    .text_color(color)
    .child(message.into())
    .into_any_element()
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

fn code_reference_requests_from_markdown(markdown: &str) -> Vec<GithubBlobLineReference> {
  extract_github_blob_line_references(markdown)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubPrFileStatus {
  Added,
  Modified,
  Deleted,
  Renamed,
}

fn repo_status_for_pr_file(status: GithubPrFileStatus) -> RepoStatusKind {
  match status {
    GithubPrFileStatus::Added => RepoStatusKind::Added,
    GithubPrFileStatus::Modified => RepoStatusKind::Modified,
    GithubPrFileStatus::Deleted => RepoStatusKind::Deleted,
    GithubPrFileStatus::Renamed => RepoStatusKind::Renamed,
  }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommitNavigationDirection {
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

fn parse_github_commit_url(url: &str) -> Option<(String, String, String)> {
  let url = url.trim();
  let tail = url
    .strip_prefix("https://github.com/")
    .or_else(|| url.strip_prefix("http://github.com/"))
    .or_else(|| url.strip_prefix("github.com/"))?;
  let tail = tail
    .split('#')
    .next()
    .unwrap_or(tail)
    .split('?')
    .next()
    .unwrap_or(tail);

  let parts = tail
    .split('/')
    .map(str::trim)
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>();
  if parts.len() < 4 {
    return None;
  }

  let owner = parts[0].to_string();
  let repo = parts[1].to_string();
  let sha = match parts[2] {
    "commit" => parts.get(3)?,
    "pull" if parts.get(4).copied() == Some("commits") => parts.get(5)?,
    _ => return None,
  };

  Some((owner, repo, (*sha).to_string()))
}

fn resolve_same_pr_commit_link_sha(
  current_pr_context: Option<&CurrentPrContext>,
  commits: &[GithubPullRequestCommit],
  url: &str,
) -> Option<String> {
  let (owner, repo, linked_sha) = parse_github_commit_url(url)?;
  let context = current_pr_context?;
  if !context.owner.eq_ignore_ascii_case(&owner) || !context.repo.eq_ignore_ascii_case(&repo) {
    return None;
  }

  let linked_sha = linked_sha.trim();
  if linked_sha.is_empty() {
    return None;
  }

  if let Some(commit) = commits
    .iter()
    .find(|commit| commit.sha.eq_ignore_ascii_case(linked_sha))
  {
    return Some(commit.sha.clone());
  }

  let linked_sha = linked_sha.to_ascii_lowercase();
  let mut matches = commits.iter().filter(|commit| {
    commit
      .sha
      .to_ascii_lowercase()
      .starts_with(linked_sha.as_str())
  });
  let first_match = matches.next()?;
  if matches.next().is_some() {
    return None;
  }

  Some(first_match.sha.clone())
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
    suggestion_context: suggestion_context_from_review_comment(comment),
    created_at: Arc::from(format_relative_time(&comment.created_at).to_string()),
    thread_id: (!comment.thread_id.is_empty())
      .then(|| Arc::<str>::from(comment.thread_id.as_str())),
    is_resolved: comment.is_resolved,
    is_outdated: comment.is_outdated,
    viewer_can_resolve: comment.viewer_can_resolve,
    viewer_can_unresolve: comment.viewer_can_unresolve,
    is_pending: comment.is_pending,
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

fn reviewer_status_tooltip(status: ReviewerStatus, login: &str) -> SharedString {
  match status {
    ReviewerStatus::Awaiting => format!("Awaiting requested review from {login}").into(),
    ReviewerStatus::Approved => format!("{login} approved").into(),
    ReviewerStatus::Commented => format!("{login} left review comments").into(),
    ReviewerStatus::ChangesRequested => format!("{login} requested changes").into(),
  }
}

fn merged_reviewers(
  requested_reviewers: &[GithubPullRequestFilterOptionUser],
  reviews: &[GithubPullRequestReview],
  author_login: &str,
) -> Vec<GithubPullRequestFilterOptionUser> {
  let mut reviewers = Vec::new();
  let mut seen: HashSet<String> = HashSet::new();

  for reviewer in requested_reviewers {
    let key = reviewer.login.to_lowercase();
    if github_shared::logins_match_case_insensitive(reviewer.login.as_str(), author_login) {
      continue;
    }
    if seen.insert(key) {
      reviewers.push(reviewer.clone());
    }
  }

  for review in reviews {
    let Some(user) = review.user.as_ref() else {
      continue;
    };
    let key = user.login.to_lowercase();
    if github_shared::logins_match_case_insensitive(user.login.as_str(), author_login) {
      continue;
    }
    if seen.insert(key) {
      reviewers.push(GithubPullRequestFilterOptionUser {
        login: user.login.clone(),
        avatar_url: user.avatar_url.clone(),
      });
    }
  }

  reviewers
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OverviewCheckRow {
  id: String,
  state: GithubPullRequestChecksRollupState,
  title: String,
  status_label: Option<String>,
  app_label: Option<String>,
  app_slug: Option<String>,
  app_avatar_url: Option<String>,
  open_url: Option<String>,
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

fn non_empty_owned(value: &str) -> Option<String> {
  let value = value.trim();
  if value.is_empty() {
    None
  } else {
    Some(value.to_string())
  }
}

fn singular_or_plural(count: u64, singular: &'static str, plural: &'static str) -> &'static str {
  if count == 1 { singular } else { plural }
}

fn overview_checks_summary_title(checks: &GithubPullRequestChecksSummary) -> String {
  if checks.total_checks == 0 {
    return "No checks have run".to_string();
  }

  match checks.overall_state {
    GithubPullRequestChecksRollupState::Success => "All checks have passed".to_string(),
    GithubPullRequestChecksRollupState::Skipped => "All checks were skipped".to_string(),
    GithubPullRequestChecksRollupState::Pending => {
      if checks.pending_checks == 0 {
        "Checks are pending".to_string()
      } else {
        "Checks".to_string()
      }
    }
    GithubPullRequestChecksRollupState::Failure => {
      if checks.failed_checks == 0 {
        "Checks need attention".to_string()
      } else {
        "Checks".to_string()
      }
    }
  }
}

fn overview_checks_summary_subtitle(checks: &GithubPullRequestChecksSummary) -> String {
  if checks.total_checks == 0 {
    return "No checks reported".to_string();
  }

  let mut parts = Vec::new();
  if checks.failed_checks > 0 {
    parts.push(format!("{} failing", checks.failed_checks));
  }
  if checks.pending_checks > 0 {
    parts.push(format!("{} pending", checks.pending_checks));
  }
  if checks.skipped_checks > 0 {
    parts.push(format!("{} skipped", checks.skipped_checks));
  }
  if checks.successful_checks > 0 {
    parts.push(format!(
      "{} successful {}",
      checks.successful_checks,
      singular_or_plural(checks.successful_checks, "check", "checks")
    ));
  }

  parts.join(", ")
}

fn overview_checks_uniform_state(
  checks: &GithubPullRequestChecksSummary,
) -> Option<GithubPullRequestChecksRollupState> {
  if checks.total_checks == 0 {
    return None;
  }

  if checks.successful_checks == checks.total_checks {
    Some(GithubPullRequestChecksRollupState::Success)
  } else if checks.pending_checks == checks.total_checks {
    Some(GithubPullRequestChecksRollupState::Pending)
  } else if checks.failed_checks == checks.total_checks {
    Some(GithubPullRequestChecksRollupState::Failure)
  } else if checks.skipped_checks == checks.total_checks {
    Some(GithubPullRequestChecksRollupState::Skipped)
  } else {
    None
  }
}

fn overview_checks_summary_slices(
  checks: &GithubPullRequestChecksSummary,
  theme: &gpui_component::Theme,
) -> Vec<OverviewChecksSummarySlice> {
  let mut slices = Vec::new();

  if checks.failed_checks > 0 {
    slices.push(OverviewChecksSummarySlice {
      value: checks.failed_checks as f32,
      color: theme.status_red(),
    });
  }

  if checks.pending_checks > 0 {
    slices.push(OverviewChecksSummarySlice {
      value: checks.pending_checks as f32,
      color: theme.status_orange(),
    });
  }

  if checks.skipped_checks > 0 {
    slices.push(OverviewChecksSummarySlice {
      value: checks.skipped_checks as f32,
      color: theme.status_gray(),
    });
  }

  if checks.successful_checks > 0 {
    slices.push(OverviewChecksSummarySlice {
      value: checks.successful_checks as f32,
      color: theme.status_green(),
    });
  }

  if slices.is_empty() {
    slices.push(OverviewChecksSummarySlice {
      value: 1.0,
      color: theme.muted_foreground.opacity(0.3),
    });
  }

  slices
}

fn overview_checks_summary_segments(
  slices: &[OverviewChecksSummarySlice],
) -> Vec<OverviewChecksSummarySegment> {
  let total: f32 = slices.iter().map(|slice| slice.value.max(0.0)).sum();
  if total <= 0.0 {
    return Vec::new();
  }

  let mut start_angle = 0.0;
  let mut segments = Vec::new();

  for slice in slices {
    let span = (slice.value.max(0.0) / total) * std::f32::consts::TAU;
    if span <= 0.0 {
      continue;
    }

    let half_gap = if slices.len() > 1 {
      (OVERVIEW_CHECKS_SUMMARY_RING_GAP_ANGLE / 2.0).min(span / 4.0)
    } else {
      0.0
    };
    let segment_start_angle = start_angle + half_gap;
    let segment_end_angle = start_angle + span - half_gap;
    if segment_end_angle > segment_start_angle {
      segments.push(OverviewChecksSummarySegment {
        start_angle: segment_start_angle,
        end_angle: segment_end_angle,
        color: slice.color,
      });
    }

    start_angle += span;
  }

  segments
}

fn overview_checks_summary_caps(
  segments: &[OverviewChecksSummarySegment],
) -> Vec<OverviewChecksSummaryCap> {
  if segments.len() <= 1 {
    return Vec::new();
  }

  let center = OVERVIEW_CHECKS_SUMMARY_RING_SIZE / 2.0;
  let mut caps = Vec::new();

  for segment in segments {
    for angle in [segment.start_angle, segment.end_angle] {
      let visual_angle = angle - std::f32::consts::FRAC_PI_2;
      let x = center + OVERVIEW_CHECKS_SUMMARY_RING_RADIUS * visual_angle.cos();
      let y = center + OVERVIEW_CHECKS_SUMMARY_RING_RADIUS * visual_angle.sin();
      caps.push(OverviewChecksSummaryCap {
        left: x - OVERVIEW_CHECKS_SUMMARY_RING_STROKE_WIDTH / 2.0,
        top: y - OVERVIEW_CHECKS_SUMMARY_RING_STROKE_WIDTH / 2.0,
        size: OVERVIEW_CHECKS_SUMMARY_RING_STROKE_WIDTH,
        color: segment.color,
      });
    }
  }

  caps
}

fn format_overview_check_duration(total_seconds: u64) -> String {
  if total_seconds < 60 {
    return format!("{total_seconds}s");
  }

  let total_minutes = total_seconds / 60;
  let seconds = total_seconds % 60;
  if total_minutes < 60 {
    return if seconds == 0 {
      format!("{total_minutes}m")
    } else {
      format!("{total_minutes}m {seconds}s")
    };
  }

  let total_hours = total_minutes / 60;
  let minutes = total_minutes % 60;
  if total_hours < 24 {
    return if minutes == 0 {
      format!("{total_hours}h")
    } else {
      format!("{total_hours}h {minutes}m")
    };
  }

  let days = total_hours / 24;
  let hours = total_hours % 24;
  if hours == 0 {
    format!("{days}d")
  } else {
    format!("{days}d {hours}h")
  }
}

fn overview_check_duration_label(
  started_at: Option<&str>,
  finished_at: Option<&str>,
  state: GithubPullRequestChecksRollupState,
) -> Option<String> {
  let started_at = parse_rfc3339(started_at?)?;
  let finished_at = finished_at.and_then(parse_rfc3339).or_else(|| {
    (state == GithubPullRequestChecksRollupState::Pending).then(time::OffsetDateTime::now_utc)
  })?;
  let elapsed_seconds = (finished_at - started_at).whole_seconds();
  if elapsed_seconds < 0 {
    return None;
  }

  Some(format_overview_check_duration(elapsed_seconds as u64))
}

fn overview_check_status_label(
  state: GithubPullRequestChecksRollupState,
  started_at: Option<&str>,
  finished_at: Option<&str>,
) -> Option<String> {
  match state {
    GithubPullRequestChecksRollupState::Success => Some(
      overview_check_duration_label(started_at, finished_at, state)
        .map(|d| format!("Successful in {d}"))
        .unwrap_or_else(|| "Successful".to_string()),
    ),
    GithubPullRequestChecksRollupState::Failure => Some(
      overview_check_duration_label(started_at, finished_at, state)
        .map(|d| format!("Failed in {d}"))
        .unwrap_or_else(|| "Failed".to_string()),
    ),
    GithubPullRequestChecksRollupState::Skipped => Some(
      finished_at
        .or(started_at)
        .map(|value| format!("Skipped {}", format_relative_time(value)))
        .unwrap_or_else(|| "Skipped".to_string()),
    ),
    GithubPullRequestChecksRollupState::Pending => Some(
      overview_check_duration_label(started_at, finished_at, state)
        .map(|d| format!("In progress - {d}"))
        .unwrap_or_else(|| "In progress".to_string()),
    ),
  }
}

fn overview_check_state_sort_key(row: &OverviewCheckRow) -> u8 {
  match row.state {
    GithubPullRequestChecksRollupState::Failure => 0,
    GithubPullRequestChecksRollupState::Pending => 1,
    GithubPullRequestChecksRollupState::Skipped => 2,
    GithubPullRequestChecksRollupState::Success => 3,
  }
}

fn overview_check_rows(checks: &GithubPullRequestChecksSummary) -> Vec<OverviewCheckRow> {
  let mut rows = Vec::new();

  for (ix, context) in checks.missing_required_contexts.iter().enumerate() {
    rows.push(OverviewCheckRow {
      id: format!("missing-required-context-{ix}"),
      state: GithubPullRequestChecksRollupState::Pending,
      title: context.clone(),
      status_label: Some("Required check has not reported yet".to_string()),
      app_label: None,
      app_slug: None,
      app_avatar_url: None,
      open_url: None,
    });
  }

  for run in &checks.actions_runs {
    let run_name = run
      .name
      .as_deref()
      .and_then(non_empty_owned)
      .unwrap_or_else(|| "GitHub Actions".to_string());
    let event_suffix = non_empty_owned(&run.event).map(|event| format!(" ({event})"));
    let run_started_at = run
      .run_started_at
      .as_deref()
      .or(Some(run.created_at.as_str()));
    let run_finished_at =
      (run.state != GithubPullRequestChecksRollupState::Pending).then_some(run.updated_at.as_str());

    if run.jobs.is_empty() {
      let title = match event_suffix.as_deref() {
        Some(suffix) => format!("{run_name}{suffix}"),
        None => run_name.clone(),
      };
      rows.push(OverviewCheckRow {
        id: format!("workflow-run-{}", run.id),
        state: run.state,
        title,
        status_label: overview_check_status_label(run.state, run_started_at, run_finished_at),
        app_label: Some("GitHub Actions".to_string()),
        app_slug: Some("github-actions".to_string()),
        app_avatar_url: None,
        open_url: run.html_url.clone(),
      });
      continue;
    }

    for job in &run.jobs {
      let job_name = non_empty_owned(&job.name).unwrap_or_else(|| run_name.clone());
      let title = match event_suffix.as_deref() {
        Some(suffix) => format!("{run_name} / {job_name}{suffix}"),
        None => format!("{run_name} / {job_name}"),
      };
      let job_started_at = job.started_at.as_deref().or(run_started_at);
      let job_finished_at = if job.state == GithubPullRequestChecksRollupState::Pending {
        None
      } else {
        job.completed_at.as_deref().or(run_finished_at)
      };

      rows.push(OverviewCheckRow {
        id: format!("workflow-job-{}", job.id),
        state: job.state,
        title,
        status_label: overview_check_status_label(job.state, job_started_at, job_finished_at),
        app_label: job
          .app_name
          .as_deref()
          .and_then(non_empty_owned)
          .or_else(|| Some("GitHub Actions".to_string())),
        app_slug: job
          .app_slug
          .as_deref()
          .and_then(non_empty_owned)
          .or_else(|| Some("github-actions".to_string())),
        app_avatar_url: job.app_avatar_url.as_deref().and_then(non_empty_owned),
        open_url: job.html_url.clone().or_else(|| run.html_url.clone()),
      });
    }
  }

  for check in &checks.other_checks {
    let title = non_empty_owned(&check.name).unwrap_or_else(|| "Check run".to_string());
    let finished_at = (check.state != GithubPullRequestChecksRollupState::Pending)
      .then_some(check.completed_at.as_deref())
      .flatten();

    rows.push(OverviewCheckRow {
      id: format!("check-run-{}", check.id),
      state: check.state,
      title,
      status_label: overview_check_status_label(
        check.state,
        check.started_at.as_deref(),
        finished_at,
      ),
      app_label: check
        .app_name
        .as_deref()
        .and_then(non_empty_owned)
        .or_else(|| check.app_slug.as_deref().and_then(non_empty_owned)),
      app_slug: check.app_slug.as_deref().and_then(non_empty_owned),
      app_avatar_url: check.app_avatar_url.as_deref().and_then(non_empty_owned),
      open_url: check.details_url.clone().or_else(|| check.html_url.clone()),
    });
  }

  for status in &checks.legacy_statuses {
    let title = non_empty_owned(&status.context).unwrap_or_else(|| "Status check".to_string());
    let finished_at = (status.state != GithubPullRequestChecksRollupState::Pending)
      .then_some(status.updated_at.as_str());
    rows.push(OverviewCheckRow {
      id: format!("legacy-status-{}", status.id),
      state: status.state,
      title,
      status_label: overview_check_status_label(
        status.state,
        Some(status.created_at.as_str()),
        finished_at,
      ),
      app_label: None,
      app_slug: None,
      app_avatar_url: status.avatar_url.as_deref().and_then(non_empty_owned),
      open_url: status.target_url.clone(),
    });
  }

  rows.sort_by_key(overview_check_state_sort_key);
  rows
}

fn overview_check_app_initial(row: &OverviewCheckRow) -> String {
  row
    .app_label
    .as_deref()
    .or(row.app_slug.as_deref())
    .unwrap_or(row.title.as_str())
    .chars()
    .next()
    .map(|c| c.to_uppercase().collect::<String>())
    .filter(|initial| !initial.is_empty())
    .unwrap_or_else(|| "C".to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OverviewPrAlertKind {
  Conflicts,
  OutOfDate,
  Blocked,
}

fn overview_pr_alert_kind(
  merge_readiness: Option<&GithubPullRequestMergeReadiness>,
  checks: Option<&GithubPullRequestChecksSummary>,
) -> Option<OverviewPrAlertKind> {
  if let Some(readiness) = merge_readiness {
    match readiness
      .mergeable_state
      .as_deref()
      .map(str::trim)
      .map(str::to_ascii_lowercase)
      .as_deref()
    {
      Some("dirty") => return Some(OverviewPrAlertKind::Conflicts),
      Some("behind") => return Some(OverviewPrAlertKind::OutOfDate),
      _ => {}
    }

    if matches!(
      readiness.status,
      GithubPullRequestMergeReadinessStatus::Blocked
    ) {
      return Some(OverviewPrAlertKind::Blocked);
    }
  }

  if checks.is_some_and(|checks| checks.requires_up_to_date_branch) {
    return Some(OverviewPrAlertKind::OutOfDate);
  }

  None
}

#[derive(Clone, Debug)]
struct OverviewReviewStatusInfo {
  title: &'static str,
  message: String,
}

fn overview_review_status_info(
  merge_readiness: Option<&GithubPullRequestMergeReadiness>,
  requested_reviewers: &[GithubPullRequestFilterOptionUser],
  reviews: &[GithubPullRequestReview],
  author_login: &str,
) -> Option<OverviewReviewStatusInfo> {
  // Branch protection explicitly blocks the merge for review requirements.
  if let Some(readiness) = merge_readiness
    && matches!(
      readiness.status,
      GithubPullRequestMergeReadinessStatus::Blocked
    )
  {
    let state = readiness
      .mergeable_state
      .as_deref()
      .map(str::trim)
      .map(str::to_ascii_lowercase);
    if !matches!(state.as_deref(), Some("dirty") | Some("behind")) {
      return Some(OverviewReviewStatusInfo {
        title: "Review required",
        message: readiness.message.clone(),
      });
    }
  }

  // Derive status from requested reviewers and submitted reviews.
  let reviewers = merged_reviewers(requested_reviewers, reviews, author_login);
  if reviewers.is_empty() {
    return None;
  }

  let has_changes_requested = reviewers.iter().any(|r| {
    matches!(
      reviewer_status_for_login(reviews, &r.login, requested_reviewers),
      ReviewerStatus::ChangesRequested
    )
  });
  let has_approval = reviewers.iter().any(|r| {
    matches!(
      reviewer_status_for_login(reviews, &r.login, requested_reviewers),
      ReviewerStatus::Approved
    )
  });

  if has_changes_requested {
    Some(OverviewReviewStatusInfo {
      title: "Changes requested",
      message: "Some reviewers have requested changes.".to_string(),
    })
  } else if has_approval {
    Some(OverviewReviewStatusInfo {
      title: "Changes approved",
      message: "Pull request has been approved.".to_string(),
    })
  } else {
    Some(OverviewReviewStatusInfo {
      title: "Review required",
      message: "Awaiting review from requested reviewers.".to_string(),
    })
  }
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

fn overview_conflicts_info(
  merge_readiness: Option<&GithubPullRequestMergeReadiness>,
  checks: Option<&GithubPullRequestChecksSummary>,
) -> Option<OverviewConflictsInfo> {
  if let Some(readiness) = merge_readiness {
    let state = readiness
      .mergeable_state
      .as_deref()
      .map(str::trim)
      .map(str::to_ascii_lowercase);
    match state.as_deref() {
      Some("dirty") => {
        return Some(OverviewConflictsInfo {
          kind: OverviewConflictsKind::Conflicts,
          title: "Merge conflicts detected",
          message: "Resolve conflicts before continuing.".to_string(),
        });
      }
      Some("behind") => {
        return Some(OverviewConflictsInfo {
          kind: OverviewConflictsKind::OutOfDate,
          title: "Branch is out of date",
          message: "Update this branch before merging.".to_string(),
        });
      }
      _ => {
        return Some(OverviewConflictsInfo {
          kind: OverviewConflictsKind::NoConflicts,
          title: "No conflicts with base branch",
          message: "Merging can be performed automatically.".to_string(),
        });
      }
    }
  }

  if checks.is_some_and(|c| c.requires_up_to_date_branch) {
    return Some(OverviewConflictsInfo {
      kind: OverviewConflictsKind::OutOfDate,
      title: "Branch is out of date",
      message: "The base branch rules require this pull request to be up to date before merging."
        .to_string(),
    });
  }

  None
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

fn filter_option_users_contains(
  users: &[GithubPullRequestFilterOptionUser],
  candidate: &str,
) -> bool {
  users
    .iter()
    .any(|user| github_shared::logins_match_case_insensitive(user.login.as_str(), candidate))
}

fn matching_filter_option_users(
  options: &[GithubPullRequestFilterOptionUser],
  query: &str,
  selected: &[GithubPullRequestFilterOptionUser],
) -> Vec<GithubPullRequestFilterOptionUser> {
  let normalized_query = query.trim().to_lowercase();
  options
    .iter()
    .filter(|option| !filter_option_users_contains(selected, option.login.as_str()))
    .filter(|option| {
      normalized_query.is_empty() || option.login.to_lowercase().contains(&normalized_query)
    })
    .take(6)
    .cloned()
    .collect()
}

fn find_filter_option_user(
  options: &[GithubPullRequestFilterOptionUser],
  login: &str,
) -> GithubPullRequestFilterOptionUser {
  options
    .iter()
    .find(|option| github_shared::logins_match_case_insensitive(option.login.as_str(), login))
    .cloned()
    .unwrap_or_else(|| GithubPullRequestFilterOptionUser {
      login: login.trim().to_string(),
      avatar_url: None,
    })
}

fn upsert_filter_option_user(
  users: &mut Vec<GithubPullRequestFilterOptionUser>,
  user: GithubPullRequestFilterOptionUser,
) {
  if let Some(existing) = users.iter_mut().find(|existing| {
    github_shared::logins_match_case_insensitive(existing.login.as_str(), user.login.as_str())
  }) {
    *existing = user;
    return;
  }

  users.push(user);
}

fn remove_filter_option_user(users: &mut Vec<GithubPullRequestFilterOptionUser>, login: &str) {
  users.retain(|user| !github_shared::logins_match_case_insensitive(user.login.as_str(), login));
}

fn labels_contains(labels: &[GithubPullRequestLabel], candidate: &str) -> bool {
  labels
    .iter()
    .any(|label| label.name.eq_ignore_ascii_case(candidate))
}

fn matching_label_options(
  options: &[GithubPullRequestFilterOptionLabel],
  query: &str,
  selected: &[GithubPullRequestLabel],
) -> Vec<GithubPullRequestFilterOptionLabel> {
  let normalized_query = query.trim().to_lowercase();
  options
    .iter()
    .filter(|option| !labels_contains(selected, option.name.as_str()))
    .filter(|option| {
      normalized_query.is_empty() || option.name.to_lowercase().contains(&normalized_query)
    })
    .take(8)
    .cloned()
    .collect()
}

fn upsert_label(labels: &mut Vec<GithubPullRequestLabel>, label: GithubPullRequestLabel) {
  if let Some(existing) = labels
    .iter_mut()
    .find(|existing| existing.name.eq_ignore_ascii_case(label.name.as_str()))
  {
    *existing = label;
    return;
  }
  labels.push(label);
}

fn remove_label(labels: &mut Vec<GithubPullRequestLabel>, name: &str) {
  labels.retain(|label| !label.name.eq_ignore_ascii_case(name));
}

fn suggestion_context_from_review_comment(
  comment: &GithubPullRequestReviewComment,
) -> Option<SuggestionContext> {
  let (_, line) = review_comment_preview_line_range(comment)?;
  let start_line = comment
    .start_line
    .or(comment.line)
    .or(comment.original_start_line)
    .or(comment.original_line);
  let original_range = github_shared::extract_original_line_range_from_diff_hunk(
    &comment.diff_hunk,
    start_line,
    line as i64,
  )?;
  Some(SuggestionContext {
    original_start_line: Some(original_range.start_line),
    suggested_start_line: Some(original_range.start_line),
    original_lines: original_range.lines,
    path: Arc::from(comment.path.as_str()),
  })
}

fn review_comment_owned_by_login(comment: &GithubPullRequestReviewComment, login: &str) -> bool {
  github_shared::logins_match_case_insensitive(comment.user.login.as_str(), login)
}

fn upsert_review_local(
  reviews: &mut Vec<GithubPullRequestReview>,
  mut review: GithubPullRequestReview,
) {
  if let Some(existing) = reviews.iter_mut().find(|existing| existing.id == review.id) {
    if review.node_id.is_empty() {
      review.node_id.clone_from(&existing.node_id);
    }
    *existing = review;
    return;
  }

  reviews.push(review);
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

fn build_local_project_tree_items(
  files: &[Rc<GithubPrLocalProjectFile>],
) -> FileTreeBuildResult<GithubPrLocalProjectFile> {
  build_path_tree_items(files, |file| file.path.as_ref())
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

fn should_prepare_local_branch_before_merging_base(
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
  use super::*;
  use crate::api::{
    GithubPullRequestCheckRun, GithubPullRequestChecksRollupState, GithubPullRequestChecksSummary,
    GithubPullRequestCommit, GithubPullRequestDetails, GithubPullRequestFile,
    GithubPullRequestLegacyStatus, GithubPullRequestMergeMethod, GithubPullRequestMergeReadiness,
    GithubPullRequestMergeReadinessStatus, GithubPullRequestReview, GithubPullRequestReviewComment,
    GithubPullRequestReviewCommentUser, GithubPullRequestReviewEvent, GithubPullRequestReviewState,
    GithubPullRequestReviewUser, GithubPullRequestState, GithubPullRequestWorkflowJob,
    GithubPullRequestWorkflowRun, GithubRepository,
  };
  use crate::workspace::WorkspaceApi;
  use git::{BranchKind, BranchRef, merge_branch};
  use git2::{BranchType, Repository, Signature};
  use gpui::TestAppContext;
  use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicU64, Ordering},
    thread,
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
    let repo_root = crate::test_support::temp_path("pr-details-local-repo");

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
      skipped_checks: 0,
      required_checks_total: 3,
      required_checks_passed: 1,
      required_checks_failed: 1,
      required_checks_pending: 1,
      required_checks_skipped: 0,
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
  fn overview_checks_summary_subtitle_lists_skipped_alongside_success() {
    let mut checks = make_checks_summary();
    checks.total_checks = 31;
    checks.successful_checks = 15;
    checks.skipped_checks = 16;
    checks.failed_checks = 0;
    checks.pending_checks = 0;
    checks.overall_state = GithubPullRequestChecksRollupState::Success;

    assert_eq!(
      overview_checks_summary_subtitle(&checks),
      "16 skipped, 15 successful checks"
    );
  }

  #[test]
  fn overview_check_status_label_formats_skipped_and_success_states() {
    assert_eq!(
      overview_check_status_label(
        GithubPullRequestChecksRollupState::Success,
        Some("2026-04-25T10:00:00Z"),
        Some("2026-04-25T10:00:07Z"),
      )
      .as_deref(),
      Some("Successful in 7s"),
    );
    assert!(
      overview_check_status_label(
        GithubPullRequestChecksRollupState::Skipped,
        Some("2026-04-24T10:00:00Z"),
        Some("2026-04-24T10:00:00Z"),
      )
      .unwrap()
      .starts_with("Skipped "),
    );
  }

  #[test]
  fn overview_check_rows_prefix_workflow_name_with_event_suffix() {
    let mut checks = make_checks_summary();
    checks.missing_required_contexts.clear();
    checks.actions_runs = vec![GithubPullRequestWorkflowRun {
      id: 100,
      name: Some("CI".to_string()),
      display_title: Some("CI".to_string()),
      event: "pull_request".to_string(),
      status: Some("completed".to_string()),
      conclusion: Some("success".to_string()),
      state: GithubPullRequestChecksRollupState::Success,
      created_at: "2026-04-25T10:00:00Z".to_string(),
      updated_at: "2026-04-25T10:02:00Z".to_string(),
      run_started_at: Some("2026-04-25T10:00:00Z".to_string()),
      run_number: 12,
      run_attempt: Some(1),
      html_url: Some("https://github.com/acme/widget/actions/runs/100".to_string()),
      jobs: vec![GithubPullRequestWorkflowJob {
        id: 200,
        name: "Frontend (build)".to_string(),
        status: Some("completed".to_string()),
        conclusion: Some("success".to_string()),
        state: GithubPullRequestChecksRollupState::Success,
        started_at: Some("2026-04-25T10:00:00Z".to_string()),
        completed_at: Some("2026-04-25T10:02:00Z".to_string()),
        html_url: None,
        required: false,
        app_name: Some("GitHub Actions".to_string()),
        app_slug: Some("github-actions".to_string()),
        app_avatar_url: None,
        steps: Vec::new(),
      }],
    }];

    let rows = overview_check_rows(&checks);
    let row = rows
      .iter()
      .find(|row| row.id == "workflow-job-200")
      .expect("workflow job row");

    assert_eq!(row.title, "CI / Frontend (build) (pull_request)");
    assert_eq!(row.status_label.as_deref(), Some("Successful in 2m"));
  }

  #[test]
  fn overview_check_rows_keep_provider_avatar_urls() {
    let mut checks = make_checks_summary();
    checks.missing_required_contexts.clear();
    checks.actions_runs = vec![GithubPullRequestWorkflowRun {
      id: 100,
      name: Some("CI".to_string()),
      display_title: Some("build branch".to_string()),
      event: "pull_request".to_string(),
      status: Some("completed".to_string()),
      conclusion: Some("success".to_string()),
      state: GithubPullRequestChecksRollupState::Success,
      created_at: "2026-03-19T10:00:00Z".to_string(),
      updated_at: "2026-03-19T10:02:00Z".to_string(),
      run_started_at: Some("2026-03-19T10:00:00Z".to_string()),
      run_number: 12,
      run_attempt: Some(1),
      html_url: Some("https://github.com/acme/widget/actions/runs/100".to_string()),
      jobs: vec![GithubPullRequestWorkflowJob {
        id: 200,
        name: "build".to_string(),
        status: Some("completed".to_string()),
        conclusion: Some("success".to_string()),
        state: GithubPullRequestChecksRollupState::Success,
        started_at: Some("2026-03-19T10:00:00Z".to_string()),
        completed_at: Some("2026-03-19T10:02:00Z".to_string()),
        html_url: Some("https://github.com/acme/widget/actions/runs/100/job/200".to_string()),
        required: true,
        app_name: Some("GitHub Actions".to_string()),
        app_slug: Some("github-actions".to_string()),
        app_avatar_url: Some("https://avatars.githubusercontent.com/in/15368?v=4".to_string()),
        steps: Vec::new(),
      }],
    }];
    checks.other_checks = vec![GithubPullRequestCheckRun {
      id: 301,
      name: "lint".to_string(),
      status: Some("completed".to_string()),
      conclusion: Some("failure".to_string()),
      state: GithubPullRequestChecksRollupState::Failure,
      started_at: Some("2026-03-19T10:01:00Z".to_string()),
      completed_at: Some("2026-03-19T10:03:00Z".to_string()),
      html_url: Some("https://github.com/acme/widget/runs/301".to_string()),
      details_url: Some("https://github.com/acme/widget/runs/301".to_string()),
      required: true,
      app_name: Some("Reviewdog".to_string()),
      app_slug: Some("reviewdog".to_string()),
      app_avatar_url: Some("https://avatars.githubusercontent.com/u/15138054?v=4".to_string()),
      title: Some("Lint".to_string()),
      summary: Some("Lint failed".to_string()),
      text: None,
      annotations_count: 2,
    }];
    checks.legacy_statuses = vec![GithubPullRequestLegacyStatus {
      id: 401,
      context: "security/brakeman".to_string(),
      status: "success".to_string(),
      state: GithubPullRequestChecksRollupState::Success,
      description: Some("Security checks passed".to_string()),
      target_url: Some("https://ci.example.com/401".to_string()),
      avatar_url: Some("https://ci.example.com/avatar.png".to_string()),
      created_at: "2026-03-19T10:00:00Z".to_string(),
      updated_at: "2026-03-19T10:04:00Z".to_string(),
      required: false,
    }];

    let rows = overview_check_rows(&checks);

    assert_eq!(
      rows
        .iter()
        .find(|row| row.id == "workflow-job-200")
        .and_then(|row| row.app_avatar_url.as_deref()),
      Some("https://avatars.githubusercontent.com/in/15368?v=4")
    );
    assert_eq!(
      rows
        .iter()
        .find(|row| row.id == "check-run-301")
        .and_then(|row| row.app_avatar_url.as_deref()),
      Some("https://avatars.githubusercontent.com/u/15138054?v=4")
    );
    assert_eq!(
      rows
        .iter()
        .find(|row| row.id == "legacy-status-401")
        .and_then(|row| row.app_avatar_url.as_deref()),
      Some("https://ci.example.com/avatar.png")
    );
  }

  #[test]
  fn overview_conflicts_info_returns_conflicts_for_dirty_mergeable_state() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("dirty"),
      "This pull request has merge conflicts that must be resolved before it can be merged.",
    );

    let info = overview_conflicts_info(Some(&readiness), None).expect("conflicts info");

    assert_eq!(info.kind, OverviewConflictsKind::Conflicts);
    assert_eq!(info.title, "Merge conflicts detected");
  }

  #[test]
  fn overview_conflicts_info_returns_out_of_date_for_behind_mergeable_state() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("behind"),
      "This pull request branch is out of date with the base branch.",
    );

    let info = overview_conflicts_info(Some(&readiness), None).expect("conflicts info");

    assert_eq!(info.kind, OverviewConflictsKind::OutOfDate);
    assert_eq!(info.title, "Branch is out of date");
  }

  #[test]
  fn overview_conflicts_info_falls_back_to_checks_requirement() {
    let info = overview_conflicts_info(None, Some(&make_checks_summary())).expect("conflicts info");

    assert_eq!(info.kind, OverviewConflictsKind::OutOfDate);
    assert_eq!(info.title, "Branch is out of date");
  }

  #[test]
  fn overview_conflicts_info_returns_no_conflicts_when_pr_is_ready() {
    let mut checks = make_checks_summary();
    checks.requires_up_to_date_branch = false;
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );

    let info = overview_conflicts_info(Some(&readiness), Some(&checks)).expect("conflicts info");
    assert_eq!(info.kind, OverviewConflictsKind::NoConflicts);
  }

  #[test]
  fn overview_review_status_returns_review_required_when_blocked() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("blocked"),
      "Review is required by reviewers with write access.",
    );

    let info =
      overview_review_status_info(Some(&readiness), &[], &[], "author").expect("review info");
    assert_eq!(info.title, "Review required");
  }

  #[test]
  fn overview_review_status_returns_none_for_dirty_state() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("dirty"),
      "Merge conflicts.",
    );

    assert!(overview_review_status_info(Some(&readiness), &[], &[], "author").is_none());
  }

  #[test]
  fn overview_review_status_returns_none_when_ready_and_no_reviewers() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );

    assert!(overview_review_status_info(Some(&readiness), &[], &[], "author").is_none());
  }

  #[test]
  fn overview_review_status_returns_awaiting_when_reviewer_has_not_approved() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );
    let reviewers = vec![GithubPullRequestFilterOptionUser {
      login: "reviewer1".to_string(),
      avatar_url: None,
    }];

    let info = overview_review_status_info(Some(&readiness), &reviewers, &[], "author")
      .expect("review info");
    assert_eq!(info.title, "Review required");
  }

  #[test]
  fn overview_review_status_returns_approved_when_all_reviewers_approved() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );
    let reviews = vec![GithubPullRequestReview {
      node_id: "PRR_1".to_string(),
      id: 1,
      body: None,
      state: GithubPullRequestReviewState::Approved,
      submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
      commit_id: None,
      html_url: String::new(),
      user: Some(GithubPullRequestReviewUser {
        login: "reviewer1".to_string(),
        avatar_url: None,
      }),
    }];

    let info =
      overview_review_status_info(Some(&readiness), &[], &reviews, "author").expect("review info");
    assert_eq!(info.title, "Changes approved");
  }

  #[test]
  fn overview_review_status_returns_changes_requested() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );
    let reviews = vec![GithubPullRequestReview {
      node_id: "PRR_1".to_string(),
      id: 1,
      body: None,
      state: GithubPullRequestReviewState::RequestChanges,
      submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
      commit_id: None,
      html_url: String::new(),
      user: Some(GithubPullRequestReviewUser {
        login: "reviewer1".to_string(),
        avatar_url: None,
      }),
    }];

    let info =
      overview_review_status_info(Some(&readiness), &[], &reviews, "author").expect("review info");
    assert_eq!(info.title, "Changes requested");
  }

  #[test]
  fn overview_review_status_returns_approved_when_one_approved_and_one_commented() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );
    let reviews = vec![
      GithubPullRequestReview {
        node_id: "PRR_1".to_string(),
        id: 1,
        body: None,
        state: GithubPullRequestReviewState::Approved,
        submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
        commit_id: None,
        html_url: String::new(),
        user: Some(GithubPullRequestReviewUser {
          login: "reviewer1".to_string(),
          avatar_url: None,
        }),
      },
      GithubPullRequestReview {
        node_id: "PRR_2".to_string(),
        id: 2,
        body: None,
        state: GithubPullRequestReviewState::Commented,
        submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
        commit_id: None,
        html_url: String::new(),
        user: Some(GithubPullRequestReviewUser {
          login: "reviewer2".to_string(),
          avatar_url: None,
        }),
      },
    ];

    let info =
      overview_review_status_info(Some(&readiness), &[], &reviews, "author").expect("review info");
    assert_eq!(info.title, "Changes approved");
  }

  #[test]
  fn overview_review_status_changes_requested_overrides_approval() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );
    let reviews = vec![
      GithubPullRequestReview {
        node_id: "PRR_1".to_string(),
        id: 1,
        body: None,
        state: GithubPullRequestReviewState::Approved,
        submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
        commit_id: None,
        html_url: String::new(),
        user: Some(GithubPullRequestReviewUser {
          login: "reviewer1".to_string(),
          avatar_url: None,
        }),
      },
      GithubPullRequestReview {
        node_id: "PRR_2".to_string(),
        id: 2,
        body: None,
        state: GithubPullRequestReviewState::RequestChanges,
        submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
        commit_id: None,
        html_url: String::new(),
        user: Some(GithubPullRequestReviewUser {
          login: "reviewer2".to_string(),
          avatar_url: None,
        }),
      },
    ];

    let info =
      overview_review_status_info(Some(&readiness), &[], &reviews, "author").expect("review info");
    assert_eq!(info.title, "Changes requested");
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
  fn focus_file_tree_action_switches_to_changes_tab_and_focuses_tree(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    let files = files_from_api(vec![make_api_file("src/main.rs", "modified", None)]);
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

      this.active_tab_ix = PR_TAB_OVERVIEW_IX;
      let external_focus = cx.focus_handle();
      window.focus(&external_focus, cx);

      this.focus_file_tree_action(&crate::FocusFileTree, window, cx);

      assert_eq!(this.active_tab_ix, PR_TAB_CHANGES_IX);
      let focused = window.focused(cx).expect("changes tree should take focus");
      assert_ne!(focused, external_focus);
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

    cx.run_until_parked();
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
    assert!(cx.debug_bounds("pr-whitespace-toggle").is_some());
  }

  #[gpui::test]
  async fn changes_raster_image_preview_renders_without_source_editor(cx: &mut TestAppContext) {
    init_gpui_test(cx);
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
    cx.run_until_parked();
    let preview_bounds = cx
      .debug_bounds(GITHUB_PR_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR)
      .expect("binary preview render pane bounds")
      .size;

    assert!(is_raster_preview);
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
    assert!(cx.debug_bounds("pr-whitespace-toggle").is_none());
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
    cx.run_until_parked();
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
    cx.run_until_parked();
    let preview_bounds = cx
      .debug_bounds(GITHUB_PR_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR)
      .expect("binary preview placeholder bounds")
      .size;

    assert!(is_placeholder);
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
    assert!(cx.debug_bounds("pr-whitespace-toggle").is_none());
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

    cx.run_until_parked();
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

    cx.run_until_parked();
    let count_bounds = cx
      .debug_bounds("github-pr-changes-tab-count")
      .expect("changes tab count bounds")
      .size;
    assert!(count_bounds.width > gpui::px(0.0));
    assert!(count_bounds.height > gpui::px(0.0));
  }

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
  fn overview_conflicts_status_is_built_when_pr_has_merge_conflicts() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("dirty"),
      "This pull request has merge conflicts that must be resolved before it can be merged.",
    );

    let info = overview_conflicts_info(Some(&readiness), None).expect("conflicts info");
    assert_eq!(info.kind, OverviewConflictsKind::Conflicts);
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
  fn status_action_is_available_for_loaded_pull_request(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, _cx| {
      this.pull_request = Some(make_pr_details_for_stats());
    });

    let action = page.read_with(cx, |this, _cx| this.pull_request_status_action());
    assert_eq!(action, Some(GithubPrStatusAction::ConvertToDraft));
  }

  #[gpui::test]
  fn copy_pr_branch_command_writes_head_ref_to_clipboard(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let mut mounted_page = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let page = cx.new(|cx| GithubPrDetailsPage::new(window, cx));
      mounted_page = Some(page.clone());
      gpui_component::Root::new(page, window, cx)
    });
    let page = mounted_page.expect("pr details page");

    page.update_in(cx, |this, window, cx| {
      this.pull_request = Some(make_pr_details_for_stats());
      let result =
        this.handle_command_palette_action(ui::CommandPaletteAction::CopyPrBranch, window, cx);
      assert!(result.is_ok());
      let clipboard = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("clipboard text");
      assert_eq!(clipboard, "feature");
    });
  }

  #[gpui::test]
  fn copy_pr_branch_command_is_included_when_pull_request_is_loaded(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    let no_pr_commands = page.read_with(cx, |this, cx| this.command_palette_commands(cx));
    assert!(
      !no_pr_commands
        .iter()
        .any(|command| command.id == ui::CommandPaletteCommandId::CopyPrBranch),
      "copy PR branch should be hidden without a loaded pull request"
    );

    page.update_in(cx, |this, _window, _cx| {
      this.pull_request = Some(make_pr_details_for_stats());
    });

    let with_pr_commands = page.read_with(cx, |this, cx| this.command_palette_commands(cx));
    assert!(
      with_pr_commands
        .iter()
        .any(|command| command.id == ui::CommandPaletteCommandId::CopyPrBranch),
      "copy PR branch should be present once a pull request is loaded"
    );
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
    cx.run_until_parked();

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
    cx.run_until_parked();

    assert!(cx.debug_bounds("github-pr-merge-button").is_none());
    assert!(cx.debug_bounds("github-pr-review-button").is_some());
  }

  #[gpui::test]
  async fn draft_status_action_failure_shows_error_notification(cx: &mut TestAppContext) {
    init_gpui_test(cx);
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

  #[test]
  fn adjacent_pr_tab_ix_wraps_in_both_directions() {
    assert_eq!(
      adjacent_pr_tab_ix(PR_TAB_OVERVIEW_IX, TabNavigationDirection::Previous),
      PR_TAB_CHANGES_IX
    );
    assert_eq!(
      adjacent_pr_tab_ix(PR_TAB_CHANGES_IX, TabNavigationDirection::Previous),
      PR_TAB_OVERVIEW_IX
    );
    assert_eq!(
      adjacent_pr_tab_ix(PR_TAB_CHANGES_IX, TabNavigationDirection::Next),
      PR_TAB_OVERVIEW_IX
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

    let repo_root = crate::test_support::temp_path("pr-switch-palette");
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

    // The deferred command has run by now; the switch it started may still be in
    // flight (fast machines finish it inside run_until_parked), so only await when
    // the task is still around.
    let switch_task = page.update_in(cx, |this, _window, _cx| {
      this.local_branch_switch_task.take()
    });
    if let Some(switch_task) = switch_task {
      switch_task.await;
    }
    cx.run_until_parked();

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
  fn overview_conflicts_action_lands_in_the_shell_when_conflict_resolution_is_already_active(
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
      this.open_overview_pr_alert_in_shell(window, cx);
    });

    cx.update(|_, cx| {
      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/session");
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
    let repo_root = crate::test_support::temp_path("pr-search-local");
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

    let repo_root = crate::test_support::temp_path("pr-local-project");
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

  #[gpui::test]
  async fn same_pr_commit_links_switch_to_changes_and_select_the_commit(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/github/acme/widget/pull/42", cx);
    });
    let target_sha = "abcdef1234567890abcdef1234567890abcdef12";
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    let files_task = page.update_in(cx, |this, window, cx| {
      this.api = make_test_api_client("http://127.0.0.1:1");
      this.current_pr_context = Some(CurrentPrContext {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
        number: 42,
      });
      this.commits = vec![make_api_commit(
        target_sha,
        "feat: add filter",
        Some("2026-02-26T10:00:00Z"),
        Some("0000000000000000000000000000000000000000"),
      )];
      this.active_tab_ix = PR_TAB_OVERVIEW_IX;

      let handled =
        this.handle_gfm_link("https://github.com/acme/widget/commit/abcdef1", window, cx);
      assert!(handled);

      this.files_task.take()
    });
    if let Some(task) = files_task {
      task.await;
    }

    let (active_tab_ix, selected_commit_sha) = page.read_with(cx, |this, _cx| {
      (this.active_tab_ix, this.selected_commit_sha.clone())
    });
    assert_eq!(active_tab_ix, PR_TAB_CHANGES_IX);
    assert_eq!(selected_commit_sha.as_deref(), Some(target_sha));
  }

  fn make_review_comment(
    id: u64,
    created_at: &str,
    in_reply_to_id: Option<u64>,
  ) -> GithubPullRequestReviewComment {
    GithubPullRequestReviewComment {
      node_id: format!("PRRC_{id}"),
      is_outdated: false,
      thread_id: String::new(),
      is_resolved: false,
      is_collapsed: false,
      viewer_can_resolve: false,
      viewer_can_unresolve: false,
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
      is_pending: false,
      pull_request_review_node_id: None,
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
  fn suggestion_context_from_review_comment_falls_back_to_original_line_fields() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.diff_hunk = "@@ -10,3 +10,3 @@\n keep\n current\n keep".to_string();
    comment.start_line = None;
    comment.line = None;
    comment.original_start_line = None;
    comment.original_line = Some(11);

    let ctx = suggestion_context_from_review_comment(&comment).expect("suggestion context");

    assert_eq!(ctx.original_start_line, Some(11));
    assert_eq!(ctx.suggested_start_line, Some(11));
    assert_eq!(ctx.original_lines, vec!["current".to_string()]);
  }

  #[test]
  fn suggested_change_original_lines_detects_stale_head_content() {
    let file_contents = HashMap::from([(
      "src/main.rs".to_string(),
      GithubPrFileContents {
        base: None,
        head: Some("fn main() {\n  println!(\"new\");\n}\n".to_string()),
      },
    )]);
    let original_lines = vec!["  println!(\"old\");".to_string()];

    assert_eq!(
      GithubPrDetailsPage::suggested_change_original_lines_match_current_head(
        &file_contents,
        "src/main.rs",
        Some(2),
        &original_lines,
      ),
      Some(false)
    );
  }

  #[test]
  fn suggested_change_original_lines_match_current_head_content() {
    let file_contents = HashMap::from([(
      "src/main.rs".to_string(),
      GithubPrFileContents {
        base: None,
        head: Some("fn main() {\r\n  println!(\"old\");\r\n}\r\n".to_string()),
      },
    )]);
    let original_lines = vec!["  println!(\"old\");".to_string()];

    assert_eq!(
      GithubPrDetailsPage::suggested_change_original_lines_match_current_head(
        &file_contents,
        "src/main.rs",
        Some(2),
        &original_lines,
      ),
      Some(true)
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
      },
      assignees: Vec::new(),
      requested_reviewers: Vec::new(),
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

  fn make_repo_branch(name: &str) -> GithubRepositoryBranch {
    GithubRepositoryBranch {
      name: name.to_string(),
      commit: crate::api::GithubRepositoryBranchCommit {
        sha: format!("{name}-sha"),
        url: format!("https://api.github.com/repos/acme/widget/commits/{name}"),
      },
      protected: false,
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
      github_shared::commit_subject("\n\nfeat: add filter\n\nbody details"),
      "feat: add filter"
    );
    assert_eq!(github_shared::commit_subject(""), "No commit message");
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
  fn target_branch_selector_sorts_dedupes_and_keeps_current_base() {
    let branches = vec![
      make_repo_branch("release/next"),
      make_repo_branch("main"),
      make_repo_branch("Main"),
    ];

    let names = sorted_branch_names_for_target_selector(branches, "develop", "feature");

    assert_eq!(names, vec!["develop", "main", "release/next"]);
  }

  #[test]
  fn target_branch_selector_excludes_head_branch() {
    let branches = vec![
      make_repo_branch("main"),
      make_repo_branch("feature"),
      make_repo_branch("Feature"),
      make_repo_branch("release/next"),
    ];

    let names = sorted_branch_names_for_target_selector(branches, "main", "feature");

    assert_eq!(names, vec!["main", "release/next"]);
  }

  #[test]
  fn overview_change_stat_labels_are_compact() {
    let pr = make_pr_details_for_stats();
    let labels = overview_change_stat_labels(&pr);

    assert_eq!(labels, ["+20".to_string(), "-4".to_string()]);
  }

  #[test]
  fn parse_github_commit_url_accepts_repository_commit_urls() {
    let parsed = parse_github_commit_url("https://github.com/acme/widget/commit/abcdef1234567890");
    assert_eq!(
      parsed,
      Some((
        "acme".to_string(),
        "widget".to_string(),
        "abcdef1234567890".to_string(),
      ))
    );
  }

  #[test]
  fn parse_github_commit_url_accepts_pull_request_commit_urls() {
    let parsed = parse_github_commit_url(
      "https://github.com/acme/widget/pull/42/commits/abcdef1234567890?diff=split",
    );
    assert_eq!(
      parsed,
      Some((
        "acme".to_string(),
        "widget".to_string(),
        "abcdef1234567890".to_string(),
      ))
    );
  }

  #[test]
  fn resolve_same_pr_commit_link_sha_matches_exact_and_unique_prefix_links() {
    let commits = vec![
      make_api_commit(
        "abcdef1234567890abcdef1234567890abcdef12",
        "first",
        Some("2026-02-20T10:00:00Z"),
        Some("p1"),
      ),
      make_api_commit(
        "fedcba9876543210fedcba9876543210fedcba98",
        "second",
        Some("2026-02-21T10:00:00Z"),
        Some("p2"),
      ),
    ];
    let context = CurrentPrContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      number: 42,
    };

    let exact = resolve_same_pr_commit_link_sha(
      Some(&context),
      &commits,
      "https://github.com/acme/widget/commit/abcdef1234567890abcdef1234567890abcdef12",
    );
    assert_eq!(
      exact.as_deref(),
      Some("abcdef1234567890abcdef1234567890abcdef12")
    );

    let prefix = resolve_same_pr_commit_link_sha(
      Some(&context),
      &commits,
      "https://github.com/acme/widget/commit/fedcba9",
    );
    assert_eq!(
      prefix.as_deref(),
      Some("fedcba9876543210fedcba9876543210fedcba98")
    );
  }

  #[test]
  fn resolve_same_pr_commit_link_sha_rejects_other_repos_and_ambiguous_prefixes() {
    let commits = vec![
      make_api_commit(
        "abcdef1234567890abcdef1234567890abcdef12",
        "first",
        Some("2026-02-20T10:00:00Z"),
        Some("p1"),
      ),
      make_api_commit(
        "abcdef9999999999abcdef9999999999abcdef99",
        "second",
        Some("2026-02-21T10:00:00Z"),
        Some("p2"),
      ),
    ];
    let context = CurrentPrContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      number: 42,
    };

    let other_repo = resolve_same_pr_commit_link_sha(
      Some(&context),
      &commits,
      "https://github.com/acme/other/commit/abcdef1234567890abcdef1234567890abcdef12",
    );
    assert!(other_repo.is_none());

    let ambiguous = resolve_same_pr_commit_link_sha(
      Some(&context),
      &commits,
      "https://github.com/acme/widget/commit/abcdef",
    );
    assert!(ambiguous.is_none());
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
