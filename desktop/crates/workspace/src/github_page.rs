use std::{collections::HashSet, rc::Rc, sync::Arc};

use gpui::{
  AnyElement, App, Context, Entity, FocusHandle, Focusable, MouseButton, ParentElement, Pixels,
  Point, Render, SharedString, Styled, Subscription, Task, Window, div, prelude::*, px, relative,
  size,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _, StyledExt as _,
  avatar::Avatar,
  button::{Button, ButtonVariants as _},
  checkbox::Checkbox,
  dialog::{DialogDescription, DialogFooter, DialogHeader, DialogTitle},
  h_flex,
  input::InputEvent,
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  scroll::ScrollableElement,
  select::{Select, SelectEvent, SelectState},
  spinner::Spinner,
  tab::{Tab, TabBar},
  tag::Tag,
  v_flex,
};
use sentry::protocol::{Map, Value};
use smol::unblock;
use time::{OffsetDateTime, macros::format_description};

use crate::dock_badge::set_dock_badge;
use crate::notification_count::NotificationCountStore;
use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteGithubRepoTab, CommandPaletteHandler, CommandPalettePage, ConfirmDialog,
  DETAILS_PAGE_CONTAINER_MAX_WIDTH, Input, InputState, SelectableRowStyle, StatusTag,
  StatusThemeExt, UiIconName, VariableList, VariableListDelegate, VariableListEvent,
  VariableListState, WindowExt, selectable_list_item,
};

#[cfg(test)]
use crate::date_format::format_relative_time_at;
use crate::{
  AuthCallbackTarget, ShowCommandPalette,
  api::{ApiClient, GithubNotification, GithubPullRequest, GithubUserRepository},
  auth_state::{AuthState, AuthStateStore, GithubAccessState},
  billing_page::{ReviuProCheckoutCta, reviu_pro_checkout_button},
  config::ConfigStore,
  date_format::format_relative_time,
  github_home_tabs::{
    GithubHomePullRequestTab, GithubPullRequestFilterOptionLabel,
    GithubPullRequestFilterOptionUser, GithubPullRequestFilterOptions,
    GithubPullRequestReviewStatus, GithubPullRequestSearchFilters,
    generate_github_home_pull_request_tab_id, normalize_github_home_pull_request_tab,
  },
  github_navigation::{open_pr_target, open_repo_target},
  github_pr_details_page::GithubPrDetailsPageHandle,
  github_shared,
  navigation::NavigationHistory,
  pricing_copy::{active_reviu_pro_launch_offer, github_upgrade_description},
  sentry_context,
  workspace::WorkspaceApi,
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

fn variable_list_base_item(
  ix: usize,
  selected_index: Option<usize>,
  theme: &gpui_component::Theme,
) -> ListItem {
  selectable_list_item(
    ("github-variable-list-item", ix),
    Some(ix) == selected_index,
    SelectableRowStyle::Inset,
    theme,
  )
  .px_2()
}

fn update_selected_index<D: ListDelegate>(
  selected_index: &mut Option<IndexPath>,
  ix: Option<IndexPath>,
  cx: &mut Context<ListState<D>>,
) {
  *selected_index = ix;
  cx.notify();
}

fn update_variable_list_selected_index<D: VariableListDelegate>(
  selected_index: &mut Option<usize>,
  ix: Option<usize>,
  cx: &mut Context<VariableListState<D>>,
) {
  *selected_index = ix;
  cx.notify();
}

fn move_item_in_vec<T>(items: &mut Vec<T>, from_index: usize, to_index: usize) -> bool {
  if from_index >= items.len() || to_index > items.len() {
    return false;
  }

  if from_index == to_index || from_index + 1 == to_index {
    return false;
  }

  let item = items.remove(from_index);
  let adjusted_index = if to_index > from_index {
    to_index.saturating_sub(1)
  } else {
    to_index
  };
  items.insert(adjusted_index.min(items.len()), item);
  true
}

#[cfg(test)]
fn repository_updated_label_at(updated_at: &str, now: OffsetDateTime) -> SharedString {
  format!("Updated {}", format_relative_time_at(updated_at, now)).into()
}

fn repository_updated_label(updated_at: &str) -> SharedString {
  format!("Updated {}", format_relative_time(updated_at)).into()
}

#[derive(Clone, Debug)]
struct GithubPullRequestRow {
  pr: Rc<GithubPullRequest>,
}

impl GithubPullRequestRow {
  fn matches(&self, query: &str) -> bool {
    if query.is_empty() {
      return true;
    }

    let q = query.to_lowercase();
    self.pr.title.to_lowercase().contains(&q)
      || github_shared::pull_request_author_display_name(&self.pr.author)
        .to_lowercase()
        .contains(&q)
      || github_shared::repo_label(&self.pr.repository.owner, &self.pr.repository.repo)
        .to_lowercase()
        .contains(&q)
  }
}

#[derive(Clone, Debug)]
struct GithubPullRequestSection {
  repo_label: SharedString,
  rows: Vec<Rc<GithubPullRequestRow>>,
}

fn build_pull_request_sections(rows: &[Rc<GithubPullRequestRow>]) -> Vec<GithubPullRequestSection> {
  let mut sections = Vec::new();

  for row in rows {
    let repo_label = github_shared::repo_label(&row.pr.repository.owner, &row.pr.repository.repo);
    if let Some(section) = sections
      .iter_mut()
      .find(|section: &&mut GithubPullRequestSection| section.repo_label.as_ref() == repo_label)
    {
      section.rows.push(row.clone());
      continue;
    }

    sections.push(GithubPullRequestSection {
      repo_label: repo_label.into(),
      rows: vec![row.clone()],
    });
  }

  sections
}

#[derive(Clone, Debug)]
struct GithubPullRequestTabState {
  tab: GithubHomePullRequestTab,
  rows: Vec<Rc<GithubPullRequestRow>>,
  loading: bool,
  error: Option<SharedString>,
  loaded_once: bool,
}

impl GithubPullRequestTabState {
  fn new(tab: GithubHomePullRequestTab) -> Self {
    Self {
      tab,
      rows: Vec::new(),
      loading: false,
      error: None,
      loaded_once: false,
    }
  }
}

#[derive(Clone)]
struct DraggedPullRequestTab {
  tab_id: String,
  name: SharedString,
}

struct DraggedPullRequestTabPreview {
  name: SharedString,
  position: Point<Pixels>,
}

impl DraggedPullRequestTabPreview {
  fn new(name: SharedString, position: Point<Pixels>) -> Self {
    Self { name, position }
  }
}

impl Render for DraggedPullRequestTabPreview {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    div().pl(self.position.x).pt(self.position.y).child(
      h_flex()
        .items_center()
        .gap_2()
        .w(px(240.0))
        .h(px(36.0))
        .px_3()
        .rounded(theme.radius)
        .bg(theme.background)
        .border_1()
        .border_color(theme.drag_border)
        .text_color(theme.foreground)
        .child(
          Icon::new(UiIconName::EllipsisVertical)
            .size_3()
            .text_color(theme.muted_foreground),
        )
        .child(Label::new(self.name.clone()).truncate()),
    )
  }
}

fn make_pull_request_tab_states(
  tabs: Vec<GithubHomePullRequestTab>,
) -> Vec<GithubPullRequestTabState> {
  tabs
    .into_iter()
    .map(GithubPullRequestTabState::new)
    .collect()
}

const GITHUB_HOME_MANAGE_TABS_ID: &str = "github-home-manage-tabs";

fn pull_request_review_status_select_items() -> Vec<String> {
  vec![
    "Any review state".to_string(),
    "Review required".to_string(),
    "Approved".to_string(),
    "Changes requested".to_string(),
    "No review".to_string(),
  ]
}

fn pull_request_review_status_label(status: GithubPullRequestReviewStatus) -> &'static str {
  match status {
    GithubPullRequestReviewStatus::Any => "Any review state",
    GithubPullRequestReviewStatus::None => "No review",
    GithubPullRequestReviewStatus::Required => "Review required",
    GithubPullRequestReviewStatus::Approved => "Approved",
    GithubPullRequestReviewStatus::ChangesRequested => "Changes requested",
  }
}

fn pull_request_review_status_from_label(label: &str) -> GithubPullRequestReviewStatus {
  match label {
    "Review required" => GithubPullRequestReviewStatus::Required,
    "Approved" => GithubPullRequestReviewStatus::Approved,
    "Changes requested" => GithubPullRequestReviewStatus::ChangesRequested,
    "No review" => GithubPullRequestReviewStatus::None,
    _ => GithubPullRequestReviewStatus::Any,
  }
}

fn pull_request_tab_filter_tag_labels(filters: &GithubPullRequestSearchFilters) -> Vec<String> {
  let mut labels = Vec::new();

  labels.extend(filters.repos.iter().map(|repo| repo.to_string()));
  labels.extend(filters.labels.iter().map(|label| label.to_string()));
  labels.extend(
    filters
      .authors
      .iter()
      .map(|author| format!("Author: {author}")),
  );
  labels.extend(
    filters
      .assignees
      .iter()
      .map(|assignee| format!("Assignee: {assignee}")),
  );
  labels.extend(
    filters
      .requested_reviewers
      .iter()
      .map(|reviewer| format!("Reviewer: {reviewer}")),
  );

  if filters.review_status != GithubPullRequestReviewStatus::Any {
    labels.push(pull_request_review_status_label(filters.review_status).to_string());
  }
  if !filters.include_drafts {
    labels.push("Drafts hidden".to_string());
  }

  labels
}

fn pull_request_tab_delete_confirmation(tab_name: &str) -> (SharedString, SharedString) {
  (
    "Delete pull request list?".into(),
    format!("Delete \"{tab_name}\" from your saved GitHub home tabs?").into(),
  )
}

/// Extracts the trailing number from a GitHub API URL.
/// e.g. `https://api.github.com/repos/owner/repo/pulls/123` → `Some(123)`
fn extract_number_from_api_url(url: &str) -> Option<u64> {
  url.rsplit('/').next()?.parse().ok()
}

/// Converts a GitHub API subject URL to an HTML URL.
/// e.g. `https://api.github.com/repos/o/r/pulls/1` → `https://github.com/o/r/pull/1`
/// e.g. `https://api.github.com/repos/o/r/issues/1` → `https://github.com/o/r/issues/1`
fn github_html_url_from_notification(full_name: &str, subject_type: &str, api_url: &str) -> String {
  let number = api_url.rsplit('/').next().unwrap_or("");
  match subject_type {
    "PullRequest" => format!("https://github.com/{full_name}/pull/{number}"),
    "Issue" => format!("https://github.com/{full_name}/issues/{number}"),
    "Release" => format!("https://github.com/{full_name}/releases"),
    "Discussion" => format!("https://github.com/{full_name}/discussions"),
    _ => format!("https://github.com/{full_name}"),
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubLockedAction {
  SignIn,
  Subscribe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GithubLockedPresentation {
  title: &'static str,
  description: &'static str,
  action: GithubLockedAction,
}

fn github_locked_presentation(access_state: GithubAccessState) -> Option<GithubLockedPresentation> {
  match access_state {
    GithubAccessState::Available => None,
    GithubAccessState::NeedsSignIn => Some(GithubLockedPresentation {
      title: "Sign in to bring GitHub work into the app.",
      description: "Sign in with GitHub to unlock notifications, repository browsing, pull request reviews, issues, and branch-to-PR shortcuts in one place.",
      action: GithubLockedAction::SignIn,
    }),
    GithubAccessState::NeedsSubscription => Some(GithubLockedPresentation {
      title: "Upgrade to unlock GitHub workflows.",
      description: github_upgrade_description(),
      action: GithubLockedAction::Subscribe,
    }),
  }
}

fn github_notification_count_label(unread_count: usize) -> Option<String> {
  (unread_count > 0).then(|| unread_count.to_string())
}

const GITHUB_HOME_NOTIFICATIONS_PANEL_DEBUG_SELECTOR: &str = "github-home-notifications-panel";
const GITHUB_HOME_REPOSITORIES_PANEL_DEBUG_SELECTOR: &str = "github-home-repositories-panel";
const GITHUB_HOME_REVIEW_INBOX_PANEL_DEBUG_SELECTOR: &str = "github-home-review-inbox-panel";

fn github_home_display_name(auth_state: &AuthState) -> Option<SharedString> {
  let AuthState::Authenticated(user) = auth_state else {
    return None;
  };

  let trimmed_name = user.name.trim();
  if !trimmed_name.is_empty() {
    return Some(trimmed_name.to_string().into());
  }

  user
    .github_login
    .as_deref()
    .map(str::trim)
    .filter(|login| !login.is_empty())
    .map(|login| login.to_string().into())
}

fn github_home_greeting_at(name: Option<&str>, now: OffsetDateTime) -> SharedString {
  let greeting = match now.hour() {
    5..=11 => "Good morning",
    12..=17 => "Good afternoon",
    _ => "Good evening",
  };

  let trimmed_name = name.map(str::trim).filter(|name| !name.is_empty());

  match trimmed_name {
    Some(name) => format!("{greeting}, {name}").into(),
    None => greeting.into(),
  }
}

fn github_home_date_label_at(now: OffsetDateTime) -> SharedString {
  now
    .format(format_description!(
      "[weekday repr:long], [month repr:long] [day padding:none], [year]"
    ))
    .unwrap_or_else(|_| now.date().to_string())
    .into()
}
const GITHUB_HOME_NOTIFICATIONS_UNREAD_BADGE_DEBUG_SELECTOR: &str =
  "github-home-notifications-unread-badge";
const GITHUB_HOME_NOTIFICATIONS_COUNT_BADGE_DEBUG_SELECTOR: &str =
  "github-home-notifications-count-badge";
const GITHUB_HOME_REPO_SECTION_ROW_HEIGHT_PX: f32 = 26.0;
const GITHUB_HOME_NOTIFICATION_ROW_HEIGHT_PX: f32 = 56.0;

fn github_home_refresh_in_progress(
  pull_requests_loading: bool,
  notifications_loading: bool,
  repositories_loading: bool,
) -> bool {
  pull_requests_loading || notifications_loading || repositories_loading
}

#[derive(Clone, Debug)]
struct GithubNotificationRow {
  notification: Rc<GithubNotification>,
}

impl GithubNotificationRow {
  fn matches(&self, query: &str) -> bool {
    if query.is_empty() {
      return true;
    }

    let q = query.to_lowercase();
    self.notification.subject.title.to_lowercase().contains(&q)
      || self
        .notification
        .repository
        .full_name
        .to_lowercase()
        .contains(&q)
      || self.notification.reason.to_lowercase().contains(&q)
  }
}

#[derive(Clone, Debug)]
struct GithubRepositoryRow {
  repository: Rc<GithubUserRepository>,
  pinned: bool,
}

impl GithubRepositoryRow {
  fn matches(&self, query: &str) -> bool {
    if query.is_empty() {
      return true;
    }

    let q = query.to_lowercase();
    self.repository.full_name.to_lowercase().contains(&q)
      || self
        .repository
        .description
        .as_deref()
        .is_some_and(|value| value.to_lowercase().contains(&q))
  }
}

struct GithubRepositoryListDelegate {
  all_rows: Vec<Rc<GithubRepositoryRow>>,
  matched_rows: Vec<Rc<GithubRepositoryRow>>,
  selected_index: Option<IndexPath>,
  query: SharedString,
  loading: bool,
  pinned_repos: HashSet<String>,
}

impl GithubRepositoryListDelegate {
  fn new() -> Self {
    let pinned_repos: HashSet<String> = ConfigStore::load_pinned_repos().into_iter().collect();
    Self {
      all_rows: Vec::new(),
      matched_rows: Vec::new(),
      selected_index: Some(IndexPath::default()),
      query: "".into(),
      loading: false,
      pinned_repos,
    }
  }

  fn set_rows(&mut self, rows: Vec<Rc<GithubRepositoryRow>>) {
    self.all_rows = rows
      .into_iter()
      .map(|row| {
        if self.pinned_repos.contains(&row.repository.full_name) {
          Rc::new(GithubRepositoryRow {
            repository: row.repository.clone(),
            pinned: true,
          })
        } else {
          row
        }
      })
      .collect();
    self.prepare(self.query.clone());
  }

  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();
    let q = self.query.as_ref();

    let mut rows: Vec<Rc<GithubRepositoryRow>> = self
      .all_rows
      .iter()
      .filter(|row| row.matches(q))
      .cloned()
      .collect();

    rows.sort_by(|a, b| b.pinned.cmp(&a.pinned));
    self.matched_rows = rows;
  }

  fn toggle_pin(&mut self, full_name: &str) {
    if self.pinned_repos.contains(full_name) {
      self.pinned_repos.remove(full_name);
      ConfigStore::unpin_repo(full_name);
    } else {
      self.pinned_repos.insert(full_name.to_string());
      ConfigStore::pin_repo(full_name);
    }

    for row in &mut self.all_rows {
      if row.repository.full_name == full_name {
        *row = Rc::new(GithubRepositoryRow {
          repository: row.repository.clone(),
          pinned: self.pinned_repos.contains(full_name),
        });
      }
    }

    self.prepare(self.query.clone());
  }
}

impl ListDelegate for GithubRepositoryListDelegate {
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
    let updated_at = repository_updated_label(&row.repository.updated_at);
    let is_pinned = row.pinned;
    let full_name = row.repository.full_name.clone();
    let pin_color = if is_pinned {
      theme.foreground
    } else {
      theme.muted_foreground
    };
    let entity = cx.entity().clone();

    Some(
      base_item.px_2().py_2().child(
        v_flex()
          .size_full()
          .group("repo-row")
          .gap_1()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(
                Avatar::new()
                  .name(row.repository.full_name.clone())
                  .when_some(row.repository.owner_avatar_url.clone(), |this, url| {
                    this.src(url)
                  })
                  .small(),
              )
              .child(
                div()
                  .min_w_0()
                  .flex_1()
                  .child(Label::new(row.repository.full_name.clone()).truncate()),
              )
              .child(
                div()
                  .id(SharedString::from(format!("pin-{}", full_name)))
                  .cursor_pointer()
                  .when(!is_pinned, |this| {
                    this
                      .opacity(0.0)
                      .group_hover("repo-row", |this| this.opacity(1.0))
                  })
                  .on_mouse_down(MouseButton::Left, {
                    let full_name = full_name.clone();
                    move |_event, _window, cx| {
                      cx.stop_propagation();
                      entity.update(cx, |state, cx| {
                        state.delegate_mut().toggle_pin(&full_name);
                        cx.notify();
                      });
                    }
                  })
                  .child(Icon::new(UiIconName::Pin).size_3().text_color(pin_color)),
              )
              .when(row.repository.private, |this| {
                this.child(
                  Icon::new(UiIconName::Lock)
                    .size_3()
                    .text_color(theme.muted_foreground),
                )
              }),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(updated_at),
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
      .child(Icon::new(IconName::Folder).size_6())
      .child("No repositories found")
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
struct GithubNotificationSection {
  repo_label: SharedString,
  rows: Vec<Rc<GithubNotificationRow>>,
}

#[derive(Clone, Debug)]
enum GithubNotificationListEntry {
  SectionHeader {
    repo_label: SharedString,
    section: usize,
    collapsed: bool,
  },
  Item(Rc<GithubNotificationRow>),
}

fn build_notification_sections(
  rows: &[Rc<GithubNotificationRow>],
) -> Vec<GithubNotificationSection> {
  let mut sections = Vec::new();

  for row in rows {
    let repo_label = &row.notification.repository.full_name;
    if let Some(section) = sections
      .iter_mut()
      .find(|section: &&mut GithubNotificationSection| section.repo_label.as_ref() == repo_label)
    {
      section.rows.push(row.clone());
      continue;
    }

    sections.push(GithubNotificationSection {
      repo_label: repo_label.clone().into(),
      rows: vec![row.clone()],
    });
  }

  sections
}

struct GithubNotificationListDelegate {
  all_rows: Vec<Rc<GithubNotificationRow>>,
  matched_rows: Vec<Rc<GithubNotificationRow>>,
  sections: Vec<GithubNotificationSection>,
  visible_rows: Vec<GithubNotificationListEntry>,
  collapsed_repo_labels: HashSet<String>,
  selected_index: Option<usize>,
  query: SharedString,
  loading: bool,
  api: ApiClient,
}

impl GithubNotificationListDelegate {
  fn new(api: ApiClient) -> Self {
    Self {
      all_rows: Vec::new(),
      matched_rows: Vec::new(),
      sections: Vec::new(),
      visible_rows: Vec::new(),
      collapsed_repo_labels: HashSet::new(),
      selected_index: None,
      query: "".into(),
      loading: false,
      api,
    }
  }

  fn set_rows(&mut self, rows: Vec<Rc<GithubNotificationRow>>) {
    self.all_rows = rows;
    self.prepare(self.query.clone());
  }

  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();
    let q = self.query.as_ref();

    let rows: Vec<Rc<GithubNotificationRow>> = self
      .all_rows
      .iter()
      .filter(|row| row.matches(q))
      .cloned()
      .collect();

    self.matched_rows = rows;
    self.sections = build_notification_sections(&self.matched_rows);
    self.collapsed_repo_labels.retain(|repo_label| {
      self
        .sections
        .iter()
        .any(|section| section.repo_label.as_ref() == repo_label)
    });
    self.rebuild_visible_rows();
  }

  fn row_at(&self, ix: usize) -> Option<Rc<GithubNotificationRow>> {
    match self.visible_rows.get(ix)? {
      GithubNotificationListEntry::Item(row) => Some(row.clone()),
      GithubNotificationListEntry::SectionHeader { .. } => None,
    }
  }

  fn unread_count(&self) -> usize {
    self
      .all_rows
      .iter()
      .filter(|row| row.notification.unread)
      .count()
  }

  fn rebuild_visible_rows(&mut self) {
    let mut visible_rows = Vec::new();

    for (section_ix, section) in self.sections.iter().enumerate() {
      let collapsed = self.section_is_collapsed(section_ix);
      visible_rows.push(GithubNotificationListEntry::SectionHeader {
        repo_label: section.repo_label.clone(),
        section: section_ix,
        collapsed,
      });

      if collapsed {
        continue;
      }

      visible_rows.extend(
        section
          .rows
          .iter()
          .cloned()
          .map(GithubNotificationListEntry::Item),
      );
    }

    self.visible_rows = visible_rows;
  }

  fn section_is_collapsed(&self, section: usize) -> bool {
    self.query.is_empty()
      && self.sections.get(section).is_some_and(|section| {
        self
          .collapsed_repo_labels
          .contains(section.repo_label.as_ref())
      })
  }

  fn toggle_section(&mut self, section: usize) {
    let Some(repo_label) = self
      .sections
      .get(section)
      .map(|section| section.repo_label.to_string())
    else {
      return;
    };

    if !self.collapsed_repo_labels.insert(repo_label.clone()) {
      self.collapsed_repo_labels.remove(&repo_label);
    }
    self.rebuild_visible_rows();
  }
}

impl VariableListDelegate for GithubNotificationListDelegate {
  type Item = ListItem;

  fn items_count(&self, _cx: &App) -> usize {
    self.visible_rows.len()
  }

  fn item_size(&self, ix: usize, _cx: &App) -> gpui::Size<Pixels> {
    let height = match self.visible_rows.get(ix) {
      Some(GithubNotificationListEntry::SectionHeader { .. }) => {
        px(GITHUB_HOME_REPO_SECTION_ROW_HEIGHT_PX)
      }
      Some(GithubNotificationListEntry::Item(_)) => px(GITHUB_HOME_NOTIFICATION_ROW_HEIGHT_PX),
      None => px(0.0),
    };
    size(px(0.0), height)
  }

  fn render_item(
    &mut self,
    ix: usize,
    _window: &mut Window,
    cx: &mut Context<VariableListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = cx.theme().clone();
    let base_item = variable_list_base_item(ix, self.selected_index, &theme);

    match self.visible_rows.get(ix)? {
      GithubNotificationListEntry::SectionHeader {
        repo_label,
        collapsed,
        ..
      } => Some(
        base_item
          .px_0()
          .h(px(GITHUB_HOME_REPO_SECTION_ROW_HEIGHT_PX))
          .child(github_shared::repo_section_header(
            repo_label.clone(),
            *collapsed,
            cx,
          )),
      ),
      GithubNotificationListEntry::Item(row) => {
        let notification = &row.notification;
        let updated_at = format_relative_time(&notification.updated_at);
        let subject = notification.subject.title.clone();
        let reason_tag = Tag::secondary()
          .small()
          .rounded_full()
          .child(notification.reason.clone());

        let notification_id = notification.id.clone();
        let api = self.api.clone();
        let list_entity = cx.entity().clone();

        Some(
          base_item
            .h(px(GITHUB_HOME_NOTIFICATION_ROW_HEIGHT_PX))
            .child(
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
                        .child(Label::new(subject).truncate()),
                    )
                    .when(notification.unread, |this| {
                      this.child(div().size(px(6.)).rounded_full().bg(theme.status_blue()))
                    })
                    .child(
                      div()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                          Button::new(format!("notif-done-{}", notification_id))
                            .ghost()
                            .xsmall()
                            .compact()
                            .icon(IconName::Check)
                            .tooltip("Mark as done")
                            .on_click({
                              let notification_id = notification_id.clone();
                              move |_, _window, cx| {
                                cx.stop_propagation();
                                let api = api.clone();
                                let thread_id = notification_id.clone();

                                list_entity.update(cx, |state, cx| {
                                  let delegate = state.delegate_mut();
                                  delegate.all_rows.retain(|r| r.notification.id != thread_id);
                                  delegate.prepare(delegate.query.clone());
                                  let unread = delegate.unread_count();
                                  NotificationCountStore::set(cx, unread);
                                  set_dock_badge(unread);
                                  cx.notify();
                                });

                                cx.spawn(async move |_| {
                                  let _ =
                                    unblock(move || api.mark_notification_done(&thread_id)).await;
                                })
                                .detach();
                              }
                            }),
                        ),
                    ),
                )
                .child(
                  h_flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("Updated {}", updated_at))
                    .child(reason_tag),
                ),
            ),
        )
      }
    }
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<VariableListState<Self>>,
  ) -> impl IntoElement {
    v_flex()
      .size_full()
      .items_center()
      .justify_center()
      .gap_2()
      .text_color(cx.theme().muted_foreground)
      .child(Icon::new(IconName::Inbox).size_6())
      .child("No notifications")
  }

  fn set_selected_index(
    &mut self,
    ix: Option<usize>,
    _window: &mut Window,
    cx: &mut Context<VariableListState<Self>>,
  ) {
    update_variable_list_selected_index(&mut self.selected_index, ix, cx);
  }

  fn perform_search(
    &mut self,
    query: &str,
    _: &mut Window,
    _: &mut Context<VariableListState<Self>>,
  ) -> Task<()> {
    self.prepare(query.to_owned());
    Task::ready(())
  }

  fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<VariableListState<Self>>) {
    let Some(ix) = self.selected_index else {
      return;
    };

    if let Some(GithubNotificationListEntry::SectionHeader { section, .. }) =
      self.visible_rows.get(ix)
    {
      self.toggle_section(*section);
      cx.notify();
    }
  }

  fn loading(&self, _: &App) -> bool {
    self.loading
  }
}

struct GithubPullRequestListDelegate {
  all_rows: Vec<Rc<GithubPullRequestRow>>,
  matched_rows: Vec<Rc<GithubPullRequestRow>>,
  sections: Vec<GithubPullRequestSection>,
  visible_rows: Vec<GithubPullRequestListEntry>,
  collapsed_repo_labels: HashSet<String>,
  selected_index: Option<usize>,
  query: SharedString,
  show_author: bool,
  loading: bool,
}

#[derive(Clone, Debug)]
enum GithubPullRequestListEntry {
  SectionHeader {
    repo_label: SharedString,
    section: usize,
    collapsed: bool,
  },
  Item(Rc<GithubPullRequestRow>),
}

impl GithubPullRequestListDelegate {
  fn new() -> Self {
    Self {
      all_rows: Vec::new(),
      matched_rows: Vec::new(),
      sections: Vec::new(),
      visible_rows: Vec::new(),
      collapsed_repo_labels: HashSet::new(),
      selected_index: None,
      query: "".into(),
      show_author: true,
      loading: false,
    }
  }

  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();
    let q = self.query.as_ref();

    let rows: Vec<Rc<GithubPullRequestRow>> = self
      .all_rows
      .iter()
      .filter(|row| row.matches(q))
      .cloned()
      .collect();

    self.matched_rows = rows;
    self.sections = build_pull_request_sections(&self.matched_rows);
    self.collapsed_repo_labels.retain(|repo_label| {
      self
        .sections
        .iter()
        .any(|section| section.repo_label.as_ref() == repo_label)
    });
    self.rebuild_visible_rows();
  }

  fn set_rows(&mut self, rows: Vec<Rc<GithubPullRequestRow>>) {
    self.all_rows = rows;
    self.prepare(self.query.clone());
  }

  fn row_at(&self, ix: usize) -> Option<Rc<GithubPullRequestRow>> {
    match self.visible_rows.get(ix)? {
      GithubPullRequestListEntry::Item(row) => Some(row.clone()),
      GithubPullRequestListEntry::SectionHeader { .. } => None,
    }
  }

  fn rebuild_visible_rows(&mut self) {
    let mut visible_rows = Vec::new();

    for (section_ix, section) in self.sections.iter().enumerate() {
      let collapsed = self.section_is_collapsed(section_ix);
      visible_rows.push(GithubPullRequestListEntry::SectionHeader {
        repo_label: section.repo_label.clone(),
        section: section_ix,
        collapsed,
      });

      if collapsed {
        continue;
      }

      visible_rows.extend(
        section
          .rows
          .iter()
          .cloned()
          .map(GithubPullRequestListEntry::Item),
      );
    }

    self.visible_rows = visible_rows;
  }

  fn section_is_collapsed(&self, section: usize) -> bool {
    self.query.is_empty()
      && self.sections.get(section).is_some_and(|section| {
        self
          .collapsed_repo_labels
          .contains(section.repo_label.as_ref())
      })
  }

  fn toggle_section(&mut self, section: usize) {
    let Some(repo_label) = self
      .sections
      .get(section)
      .map(|section| section.repo_label.to_string())
    else {
      return;
    };

    if !self.collapsed_repo_labels.insert(repo_label.clone()) {
      self.collapsed_repo_labels.remove(&repo_label);
    }
    self.rebuild_visible_rows();
  }
}

impl VariableListDelegate for GithubPullRequestListDelegate {
  type Item = ListItem;

  fn items_count(&self, _cx: &App) -> usize {
    self.visible_rows.len()
  }

  fn item_size(&self, ix: usize, _cx: &App) -> gpui::Size<Pixels> {
    let height = match self.visible_rows.get(ix) {
      Some(GithubPullRequestListEntry::SectionHeader { .. }) => {
        px(GITHUB_HOME_REPO_SECTION_ROW_HEIGHT_PX)
      }
      Some(GithubPullRequestListEntry::Item(row)) => {
        px(github_shared::pull_request_row_height_px(row.pr.as_ref()))
      }
      None => px(0.0),
    };
    size(px(0.0), height)
  }

  fn render_item(
    &mut self,
    ix: usize,
    _window: &mut Window,
    cx: &mut Context<VariableListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = cx.theme().clone();
    let base_item = variable_list_base_item(ix, self.selected_index, &theme);

    match self.visible_rows.get(ix)? {
      GithubPullRequestListEntry::SectionHeader {
        repo_label,
        collapsed,
        ..
      } => Some(
        base_item
          .px_0()
          .h(px(GITHUB_HOME_REPO_SECTION_ROW_HEIGHT_PX))
          .child(github_shared::repo_section_header(
            repo_label.clone(),
            *collapsed,
            cx,
          )),
      ),
      GithubPullRequestListEntry::Item(row) => Some(
        base_item
          .h(px(github_shared::pull_request_row_height_px(
            row.pr.as_ref(),
          )))
          .child(github_shared::pull_request_list_row_body(
            row.pr.as_ref(),
            &theme,
            false,
            self.show_author,
          )),
      ),
    }
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<VariableListState<Self>>,
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
    ix: Option<usize>,
    _window: &mut Window,
    cx: &mut Context<VariableListState<Self>>,
  ) {
    update_variable_list_selected_index(&mut self.selected_index, ix, cx);
  }

  fn perform_search(
    &mut self,
    query: &str,
    _: &mut Window,
    _: &mut Context<VariableListState<Self>>,
  ) -> Task<()> {
    self.prepare(query.to_owned());
    Task::ready(())
  }

  fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<VariableListState<Self>>) {
    let Some(ix) = self.selected_index else {
      return;
    };

    if let Some(GithubPullRequestListEntry::SectionHeader { section, .. }) =
      self.visible_rows.get(ix)
    {
      self.toggle_section(*section);
      cx.notify();
    }
  }

  fn loading(&self, _: &App) -> bool {
    self.loading
  }
}

pub struct GithubPage {
  focus_handle: FocusHandle,
  api: ApiClient,
  repositories: Entity<ListState<GithubRepositoryListDelegate>>,
  notifications: Entity<VariableListState<GithubNotificationListDelegate>>,
  pull_requests: Entity<VariableListState<GithubPullRequestListDelegate>>,
  pull_request_tabs: Vec<GithubPullRequestTabState>,
  active_pull_request_tab_id: Option<String>,
  load_task: Option<Task<()>>,
  repositories_task: Option<Task<()>>,
  notifications_task: Option<Task<()>>,
  repositories_error: Option<SharedString>,
  notifications_error: Option<SharedString>,
  error: Option<SharedString>,
  access_error: Option<SharedString>,
  focus_on_next_render: bool,
  subscribe_loading: bool,
  subscribe_task: Option<Task<()>>,
  last_access_state: GithubAccessState,
  _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Default)]
pub struct GithubPageHandle {
  github_page: Option<gpui::WeakEntity<GithubPage>>,
}

impl gpui::Global for GithubPageHandle {}

impl GithubPageHandle {
  pub fn register(cx: &mut Context<GithubPage>) {
    cx.set_global(Self {
      github_page: Some(cx.entity().downgrade()),
    });
  }

  pub fn is_refreshing(cx: &App) -> bool {
    let Some(weak) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.github_page.clone())
    else {
      return false;
    };

    weak
      .read_with(cx, |this, cx| {
        let pull_requests_loading = this.pull_requests.read(cx).delegate().loading;
        let notifications_loading = this.notifications.read(cx).delegate().loading;
        let repositories_loading = this.repositories.read(cx).delegate().loading;
        github_home_refresh_in_progress(
          pull_requests_loading,
          notifications_loading,
          repositories_loading,
        )
      })
      .unwrap_or(false)
  }

  pub fn refresh(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().github_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| {
      let access_state = AuthStateStore::github_access_state(cx);
      this.last_access_state = access_state;
      this.focus_on_next_render = true;
      if matches!(access_state, GithubAccessState::Available) {
        this.refresh_pull_requests(cx);
      }
    });
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubPullRequestTabDialogMode {
  Create,
  Edit,
}

struct GithubPullRequestTabDialog {
  api: ApiClient,
  window_handle: gpui::AnyWindowHandle,
  github_page: gpui::WeakEntity<GithubPage>,
  mode: GithubPullRequestTabDialogMode,
  original_tab_id: Option<String>,
  name_input: Entity<InputState>,
  repo_input: Entity<InputState>,
  label_input: Entity<InputState>,
  author_input: Entity<InputState>,
  assignee_input: Entity<InputState>,
  requested_reviewer_input: Entity<InputState>,
  review_status_select: Entity<SelectState<Vec<String>>>,
  available_repositories: Vec<GithubUserRepository>,
  filter_options: GithubPullRequestFilterOptions,
  selected_repos: Vec<String>,
  selected_labels: Vec<String>,
  selected_authors: Vec<String>,
  selected_assignees: Vec<String>,
  selected_requested_reviewers: Vec<String>,
  include_drafts: bool,
  filter_options_loading: bool,
  filter_options_task: Option<Task<()>>,
  submit_loading: bool,
  submit_task: Option<Task<()>>,
  validation_error: Option<SharedString>,
  _subscriptions: Vec<Subscription>,
}

fn filter_tokens_contains(values: &[String], candidate: &str) -> bool {
  values
    .iter()
    .any(|value| value.eq_ignore_ascii_case(candidate))
}

fn push_filter_token(values: &mut Vec<String>, raw_value: &str) -> bool {
  let Some(value) = github_shared::normalize_non_empty_text(raw_value) else {
    return false;
  };
  if filter_tokens_contains(values, &value) {
    return false;
  }
  values.push(value);
  true
}

fn remove_filter_token(values: &mut Vec<String>, raw_value: &str) {
  values.retain(|value| !value.eq_ignore_ascii_case(raw_value));
}

fn matching_filter_option_labels(
  options: &[GithubPullRequestFilterOptionLabel],
  query: &str,
  selected: &[String],
) -> Vec<String> {
  let query = query.trim().to_lowercase();
  options
    .iter()
    .filter(|option| !filter_tokens_contains(selected, &option.name))
    .filter(|option| query.is_empty() || option.name.to_lowercase().contains(&query))
    .map(|option| option.name.clone())
    .take(6)
    .collect()
}

fn matching_filter_option_users(
  options: &[GithubPullRequestFilterOptionUser],
  query: &str,
  selected: &[String],
) -> Vec<GithubPullRequestFilterOptionUser> {
  let query = query.trim().to_lowercase();
  options
    .iter()
    .filter(|option| !filter_tokens_contains(selected, &option.login))
    .filter(|option| query.is_empty() || option.login.to_lowercase().contains(&query))
    .take(6)
    .cloned()
    .collect()
}

fn filter_option_users_contains(
  options: &[GithubPullRequestFilterOptionUser],
  candidate: &str,
) -> bool {
  options
    .iter()
    .any(|option| option.login.eq_ignore_ascii_case(candidate))
}

const CURRENT_USER_PULL_REQUEST_FILTER: &str = "@me";

fn current_user_filter_option() -> GithubPullRequestFilterOptionUser {
  GithubPullRequestFilterOptionUser {
    login: CURRENT_USER_PULL_REQUEST_FILTER.to_string(),
    avatar_url: None,
  }
}

fn matching_user_filter_suggestions(
  options: &[GithubPullRequestFilterOptionUser],
  query: &str,
  selected: &[String],
  include_current_user_fallback: bool,
) -> Vec<GithubPullRequestFilterOptionUser> {
  let mut suggestions = Vec::new();
  let normalized_query = query.trim().to_lowercase();

  if include_current_user_fallback
    && !filter_tokens_contains(selected, CURRENT_USER_PULL_REQUEST_FILTER)
    && (normalized_query.is_empty()
      || CURRENT_USER_PULL_REQUEST_FILTER.contains(normalized_query.as_str()))
  {
    suggestions.push(current_user_filter_option());
  }

  for option in matching_filter_option_users(options, query, selected) {
    if filter_option_users_contains(&suggestions, &option.login) {
      continue;
    }
    suggestions.push(option);
    if suggestions.len() == 6 {
      break;
    }
  }

  suggestions
}

impl GithubPullRequestTabDialog {
  fn new(
    api: ApiClient,
    window_handle: gpui::AnyWindowHandle,
    github_page: gpui::WeakEntity<GithubPage>,
    mode: GithubPullRequestTabDialogMode,
    initial_tab: Option<GithubHomePullRequestTab>,
    available_repositories: Vec<GithubUserRepository>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let review_status_select =
      cx.new(|cx| SelectState::new(pull_request_review_status_select_items(), None, window, cx));

    let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("List name"));
    let repo_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add repositories..."));
    let label_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add labels..."));
    let author_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add authors..."));
    let assignee_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add assignees..."));
    let requested_reviewer_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Add requested reviewers..."));

    let mut subscriptions = vec![
      cx.subscribe_in(&repo_input, window, Self::on_repo_input_event),
      cx.subscribe_in(&label_input, window, Self::on_label_input_event),
      cx.subscribe_in(&author_input, window, Self::on_author_input_event),
      cx.subscribe_in(&assignee_input, window, Self::on_assignee_input_event),
      cx.subscribe_in(
        &requested_reviewer_input,
        window,
        Self::on_requested_reviewer_input_event,
      ),
      cx.subscribe_in(
        &review_status_select,
        window,
        |this, _, _: &SelectEvent<Vec<String>>, _, cx| {
          cx.notify();
          let _ = this;
        },
      ),
    ];

    subscriptions.push(
      cx.subscribe_in(&name_input, window, |_, _, _: &InputEvent, _, cx| {
        cx.notify();
      }),
    );

    let mut this = Self {
      api,
      window_handle,
      github_page,
      mode,
      original_tab_id: initial_tab.as_ref().map(|tab| tab.id.clone()),
      name_input,
      repo_input,
      label_input,
      author_input,
      assignee_input,
      requested_reviewer_input,
      review_status_select,
      available_repositories,
      filter_options: GithubPullRequestFilterOptions::default(),
      selected_repos: initial_tab
        .as_ref()
        .map(|tab| tab.filters.repos.clone())
        .unwrap_or_default(),
      selected_labels: initial_tab
        .as_ref()
        .map(|tab| tab.filters.labels.clone())
        .unwrap_or_default(),
      selected_authors: initial_tab
        .as_ref()
        .map(|tab| tab.filters.authors.clone())
        .unwrap_or_default(),
      selected_assignees: initial_tab
        .as_ref()
        .map(|tab| tab.filters.assignees.clone())
        .unwrap_or_default(),
      selected_requested_reviewers: initial_tab
        .as_ref()
        .map(|tab| tab.filters.requested_reviewers.clone())
        .unwrap_or_default(),
      include_drafts: initial_tab
        .as_ref()
        .map(|tab| tab.filters.include_drafts)
        .unwrap_or(true),
      filter_options_loading: false,
      filter_options_task: None,
      submit_loading: false,
      submit_task: None,
      validation_error: None,
      _subscriptions: subscriptions,
    };

    if let Some(tab) = initial_tab {
      this.name_input.update(cx, |input, cx| {
        input.set_value(tab.name, window, cx);
      });
      this.review_status_select.update(cx, |state, cx| {
        state.set_selected_value(
          &pull_request_review_status_label(tab.filters.review_status).to_string(),
          window,
          cx,
        );
      });
    } else {
      this.review_status_select.update(cx, |state, cx| {
        state.set_selected_value(
          &pull_request_review_status_label(GithubPullRequestReviewStatus::Any).to_string(),
          window,
          cx,
        );
      });
    }

    if !this.selected_repos.is_empty() {
      this.refresh_filter_options(cx);
    }

    this
  }

  fn title(&self) -> &'static str {
    match self.mode {
      GithubPullRequestTabDialogMode::Create => "Create Pull Request List",
      GithubPullRequestTabDialogMode::Edit => "Edit Pull Request List",
    }
  }

  fn submit_label(&self) -> &'static str {
    match self.mode {
      GithubPullRequestTabDialogMode::Create => "Create list",
      GithubPullRequestTabDialogMode::Edit => "Save changes",
    }
  }

  fn repo_query(&self, cx: &App) -> String {
    self.repo_input.read(cx).value().trim().to_string()
  }

  fn label_query(&self, cx: &App) -> String {
    self.label_input.read(cx).value().trim().to_string()
  }

  fn author_query(&self, cx: &App) -> String {
    self.author_input.read(cx).value().trim().to_string()
  }

  fn assignee_query(&self, cx: &App) -> String {
    self.assignee_input.read(cx).value().trim().to_string()
  }

  fn requested_reviewer_query(&self, cx: &App) -> String {
    self
      .requested_reviewer_input
      .read(cx)
      .value()
      .trim()
      .to_string()
  }

  fn matching_repo_suggestions(&self, cx: &App) -> Vec<String> {
    let query = self.repo_query(cx).to_lowercase();
    self
      .available_repositories
      .iter()
      .map(|repo| repo.full_name.clone())
      .filter(|full_name| !filter_tokens_contains(&self.selected_repos, full_name))
      .filter(|full_name| query.is_empty() || full_name.to_lowercase().contains(&query))
      .take(6)
      .collect()
  }

  fn clear_input(input: &Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) {
    input.update(cx, |input, cx| input.set_value("", window, cx));
  }

  fn add_repo_value(&mut self, raw_value: &str, window: &mut Window, cx: &mut Context<Self>) {
    if push_filter_token(&mut self.selected_repos, raw_value) {
      Self::clear_input(&self.repo_input, window, cx);
      self.refresh_filter_options(cx);
      cx.notify();
    }
  }

  fn add_label_value(&mut self, raw_value: &str, window: &mut Window, cx: &mut Context<Self>) {
    if push_filter_token(&mut self.selected_labels, raw_value) {
      Self::clear_input(&self.label_input, window, cx);
      cx.notify();
    }
  }

  fn add_author_value(&mut self, raw_value: &str, window: &mut Window, cx: &mut Context<Self>) {
    if push_filter_token(&mut self.selected_authors, raw_value) {
      Self::clear_input(&self.author_input, window, cx);
      cx.notify();
    }
  }

  fn add_assignee_value(&mut self, raw_value: &str, window: &mut Window, cx: &mut Context<Self>) {
    if push_filter_token(&mut self.selected_assignees, raw_value) {
      Self::clear_input(&self.assignee_input, window, cx);
      cx.notify();
    }
  }

  fn add_requested_reviewer_value(
    &mut self,
    raw_value: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if push_filter_token(&mut self.selected_requested_reviewers, raw_value) {
      Self::clear_input(&self.requested_reviewer_input, window, cx);
      cx.notify();
    }
  }

  fn remove_repo(&mut self, full_name: &str, cx: &mut Context<Self>) {
    remove_filter_token(&mut self.selected_repos, full_name);
    self.refresh_filter_options(cx);
    cx.notify();
  }

  fn remove_label(&mut self, label: &str, cx: &mut Context<Self>) {
    remove_filter_token(&mut self.selected_labels, label);
    cx.notify();
  }

  fn remove_author(&mut self, login: &str, cx: &mut Context<Self>) {
    remove_filter_token(&mut self.selected_authors, login);
    cx.notify();
  }

  fn remove_assignee(&mut self, login: &str, cx: &mut Context<Self>) {
    remove_filter_token(&mut self.selected_assignees, login);
    cx.notify();
  }

  fn remove_requested_reviewer(&mut self, login: &str, cx: &mut Context<Self>) {
    remove_filter_token(&mut self.selected_requested_reviewers, login);
    cx.notify();
  }

  fn refresh_filter_options(&mut self, cx: &mut Context<Self>) {
    if self.selected_repos.is_empty() {
      self.filter_options = GithubPullRequestFilterOptions::default();
      self.filter_options_loading = false;
      cx.notify();
      return;
    }

    self.filter_options_loading = true;
    let api = self.api.clone();
    let repos = self.selected_repos.clone();
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_github_pull_request_filter_options(&repos)).await;

      let _ = cx.update_window(window_handle, |_, _, cx| {
        let _ = this.update(cx, |this, cx| {
          this.filter_options_loading = false;
          if let Ok(options) = result {
            this.filter_options = options;
          }
          cx.notify();
        });
      });
    });

    self.filter_options_task = Some(task);
    cx.notify();
  }

  fn review_status(&self, cx: &App) -> GithubPullRequestReviewStatus {
    self
      .review_status_select
      .read(cx)
      .selected_value()
      .map(|value| pull_request_review_status_from_label(value.as_str()))
      .unwrap_or_default()
  }

  fn build_tab(&self, cx: &App) -> Option<GithubHomePullRequestTab> {
    let name = github_shared::normalize_non_empty_text(self.name_input.read(cx).value().as_str())?;
    let id = self
      .original_tab_id
      .clone()
      .unwrap_or_else(generate_github_home_pull_request_tab_id);
    Some(normalize_github_home_pull_request_tab(
      &GithubHomePullRequestTab {
        id,
        name,
        filters: GithubPullRequestSearchFilters {
          repos: self.selected_repos.clone(),
          labels: self.selected_labels.clone(),
          authors: self.selected_authors.clone(),
          assignees: self.selected_assignees.clone(),
          requested_reviewers: self.selected_requested_reviewers.clone(),
          review_status: self.review_status(cx),
          include_drafts: self.include_drafts,
        },
      },
    ))
  }

  fn on_repo_input_event(
    &mut self,
    state: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match event {
      InputEvent::Change => cx.notify(),
      InputEvent::PressEnter { .. } => {
        let query = state.read(cx).value().to_string();
        let suggestion = self.matching_repo_suggestions(cx).into_iter().next();
        self.add_repo_value(suggestion.as_deref().unwrap_or(query.as_str()), window, cx);
      }
      _ => {}
    }
  }

  fn on_label_input_event(
    &mut self,
    state: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match event {
      InputEvent::Change => cx.notify(),
      InputEvent::PressEnter { .. } => {
        self.add_label_value(state.read(cx).value().as_str(), window, cx);
      }
      _ => {}
    }
  }

  fn on_author_input_event(
    &mut self,
    state: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match event {
      InputEvent::Change => cx.notify(),
      InputEvent::PressEnter { .. } => {
        self.add_author_value(state.read(cx).value().as_str(), window, cx);
      }
      _ => {}
    }
  }

  fn on_assignee_input_event(
    &mut self,
    state: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match event {
      InputEvent::Change => cx.notify(),
      InputEvent::PressEnter { .. } => {
        self.add_assignee_value(state.read(cx).value().as_str(), window, cx);
      }
      _ => {}
    }
  }

  fn on_requested_reviewer_input_event(
    &mut self,
    state: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match event {
      InputEvent::Change => cx.notify(),
      InputEvent::PressEnter { .. } => {
        self.add_requested_reviewer_value(state.read(cx).value().as_str(), window, cx);
      }
      _ => {}
    }
  }

  fn submit_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    if self.submit_loading {
      return;
    }

    let Some(tab) = self.build_tab(cx) else {
      self.validation_error = Some("List name is required.".into());
      cx.notify();
      return;
    };

    self.validation_error = None;
    self.submit_loading = true;
    let github_page = self.github_page.clone();
    let original_tab_id = self.original_tab_id.clone();
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = github_page.update(cx, |github_page, cx| {
          github_page.save_pull_request_tab(tab.clone(), original_tab_id.clone(), cx);
        });
        let _ = this.update(cx, |this, cx| {
          this.submit_loading = false;
          cx.notify();
        });
        window.close_dialog(cx);
      });
    });

    self.submit_task = Some(task);
    cx.notify();
  }

  fn toggle_include_drafts(&mut self, checked: bool, _: &mut Window, cx: &mut Context<Self>) {
    self.include_drafts = checked;
    cx.notify();
  }

  fn render_token_row(
    id_prefix: &'static str,
    values: &[String],
    on_remove: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
  ) -> impl IntoElement {
    h_flex()
      .gap_1()
      .flex_wrap()
      .children(values.iter().cloned().map(move |value| {
        let on_remove = on_remove.clone();
        h_flex()
          .items_center()
          .gap_1()
          .px_2()
          .py_1()
          .rounded_full()
          .child(div().text_sm().child(value.clone()))
          .child(
            Button::new(format!("{id_prefix}-remove-{}", value))
              .ghost()
              .xsmall()
              .compact()
              .icon(IconName::Close)
              .on_click(move |_, window, cx| {
                on_remove(value.clone(), window, cx);
              }),
          )
      }))
  }
}

impl Focusable for GithubPullRequestTabDialog {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    self.name_input.read(cx).focus_handle(cx)
  }
}

impl Render for GithubPullRequestTabDialog {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let repo_suggestions = self.matching_repo_suggestions(cx);
    let label_suggestions = matching_filter_option_labels(
      &self.filter_options.labels,
      &self.label_query(cx),
      &self.selected_labels,
    );
    let author_suggestions = matching_user_filter_suggestions(
      &self.filter_options.authors,
      &self.author_query(cx),
      &self.selected_authors,
      true,
    );
    let assignee_suggestions = matching_user_filter_suggestions(
      &self.filter_options.assignees,
      &self.assignee_query(cx),
      &self.selected_assignees,
      true,
    );
    let reviewer_suggestions = matching_user_filter_suggestions(
      &self.filter_options.assignees,
      &self.requested_reviewer_query(cx),
      &self.selected_requested_reviewers,
      true,
    );
    let needs_repos = self.selected_repos.is_empty();

    div()
      .id("github-home-pull-request-tab-dialog")
      .debug_selector(|| "github-home-pull-request-tab-dialog".to_string())
      .flex()
      .flex_col()
      .w_full()
      .child(
        DialogHeader::new()
          .p_4()
          .child(DialogTitle::new().child(self.title()))
          .child(
            DialogDescription::new()
              .child("Save a pull request list with reusable GitHub filters."),
          ),
      )
      .child(
        v_flex()
          .id("github-home-pull-request-tab-dialog-body")
          .px_4()
          .pb_4()
          .gap_3()
          .child(
            v_flex()
              .gap_1()
              .child(div().text_sm().child("Name"))
              .child(Input::new(&self.name_input).w_full()),
          )
          .child(
            v_flex()
              .gap_1()
              .child(div().text_sm().child("Repositories"))
              .when(!self.selected_repos.is_empty(), |this| {
                this.child(Self::render_token_row(
                  "github-tab-repo",
                  &self.selected_repos,
                  {
                    let view = cx.entity().clone();
                    move |value, _, cx| {
                      view.update(cx, |this, cx| {
                        this.remove_repo(&value, cx);
                      });
                    }
                  },
                ))
              })
              .child(Input::new(&self.repo_input).w_full())
              .when(!repo_suggestions.is_empty(), |this| {
                this.child(
                  h_flex()
                    .gap_1()
                    .flex_wrap()
                    .children(repo_suggestions.into_iter().map(|repo| {
                      Button::new(format!("github-tab-repo-suggestion-{repo}"))
                        .label(repo.clone())
                        .xsmall()
                        .outline()
                        .on_click({
                          let view = cx.entity().clone();
                          move |_, window, cx| {
                            view.update(cx, |this, cx| {
                              this.add_repo_value(&repo, window, cx);
                            });
                          }
                        })
                    })),
                )
              }),
          )
          .child(
            v_flex()
              .gap_1()
              .child(
                h_flex()
                  .justify_between()
                  .items_center()
                  .child(div().text_sm().child("Labels"))
                  .when(self.filter_options_loading, |this| {
                    this.child(Spinner::new().xsmall())
                  }),
              )
              .when(!self.selected_labels.is_empty(), |this| {
                this.child(Self::render_token_row(
                  "github-tab-label",
                  &self.selected_labels,
                  {
                    let view = cx.entity().clone();
                    move |value, _, cx| {
                      view.update(cx, |this, cx| this.remove_label(&value, cx));
                    }
                  },
                ))
              })
              .child(Input::new(&self.label_input).w_full().disabled(needs_repos))
              .when(needs_repos, |this| {
                this.child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Select at least one repository to load labels."),
                )
              })
              .when(!label_suggestions.is_empty(), |this| {
                this.child(h_flex().gap_1().flex_wrap().children(
                  label_suggestions.into_iter().map(|label| {
                    Button::new(format!("github-tab-label-suggestion-{label}"))
                      .label(label.clone())
                      .xsmall()
                      .outline()
                      .on_click({
                        let view = cx.entity().clone();
                        move |_, window, cx| {
                          view.update(cx, |this, cx| {
                            this.add_label_value(&label, window, cx);
                          });
                        }
                      })
                  }),
                ))
              }),
          )
          .child(
            v_flex()
              .gap_1()
              .child(div().text_sm().child("Authors"))
              .when(!self.selected_authors.is_empty(), |this| {
                this.child(Self::render_token_row(
                  "github-tab-author",
                  &self.selected_authors,
                  {
                    let view = cx.entity().clone();
                    move |value, _, cx| {
                      view.update(cx, |this, cx| this.remove_author(&value, cx));
                    }
                  },
                ))
              })
              .child(Input::new(&self.author_input).w_full())
              .when(!author_suggestions.is_empty(), |this| {
                this.child(h_flex().gap_1().flex_wrap().children(
                  author_suggestions.into_iter().map(|author| {
                    Button::new(format!("github-tab-author-suggestion-{}", author.login))
                      .label(author.login.clone())
                      .xsmall()
                      .outline()
                      .on_click({
                        let view = cx.entity().clone();
                        move |_, window, cx| {
                          view.update(cx, |this, cx| {
                            this.add_author_value(&author.login, window, cx);
                          });
                        }
                      })
                  }),
                ))
              }),
          )
          .child(
            v_flex()
              .gap_1()
              .child(div().text_sm().child("Assignees"))
              .when(!self.selected_assignees.is_empty(), |this| {
                this.child(Self::render_token_row(
                  "github-tab-assignee",
                  &self.selected_assignees,
                  {
                    let view = cx.entity().clone();
                    move |value, _, cx| {
                      view.update(cx, |this, cx| this.remove_assignee(&value, cx));
                    }
                  },
                ))
              })
              .child(Input::new(&self.assignee_input).w_full())
              .when(!assignee_suggestions.is_empty(), |this| {
                this.child(h_flex().gap_1().flex_wrap().children(
                  assignee_suggestions.into_iter().map(|assignee| {
                    Button::new(format!("github-tab-assignee-suggestion-{}", assignee.login))
                      .label(assignee.login.clone())
                      .xsmall()
                      .outline()
                      .on_click({
                        let view = cx.entity().clone();
                        move |_, window, cx| {
                          view.update(cx, |this, cx| {
                            this.add_assignee_value(&assignee.login, window, cx);
                          });
                        }
                      })
                  }),
                ))
              }),
          )
          .child(
            v_flex()
              .gap_1()
              .child(div().text_sm().child("Requested Reviewers"))
              .when(!self.selected_requested_reviewers.is_empty(), |this| {
                this.child(Self::render_token_row(
                  "github-tab-reviewer",
                  &self.selected_requested_reviewers,
                  {
                    let view = cx.entity().clone();
                    move |value, _, cx| {
                      view.update(cx, |this, cx| this.remove_requested_reviewer(&value, cx));
                    }
                  },
                ))
              })
              .child(Input::new(&self.requested_reviewer_input).w_full())
              .when(!reviewer_suggestions.is_empty(), |this| {
                this.child(h_flex().gap_1().flex_wrap().children(
                  reviewer_suggestions.into_iter().map(|reviewer| {
                    Button::new(format!("github-tab-reviewer-suggestion-{}", reviewer.login))
                      .label(reviewer.login.clone())
                      .xsmall()
                      .outline()
                      .on_click({
                        let view = cx.entity().clone();
                        move |_, window, cx| {
                          view.update(cx, |this, cx| {
                            this.add_requested_reviewer_value(&reviewer.login, window, cx);
                          });
                        }
                      })
                  }),
                ))
              }),
          )
          .child(
            v_flex()
              .gap_1()
              .child(div().text_sm().child("Review"))
              .child(Select::new(&self.review_status_select).w_full()),
          )
          .child(
            Checkbox::new("github-tab-include-drafts")
              .checked(self.include_drafts)
              .label("Include draft pull requests")
              .on_click(cx.listener(|this, checked, window, cx| {
                this.toggle_include_drafts(*checked, window, cx);
              })),
          )
          .when(self.validation_error.is_some(), |this| {
            this.child(
              div()
                .text_xs()
                .text_color(theme.status_red())
                .child(self.validation_error.clone().unwrap_or_default()),
            )
          }),
      )
      .child(
        DialogFooter::new()
          .px_4()
          .pb_4()
          .pt_1()
          .justify_end()
          .child(
            Button::new("github-tab-cancel")
              .label("Cancel")
              .outline()
              .disabled(self.submit_loading)
              .on_click(|_, window, cx| window.close_dialog(cx)),
          )
          .child(
            div()
              .debug_selector(|| "github-home-pull-request-tab-dialog-submit".to_string())
              .child(
                Button::new("github-tab-submit")
                  .label(self.submit_label())
                  .primary()
                  .loading(self.submit_loading)
                  .disabled(self.submit_loading)
                  .on_click(cx.listener(Self::submit_action)),
              ),
          ),
      )
  }
}

impl GithubPage {
  fn add_github_breadcrumb(&self, message: &str, data: Map<String, Value>) {
    sentry_context::add_breadcrumb("github.page", message, data);
  }

  fn record_github_error(
    &self,
    operation: &'static str,
    error: &str,
    mut data: Map<String, Value>,
  ) {
    data.insert("error".into(), error.to_string().into());
    if github_shared::is_unauthorized_error_message(error) {
      sentry_context::record_expected_error(operation, "unauthorized", data);
      return;
    }

    let io_error = std::io::Error::other(error.to_string());
    sentry_context::capture_unexpected_error(operation, &io_error, data);
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let api = WorkspaceApi::global(cx).api.clone();
    Self::new_with_api(api, window, cx)
  }

  fn new_with_api(api: ApiClient, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let repositories =
      cx.new(|cx| ListState::new(GithubRepositoryListDelegate::new(), window, cx).searchable(true));
    let notifications = cx.new(|cx| {
      VariableListState::new(GithubNotificationListDelegate::new(api.clone()), window, cx)
        .searchable(true)
    });
    let pull_requests = cx.new(|cx| {
      VariableListState::new(GithubPullRequestListDelegate::new(), window, cx).searchable(true)
    });
    let pull_request_tabs =
      make_pull_request_tab_states(ConfigStore::load_or_seed_github_home_pull_request_tabs());
    let active_pull_request_tab_id = pull_request_tabs.first().map(|tab| tab.tab.id.clone());

    let view = Self {
      focus_handle: cx.focus_handle(),
      api,
      repositories,
      notifications,
      pull_requests,
      pull_request_tabs,
      active_pull_request_tab_id,
      load_task: None,
      repositories_task: None,
      notifications_task: None,
      repositories_error: None,
      notifications_error: None,
      error: None,
      access_error: None,
      focus_on_next_render: true,
      subscribe_loading: false,
      subscribe_task: None,
      last_access_state: AuthStateStore::github_access_state(cx),
      _subscriptions: Vec::new(),
    };

    let mut view = view;
    view.subscribe_to_list(cx);

    GithubPageHandle::register(cx);

    view
  }

  #[cfg(test)]
  fn new_for_test(api: ApiClient, window: &mut Window, cx: &mut Context<Self>) -> Self {
    Self::new_with_api(api, window, cx)
  }

  fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
    self.pull_requests.update(cx, |state, cx| {
      state.focus(window, cx);
    });
  }

  fn subscribe_to_list(&mut self, cx: &mut Context<Self>) {
    let pull_requests_subscription = cx.subscribe(
      &self.pull_requests,
      move |_this, state, event: &VariableListEvent, cx| {
        if let VariableListEvent::Confirm(ix) = event {
          let row = state.read(cx).delegate().row_at(*ix);
          if let Some(row) = row {
            GithubPrDetailsPageHandle::show(
              row.pr.repository.owner.clone().into(),
              row.pr.repository.repo.clone().into(),
              row.pr.number,
              cx,
            );
          }
        }
      },
    );
    self._subscriptions.push(pull_requests_subscription);

    let repositories_subscription = cx.subscribe(
      &self.repositories,
      move |_this, state, event: &ListEvent, cx| {
        if let ListEvent::Confirm(ix) = event {
          let row = state.read(cx).delegate().matched_rows.get(ix.row).cloned();
          if let Some(row) = row {
            open_repo_target(
              row.repository.owner.clone(),
              row.repository.repo.clone(),
              None,
              None,
              None,
              cx,
            );
          }
        }
      },
    );
    self._subscriptions.push(repositories_subscription);

    let api = self.api.clone();
    let notifications_entity = self.notifications.downgrade();
    let notifications_subscription = cx.subscribe(
      &self.notifications,
      move |_this, state, event: &VariableListEvent, cx| {
        if let VariableListEvent::Confirm(ix) = event {
          let row = state.read(cx).delegate().row_at(*ix);
          if let Some(row) = row {
            let notification = &row.notification;
            let full_name = &notification.repository.full_name;
            let (owner, repo) = full_name.split_once('/').unwrap_or((full_name, ""));

            if notification.unread {
              let thread_id = notification.id.clone();
              let api = api.clone();
              let notifications_entity = notifications_entity.clone();

              if let Some(entity) = notifications_entity.upgrade() {
                entity.update(cx, |state, cx| {
                  let delegate = state.delegate_mut();
                  delegate.all_rows = delegate
                    .all_rows
                    .iter()
                    .map(|r| {
                      if r.notification.id == thread_id {
                        let mut updated = (*r.notification).clone();
                        updated.unread = false;
                        Rc::new(GithubNotificationRow {
                          notification: Rc::new(updated),
                        })
                      } else {
                        r.clone()
                      }
                    })
                    .collect();
                  delegate.prepare(delegate.query.clone());
                  let unread = delegate.unread_count();
                  NotificationCountStore::set(cx, unread);
                  set_dock_badge(unread);
                  cx.notify();
                });
              }

              cx.spawn(async move |_, _| {
                let _ = unblock(move || api.mark_notification_read(&thread_id)).await;
              })
              .detach();
            }

            match notification.subject.subject_type.as_str() {
              "PullRequest" => {
                if let Some(number) = notification
                  .subject
                  .url
                  .as_deref()
                  .and_then(extract_number_from_api_url)
                {
                  open_pr_target(
                    owner.to_string(),
                    repo.to_string(),
                    number,
                    false,
                    None,
                    None,
                    cx,
                  );
                }
              }
              "Issue" => {
                let issue_number = notification
                  .subject
                  .url
                  .as_deref()
                  .and_then(extract_number_from_api_url);
                open_repo_target(
                  owner.to_string(),
                  repo.to_string(),
                  Some(CommandPaletteGithubRepoTab::Issues),
                  issue_number,
                  None,
                  cx,
                );
              }
              _ => {
                let url = notification
                  .subject
                  .url
                  .as_deref()
                  .map(|api_url| {
                    github_html_url_from_notification(
                      full_name,
                      &notification.subject.subject_type,
                      api_url,
                    )
                  })
                  .unwrap_or_else(|| format!("https://github.com/{full_name}"));
                cx.open_url(&url);
              }
            }
          }
        }
      },
    );
    self._subscriptions.push(notifications_subscription);
  }

  fn active_pull_request_tab_index(&self) -> Option<usize> {
    let active_id = self.active_pull_request_tab_id.as_ref()?;
    self
      .pull_request_tabs
      .iter()
      .position(|tab_state| &tab_state.tab.id == active_id)
  }

  fn managing_pull_request_tabs(&self) -> bool {
    self.active_pull_request_tab_id.as_deref() == Some(GITHUB_HOME_MANAGE_TABS_ID)
  }

  fn active_pull_request_inbox_tab_index(&self) -> usize {
    self
      .active_pull_request_tab_index()
      .unwrap_or(self.pull_request_tabs.len())
  }

  fn active_pull_request_tab_state(&self) -> Option<&GithubPullRequestTabState> {
    self
      .active_pull_request_tab_index()
      .and_then(|index| self.pull_request_tabs.get(index))
  }

  fn active_pull_request_tab_state_mut(&mut self) -> Option<&mut GithubPullRequestTabState> {
    let index = self.active_pull_request_tab_index()?;
    self.pull_request_tabs.get_mut(index)
  }

  fn active_pull_request_rows(&self) -> Vec<Rc<GithubPullRequestRow>> {
    self
      .active_pull_request_tab_state()
      .map(|tab_state| tab_state.rows.clone())
      .unwrap_or_default()
  }

  fn apply_active_pull_request_rows(&mut self, cx: &mut Context<Self>) {
    let rows = self.active_pull_request_rows();
    self.pull_requests.update(cx, |state, cx| {
      state.delegate_mut().show_author = true;
      state.delegate_mut().set_rows(rows);
      cx.notify();
    });
  }

  fn ensure_active_pull_request_tab(&mut self) {
    let has_active = self
      .active_pull_request_tab_id
      .as_ref()
      .is_some_and(|active_id| {
        active_id == GITHUB_HOME_MANAGE_TABS_ID
          || self
            .pull_request_tabs
            .iter()
            .any(|tab| &tab.tab.id == active_id)
      });
    if has_active {
      return;
    }
    self.active_pull_request_tab_id = self
      .pull_request_tabs
      .first()
      .map(|tab| tab.tab.id.clone())
      .or_else(|| Some(GITHUB_HOME_MANAGE_TABS_ID.to_string()));
  }

  fn pull_request_tab_configs(&self) -> Vec<GithubHomePullRequestTab> {
    self
      .pull_request_tabs
      .iter()
      .map(|tab_state| tab_state.tab.clone())
      .collect()
  }

  fn persist_pull_request_tabs(&self) {
    ConfigStore::persist_github_home_pull_request_tabs(&self.pull_request_tab_configs());
  }

  fn move_pull_request_tab(&mut self, from_index: usize, to_index: usize, cx: &mut Context<Self>) {
    if !move_item_in_vec(&mut self.pull_request_tabs, from_index, to_index) {
      return;
    }

    self.ensure_active_pull_request_tab();
    self.persist_pull_request_tabs();
    self.apply_active_pull_request_rows(cx);
    cx.notify();
  }

  fn move_pull_request_tab_before(
    &mut self,
    dragged_tab_id: &str,
    target_index: usize,
    cx: &mut Context<Self>,
  ) {
    let Some(from_index) = self
      .pull_request_tabs
      .iter()
      .position(|tab_state| tab_state.tab.id == dragged_tab_id)
    else {
      return;
    };

    self.move_pull_request_tab(from_index, target_index, cx);
  }

  fn move_pull_request_tab_to_end(&mut self, dragged_tab_id: &str, cx: &mut Context<Self>) {
    let Some(from_index) = self
      .pull_request_tabs
      .iter()
      .position(|tab_state| tab_state.tab.id == dragged_tab_id)
    else {
      return;
    };

    self.move_pull_request_tab(from_index, self.pull_request_tabs.len(), cx);
  }

  fn set_active_pull_request_tab(
    &mut self,
    index: usize,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if index == self.pull_request_tabs.len() {
      if self.managing_pull_request_tabs() {
        return;
      }

      self.active_pull_request_tab_id = Some(GITHUB_HOME_MANAGE_TABS_ID.to_string());
      self.apply_active_pull_request_rows(cx);
      cx.notify();
      return;
    }

    let Some((tab_id, should_refresh)) = self.pull_request_tabs.get(index).map(|tab_state| {
      (
        tab_state.tab.id.clone(),
        !tab_state.loaded_once && !tab_state.loading,
      )
    }) else {
      return;
    };
    if self.active_pull_request_tab_id.as_deref() == Some(tab_id.as_str()) {
      return;
    }

    self.active_pull_request_tab_id = Some(tab_id);
    self.apply_active_pull_request_rows(cx);
    if should_refresh {
      self.refresh_active_pull_request_tab(cx);
    }
    cx.notify();
  }

  fn refresh_active_pull_request_tab(&mut self, cx: &mut Context<Self>) {
    let Some(active_tab) = self
      .active_pull_request_tab_state()
      .map(|tab_state| tab_state.tab.clone())
    else {
      self.error = None;
      self.pull_requests.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        state.delegate_mut().set_rows(Vec::new());
        cx.notify();
      });
      return;
    };

    let active_tab_id = active_tab.id.clone();
    let active_tab_name = active_tab.name.clone();
    let filters = active_tab.filters.clone();
    let api = self.api.clone();
    self.add_github_breadcrumb("Refresh pull requests started", Map::new());
    self.error = None;

    if let Some(tab_state) = self.active_pull_request_tab_state_mut() {
      tab_state.loading = true;
      tab_state.error = None;
    }

    self.pull_requests.update(cx, |state, cx| {
      state.delegate_mut().loading = true;
      cx.notify();
    });

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_github_pull_requests(&filters))
        .await
        .map_err(|error| error.to_string());

      let _ = this.update(cx, |this, cx| {
        let (rows, error): (Vec<Rc<GithubPullRequestRow>>, Option<SharedString>) = match result {
          Ok(pull_requests) => (
            pull_requests
              .into_iter()
              .map(|pr| Rc::new(GithubPullRequestRow { pr: Rc::new(pr) }))
              .collect::<Vec<_>>(),
            None,
          ),
          Err(error) => (Vec::new(), Some(error.into())),
        };

        match error.as_ref() {
          Some(error) => {
            let mut data = Map::new();
            data.insert("tab".into(), active_tab_name.clone().into());
            this.add_github_breadcrumb("Refresh pull requests failed", data.clone());
            this.record_github_error("github.pull_requests.refresh", error.as_ref(), data);
          }
          None => {
            let mut data = Map::new();
            data.insert("tab".into(), active_tab_name.into());
            data.insert("count".into(), rows.len().into());
            this.add_github_breadcrumb("Refresh pull requests succeeded", data);
          }
        }

        if let Some(tab_state) = this
          .pull_request_tabs
          .iter_mut()
          .find(|tab_state| tab_state.tab.id == active_tab_id)
        {
          tab_state.rows = rows.clone();
          tab_state.loading = false;
          tab_state.error = error.clone();
          tab_state.loaded_once = true;
        }

        this.error = error;
        this.pull_requests.update(cx, |state, cx| {
          state.delegate_mut().loading = false;
          cx.notify();
        });
        this.apply_active_pull_request_rows(cx);
        cx.notify();
      });
    });

    self.load_task = Some(task);
  }

  fn available_repository_filters(&self, cx: &App) -> Vec<GithubUserRepository> {
    self
      .repositories
      .read(cx)
      .delegate()
      .all_rows
      .iter()
      .map(|row| (*row.repository).clone())
      .collect()
  }

  fn open_pull_request_tab_dialog(
    &mut self,
    mode: GithubPullRequestTabDialogMode,
    initial_tab: Option<GithubHomePullRequestTab>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let api = self.api.clone();
    let window_handle = window.window_handle();
    let github_page = cx.entity().downgrade();
    let available_repositories = self.available_repository_filters(cx);
    let dialog = cx.new(|cx| {
      GithubPullRequestTabDialog::new(
        api.clone(),
        window_handle,
        github_page,
        mode,
        initial_tab,
        available_repositories,
        window,
        cx,
      )
    });
    let dialog_for_overlay = dialog.clone();
    let dialog_for_focus = dialog.clone();

    window.open_dialog(cx, move |overlay, window, _| {
      let use_relative_height = window.viewport_size().height <= px(950.0);

      overlay
        .p_0()
        .min_h_0()
        .w(px(680.0))
        .when(use_relative_height, |this| this.h(relative(0.85)))
        .child(dialog_for_overlay.clone())
    });

    window.on_next_frame(move |window, cx| {
      let focus_handle = dialog_for_focus.read(cx).focus_handle(cx);
      window.focus(&focus_handle, cx);
    });
  }

  fn open_create_pull_request_tab_dialog(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_pull_request_tab_dialog(GithubPullRequestTabDialogMode::Create, None, window, cx);
  }

  fn render_manage_pull_request_tab_row(
    &self,
    index: usize,
    tab_state: &GithubPullRequestTabState,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let tab = tab_state.tab.clone();
    let edit_tab = tab.clone();
    let delete_tab = tab.clone();
    let drag_tab = DraggedPullRequestTab {
      tab_id: tab.id.clone(),
      name: tab.name.clone().into(),
    };
    let drag_tab_id = tab.id.clone();
    let filter_labels = pull_request_tab_filter_tag_labels(&tab.filters);

    v_flex()
      .id(("github-home-manage-tab-row", index))
      .gap_2()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .bg(theme.background)
      .p_3()
      .drag_over::<DraggedPullRequestTab>(move |this, dragged_tab, _, cx| {
        if dragged_tab.tab_id == drag_tab_id {
          this
        } else {
          this
            .border_color(cx.theme().drag_border)
            .bg(cx.theme().drop_target)
        }
      })
      .on_drop(cx.listener(
        move |this, dragged_tab: &DraggedPullRequestTab, _window, cx| {
          this.move_pull_request_tab_before(&dragged_tab.tab_id, index, cx);
        },
      ))
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_3()
          .child(
            h_flex()
              .items_start()
              .gap_3()
              .flex_1()
              .min_w_0()
              .child(
                div()
                  .id(("github-home-manage-drag-pr-tab", index))
                  .mt_1()
                  .cursor_move()
                  .text_color(theme.muted_foreground)
                  .on_drag(drag_tab, |drag, position, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| DraggedPullRequestTabPreview::new(drag.name.clone(), position))
                  })
                  .child(Icon::new(UiIconName::EllipsisVertical).size_4()),
              )
              .child(
                v_flex()
                  .gap_1()
                  .flex_1()
                  .min_w_0()
                  .child(tab.name.clone())
                  .child(if filter_labels.is_empty() {
                    div()
                      .text_sm()
                      .text_color(theme.muted_foreground)
                      .child("All open pull requests")
                      .into_any_element()
                  } else {
                    h_flex()
                      .gap_1()
                      .flex_wrap()
                      .children(filter_labels.into_iter().map(|label| {
                        Tag::secondary()
                          .small()
                          .rounded_full()
                          .child(label)
                          .into_any_element()
                      }))
                      .into_any_element()
                  }),
              ),
          )
          .child(
            h_flex()
              .gap_2()
              .child(
                Button::new(("github-home-manage-edit-pr-tab", index))
                  .label("Edit")
                  .icon(UiIconName::SquarePen)
                  .small()
                  .outline()
                  .on_click({
                    let view = cx.entity().clone();
                    move |_, window, cx| {
                      view.update(cx, |this, cx| {
                        this.open_pull_request_tab_dialog(
                          GithubPullRequestTabDialogMode::Edit,
                          Some(edit_tab.clone()),
                          window,
                          cx,
                        );
                      });
                    }
                  }),
              )
              .child(
                Button::new(("github-home-manage-delete-pr-tab", index))
                  .label("Delete")
                  .small()
                  .danger()
                  .on_click({
                    let view = cx.entity().clone();
                    move |_, window, cx| {
                      view.update(cx, |this, cx| {
                        this.confirm_delete_pull_request_tab(delete_tab.clone(), window, cx);
                      });
                    }
                  }),
              ),
          ),
      )
      .into_any_element()
  }

  fn render_manage_pull_request_tab_end_drop_zone(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();

    h_flex()
      .id("github-home-manage-tab-end-drop-zone")
      .items_center()
      .justify_center()
      .h_10()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .text_sm()
      .text_color(theme.muted_foreground)
      .child("Drag here to move to the end")
      .drag_over::<DraggedPullRequestTab>(|this, _, _, cx| {
        this
          .border_color(cx.theme().drag_border)
          .bg(cx.theme().drop_target)
      })
      .on_drop(
        cx.listener(|this, dragged_tab: &DraggedPullRequestTab, _window, cx| {
          this.move_pull_request_tab_to_end(&dragged_tab.tab_id, cx);
        }),
      )
      .into_any_element()
  }

  fn render_manage_pull_request_tabs(
    &self,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let mut tab_rows = Vec::with_capacity(self.pull_request_tabs.len());
    for (index, tab_state) in self.pull_request_tabs.iter().enumerate() {
      tab_rows.push(self.render_manage_pull_request_tab_row(index, tab_state, cx));
    }
    let end_drop_zone = self.render_manage_pull_request_tab_end_drop_zone(cx);

    v_flex()
      .gap_3()
      .flex_1()
      .min_h_0()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .child(
            v_flex()
              .gap_1()
              .child(
                h_flex()
                  .items_center()
                  .gap_2()
                  .child(Icon::new(IconName::Settings2).size_4())
                  .child("Manage tabs"),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .child("Drag to reorder, then edit or delete saved pull request lists."),
              ),
          )
          .child(
            Button::new("github-home-manage-add-pr-tab")
              .label("Add list")
              .icon(IconName::Plus)
              .small()
              .primary()
              .on_click(cx.listener(GithubPage::open_create_pull_request_tab_dialog)),
          ),
      )
      .when(self.pull_request_tabs.is_empty(), |this| {
        this.child(
          v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap_3()
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius)
            .p_4()
            .text_color(theme.muted_foreground)
            .child(Icon::new(UiIconName::GitPullRequestArrow).size_6())
            .child("No saved pull request lists")
            .child(
              Button::new("github-home-manage-empty-add-pr-tab")
                .label("Create first list")
                .small()
                .primary()
                .on_click(cx.listener(GithubPage::open_create_pull_request_tab_dialog)),
            ),
        )
      })
      .when(!self.pull_request_tabs.is_empty(), |this| {
        this.child(
          v_flex()
            .gap_2()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .children(tab_rows.into_iter().chain(std::iter::once(end_drop_zone))),
        )
      })
      .into_any_element()
  }

  fn confirm_delete_pull_request_tab(
    &mut self,
    tab: GithubHomePullRequestTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let (title, message) = pull_request_tab_delete_confirmation(&tab.name);
    let tab_id = tab.id.clone();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let tab_id = tab_id.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Delete")
        .cancel_text("Cancel")
        .destructive()
        .on_confirm(move |_, _, cx| {
          view.update(cx, |this, cx| {
            this.delete_pull_request_tab(&tab_id, cx);
          });
          true
        })
        .build(alert)
    });
  }

  fn save_pull_request_tab(
    &mut self,
    tab: GithubHomePullRequestTab,
    original_tab_id: Option<String>,
    cx: &mut Context<Self>,
  ) {
    let existing_index = original_tab_id.as_ref().and_then(|tab_id| {
      self
        .pull_request_tabs
        .iter()
        .position(|state| state.tab.id == *tab_id)
    });

    if let Some(index) = existing_index {
      if let Some(tab_state) = self.pull_request_tabs.get_mut(index) {
        tab_state.tab = tab.clone();
        tab_state.rows.clear();
        tab_state.loading = false;
        tab_state.error = None;
        tab_state.loaded_once = false;
      }
    } else {
      self
        .pull_request_tabs
        .push(GithubPullRequestTabState::new(tab.clone()));
    }

    self.active_pull_request_tab_id = Some(tab.id.clone());
    self.persist_pull_request_tabs();
    self.apply_active_pull_request_rows(cx);
    self.refresh_active_pull_request_tab(cx);
    cx.notify();
  }

  fn delete_pull_request_tab(&mut self, tab_id: &str, cx: &mut Context<Self>) {
    let removed_index = self
      .pull_request_tabs
      .iter()
      .position(|tab_state| tab_state.tab.id == tab_id);
    self
      .pull_request_tabs
      .retain(|tab_state| tab_state.tab.id != tab_id);

    if self.active_pull_request_tab_id.as_deref() == Some(tab_id) {
      self.active_pull_request_tab_id = removed_index.and_then(|index| {
        if self.pull_request_tabs.is_empty() {
          None
        } else {
          let next_index = index.min(self.pull_request_tabs.len().saturating_sub(1));
          self
            .pull_request_tabs
            .get(next_index)
            .map(|tab_state| tab_state.tab.id.clone())
        }
      });
    }

    self.ensure_active_pull_request_tab();
    self.persist_pull_request_tabs();
    self.apply_active_pull_request_rows(cx);
    if self
      .active_pull_request_tab_state()
      .is_some_and(|tab_state| !tab_state.loaded_once)
    {
      self.refresh_active_pull_request_tab(cx);
    }
    cx.notify();
  }

  fn subscribe_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    self.start_checkout(cx);
  }

  fn start_checkout(&mut self, cx: &mut Context<Self>) {
    if self.subscribe_loading {
      return;
    }

    self.subscribe_loading = true;
    self.access_error = None;

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.checkout_subscription("pro"))
        .await
        .map_err(|error| error.to_string());

      match result {
        Ok(url) => {
          cx.update(|cx| cx.open_url(&url));
          let _ = this.update(cx, |this, cx| {
            this.subscribe_loading = false;
            cx.notify();
          });
        }
        Err(error) => {
          let _ = this.update(cx, |this, cx| {
            this.subscribe_loading = false;
            this.access_error = Some(error.into());
            cx.notify();
          });
        }
      }
    });

    self.subscribe_task = Some(task);
    cx.notify();
  }

  fn refresh_pull_requests(&mut self, cx: &mut Context<Self>) {
    self.refresh_active_pull_request_tab(cx);
    self.refresh_notifications(cx);
    self.refresh_repositories(cx);
  }

  fn refresh_notifications(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    self.add_github_breadcrumb("Refresh notifications started", Map::new());
    self.notifications_error = None;
    self.notifications.update(cx, |state, cx| {
      state.delegate_mut().loading = true;
      cx.notify();
    });

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_github_notifications())
        .await
        .map_err(|error| error.to_string());

      let _ = this.update(cx, |this, cx| {
        let (rows, error): (Vec<Rc<GithubNotificationRow>>, Option<SharedString>) = match result {
          Ok(notifications) => (
            notifications
              .into_iter()
              .map(|notification| {
                Rc::new(GithubNotificationRow {
                  notification: Rc::new(notification),
                })
              })
              .collect::<Vec<_>>(),
            None,
          ),
          Err(error) => (Vec::new(), Some(error.into())),
        };

        match error.as_ref() {
          Some(error) => {
            let data = Map::new();
            this.add_github_breadcrumb("Refresh notifications failed", data.clone());
            this.record_github_error("github.notifications.refresh", error.as_ref(), data);
          }
          None => {
            let mut data = Map::new();
            data.insert("count".into(), rows.len().into());
            this.add_github_breadcrumb("Refresh notifications succeeded", data);
          }
        }

        this.notifications_error = error;

        let unread = rows.iter().filter(|r| r.notification.unread).count();
        NotificationCountStore::set(cx, unread);
        set_dock_badge(unread);
        {
          let notifications: Vec<_> = rows.iter().map(|r| (*r.notification).clone()).collect();
          crate::status_bar::update_status_bar(unread, &notifications);
        }

        this.notifications.update(cx, |state, cx| {
          state.delegate_mut().loading = false;
          state.delegate_mut().set_rows(rows);
          cx.notify();
        });

        cx.notify();
      });
    });

    self.notifications_task = Some(task);
  }

  fn refresh_repositories(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    self.add_github_breadcrumb("Refresh repositories started", Map::new());
    self.repositories_error = None;
    self.repositories.update(cx, |state, cx| {
      state.delegate_mut().loading = true;
      cx.notify();
    });

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_github_user_repositories())
        .await
        .map_err(|error| error.to_string());

      let _ = this.update(cx, |this, cx| {
        let (rows, error): (Vec<Rc<GithubRepositoryRow>>, Option<SharedString>) = match result {
          Ok(repositories) => (
            repositories
              .into_iter()
              .map(|repository| {
                Rc::new(GithubRepositoryRow {
                  repository: Rc::new(repository),
                  pinned: false,
                })
              })
              .collect::<Vec<_>>(),
            None,
          ),
          Err(error) => (Vec::new(), Some(error.into())),
        };

        match error.as_ref() {
          Some(error) => {
            let data = Map::new();
            this.add_github_breadcrumb("Refresh repositories failed", data.clone());
            this.record_github_error("github.repositories.refresh", error.as_ref(), data);
          }
          None => {
            let mut data = Map::new();
            data.insert("count".into(), rows.len().into());
            this.add_github_breadcrumb("Refresh repositories succeeded", data);
          }
        }

        this.repositories_error = error;
        this.repositories.update(cx, |state, cx| {
          state.delegate_mut().loading = false;
          state.delegate_mut().set_rows(rows);
          cx.notify();
        });

        cx.notify();
      });
    });

    self.repositories_task = Some(task);
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
    let include_github = AuthStateStore::has_github_access(cx);
    let commands =
      CommandPaletteCommand::default_global_commands(CommandPalettePage::Github, include_github);

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
      CommandPaletteAction::OpenGitPage => {
        NavigationHistory::navigate("/git", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPage => {
        self.focus_on_next_render = true;
        self.refresh_pull_requests(cx);
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
        open_pr_target(
          owner,
          repo,
          number,
          open_changes_tab,
          review_comment_id,
          None,
          cx,
        );
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
      _ => Err("Command not available.".into()),
    }
  }

  fn render_access_feature_card(
    icon: Icon,
    title: &'static str,
    description: &'static str,
    theme: &gpui_component::Theme,
  ) -> impl IntoElement {
    div()
      .flex()
      .flex_col()
      .gap_3()
      .p_4()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .bg(theme.sidebar)
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_2()
          .child(icon.size_4())
          .child(StatusTag::new(theme.status_blue()).outline().child("Pro")),
      )
      .child(
        div()
          .text_sm()
          .font_semibold()
          .text_color(theme.foreground)
          .child(title),
      )
      .child(
        div()
          .max_w(px(400.0))
          .text_sm()
          .text_color(theme.muted_foreground)
          .child(description),
      )
  }

  fn render_locked_eyebrow(&self, theme: &gpui_component::Theme) -> AnyElement {
    h_flex()
      .items_center()
      .gap_0()
      .child(
        div()
          .text_xs()
          .font_semibold()
          .text_color(theme.foreground)
          .child("Rev"),
      )
      .child(
        div()
          .text_xs()
          .font_semibold()
          .text_color(theme.status_blue())
          .child("iu Pro"),
      )
      .into_any_element()
  }

  fn render_locked_page(
    &mut self,
    access_state: GithubAccessState,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let presentation = github_locked_presentation(access_state)
      .expect("locked page should only render for unavailable GitHub access");
    let launch_offer = active_reviu_pro_launch_offer();
    let action = match presentation.action {
      GithubLockedAction::SignIn => Button::new("github-access-sign-in")
        .icon(IconName::Github)
        .label("Sign in with GitHub")
        .small()
        .on_click(|_, _, cx| {
          AuthCallbackTarget::start_sign_in(cx);
        })
        .into_any_element(),
      GithubLockedAction::Subscribe => reviu_pro_checkout_button(
        "github-access-subscribe",
        ReviuProCheckoutCta::StartFreeTrial,
      )
      .loading(self.subscribe_loading)
      .disabled(self.subscribe_loading)
      .on_click(cx.listener(Self::subscribe_action))
      .into_any_element(),
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GithubPage::show_command_palette_action))
      .child(
        div().w_full().h_full().min_h_0().overflow_y_scrollbar().child(
          div().flex().flex_col()
            .w_full().mt_20()
            .max_w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH))
            .w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH))
            .mx_auto()
            .gap_4()
            .p_4()
            .child(
              div().flex().flex_col()
                .gap_4()
                .p_4()
                .pb_9()
                .border_1()
                .border_color(theme.border)
                .rounded(theme.radius)
                .bg(theme.sidebar)
                .child(
                  div().flex()
                    .min_w_0()
                    .items_start()
                    .gap_3()
                    .child(
                      div().mt_6()
                        .size_12()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme.background)
                        .border_1()
                        .border_color(theme.border)
                        .child(Icon::new(IconName::Github).size_6()),
                    )
                    .child(
                      div().flex().flex_col()
                        .min_w_0()
                        .flex_1()
                        .gap_1()
                        .child(self.render_locked_eyebrow(&theme))
                        .child(
                          div()
                            .text_3xl()
                            .font_semibold()
                            .text_color(theme.foreground)
                            .child(presentation.title),
                        )
                        .child(
                          div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(presentation.description),
                        ).child(div()
                  .flex()
                  .flex_col()
                        .gap_3()
                        .child(
                          div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .when_some(launch_offer, |this, offer| {
                              this.child(
                                h_flex()
                                  .items_end()
                                  .gap_2()
                                  .child(
                                    div()
                                      .text_sm()
                                      .text_color(theme.muted_foreground)
                                      .line_through()
                                      .child(offer.regular_price),
                                  )
                                  .child(
                                    div()
                                      .text_xl()
                                      .font_semibold()
                                      .text_color(theme.foreground)
                                      .child(offer.launch_price),
                                  )
                                  .child(
                                    div()
                                      .text_sm()
                                      .text_color(theme.muted_foreground)
                                      .child(offer.billing_period),
                                  ),
                              )
                            })
                            .when(launch_offer.is_none(), |this| {
                              this.child(
                                h_flex()
                                  .items_end()
                                  .gap_2()
                                  .child(
                                    div()
                                      .text_xl()
                                      .font_semibold()
                                      .text_color(theme.foreground)
                                      .child("$19"),
                                  )
                                  .child(
                                    div()
                                      .text_sm()
                                      .text_color(theme.muted_foreground)
                                      .child("/ month"),
                                  ),
                              )
                            }),
                        )
                        .child(div().flex().justify_start().child(action))
                        .when_some(self.access_error.clone(), |this, error| {
                          this.child(div().text_sm().text_color(theme.status_red()).child(error))
                        })),
                    ),
                )
            )
            .child(
              div().grid()
                .grid_cols(2)
                .gap_3()
                .child(Self::render_access_feature_card(
                  Icon::new(IconName::Bell),
                  "GitHub notifications in-app",
                  "Track unread threads and review work from a desktop inbox without bouncing back to the browser.",
                  &theme,
                ))
                .child(Self::render_access_feature_card(
                  Icon::new(IconName::Folder),
                  "Browse repos, pull requests, and issues",
                  "Open Overview, Readme, Code, Pull Requests, and Issues from one place inside Reviu.",
                  &theme,
                ))
                .child(Self::render_access_feature_card(
                  Icon::new(UiIconName::GitPullRequestArrow),
                  "Desktop pull request review",
                  "Review changed files in inline or split diff mode with markdown and SVG previews plus full comment actions.",
                  &theme,
                ))
                .child(Self::render_access_feature_card(
                  Icon::new(UiIconName::GitBranch),
                  "Branch-to-PR shortcuts",
                  "Jump from the Git page to the pull request for the current branch, or open GitHub to create one when none exists.",
                  &theme,
                )),
            ),
        ),
      )
  }
}

impl Render for GithubPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let access_state = AuthStateStore::github_access_state(cx);
    let auth_state = AuthStateStore::get(cx);

    if access_state != self.last_access_state {
      self.last_access_state = access_state;
      if matches!(access_state, GithubAccessState::Available) {
        self.focus_on_next_render = true;
        self.refresh_pull_requests(cx);
      }
    }

    if !matches!(access_state, GithubAccessState::Available) {
      self.focus_on_next_render = false;
      return self
        .render_locked_page(access_state, window, cx)
        .into_any_element();
    }

    if self.focus_on_next_render {
      self.focus_on_next_render = false;
      cx.on_next_frame(window, |this, window, cx| this.focus_search(window, cx));
    }

    let pull_requests_search_placeholder = self
      .active_pull_request_tab_state()
      .map(|tab_state| format!("Search {}...", tab_state.tab.name.to_lowercase()))
      .unwrap_or_else(|| "Search pull requests...".to_string());
    let show_manage_tabs = self.managing_pull_request_tabs();
    let now = OffsetDateTime::now_utc();
    let greeting = github_home_greeting_at(
      github_home_display_name(&auth_state)
        .as_deref()
        .map(|display_name| &**display_name),
      now,
    );
    let date_label = github_home_date_label_at(now);

    let pull_requests_list = VariableList::new(&self.pull_requests)
      .search_placeholder(pull_requests_search_placeholder)
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .flex_1()
      .min_w(px(0.0))
      .min_h_0()
      .p(px(8.));
    let notifications_list = VariableList::new(&self.notifications)
      .search_placeholder("Search notifications...")
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .flex_1()
      .min_h_0()
      .p(px(8.));
    let repositories_list = List::new(&self.repositories)
      .search_placeholder("Search repositories...")
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .flex_1()
      .min_h_0()
      .p(px(8.));
    let unread_count = self.notifications.read(cx).delegate().unread_count();
    let unread_notification_label = github_notification_count_label(unread_count);
    let notifications_count = self.notifications.read(cx).delegate().matched_rows.len();
    let pr_tabs = TabBar::new("github-home-pr-tabs")
      .w_full()
      .segmented()
      .selected_index(self.active_pull_request_inbox_tab_index())
      .on_click(cx.listener(|this, ix: &usize, window, cx| {
        this.set_active_pull_request_tab(*ix, window, cx);
      }))
      .children(
        self
          .pull_request_tabs
          .iter()
          .map(|tab_state| {
            Tab::new().child(
              h_flex()
                .items_center()
                .gap_2()
                .child(tab_state.tab.name.clone())
                .child(
                  Tag::secondary()
                    .small()
                    .rounded_full()
                    .child(tab_state.rows.len().to_string()),
                ),
            )
          })
          .chain(std::iter::once(
            Tab::new().child(
              h_flex()
                .items_center()
                .gap_2()
                .child(Icon::new(IconName::Settings2).size_4())
                .child("Manage tabs"),
            ),
          )),
      );

    let notifications_panel = v_flex()
      .debug_selector(|| GITHUB_HOME_NOTIFICATIONS_PANEL_DEBUG_SELECTOR.to_string())
      .gap_2()
      .flex_1()
      .min_h_0()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(Icon::new(IconName::Bell).size_4())
              .child("Notifications")
              .when_some(unread_notification_label, |this, label| {
                this.child(
                  div()
                    .debug_selector(|| {
                      GITHUB_HOME_NOTIFICATIONS_UNREAD_BADGE_DEBUG_SELECTOR.to_string()
                    })
                    .child(StatusTag::new(theme.status_red()).xsmall().child(label)),
                )
              }),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .when(notifications_count > 0, |this| {
                this.child(
                  div()
                    .debug_selector(|| {
                      GITHUB_HOME_NOTIFICATIONS_COUNT_BADGE_DEBUG_SELECTOR.to_string()
                    })
                    .child(
                      Tag::secondary()
                        .small()
                        .rounded_full()
                        .child(notifications_count.to_string()),
                    ),
                )
              }),
          ),
      )
      .when_some(self.notifications_error.clone(), |this, error| {
        this.child(div().text_sm().text_color(theme.status_red()).child(error))
      })
      .child(notifications_list);

    let repositories_count = self.repositories.read(cx).delegate().matched_rows.len();
    let repositories_panel = v_flex()
      .debug_selector(|| GITHUB_HOME_REPOSITORIES_PANEL_DEBUG_SELECTOR.to_string())
      .gap_2()
      .flex_1()
      .min_h_0()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(Icon::new(IconName::Folder).size_4())
              .child("My Repositories"),
          )
          .when(repositories_count > 0, |this| {
            this.child(
              Tag::secondary()
                .small()
                .rounded_full()
                .child(repositories_count.to_string()),
            )
          }),
      )
      .when_some(self.repositories_error.clone(), |this, error| {
        this.child(div().text_sm().text_color(theme.status_red()).child(error))
      })
      .child(repositories_list);

    let right_panel = v_flex()
      .debug_selector(|| GITHUB_HOME_REVIEW_INBOX_PANEL_DEBUG_SELECTOR.to_string())
      .gap_2()
      .flex_1()
      .min_w_0()
      .h_full()
      .min_h_0()
      .child(pr_tabs)
      .when(!show_manage_tabs, |this| {
        this.when_some(self.error.clone(), |this, error| {
          this.child(div().text_sm().text_color(theme.status_red()).child(error))
        })
      })
      .child(if show_manage_tabs {
        self.render_manage_pull_request_tabs(window, cx)
      } else {
        pull_requests_list.into_any_element()
      });

    let left_column = v_flex()
      .w(px(600.0))
      .h_full()
      .min_h_0()
      .gap_6()
      .child(notifications_panel)
      .child(repositories_panel);

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GithubPage::show_command_palette_action))
      .child(
        v_flex()
          .w_full()
          .mx_auto()
          .h_full()
          .min_h_0()
          .gap_5()
          .p_4()
          .child(
            v_flex()
              .child(
                div()
                  .font_semibold()
                  .text_color(theme.foreground)
                  .child(greeting),
              )
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(date_label),
              ),
          )
          .child(
            h_flex()
              .flex_1()
              .gap_10()
              .min_h_0()
              .items_start()
              .child(left_column)
              .child(right_panel),
          ),
      )
      .into_any_element()
  }
}

impl Focusable for GithubPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::api::{
    ApiClient, GithubNotification, GithubNotificationRepository, GithubNotificationSubject,
    GithubPullRequest, GithubPullRequestAuthor, GithubPullRequestLabel, GithubPullRequestState,
    GithubRepository, GithubUserRepository, User, UserRole, UserSubscription,
  };
  use crate::auth_state::AuthState;
  use gpui::{Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowOptions, point, size};
  use std::{
    fs,
    ops::Deref,
    path::PathBuf,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
  };

  struct GithubPageTestConfigGuard {
    path: PathBuf,
  }

  impl GithubPageTestConfigGuard {
    fn new(name: &str) -> Self {
      let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
      let path = std::env::temp_dir().join(format!("reviu-github-page-{name}-{timestamp}.sqlite"));
      let _ = fs::remove_file(&path);
      let _ = fs::remove_file(path.with_extension("sqlite-shm"));
      let _ = fs::remove_file(path.with_extension("sqlite-wal"));
      ConfigStore::set_test_db_path(Some(path.clone()));
      Self { path }
    }
  }

  impl Drop for GithubPageTestConfigGuard {
    fn drop(&mut self) {
      ConfigStore::set_test_db_path(None);
      let _ = fs::remove_file(&self.path);
      let _ = fs::remove_file(self.path.with_extension("sqlite-shm"));
      let _ = fs::remove_file(self.path.with_extension("sqlite-wal"));
    }
  }

  fn make_pull_request_row(title: &str, owner: &str, repo: &str) -> GithubPullRequestRow {
    make_pull_request_row_with_labels(title, owner, repo, &["test"])
  }

  fn make_pull_request_row_with_labels(
    title: &str,
    owner: &str,
    repo: &str,
    labels: &[&str],
  ) -> GithubPullRequestRow {
    GithubPullRequestRow {
      pr: Rc::new(GithubPullRequest {
        number: 1,
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
            color: Some("f29513".to_string()),
          })
          .collect(),
        repository: GithubRepository {
          owner: owner.to_string(),
          repo: repo.to_string(),
        },
      }),
    }
  }

  fn make_notification_row(
    title: &str,
    full_name: &str,
    reason: &str,
    unread: bool,
  ) -> GithubNotificationRow {
    GithubNotificationRow {
      notification: Rc::new(GithubNotification {
        id: "1".to_string(),
        repository: GithubNotificationRepository {
          name: full_name
            .split('/')
            .next_back()
            .unwrap_or(full_name)
            .to_string(),
          full_name: full_name.to_string(),
          owner: None,
        },
        subject: GithubNotificationSubject {
          title: title.to_string(),
          subject_type: "PullRequest".to_string(),
          url: None,
          latest_comment_url: None,
        },
        reason: reason.to_string(),
        unread,
        updated_at: "2026-02-15T12:00:00Z".to_string(),
        last_read_at: None,
        url: "https://api.github.test/notif/1".to_string(),
        subscription_url: "https://api.github.test/sub/1".to_string(),
      }),
    }
  }

  fn init_gpui_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
      gpui_component::init(cx);
      ui::init(cx);
      if cx.try_global::<AuthStateStore>().is_none() {
        cx.set_global(AuthStateStore::default());
      }
      if cx.try_global::<NotificationCountStore>().is_none() {
        cx.set_global(NotificationCountStore::default());
      }
    });
  }

  fn add_window_view_with_size<F, V>(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    build_root_view: F,
  ) -> (Entity<V>, &mut VisualTestContext)
  where
    F: FnOnce(&mut Window, &mut Context<V>) -> V,
    V: 'static + Render,
  {
    let window = cx.update(|cx| {
      let bounds = Bounds::new(
        point(px(-10000.0), px(-10000.0)),
        size(px(width), px(height)),
      );
      cx.open_window(
        WindowOptions {
          window_bounds: Some(WindowBounds::Windowed(bounds)),
          focus: false,
          show: true,
          ..Default::default()
        },
        |window, cx| cx.new(|cx| build_root_view(window, cx)),
      )
      .expect("open window")
    });

    let view = window.root(cx).expect("window root");
    let cx = VisualTestContext::from_window(*window.deref(), cx).into_mut();
    cx.run_until_parked();
    (view, cx)
  }

  fn make_available_user() -> User {
    User {
      id: "user_123".to_string(),
      name: "Joris".to_string(),
      email: "joris@example.com".to_string(),
      email_verified: true,
      image: None,
      github_login: Some("joris-gallot".to_string()),
      role: UserRole::Pro,
      subscription: UserSubscription {
        portal_url: None,
        active_subscription: None,
      },
    }
  }

  struct TestProbeView {
    labeled: GithubPullRequestRow,
    unlabeled: GithubPullRequestRow,
  }

  impl Render for TestProbeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
      let theme = cx.theme().clone();

      v_flex()
        .gap_2()
        .child(div().debug_selector(|| "labeled".to_string()).child(
          github_shared::pull_request_list_row_body(self.labeled.pr.as_ref(), &theme, false, false),
        ))
        .child(div().debug_selector(|| "unlabeled".to_string()).child(
          github_shared::pull_request_list_row_body(
            self.unlabeled.pr.as_ref(),
            &theme,
            false,
            false,
          ),
        ))
    }
  }

  #[test]
  fn repository_updated_label_uses_relative_time() {
    let now = OffsetDateTime::parse(
      "2026-02-15T18:00:00Z",
      &time::format_description::well_known::Rfc3339,
    )
    .expect("parse now");

    assert_eq!(
      repository_updated_label_at("2026-02-14T12:00:00Z", now).as_ref(),
      "Updated yesterday"
    );
    assert_eq!(
      repository_updated_label_at("2026-02-12T12:00:00Z", now).as_ref(),
      "Updated 3 days ago"
    );
  }

  #[test]
  fn github_home_greeting_uses_time_of_day_and_name() {
    let morning = OffsetDateTime::parse(
      "2026-04-02T08:00:00Z",
      &time::format_description::well_known::Rfc3339,
    )
    .expect("parse morning");
    let afternoon = OffsetDateTime::parse(
      "2026-04-02T14:00:00Z",
      &time::format_description::well_known::Rfc3339,
    )
    .expect("parse afternoon");
    let evening = OffsetDateTime::parse(
      "2026-04-02T21:00:00Z",
      &time::format_description::well_known::Rfc3339,
    )
    .expect("parse evening");

    assert_eq!(
      github_home_greeting_at(Some("Joris Gallot"), morning).as_ref(),
      "Good morning, Joris Gallot"
    );
    assert_eq!(
      github_home_greeting_at(Some("Joris Gallot"), afternoon).as_ref(),
      "Good afternoon, Joris Gallot"
    );
    assert_eq!(
      github_home_greeting_at(Some("Joris Gallot"), evening).as_ref(),
      "Good evening, Joris Gallot"
    );
  }

  #[test]
  fn github_home_date_label_formats_full_date() {
    let now = OffsetDateTime::parse(
      "2026-04-02T21:00:00Z",
      &time::format_description::well_known::Rfc3339,
    )
    .expect("parse now");

    assert_eq!(
      github_home_date_label_at(now).as_ref(),
      "Thursday, April 2, 2026"
    );
  }

  #[test]
  fn github_locked_presentation_uses_sign_in_for_unauthenticated_access() {
    let presentation = github_locked_presentation(GithubAccessState::NeedsSignIn)
      .expect("sign-in state should have locked presentation");

    assert_eq!(presentation.action, GithubLockedAction::SignIn);
    assert!(presentation.description.contains("Sign in"));
  }

  #[test]
  fn github_locked_presentation_uses_subscribe_for_subscription_gate() {
    let presentation = github_locked_presentation(GithubAccessState::NeedsSubscription)
      .expect("subscription state should have locked presentation");

    assert_eq!(presentation.action, GithubLockedAction::Subscribe);
    assert!(presentation.description.contains("Reviu Pro"));
  }

  #[test]
  fn pull_request_row_matches_title_or_repo_case_insensitive() {
    let row = make_pull_request_row("Fix Login Bug", "Acme", "Portal");
    assert!(row.matches("login"));
    assert!(row.matches("acme/portal"));
    assert!(!row.matches("missing"));
  }

  #[test]
  fn pull_request_delegate_groups_rows_by_repo_section() {
    let mut delegate = GithubPullRequestListDelegate::new();
    delegate.set_rows(vec![
      Rc::new(make_pull_request_row("Fix login", "acme", "portal")),
      Rc::new(make_pull_request_row("Improve API", "acme", "backend")),
      Rc::new(make_pull_request_row("Refactor auth", "acme", "portal")),
    ]);

    assert_eq!(delegate.matched_rows.len(), 3);
    assert_eq!(delegate.sections.len(), 2);
    assert_eq!(delegate.sections[0].repo_label.as_ref(), "acme/portal");
    assert_eq!(delegate.sections[1].repo_label.as_ref(), "acme/backend");
    assert_eq!(delegate.sections[0].rows.len(), 2);
    assert_eq!(delegate.sections[1].rows.len(), 1);
    assert_eq!(delegate.sections[0].rows[0].pr.title, "Fix login");
    assert_eq!(delegate.sections[0].rows[1].pr.title, "Refactor auth");
    assert_eq!(delegate.sections[1].rows[0].pr.title, "Improve API");
  }

  #[test]
  fn pull_request_delegate_row_at_uses_visible_row_indexes() {
    let mut delegate = GithubPullRequestListDelegate::new();
    delegate.set_rows(vec![
      Rc::new(make_pull_request_row("Fix login", "acme", "portal")),
      Rc::new(make_pull_request_row("Improve API", "acme", "backend")),
      Rc::new(make_pull_request_row("Refactor auth", "acme", "portal")),
    ]);

    assert_eq!(
      delegate
        .row_at(2)
        .expect("portal second row in visible rows")
        .pr
        .title,
      "Refactor auth"
    );
    assert_eq!(
      delegate
        .row_at(4)
        .expect("backend first row in visible rows")
        .pr
        .title,
      "Improve API"
    );
    assert!(delegate.row_at(0).is_none());
  }

  #[test]
  fn pull_request_delegate_search_keeps_matching_repo_sections() {
    let mut delegate = GithubPullRequestListDelegate::new();
    delegate.set_rows(vec![
      Rc::new(make_pull_request_row("Fix login", "acme", "portal")),
      Rc::new(make_pull_request_row("Improve API", "acme", "backend")),
      Rc::new(make_pull_request_row("Refactor auth", "acme", "portal")),
    ]);

    delegate.prepare("portal");

    assert_eq!(delegate.matched_rows.len(), 2);
    assert_eq!(delegate.sections.len(), 1);
    assert_eq!(delegate.sections[0].repo_label.as_ref(), "acme/portal");
    assert_eq!(delegate.sections[0].rows.len(), 2);
  }

  #[test]
  fn pull_request_delegate_builds_clickable_repo_headers_and_hides_collapsed_rows() {
    let mut delegate = GithubPullRequestListDelegate::new();
    delegate.set_rows(vec![
      Rc::new(make_pull_request_row("Fix login", "acme", "portal")),
      Rc::new(make_pull_request_row("Improve API", "acme", "backend")),
      Rc::new(make_pull_request_row("Refactor auth", "acme", "portal")),
    ]);

    delegate.toggle_section(0);

    assert!(delegate.section_is_collapsed(0));
    assert_eq!(delegate.sections.len(), 2);
    assert!(matches!(
      delegate.visible_rows.first(),
      Some(GithubPullRequestListEntry::SectionHeader {
        repo_label,
        collapsed: true,
        ..
      }) if repo_label.as_ref() == "acme/portal"
    ));
    assert_eq!(delegate.visible_rows.len(), 3);
    assert!(matches!(
      delegate.visible_rows.get(2),
      Some(GithubPullRequestListEntry::Item(row)) if row.pr.title == "Improve API"
    ));

    delegate.prepare("portal");

    assert!(!delegate.section_is_collapsed(0));
    assert_eq!(delegate.visible_rows.len(), 3);
    assert!(matches!(
      delegate.visible_rows.first(),
      Some(GithubPullRequestListEntry::SectionHeader {
        repo_label,
        collapsed: false,
        ..
      }) if repo_label.as_ref() == "acme/portal"
    ));
  }

  #[test]
  fn pull_request_review_status_label_round_trips() {
    for status in [
      GithubPullRequestReviewStatus::Any,
      GithubPullRequestReviewStatus::Required,
      GithubPullRequestReviewStatus::Approved,
      GithubPullRequestReviewStatus::ChangesRequested,
      GithubPullRequestReviewStatus::None,
    ] {
      assert_eq!(
        pull_request_review_status_from_label(pull_request_review_status_label(status)),
        status
      );
    }
  }

  #[test]
  fn make_pull_request_tab_states_preserves_seeded_tab_order() {
    let tabs =
      make_pull_request_tab_states(crate::github_home_tabs::seed_github_home_pull_request_tabs());

    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0].tab.name, "My Open PRs");
    assert_eq!(tabs[1].tab.name, "Need Review");
    assert!(tabs.iter().all(|tab| tab.rows.is_empty()));
    assert!(tabs.iter().all(|tab| !tab.loaded_once));
  }

  #[test]
  fn move_item_in_vec_reorders_items_before_target_index() {
    let mut items = vec!["a", "b", "c"];

    assert!(move_item_in_vec(&mut items, 2, 0));
    assert_eq!(items, vec!["c", "a", "b"]);
  }

  #[test]
  fn move_item_in_vec_supports_moving_items_to_end() {
    let mut items = vec!["a", "b", "c"];

    assert!(move_item_in_vec(&mut items, 0, 3));
    assert_eq!(items, vec!["b", "c", "a"]);
  }

  #[test]
  fn move_item_in_vec_skips_noop_reorders() {
    let mut items = vec!["a", "b", "c"];

    assert!(!move_item_in_vec(&mut items, 1, 1));
    assert!(!move_item_in_vec(&mut items, 1, 2));
    assert_eq!(items, vec!["a", "b", "c"]);
  }

  #[test]
  fn pull_request_tab_delete_confirmation_mentions_tab_name() {
    let (title, message) = pull_request_tab_delete_confirmation("Needs Review");

    assert_eq!(title.as_ref(), "Delete pull request list?");
    assert!(message.as_ref().contains("Needs Review"));
  }

  #[test]
  fn pull_request_tab_filter_tag_labels_expand_selected_filters() {
    let labels = pull_request_tab_filter_tag_labels(&GithubPullRequestSearchFilters {
      repos: vec!["acme/reviu".to_string()],
      labels: vec!["bug".to_string()],
      authors: vec!["@me".to_string()],
      assignees: vec!["alice".to_string()],
      requested_reviewers: vec!["bob".to_string()],
      review_status: GithubPullRequestReviewStatus::Required,
      include_drafts: false,
    });

    assert_eq!(
      labels,
      vec![
        "acme/reviu".to_string(),
        "bug".to_string(),
        "Author: @me".to_string(),
        "Assignee: alice".to_string(),
        "Reviewer: bob".to_string(),
        "Review required".to_string(),
        "Drafts hidden".to_string(),
      ]
    );
  }

  #[test]
  fn matching_user_filter_suggestions_offer_me_without_selected_repositories() {
    let suggestions = matching_user_filter_suggestions(&[], "", &[], true);

    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].login, "@me");
  }

  #[test]
  fn matching_user_filter_suggestions_offer_me_first_with_selected_repositories() {
    let suggestions = matching_user_filter_suggestions(
      &[GithubPullRequestFilterOptionUser {
        login: "alice".to_string(),
        avatar_url: None,
      }],
      "",
      &[],
      true,
    );

    assert_eq!(suggestions.len(), 2);
    assert_eq!(suggestions[0].login, "@me");
    assert_eq!(suggestions[1].login, "alice");
  }

  #[test]
  fn matching_user_filter_suggestions_hide_me_once_already_selected() {
    let suggestions = matching_user_filter_suggestions(&[], "", &["@me".to_string()], true);

    assert!(suggestions.is_empty());
  }

  #[test]
  fn matching_user_filter_suggestions_respect_query_when_offering_me() {
    let suggestions = matching_user_filter_suggestions(&[], "alice", &[], true);

    assert!(suggestions.is_empty());
  }

  #[gpui::test]
  fn pull_request_tab_dialog_opens_in_small_windows(cx: &mut TestAppContext) {
    let _config_guard = GithubPageTestConfigGuard::new("dialog-small-window");
    init_gpui_test(cx);
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_available_user())),
      );
    });

    let api = ApiClient::new_with_base_url("http://localhost:0".to_string());
    let mut mounted_github_page = None;
    let (_root, cx) = add_window_view_with_size(cx, 720.0, 520.0, |window, cx| {
      let github_page = cx.new(|cx| GithubPage::new_for_test(api.clone(), window, cx));
      mounted_github_page = Some(github_page.clone());
      gpui_component::Root::new(github_page, window, cx)
    });
    let github_page = mounted_github_page.expect("github page");

    github_page.update_in(cx, |this, window, cx| {
      this.open_pull_request_tab_dialog(GithubPullRequestTabDialogMode::Create, None, window, cx);
    });
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let dialog_open = github_page.update_in(cx, |_this, window, cx| window.has_active_dialog(cx));
    assert!(dialog_open);
  }

  #[gpui::test]
  fn manage_tabs_uses_the_last_review_inbox_tab_index(cx: &mut TestAppContext) {
    let _config_guard = GithubPageTestConfigGuard::new("manage-tabs-index");
    init_gpui_test(cx);
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_available_user())),
      );
    });
    let api = ApiClient::new_with_base_url("http://localhost:0".to_string());
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));

    let selected_index = github_page.update_in(cx, |this, _window, _cx| {
      this.pull_request_tabs = make_pull_request_tab_states(vec![
        GithubHomePullRequestTab {
          id: "tab-a".to_string(),
          name: "A".to_string(),
          filters: GithubPullRequestSearchFilters::default(),
        },
        GithubHomePullRequestTab {
          id: "tab-b".to_string(),
          name: "B".to_string(),
          filters: GithubPullRequestSearchFilters::default(),
        },
      ]);
      this.active_pull_request_tab_id = Some(GITHUB_HOME_MANAGE_TABS_ID.to_string());
      this.active_pull_request_inbox_tab_index()
    });

    assert_eq!(selected_index, 2);
  }

  #[gpui::test]
  fn delete_last_pull_request_tab_falls_back_to_manage_tabs(cx: &mut TestAppContext) {
    let _config_guard = GithubPageTestConfigGuard::new("delete-last-tab");
    init_gpui_test(cx);
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_available_user())),
      );
    });
    let api = ApiClient::new_with_base_url("http://localhost:0".to_string());
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));

    github_page.update_in(cx, |this, _window, cx| {
      this.pull_request_tabs = make_pull_request_tab_states(vec![GithubHomePullRequestTab {
        id: "solo-tab".to_string(),
        name: "Solo".to_string(),
        filters: GithubPullRequestSearchFilters::default(),
      }]);
      this.active_pull_request_tab_id = Some("solo-tab".to_string());
      this.delete_pull_request_tab("solo-tab", cx);
    });

    let (remaining_tabs, managing_tabs, selected_index) = github_page.read_with(cx, |this, _cx| {
      (
        this.pull_request_tabs.len(),
        this.managing_pull_request_tabs(),
        this.active_pull_request_inbox_tab_index(),
      )
    });

    assert_eq!(remaining_tabs, 0);
    assert!(managing_tabs);
    assert_eq!(selected_index, 0);
  }

  #[gpui::test]
  fn move_pull_request_tab_reorders_tabs_and_keeps_active_tab(cx: &mut TestAppContext) {
    let _config_guard = GithubPageTestConfigGuard::new("reorder-tabs");
    init_gpui_test(cx);
    let api = ApiClient::new_with_base_url("http://localhost:0".to_string());
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));

    github_page.update_in(cx, |this, _window, cx| {
      this.pull_request_tabs = make_pull_request_tab_states(vec![
        GithubHomePullRequestTab {
          id: "tab-a".to_string(),
          name: "A".to_string(),
          filters: GithubPullRequestSearchFilters::default(),
        },
        GithubHomePullRequestTab {
          id: "tab-b".to_string(),
          name: "B".to_string(),
          filters: GithubPullRequestSearchFilters::default(),
        },
        GithubHomePullRequestTab {
          id: "tab-c".to_string(),
          name: "C".to_string(),
          filters: GithubPullRequestSearchFilters::default(),
        },
      ]);
      this.active_pull_request_tab_id = Some("tab-b".to_string());
      this.move_pull_request_tab_before("tab-c", 0, cx);
    });

    let (tab_order, active_tab_id, active_index) = github_page.read_with(cx, |this, _cx| {
      (
        this
          .pull_request_tabs
          .iter()
          .map(|tab_state| tab_state.tab.id.clone())
          .collect::<Vec<_>>(),
        this.active_pull_request_tab_id.clone(),
        this.active_pull_request_tab_index(),
      )
    });
    let stored_tabs = ConfigStore::load_or_seed_github_home_pull_request_tabs();
    let stored_order = stored_tabs
      .into_iter()
      .map(|tab| tab.id)
      .collect::<Vec<_>>();

    assert_eq!(tab_order, vec!["tab-c", "tab-a", "tab-b"]);
    assert_eq!(stored_order, vec!["tab-c", "tab-a", "tab-b"]);
    assert_eq!(active_tab_id.as_deref(), Some("tab-b"));
    assert_eq!(active_index, Some(2));
  }

  #[gpui::test]
  fn move_pull_request_tab_to_end_places_tab_after_last_entry(cx: &mut TestAppContext) {
    let _config_guard = GithubPageTestConfigGuard::new("reorder-tabs-end");
    init_gpui_test(cx);
    let api = ApiClient::new_with_base_url("http://localhost:0".to_string());
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));

    github_page.update_in(cx, |this, _window, cx| {
      this.pull_request_tabs = make_pull_request_tab_states(vec![
        GithubHomePullRequestTab {
          id: "tab-a".to_string(),
          name: "A".to_string(),
          filters: GithubPullRequestSearchFilters::default(),
        },
        GithubHomePullRequestTab {
          id: "tab-b".to_string(),
          name: "B".to_string(),
          filters: GithubPullRequestSearchFilters::default(),
        },
        GithubHomePullRequestTab {
          id: "tab-c".to_string(),
          name: "C".to_string(),
          filters: GithubPullRequestSearchFilters::default(),
        },
      ]);
      this.active_pull_request_tab_id = Some(GITHUB_HOME_MANAGE_TABS_ID.to_string());
      this.move_pull_request_tab_to_end("tab-a", cx);
    });

    let (tab_order, selected_index) = github_page.read_with(cx, |this, _cx| {
      (
        this
          .pull_request_tabs
          .iter()
          .map(|tab_state| tab_state.tab.id.clone())
          .collect::<Vec<_>>(),
        this.active_pull_request_inbox_tab_index(),
      )
    });

    assert_eq!(tab_order, vec!["tab-b", "tab-c", "tab-a"]);
    assert_eq!(selected_index, 3);
  }

  #[gpui::test]
  fn github_home_layout_stacks_notifications_above_repositories(cx: &mut TestAppContext) {
    let _config_guard = GithubPageTestConfigGuard::new("layout-stacks");
    init_gpui_test(cx);
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_available_user())),
      );
    });
    let api = ApiClient::new_with_base_url("http://localhost:0".to_string());
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));

    github_page.update_in(cx, |this, _window, cx| {
      this.error = None;
      this.notifications_error = None;
      this.repositories_error = None;
      this.pull_requests.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        cx.notify();
      });
      this.notifications.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        state
          .delegate_mut()
          .set_rows(vec![Rc::new(make_notification_row(
            "Please review",
            "acme/portal",
            "mention",
            true,
          ))]);
        cx.notify();
      });
      this.repositories.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        state
          .delegate_mut()
          .set_rows(vec![Rc::new(GithubRepositoryRow {
            repository: Rc::new(GithubUserRepository {
              owner: "acme".to_string(),
              repo: "portal".to_string(),
              full_name: "acme/portal".to_string(),
              description: Some("Main app".to_string()),
              private: true,
              owner_avatar_url: Some("https://example.com/acme.png".to_string()),
              updated_at: "2026-02-15T12:30:00Z".to_string(),
            }),
            pinned: false,
          })]);
        cx.notify();
      });
      cx.notify();
    });

    let notifications_bounds = cx
      .debug_bounds(GITHUB_HOME_NOTIFICATIONS_PANEL_DEBUG_SELECTOR)
      .expect("notifications panel bounds");
    let repositories_bounds = cx
      .debug_bounds(GITHUB_HOME_REPOSITORIES_PANEL_DEBUG_SELECTOR)
      .expect("repositories panel bounds");

    assert!(notifications_bounds.origin.y < repositories_bounds.origin.y);
    assert!(
      notifications_bounds.origin.y + notifications_bounds.size.height
        <= repositories_bounds.origin.y
    );
  }

  #[gpui::test]
  fn github_home_layout_adds_clear_gap_between_columns(cx: &mut TestAppContext) {
    let _config_guard = GithubPageTestConfigGuard::new("layout-column-gap");
    init_gpui_test(cx);
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_available_user())),
      );
    });
    let api = ApiClient::new_with_base_url("http://localhost:0".to_string());
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));

    github_page.update_in(cx, |this, _window, cx| {
      this.error = None;
      this.notifications_error = None;
      this.repositories_error = None;
      this.pull_requests.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        cx.notify();
      });
      this.notifications.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        cx.notify();
      });
      this.repositories.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        cx.notify();
      });
      cx.notify();
    });

    let repositories_bounds = cx
      .debug_bounds(GITHUB_HOME_REPOSITORIES_PANEL_DEBUG_SELECTOR)
      .expect("repositories panel bounds");
    let review_inbox_bounds = cx
      .debug_bounds(GITHUB_HOME_REVIEW_INBOX_PANEL_DEBUG_SELECTOR)
      .expect("review inbox panel bounds");

    let column_gap = review_inbox_bounds.origin.x
      - (repositories_bounds.origin.x + repositories_bounds.size.width);
    assert!(column_gap >= px(20.0), "column gap: {column_gap:?}");
  }

  #[gpui::test]
  fn github_home_layout_places_unread_badge_next_to_notifications_title(cx: &mut TestAppContext) {
    let _config_guard = GithubPageTestConfigGuard::new("layout-unread-badge");
    init_gpui_test(cx);
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_available_user())),
      );
    });
    let api = ApiClient::new_with_base_url("http://localhost:0".to_string());
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));

    github_page.update_in(cx, |this, _window, cx| {
      this.notifications_error = None;
      this.repositories_error = None;
      this.pull_requests.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        cx.notify();
      });
      this.notifications.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        state
          .delegate_mut()
          .set_rows(vec![Rc::new(make_notification_row(
            "Please review",
            "acme/portal",
            "mention",
            true,
          ))]);
        cx.notify();
      });
      this.repositories.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        state
          .delegate_mut()
          .set_rows(vec![Rc::new(GithubRepositoryRow {
            repository: Rc::new(GithubUserRepository {
              owner: "acme".to_string(),
              repo: "portal".to_string(),
              full_name: "acme/portal".to_string(),
              description: Some("Main app".to_string()),
              private: true,
              owner_avatar_url: Some("https://example.com/acme.png".to_string()),
              updated_at: "2026-02-15T12:30:00Z".to_string(),
            }),
            pinned: false,
          })]);
        cx.notify();
      });
      cx.notify();
    });

    let unread_badge_bounds = cx
      .debug_bounds(GITHUB_HOME_NOTIFICATIONS_UNREAD_BADGE_DEBUG_SELECTOR)
      .expect("unread badge bounds");
    let count_badge_bounds = cx
      .debug_bounds(GITHUB_HOME_NOTIFICATIONS_COUNT_BADGE_DEBUG_SELECTOR)
      .expect("count badge bounds");

    assert!(unread_badge_bounds.origin.x < count_badge_bounds.origin.x);
  }

  #[test]
  fn github_notification_count_label_hides_zero_count() {
    assert_eq!(github_notification_count_label(0), None);
    assert_eq!(github_notification_count_label(7), Some("7".to_string()));
  }

  #[test]
  fn notification_row_matches_title_repo_or_reason() {
    let row = make_notification_row("Review request", "acme/portal", "mention", true);
    assert!(row.matches("review"));
    assert!(row.matches("ACME/PORTAL"));
    assert!(row.matches("MENTION"));
    assert!(!row.matches("missing"));
  }

  #[test]
  fn notification_delegate_prepare_filters_and_counts_unread() {
    let mut delegate =
      GithubNotificationListDelegate::new(ApiClient::new_with_base_url("http://unused"));
    delegate.set_rows(vec![
      Rc::new(make_notification_row(
        "Review request",
        "acme/portal",
        "mention",
        true,
      )),
      Rc::new(make_notification_row(
        "Dependency update",
        "acme/backend",
        "subscribed",
        false,
      )),
    ]);

    assert_eq!(delegate.unread_count(), 1);
    assert_eq!(delegate.matched_rows.len(), 2);
    assert_eq!(delegate.sections.len(), 2);
    assert_eq!(delegate.sections[0].repo_label.as_ref(), "acme/portal");
    assert_eq!(delegate.sections[1].repo_label.as_ref(), "acme/backend");

    delegate.prepare("backend");
    assert_eq!(delegate.matched_rows.len(), 1);
    assert_eq!(delegate.sections.len(), 1);
    assert_eq!(delegate.sections[0].repo_label.as_ref(), "acme/backend");
  }

  #[test]
  fn notification_delegate_builds_clickable_repo_headers_and_hides_collapsed_rows() {
    let mut delegate =
      GithubNotificationListDelegate::new(ApiClient::new_with_base_url("http://unused"));
    delegate.set_rows(vec![
      Rc::new(make_notification_row(
        "Review request",
        "acme/portal",
        "mention",
        true,
      )),
      Rc::new(make_notification_row(
        "Dependency update",
        "acme/backend",
        "subscribed",
        false,
      )),
    ]);

    delegate.toggle_section(0);

    assert!(delegate.section_is_collapsed(0));
    assert_eq!(delegate.sections.len(), 2);
    assert!(matches!(
      delegate.visible_rows.first(),
      Some(GithubNotificationListEntry::SectionHeader {
        repo_label,
        collapsed: true,
        ..
      }) if repo_label.as_ref() == "acme/portal"
    ));
    assert_eq!(delegate.visible_rows.len(), 3);
    assert!(matches!(
      delegate.visible_rows.get(2),
      Some(GithubNotificationListEntry::Item(row))
        if row.notification.subject.title == "Dependency update"
    ));

    delegate.prepare("portal");

    assert!(!delegate.section_is_collapsed(0));
    assert_eq!(delegate.visible_rows.len(), 2);
    assert!(matches!(
      delegate.visible_rows.first(),
      Some(GithubNotificationListEntry::SectionHeader {
        repo_label,
        collapsed: false,
        ..
      }) if repo_label.as_ref() == "acme/portal"
    ));
  }

  #[gpui::test]
  fn pull_request_delegate_rows_use_less_height_without_labels(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let labeled =
      make_pull_request_row_with_labels("Labeled pull request", "acme", "portal", &["bug"]);
    let unlabeled =
      make_pull_request_row_with_labels("Unlabeled pull request", "acme", "portal", &[]);
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

    assert!(labeled_height > unlabeled_height);
  }

  #[gpui::test]
  fn refresh_pull_requests_sets_unauthorized_errors(cx: &mut TestAppContext) {
    let _config_guard = GithubPageTestConfigGuard::new("refresh-unauthorized");
    init_gpui_test(cx);
    let api = ApiClient::new_with_base_url("http://localhost:0".to_string());
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));

    github_page.update_in(cx, |this, _window, cx| {
      let error: SharedString = "unauthorized".into();
      this.error = Some(error.clone());
      this.notifications_error = Some(error.clone());
      this.repositories_error = Some(error);
      this.pull_requests.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        cx.notify();
      });
      this.notifications.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        cx.notify();
      });
      this.repositories.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        cx.notify();
      });
      cx.notify();
    });

    let (
      error,
      notifications_error,
      repositories_error,
      pr_count,
      notifications_count,
      repositories_count,
      pr_loading,
      notifications_loading,
      repositories_loading,
    ) = github_page.read_with(cx, |this, cx| {
      let pull_requests = this.pull_requests.read(cx);
      let notifications = this.notifications.read(cx);
      let repositories = this.repositories.read(cx);
      (
        this.error.clone(),
        this.notifications_error.clone(),
        this.repositories_error.clone(),
        pull_requests.delegate().matched_rows.len(),
        notifications.delegate().matched_rows.len(),
        repositories.delegate().matched_rows.len(),
        pull_requests.delegate().loading,
        notifications.delegate().loading,
        repositories.delegate().loading,
      )
    });

    assert!(
      error
        .as_ref()
        .is_some_and(|value| github_shared::is_unauthorized_error_message(value.as_ref()))
    );
    assert!(
      notifications_error
        .as_ref()
        .is_some_and(|value| github_shared::is_unauthorized_error_message(value.as_ref()))
    );
    assert!(
      repositories_error
        .as_ref()
        .is_some_and(|value| github_shared::is_unauthorized_error_message(value.as_ref()))
    );
    assert_eq!(pr_count, 0);
    assert_eq!(notifications_count, 0);
    assert_eq!(repositories_count, 0);
    assert!(!pr_loading);
    assert!(!notifications_loading);
    assert!(!repositories_loading);
  }

  #[gpui::test]
  fn refresh_pull_requests_populates_pull_requests_and_notifications_on_success(
    cx: &mut TestAppContext,
  ) {
    let _config_guard = GithubPageTestConfigGuard::new("refresh-success");
    init_gpui_test(cx);
    let api = ApiClient::new_with_base_url("http://localhost:0".to_string());
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));

    github_page.update_in(cx, |this, _window, cx| {
      this.pull_request_tabs = make_pull_request_tab_states(vec![
        GithubHomePullRequestTab {
          id: "my-open".to_string(),
          name: "My Open PRs".to_string(),
          filters: GithubPullRequestSearchFilters {
            authors: vec!["@me".to_string()],
            ..GithubPullRequestSearchFilters::default()
          },
        },
        GithubHomePullRequestTab {
          id: "need-review".to_string(),
          name: "Need Review".to_string(),
          filters: GithubPullRequestSearchFilters {
            requested_reviewers: vec!["@me".to_string()],
            ..GithubPullRequestSearchFilters::default()
          },
        },
      ]);
      this.active_pull_request_tab_id = Some("my-open".to_string());
      this.pull_request_tabs[0].rows = vec![Rc::new(GithubPullRequestRow {
        pr: Rc::new(GithubPullRequest {
          number: 42,
          title: "Fix login".to_string(),
          state: GithubPullRequestState::Open,
          created_at: String::new(),
          closed_at: None,
          merged_at: None,
          draft: false,
          updated_at: "2026-02-15T12:00:00Z".to_string(),
          comments_count: 0,
          labels: vec![GithubPullRequestLabel {
            name: "bug".to_string(),
            color: Some("f29513".to_string()),
          }],
          repository: GithubRepository {
            owner: "acme".to_string(),
            repo: "portal".to_string(),
          },
          author: GithubPullRequestAuthor::default(),
        }),
      })];
      this.pull_request_tabs[0].loaded_once = true;
      this.pull_request_tabs[1].rows = vec![Rc::new(GithubPullRequestRow {
        pr: Rc::new(GithubPullRequest {
          number: 55,
          title: "Review billing flow".to_string(),
          state: GithubPullRequestState::Open,
          created_at: String::new(),
          closed_at: None,
          merged_at: None,
          draft: false,
          updated_at: "2026-02-15T12:05:00Z".to_string(),
          comments_count: 0,
          labels: vec![GithubPullRequestLabel {
            name: "review".to_string(),
            color: Some("5319e7".to_string()),
          }],
          repository: GithubRepository {
            owner: "acme".to_string(),
            repo: "payments".to_string(),
          },
          author: GithubPullRequestAuthor::default(),
        }),
      })];
      this.pull_request_tabs[1].loaded_once = true;
      this.error = None;
      this.apply_active_pull_request_rows(cx);
      this.pull_requests.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        cx.notify();
      });

      this.notifications_error = None;
      this.notifications.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        state
          .delegate_mut()
          .set_rows(vec![Rc::new(GithubNotificationRow {
            notification: Rc::new(GithubNotification {
              id: "n1".to_string(),
              repository: GithubNotificationRepository {
                name: "portal".to_string(),
                full_name: "acme/portal".to_string(),
                owner: None,
              },
              subject: GithubNotificationSubject {
                title: "Please review".to_string(),
                subject_type: "PullRequest".to_string(),
                url: None,
                latest_comment_url: None,
              },
              reason: "mention".to_string(),
              unread: true,
              updated_at: "2026-02-15T12:10:00Z".to_string(),
              last_read_at: None,
              url: "https://api.github.test/notif/1".to_string(),
              subscription_url: "https://api.github.test/sub/1".to_string(),
            }),
          })]);
        cx.notify();
      });

      this.repositories_error = None;
      this.repositories.update(cx, |state, cx| {
        state.delegate_mut().loading = false;
        state
          .delegate_mut()
          .set_rows(vec![Rc::new(GithubRepositoryRow {
            repository: Rc::new(GithubUserRepository {
              owner: "acme".to_string(),
              repo: "portal".to_string(),
              full_name: "acme/portal".to_string(),
              description: Some("Main app".to_string()),
              private: true,
              owner_avatar_url: Some("https://example.com/acme.png".to_string()),
              updated_at: "2026-02-15T12:30:00Z".to_string(),
            }),
            pinned: false,
          })]);
        cx.notify();
      });

      cx.notify();
    });

    let (
      error,
      notifications_error,
      repositories_error,
      pr_titles,
      notification_titles,
      unread_count,
      repositories_count,
    ) = github_page.read_with(cx, |this, cx| {
      let pull_requests = this.pull_requests.read(cx);
      let notifications = this.notifications.read(cx);
      let repositories = this.repositories.read(cx);
      (
        this.error.clone(),
        this.notifications_error.clone(),
        this.repositories_error.clone(),
        pull_requests
          .delegate()
          .matched_rows
          .iter()
          .map(|row| row.pr.title.clone())
          .collect::<Vec<_>>(),
        notifications
          .delegate()
          .matched_rows
          .iter()
          .map(|row| row.notification.subject.title.clone())
          .collect::<Vec<_>>(),
        notifications.delegate().unread_count(),
        repositories.delegate().matched_rows.len(),
      )
    });

    assert!(error.is_none());
    assert!(notifications_error.is_none());
    assert!(repositories_error.is_none());
    assert_eq!(pr_titles, vec!["Fix login".to_string()]);
    assert_eq!(notification_titles, vec!["Please review".to_string()]);
    assert_eq!(unread_count, 1);
    assert_eq!(repositories_count, 1);

    github_page.update_in(cx, |this, window, cx| {
      this.set_active_pull_request_tab(1, window, cx);
    });
    let need_review_titles = github_page.read_with(cx, |this, cx| {
      this
        .pull_requests
        .read(cx)
        .delegate()
        .matched_rows
        .iter()
        .map(|row| row.pr.title.clone())
        .collect::<Vec<_>>()
    });
    assert_eq!(need_review_titles, vec!["Review billing flow".to_string()]);
  }

  #[test]
  fn github_home_refresh_helper_reports_loading_when_any_section_is_refreshing() {
    assert!(!github_home_refresh_in_progress(false, false, false));
    assert!(github_home_refresh_in_progress(true, false, false));
    assert!(github_home_refresh_in_progress(false, true, false));
    assert!(github_home_refresh_in_progress(false, false, true));
  }
}
