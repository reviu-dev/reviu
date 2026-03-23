use std::{
  collections::{BTreeMap, HashMap, HashSet},
  hash::{DefaultHasher, Hash, Hasher},
  path::{Path, PathBuf},
  rc::Rc,
  sync::Arc,
};

use editor::Editor;
use gfm_markdown_viewer::{
  GithubBlobLineReference, GithubCodeReferencePreview, LinkAction, MarkdownRenderOptions,
  MarkdownRenderState, extract_github_blob_line_references, render_markdown,
};
use gpui::{
  AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, ParentElement, Render,
  RenderImage, ScrollAnchor, ScrollHandle, SharedString, Styled, Subscription, Task, Window, div,
  img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, IndexPath, Placement, Selectable, Sizable as _,
  StyledExt,
  avatar::Avatar,
  button::{Button, ButtonVariants as _},
  h_flex,
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  scroll::ScrollableElement,
  select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
  spinner::Spinner,
  tab::{Tab, TabBar},
  tag::Tag,
  text::TextView,
  tree::{TreeItem, TreeState, tree},
  v_flex,
};
use smol::unblock;

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteGithubRepoTab, CommandPaletteHandler, CommandPalettePage, ConfirmDialog,
  DETAILS_PAGE_CONTAINER_MAX_WIDTH, FILE_ICON_SIZE_PX, Input, InputState, SearchFileEntry,
  SearchFileHandler, SelectableRowStyle, StatusTag, StatusThemeExt as _, UiIconName, WindowExt,
  file_icon_path_for_name_with_theme, h_resizable, parse_github_url_action, resizable_panel,
  selectable_list_item,
};

use crate::{
  ShowCommandPalette, ShowFileSearch,
  api::{
    ApiClient, GithubIssue, GithubIssueDescriptionUpdate, GithubIssueDetails,
    GithubIssueDetailsComment, GithubIssueStateReason, GithubIssueUser, GithubPullRequest,
    GithubRepositoryDetails,
  },
  auth_state::{AuthState, AuthStateStore},
  date_format::{format_compact_datetime, format_long_date_opt},
  file_preview::{is_markdown_path, is_svg_path},
  file_search_palette::open_file_search_palette as open_shared_file_search_palette,
  github_navigation::{
    SameRepoIssueLinkNavigation, open_pr_target, open_repo_target, same_repo_issue_link_navigation,
    should_open_externally,
  },
  github_page::GithubPageHandle,
  github_pr_details_page::{GithubPrDetailsPageHandle, GithubPrOpenTarget},
  github_shared,
  workspace::{WorkspaceApi, WorkspacePage, WorkspaceRoute},
};

fn list_base_item(
  ix: IndexPath,
  selected_index: Option<IndexPath>,
  theme: &gpui_component::Theme,
) -> ListItem {
  selectable_list_item(
    ix,
    Some(ix) == selected_index,
    SelectableRowStyle::Inset,
    theme,
  )
}

fn update_selected_index<D: ListDelegate>(
  selected_index: &mut Option<IndexPath>,
  ix: Option<IndexPath>,
  cx: &mut Context<ListState<D>>,
) {
  *selected_index = ix;
  cx.notify();
}

fn format_repo_size(size_kb: u64) -> SharedString {
  const KB_PER_MB: u64 = 1024;
  const KB_PER_GB: u64 = 1024 * 1024;

  if size_kb >= KB_PER_GB {
    return format!("{:.1} GB", size_kb as f64 / KB_PER_GB as f64).into();
  }
  if size_kb >= KB_PER_MB {
    return format!("{:.1} MB", size_kb as f64 / KB_PER_MB as f64).into();
  }
  format!("{} KB", size_kb).into()
}

fn github_page_navigation(has_pro_access: bool) -> (WorkspacePage, bool) {
  if has_pro_access {
    (WorkspacePage::Github, true)
  } else {
    (WorkspacePage::Billing, false)
  }
}

fn should_show_overview_loading_state(repository_loading: bool, has_repository: bool) -> bool {
  repository_loading && !has_repository
}

fn repo_palette_open_target(has_pro_access: bool) -> WorkspacePage {
  if has_pro_access {
    WorkspacePage::GithubRepo
  } else {
    WorkspacePage::Billing
  }
}

fn repo_tab_count_label(count: usize) -> SharedString {
  count.to_string().into()
}

const CODE_SIDEBAR_DEFAULT_WIDTH: f32 = 400.0;
const CODE_SIDEBAR_MIN_WIDTH: f32 = 250.0;
const CODE_SIDEBAR_MAX_WIDTH: f32 = 1500.0;
const CODE_HEADER_HEIGHT: f32 = 40.0;
const REPO_TAB_OVERVIEW_IX: usize = 0;
const REPO_TAB_README_IX: usize = 1;
const REPO_TAB_CODE_IX: usize = 2;
const REPO_TAB_PULL_REQUESTS_IX: usize = 3;
const REPO_TAB_ISSUES_IX: usize = 4;
const GITHUB_REPO_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR: &str =
  "github-repo-markdown-preview-editor-pane";
const GITHUB_REPO_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR: &str =
  "github-repo-markdown-preview-render-pane";

#[derive(Clone, Debug)]
struct GithubRepoCodeFile {
  path: SharedString,
  sha: SharedString,
}

#[derive(Default)]
struct GithubRepoCodeTreeNode {
  name: String,
  path: String,
  children: BTreeMap<String, GithubRepoCodeTreeNode>,
  file: Option<Rc<GithubRepoCodeFile>>,
}

impl GithubRepoCodeTreeNode {
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

type RepoCodeTreeBuildResult = (
  Vec<TreeItem>,
  HashMap<String, Rc<GithubRepoCodeFile>>,
  Option<usize>,
  Option<String>,
);

fn build_repo_code_tree_items(files: &[Rc<GithubRepoCodeFile>]) -> RepoCodeTreeBuildResult {
  fn insert_node(
    map: &mut BTreeMap<String, GithubRepoCodeTreeNode>,
    parts: &[&str],
    prefix: &str,
    file: Rc<GithubRepoCodeFile>,
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
      .or_insert_with(|| GithubRepoCodeTreeNode::new(head.to_string(), path.clone()));

    if tail.is_empty() {
      node.file = Some(file);
      return;
    }

    let node_path = node.path.clone();
    insert_node(&mut node.children, tail, &node_path, file);
  }

  let mut root: BTreeMap<String, GithubRepoCodeTreeNode> = BTreeMap::new();
  let mut file_lookup: HashMap<String, Rc<GithubRepoCodeFile>> = HashMap::new();

  for file in files {
    let path = file.path.as_ref();
    file_lookup.insert(path.to_string(), file.clone());
    let parts: Vec<&str> = path.split('/').collect();
    insert_node(&mut root, &parts, "", file.clone());
  }

  let mut order = Vec::new();
  let mut first_file_id = None;

  let mut root_nodes: Vec<GithubRepoCodeTreeNode> = root.into_values().collect();
  root_nodes.sort_by(|a, b| {
    b.is_folder()
      .cmp(&a.is_folder())
      .then_with(|| a.name.cmp(&b.name))
  });

  let items = root_nodes
    .into_iter()
    .map(|node| build_repo_code_tree_item(node, &mut order, &mut first_file_id))
    .collect::<Vec<_>>();

  let selected_index = first_file_id
    .as_ref()
    .and_then(|id| order.iter().position(|candidate| candidate == id));

  (items, file_lookup, selected_index, first_file_id)
}

fn build_repo_code_tree_item(
  node: GithubRepoCodeTreeNode,
  order: &mut Vec<String>,
  first_file_id: &mut Option<String>,
) -> TreeItem {
  let mut child_nodes: Vec<GithubRepoCodeTreeNode> = node.children.into_values().collect();
  child_nodes.sort_by(|a, b| {
    b.is_folder()
      .cmp(&a.is_folder())
      .then_with(|| a.name.cmp(&b.name))
  });

  let mut item = TreeItem::new(node.path.clone(), node.name.clone());
  if !child_nodes.is_empty() {
    let children = child_nodes
      .into_iter()
      .map(|child| build_repo_code_tree_item(child, order, first_file_id))
      .collect::<Vec<_>>();
    item = item.children(children);
  }

  order.push(node.path.clone());
  if node.file.is_some() && first_file_id.is_none() {
    *first_file_id = Some(node.path.clone());
  }

  item
}

fn normalize_non_empty_string(value: &str) -> Option<String> {
  let trimmed = value.trim();
  if trimmed.is_empty() {
    None
  } else {
    Some(trimmed.to_string())
  }
}

fn homepage_button_label(homepage: &str) -> SharedString {
  normalize_non_empty_string(homepage)
    .unwrap_or_else(|| "Homepage".to_string())
    .into()
}

fn effective_repo_branch(
  selected_branch: Option<&str>,
  default_branch: Option<&str>,
) -> Option<String> {
  selected_branch
    .and_then(normalize_non_empty_string)
    .or_else(|| default_branch.and_then(normalize_non_empty_string))
}

fn should_apply_repo_request_result(request_generation: u64, task_generation: u64) -> bool {
  request_generation == task_generation
}

fn should_fetch_readme_for_branch(
  loaded_branch: Option<&str>,
  requested_branch: &str,
  has_error: bool,
) -> bool {
  if has_error {
    return true;
  }

  let requested_branch = requested_branch.trim();
  if requested_branch.is_empty() {
    return false;
  }

  loaded_branch
    .map(str::trim)
    .filter(|branch| !branch.is_empty())
    != Some(requested_branch)
}

fn should_prefetch_code_tree_for_tab(tab_ix: usize) -> bool {
  tab_ix == REPO_TAB_OVERVIEW_IX || tab_ix == REPO_TAB_CODE_IX
}

#[derive(Clone)]
struct GithubRepoBranchSelectItem {
  branch: String,
  label: SharedString,
}

impl GithubRepoBranchSelectItem {
  fn new(branch: String) -> Self {
    let label: SharedString = branch.clone().into();
    Self { branch, label }
  }
}

impl SelectItem for GithubRepoBranchSelectItem {
  type Value = String;

  fn title(&self) -> SharedString {
    self.label.clone()
  }

  fn render(&self, _: &mut Window, _cx: &mut App) -> impl IntoElement {
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

fn build_repo_branch_select_items(
  branches: Vec<String>,
  selected_branch: Option<&str>,
) -> Vec<GithubRepoBranchSelectItem> {
  let mut names = branches
    .into_iter()
    .filter_map(|name| normalize_non_empty_string(&name))
    .collect::<Vec<_>>();
  names.sort();
  names.dedup();

  if let Some(selected_branch) = selected_branch.and_then(normalize_non_empty_string)
    && !names.iter().any(|name| name == &selected_branch)
  {
    names.push(selected_branch);
    names.sort();
  }

  names
    .into_iter()
    .map(GithubRepoBranchSelectItem::new)
    .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubRepoOpenTarget {
  Overview,
  PullRequests,
  Issues {
    issue_number: Option<u64>,
    issue_comment_id: Option<u64>,
  },
}

impl GithubRepoOpenTarget {
  fn tab_ix(self) -> usize {
    match self {
      GithubRepoOpenTarget::Overview => REPO_TAB_OVERVIEW_IX,
      GithubRepoOpenTarget::PullRequests => REPO_TAB_PULL_REQUESTS_IX,
      GithubRepoOpenTarget::Issues { .. } => REPO_TAB_ISSUES_IX,
    }
  }

  fn issue_number(self) -> Option<u64> {
    match self {
      GithubRepoOpenTarget::Issues { issue_number, .. } => issue_number,
      _ => None,
    }
  }

  fn issue_comment_id(self) -> Option<u64> {
    match self {
      GithubRepoOpenTarget::Issues {
        issue_comment_id, ..
      } => issue_comment_id,
      _ => None,
    }
  }
}

fn repo_open_target_from_palette(
  tab: Option<CommandPaletteGithubRepoTab>,
  issue_number: Option<u64>,
  issue_comment_id: Option<u64>,
) -> GithubRepoOpenTarget {
  match tab {
    Some(CommandPaletteGithubRepoTab::PullRequests) => GithubRepoOpenTarget::PullRequests,
    Some(CommandPaletteGithubRepoTab::Issues) => GithubRepoOpenTarget::Issues {
      issue_number,
      issue_comment_id,
    },
    Some(CommandPaletteGithubRepoTab::Overview) | None => GithubRepoOpenTarget::Overview,
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubIssueVisualState {
  Open,
  Completed,
  NotPlanned,
}

fn issue_visual_state(
  state: &str,
  reason: Option<GithubIssueStateReason>,
) -> GithubIssueVisualState {
  if state.eq_ignore_ascii_case("open") {
    return GithubIssueVisualState::Open;
  }

  match reason {
    Some(GithubIssueStateReason::Reopened) => GithubIssueVisualState::Open,
    Some(GithubIssueStateReason::NotPlanned | GithubIssueStateReason::Duplicate) => {
      GithubIssueVisualState::NotPlanned
    }
    Some(GithubIssueStateReason::Completed) | None => GithubIssueVisualState::Completed,
  }
}

fn issue_user_display_name(user: Option<&GithubIssueUser>) -> SharedString {
  let fallback = "unknown".to_string();
  let name = user
    .and_then(|user| user.name.clone())
    .filter(|name| !name.trim().is_empty());
  let login = user
    .map(|user| user.login.clone())
    .filter(|login| !login.trim().is_empty());
  name.or(login).unwrap_or(fallback).into()
}

fn issue_markdown_body_or_fallback(body: Option<&str>) -> SharedString {
  body
    .map(str::trim)
    .filter(|body| !body.is_empty())
    .unwrap_or("No description provided.")
    .to_string()
    .into()
}

fn issue_comment_markdown_body_or_fallback(body: Option<&str>) -> SharedString {
  body
    .map(str::trim)
    .filter(|body| !body.is_empty())
    .unwrap_or("No comment body.")
    .to_string()
    .into()
}

fn current_github_login(cx: &App) -> Option<String> {
  match AuthStateStore::get(cx) {
    AuthState::Authenticated(user) => user
      .github_login
      .or_else(|| github_shared::normalize_non_empty_text(user.name.as_str())),
    _ => None,
  }
}

fn issue_details_comment_owned_by_login(comment: &GithubIssueDetailsComment, login: &str) -> bool {
  comment
    .user
    .as_ref()
    .is_some_and(|user| github_shared::logins_match_case_insensitive(user.login.as_str(), login))
}

fn next_issue_description_body(raw_value: &str, initial_value: &str) -> Option<String> {
  github_shared::next_trimmed_text_update(raw_value, initial_value)
}

fn apply_issue_description_update_local(
  issue: &mut GithubIssueDetails,
  update: GithubIssueDescriptionUpdate,
) {
  issue.body = update.body;
  issue.updated_at = update.updated_at;
}

fn upsert_issue_details_comment_local(
  comments: &mut Vec<GithubIssueDetailsComment>,
  comment: GithubIssueDetailsComment,
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

fn remove_issue_details_comment_local(
  comments: &mut Vec<GithubIssueDetailsComment>,
  comment_id: u64,
) -> Option<(usize, GithubIssueDetailsComment)> {
  let (index, removed) = comments
    .iter()
    .enumerate()
    .find(|(_, comment)| comment.id == comment_id)
    .map(|(index, comment)| (index, comment.clone()))?;
  comments.remove(index);
  Some((index, removed))
}

fn restore_issue_details_comment_local(
  comments: &mut Vec<GithubIssueDetailsComment>,
  index: usize,
  comment: GithubIssueDetailsComment,
) {
  let insert_index = index.min(comments.len());
  comments.insert(insert_index, comment);
}

fn issue_description_scope_id(issue_id: u64) -> usize {
  (issue_id as usize).wrapping_mul(1_000_003).wrapping_add(1)
}

fn issue_comment_scope_id(issue_id: u64, comment_id: u64) -> usize {
  (issue_id as usize)
    .wrapping_mul(1_000_003)
    .wrapping_add(comment_id as usize)
    .wrapping_mul(31)
    .wrapping_add(2)
}

fn readme_scope_id(owner: &str, repo: &str, branch: &str) -> usize {
  let mut hasher = DefaultHasher::new();
  "github-repo-readme".hash(&mut hasher);
  owner.to_ascii_lowercase().hash(&mut hasher);
  repo.to_ascii_lowercase().hash(&mut hasher);
  branch.to_ascii_lowercase().hash(&mut hasher);
  hasher.finish() as usize
}

fn readme_image_base_url(
  owner: &str,
  repo: &str,
  branch: &str,
  readme_path: Option<&str>,
) -> Option<String> {
  let owner = normalize_non_empty_string(owner)?;
  let repo = normalize_non_empty_string(repo)?;
  let branch = normalize_non_empty_string(branch)?;

  let readme_path = readme_path
    .and_then(normalize_non_empty_string)
    .unwrap_or_else(|| "README.md".to_string())
    .trim_start_matches('/')
    .to_string();

  let readme_dir = Path::new(readme_path.as_str())
    .parent()
    .map(|path| path.to_string_lossy().replace('\\', "/"))
    .and_then(|path| {
      let normalized = path.trim().trim_matches('/');
      if normalized.is_empty() || normalized == "." {
        None
      } else {
        Some(normalized.to_string())
      }
    });

  let mut base_url = format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}/");
  if let Some(dir) = readme_dir {
    base_url.push_str(dir.as_str());
    base_url.push('/');
  }

  Some(base_url)
}

const ISSUE_DETAILS_SHEET_WIDTH_PX: f32 = 850.0;
const ISSUE_DETAILS_SHEET_MIN_WIDTH_PX: f32 = 600.0;
const ISSUE_DETAILS_SHEET_MAX_WIDTH_PX: f32 = 1200.0;
const ISSUE_COMMENT_INPUT_HEIGHT_PX: f32 = 100.0;
const ISSUE_DESCRIPTION_INPUT_HEIGHT_PX: f32 = 500.0;

#[derive(Clone)]
struct IssueSheetResizeDrag;

impl Render for IssueSheetResizeDrag {
  fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
    gpui::Empty
  }
}

fn issue_state_label(state: &str, reason: Option<GithubIssueStateReason>) -> SharedString {
  if state.eq_ignore_ascii_case("open") {
    return "Open".into();
  }

  match reason {
    Some(GithubIssueStateReason::Completed) => "Closed".into(),
    Some(GithubIssueStateReason::Reopened) => "Reopened".into(),
    Some(GithubIssueStateReason::NotPlanned) => "Not planned".into(),
    Some(GithubIssueStateReason::Duplicate) => "Duplicate".into(),
    None => "Closed".into(),
  }
}

fn issue_code_reference_requests(
  issue: &GithubIssueDetails,
) -> (
  Vec<GithubBlobLineReference>,
  HashMap<u64, Vec<GithubBlobLineReference>>,
) {
  let description = issue_markdown_body_or_fallback(issue.body.as_deref());
  let description_references = extract_github_blob_line_references(description.as_ref());

  let comment_references: HashMap<u64, Vec<GithubBlobLineReference>> = issue
    .comments
    .iter()
    .filter_map(|comment| {
      let body = issue_comment_markdown_body_or_fallback(comment.body.as_deref());
      let references = extract_github_blob_line_references(body.as_ref());
      if references.is_empty() {
        None
      } else {
        Some((comment.id, references))
      }
    })
    .collect();

  (description_references, comment_references)
}

fn collect_unique_issue_code_reference_requests(
  description_references: &[GithubBlobLineReference],
  comment_references: &HashMap<u64, Vec<GithubBlobLineReference>>,
) -> Vec<GithubBlobLineReference> {
  let mut references = Vec::new();
  let mut seen = HashSet::new();
  for reference in description_references
    .iter()
    .chain(comment_references.values().flat_map(|refs| refs.iter()))
  {
    if seen.insert(reference.url.clone()) {
      references.push(reference.clone());
    }
  }
  references
}

fn github_code_reference_preview_from_content(
  reference: &GithubBlobLineReference,
  content: &str,
) -> Option<GithubCodeReferencePreview> {
  github_shared::line_snippets_from_content(content, reference.start_line, reference.end_line).map(
    |snippets| {
      let actual_end_line = reference
        .start_line
        .saturating_add(snippets.len().saturating_sub(1));
      GithubCodeReferencePreview {
        url: Arc::from(reference.url.as_str()),
        repo: Arc::from(github_shared::repo_label(&reference.owner, &reference.repo)),
        path: Arc::from(reference.path.as_str()),
        reference: Arc::from(reference.reference.as_str()),
        start_line: reference.start_line,
        end_line: actual_end_line,
        snippets: snippets.into_iter().map(Arc::<str>::from).collect(),
      }
    },
  )
}

fn github_code_reference_preview_map(
  references: &[GithubBlobLineReference],
  cache: &HashMap<String, Option<GithubCodeReferencePreview>>,
) -> Option<Arc<HashMap<Arc<str>, GithubCodeReferencePreview>>> {
  let previews: HashMap<Arc<str>, GithubCodeReferencePreview> = references
    .iter()
    .filter_map(|reference| {
      cache
        .get(&reference.url)
        .and_then(|preview| preview.clone())
        .map(|preview| (preview.url.clone(), preview))
    })
    .collect();

  if previews.is_empty() {
    None
  } else {
    Some(Arc::new(previews))
  }
}

fn should_apply_issue_request_result(request_generation: u64, task_generation: u64) -> bool {
  request_generation == task_generation
}

fn should_keep_issue_sheet_open_for_repo_target(
  current_owner: &str,
  current_repo: &str,
  owner: &str,
  repo: &str,
  tab: Option<CommandPaletteGithubRepoTab>,
  issue_number: Option<u64>,
) -> bool {
  current_owner.eq_ignore_ascii_case(owner)
    && current_repo.eq_ignore_ascii_case(repo)
    && tab == Some(CommandPaletteGithubRepoTab::Issues)
    && issue_number.is_some()
}

fn clamp_issue_sheet_width(width: f32) -> f32 {
  width.clamp(
    ISSUE_DETAILS_SHEET_MIN_WIDTH_PX,
    ISSUE_DETAILS_SHEET_MAX_WIDTH_PX,
  )
}

fn issue_sheet_width_from_cursor_x(viewport_width: f32, cursor_x: f32) -> f32 {
  clamp_issue_sheet_width((viewport_width - cursor_x).max(0.0))
}

#[derive(Clone, Debug)]
struct GithubRepoPullRequestRow {
  pr: Rc<GithubPullRequest>,
}

impl GithubRepoPullRequestRow {
  fn matches(&self, query: &str) -> bool {
    if query.is_empty() {
      return true;
    }

    let q = query.to_lowercase();
    self.pr.title.to_lowercase().contains(&q)
      || github_shared::pull_request_author_display_name(&self.pr.author)
        .to_lowercase()
        .contains(&q)
      || self.pr.number.to_string().contains(&q)
      || self
        .pr
        .labels
        .iter()
        .any(|label| label.name.to_lowercase().contains(&q))
  }
}

struct GithubRepoPullRequestListDelegate {
  all_rows: Vec<Rc<GithubRepoPullRequestRow>>,
  matched_rows: Vec<Rc<GithubRepoPullRequestRow>>,
  selected_index: Option<IndexPath>,
  query: SharedString,
  loading: bool,
}

impl GithubRepoPullRequestListDelegate {
  fn new() -> Self {
    Self {
      all_rows: Vec::new(),
      matched_rows: Vec::new(),
      selected_index: Some(IndexPath::default()),
      query: "".into(),
      loading: false,
    }
  }

  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();
    let q = self.query.as_ref();

    let rows: Vec<Rc<GithubRepoPullRequestRow>> = self
      .all_rows
      .iter()
      .filter(|row| row.matches(q))
      .cloned()
      .collect();

    self.matched_rows = rows;
  }

  fn set_rows(&mut self, rows: Vec<Rc<GithubRepoPullRequestRow>>) {
    self.all_rows = rows;
    self.prepare(self.query.clone());
  }
}

impl ListDelegate for GithubRepoPullRequestListDelegate {
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
    let base_item = list_base_item(ix, self.selected_index, &theme);
    let row = self.matched_rows.get(ix.row)?;

    Some(
      base_item
        .px_2()
        .py_1()
        .child(github_shared::pull_request_list_row_body(
          row.pr.as_ref(),
          &theme,
          false,
          true,
        )),
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
      .gap_2()
      .text_color(cx.theme().muted_foreground)
      .child(Icon::new(IconName::Inbox).size_6())
      .child("No pull request found")
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    update_selected_index(&mut self.selected_index, ix, cx);
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

  fn loading(&self, _: &App) -> bool {
    self.loading
  }
}

#[derive(Clone, Debug)]
struct GithubRepoIssueRow {
  issue: Rc<GithubIssue>,
}

impl GithubRepoIssueRow {
  fn matches(&self, query: &str) -> bool {
    if query.is_empty() {
      return true;
    }

    let q = query.to_lowercase();
    self.issue.title.to_lowercase().contains(&q)
      || self.issue.number.to_string().contains(&q)
      || self
        .issue
        .labels
        .iter()
        .any(|label| label.name.to_lowercase().contains(&q))
      || self
        .issue
        .user
        .as_ref()
        .map(|user| {
          user.login.to_lowercase().contains(&q)
            || user
              .name
              .as_ref()
              .map(|name| name.to_lowercase().contains(&q))
              .unwrap_or(false)
        })
        .unwrap_or(false)
  }
}

struct GithubRepoIssueListDelegate {
  all_rows: Vec<Rc<GithubRepoIssueRow>>,
  matched_rows: Vec<Rc<GithubRepoIssueRow>>,
  selected_index: Option<IndexPath>,
  query: SharedString,
  loading: bool,
}

impl GithubRepoIssueListDelegate {
  fn new() -> Self {
    Self {
      all_rows: Vec::new(),
      matched_rows: Vec::new(),
      selected_index: Some(IndexPath::default()),
      query: "".into(),
      loading: false,
    }
  }

  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();
    let q = self.query.as_ref();

    let rows: Vec<Rc<GithubRepoIssueRow>> = self
      .all_rows
      .iter()
      .filter(|row| row.matches(q))
      .cloned()
      .collect();

    self.matched_rows = rows;
  }

  fn set_rows(&mut self, rows: Vec<Rc<GithubRepoIssueRow>>) {
    self.all_rows = rows;
    self.prepare(self.query.clone());
  }
}

impl ListDelegate for GithubRepoIssueListDelegate {
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
    let base_item = list_base_item(ix, self.selected_index, &theme);
    let row = self.matched_rows.get(ix.row)?;
    let issue = &row.issue;

    let display_name = issue_user_display_name(issue.user.as_ref());
    let created_at = format_compact_datetime(&issue.created_at);
    let updated_at = format_compact_datetime(&issue.updated_at);

    let (state_icon, state_color) =
      match issue_visual_state(&issue.state, issue.state_reason.clone()) {
        GithubIssueVisualState::Open => (UiIconName::CircleDot, theme.status_green()),
        GithubIssueVisualState::Completed => (UiIconName::CircleCheck, theme.status_violet()),
        GithubIssueVisualState::NotPlanned => (UiIconName::CircleSlash, theme.status_gray()),
      };

    let issue_user = h_flex()
      .items_center()
      .gap_2()
      .child(
        Avatar::new()
          .name(display_name.clone())
          .when_some(
            issue.user.as_ref().and_then(|user| user.avatar_url.clone()),
            |this, url| this.src(url),
          )
          .small(),
      )
      .child(
        div()
          .min_w_0()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(Label::new(display_name).truncate()),
      );

    let label_tags = issue.labels.iter().take(4).map(|label| {
      Tag::secondary()
        .small()
        .rounded_full()
        .child(label.name.clone())
    });

    Some(
      base_item.px_2().py_2().child(
        v_flex()
          .gap_1()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(Icon::new(state_icon).size_3().text_color(state_color))
              .child(
                div()
                  .min_w_0()
                  .flex_1()
                  .child(Label::new(issue.title.clone()).truncate()),
              )
              .when(!issue.labels.is_empty(), |this| {
                this.child(
                  h_flex()
                    .min_w_0()
                    .overflow_hidden()
                    .gap_1()
                    .children(label_tags),
                )
              }),
          )
          .child(
            h_flex()
              .gap_3()
              .items_center()
              .min_w_0()
              .overflow_hidden()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(format!("#{}", issue.number))
              .child(issue_user)
              .child(format!("Opened {}", created_at))
              .child(format!("Updated {}", updated_at)),
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
      .gap_2()
      .text_color(cx.theme().muted_foreground)
      .child(Icon::new(IconName::Inbox).size_6())
      .child("No issue found")
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    update_selected_index(&mut self.selected_index, ix, cx);
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

  fn loading(&self, _: &App) -> bool {
    self.loading
  }
}

struct GithubIssueDetailsSheetView {
  focus_handle: FocusHandle,
  scroll_handle: ScrollHandle,
  api: ApiClient,
  owner: String,
  repo: String,
  issue_number: u64,
  issue: Option<GithubIssueDetails>,
  loading: bool,
  error: Option<SharedString>,
  task: Option<Task<()>>,
  markdown_state: MarkdownRenderState,
  code_reference_cache: HashMap<String, Option<GithubCodeReferencePreview>>,
  code_reference_tasks: HashMap<String, Task<()>>,
  description_references: Vec<GithubBlobLineReference>,
  comment_references: HashMap<u64, Vec<GithubBlobLineReference>>,
  pending_comment_scroll_id: Option<u64>,
  pending_comment_scroll_attempts: u8,
  request_generation: u64,
  comment_input: Option<Entity<InputState>>,
  comment_input_submitting: bool,
  comment_input_error: Option<SharedString>,
  edit_input: Option<Entity<InputState>>,
  editing_comment_id: Option<u64>,
  edit_initial_body: Option<String>,
  edit_submitting: bool,
  edit_error: Option<SharedString>,
  description_edit_input: Option<Entity<InputState>>,
  description_editing: bool,
  description_initial_body: Option<String>,
  description_submitting: bool,
  description_error: Option<SharedString>,
}

impl GithubIssueDetailsSheetView {
  fn new(
    api: ApiClient,
    owner: String,
    repo: String,
    issue_number: u64,
    issue_comment_id: Option<u64>,
    cx: &mut Context<Self>,
  ) -> Self {
    let mut this = Self {
      focus_handle: cx.focus_handle(),
      scroll_handle: ScrollHandle::new(),
      api,
      owner: owner.clone(),
      repo: repo.clone(),
      issue_number,
      issue: None,
      loading: false,
      error: None,
      task: None,
      markdown_state: MarkdownRenderState::new(),
      code_reference_cache: HashMap::new(),
      code_reference_tasks: HashMap::new(),
      description_references: Vec::new(),
      comment_references: HashMap::new(),
      pending_comment_scroll_id: issue_comment_id,
      pending_comment_scroll_attempts: if issue_comment_id.is_some() { 4 } else { 0 },
      request_generation: 0,
      comment_input: None,
      comment_input_submitting: false,
      comment_input_error: None,
      edit_input: None,
      editing_comment_id: None,
      edit_initial_body: None,
      edit_submitting: false,
      edit_error: None,
      description_edit_input: None,
      description_editing: false,
      description_initial_body: None,
      description_submitting: false,
      description_error: None,
    };
    this.load_issue(owner, repo, issue_number, cx);
    this
  }

  fn load_issue(&mut self, owner: String, repo: String, issue_number: u64, cx: &mut Context<Self>) {
    self.owner = owner.clone();
    self.repo = repo.clone();
    self.issue_number = issue_number;
    self.issue = None;
    self.loading = true;
    self.error = None;
    self.code_reference_cache.clear();
    self.code_reference_tasks.clear();
    self.description_references.clear();
    self.comment_references.clear();
    self.request_generation = self.request_generation.saturating_add(1);
    self.comment_input_submitting = false;
    self.comment_input_error = None;
    self.editing_comment_id = None;
    self.edit_initial_body = None;
    self.edit_submitting = false;
    self.edit_error = None;
    self.description_editing = false;
    self.description_initial_body = None;
    self.description_submitting = false;
    self.description_error = None;
    let generation = self.request_generation;

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.fetch_github_repository_issue_details(&owner, &repo, issue_number))
          .await;

      let _ = this.update(cx, |this, cx| {
        if !should_apply_issue_request_result(this.request_generation, generation) {
          return;
        }

        this.loading = false;

        match result {
          Ok(issue) => {
            let (description_references, comment_references) =
              issue_code_reference_requests(&issue);
            this.description_references = description_references;
            this.comment_references = comment_references;
            this.issue = Some(issue);
            this.error = None;
            this.schedule_issue_code_reference_fetches(generation, cx);
          }
          Err(error) => {
            let message = error.to_string();
            let error_message: SharedString =
              if github_shared::is_unauthorized_error_message(&message) {
                "Authentication required. Please sign in again.".into()
              } else {
                message.into()
              };
            this.issue = None;
            this.error = Some(error_message);
          }
        }

        cx.notify();
      });
    });

    self.task = Some(task);
    cx.notify();
  }

  fn schedule_issue_code_reference_fetches(&mut self, generation: u64, cx: &mut Context<Self>) {
    let references = collect_unique_issue_code_reference_requests(
      &self.description_references,
      &self.comment_references,
    );

    for reference in references {
      if self.code_reference_cache.contains_key(&reference.url)
        || self.code_reference_tasks.contains_key(&reference.url)
      {
        continue;
      }

      let cache_key = reference.url.clone();
      let owner = reference.owner.clone();
      let repo = reference.repo.clone();
      let path = reference.path.clone();
      let revision = reference.reference.clone();
      let api = self.api.clone();
      let reference_for_preview = reference.clone();

      let task = cx.spawn(async move |this, cx| {
        let result =
          unblock(move || api.fetch_github_file_content(&owner, &repo, &path, &revision)).await;

        let preview = match result {
          Ok(Some(content)) => {
            github_code_reference_preview_from_content(&reference_for_preview, &content)
          }
          _ => None,
        };

        let _ = this.update(cx, |this, cx| {
          if !should_apply_issue_request_result(this.request_generation, generation) {
            return;
          }

          this.code_reference_cache.insert(cache_key.clone(), preview);
          this.code_reference_tasks.remove(&cache_key);
          cx.notify();
        });
      });

      self
        .code_reference_tasks
        .insert(reference.url.clone(), task);
    }
  }

  fn schedule_pending_comment_scroll(
    &mut self,
    pending_anchor: Option<ScrollAnchor>,
    comment_found: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if let Some(anchor) = pending_anchor {
      anchor.scroll_to(window, cx);
      if self.pending_comment_scroll_attempts > 1 {
        self.pending_comment_scroll_attempts -= 1;
      } else {
        self.pending_comment_scroll_id = None;
        self.pending_comment_scroll_attempts = 0;
      }
      cx.notify();
      return;
    }

    if self.pending_comment_scroll_id.is_some() && !comment_found {
      self.pending_comment_scroll_id = None;
      self.pending_comment_scroll_attempts = 0;
    }
  }

  fn handle_gfm_link(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
    if should_open_externally(window) {
      return false;
    }

    let Some(action) = parse_github_url_action(url) else {
      return false;
    };

    match action {
      CommandPaletteAction::OpenGithubRepoDetails {
        owner,
        repo,
        tab,
        issue_number,
        issue_comment_id,
      } => {
        if should_keep_issue_sheet_open_for_repo_target(
          self.owner.as_str(),
          self.repo.as_str(),
          &owner,
          &repo,
          tab,
          issue_number,
        ) && let Some(issue_number) = issue_number
        {
          match same_repo_issue_link_navigation(self.issue_number, issue_number, issue_comment_id) {
            SameRepoIssueLinkNavigation::Noop => {
              return true;
            }
            SameRepoIssueLinkNavigation::ScrollComment { comment_id } => {
              self.pending_comment_scroll_id = Some(comment_id);
              self.pending_comment_scroll_attempts = 4;
              cx.notify();
              return true;
            }
            SameRepoIssueLinkNavigation::ReloadIssue {
              issue_number,
              issue_comment_id,
            } => {
              self.pending_comment_scroll_id = issue_comment_id;
              self.pending_comment_scroll_attempts = if issue_comment_id.is_some() { 4 } else { 0 };
              self.load_issue(owner, repo, issue_number, cx);
              return true;
            }
          }
        }

        window.close_sheet(cx);
        open_repo_target(owner, repo, tab, issue_number, issue_comment_id, cx);
        true
      }
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
        open_changes_tab: _,
        review_comment_id,
      } => {
        window.close_sheet(cx);
        open_pr_target(
          owner,
          repo,
          number,
          review_comment_id.is_some(),
          review_comment_id,
          Some((self.owner.to_string(), self.repo.to_string())),
          cx,
        );
        true
      }
      _ => false,
    }
  }

  fn editable_issue_comment_ids(&self, cx: &App) -> HashSet<u64> {
    let Some(login) = current_github_login(cx) else {
      return HashSet::new();
    };
    let Some(issue) = self.issue.as_ref() else {
      return HashSet::new();
    };

    issue
      .comments
      .iter()
      .filter_map(|comment| {
        issue_details_comment_owned_by_login(comment, login.as_str()).then_some(comment.id)
      })
      .collect()
  }

  fn ensure_comment_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<InputState> {
    if let Some(input) = self.comment_input.as_ref() {
      return input.clone();
    }

    let input = cx.new(|cx| {
      InputState::new(window, cx)
        .multi_line(true)
        .rows(6)
        .placeholder("Add comment...")
    });
    self.comment_input = Some(input.clone());
    input
  }

  fn ensure_edit_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<InputState> {
    if let Some(input) = self.edit_input.as_ref() {
      return input.clone();
    }

    let input = cx.new(|cx| {
      InputState::new(window, cx)
        .multi_line(true)
        .rows(6)
        .placeholder("Edit comment...")
    });
    self.edit_input = Some(input.clone());
    input
  }

  fn ensure_description_edit_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<InputState> {
    if let Some(input) = self.description_edit_input.as_ref() {
      return input.clone();
    }

    let input = cx.new(|cx| {
      InputState::new(window, cx)
        .multi_line(true)
        .rows(6)
        .placeholder("Edit description...")
    });
    self.description_edit_input = Some(input.clone());
    input
  }

  fn clear_edit_state(&mut self) {
    self.editing_comment_id = None;
    self.edit_initial_body = None;
    self.edit_submitting = false;
    self.edit_error = None;
  }

  fn clear_description_edit_state(&mut self) {
    self.description_editing = false;
    self.description_initial_body = None;
    self.description_submitting = false;
    self.description_error = None;
  }

  fn upsert_issue_comment(&mut self, comment: crate::api::GithubIssueDetailsComment) {
    let Some(issue) = self.issue.as_mut() else {
      return;
    };
    upsert_issue_details_comment_local(&mut issue.comments, comment);
  }

  fn start_issue_description_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.comment_input_submitting || self.edit_submitting || self.description_submitting {
      return;
    }
    let Some(issue) = self.issue.as_ref() else {
      return;
    };

    let initial_body = issue.body.clone().unwrap_or_default();
    let input = self.ensure_description_edit_input(window, cx);
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

    self.description_editing = true;
    self.description_initial_body = Some(initial_body);
    self.description_error = None;
    cx.notify();
  }

  fn cancel_issue_description_edit(&mut self, cx: &mut Context<Self>) {
    if self.description_submitting || !self.description_editing {
      return;
    }
    self.clear_description_edit_state();
    cx.notify();
  }

  fn submit_issue_description_edit(&mut self, cx: &mut Context<Self>) {
    if self.comment_input_submitting
      || self.edit_submitting
      || self.description_submitting
      || !self.description_editing
    {
      return;
    }
    let Some(input) = self.description_edit_input.as_ref() else {
      return;
    };
    let initial_body = self.description_initial_body.as_deref().unwrap_or_default();
    let raw_value = input.read(cx).value().to_string();
    let Some(next_body) = next_issue_description_body(raw_value.as_str(), initial_body) else {
      self.clear_description_edit_state();
      cx.notify();
      return;
    };

    self.description_submitting = true;
    self.description_error = None;
    cx.notify();

    let owner = self.owner.clone();
    let repo = self.repo.clone();
    let issue_number = self.issue_number;
    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.update_issue_description(&owner, &repo, issue_number, &next_body))
          .await;

      let _ = this.update(cx, |this, cx| {
        this.description_submitting = false;
        match result {
          Ok(update) => {
            if let Some(issue) = this.issue.as_mut() {
              apply_issue_description_update_local(issue, update);
              let description = issue_markdown_body_or_fallback(issue.body.as_deref());
              this.description_references =
                extract_github_blob_line_references(description.as_ref());
              this.schedule_issue_code_reference_fetches(this.request_generation, cx);
            }
            this.clear_description_edit_state();
          }
          Err(error) => {
            this.description_error = Some(error.to_string().into());
          }
        }
        cx.notify();
      });
    });
    self.task = Some(task);
  }

  fn start_issue_comment_edit(
    &mut self,
    comment_id: u64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.comment_input_submitting || self.edit_submitting || self.description_submitting {
      return;
    }
    if !self.editable_issue_comment_ids(cx).contains(&comment_id) {
      return;
    }
    let Some(issue) = self.issue.as_ref() else {
      return;
    };
    let Some(comment) = issue
      .comments
      .iter()
      .find(|comment| comment.id == comment_id)
    else {
      return;
    };
    let initial_body = comment.body.clone().unwrap_or_default();

    let input = self.ensure_edit_input(window, cx);
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

    self.editing_comment_id = Some(comment_id);
    self.edit_initial_body = Some(initial_body);
    self.edit_error = None;
    cx.notify();
  }

  fn cancel_issue_comment_edit(&mut self, cx: &mut Context<Self>) {
    if self.edit_submitting || self.editing_comment_id.is_none() {
      return;
    }
    self.clear_edit_state();
    cx.notify();
  }

  fn submit_issue_comment_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.comment_input_submitting || self.edit_submitting || self.description_submitting {
      return;
    }
    let Some(input) = self.comment_input.as_ref() else {
      return;
    };
    let Some(body) = github_shared::normalize_non_empty_text(input.read(cx).value().as_str())
    else {
      return;
    };

    self.comment_input_submitting = true;
    self.comment_input_error = None;
    cx.notify();

    let owner = self.owner.clone();
    let repo = self.repo.clone();
    let issue_number = self.issue_number;
    let api = self.api.clone();
    let task = cx.spawn_in(window, async move |this, cx| {
      let result =
        unblock(move || api.create_issue_comment(&owner, &repo, issue_number, body.as_str())).await;

      let _ = this.update_in(cx, |this, window, cx| {
        this.comment_input_submitting = false;
        match result {
          Ok(comment) => {
            this.upsert_issue_comment(comment);
            this.comment_input_error = None;
            if let Some(input_state) = this.comment_input.clone() {
              input_state.update(cx, |state, cx| {
                state.set_value("", window, cx);
              });
            }
          }
          Err(error) => {
            this.comment_input_error = Some(error.to_string().into());
          }
        }
        cx.notify();
      });
    });
    self.task = Some(task);
  }

  fn submit_issue_comment_edit(&mut self, cx: &mut Context<Self>) {
    if self.comment_input_submitting || self.edit_submitting || self.description_submitting {
      return;
    }
    let Some(comment_id) = self.editing_comment_id else {
      return;
    };
    if !self.editable_issue_comment_ids(cx).contains(&comment_id) {
      return;
    }
    let Some(input) = self.edit_input.as_ref() else {
      return;
    };
    let Some(initial_body) = self.edit_initial_body.as_deref() else {
      return;
    };
    let raw_value = input.read(cx).value().to_string();
    let Some(next_body) = ({
      let next_body = github_shared::normalize_non_empty_text(raw_value.as_str());
      match next_body {
        Some(ref value) if value == initial_body.trim() => None,
        other => other,
      }
    }) else {
      self.clear_edit_state();
      cx.notify();
      return;
    };

    self.edit_submitting = true;
    self.edit_error = None;
    cx.notify();

    let owner = self.owner.clone();
    let repo = self.repo.clone();
    let issue_number = self.issue_number;
    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        api.update_issue_comment(&owner, &repo, issue_number, comment_id, next_body.as_str())
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.edit_submitting = false;
        match result {
          Ok(comment) => {
            this.upsert_issue_comment(comment);
            this.clear_edit_state();
          }
          Err(error) => {
            this.edit_error = Some(error.to_string().into());
          }
        }
        cx.notify();
      });
    });
    self.task = Some(task);
  }

  fn confirm_issue_comment_delete(
    &mut self,
    comment_id: u64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.comment_input_submitting || self.edit_submitting || self.description_submitting {
      return;
    }
    if !self.editable_issue_comment_ids(cx).contains(&comment_id) {
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
            this.submit_issue_comment_delete(comment_id, cx);
          });
          true
        })
        .build(alert)
    });
  }

  fn submit_issue_comment_delete(&mut self, comment_id: u64, cx: &mut Context<Self>) {
    if self.comment_input_submitting || self.edit_submitting || self.description_submitting {
      return;
    }
    let Some(issue) = self.issue.as_mut() else {
      return;
    };
    let Some((removed_index, removed_comment)) =
      remove_issue_details_comment_local(&mut issue.comments, comment_id)
    else {
      return;
    };
    self.comment_input_submitting = true;
    self.comment_input_error = None;
    cx.notify();

    let owner = self.owner.clone();
    let repo = self.repo.clone();
    let issue_number = self.issue_number;
    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.delete_issue_comment(&owner, &repo, issue_number, comment_id)).await;

      let _ = this.update(cx, |this, cx| {
        this.comment_input_submitting = false;
        if let Err(error) = result {
          if let Some(issue) = this.issue.as_mut() {
            restore_issue_details_comment_local(
              &mut issue.comments,
              removed_index,
              removed_comment.clone(),
            );
          }
          this.comment_input_error = Some(error.to_string().into());
        } else if this.editing_comment_id == Some(comment_id) {
          this.clear_edit_state();
        }
        cx.notify();
      });
    });
    self.task = Some(task);
  }
}

impl Focusable for GithubIssueDetailsSheetView {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for GithubIssueDetailsSheetView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let content = if self.loading {
      v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading issue details..."),
        )
        .into_any_element()
    } else if let Some(error) = self.error.clone() {
      v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(div().text_sm().text_color(theme.status_red()).child(error))
        .into_any_element()
    } else if let Some(issue) = self.issue.clone() {
      let author_name = issue_user_display_name(issue.user.as_ref());
      let opened_at = format_compact_datetime(&issue.created_at);
      let updated_at = format_compact_datetime(&issue.updated_at);
      let closed_at = issue
        .closed_at
        .as_deref()
        .map(format_compact_datetime)
        .unwrap_or_else(|| "—".into());
      let body = issue_markdown_body_or_fallback(issue.body.as_deref());
      let description_previews =
        github_code_reference_preview_map(&self.description_references, &self.code_reference_cache);
      let issue_url = github_shared::issue_url(
        &issue.repository.owner,
        &issue.repository.repo,
        issue.number,
      );
      let state_text = issue_state_label(&issue.state, issue.state_reason.clone());
      let state_color = match issue_visual_state(&issue.state, issue.state_reason.clone()) {
        GithubIssueVisualState::Open => theme.status_green(),
        GithubIssueVisualState::Completed => theme.status_violet(),
        GithubIssueVisualState::NotPlanned => theme.status_gray(),
      };
      let issue_details_view = cx.entity().clone();
      let gfm_link_handler = Arc::new(move |url: &str, window: &mut Window, cx: &mut App| {
        let handled =
          issue_details_view.update(cx, |this, cx| this.handle_gfm_link(url, window, cx));
        if handled {
          LinkAction::Handled
        } else {
          LinkAction::Open
        }
      });

      let label_tags = issue.labels.iter().map(|label| {
        Tag::secondary()
          .small()
          .rounded_full()
          .child(label.name.clone())
      });
      let editable_issue_comment_ids = self.editable_issue_comment_ids(cx);
      let comment_submission_in_flight =
        self.comment_input_submitting || self.edit_submitting || self.description_submitting;
      let editing_comment_id = self.editing_comment_id;
      let issue_details_page = cx.entity().clone();
      let comment_input = self.ensure_comment_input(window, cx);

      let mut comments_rows = v_flex().gap_2();
      let mut pending_comment_anchor: Option<ScrollAnchor> = None;
      let mut pending_comment_found = false;
      for comment in &issue.comments {
        let comment_id = comment.id;
        let comment_author = issue_user_display_name(comment.user.as_ref());
        let comment_created_at = format_compact_datetime(&comment.created_at);
        let comment_updated_at = format_compact_datetime(&comment.updated_at);
        let comment_body = issue_comment_markdown_body_or_fallback(comment.body.as_deref());
        let comment_previews = self
          .comment_references
          .get(&comment_id)
          .and_then(|references| {
            github_code_reference_preview_map(references, &self.code_reference_cache)
          });
        let comment_anchor = if self.pending_comment_scroll_id == Some(comment_id) {
          pending_comment_found = true;
          let anchor = ScrollAnchor::for_handle(self.scroll_handle.clone());
          pending_comment_anchor = Some(anchor.clone());
          Some(anchor)
        } else {
          None
        };
        let is_editable = editable_issue_comment_ids.contains(&comment_id);
        let is_editing = editing_comment_id == Some(comment_id);
        let edit_button = if is_editable && !comment_submission_in_flight && !is_editing {
          let page = issue_details_page.clone();
          Some(
            Button::new(format!("issue-sheet-comment-edit-{}", comment_id))
              .ghost()
              .xsmall()
              .compact()
              .icon(UiIconName::SquarePen)
              .tooltip("Edit comment")
              .on_click(move |_, window, cx| {
                page.update(cx, |this, cx| {
                  this.start_issue_comment_edit(comment_id, window, cx);
                });
              })
              .into_any_element(),
          )
        } else {
          None
        };
        let delete_button = if is_editable && !comment_submission_in_flight {
          let page = issue_details_page.clone();
          Some(
            Button::new(format!("issue-sheet-comment-delete-{}", comment_id))
              .ghost()
              .xsmall()
              .compact()
              .icon(IconName::Delete)
              .tooltip("Delete comment")
              .on_click(move |_, window, cx| {
                page.update(cx, |this, cx| {
                  this.confirm_issue_comment_delete(comment_id, window, cx);
                });
              })
              .into_any_element(),
          )
        } else {
          None
        };

        let comment_body_element = if is_editing {
          if let Some(input_state) = self.edit_input.clone() {
            let can_save = self
              .edit_initial_body
              .as_deref()
              .and_then(|initial| {
                let raw_value = input_state.read(cx).value().to_string();
                let next_body = github_shared::normalize_non_empty_text(raw_value.as_str())?;
                (next_body != initial.trim()).then_some(next_body)
              })
              .is_some();
            let page_for_cancel = issue_details_page.clone();
            let page_for_save = issue_details_page.clone();
            v_flex()
              .gap_2()
              .child(
                div().w_full().child(
                  Input::new(&input_state)
                    .disabled(self.edit_submitting)
                    .h(px(ISSUE_COMMENT_INPUT_HEIGHT_PX)),
                ),
              )
              .when_some(self.edit_error.clone(), |this, error| {
                this.child(div().text_xs().text_color(theme.status_red()).child(error))
              })
              .child(
                h_flex()
                  .items_center()
                  .justify_end()
                  .gap_2()
                  .child(
                    Button::new(format!("issue-sheet-comment-edit-cancel-{}", comment_id))
                      .ghost()
                      .xsmall()
                      .compact()
                      .label("Cancel")
                      .on_click(move |_, _, cx| {
                        page_for_cancel.update(cx, |this, cx| {
                          this.cancel_issue_comment_edit(cx);
                        });
                      }),
                  )
                  .child(
                    Button::new(format!("issue-sheet-comment-edit-save-{}", comment_id))
                      .xsmall()
                      .compact()
                      .label("Save")
                      .disabled(!can_save || comment_submission_in_flight)
                      .on_click(move |_, _, cx| {
                        page_for_save.update(cx, |this, cx| {
                          this.submit_issue_comment_edit(cx);
                        });
                      }),
                  ),
              )
              .into_any_element()
          } else {
            div().into_any_element()
          }
        } else {
          let mut options = MarkdownRenderOptions::with_on_link(gfm_link_handler.clone())
            .with_state(self.markdown_state.clone())
            .with_github_issue_reference_context(
              issue.repository.owner.as_str(),
              issue.repository.repo.as_str(),
            )
            .with_scope_id(issue_comment_scope_id(issue.id, comment_id));
          if let Some(previews) = comment_previews {
            options = options.with_github_code_reference_previews(previews);
          }
          render_markdown(comment_body.as_ref(), &options, cx)
        };

        comments_rows = comments_rows.child(
          v_flex()
            .id(format!("github-issue-comment-{}", comment_id))
            .anchor_scroll(comment_anchor)
            .gap_2()
            .p_3()
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius)
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
                      Avatar::new()
                        .name(comment_author.clone())
                        .when_some(
                          comment
                            .user
                            .as_ref()
                            .and_then(|user| user.avatar_url.clone()),
                          |this, url| this.src(url),
                        )
                        .small(),
                    )
                    .child(
                      div()
                        .text_sm()
                        .text_color(theme.foreground)
                        .child(comment_author.clone()),
                    ),
                )
                .child(
                  h_flex()
                    .items_center()
                    .gap_1()
                    .when_some(edit_button, |this, button| this.child(button))
                    .when_some(delete_button, |this, button| this.child(button)),
                ),
            )
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!(
                  "Created {comment_created_at} • Updated {comment_updated_at}"
                )),
            )
            .child(comment_body_element),
        );
      }

      self.schedule_pending_comment_scroll(
        pending_comment_anchor,
        pending_comment_found,
        window,
        cx,
      );

      v_flex()
        .w_full()
        .gap_3()
        .pt_3()
        .pb_8()
        .px_6()
        .child(
          div()
            .text_lg()
            .font_semibold()
            .whitespace_normal()
            .child(issue.title.clone()),
        )
        .child(
          h_flex()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(format!("#{}", issue.number))
            .child(StatusTag::new(state_color).child(state_text)),
        )
        .when(!issue.labels.is_empty(), |this| {
          this.child(h_flex().gap_1().flex_wrap().children(label_tags))
        })
        .child(
          h_flex()
            .items_center()
            .gap_2()
            .child(
              Avatar::new()
                .name(author_name.clone())
                .when_some(
                  issue.user.as_ref().and_then(|user| user.avatar_url.clone()),
                  |this, url| this.src(url),
                )
                .small(),
            )
            .child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(author_name),
            ),
        )
        .child(
          v_flex()
            .gap_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("Opened {opened_at}"))
            .child(format!("Updated {updated_at}"))
            .child(format!("Closed {closed_at}")),
        )
        .child(
          Button::new("issue-details-open-on-github")
            .icon(IconName::ExternalLink)
            .small()
            .label("Open on GitHub")
            .on_click(move |_, _, cx| {
              cx.open_url(&issue_url);
            }),
        )
        .child(
          v_flex()
            .gap_2()
            .child(
              h_flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(div().text_sm().font_semibold().child("Description"))
                .child(
                  Button::new(format!("issue-sheet-description-edit-{}", issue.id))
                    .ghost()
                    .xsmall()
                    .compact()
                    .icon(UiIconName::SquarePen)
                    .tooltip("Edit description")
                    .disabled(comment_submission_in_flight || self.description_editing)
                    .on_click({
                      let page = issue_details_page.clone();
                      move |_, window, cx| {
                        page.update(cx, |this, cx| {
                          this.start_issue_description_edit(window, cx);
                        });
                      }
                    }),
                ),
            )
            .child(if self.description_editing {
              if let Some(input_state) = self.description_edit_input.clone() {
                let can_save = self
                  .description_initial_body
                  .as_deref()
                  .and_then(|initial| {
                    let raw_value = input_state.read(cx).value().to_string();
                    next_issue_description_body(raw_value.as_str(), initial)
                  })
                  .is_some();
                let page_for_cancel = issue_details_page.clone();
                let page_for_save = issue_details_page.clone();
                v_flex()
                  .gap_2()
                  .child(
                    div().w_full().child(
                      Input::new(&input_state)
                        .disabled(self.description_submitting)
                        .h(px(ISSUE_DESCRIPTION_INPUT_HEIGHT_PX)),
                    ),
                  )
                  .when_some(self.description_error.clone(), |this, error| {
                    this.child(div().text_xs().text_color(theme.status_red()).child(error))
                  })
                  .child(
                    h_flex()
                      .items_center()
                      .justify_end()
                      .gap_2()
                      .child(
                        Button::new(format!("issue-sheet-description-cancel-{}", issue.id))
                          .ghost()
                          .xsmall()
                          .compact()
                          .label("Cancel")
                          .disabled(self.description_submitting)
                          .on_click(move |_, _, cx| {
                            page_for_cancel.update(cx, |this, cx| {
                              this.cancel_issue_description_edit(cx);
                            });
                          }),
                      )
                      .child(
                        Button::new(format!("issue-sheet-description-save-{}", issue.id))
                          .xsmall()
                          .compact()
                          .label("Save")
                          .disabled(!can_save || self.description_submitting)
                          .on_click(move |_, _, cx| {
                            page_for_save.update(cx, |this, cx| {
                              this.submit_issue_description_edit(cx);
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
                  let mut options = MarkdownRenderOptions::with_on_link(gfm_link_handler)
                    .with_state(self.markdown_state.clone())
                    .with_github_issue_reference_context(
                      issue.repository.owner.as_str(),
                      issue.repository.repo.as_str(),
                    )
                    .with_scope_id(issue_description_scope_id(issue.id));
                  if let Some(previews) = description_previews.clone() {
                    options = options.with_github_code_reference_previews(previews);
                  }
                  render_markdown(body.as_ref(), &options, cx)
                })
                .into_any_element()
            }),
        )
        .child(
          v_flex()
            .gap_2()
            .child(div().text_sm().font_semibold().child("Comments"))
            .when(issue.comments.is_empty(), |this| {
              this.child(
                div()
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .child("No comments yet"),
              )
            })
            .when(!issue.comments.is_empty(), |this| this.child(comments_rows))
            .child(
              v_flex()
                .gap_2()
                .pt_2()
                .child(
                  div()
                    .w_full()
                    .child(Input::new(&comment_input).h(px(ISSUE_COMMENT_INPUT_HEIGHT_PX))),
                )
                .when_some(self.comment_input_error.clone(), |this, error| {
                  this.child(div().text_xs().text_color(theme.status_red()).child(error))
                })
                .child(
                  h_flex().items_center().justify_end().gap_2().child(
                    Button::new("issue-sheet-comment-create")
                      .xsmall()
                      .compact()
                      .label("Comment")
                      .disabled(
                        comment_submission_in_flight
                          || github_shared::normalize_non_empty_text(
                            comment_input.read(cx).value().as_str(),
                          )
                          .is_none(),
                      )
                      .on_click({
                        let page = issue_details_page.clone();
                        move |_, window, cx| {
                          page.update(cx, |this, cx| {
                            this.submit_issue_comment_create(window, cx);
                          });
                        }
                      }),
                  ),
                ),
            ),
        )
        .into_any_element()
    } else {
      v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child("No issue selected")
        .into_any_element()
    };

    div()
      .id("github-issue-details-sheet-scroll")
      .size_full()
      .relative()
      .track_scroll(&self.scroll_handle)
      .overflow_y_scroll()
      .vertical_scrollbar(&self.scroll_handle)
      .track_focus(&self.focus_handle)
      .child(content)
  }
}

pub struct GithubRepoPage {
  focus_handle: FocusHandle,
  window_handle: AnyWindowHandle,
  api: ApiClient,
  owner: SharedString,
  repo: SharedString,
  branch_select: Entity<SelectState<SearchableVec<GithubRepoBranchSelectItem>>>,
  selected_branch: Option<SharedString>,
  branches_loading: bool,
  branches_error: Option<SharedString>,
  branches_task: Option<Task<()>>,
  branches_request_generation: u64,
  repository: Option<GithubRepositoryDetails>,
  repository_loading: bool,
  repository_error: Option<SharedString>,
  repository_task: Option<Task<()>>,
  readme_request_generation: u64,
  readme_loading: bool,
  readme_error: Option<SharedString>,
  readme_task: Option<Task<()>>,
  readme_content: Option<SharedString>,
  readme_path: Option<SharedString>,
  readme_loaded_branch: Option<SharedString>,
  readme_markdown_state: MarkdownRenderState,
  code_request_generation: u64,
  code_tree_state: Entity<TreeState>,
  code_files_loading: bool,
  code_files_error: Option<SharedString>,
  code_tree_task: Option<Task<()>>,
  code_lookup: HashMap<String, Rc<GithubRepoCodeFile>>,
  code_selected_file: Option<Rc<GithubRepoCodeFile>>,
  code_selected_tree_id: Option<String>,
  code_file_loading: bool,
  code_file_error: Option<SharedString>,
  code_file_contents_cache: HashMap<String, Option<String>>,
  code_file_tasks: HashMap<String, Task<()>>,
  code_editor: Entity<Editor>,
  show_markdown_preview: bool,
  svg_preview: Option<Result<Arc<RenderImage>, SharedString>>,
  svg_preview_source: Option<SharedString>,
  svg_preview_task: Option<Task<()>>,
  pull_requests: Entity<ListState<GithubRepoPullRequestListDelegate>>,
  pull_requests_error: Option<SharedString>,
  pull_requests_task: Option<Task<()>>,
  issues: Entity<ListState<GithubRepoIssueListDelegate>>,
  issues_error: Option<SharedString>,
  issues_task: Option<Task<()>>,
  issue_sheet_width_px: f32,
  active_tab_ix: usize,
  pending_issue_sheet_number: Option<u64>,
  pending_issue_sheet_comment_id: Option<u64>,
  _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Default)]
pub struct GithubRepoPageHandle {
  page: Option<gpui::WeakEntity<GithubRepoPage>>,
}

impl gpui::Global for GithubRepoPageHandle {}

impl GithubRepoPageHandle {
  pub fn register(cx: &mut Context<GithubRepoPage>) {
    cx.set_global(Self {
      page: Some(cx.entity().downgrade()),
    });
  }

  pub fn show(owner: SharedString, repo: SharedString, cx: &mut App) {
    Self::show_with_target(owner, repo, GithubRepoOpenTarget::Overview, cx);
  }

  pub fn show_pull_requests(owner: SharedString, repo: SharedString, cx: &mut App) {
    Self::show_with_target(owner, repo, GithubRepoOpenTarget::PullRequests, cx);
  }

  pub fn show_issues(
    owner: SharedString,
    repo: SharedString,
    issue_number: Option<u64>,
    issue_comment_id: Option<u64>,
    cx: &mut App,
  ) {
    Self::show_with_target(
      owner,
      repo,
      GithubRepoOpenTarget::Issues {
        issue_number,
        issue_comment_id,
      },
      cx,
    );
  }

  fn show_with_target(
    owner: SharedString,
    repo: SharedString,
    target: GithubRepoOpenTarget,
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
    let _ = weak.update(cx, |this, cx| {
      this.load_repository(owner_string, repo_string, target, cx);
    });

    WorkspaceRoute::global_mut(cx).page = WorkspacePage::GithubRepo;
    cx.refresh_windows();
  }
}

impl GithubRepoPage {
  fn build_detached_code_editor(
    path: impl Into<PathBuf>,
    cx: &mut Context<Self>,
  ) -> Entity<Editor> {
    let editor_path = path.into();
    let load_root = PathBuf::from(".");
    let load_path = PathBuf::from(".reviu-github-repo-preview").join(&editor_path);
    let loaded = Editor::load_file_for_editor(&load_root, &load_path);
    let detached_root = PathBuf::from(".reviu-github-repo-editor-root");

    cx.new(move |cx| {
      let mut editor = Editor::new_with_loaded_file(detached_root, editor_path, loaded, cx);
      editor.is_read_only = true;
      editor
    })
  }

  fn open_github_home(cx: &mut App) {
    let (target, should_refresh) = github_page_navigation(AuthStateStore::has_pro_access(cx));

    if should_refresh {
      GithubPageHandle::refresh(cx);
    }

    match target {
      WorkspacePage::Github => WorkspaceRoute::open_github(cx),
      WorkspacePage::Billing => WorkspaceRoute::open_billing(cx),
      _ => {}
    }
    cx.refresh_windows();
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    GithubRepoPageHandle::register(cx);

    let code_tree_state = cx.new(|cx| TreeState::new(cx));
    let branch_select = cx.new(|cx| {
      SelectState::new(
        SearchableVec::new(Vec::<GithubRepoBranchSelectItem>::new()),
        None,
        window,
        cx,
      )
      .searchable(true)
    });
    let pull_requests = cx.new(|cx| {
      ListState::new(GithubRepoPullRequestListDelegate::new(), window, cx).searchable(true)
    });
    let issues =
      cx.new(|cx| ListState::new(GithubRepoIssueListDelegate::new(), window, cx).searchable(true));
    let code_editor = Self::build_detached_code_editor("__reviu_github_repo_placeholder__.txt", cx);

    let api = WorkspaceApi::global(cx).api.clone();
    let mut this = Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      api,
      owner: "".into(),
      repo: "".into(),
      branch_select,
      selected_branch: None,
      branches_loading: false,
      branches_error: None,
      branches_task: None,
      branches_request_generation: 0,
      repository: None,
      repository_loading: false,
      repository_error: None,
      repository_task: None,
      readme_request_generation: 0,
      readme_loading: false,
      readme_error: None,
      readme_task: None,
      readme_content: None,
      readme_path: None,
      readme_loaded_branch: None,
      readme_markdown_state: MarkdownRenderState::new(),
      code_request_generation: 0,
      code_tree_state,
      code_files_loading: false,
      code_files_error: None,
      code_tree_task: None,
      code_lookup: HashMap::new(),
      code_selected_file: None,
      code_selected_tree_id: None,
      code_file_loading: false,
      code_file_error: None,
      code_file_contents_cache: HashMap::new(),
      code_file_tasks: HashMap::new(),
      code_editor,
      show_markdown_preview: false,
      svg_preview: None,
      svg_preview_source: None,
      svg_preview_task: None,
      pull_requests,
      pull_requests_error: None,
      pull_requests_task: None,
      issues,
      issues_error: None,
      issues_task: None,
      issue_sheet_width_px: ISSUE_DETAILS_SHEET_WIDTH_PX,
      active_tab_ix: 0,
      pending_issue_sheet_number: None,
      pending_issue_sheet_comment_id: None,
      _subscriptions: Vec::new(),
    };

    this.subscribe_to_pull_requests(cx);
    this.subscribe_to_issues(window, cx);
    this.subscribe_to_branch_select(cx);
    this
  }

  fn subscribe_to_pull_requests(&mut self, cx: &mut Context<Self>) {
    let subscription = cx.subscribe(&self.pull_requests, |this, state, event: &ListEvent, cx| {
      if let ListEvent::Confirm(ix) = event {
        let row = state.read(cx).delegate().matched_rows.get(ix.row).cloned();
        if let Some(row) = row {
          GithubPrDetailsPageHandle::show_with_repo_return(
            row.pr.repository.owner.clone().into(),
            row.pr.repository.repo.clone().into(),
            row.pr.number,
            this.owner.clone(),
            this.repo.clone(),
            cx,
          );
        }
      }
    });

    self._subscriptions.push(subscription);
  }

  fn subscribe_to_issues(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let subscription = cx.subscribe_in(
      &self.issues,
      window,
      |this, state, event: &ListEvent, window, cx| {
        if let ListEvent::Confirm(ix) = event {
          let row = state.read(cx).delegate().matched_rows.get(ix.row).cloned();
          if let Some(row) = row {
            this.open_issue_details_sheet(row.issue.clone(), None, window, cx);
          }
        }
      },
    );

    self._subscriptions.push(subscription);
  }

  fn subscribe_to_branch_select(&mut self, cx: &mut Context<Self>) {
    cx.subscribe(
      &self.branch_select,
      |this, _state, event: &SelectEvent<SearchableVec<GithubRepoBranchSelectItem>>, cx| {
        if let SelectEvent::Confirm(Some(branch)) = event {
          this.set_selected_branch(branch.clone(), cx);
        }
      },
    )
    .detach();
  }

  fn set_branch_select_items(
    &mut self,
    items: Vec<GithubRepoBranchSelectItem>,
    selected_branch: Option<String>,
    cx: &mut Context<Self>,
  ) {
    let branch_select = self.branch_select.clone();
    let window_handle = self.window_handle;
    let _ = cx.update_window(window_handle, move |_, window, cx| {
      branch_select.update(cx, |state, cx| {
        state.set_items(SearchableVec::new(items), window, cx);
        if let Some(selected_branch) = selected_branch.as_ref() {
          state.set_selected_value(selected_branch, window, cx);
        } else {
          state.set_selected_index(None, window, cx);
        }
      });
    });
  }

  fn set_branch_select_selected_value(&mut self, selected_branch: String, cx: &mut Context<Self>) {
    let branch_select = self.branch_select.clone();
    let window_handle = self.window_handle;
    let _ = cx.update_window(window_handle, move |_, window, cx| {
      branch_select.update(cx, |state, cx| {
        state.set_selected_value(&selected_branch, window, cx);
      });
    });
  }

  fn reset_branch_state(&mut self, cx: &mut Context<Self>) {
    self.branches_loading = false;
    self.branches_error = None;
    self.branches_task = None;
    self.selected_branch = None;
    self.branches_request_generation = self.branches_request_generation.wrapping_add(1);
    self.set_branch_select_items(Vec::new(), None, cx);
  }

  fn effective_branch(&self) -> Option<String> {
    effective_repo_branch(
      self.selected_branch.as_ref().map(SharedString::as_ref),
      self
        .repository
        .as_ref()
        .map(|repository| repository.default_branch.as_str()),
    )
  }

  fn set_selected_branch(&mut self, branch: String, cx: &mut Context<Self>) {
    let Some(next_branch) = normalize_non_empty_string(&branch) else {
      return;
    };
    let current_branch = self
      .selected_branch
      .as_ref()
      .and_then(|branch| normalize_non_empty_string(branch.as_ref()));
    if current_branch.as_deref() == Some(next_branch.as_str()) {
      return;
    }

    self.selected_branch = Some(next_branch.clone().into());
    self.set_branch_select_selected_value(next_branch, cx);
    self.reset_readme_state(cx);
    self.reset_code_state(cx);
    if self.active_tab_ix == REPO_TAB_README_IX {
      self.load_readme_if_needed(cx);
    }
    if self.active_tab_ix == REPO_TAB_CODE_IX {
      self.load_code_tree_if_needed(cx);
    }
    cx.notify();
  }

  fn load_repository_branches(&mut self, cx: &mut Context<Self>) {
    let owner = self.owner.to_string();
    let repo = self.repo.to_string();
    if owner.trim().is_empty() || repo.trim().is_empty() {
      return;
    }

    self.branches_loading = true;
    self.branches_error = None;
    self.branches_request_generation = self.branches_request_generation.wrapping_add(1);
    let request_generation = self.branches_request_generation;

    let api = self.api.clone();
    let owner_for_task = owner.clone();
    let repo_for_task = repo.clone();
    let owner_for_fetch = owner_for_task.clone();
    let repo_for_fetch = repo_for_task.clone();
    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.fetch_github_repository_branches(&owner_for_fetch, &repo_for_fetch))
          .await;

      let _ = this.update(cx, |this, cx| {
        if !this
          .owner
          .as_ref()
          .eq_ignore_ascii_case(owner_for_task.as_str())
          || !this
            .repo
            .as_ref()
            .eq_ignore_ascii_case(repo_for_task.as_str())
          || !should_apply_repo_request_result(this.branches_request_generation, request_generation)
        {
          return;
        }

        this.branches_task = None;
        this.branches_loading = false;

        match result {
          Ok(branches) => {
            let branch_names = branches
              .into_iter()
              .map(|branch| branch.name)
              .collect::<Vec<_>>();
            let selected_branch = this.effective_branch();
            this.selected_branch = selected_branch.clone().map(Into::into);
            let items = build_repo_branch_select_items(branch_names, selected_branch.as_deref());
            this.set_branch_select_items(items, selected_branch, cx);
            this.branches_error = None;
          }
          Err(error) => {
            let message = error.to_string();
            if github_shared::is_unauthorized_error_message(&message) {
              this.branches_error = Some("Authentication required. Please sign in again.".into());
            } else {
              this.branches_error = Some(message.into());
            }
          }
        }

        cx.notify();
      });
    });

    self.branches_task = Some(task);
    cx.notify();
  }

  fn open_issue_details_sheet(
    &mut self,
    issue: Rc<GithubIssue>,
    issue_comment_id: Option<u64>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.pending_issue_sheet_number = None;
    self.pending_issue_sheet_comment_id = None;
    let issue_number = issue.number;
    let issue_details_view = cx.new(|cx| {
      GithubIssueDetailsSheetView::new(
        self.api.clone(),
        issue.repository.owner.clone(),
        issue.repository.repo.clone(),
        issue_number,
        issue_comment_id,
        cx,
      )
    });

    let issues_list = self.issues.clone();
    let repo_page = cx.entity().clone();
    window.open_sheet_at(Placement::Right, cx, move |sheet, _, cx| {
      let width = repo_page.read(cx).issue_sheet_width_px;

      sheet
        .overlay(true)
        .overlay_closable(true)
        .resizable(true)
        .size(px(width))
        .title(format!("Issue #{issue_number}"))
        .on_close({
          let issues_list = issues_list.clone();
          move |_, window, cx| {
            issues_list.update(cx, |state, cx| {
              state.focus(window, cx);
            });
          }
        })
        .child(
          div()
            .id(("github-repo-issue-sheet-content", issue_number))
            .size_full()
            .relative()
            .child(
              div()
                .id(("github-repo-issue-sheet-resize-handle", issue_number))
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(-16.0))
                .w(px(6.0))
                .cursor(gpui::CursorStyle::ResizeColumn)
                .on_drag(
                  IssueSheetResizeDrag,
                  |drag: &IssueSheetResizeDrag, _, _, cx: &mut App| cx.new(|_| drag.clone()),
                )
                .on_drag_move::<IssueSheetResizeDrag>({
                  let repo_page = repo_page.clone();
                  move |event: &gpui::DragMoveEvent<IssueSheetResizeDrag>,
                        window: &mut Window,
                        cx: &mut App| {
                    let next_width = issue_sheet_width_from_cursor_x(
                      f32::from(window.viewport_size().width),
                      f32::from(event.event.position.x),
                    );
                    repo_page.update(cx, |this, cx| {
                      if (this.issue_sheet_width_px - next_width).abs() <= f32::EPSILON {
                        return;
                      }
                      this.issue_sheet_width_px = next_width;
                      cx.notify();
                    });
                  }
                }),
            )
            .child(issue_details_view.clone()),
        )
    });
  }

  fn set_active_tab(&mut self, tab_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
    if self.active_tab_ix == tab_ix {
      return;
    }
    self.active_tab_ix = tab_ix;
    if tab_ix != REPO_TAB_ISSUES_IX {
      self.pending_issue_sheet_number = None;
      self.pending_issue_sheet_comment_id = None;
    }

    if tab_ix == REPO_TAB_README_IX {
      self.load_readme_if_needed(cx);
      cx.notify();
      return;
    }

    if tab_ix == REPO_TAB_CODE_IX {
      self.load_code_tree_if_needed(cx);
      cx.notify();
      return;
    }

    cx.notify();

    if tab_ix == REPO_TAB_PULL_REQUESTS_IX {
      cx.on_next_frame(window, |this, window, cx| {
        this.pull_requests.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      });
      return;
    }

    if tab_ix == REPO_TAB_ISSUES_IX {
      cx.on_next_frame(window, |this, window, cx| {
        this.issues.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      });
    }
  }

  fn reset_readme_state(&mut self, _cx: &mut Context<Self>) {
    self.readme_request_generation = self.readme_request_generation.wrapping_add(1);
    self.readme_loading = false;
    self.readme_error = None;
    self.readme_task = None;
    self.readme_content = None;
    self.readme_path = None;
    self.readme_loaded_branch = None;
    self.readme_markdown_state = MarkdownRenderState::new();
  }

  fn load_readme_if_needed(&mut self, cx: &mut Context<Self>) {
    if self.active_tab_ix != REPO_TAB_README_IX {
      return;
    }
    if self.readme_loading || self.readme_task.is_some() {
      return;
    }
    let Some(_repository) = self.repository.as_ref() else {
      return;
    };

    let owner = self.owner.to_string();
    let repo = self.repo.to_string();
    let Some(branch) = self.effective_branch() else {
      return;
    };
    if owner.trim().is_empty() || repo.trim().is_empty() || branch.trim().is_empty() {
      return;
    }
    if !should_fetch_readme_for_branch(
      self
        .readme_loaded_branch
        .as_deref()
        .map(|branch| branch.as_ref()),
      &branch,
      self.readme_error.is_some(),
    ) {
      return;
    }

    self.readme_loading = true;
    self.readme_error = None;
    self.readme_request_generation = self.readme_request_generation.wrapping_add(1);
    let request_generation = self.readme_request_generation;

    let api = self.api.clone();
    let owner_for_task = owner.clone();
    let repo_for_task = repo.clone();
    let branch_for_task = branch.clone();
    let owner_for_fetch = owner_for_task.clone();
    let repo_for_fetch = repo_for_task.clone();
    let branch_for_fetch = branch_for_task.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        api.fetch_github_repository_readme(
          &owner_for_fetch,
          &repo_for_fetch,
          Some(branch_for_fetch.as_str()),
        )
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        if !this
          .owner
          .as_ref()
          .eq_ignore_ascii_case(owner_for_task.as_str())
          || !this
            .repo
            .as_ref()
            .eq_ignore_ascii_case(repo_for_task.as_str())
          || !should_apply_repo_request_result(this.readme_request_generation, request_generation)
          || this.effective_branch().as_deref() != Some(branch_for_task.as_str())
        {
          return;
        }

        this.readme_task = None;
        this.readme_loading = false;

        match result {
          Ok(readme) => {
            this.readme_content = readme
              .as_ref()
              .and_then(|readme| readme.content.as_ref())
              .map(ToString::to_string)
              .map(SharedString::from);
            this.readme_path = readme
              .and_then(|readme| readme.path)
              .map(SharedString::from);
            this.readme_loaded_branch = Some(branch_for_task.clone().into());
            this.readme_error = None;
          }
          Err(error) => {
            let message = error.to_string();
            this.readme_content = None;
            this.readme_path = None;
            this.readme_loaded_branch = None;
            if github_shared::is_unauthorized_error_message(&message) {
              this.readme_error = Some("Authentication required. Please sign in again.".into());
            } else {
              this.readme_error = Some(message.into());
            }
          }
        }

        cx.notify();
      });
    });

    self.readme_task = Some(task);
    cx.notify();
  }

  fn reset_code_state(&mut self, cx: &mut Context<Self>) {
    self.code_request_generation = self.code_request_generation.wrapping_add(1);
    self.code_files_loading = false;
    self.code_files_error = None;
    self.code_tree_task = None;
    self.code_lookup.clear();
    self.code_selected_file = None;
    self.code_selected_tree_id = None;
    self.code_file_loading = false;
    self.code_file_error = None;
    self.code_file_contents_cache.clear();
    self.code_file_tasks.clear();
    self.show_markdown_preview = false;
    self.svg_preview = None;
    self.svg_preview_source = None;
    self.svg_preview_task = None;
    self.code_tree_state.update(cx, |state, cx| {
      state.set_items(Vec::new(), cx);
      state.set_selected_index(None, cx);
    });
    self.clear_code_editor(cx);
  }

  fn load_code_tree_if_needed(&mut self, cx: &mut Context<Self>) {
    if !should_prefetch_code_tree_for_tab(self.active_tab_ix) {
      return;
    }
    if self.code_files_loading || self.code_tree_task.is_some() || !self.code_lookup.is_empty() {
      return;
    }
    let Some(_repository) = self.repository.as_ref() else {
      return;
    };

    let owner = self.owner.to_string();
    let repo = self.repo.to_string();
    let Some(branch) = self.effective_branch() else {
      return;
    };
    if owner.trim().is_empty() || repo.trim().is_empty() || branch.trim().is_empty() {
      return;
    }

    self.code_files_loading = true;
    self.code_files_error = None;
    self.code_file_loading = false;
    self.code_file_error = None;
    self.code_lookup.clear();
    self.code_selected_file = None;
    self.code_selected_tree_id = None;
    self.code_file_contents_cache.clear();
    self.code_file_tasks.clear();
    self.code_tree_state.update(cx, |state, cx| {
      state.set_items(Vec::new(), cx);
      state.set_selected_index(None, cx);
    });
    self.clear_code_editor(cx);
    self.code_request_generation = self.code_request_generation.wrapping_add(1);
    let request_generation = self.code_request_generation;

    let api = self.api.clone();
    let owner_for_task = owner.clone();
    let repo_for_task = repo.clone();
    let branch_for_task = branch.clone();
    let owner_for_fetch = owner_for_task.clone();
    let repo_for_fetch = repo_for_task.clone();
    let branch_for_fetch = branch_for_task.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        api.fetch_github_repository_tree(&owner_for_fetch, &repo_for_fetch, &branch_for_fetch)
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        if !this
          .owner
          .as_ref()
          .eq_ignore_ascii_case(owner_for_task.as_str())
          || !this
            .repo
            .as_ref()
            .eq_ignore_ascii_case(repo_for_task.as_str())
          || !should_apply_repo_request_result(this.code_request_generation, request_generation)
          || this.effective_branch().as_deref() != Some(branch_for_task.as_str())
        {
          return;
        }

        this.code_tree_task = None;
        this.code_files_loading = false;

        match result {
          Ok(tree_payload) => {
            let files: Vec<Rc<GithubRepoCodeFile>> = tree_payload
              .tree
              .into_iter()
              .filter(|entry| entry.entry_type.eq_ignore_ascii_case("blob"))
              .map(|entry| {
                Rc::new(GithubRepoCodeFile {
                  path: entry.path.into(),
                  sha: entry.sha.into(),
                })
              })
              .collect();

            let (items, lookup, _selected_index, _selected_id) = build_repo_code_tree_items(&files);
            this.code_lookup = lookup;
            this.code_selected_tree_id = None;
            this.code_tree_state.update(cx, |state, cx| {
              state.set_items(items, cx);
              state.set_selected_index(None, cx);
            });
            this.set_selected_code_file(None, cx);
            this.code_files_error = None;
          }
          Err(error) => {
            this.code_lookup.clear();
            this.code_selected_file = None;
            this.code_selected_tree_id = None;
            this.code_tree_state.update(cx, |state, cx| {
              state.set_items(Vec::new(), cx);
              state.set_selected_index(None, cx);
            });
            let message = error.to_string();
            if github_shared::is_unauthorized_error_message(&message) {
              this.code_files_error = Some("Authentication required. Please sign in again.".into());
            } else {
              this.code_files_error = Some(message.into());
            }
          }
        }

        cx.notify();
      });
    });
    self.code_tree_task = Some(task);
    cx.notify();
  }

  fn set_selected_code_file(
    &mut self,
    selected: Option<Rc<GithubRepoCodeFile>>,
    cx: &mut Context<Self>,
  ) {
    let current_id = self
      .code_selected_file
      .as_ref()
      .map(|file| file.path.as_ref().to_string());
    let next_id = selected.as_ref().map(|file| file.path.as_ref().to_string());
    if current_id == next_id {
      return;
    }

    self.code_selected_file = selected.clone();
    self.code_selected_tree_id = selected.as_ref().map(|file| file.path.as_ref().to_string());
    if !self.selected_code_file_is_markdown() && !self.selected_code_file_is_svg() {
      self.show_markdown_preview = false;
    }
    self.svg_preview = None;
    self.svg_preview_source = None;

    if let Some(file) = selected {
      self.ensure_code_editor_for_path(file.path.as_ref(), cx);
      self.sync_code_tree_selection(cx);

      let key = file.path.as_ref().to_string();
      if let Some(content) = self.code_file_contents_cache.get(&key).cloned() {
        if let Some(content) = content {
          self.apply_code_editor_content(&content, cx);
          self.code_file_error = None;
        } else {
          self.code_file_loading = false;
          self.code_file_error = Some("File contents unavailable".into());
          self.clear_code_editor(cx);
        }
      } else {
        self.code_file_loading = true;
        self.code_file_error = None;
        self.clear_code_editor(cx);
        self.maybe_fetch_code_file_content(file, cx);
      }
    } else {
      self.code_file_loading = false;
      self.code_file_error = None;
      self.clear_code_editor(cx);
    }

    cx.notify();
  }

  fn maybe_fetch_code_file_content(
    &mut self,
    file: Rc<GithubRepoCodeFile>,
    cx: &mut Context<Self>,
  ) {
    let key = file.path.as_ref().to_string();
    if self.code_file_contents_cache.contains_key(&key) || self.code_file_tasks.contains_key(&key) {
      return;
    }

    let Some(reference) = self.effective_branch() else {
      return;
    };

    let owner = self.owner.to_string();
    let repo = self.repo.to_string();
    let path = key.clone();
    let key_for_task = key.clone();
    let api = self.api.clone();
    let owner_for_fetch = owner.clone();
    let repo_for_fetch = repo.clone();
    let path_for_fetch = path.clone();
    let reference_for_fetch = reference.clone();
    let request_generation = self.code_request_generation;

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        api.fetch_github_file_content(
          &owner_for_fetch,
          &repo_for_fetch,
          &path_for_fetch,
          &reference_for_fetch,
        )
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        if !this.owner.as_ref().eq_ignore_ascii_case(owner.as_str())
          || !this.repo.as_ref().eq_ignore_ascii_case(repo.as_str())
          || !should_apply_repo_request_result(this.code_request_generation, request_generation)
          || this.effective_branch().as_deref() != Some(reference.as_str())
        {
          return;
        }

        this.code_file_tasks.remove(&key_for_task);
        match result {
          Ok(content) => {
            this
              .code_file_contents_cache
              .insert(key_for_task.clone(), content.clone());
            if this.code_selected_tree_id.as_deref() == Some(key_for_task.as_str()) {
              if let Some(content) = content {
                this.apply_code_editor_content(&content, cx);
                this.code_file_error = None;
              } else {
                this.code_file_loading = false;
                this.code_file_error = Some("File contents unavailable".into());
                this.clear_code_editor(cx);
              }
            }
          }
          Err(_) => {
            if this.code_selected_tree_id.as_deref() == Some(key_for_task.as_str()) {
              this.code_file_loading = false;
              this.code_file_error = Some("Failed to load file contents".into());
              this.clear_code_editor(cx);
            }
          }
        }

        cx.notify();
      });
    });

    self.code_file_tasks.insert(key, task);
  }

  fn selected_code_file_is_markdown(&self) -> bool {
    self
      .code_selected_file
      .as_ref()
      .map(|file| is_markdown_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  fn selected_code_file_is_svg(&self) -> bool {
    self
      .code_selected_file
      .as_ref()
      .map(|file| is_svg_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  fn toggle_code_markdown_preview(&mut self, cx: &mut Context<Self>) {
    if !self.selected_code_file_is_markdown() && !self.selected_code_file_is_svg() {
      return;
    }

    self.show_markdown_preview = !self.show_markdown_preview;
    cx.notify();
  }

  fn update_code_svg_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.show_markdown_preview || !self.selected_code_file_is_svg() {
      return;
    }

    let document = self.code_editor.read(cx).document().read(cx);
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

  fn show_file_search_action(
    &mut self,
    _: &ShowFileSearch,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != REPO_TAB_CODE_IX {
      return;
    }

    self.open_code_file_search_palette(window, cx);
  }

  fn open_code_file_search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.code_lookup.is_empty() {
      return;
    }

    let entries = self
      .code_lookup
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
        view.select_code_file_from_palette(&path, cx);
      });

      let view_for_focus = view.clone();
      window.on_next_frame(move |window, cx| {
        let focus_handle = view_for_focus
          .read(cx)
          .code_editor
          .read(cx)
          .focus_handle(cx);
        window.focus(&focus_handle, cx);
      });

      Ok(())
    });
    open_shared_file_search_palette(window, cx, entries, handler, true);
  }

  fn select_code_file_from_palette(&mut self, path: &Path, cx: &mut Context<Self>) {
    let key = path.to_string_lossy().to_string();
    let Some(file) = self.code_lookup.get(&key).cloned() else {
      return;
    };

    let tree_item = TreeItem::new(key.clone(), key.clone());
    self.code_tree_state.update(cx, |state, cx| {
      state.set_selected_item(Some(&tree_item), cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
    });

    self.set_selected_code_file(Some(file), cx);
  }

  fn ensure_code_editor_for_path(&mut self, path: &str, cx: &mut Context<Self>) {
    let desired_path = PathBuf::from(path);
    let mut current_path = None;
    self.code_editor.update(cx, |editor, _| {
      current_path = Some(editor.workdir_path.clone());
    });
    if current_path.as_ref() == Some(&desired_path) {
      return;
    }

    self.code_editor = Self::build_detached_code_editor(desired_path, cx);
  }

  fn clear_code_editor(&mut self, cx: &mut Context<Self>) {
    self.code_editor.update(cx, |editor, cx| {
      editor.document().update(cx, |doc, cx| {
        doc.replace_all("", cx);
      });
      editor.reset_after_replace();
      editor.reset_selection(cx);
      editor.set_diffs(None, cx);
      editor.is_read_only = true;
    });
  }

  fn apply_code_editor_content(&mut self, content: &str, cx: &mut Context<Self>) {
    self.code_editor.update(cx, |editor, cx| {
      editor.document().update(cx, |doc, cx| {
        doc.replace_all(content, cx);
      });
      editor.reset_after_replace();
      editor.reset_selection(cx);
      editor.set_diffs(None, cx);
      editor.is_read_only = true;
    });
    self.code_file_loading = false;
  }

  fn sync_code_tree_selection(&mut self, cx: &mut Context<Self>) {
    let Some(file) = self.code_selected_file.as_ref() else {
      return;
    };

    let key = file.path.as_ref().to_string();
    let tree_item = TreeItem::new(key.clone(), key.clone());
    self.code_tree_state.update(cx, |state, cx| {
      state.set_selected_item(Some(&tree_item), cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
    });
  }

  fn try_open_pending_issue_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(issue_number) = self.pending_issue_sheet_number else {
      return;
    };
    if self.active_tab_ix != REPO_TAB_ISSUES_IX {
      return;
    }

    let (loading, issue) = {
      let issues = self.issues.read(cx);
      let delegate = issues.delegate();
      let issue = delegate
        .all_rows
        .iter()
        .find(|row| row.issue.number == issue_number)
        .map(|row| row.issue.clone());
      (delegate.loading, issue)
    };

    if let Some(issue) = issue {
      self.open_issue_details_sheet(issue, self.pending_issue_sheet_comment_id, window, cx);
      return;
    }

    if !loading {
      self.pending_issue_sheet_number = None;
      self.pending_issue_sheet_comment_id = None;
    }
  }

  fn load_repository(
    &mut self,
    owner: String,
    repo: String,
    open_target: GithubRepoOpenTarget,
    cx: &mut Context<Self>,
  ) {
    self.owner = owner.clone().into();
    self.repo = repo.clone().into();
    self.active_tab_ix = open_target.tab_ix();
    self.pending_issue_sheet_number = open_target.issue_number();
    self.pending_issue_sheet_comment_id = open_target.issue_comment_id();

    self.reset_branch_state(cx);
    self.repository = None;
    self.repository_loading = true;
    self.repository_error = None;
    self.reset_readme_state(cx);
    self.reset_code_state(cx);

    self.pull_requests_error = None;
    self.pull_requests.update(cx, |state, cx| {
      state.delegate_mut().loading = true;
      state.delegate_mut().set_rows(Vec::new());
      cx.notify();
    });
    self.issues_error = None;
    self.issues.update(cx, |state, cx| {
      state.delegate_mut().loading = true;
      state.delegate_mut().set_rows(Vec::new());
      cx.notify();
    });

    let details_api = self.api.clone();
    let details_owner = owner.clone();
    let details_repo = repo.clone();
    let details_task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || details_api.fetch_github_repository_details(&details_owner, &details_repo))
          .await;

      let _ = this.update(cx, |this, cx| {
        this.repository_loading = false;

        match result {
          Ok(repository) => {
            let selected_branch =
              normalize_non_empty_string(&repository.default_branch).map(SharedString::from);
            this.repository = Some(repository);
            this.selected_branch = selected_branch.clone();
            let branch_items = build_repo_branch_select_items(
              selected_branch
                .clone()
                .map(|branch| vec![branch.to_string()])
                .unwrap_or_default(),
              selected_branch.as_ref().map(SharedString::as_ref),
            );
            this.set_branch_select_items(
              branch_items,
              selected_branch.as_ref().map(ToString::to_string),
              cx,
            );
            this.repository_error = None;
            this.load_repository_branches(cx);
            this.load_readme_if_needed(cx);
            this.load_code_tree_if_needed(cx);
          }
          Err(error) => {
            let message = error.to_string();
            this.repository = None;
            this.repository_error = Some(message.into());
          }
        }

        cx.notify();
      });
    });
    self.repository_task = Some(details_task);

    let pull_requests_api = self.api.clone();
    let pull_requests_owner = owner.clone();
    let pull_requests_repo = repo.clone();
    let pull_requests_task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        pull_requests_api
          .fetch_github_repository_pull_requests(&pull_requests_owner, &pull_requests_repo)
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        let mut rows = Vec::new();

        match result {
          Ok(pull_requests) => {
            rows = pull_requests
              .into_iter()
              .map(|pr| Rc::new(GithubRepoPullRequestRow { pr: Rc::new(pr) }))
              .collect();
            this.pull_requests_error = None;
          }
          Err(error) => {
            let message = error.to_string();
            if github_shared::is_unauthorized_error_message(&message) {
              this.pull_requests_error =
                Some("Authentication required. Please sign in again.".into());
            } else {
              this.pull_requests_error = Some(message.into());
            }
          }
        }

        this.pull_requests.update(cx, |state, cx| {
          state.delegate_mut().loading = false;
          state.delegate_mut().set_rows(rows);
          cx.notify();
        });
      });
    });
    self.pull_requests_task = Some(pull_requests_task);

    let issues_api = self.api.clone();
    let issues_owner = owner;
    let issues_repo = repo;
    let issues_task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || issues_api.fetch_github_repository_issues(&issues_owner, &issues_repo))
          .await;

      let _ = this.update(cx, |this, cx| {
        let mut rows = Vec::new();

        match result {
          Ok(issues) => {
            rows = issues
              .into_iter()
              .map(|issue| {
                Rc::new(GithubRepoIssueRow {
                  issue: Rc::new(issue),
                })
              })
              .collect();
            this.issues_error = None;
          }
          Err(error) => {
            let message = error.to_string();
            if github_shared::is_unauthorized_error_message(&message) {
              this.issues_error = Some("Authentication required. Please sign in again.".into());
            } else {
              this.issues_error = Some(message.into());
            }
          }
        }

        this.issues.update(cx, |state, cx| {
          state.delegate_mut().loading = false;
          state.delegate_mut().set_rows(rows);
          cx.notify();
        });
      });
    });
    self.issues_task = Some(issues_task);

    cx.notify();
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let include_github = matches!(AuthStateStore::get(cx), AuthState::Authenticated(_));
    let commands = CommandPaletteCommand::default_global_commands(
      CommandPalettePage::GithubRepo,
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

  fn handle_readme_gfm_link(
    &mut self,
    url: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> bool {
    if should_open_externally(window) {
      return false;
    }

    let Some(action) = parse_github_url_action(url) else {
      return false;
    };

    self.handle_command_palette_action(action, cx).is_ok()
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
      CommandPaletteAction::OpenGithubRepoDetails {
        owner,
        repo,
        tab,
        issue_number,
        issue_comment_id,
      } => {
        match repo_palette_open_target(AuthStateStore::has_pro_access(cx)) {
          WorkspacePage::GithubRepo => {
            self.load_repository(
              owner,
              repo,
              repo_open_target_from_palette(tab, issue_number, issue_comment_id),
              cx,
            );
            WorkspaceRoute::global_mut(cx).page = WorkspacePage::GithubRepo;
          }
          WorkspacePage::Billing => {
            WorkspaceRoute::open_billing(cx);
          }
          _ => {}
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
        if self.owner.as_ref().is_empty() || self.repo.as_ref().is_empty() {
          GithubPrDetailsPageHandle::show_with_open_target(
            owner.into(),
            repo.into(),
            number,
            open_changes_tab,
            review_comment_id,
            cx,
          );
        } else {
          GithubPrDetailsPageHandle::show_with_repo_return_open_target(
            owner.into(),
            repo.into(),
            number,
            self.owner.clone(),
            self.repo.clone(),
            GithubPrOpenTarget::new(open_changes_tab, review_comment_id),
            cx,
          );
        }
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

  fn render_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let repo_label: SharedString =
      if self.owner.as_ref().is_empty() || self.repo.as_ref().is_empty() {
        "Repository".into()
      } else {
        github_shared::repo_label(self.owner.as_ref(), self.repo.as_ref()).into()
      };
    let pull_requests_count = self.pull_requests.read(cx).delegate().all_rows.len();
    let issues_count = self.issues.read(cx).delegate().all_rows.len();

    let tab_bar = TabBar::new("github-repo-tabs")
      .w_full()
      .segmented()
      .selected_index(self.active_tab_ix)
      .on_click(cx.listener(|this, ix: &usize, window, cx| {
        this.set_active_tab(*ix, window, cx);
      }))
      .child(Tab::new().label("Overview"))
      .child(Tab::new().label("Readme"))
      .child(Tab::new().label("Code"))
      .child(
        Tab::new().child(
          h_flex()
            .items_center()
            .gap_2()
            .child("Pull Requests")
            .child(
              Tag::secondary()
                .small()
                .rounded_full()
                .child(repo_tab_count_label(pull_requests_count)),
            ),
        ),
      )
      .child(
        Tab::new().child(
          h_flex().items_center().gap_2().child("Issues").child(
            Tag::secondary()
              .small()
              .rounded_full()
              .child(repo_tab_count_label(issues_count)),
          ),
        ),
      );

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
        div().flex().items_center().justify_between().child(
          h_flex()
            .items_center()
            .gap_2()
            .child(
              Button::new("repo-back")
                .icon(IconName::ArrowLeft)
                .ghost()
                .compact()
                .on_click(|_, _, cx| {
                  Self::open_github_home(cx);
                }),
            )
            .child(div().text_sm().font_medium().child(repo_label)),
        ),
      )
      .child(tab_bar)
  }

  fn render_overview(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    if should_show_overview_loading_state(self.repository_loading, self.repository.is_some()) {
      return v_flex()
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
            .child("Loading repository details..."),
        );
    }

    if let Some(error) = self.repository_error.as_ref() {
      return v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(div().text_sm().text_color(theme.red).child(error.clone()));
    }

    let Some(repository) = self.repository.as_ref() else {
      return v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child("No repository selected");
    };

    let description = repository
      .description
      .clone()
      .filter(|value| !value.trim().is_empty())
      .unwrap_or_else(|| "No description provided.".to_string());
    let language = repository
      .language
      .clone()
      .filter(|value| !value.trim().is_empty())
      .unwrap_or_else(|| "Unknown".to_string());
    let license = repository
      .license
      .as_ref()
      .map(|value| value.name.clone())
      .unwrap_or_else(|| "Unknown".to_string());
    let homepage = repository
      .homepage
      .clone()
      .filter(|value| !value.trim().is_empty());
    let pushed_at = format_long_date_opt(repository.pushed_at.as_deref());

    let stats = h_flex().gap_2().flex_wrap().children([
      Tag::secondary()
        .small()
        .rounded_full()
        .child(format!("Stars {}", repository.stargazers_count)),
      Tag::secondary()
        .small()
        .rounded_full()
        .child(format!("Forks {}", repository.forks_count)),
      Tag::secondary()
        .small()
        .rounded_full()
        .child(format!("Watchers {}", repository.subscribers_count)),
      Tag::secondary()
        .small()
        .rounded_full()
        .child(format!("Open issues {}", repository.open_issues_count)),
    ]);
    let branch_select = Select::new(&self.branch_select)
      .placeholder("Select branch...")
      .search_placeholder("Search branches...")
      .menu_width(px(320.))
      .w(px(280.))
      .small()
      .disabled(self.repository.is_none());

    v_flex().w_full().h_full().min_h_0().p_4().child(
      v_flex()
        .w_full()
        .max_w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH))
        .mx_auto()
        .gap_4()
        .child(
          h_flex()
            .items_center()
            .gap_3()
            .child(
              Avatar::new()
                .name(repository.owner.login.clone())
                .when_some(repository.owner.avatar_url.clone(), |this, url| {
                  this.src(url)
                })
                .small(),
            )
            .child(
              v_flex()
                .gap_1()
                .child(
                  div()
                    .text_lg()
                    .font_semibold()
                    .child(repository.full_name.clone()),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(repository.owner.login.clone()),
                ),
            ),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(description),
        )
        .child(
          h_flex()
            .gap_2()
            .flex_wrap()
            .child(
              Button::new("repo-open-on-github")
                .icon(IconName::ExternalLink)
                .small()
                .label("Open on GitHub")
                .on_click({
                  let url = repository.html_url.clone();
                  move |_, _, cx| {
                    cx.open_url(&url);
                  }
                }),
            )
            .when_some(homepage.clone(), |this, homepage| {
              let homepage_label = homepage_button_label(&homepage);
              this.child(
                Button::new("repo-open-homepage")
                  .icon(IconName::ExternalLink)
                  .ghost()
                  .small()
                  .label(homepage_label)
                  .on_click(move |_, _, cx| {
                    cx.open_url(&homepage);
                  }),
              )
            }),
        )
        .child(stats)
        .child(
          v_flex()
            .gap_2()
            .child(div().text_sm().font_semibold().child("Code branch"))
            .child(
              h_flex()
                .items_center()
                .gap_2()
                .child(branch_select)
                .when(self.branches_loading, |this| {
                  this.child(Spinner::new().small())
                }),
            )
            .when_some(self.branches_error.clone(), |this, error| {
              this.child(div().text_xs().text_color(theme.status_red()).child(error))
            }),
        )
        .child(
          v_flex()
            .gap_2()
            .child(div().text_sm().font_semibold().child("Repository info"))
            .child(
              h_flex()
                .gap_6()
                .flex_wrap()
                .items_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(format!("Language: {}", language))
                .child(format!("License: {}", license))
                .child(format!("Default branch: {}", repository.default_branch))
                .child(format!("Last push: {}", pushed_at))
                .child(format!("Size: {}", format_repo_size(repository.size)))
                .when_some(homepage, |this, homepage| {
                  this.child(format!("Homepage: {}", homepage))
                }),
            ),
        ),
    )
  }

  fn render_code_files_sidebar(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let count = self.code_lookup.len();

    if let Some(selected_id) = self
      .code_tree_state
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string())
      && Some(selected_id.as_str()) != self.code_selected_tree_id.as_deref()
      && let Some(file) = self.code_lookup.get(&selected_id).cloned()
    {
      self.code_selected_tree_id = Some(selected_id.clone());
      cx.on_next_frame(window, move |this, _, cx| {
        this.set_selected_code_file(Some(file), cx);
      });
    }

    let header = div()
      .px_3()
      .flex()
      .items_center()
      .h(px(CODE_HEADER_HEIGHT))
      .border_b_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .w_full()
          .justify_between()
          .child(div().text_sm().text_color(theme.foreground).child("Files"))
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(count.to_string()),
          ),
      );

    let list = if self.code_files_loading {
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
    } else if self.code_files_error.is_some() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.code_files_error.clone().unwrap_or_default())
        .into_any_element()
    } else if count == 0 {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("No files found")
        .into_any_element()
    } else {
      let view = cx.entity();
      tree(
        &self.code_tree_state,
        move |ix, entry, selected, _window, cx| {
          view.update(cx, |this, cx| {
            let theme = cx.theme().clone();
            let item = entry.item();
            let is_folder = entry.is_folder();
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
                h_flex().items_center().gap_2().child(icon).child(
                  div()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis_start()
                    .child(item.label.clone()),
                ),
              );

            if !is_folder && this.code_lookup.contains_key(item.id.as_ref()) {
              let id = item.id.clone();
              row = row.on_click(cx.listener(move |this, _, _, cx| {
                if let Some(file) = this.code_lookup.get(id.as_ref()).cloned() {
                  this.set_selected_code_file(Some(file), cx);
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

  fn render_code_header(
    &self,
    file: &GithubRepoCodeFile,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let path = file.path.as_ref();
    let file_name = path.rsplit('/').next().unwrap_or(path).to_string();
    let dir_path = path
      .rsplit_once('/')
      .map(|(dir, _)| dir.to_string())
      .unwrap_or_default();
    let icon = file_icon_path_for_name_with_theme(&file_name, &theme)
      .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
      .unwrap_or_else(|| {
        Icon::new(IconName::File)
          .size_3()
          .text_color(theme.muted_foreground)
          .into_any_element()
      });
    let is_markdown = is_markdown_path(Path::new(path));
    let is_svg = is_svg_path(Path::new(path));
    let short_sha: SharedString = github_shared::short_sha(file.sha.as_ref()).into();
    let preview_active = (is_markdown || is_svg) && self.show_markdown_preview;
    let view = cx.entity();
    let preview_button = Button::new("repo-code-markdown-preview")
      .label("Preview")
      .icon(if preview_active {
        IconName::EyeOff
      } else {
        IconName::Eye
      })
      .xsmall()
      .ghost()
      .selected(preview_active)
      .disabled(self.code_file_loading)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_code_markdown_preview(cx);
        });
      });

    div()
      .h(px(CODE_HEADER_HEIGHT))
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
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(short_sha),
          )
          .when(is_markdown || is_svg, |this| this.child(preview_button)),
      )
  }

  fn render_code(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let is_markdown = self.selected_code_file_is_markdown();
    let is_svg = self.selected_code_file_is_svg();
    let preview_active = self.show_markdown_preview && (is_markdown || is_svg);

    let editor_content: gpui::AnyElement = if self.code_files_loading {
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
            .child("Loading files..."),
        )
        .into_any_element()
    } else if self.code_file_loading {
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
    } else if self.code_file_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.code_file_error.clone().unwrap_or_default())
        .into_any_element()
    } else if self.code_files_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.code_files_error.clone().unwrap_or_default())
        .into_any_element()
    } else if self.code_selected_file.is_some() {
      if preview_active {
        let preview_panel = if is_svg {
          self.update_code_svg_preview(window, cx);
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
          let markdown = self.code_editor.read(cx).document().read(cx);
          let markdown = markdown.slice_to_string(0..markdown.len());
          div()
            .flex_1()
            .min_h_0()
            .min_w(px(0.0))
            .bg(theme.background)
            .child(
              div().size_full().pb_4().px_4().child(
                TextView::markdown("github-repo-code-markdown-preview-text", markdown)
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
            h_resizable("github-repo-code-markdown-preview")
              .child(
                resizable_panel().child(
                  div()
                    .size_full()
                    .min_w(px(0.0))
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .debug_selector(|| {
                      GITHUB_REPO_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR.to_string()
                    })
                    .child(self.code_editor.clone()),
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
                    .debug_selector(|| {
                      GITHUB_REPO_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR.to_string()
                    })
                    .child(preview_panel),
                ),
              ),
          )
          .into_any_element()
      } else {
        div()
          .flex_1()
          .min_h_0()
          .child(self.code_editor.clone())
          .into_any_element()
      }
    } else {
      v_flex()
        .flex_1()
        .size_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("Select a file to view code")
        .into_any_element()
    };

    let editor_panel = v_flex()
      .size_full()
      .overflow_hidden()
      .when_some(self.code_selected_file.as_ref(), |this, file| {
        this.child(self.render_code_header(file, cx))
      })
      .child(editor_content);

    h_resizable("github-repo-code")
      .child(
        resizable_panel()
          .size(px(CODE_SIDEBAR_DEFAULT_WIDTH))
          .size_range(px(CODE_SIDEBAR_MIN_WIDTH)..px(CODE_SIDEBAR_MAX_WIDTH))
          .child(self.render_code_files_sidebar(window, cx)),
      )
      .child(resizable_panel().child(editor_panel))
  }

  fn render_readme(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let body = if self.readme_loading {
      v_flex()
        .w_full()
        .min_h(px(240.))
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading README..."),
        )
        .into_any_element()
    } else if let Some(error) = self.readme_error.as_ref() {
      v_flex()
        .w_full()
        .min_h(px(240.))
        .items_center()
        .justify_center()
        .child(
          div()
            .text_sm()
            .text_color(theme.status_red())
            .child(error.clone()),
        )
        .into_any_element()
    } else if let Some(content) = self.readme_content.as_ref() {
      let repo_page = cx.entity().clone();
      let gfm_link_handler = Arc::new(move |url: &str, window: &mut Window, cx: &mut App| {
        let handled = repo_page.update(cx, |this, cx| this.handle_readme_gfm_link(url, window, cx));
        if handled {
          LinkAction::Handled
        } else {
          LinkAction::Open
        }
      });
      let readme_branch = self
        .readme_loaded_branch
        .as_ref()
        .map(SharedString::as_ref)
        .unwrap_or_default();
      let image_base_url = readme_image_base_url(
        self.owner.as_ref(),
        self.repo.as_ref(),
        readme_branch,
        self.readme_path.as_ref().map(SharedString::as_ref),
      );
      let mut options = MarkdownRenderOptions::with_on_link(gfm_link_handler)
        .with_state(self.readme_markdown_state.clone())
        .with_github_issue_reference_context(self.owner.as_ref(), self.repo.as_ref())
        .with_expanded_code_blocks()
        .with_scope_id(readme_scope_id(
          self.owner.as_ref(),
          self.repo.as_ref(),
          readme_branch,
        ));
      if let Some(image_base_url) = image_base_url {
        options = options.with_image_base_url(image_base_url);
      }

      render_markdown(content.as_ref(), &options, cx)
    } else {
      v_flex()
        .w_full()
        .min_h(px(240.))
        .items_center()
        .justify_center()
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("No README found for this branch"),
        )
        .into_any_element()
    };

    div()
      .id("github-repo-readme-scroll")
      .size_full()
      .overflow_y_scrollbar()
      .child(
        v_flex().w_full().px_4().pt_4().pb_32().child(
          v_flex()
            .w_full()
            .max_w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH))
            .mx_auto()
            .child(body),
        ),
      )
  }

  fn render_pull_requests(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let list = List::new(&self.pull_requests)
      .search_placeholder("Search pull requests...")
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .flex_1()
      .min_w(px(0.0))
      .min_h_0()
      .p(px(8.));

    v_flex().w_full().h_full().min_h_0().p_4().child(
      v_flex()
        .w_full()
        .max_w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH))
        .mx_auto()
        .h_full()
        .min_h_0()
        .gap_3()
        .when_some(self.pull_requests_error.clone(), |this, error| {
          this.child(div().text_sm().text_color(theme.red).child(error))
        })
        .child(list),
    )
  }

  fn render_issues(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let list = List::new(&self.issues)
      .search_placeholder("Search issues...")
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .flex_1()
      .min_w(px(0.0))
      .min_h_0()
      .p(px(8.));

    v_flex().w_full().h_full().min_h_0().p_4().child(
      v_flex()
        .w_full()
        .h_full()
        .min_h_0()
        .max_w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH))
        .mx_auto()
        .gap_3()
        .when_some(self.issues_error.clone(), |this, error| {
          this.child(div().text_sm().text_color(theme.red).child(error))
        })
        .child(list),
    )
  }
}

impl Render for GithubRepoPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    self.try_open_pending_issue_sheet(window, cx);

    let content = match self.active_tab_ix {
      REPO_TAB_OVERVIEW_IX => self.render_overview(cx).into_any_element(),
      REPO_TAB_README_IX => self.render_readme(cx).into_any_element(),
      REPO_TAB_CODE_IX => self.render_code(window, cx).into_any_element(),
      REPO_TAB_PULL_REQUESTS_IX => self.render_pull_requests(cx).into_any_element(),
      _ => self.render_issues(cx).into_any_element(),
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GithubRepoPage::show_command_palette_action))
      .on_action(cx.listener(GithubRepoPage::show_file_search_action))
      .child(self.render_header(cx))
      .child(v_flex().w_full().h_full().min_h_0().child(content))
  }
}

impl Focusable for GithubRepoPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::api::{
    GithubIssueDescriptionUpdate, GithubIssueDetailsComment, GithubIssueUser,
    GithubPullRequestAuthor, GithubPullRequestLabel, GithubPullRequestState, GithubRepository,
    GithubRepositoryTreeEntry,
  };
  use crate::workspace::WorkspaceApi;
  use gpui::TestAppContext;

  fn make_issue_comment(
    id: u64,
    body: Option<&str>,
    login: Option<&str>,
  ) -> GithubIssueDetailsComment {
    GithubIssueDetailsComment {
      id,
      body: body.map(str::to_string),
      created_at: "2024-01-03T00:00:00Z".to_string(),
      updated_at: "2024-01-04T00:00:00Z".to_string(),
      user: login.map(|login| GithubIssueUser {
        login: login.to_string(),
        name: None,
        avatar_url: None,
      }),
    }
  }

  fn test_issue_details_with_code_links() -> GithubIssueDetails {
    GithubIssueDetails {
      id: 1,
      number: 42,
      title: "Issue".to_string(),
      body: Some("See:\nhttps://github.com/acme/widget/blob/main/src/lib.rs#L3-L5".to_string()),
      state: "open".to_string(),
      state_reason: None,
      created_at: "2024-01-01T00:00:00Z".to_string(),
      updated_at: "2024-01-02T00:00:00Z".to_string(),
      closed_at: None,
      labels: Vec::new(),
      comments: vec![
        GithubIssueDetailsComment {
          id: 7,
          body: Some(
            "[lib](https://github.com/acme/widget/blob/main/src/lib.rs#L3-L5)".to_string(),
          ),
          created_at: "2024-01-03T00:00:00Z".to_string(),
          updated_at: "2024-01-04T00:00:00Z".to_string(),
          user: None,
        },
        GithubIssueDetailsComment {
          id: 8,
          body: Some("No code reference".to_string()),
          created_at: "2024-01-03T00:00:00Z".to_string(),
          updated_at: "2024-01-04T00:00:00Z".to_string(),
          user: None,
        },
      ],
      user: None,
      repository: GithubRepository {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
      },
    }
  }

  fn init_gpui_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
      gpui_component::init(cx);
      if !cx.has_global::<WorkspaceApi>() {
        cx.set_global(WorkspaceApi::new());
      }
    });
  }

  struct TestProbeView {
    labeled: GithubRepoPullRequestRow,
    unlabeled: GithubRepoPullRequestRow,
  }

  impl Render for TestProbeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
      let theme = cx.theme().clone();

      v_flex()
        .gap_2()
        .child(div().debug_selector(|| "labeled".to_string()).child(
          github_shared::pull_request_list_row_body(self.labeled.pr.as_ref(), &theme, false, true),
        ))
        .child(div().debug_selector(|| "unlabeled".to_string()).child(
          github_shared::pull_request_list_row_body(
            self.unlabeled.pr.as_ref(),
            &theme,
            false,
            true,
          ),
        ))
    }
  }

  fn make_repo_pull_request_row(
    title: &str,
    number: u64,
    labels: &[&str],
  ) -> GithubRepoPullRequestRow {
    GithubRepoPullRequestRow {
      pr: Rc::new(GithubPullRequest {
        number,
        title: title.to_string(),
        state: GithubPullRequestState::Open,
        created_at: "2026-02-12T12:00:00Z".to_string(),
        closed_at: None,
        merged_at: None,
        draft: false,
        updated_at: "2026-02-15T12:00:00Z".to_string(),
        comments_count: 0,
        author: GithubPullRequestAuthor {
          login: "octocat".to_string(),
          avatar_url: None,
          is_bot: false,
        },
        labels: labels
          .iter()
          .map(|label| GithubPullRequestLabel {
            name: (*label).to_string(),
          })
          .collect(),
        repository: GithubRepository {
          owner: "acme".to_string(),
          repo: "widget".to_string(),
        },
      }),
    }
  }

  #[test]
  fn github_page_navigation_targets_github_and_refresh_when_subscription_is_active() {
    assert_eq!(github_page_navigation(true), (WorkspacePage::Github, true));
  }

  #[test]
  fn github_page_navigation_targets_billing_without_refresh_when_subscription_is_inactive() {
    assert_eq!(
      github_page_navigation(false),
      (WorkspacePage::Billing, false)
    );
  }

  #[test]
  fn overview_loading_state_requires_loading_and_missing_repository() {
    assert!(should_show_overview_loading_state(true, false));
    assert!(!should_show_overview_loading_state(false, false));
    assert!(!should_show_overview_loading_state(true, true));
  }

  #[test]
  fn repo_palette_open_target_follows_subscription_state() {
    assert_eq!(repo_palette_open_target(true), WorkspacePage::GithubRepo);
    assert_eq!(repo_palette_open_target(false), WorkspacePage::Billing);
  }

  #[gpui::test]
  fn repo_pull_request_delegate_rows_keep_a_stable_height(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let labeled = make_repo_pull_request_row("Labeled pull request", 1, &["bug"]);
    let unlabeled = make_repo_pull_request_row("Unlabeled pull request", 2, &[]);
    let (_view, cx) = cx.add_window_view(|_, _| TestProbeView { labeled, unlabeled });

    let labeled_height = cx
      .debug_bounds("labeled")
      .expect("labeled bounds")
      .size
      .height;
    let unlabeled_height = cx
      .debug_bounds("unlabeled")
      .expect("unlabeled bounds")
      .size
      .height;

    assert_eq!(labeled_height, unlabeled_height);
  }

  #[gpui::test]
  fn repo_markdown_preview_keeps_editor_and_preview_panes_visible(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let markdown = "# Preview\n\nRepository markdown preview should stay visible.\n";
    let file = Rc::new(GithubRepoCodeFile {
      path: "README.md".into(),
      sha: "deadbeef".into(),
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubRepoPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.owner = "acme".into();
      this.repo = "widget".into();
      this.active_tab_ix = REPO_TAB_CODE_IX;
      this.code_files_loading = false;
      this.code_file_loading = false;
      this.code_files_error = None;
      this.code_file_error = None;
      this.code_lookup.insert(file.path.to_string(), file.clone());
      this.code_selected_file = Some(file.clone());
      this.show_markdown_preview = true;
      this.ensure_code_editor_for_path(file.path.as_ref(), cx);
      this.apply_code_editor_content(markdown, cx);
      cx.notify();
    });

    let editor_bounds = cx
      .debug_bounds(GITHUB_REPO_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR)
      .expect("repo preview editor pane bounds")
      .size;
    let preview_bounds = cx
      .debug_bounds(GITHUB_REPO_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR)
      .expect("repo preview render pane bounds")
      .size;

    assert!(editor_bounds.width > gpui::px(0.0));
    assert!(editor_bounds.height > gpui::px(0.0));
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
  }

  #[test]
  fn repo_tab_count_label_formats_counts() {
    assert_eq!(repo_tab_count_label(0).as_ref(), "0");
    assert_eq!(repo_tab_count_label(42).as_ref(), "42");
  }

  #[test]
  fn homepage_button_label_uses_trimmed_url_or_fallback() {
    assert_eq!(
      homepage_button_label(" https://example.com/docs ").as_ref(),
      "https://example.com/docs"
    );
    assert_eq!(homepage_button_label("   ").as_ref(), "Homepage");
  }

  #[test]
  fn clamp_issue_sheet_width_enforces_min_and_max() {
    assert_eq!(
      clamp_issue_sheet_width(ISSUE_DETAILS_SHEET_MIN_WIDTH_PX - 200.0),
      ISSUE_DETAILS_SHEET_MIN_WIDTH_PX
    );
    assert_eq!(
      clamp_issue_sheet_width(ISSUE_DETAILS_SHEET_MAX_WIDTH_PX + 200.0),
      ISSUE_DETAILS_SHEET_MAX_WIDTH_PX
    );
    assert_eq!(
      clamp_issue_sheet_width(ISSUE_DETAILS_SHEET_WIDTH_PX),
      ISSUE_DETAILS_SHEET_WIDTH_PX
    );
  }

  #[test]
  fn issue_sheet_width_from_cursor_x_uses_right_edge_distance_and_clamps() {
    assert_eq!(
      issue_sheet_width_from_cursor_x(2000.0, 1600.0),
      ISSUE_DETAILS_SHEET_MIN_WIDTH_PX
    );
    assert_eq!(issue_sheet_width_from_cursor_x(2000.0, 900.0), 1100.0);
    assert_eq!(
      issue_sheet_width_from_cursor_x(2000.0, 100.0),
      ISSUE_DETAILS_SHEET_MAX_WIDTH_PX
    );
  }

  #[test]
  fn repo_open_target_from_palette_maps_tabs_and_issue_number() {
    assert_eq!(
      repo_open_target_from_palette(None, None, None),
      GithubRepoOpenTarget::Overview
    );
    assert_eq!(
      repo_open_target_from_palette(Some(CommandPaletteGithubRepoTab::Overview), Some(7), None),
      GithubRepoOpenTarget::Overview
    );
    assert_eq!(
      repo_open_target_from_palette(
        Some(CommandPaletteGithubRepoTab::PullRequests),
        Some(7),
        None,
      ),
      GithubRepoOpenTarget::PullRequests
    );
    assert_eq!(
      repo_open_target_from_palette(
        Some(CommandPaletteGithubRepoTab::Issues),
        Some(42),
        Some(99),
      ),
      GithubRepoOpenTarget::Issues {
        issue_number: Some(42),
        issue_comment_id: Some(99),
      }
    );
  }

  #[test]
  fn github_repo_open_target_tab_ix_accounts_for_code_tab_order() {
    assert_eq!(
      GithubRepoOpenTarget::Overview.tab_ix(),
      REPO_TAB_OVERVIEW_IX
    );
    assert_eq!(
      GithubRepoOpenTarget::PullRequests.tab_ix(),
      REPO_TAB_PULL_REQUESTS_IX
    );
    assert_eq!(
      GithubRepoOpenTarget::Issues {
        issue_number: None,
        issue_comment_id: None,
      }
      .tab_ix(),
      REPO_TAB_ISSUES_IX
    );
  }

  #[test]
  fn repo_tab_indices_match_overview_readme_code_pr_issues_order() {
    assert_eq!(REPO_TAB_OVERVIEW_IX, 0);
    assert_eq!(REPO_TAB_README_IX, 1);
    assert_eq!(REPO_TAB_CODE_IX, 2);
    assert_eq!(REPO_TAB_PULL_REQUESTS_IX, 3);
    assert_eq!(REPO_TAB_ISSUES_IX, 4);
  }

  #[test]
  fn should_prefetch_code_tree_for_tab_prefetches_overview_and_code_only() {
    assert!(should_prefetch_code_tree_for_tab(REPO_TAB_OVERVIEW_IX));
    assert!(should_prefetch_code_tree_for_tab(REPO_TAB_CODE_IX));
    assert!(!should_prefetch_code_tree_for_tab(REPO_TAB_README_IX));
    assert!(!should_prefetch_code_tree_for_tab(
      REPO_TAB_PULL_REQUESTS_IX
    ));
    assert!(!should_prefetch_code_tree_for_tab(REPO_TAB_ISSUES_IX));
  }

  #[test]
  fn build_repo_code_tree_items_prefers_folder_and_selects_first_file() {
    let files = vec![
      Rc::new(GithubRepoCodeFile {
        path: "README.md".into(),
        sha: "sha-readme".into(),
      }),
      Rc::new(GithubRepoCodeFile {
        path: "src/lib.rs".into(),
        sha: "sha-lib".into(),
      }),
      Rc::new(GithubRepoCodeFile {
        path: "src/main.rs".into(),
        sha: "sha-main".into(),
      }),
    ];

    let (items, lookup, selected_index, selected_id) = build_repo_code_tree_items(&files);

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].label.as_ref(), "src");
    assert_eq!(items[0].children.len(), 2);
    assert_eq!(items[0].children[0].label.as_ref(), "lib.rs");
    assert_eq!(items[0].children[1].label.as_ref(), "main.rs");
    assert_eq!(items[1].label.as_ref(), "README.md");
    assert!(!items[0].is_expanded());

    assert_eq!(selected_id.as_deref(), Some("src/lib.rs"));
    assert_eq!(selected_index, Some(0));
    assert!(lookup.contains_key("src/lib.rs"));
    assert!(lookup.contains_key("README.md"));
  }

  #[test]
  fn build_repo_code_tree_items_ignores_non_blob_entries() {
    let tree_entries = vec![
      GithubRepositoryTreeEntry {
        path: "src".to_string(),
        mode: "040000".to_string(),
        entry_type: "tree".to_string(),
        sha: "sha-src-tree".to_string(),
        size: None,
        url: Some("https://example.com/src".to_string()),
      },
      GithubRepositoryTreeEntry {
        path: "src/lib.rs".to_string(),
        mode: "100644".to_string(),
        entry_type: "blob".to_string(),
        sha: "sha-lib".to_string(),
        size: Some(12),
        url: Some("https://example.com/src/lib.rs".to_string()),
      },
      GithubRepositoryTreeEntry {
        path: "bin".to_string(),
        mode: "160000".to_string(),
        entry_type: "commit".to_string(),
        sha: "sha-submodule".to_string(),
        size: None,
        url: Some("https://example.com/bin".to_string()),
      },
    ];

    let files: Vec<Rc<GithubRepoCodeFile>> = tree_entries
      .into_iter()
      .filter(|entry| entry.entry_type.eq_ignore_ascii_case("blob"))
      .map(|entry| {
        Rc::new(GithubRepoCodeFile {
          path: entry.path.into(),
          sha: entry.sha.into(),
        })
      })
      .collect();

    let (items, lookup, selected_index, selected_id) = build_repo_code_tree_items(&files);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label.as_ref(), "src");
    assert_eq!(items[0].children.len(), 1);
    assert_eq!(items[0].children[0].label.as_ref(), "lib.rs");
    assert!(!items[0].is_expanded());
    assert_eq!(lookup.len(), 1);
    assert!(lookup.contains_key("src/lib.rs"));
    assert_eq!(selected_id.as_deref(), Some("src/lib.rs"));
    assert_eq!(selected_index, Some(0));
  }

  #[test]
  fn repo_code_preview_support_detects_markdown_paths() {
    assert!(is_markdown_path(Path::new("README.md")));
    assert!(is_markdown_path(Path::new("docs/guide.Markdown")));
    assert!(is_markdown_path(Path::new("post.MdX")));
    assert!(!is_markdown_path(Path::new("README")));
    assert!(!is_markdown_path(Path::new("image.svg")));
  }

  #[test]
  fn repo_code_preview_support_detects_svg_paths() {
    assert!(is_svg_path(Path::new("icon.svg")));
    assert!(is_svg_path(Path::new("assets/ICON.SVG")));
    assert!(!is_svg_path(Path::new("icon.svgz")));
    assert!(!is_svg_path(Path::new("README.md")));
    assert!(!is_svg_path(Path::new("icon")));
  }

  #[test]
  fn issue_code_reference_requests_extracts_from_description_and_comments() {
    let issue = test_issue_details_with_code_links();
    let (description, comments) = issue_code_reference_requests(&issue);

    assert_eq!(description.len(), 1);
    assert_eq!(description[0].path, "src/lib.rs");

    let comment_refs = comments.get(&7).expect("comment references");
    assert_eq!(comment_refs.len(), 1);
    assert_eq!(comment_refs[0].start_line, 3);
    assert!(comments.get(&8).is_none());
  }

  #[test]
  fn issue_details_comment_owned_by_login_is_case_insensitive() {
    let comment = make_issue_comment(1, Some("Body"), Some("octocat"));
    assert!(issue_details_comment_owned_by_login(&comment, "OCTOCAT"));
    assert!(!issue_details_comment_owned_by_login(&comment, "hubot"));
  }

  #[test]
  fn issue_details_comment_owned_by_login_requires_comment_user() {
    let comment = make_issue_comment(1, Some("Body"), None);
    assert!(!issue_details_comment_owned_by_login(&comment, "octocat"));
  }

  #[test]
  fn next_issue_description_body_returns_none_when_value_is_unchanged_after_trim() {
    assert_eq!(
      next_issue_description_body(
        "  Existing issue description  ",
        "Existing issue description"
      ),
      None
    );
  }

  #[test]
  fn apply_issue_description_update_local_updates_body_and_updated_at() {
    let mut issue = test_issue_details_with_code_links();
    let update = GithubIssueDescriptionUpdate {
      id: issue.id,
      number: issue.number,
      body: Some("Updated issue description".to_string()),
      updated_at: "2026-03-01T12:00:00Z".to_string(),
    };

    apply_issue_description_update_local(&mut issue, update);
    assert_eq!(issue.body.as_deref(), Some("Updated issue description"));
    assert_eq!(issue.updated_at, "2026-03-01T12:00:00Z");
  }

  #[test]
  fn issue_description_references_recompute_without_touching_comment_references() {
    let mut issue = test_issue_details_with_code_links();
    let update = GithubIssueDescriptionUpdate {
      id: issue.id,
      number: issue.number,
      body: Some("https://github.com/acme/widget/blob/main/src/new.rs#L9-L11".to_string()),
      updated_at: "2026-03-01T12:30:00Z".to_string(),
    };

    apply_issue_description_update_local(&mut issue, update);
    let (description, comments) = issue_code_reference_requests(&issue);

    assert_eq!(description.len(), 1);
    assert_eq!(description[0].path, "src/new.rs");
    assert_eq!(description[0].start_line, 9);
    assert_eq!(description[0].end_line, 11);
    assert_eq!(comments.get(&7).map(Vec::len), Some(1));
  }

  #[test]
  fn upsert_issue_details_comment_local_updates_existing_and_appends_missing() {
    let mut comments = vec![make_issue_comment(1, Some("Initial"), Some("octocat"))];
    let updated = make_issue_comment(1, Some("Updated"), Some("octocat"));
    upsert_issue_details_comment_local(&mut comments, updated.clone());
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, updated.body);

    upsert_issue_details_comment_local(
      &mut comments,
      make_issue_comment(2, Some("Another"), Some("octocat")),
    );
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[1].id, 2);
  }

  #[test]
  fn remove_and_restore_issue_details_comment_local_supports_delete_rollback() {
    let mut comments = vec![
      make_issue_comment(1, Some("First"), Some("octocat")),
      make_issue_comment(2, Some("Second"), Some("octocat")),
    ];
    let (index, removed) =
      remove_issue_details_comment_local(&mut comments, 1).expect("comment removed");
    assert_eq!(index, 0);
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].id, 2);

    restore_issue_details_comment_local(&mut comments, index, removed);
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0].id, 1);
    assert_eq!(comments[1].id, 2);
  }

  #[test]
  fn collect_unique_issue_code_reference_requests_deduplicates_urls() {
    let issue = test_issue_details_with_code_links();
    let (description, comments) = issue_code_reference_requests(&issue);
    let unique = collect_unique_issue_code_reference_requests(&description, &comments);

    assert_eq!(unique.len(), 1);
    assert_eq!(
      unique[0].url,
      "https://github.com/acme/widget/blob/main/src/lib.rs#L3-L5"
    );
  }

  #[test]
  fn should_apply_issue_request_result_matches_generation() {
    assert!(should_apply_issue_request_result(4, 4));
    assert!(!should_apply_issue_request_result(5, 4));
  }

  #[test]
  fn should_apply_repo_request_result_matches_generation() {
    assert!(should_apply_repo_request_result(8, 8));
    assert!(!should_apply_repo_request_result(9, 8));
  }

  #[test]
  fn should_fetch_readme_for_branch_respects_loaded_branch_and_errors() {
    assert!(!should_fetch_readme_for_branch(Some("main"), "main", false));
    assert!(should_fetch_readme_for_branch(
      Some("main"),
      "feature",
      false
    ));
    assert!(should_fetch_readme_for_branch(Some("main"), "main", true));
    assert!(should_fetch_readme_for_branch(None, "main", false));
  }

  #[test]
  fn readme_scope_id_changes_with_repo_or_branch() {
    let base = readme_scope_id("acme", "widget", "main");
    assert_eq!(base, readme_scope_id("ACME", "WIDGET", "MAIN"));
    assert_ne!(base, readme_scope_id("acme", "widget", "develop"));
    assert_ne!(base, readme_scope_id("acme", "widget-api", "main"));
  }

  #[test]
  fn readme_image_base_url_uses_readme_directory_when_present() {
    let base = readme_image_base_url("acme", "widget", "main", Some("docs/README.md"));
    assert_eq!(
      base.as_deref(),
      Some("https://raw.githubusercontent.com/acme/widget/main/docs/")
    );
  }

  #[test]
  fn readme_image_base_url_defaults_to_repo_root_for_top_level_readme() {
    let base = readme_image_base_url("acme", "widget", "main", Some("README.md"));
    assert_eq!(
      base.as_deref(),
      Some("https://raw.githubusercontent.com/acme/widget/main/")
    );
  }

  #[test]
  fn readme_image_base_url_returns_none_for_missing_repo_context() {
    assert_eq!(
      readme_image_base_url("", "widget", "main", Some("README.md")),
      None
    );
    assert_eq!(
      readme_image_base_url("acme", "", "main", Some("README.md")),
      None
    );
    assert_eq!(
      readme_image_base_url("acme", "widget", "", Some("README.md")),
      None
    );
  }

  #[test]
  fn effective_repo_branch_prefers_selected_then_default() {
    assert_eq!(
      effective_repo_branch(Some("feature"), Some("main")),
      Some("feature".to_string())
    );
    assert_eq!(
      effective_repo_branch(Some("   "), Some("main")),
      Some("main".to_string())
    );
    assert_eq!(
      effective_repo_branch(None, Some("main")),
      Some("main".to_string())
    );
    assert_eq!(effective_repo_branch(None, None), None);
  }

  #[test]
  fn build_repo_branch_select_items_adds_selected_and_sorts_uniquely() {
    let items = build_repo_branch_select_items(
      vec![
        "main".to_string(),
        "feature/a".to_string(),
        "main".to_string(),
      ],
      Some("release/1.0"),
    );
    let branches = items
      .iter()
      .map(|item| item.branch.clone())
      .collect::<Vec<_>>();
    assert_eq!(
      branches,
      vec![
        "feature/a".to_string(),
        "main".to_string(),
        "release/1.0".to_string()
      ]
    );
  }

  #[test]
  fn issue_sheet_keeps_open_for_same_repo_issue_targets_with_issue_number() {
    assert!(should_keep_issue_sheet_open_for_repo_target(
      "acme",
      "widget",
      "acme",
      "widget",
      Some(CommandPaletteGithubRepoTab::Issues),
      Some(42),
    ));
  }

  #[test]
  fn issue_sheet_closes_for_cross_page_repo_targets() {
    assert!(!should_keep_issue_sheet_open_for_repo_target(
      "acme",
      "widget",
      "acme",
      "widget",
      Some(CommandPaletteGithubRepoTab::Overview),
      None,
    ));
    assert!(!should_keep_issue_sheet_open_for_repo_target(
      "acme",
      "widget",
      "acme",
      "widget",
      Some(CommandPaletteGithubRepoTab::Issues),
      None,
    ));
    assert!(!should_keep_issue_sheet_open_for_repo_target(
      "acme",
      "widget",
      "other",
      "repo",
      Some(CommandPaletteGithubRepoTab::Issues),
      Some(42),
    ));
  }

  #[test]
  fn issue_visual_state_maps_open_completed_and_not_planned_variants() {
    assert_eq!(
      issue_visual_state("open", None),
      GithubIssueVisualState::Open
    );
    assert_eq!(
      issue_visual_state("closed", Some(GithubIssueStateReason::Completed)),
      GithubIssueVisualState::Completed
    );
    assert_eq!(
      issue_visual_state("closed", Some(GithubIssueStateReason::Duplicate)),
      GithubIssueVisualState::NotPlanned
    );
    assert_eq!(
      issue_visual_state("closed", Some(GithubIssueStateReason::NotPlanned)),
      GithubIssueVisualState::NotPlanned
    );
    assert_eq!(
      issue_visual_state("closed", Some(GithubIssueStateReason::Reopened)),
      GithubIssueVisualState::Open
    );
  }

  #[test]
  fn issue_user_display_name_prefers_name_then_login_then_unknown() {
    let with_name = GithubIssueUser {
      login: "octocat".into(),
      name: Some("The Octocat".into()),
      avatar_url: None,
    };
    assert_eq!(
      issue_user_display_name(Some(&with_name)).as_ref(),
      "The Octocat"
    );

    let with_login = GithubIssueUser {
      login: "octocat".into(),
      name: Some("".into()),
      avatar_url: None,
    };
    assert_eq!(
      issue_user_display_name(Some(&with_login)).as_ref(),
      "octocat"
    );
    assert_eq!(issue_user_display_name(None).as_ref(), "unknown");
  }

  #[test]
  fn issue_markdown_body_or_fallback_prefers_non_empty_body() {
    assert_eq!(
      issue_markdown_body_or_fallback(Some("Hello world")).as_ref(),
      "Hello world"
    );
    assert_eq!(
      issue_markdown_body_or_fallback(Some("   ")).as_ref(),
      "No description provided."
    );
    assert_eq!(
      issue_markdown_body_or_fallback(None).as_ref(),
      "No description provided."
    );
  }

  #[test]
  fn issue_comment_markdown_body_or_fallback_prefers_non_empty_body() {
    assert_eq!(
      issue_comment_markdown_body_or_fallback(Some("Looks good")).as_ref(),
      "Looks good"
    );
    assert_eq!(
      issue_comment_markdown_body_or_fallback(Some("  ")).as_ref(),
      "No comment body."
    );
    assert_eq!(
      issue_comment_markdown_body_or_fallback(None).as_ref(),
      "No comment body."
    );
  }

  #[test]
  fn issue_state_label_prefers_reason_then_state() {
    assert_eq!(issue_state_label("open", None).as_ref(), "Open");
    assert_eq!(
      issue_state_label("closed", Some(GithubIssueStateReason::Completed)).as_ref(),
      "Closed"
    );
    assert_eq!(
      issue_state_label("closed", Some(GithubIssueStateReason::Duplicate)).as_ref(),
      "Duplicate"
    );
    assert_eq!(
      issue_state_label("closed", Some(GithubIssueStateReason::NotPlanned)).as_ref(),
      "Not planned"
    );
    assert_eq!(
      issue_state_label("closed", Some(GithubIssueStateReason::Reopened)).as_ref(),
      "Reopened"
    );
    assert_eq!(issue_state_label("closed", None).as_ref(), "Closed");
  }
}
