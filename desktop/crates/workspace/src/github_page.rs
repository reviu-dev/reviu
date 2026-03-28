use std::{collections::HashSet, rc::Rc, sync::Arc};

use gpui::{
  AnyElement, App, Context, Entity, FocusHandle, Focusable, MouseButton, ParentElement, Render,
  SharedString, Styled, Subscription, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _, StyledExt as _,
  avatar::Avatar,
  button::{Button, ButtonVariants as _},
  h_flex,
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  scroll::ScrollableElement,
  tab::{Tab, TabBar},
  tag::Tag,
  v_flex,
};
use sentry::protocol::{Map, Value};
use smol::unblock;

use crate::dock_badge::set_dock_badge;
use crate::notification_count::NotificationCountStore;
#[cfg(test)]
use time::OffsetDateTime;
use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteGithubRepoTab, CommandPaletteHandler, CommandPalettePage,
  DETAILS_PAGE_CONTAINER_MAX_WIDTH, PAGE_HEADER_HEIGHT, SelectableRowStyle, StatusTag,
  StatusThemeExt, UiIconName, WindowExt, selectable_list_item,
};

#[cfg(test)]
use crate::date_format::format_relative_time_at;
use crate::{
  AuthCallbackTarget, ShowCommandPalette,
  api::{ApiClient, GithubNotification, GithubPullRequest, GithubUserRepository},
  auth_state::{AuthStateStore, GithubAccessState},
  billing_page::{ReviuProCheckoutCta, reviu_pro_checkout_button},
  config::ConfigStore,
  date_format::format_relative_time,
  github_navigation::{open_pr_target, open_repo_target},
  github_pr_details_page::GithubPrDetailsPageHandle,
  github_shared,
  navigation::NavigationHistory,
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

fn update_selected_index<D: ListDelegate>(
  selected_index: &mut Option<IndexPath>,
  ix: Option<IndexPath>,
  cx: &mut Context<ListState<D>>,
) {
  *selected_index = ix;
  cx.notify();
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubPullRequestTab {
  MyOpen,
  NeedReview,
  Notifications,
}

impl GithubPullRequestTab {
  fn as_index(&self) -> usize {
    match self {
      GithubPullRequestTab::MyOpen => 0,
      GithubPullRequestTab::NeedReview => 1,
      GithubPullRequestTab::Notifications => 2,
    }
  }

  fn from_index(index: usize) -> Option<Self> {
    match index {
      0 => Some(GithubPullRequestTab::MyOpen),
      1 => Some(GithubPullRequestTab::NeedReview),
      2 => Some(GithubPullRequestTab::Notifications),
      _ => None,
    }
  }

  fn shows_pull_request_author(&self) -> bool {
    !matches!(self, GithubPullRequestTab::MyOpen)
  }
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
      description: "Upgrade to Reviu Pro for $19/month to unlock GitHub notifications, repository browsing, pull request reviews, issues, and branch-to-PR shortcuts.",
      action: GithubLockedAction::Subscribe,
    }),
  }
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
  selected_index: Option<IndexPath>,
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
      selected_index: Some(IndexPath::default()),
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
  }

  fn row_at(&self, ix: IndexPath) -> Option<Rc<GithubNotificationRow>> {
    self
      .sections
      .get(ix.section)
      .and_then(|section| section.rows.get(ix.row))
      .cloned()
  }

  fn unread_count(&self) -> usize {
    self
      .all_rows
      .iter()
      .filter(|row| row.notification.unread)
      .count()
  }
}

impl ListDelegate for GithubNotificationListDelegate {
  type Item = ListItem;

  fn sections_count(&self, _cx: &App) -> usize {
    self.sections.len()
  }

  fn items_count(&self, section: usize, _cx: &App) -> usize {
    self
      .sections
      .get(section)
      .map_or(0, |section| section.rows.len())
  }

  fn render_section_header(
    &mut self,
    section: usize,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<impl IntoElement> {
    let section = self.sections.get(section)?;
    Some(github_shared::repo_section_header(&section.repo_label, cx))
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = cx.theme().clone();
    let base_item = list_base_item(ix, self.selected_index, &theme);
    let row = self.row_at(ix)?;
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
                            let _ = unblock(move || api.mark_notification_done(&thread_id)).await;
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
      .child("No notifications")
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

struct GithubPullRequestListDelegate {
  all_rows: Vec<Rc<GithubPullRequestRow>>,
  matched_rows: Vec<Rc<GithubPullRequestRow>>,
  sections: Vec<GithubPullRequestSection>,
  selected_index: Option<IndexPath>,
  query: SharedString,
  show_author: bool,
  loading: bool,
}

impl GithubPullRequestListDelegate {
  fn new() -> Self {
    Self {
      all_rows: Vec::new(),
      matched_rows: Vec::new(),
      sections: Vec::new(),
      selected_index: Some(IndexPath::default()),
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
  }

  fn set_rows(&mut self, rows: Vec<Rc<GithubPullRequestRow>>) {
    self.all_rows = rows;
    self.prepare(self.query.clone());
  }

  fn row_at(&self, ix: IndexPath) -> Option<Rc<GithubPullRequestRow>> {
    self
      .sections
      .get(ix.section)
      .and_then(|section| section.rows.get(ix.row))
      .cloned()
  }
}

impl ListDelegate for GithubPullRequestListDelegate {
  type Item = ListItem;

  fn sections_count(&self, _cx: &App) -> usize {
    self.sections.len()
  }

  fn items_count(&self, section: usize, _cx: &App) -> usize {
    self
      .sections
      .get(section)
      .map_or(0, |section| section.rows.len())
  }

  fn render_section_header(
    &mut self,
    section: usize,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<impl IntoElement> {
    let section = self.sections.get(section)?;
    Some(github_shared::repo_section_header(&section.repo_label, cx))
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let row = self.row_at(ix)?;
    let theme = cx.theme().clone();
    let base_item = list_base_item(ix, self.selected_index, &theme);

    Some(
      base_item
        .px_2()
        .py_2()
        .child(github_shared::pull_request_list_row_body(
          row.pr.as_ref(),
          &theme,
          false,
          self.show_author,
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

pub struct GithubPage {
  focus_handle: FocusHandle,
  api: ApiClient,
  repositories: Entity<ListState<GithubRepositoryListDelegate>>,
  notifications: Entity<ListState<GithubNotificationListDelegate>>,
  pull_requests: Entity<ListState<GithubPullRequestListDelegate>>,
  my_open_pull_request_rows: Vec<Rc<GithubPullRequestRow>>,
  need_review_pull_request_rows: Vec<Rc<GithubPullRequestRow>>,
  active_pull_request_tab: GithubPullRequestTab,
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
      ListState::new(GithubNotificationListDelegate::new(api.clone()), window, cx).searchable(true)
    });
    let pull_requests = cx
      .new(|cx| ListState::new(GithubPullRequestListDelegate::new(), window, cx).searchable(true));

    let view = Self {
      focus_handle: cx.focus_handle(),
      api,
      repositories,
      notifications,
      pull_requests,
      my_open_pull_request_rows: Vec::new(),
      need_review_pull_request_rows: Vec::new(),
      active_pull_request_tab: GithubPullRequestTab::MyOpen,
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
    match self.active_pull_request_tab {
      GithubPullRequestTab::Notifications => {
        self.notifications.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      _ => {
        self.pull_requests.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
    }
  }

  fn subscribe_to_list(&mut self, cx: &mut Context<Self>) {
    let pull_requests_subscription = cx.subscribe(
      &self.pull_requests,
      move |_this, state, event: &ListEvent, cx| {
        if let ListEvent::Confirm(ix) = event {
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
      move |_this, state, event: &ListEvent, cx| {
        if let ListEvent::Confirm(ix) = event {
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

  fn active_pull_request_rows(&self) -> Vec<Rc<GithubPullRequestRow>> {
    match self.active_pull_request_tab {
      GithubPullRequestTab::MyOpen => self.my_open_pull_request_rows.clone(),
      GithubPullRequestTab::NeedReview => self.need_review_pull_request_rows.clone(),
      GithubPullRequestTab::Notifications => self.my_open_pull_request_rows.clone(),
    }
  }

  fn apply_active_pull_request_rows(&mut self, cx: &mut Context<Self>) {
    let rows = self.active_pull_request_rows();
    let show_author = self.active_pull_request_tab.shows_pull_request_author();
    self.pull_requests.update(cx, |state, cx| {
      state.delegate_mut().show_author = show_author;
      state.delegate_mut().set_rows(rows);
      cx.notify();
    });
  }

  fn set_active_pull_request_tab(
    &mut self,
    index: usize,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(tab) = GithubPullRequestTab::from_index(index) else {
      return;
    };
    if self.active_pull_request_tab == tab {
      return;
    }

    self.active_pull_request_tab = tab;
    if !matches!(tab, GithubPullRequestTab::Notifications) {
      self.apply_active_pull_request_rows(cx);
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
    let api = self.api.clone();
    self.add_github_breadcrumb("Refresh pull requests started", Map::new());

    self.error = None;
    self.pull_requests.update(cx, |state, cx| {
      state.delegate_mut().loading = true;
      cx.notify();
    });

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        let my_open_pull_requests = api.fetch_latest_pull_requests()?;
        let need_review_pull_requests = api.fetch_need_review_pull_requests()?;
        Ok::<_, anyhow::Error>((my_open_pull_requests, need_review_pull_requests))
      })
      .await
      .map_err(|error| error.to_string());

      let _ = this.update(cx, |this, cx| {
        let (my_open_rows, need_review_rows, error): (
          Vec<Rc<GithubPullRequestRow>>,
          Vec<Rc<GithubPullRequestRow>>,
          Option<SharedString>,
        ) = match result {
          Ok((my_open_pull_requests, need_review_pull_requests)) => (
            my_open_pull_requests
              .into_iter()
              .map(|pr| Rc::new(GithubPullRequestRow { pr: Rc::new(pr) }))
              .collect::<Vec<_>>(),
            need_review_pull_requests
              .into_iter()
              .map(|pr| Rc::new(GithubPullRequestRow { pr: Rc::new(pr) }))
              .collect::<Vec<_>>(),
            None,
          ),
          Err(error) => (Vec::new(), Vec::new(), Some(error.into())),
        };

        match error.as_ref() {
          Some(error) => {
            let data = Map::new();
            this.add_github_breadcrumb("Refresh pull requests failed", data.clone());
            this.record_github_error("github.pull_requests.refresh", error.as_ref(), data);
          }
          None => {
            let mut data = Map::new();
            data.insert("my_open_count".into(), my_open_rows.len().into());
            data.insert("need_review_count".into(), need_review_rows.len().into());
            this.add_github_breadcrumb("Refresh pull requests succeeded", data);
          }
        }

        this.error = error;
        this.my_open_pull_request_rows = my_open_rows;
        this.need_review_pull_request_rows = need_review_rows;

        this.pull_requests.update(cx, |state, cx| {
          state.delegate_mut().loading = false;
          cx.notify();
        });
        this.apply_active_pull_request_rows(cx);

        cx.notify();
      });
    });

    self.load_task = Some(task);
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

  fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    div()
      .h(px(PAGE_HEADER_HEIGHT))
      .max_h(px(PAGE_HEADER_HEIGHT))
      .w_full()
      .px_3()
      .flex()
      .items_center()
      .justify_start()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(div().text_sm().text_color(theme.foreground).child("GitHub"))
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
      .child(self.render_header(cx))
      .child(
        div().w_full().h_full().min_h_0().overflow_y_scrollbar().child(
          div().flex().flex_col()
            .w_full()
            .max_w(px(DETAILS_PAGE_CONTAINER_MAX_WIDTH))
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
                          div().flex()
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

    let pull_requests_search_placeholder = match self.active_pull_request_tab {
      GithubPullRequestTab::MyOpen => "Search my open pull requests...",
      GithubPullRequestTab::NeedReview => "Search pull requests needing review...",
      GithubPullRequestTab::Notifications => "Search pull requests...",
    };

    let pull_requests_list = List::new(&self.pull_requests)
      .search_placeholder(pull_requests_search_placeholder)
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .flex_1()
      .min_w(px(0.0))
      .min_h_0()
      .p(px(8.));
    let notifications_list = List::new(&self.notifications)
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
    let my_open_count = self.my_open_pull_request_rows.len();
    let need_review_count = self.need_review_pull_request_rows.len();
    let unread_count = self.notifications.read(cx).delegate().unread_count();

    let pr_tabs = TabBar::new("github-home-pr-tabs")
      .w_full()
      .segmented()
      .selected_index(self.active_pull_request_tab.as_index())
      .on_click(cx.listener(|this, ix: &usize, window, cx| {
        this.set_active_pull_request_tab(*ix, window, cx);
      }))
      .child(
        Tab::new().child(
          h_flex().items_center().gap_2().child("My Open PRs").child(
            Tag::secondary()
              .small()
              .rounded_full()
              .child(my_open_count.to_string()),
          ),
        ),
      )
      .child(
        Tab::new().child(
          h_flex().items_center().gap_2().child("Need Review").child(
            Tag::secondary()
              .small()
              .rounded_full()
              .child(need_review_count.to_string()),
          ),
        ),
      )
      .child(
        Tab::new().child(h_flex().items_center().gap_2().child("Notifications").when(
          unread_count > 0,
          |this| {
            this.child(
              Tag::danger()
                .small()
                .rounded_full()
                .child(unread_count.to_string()),
            )
          },
        )),
      );

    let repositories_count = self.repositories.read(cx).delegate().matched_rows.len();
    let repositories_panel = v_flex()
      .gap_2()
      .w(px(560.0))
      .h_full()
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

    let active_right_error = match self.active_pull_request_tab {
      GithubPullRequestTab::Notifications => self.notifications_error.clone(),
      GithubPullRequestTab::MyOpen | GithubPullRequestTab::NeedReview => self.error.clone(),
    };
    let active_right_list = match self.active_pull_request_tab {
      GithubPullRequestTab::Notifications => notifications_list.into_any_element(),
      GithubPullRequestTab::MyOpen | GithubPullRequestTab::NeedReview => {
        pull_requests_list.into_any_element()
      }
    };

    let right_panel = v_flex()
      .gap_2()
      .flex_1()
      .min_w_0()
      .h_full()
      .min_h_0()
      .child("Review Inbox")
      .child(pr_tabs)
      .when_some(active_right_error, |this, error| {
        this.child(div().text_sm().text_color(theme.status_red()).child(error))
      })
      .child(active_right_list);

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GithubPage::show_command_palette_action))
      .child(self.render_header(cx))
      .child(
        v_flex()
          .w_full()
          .mx_auto()
          .h_full()
          .min_h_0()
          .gap_3()
          .p_4()
          .child(
            h_flex()
              .h_full()
              .gap_3()
              .min_h_0()
              .items_start()
              .child(repositories_panel)
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
    GithubRepository, GithubUserRepository,
  };
  use gpui::TestAppContext;
  use std::rc::Rc;

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
      if cx.try_global::<AuthStateStore>().is_none() {
        cx.set_global(AuthStateStore::default());
      }
      if cx.try_global::<NotificationCountStore>().is_none() {
        cx.set_global(NotificationCountStore::default());
      }
    });
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
    assert!(presentation.description.contains("$19/month"));
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
  fn pull_request_delegate_row_at_uses_section_and_row_indexes() {
    let mut delegate = GithubPullRequestListDelegate::new();
    delegate.set_rows(vec![
      Rc::new(make_pull_request_row("Fix login", "acme", "portal")),
      Rc::new(make_pull_request_row("Improve API", "acme", "backend")),
      Rc::new(make_pull_request_row("Refactor auth", "acme", "portal")),
    ]);

    assert_eq!(
      delegate
        .row_at(IndexPath::new(1).section(0))
        .expect("portal second row")
        .pr
        .title,
      "Refactor auth"
    );
    assert_eq!(
      delegate
        .row_at(IndexPath::new(0).section(1))
        .expect("backend first row")
        .pr
        .title,
      "Improve API"
    );
    assert!(delegate.row_at(IndexPath::new(2).section(0)).is_none());
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
  fn pull_request_tab_author_visibility_matches_home_context() {
    assert!(!GithubPullRequestTab::MyOpen.shows_pull_request_author());
    assert!(GithubPullRequestTab::NeedReview.shows_pull_request_author());
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

  #[gpui::test]
  fn pull_request_delegate_rows_keep_a_stable_height(cx: &mut TestAppContext) {
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

    assert_eq!(labeled_height, unlabeled_height);
  }

  #[gpui::test]
  fn refresh_pull_requests_sets_unauthorized_errors(cx: &mut TestAppContext) {
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
    init_gpui_test(cx);
    let api = ApiClient::new_with_base_url("http://localhost:0".to_string());
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));

    github_page.update_in(cx, |this, _window, cx| {
      this.my_open_pull_request_rows = vec![Rc::new(GithubPullRequestRow {
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
          }],
          repository: GithubRepository {
            owner: "acme".to_string(),
            repo: "portal".to_string(),
          },
          author: GithubPullRequestAuthor::default(),
        }),
      })];
      this.need_review_pull_request_rows = vec![Rc::new(GithubPullRequestRow {
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
          }],
          repository: GithubRepository {
            owner: "acme".to_string(),
            repo: "payments".to_string(),
          },
          author: GithubPullRequestAuthor::default(),
        }),
      })];
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

    github_page.update_in(cx, |this, window, cx| {
      this.set_active_pull_request_tab(2, window, cx);
    });
    let notifications_tab_titles = github_page.read_with(cx, |this, cx| {
      this
        .notifications
        .read(cx)
        .delegate()
        .matched_rows
        .iter()
        .map(|row| row.notification.subject.title.clone())
        .collect::<Vec<_>>()
    });
    assert_eq!(notifications_tab_titles, vec!["Please review".to_string()]);
  }
}
