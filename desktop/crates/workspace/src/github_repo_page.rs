use std::{
  collections::{BTreeMap, HashMap, HashSet},
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
  App, Context, Entity, FocusHandle, Focusable, ParentElement, Render, RenderImage, ScrollAnchor,
  ScrollHandle, SharedString, Styled, Subscription, Task, Window, div, img, prelude::*, px,
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
  CommandPaletteGithubRepoTab, CommandPaletteHandler, CommandPalettePage,
  DETAILS_PAGE_CONTAINER_MAX_WIDTH, FILE_ICON_SIZE_PX, SearchFileEntry, SearchFileHandler,
  SearchFilePalette, SearchFilePaletteConfig, StatusThemeExt as _, UiIconName, UserMenuConfig,
  UserMenuPage, UserMenuState, UserMenuUser, WindowExt, file_icon_path_for_name_with_theme,
  h_resizable, parse_github_url_action, resizable_panel, user_menu,
};

use crate::{
  AuthCallbackTarget, ShowCommandPalette, ShowFileSearch,
  api::{
    ApiClient, GithubIssue, GithubIssueDetails, GithubIssueStateReason, GithubIssueUser,
    GithubPullRequest, GithubRepositoryDetails,
  },
  auth_state::{AuthState, AuthStateStore},
  date_format::{format_compact_datetime, format_long_date_opt},
  github_navigation::{
    SameRepoIssueLinkNavigation, open_pr_target, open_repo_target, same_repo_issue_link_navigation,
    should_open_externally,
  },
  github_page::GithubPageHandle,
  github_pr_details_page::GithubPrDetailsPageHandle,
  workspace::{WorkspaceApi, WorkspacePage, WorkspaceRoute},
};

fn is_unauthorized_error_message(error: &str) -> bool {
  error.to_ascii_lowercase().contains("unauthorized")
}

fn list_base_item(ix: IndexPath, selected_index: Option<IndexPath>) -> ListItem {
  ListItem::new(ix).selected(Some(ix) == selected_index)
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

fn github_page_navigation(has_active_subscription: bool) -> (WorkspacePage, bool) {
  if has_active_subscription {
    (WorkspacePage::Github, true)
  } else {
    (WorkspacePage::Billing, false)
  }
}

fn should_show_overview_loading_state(repository_loading: bool, has_repository: bool) -> bool {
  repository_loading && !has_repository
}

fn repo_palette_open_target(has_active_subscription: bool) -> WorkspacePage {
  if has_active_subscription {
    WorkspacePage::GithubRepo
  } else {
    WorkspacePage::Billing
  }
}

const CODE_SIDEBAR_DEFAULT_WIDTH: f32 = 400.0;
const CODE_SIDEBAR_MIN_WIDTH: f32 = 250.0;
const CODE_SIDEBAR_MAX_WIDTH: f32 = 1500.0;
const CODE_HEADER_HEIGHT: f32 = 40.0;

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
      GithubRepoOpenTarget::Overview => 0,
      GithubRepoOpenTarget::PullRequests => 2,
      GithubRepoOpenTarget::Issues { .. } => 3,
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

fn github_issue_url(owner: &str, repo: &str, issue_number: u64) -> String {
  format!("https://github.com/{owner}/{repo}/issues/{issue_number}")
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

const ISSUE_DETAILS_SHEET_WIDTH_PX: f32 = 800.0;

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

fn line_snippets_from_content(
  content: &str,
  start_line: usize,
  end_line: usize,
) -> Option<Vec<String>> {
  if start_line == 0 || end_line == 0 {
    return None;
  }
  let (start_line, end_line) = if start_line <= end_line {
    (start_line, end_line)
  } else {
    (end_line, start_line)
  };

  let lines: Vec<&str> = content.split('\n').collect();
  if start_line > lines.len() {
    return None;
  }

  let end_index = end_line.min(lines.len());
  if end_index < start_line {
    return None;
  }

  Some(
    lines[start_line.saturating_sub(1)..end_index]
      .iter()
      .map(|line| line.trim_end_matches('\r').to_string())
      .collect(),
  )
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
  line_snippets_from_content(content, reference.start_line, reference.end_line).map(|snippets| {
    let actual_end_line = reference
      .start_line
      .saturating_add(snippets.len().saturating_sub(1));
    GithubCodeReferencePreview {
      url: Arc::from(reference.url.as_str()),
      repo: Arc::from(format!("{}/{}", reference.owner, reference.repo)),
      path: Arc::from(reference.path.as_str()),
      reference: Arc::from(reference.reference.as_str()),
      start_line: reference.start_line,
      end_line: actual_end_line,
      snippets: snippets.into_iter().map(Arc::<str>::from).collect(),
    }
  })
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
    let base_item = list_base_item(ix, self.selected_index);

    let row = self.matched_rows.get(ix.row)?;

    let status_tag = row.pr.status().tag(&theme);
    let updated_at = format_compact_datetime(&row.pr.updated_at);

    let label_tags = row.pr.labels.iter().take(4).map(|label| {
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
              .child(
                div()
                  .min_w_0()
                  .flex_1()
                  .child(Label::new(row.pr.title.clone()).truncate()),
              )
              .child(status_tag),
          )
          .child(
            h_flex()
              .gap_2()
              .items_center()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(format!("#{}", row.pr.number))
              .child(format!("Updated {}", updated_at)),
          )
          .when(!row.pr.labels.is_empty(), |this| {
            this.child(
              h_flex()
                .min_w_0()
                .overflow_hidden()
                .gap_1()
                .children(label_tags),
            )
          }),
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
    let base_item = list_base_item(ix, self.selected_index);
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
            let error_message: SharedString = if is_unauthorized_error_message(&message) {
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
      let issue_url = github_issue_url(
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

      let mut comments_rows = v_flex().gap_2();
      let mut pending_comment_anchor: Option<ScrollAnchor> = None;
      let mut pending_comment_found = false;
      for comment in &issue.comments {
        let comment_author = issue_user_display_name(comment.user.as_ref());
        let comment_created_at = format_compact_datetime(&comment.created_at);
        let comment_updated_at = format_compact_datetime(&comment.updated_at);
        let comment_body = issue_comment_markdown_body_or_fallback(comment.body.as_deref());
        let comment_previews = self
          .comment_references
          .get(&comment.id)
          .and_then(|references| {
            github_code_reference_preview_map(references, &self.code_reference_cache)
          });
        let comment_anchor = if self.pending_comment_scroll_id == Some(comment.id) {
          pending_comment_found = true;
          let anchor = ScrollAnchor::for_handle(self.scroll_handle.clone());
          pending_comment_anchor = Some(anchor.clone());
          Some(anchor)
        } else {
          None
        };

        comments_rows = comments_rows.child(
          v_flex()
            .id(format!("github-issue-comment-{}", comment.id))
            .anchor_scroll(comment_anchor)
            .gap_2()
            .p_3()
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius)
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
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!(
                  "Created {comment_created_at} • Updated {comment_updated_at}"
                )),
            )
            .child({
              let mut options = MarkdownRenderOptions::with_on_link(gfm_link_handler.clone())
                .with_state(self.markdown_state.clone())
                .with_scope_id(issue_comment_scope_id(issue.id, comment.id));
              if let Some(previews) = comment_previews.clone() {
                options = options.with_github_code_reference_previews(previews);
              }
              render_markdown(comment_body.as_ref(), &options, cx)
            }),
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
        .child(
          div()
            .min_w_0()
            .flex_1()
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
            .child(
              Tag::secondary()
                .small()
                .rounded_full()
                .text_color(state_color)
                .child(state_text),
            ),
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
            .child(div().text_sm().font_semibold().child("Description"))
            .child(
              div()
                .border_1()
                .border_color(theme.border)
                .rounded(theme.radius)
                .p_3()
                .child({
                  let mut options = MarkdownRenderOptions::with_on_link(gfm_link_handler)
                    .with_state(self.markdown_state.clone())
                    .with_scope_id(issue_description_scope_id(issue.id));
                  if let Some(previews) = description_previews.clone() {
                    options = options.with_github_code_reference_previews(previews);
                  }
                  render_markdown(body.as_ref(), &options, cx)
                }),
            ),
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
            .when(!issue.comments.is_empty(), |this| this.child(comments_rows)),
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
  api: ApiClient,
  owner: SharedString,
  repo: SharedString,
  repository: Option<GithubRepositoryDetails>,
  repository_loading: bool,
  repository_error: Option<SharedString>,
  repository_task: Option<Task<()>>,
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
    if !AuthStateStore::has_active_subscription(cx) {
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
  fn open_github_home(cx: &mut App) {
    let (target, should_refresh) =
      github_page_navigation(AuthStateStore::has_active_subscription(cx));

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
    let pull_requests = cx.new(|cx| {
      ListState::new(GithubRepoPullRequestListDelegate::new(), window, cx).searchable(true)
    });
    let issues =
      cx.new(|cx| ListState::new(GithubRepoIssueListDelegate::new(), window, cx).searchable(true));
    let code_editor = cx.new(|cx| {
      let mut editor = Editor::new_with_paths(PathBuf::from("."), PathBuf::from("."), cx);
      editor.is_read_only = true;
      editor
    });

    let api = WorkspaceApi::global(cx).api.clone();
    let mut this = Self {
      focus_handle: cx.focus_handle(),
      api,
      owner: "".into(),
      repo: "".into(),
      repository: None,
      repository_loading: false,
      repository_error: None,
      repository_task: None,
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
      active_tab_ix: 0,
      pending_issue_sheet_number: None,
      pending_issue_sheet_comment_id: None,
      _subscriptions: Vec::new(),
    };

    this.subscribe_to_pull_requests(cx);
    this.subscribe_to_issues(window, cx);
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
    let sheet_title: SharedString = format!("Issue #{issue_number}").into();
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
    window.open_sheet_at(Placement::Right, cx, move |sheet, _, _cx| {
      sheet
        .overlay(true)
        .overlay_closable(true)
        .size(px(ISSUE_DETAILS_SHEET_WIDTH_PX))
        .title(sheet_title.clone())
        .on_close({
          let issues_list = issues_list.clone();
          move |_, window, cx| {
            issues_list.update(cx, |state, cx| {
              state.focus(window, cx);
            });
          }
        })
        .child(issue_details_view.clone())
    });
  }

  fn set_active_tab(&mut self, tab_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
    if self.active_tab_ix == tab_ix {
      return;
    }
    self.active_tab_ix = tab_ix;
    if tab_ix != 3 {
      self.pending_issue_sheet_number = None;
      self.pending_issue_sheet_comment_id = None;
    }

    if tab_ix == 1 {
      self.load_code_tree_if_needed(cx);
      cx.notify();
      return;
    }

    cx.notify();

    if tab_ix == 2 {
      cx.on_next_frame(window, |this, window, cx| {
        this.pull_requests.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      });
      return;
    }

    if tab_ix == 3 {
      cx.on_next_frame(window, |this, window, cx| {
        this.issues.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      });
    }
  }

  fn reset_code_state(&mut self, cx: &mut Context<Self>) {
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
    if self.active_tab_ix != 1 {
      return;
    }
    if self.code_files_loading || self.code_tree_task.is_some() || !self.code_lookup.is_empty() {
      return;
    }
    let Some(repository) = self.repository.as_ref() else {
      return;
    };

    let owner = self.owner.to_string();
    let repo = self.repo.to_string();
    let default_branch = repository.default_branch.clone();
    if owner.trim().is_empty() || repo.trim().is_empty() || default_branch.trim().is_empty() {
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

    let api = self.api.clone();
    let owner_for_task = owner.clone();
    let repo_for_task = repo.clone();
    let default_branch_for_task = default_branch.clone();
    let owner_for_fetch = owner_for_task.clone();
    let repo_for_fetch = repo_for_task.clone();
    let default_branch_for_fetch = default_branch_for_task.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        api.fetch_github_repository_tree(
          &owner_for_fetch,
          &repo_for_fetch,
          &default_branch_for_fetch,
        )
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.code_tree_task = None;
        if !this
          .owner
          .as_ref()
          .eq_ignore_ascii_case(owner_for_task.as_str())
          || !this
            .repo
            .as_ref()
            .eq_ignore_ascii_case(repo_for_task.as_str())
        {
          return;
        }

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
            if is_unauthorized_error_message(&message) {
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

    let Some(reference) = self
      .repository
      .as_ref()
      .map(|repository| repository.default_branch.clone())
    else {
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
        this.code_file_tasks.remove(&key_for_task);
        if !this.owner.as_ref().eq_ignore_ascii_case(owner.as_str())
          || !this.repo.as_ref().eq_ignore_ascii_case(repo.as_str())
        {
          return;
        }

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

  fn is_markdown_path(path: &Path) -> bool {
    matches!(
      path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref(),
      Some("md" | "markdown" | "mdx")
    )
  }

  fn is_svg_path(path: &Path) -> bool {
    matches!(
      path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref(),
      Some("svg")
    )
  }

  fn selected_code_file_is_markdown(&self) -> bool {
    self
      .code_selected_file
      .as_ref()
      .map(|file| Self::is_markdown_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  fn selected_code_file_is_svg(&self) -> bool {
    self
      .code_selected_file
      .as_ref()
      .map(|file| Self::is_svg_path(Path::new(file.path.as_ref())))
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
    if self.active_tab_ix != 1 {
      return;
    }

    self.open_code_file_search_palette(window, cx);
  }

  fn open_code_file_search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.code_lookup.is_empty() {
      return;
    }

    let mut entries = self
      .code_lookup
      .values()
      .map(|file| {
        let path = PathBuf::from(file.path.as_ref());
        let label = file.path.as_ref().replace(['\n', '\r'], "");
        SearchFileEntry::new(path, label)
      })
      .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.label.cmp(&b.label));

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

    let palette = cx
      .new(|cx| SearchFilePalette::new(window, cx, SearchFilePaletteConfig::new(entries, handler)));
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

    let repo_root = PathBuf::from(".");
    let desired_path_for_editor = desired_path.clone();
    self.code_editor = cx.new(|cx| {
      let mut editor = Editor::new_with_paths(repo_root, desired_path_for_editor, cx);
      editor.is_read_only = true;
      editor
    });
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
    if self.active_tab_ix != 3 {
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

    self.repository = None;
    self.repository_loading = true;
    self.repository_error = None;
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
            this.repository = Some(repository);
            this.repository_error = None;
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
            if is_unauthorized_error_message(&message) {
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
            if is_unauthorized_error_message(&message) {
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
        if AuthStateStore::has_active_subscription(cx) {
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
        match repo_palette_open_target(AuthStateStore::has_active_subscription(cx)) {
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
            open_changes_tab,
            review_comment_id,
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

    let menu_state = match AuthStateStore::get(cx) {
      AuthState::Unknown => UserMenuState::Unknown,
      AuthState::Unauthenticated => UserMenuState::Unauthenticated,
      AuthState::Authenticated(user) => {
        let display_name = if user.name.trim().is_empty() {
          user.email.clone()
        } else {
          user.name.clone()
        };
        UserMenuState::Authenticated(UserMenuUser {
          name: display_name.into(),
          email: user.email.into(),
          image: user.image.map(Into::into),
        })
      }
    };

    let open_git = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::global_mut(cx).page = WorkspacePage::Git;
      cx.refresh_windows();
    });
    let open_github = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      Self::open_github_home(cx);
    });
    let open_billing = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::open_billing(cx);
      cx.refresh_windows();
    });
    let open_settings = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::open_settings(cx);
      cx.refresh_windows();
    });
    let open_about = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::open_about(cx);
      cx.refresh_windows();
    });
    let open_git_config = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::open_git_config(cx);
      cx.refresh_windows();
    });
    let sign_in = Rc::new(|_window: &mut Window, cx: &mut App| {
      AuthCallbackTarget::start_sign_in(cx);
    });
    let sign_out = Rc::new(|_window: &mut Window, cx: &mut App| {
      AuthCallbackTarget::sign_out(cx);
    });

    let auth_control = user_menu(UserMenuConfig {
      id: "auth-menu".into(),
      state: menu_state,
      current_page: UserMenuPage::Github,
      on_open_git: Some(open_git),
      on_open_github: Some(open_github),
      on_open_billing: Some(open_billing),
      on_open_git_config: Some(open_git_config),
      on_open_settings: Some(open_settings),
      on_open_about: Some(open_about),
      on_sign_in: Some(sign_in),
      on_sign_out: Some(sign_out),
    });

    let repo_label: SharedString =
      if self.owner.as_ref().is_empty() || self.repo.as_ref().is_empty() {
        "Repository".into()
      } else {
        format!("{}/{}", self.owner, self.repo).into()
      };

    let tab_bar = TabBar::new("github-repo-tabs")
      .w_full()
      .segmented()
      .selected_index(self.active_tab_ix)
      .on_click(cx.listener(|this, ix: &usize, window, cx| {
        this.set_active_tab(*ix, window, cx);
      }))
      .child(Tab::new().label("Overview"))
      .child(Tab::new().label("Code"))
      .child(Tab::new().label("Pull Requests"))
      .child(Tab::new().label("Issues"));

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
          .child(
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
          )
          .when_some(auth_control, |this, control| this.child(control)),
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
              this.child(
                Button::new("repo-open-homepage")
                  .icon(IconName::ExternalLink)
                  .ghost()
                  .small()
                  .label("Homepage")
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
        move |ix, entry, _selected, _window, cx| {
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
            let mut row = ListItem::new(ix)
              .w_full()
              .rounded(theme.radius)
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
    let is_markdown = Self::is_markdown_path(Path::new(path));
    let is_svg = Self::is_svg_path(Path::new(path));
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
              .child(file.sha.clone()),
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
              .child(resizable_panel().child(self.code_editor.clone()))
              .child(resizable_panel().child(preview_panel)),
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
        .h_full()
        .min_h_0()
        .max_w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH))
        .mx_auto()
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
      0 => self.render_overview(cx).into_any_element(),
      1 => self.render_code(window, cx).into_any_element(),
      2 => self.render_pull_requests(cx).into_any_element(),
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
  use crate::api::{GithubIssueDetailsComment, GithubRepository, GithubRepositoryTreeEntry};

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
    assert_eq!(GithubRepoOpenTarget::Overview.tab_ix(), 0);
    assert_eq!(GithubRepoOpenTarget::PullRequests.tab_ix(), 2);
    assert_eq!(
      GithubRepoOpenTarget::Issues {
        issue_number: None,
        issue_comment_id: None,
      }
      .tab_ix(),
      3
    );
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
        url: "https://example.com/src".to_string(),
      },
      GithubRepositoryTreeEntry {
        path: "src/lib.rs".to_string(),
        mode: "100644".to_string(),
        entry_type: "blob".to_string(),
        sha: "sha-lib".to_string(),
        size: Some(12),
        url: "https://example.com/src/lib.rs".to_string(),
      },
      GithubRepositoryTreeEntry {
        path: "bin".to_string(),
        mode: "160000".to_string(),
        entry_type: "commit".to_string(),
        sha: "sha-submodule".to_string(),
        size: None,
        url: "https://example.com/bin".to_string(),
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
    assert!(GithubRepoPage::is_markdown_path(Path::new("README.md")));
    assert!(GithubRepoPage::is_markdown_path(Path::new(
      "docs/guide.Markdown"
    )));
    assert!(GithubRepoPage::is_markdown_path(Path::new("post.MdX")));
    assert!(!GithubRepoPage::is_markdown_path(Path::new("README")));
    assert!(!GithubRepoPage::is_markdown_path(Path::new("image.svg")));
  }

  #[test]
  fn repo_code_preview_support_detects_svg_paths() {
    assert!(GithubRepoPage::is_svg_path(Path::new("icon.svg")));
    assert!(GithubRepoPage::is_svg_path(Path::new("assets/ICON.SVG")));
    assert!(!GithubRepoPage::is_svg_path(Path::new("icon.svgz")));
    assert!(!GithubRepoPage::is_svg_path(Path::new("README.md")));
    assert!(!GithubRepoPage::is_svg_path(Path::new("icon")));
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
  fn line_snippets_from_content_handles_crlf_and_bounds() {
    let content = "first\r\nsecond\r\nthird\r\n";
    assert_eq!(
      line_snippets_from_content(content, 2, 3),
      Some(vec!["second".to_string(), "third".to_string()])
    );
    assert_eq!(
      line_snippets_from_content(content, 4, 4),
      Some(vec!["".to_string()])
    );
    assert!(line_snippets_from_content(content, 0, 3).is_none());
    assert!(line_snippets_from_content(content, 10, 10).is_none());
  }

  #[test]
  fn should_apply_issue_request_result_matches_generation() {
    assert!(should_apply_issue_request_result(4, 4));
    assert!(!should_apply_issue_request_result(5, 4));
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
  fn issue_details_sheet_width_is_increased_for_readability() {
    assert_eq!(ISSUE_DETAILS_SHEET_WIDTH_PX, 800.0);
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
  fn github_issue_url_formats_expected_path() {
    assert_eq!(
      github_issue_url("acme", "widget", 42),
      "https://github.com/acme/widget/issues/42"
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
