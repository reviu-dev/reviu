use std::{rc::Rc, sync::Arc};

use gfm_markdown_viewer::{MarkdownRenderOptions, MarkdownRenderState, render_markdown};
use gpui::{
  App, Context, Entity, FocusHandle, Focusable, ParentElement, Render, SharedString, Styled,
  Subscription, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, IndexPath, Placement, Sizable as _, StyledExt,
  avatar::Avatar,
  button::{Button, ButtonVariants as _},
  h_flex,
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  scroll::ScrollableElement,
  spinner::Spinner,
  tab::{Tab, TabBar},
  tag::Tag,
  v_flex,
};
use smol::unblock;

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteGithubRepoTab, CommandPaletteHandler, CommandPalettePage,
  DETAILS_PAGE_CONTAINER_MAX_WIDTH, StatusThemeExt as _, UiIconName, UserMenuConfig, UserMenuPage,
  UserMenuState, UserMenuUser, WindowExt, user_menu,
};

use crate::{
  AuthCallbackTarget, ShowCommandPalette,
  api::{
    ApiClient, GithubIssue, GithubIssueDetails, GithubIssueStateReason, GithubIssueUser,
    GithubPullRequest, GithubRepositoryDetails,
  },
  auth_state::{AuthState, AuthStateStore},
  date_format::{format_compact_datetime, format_long_date_opt},
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubRepoOpenTarget {
  Overview,
  PullRequests,
  Issues { issue_number: Option<u64> },
}

impl GithubRepoOpenTarget {
  fn tab_ix(self) -> usize {
    match self {
      GithubRepoOpenTarget::Overview => 0,
      GithubRepoOpenTarget::PullRequests => 1,
      GithubRepoOpenTarget::Issues { .. } => 2,
    }
  }

  fn issue_number(self) -> Option<u64> {
    match self {
      GithubRepoOpenTarget::Issues { issue_number } => issue_number,
      _ => None,
    }
  }
}

fn repo_open_target_from_palette(
  tab: Option<CommandPaletteGithubRepoTab>,
  issue_number: Option<u64>,
) -> GithubRepoOpenTarget {
  match tab {
    Some(CommandPaletteGithubRepoTab::PullRequests) => GithubRepoOpenTarget::PullRequests,
    Some(CommandPaletteGithubRepoTab::Issues) => GithubRepoOpenTarget::Issues { issue_number },
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
  api: ApiClient,
  owner: String,
  repo: String,
  issue_number: u64,
  issue: Option<GithubIssueDetails>,
  loading: bool,
  error: Option<SharedString>,
  task: Option<Task<()>>,
  markdown_state: MarkdownRenderState,
  request_generation: u64,
}

impl GithubIssueDetailsSheetView {
  fn new(
    api: ApiClient,
    owner: String,
    repo: String,
    issue_number: u64,
    cx: &mut Context<Self>,
  ) -> Self {
    let mut this = Self {
      focus_handle: cx.focus_handle(),
      api,
      owner: owner.clone(),
      repo: repo.clone(),
      issue_number,
      issue: None,
      loading: false,
      error: None,
      task: None,
      markdown_state: MarkdownRenderState::new(),
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
    self.request_generation = self.request_generation.saturating_add(1);
    let generation = self.request_generation;

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.fetch_github_repository_issue_details(&owner, &repo, issue_number))
          .await;

      let _ = this.update(cx, |this, cx| {
        if this.request_generation != generation {
          return;
        }

        this.loading = false;

        match result {
          Ok(issue) => {
            this.issue = Some(issue);
            this.error = None;
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
}

impl Focusable for GithubIssueDetailsSheetView {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for GithubIssueDetailsSheetView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
    } else if let Some(issue) = self.issue.as_ref() {
      let author_name = issue_user_display_name(issue.user.as_ref());
      let opened_at = format_compact_datetime(&issue.created_at);
      let updated_at = format_compact_datetime(&issue.updated_at);
      let closed_at = issue
        .closed_at
        .as_deref()
        .map(format_compact_datetime)
        .unwrap_or_else(|| "—".into());
      let body = issue_markdown_body_or_fallback(issue.body.as_deref());
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

      let label_tags = issue.labels.iter().map(|label| {
        Tag::secondary()
          .small()
          .rounded_full()
          .child(label.name.clone())
      });

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
                .child(render_markdown(
                  body.as_ref(),
                  &MarkdownRenderOptions::default()
                    .with_state(self.markdown_state.clone())
                    .with_scope_id(issue_description_scope_id(issue.id)),
                  cx,
                )),
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
            .when(!issue.comments.is_empty(), |this| {
              this.children(issue.comments.iter().map(|comment| {
                let comment_author = issue_user_display_name(comment.user.as_ref());
                let comment_created_at = format_compact_datetime(&comment.created_at);
                let comment_updated_at = format_compact_datetime(&comment.updated_at);
                let comment_body = issue_comment_markdown_body_or_fallback(comment.body.as_deref());

                v_flex()
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
                  .child(render_markdown(
                    comment_body.as_ref(),
                    &MarkdownRenderOptions::default()
                      .with_state(self.markdown_state.clone())
                      .with_scope_id(issue_comment_scope_id(issue.id, comment.id)),
                    cx,
                  ))
              }))
            }),
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
      .overflow_y_scrollbar()
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
  pull_requests: Entity<ListState<GithubRepoPullRequestListDelegate>>,
  pull_requests_error: Option<SharedString>,
  pull_requests_task: Option<Task<()>>,
  issues: Entity<ListState<GithubRepoIssueListDelegate>>,
  issues_error: Option<SharedString>,
  issues_task: Option<Task<()>>,
  active_tab_ix: usize,
  pending_issue_sheet_number: Option<u64>,
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
    cx: &mut App,
  ) {
    Self::show_with_target(owner, repo, GithubRepoOpenTarget::Issues { issue_number }, cx);
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

    let pull_requests = cx.new(|cx| {
      ListState::new(GithubRepoPullRequestListDelegate::new(), window, cx).searchable(true)
    });
    let issues =
      cx.new(|cx| ListState::new(GithubRepoIssueListDelegate::new(), window, cx).searchable(true));

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
      pull_requests,
      pull_requests_error: None,
      pull_requests_task: None,
      issues,
      issues_error: None,
      issues_task: None,
      active_tab_ix: 0,
      pending_issue_sheet_number: None,
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
            this.open_issue_details_sheet(row.issue.clone(), window, cx);
          }
        }
      },
    );

    self._subscriptions.push(subscription);
  }

  fn open_issue_details_sheet(
    &mut self,
    issue: Rc<GithubIssue>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.pending_issue_sheet_number = None;
    let issue_number = issue.number;
    let sheet_title: SharedString = format!("Issue #{issue_number}").into();
    let issue_details_view = cx.new(|cx| {
      GithubIssueDetailsSheetView::new(
        self.api.clone(),
        issue.repository.owner.clone(),
        issue.repository.repo.clone(),
        issue_number,
        cx,
      )
    });

    let issues_list = self.issues.clone();
    window.open_sheet_at(Placement::Right, cx, move |sheet, _, _cx| {
      sheet
        .overlay(true)
        .overlay_closable(true)
        .size(px(620.0))
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
    if tab_ix != 2 {
      self.pending_issue_sheet_number = None;
    }
    cx.notify();

    if tab_ix == 1 {
      cx.on_next_frame(window, |this, window, cx| {
        this.pull_requests.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      });
      return;
    }

    if tab_ix == 2 {
      cx.on_next_frame(window, |this, window, cx| {
        this.issues.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      });
    }
  }

  fn try_open_pending_issue_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(issue_number) = self.pending_issue_sheet_number else {
      return;
    };
    if self.active_tab_ix != 2 {
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
      self.open_issue_details_sheet(issue, window, cx);
      return;
    }

    if !loading {
      self.pending_issue_sheet_number = None;
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

    self.repository = None;
    self.repository_loading = true;
    self.repository_error = None;

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
      } => {
        match repo_palette_open_target(AuthStateStore::has_active_subscription(cx)) {
          WorkspacePage::GithubRepo => {
            self.load_repository(owner, repo, repo_open_target_from_palette(tab, issue_number), cx);
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
      } => {
        if self.owner.as_ref().is_empty() || self.repo.as_ref().is_empty() {
          GithubPrDetailsPageHandle::show(owner.into(), repo.into(), number, cx);
        } else {
          GithubPrDetailsPageHandle::show_with_repo_return(
            owner.into(),
            repo.into(),
            number,
            self.owner.clone(),
            self.repo.clone(),
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
      1 => self.render_pull_requests(cx).into_any_element(),
      _ => self.render_issues(cx).into_any_element(),
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GithubRepoPage::show_command_palette_action))
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
      repo_open_target_from_palette(None, None),
      GithubRepoOpenTarget::Overview
    );
    assert_eq!(
      repo_open_target_from_palette(Some(CommandPaletteGithubRepoTab::Overview), Some(7)),
      GithubRepoOpenTarget::Overview
    );
    assert_eq!(
      repo_open_target_from_palette(Some(CommandPaletteGithubRepoTab::PullRequests), Some(7)),
      GithubRepoOpenTarget::PullRequests
    );
    assert_eq!(
      repo_open_target_from_palette(Some(CommandPaletteGithubRepoTab::Issues), Some(42)),
      GithubRepoOpenTarget::Issues {
        issue_number: Some(42)
      }
    );
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
