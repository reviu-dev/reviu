use std::{rc::Rc, sync::Arc};

use gpui::{
  App, Context, Entity, FocusHandle, Focusable, ParentElement, Render, SharedString, Styled,
  Subscription, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _, h_flex,
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  tag::Tag,
  v_flex,
};
use sentry::protocol::{Map, Value};
use smol::unblock;
use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, HEADER_HEIGHT, StatusThemeExt, WindowExt,
};

use crate::{
  AuthCallbackTarget, ShowCommandPalette,
  api::{ApiClient, GithubNotification, GithubPullRequest, GithubPullRequestStatus},
  auth_state::{AuthState, AuthStateStore},
  date_format::format_compact_datetime,
  github_pr_details_page::GithubPrDetailsPageHandle,
  sentry_context,
  workspace::{WorkspaceApi, WorkspacePage, WorkspaceRoute},
};
use ui::{UserMenuConfig, UserMenuPage, UserMenuState, UserMenuUser, user_menu};

const DEFAULT_ORG: &str = "joris-gallot";
const DEFAULT_REPO: &str = "guit";

impl GithubPullRequestStatus {
  pub fn tag(&self, theme: &gpui_component::Theme) -> Tag {
    match self {
      GithubPullRequestStatus::Open => Tag::success().small().rounded_full().child("Open"),
      GithubPullRequestStatus::Closed => Tag::custom(
        theme.status_red(),
        theme.primary_foreground,
        theme.status_red(),
      )
      .small()
      .rounded_full()
      .child("Closed"),
      GithubPullRequestStatus::Merged => Tag::custom(
        theme.status_violet(),
        theme.primary_foreground,
        theme.status_violet(),
      )
      .small()
      .rounded_full()
      .child("Merged"),
      GithubPullRequestStatus::Draft => Tag::custom(
        theme.status_gray(),
        theme.primary_foreground,
        theme.status_gray(),
      )
      .small()
      .rounded_full()
      .child("Draft"),
    }
  }
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

#[derive(Clone, Debug)]
struct GithubPullRequestRow {
  pr: Rc<GithubPullRequest>,
  owner: SharedString,
  repo: SharedString,
}

impl GithubPullRequestRow {
  fn matches(&self, query: &str) -> bool {
    if query.is_empty() {
      return true;
    }

    let q = query.to_lowercase();
    self.pr.title.to_lowercase().contains(&q)
      || format!("{}/{}", self.owner, self.repo)
        .to_lowercase()
        .contains(&q)
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

struct GithubNotificationListDelegate {
  all_rows: Vec<Rc<GithubNotificationRow>>,
  matched_rows: Vec<Rc<GithubNotificationRow>>,
  selected_index: Option<IndexPath>,
  query: SharedString,
  loading: bool,
}

impl GithubNotificationListDelegate {
  fn new() -> Self {
    Self {
      all_rows: Vec::new(),
      matched_rows: Vec::new(),
      selected_index: Some(IndexPath::default()),
      query: "".into(),
      loading: false,
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
    let notification = &row.notification;
    let updated_at = format_compact_datetime(&notification.updated_at);
    let repo_name = notification.repository.full_name.clone();
    let subject = notification.subject.title.clone();
    let reason_tag = Tag::secondary()
      .small()
      .rounded_full()
      .child(notification.reason.clone());

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
                this.child(div().size(px(6.)).rounded_full().bg(theme.status_violet()))
              }),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(repo_name)
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
  selected_index: Option<IndexPath>,
  query: SharedString,
  loading: bool,
}

impl GithubPullRequestListDelegate {
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

    let rows: Vec<Rc<GithubPullRequestRow>> = self
      .all_rows
      .iter()
      .filter(|row| row.matches(q))
      .cloned()
      .collect();

    self.matched_rows = rows;
  }

  fn set_rows(&mut self, rows: Vec<Rc<GithubPullRequestRow>>) {
    self.all_rows = rows;
    self.prepare(self.query.clone());
  }
}

impl ListDelegate for GithubPullRequestListDelegate {
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
    let repo_name = format!("{}/{}", row.owner, row.repo);

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
              .child(repo_name)
              .child(format!("Updated {}", updated_at)),
          )
          .when(!row.pr.labels.is_empty(), |this| {
            this.child(h_flex().gap_1().flex_wrap().children(label_tags))
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

pub struct GithubPage {
  focus_handle: FocusHandle,
  api: ApiClient,
  notifications: Entity<ListState<GithubNotificationListDelegate>>,
  pull_requests: Entity<ListState<GithubPullRequestListDelegate>>,
  load_task: Option<Task<()>>,
  notifications_task: Option<Task<()>>,
  notifications_error: Option<SharedString>,
  error: Option<SharedString>,
  focus_on_next_render: bool,
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
      this.focus_on_next_render = true;
      this.refresh_pull_requests(cx);
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
    if error.to_ascii_lowercase().contains("unauthorized") {
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
    let notifications = cx
      .new(|cx| ListState::new(GithubNotificationListDelegate::new(), window, cx).searchable(true));
    let pull_requests = cx
      .new(|cx| ListState::new(GithubPullRequestListDelegate::new(), window, cx).searchable(true));

    let view = Self {
      focus_handle: cx.focus_handle(),
      api,
      notifications,
      pull_requests,
      load_task: None,
      notifications_task: None,
      notifications_error: None,
      error: None,
      focus_on_next_render: true,
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
    let subscription = cx.subscribe(
      &self.pull_requests,
      move |_this, state, event: &ListEvent, cx| {
        if let ListEvent::Confirm(ix) = event {
          let row = state.read(cx).delegate().matched_rows.get(ix.row).cloned();
          if let Some(row) = row {
            GithubPrDetailsPageHandle::show(row.owner.clone(), row.repo.clone(), row.pr.number, cx);
          }
        }
      },
    );

    self._subscriptions.push(subscription);
  }

  fn refresh_pull_requests(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let owner = DEFAULT_ORG.to_string();
    let repo = DEFAULT_REPO.to_string();

    let mut start_data = Map::new();
    start_data.insert("owner".into(), owner.clone().into());
    start_data.insert("repo".into(), repo.clone().into());
    self.add_github_breadcrumb("Refresh pull requests started", start_data);

    self.error = None;
    self.pull_requests.update(cx, |state, cx| {
      state.delegate_mut().loading = true;
      cx.notify();
    });

    let task = cx.spawn(async move |this, cx| {
      let owner_for_request = owner.clone();
      let repo_for_request = repo.clone();
      let result =
        unblock(move || api.fetch_latest_pull_requests(&owner_for_request, &repo_for_request))
          .await
          .map_err(|error| error.to_string());

      let _ = this.update(cx, |this, cx| {
        let (rows, error): (Vec<Rc<GithubPullRequestRow>>, Option<SharedString>) = match result {
          Ok(pull_requests) => (
            pull_requests
              .into_iter()
              .map(|pr| {
                Rc::new(GithubPullRequestRow {
                  owner: pr.repository.owner.clone().into(),
                  repo: pr.repository.repo.clone().into(),
                  pr: Rc::new(pr),
                })
              })
              .collect::<Vec<_>>(),
            None,
          ),
          Err(error) => (Vec::new(), Some(error.into())),
        };

        match error.as_ref() {
          Some(error) => {
            let mut data = Map::new();
            data.insert("owner".into(), owner.clone().into());
            data.insert("repo".into(), repo.clone().into());
            this.add_github_breadcrumb("Refresh pull requests failed", data.clone());
            this.record_github_error("github.pull_requests.refresh", error.as_ref(), data);
          }
          None => {
            let mut data = Map::new();
            data.insert("owner".into(), owner.clone().into());
            data.insert("repo".into(), repo.clone().into());
            data.insert("count".into(), rows.len().into());
            this.add_github_breadcrumb("Refresh pull requests succeeded", data);
          }
        }

        this.error = error;

        this.pull_requests.update(cx, |state, cx| {
          state.delegate_mut().loading = false;
          state.delegate_mut().set_rows(rows);
          cx.notify();
        });

        cx.notify();
      });
    });

    self.load_task = Some(task);
    self.refresh_notifications(cx);
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
    let commands =
      CommandPaletteCommand::default_global_commands(CommandPalettePage::Github, include_github);

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
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
      } => {
        GithubPrDetailsPageHandle::show(owner.into(), repo.into(), number, cx);
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

  fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
      if AuthStateStore::has_active_subscription(cx) {
        WorkspaceRoute::open_github(cx);
      } else {
        WorkspaceRoute::open_billing(cx);
      }
      cx.refresh_windows();
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

    div()
      .h(px(HEADER_HEIGHT))
      .max_h(px(HEADER_HEIGHT))
      .w_full()
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(div().text_sm().text_color(theme.foreground).child("GitHub"))
      .when_some(auth_control, |this, control| this.child(control))
  }
}

impl Render for GithubPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    if self.focus_on_next_render {
      self.focus_on_next_render = false;
      cx.on_next_frame(window, |this, window, cx| this.focus_search(window, cx));
    }

    let list = List::new(&self.pull_requests)
      .search_placeholder("Search pull requests...")
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .flex_1()
      .min_w(px(0.0))
      .min_h_0()
      .p(px(8.));

    let unread_count = self.notifications.read(cx).delegate().unread_count();

    let notifications_list = List::new(&self.notifications)
      .search_placeholder("Search notifications...")
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .flex_1()
      .min_h_0()
      .p(px(8.));

    let notifications_panel = v_flex()
      .gap_2()
      .w(px(600.0))
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
              .child(Icon::new(IconName::Inbox).size_4())
              .child("Notifications"),
          )
          .when(unread_count > 0, |this| {
            this.child(
              Tag::secondary()
                .small()
                .rounded_full()
                .child(format!("{} unread", unread_count)),
            )
          }),
      )
      .when_some(self.notifications_error.clone(), |this, error| {
        this.child(div().text_sm().text_color(theme.status_red()).child(error))
      })
      .child(notifications_list);

    let pr_panel = v_flex()
      .gap_2()
      .flex_1()
      .min_w_0()
      .h_full()
      .min_h_0()
      .child("Latest Pull Requests")
      .child(list);

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
          .when_some(self.error.clone(), |this, error| {
            this.child(div().text_sm().text_color(theme.status_red()).child(error))
          })
          .child(
            h_flex()
              .h_full()
              .gap_3()
              .min_h_0()
              .items_start()
              .child(notifications_panel)
              .child(pr_panel),
          ),
      )
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
    ApiClient, GithubNotificationRepository, GithubNotificationSubject, GithubPullRequestLabel,
    GithubPullRequestState, GithubRepository,
  };
  use gpui::{Entity, TestAppContext};
  use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
  };

  fn make_pull_request_row(title: &str, owner: &str, repo: &str) -> GithubPullRequestRow {
    GithubPullRequestRow {
      pr: Rc::new(GithubPullRequest {
        number: 1,
        title: title.to_string(),
        state: GithubPullRequestState::Open,
        merged_at: None,
        draft: false,
        updated_at: "2026-02-15T12:00:00Z".to_string(),
        labels: vec![GithubPullRequestLabel {
          name: "test".to_string(),
        }],
        repository: GithubRepository {
          owner: owner.to_string(),
          repo: repo.to_string(),
        },
      }),
      owner: owner.to_string().into(),
      repo: repo.to_string().into(),
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
    });
  }

  async fn await_github_page_background_tasks(
    github_page: Entity<GithubPage>,
    cx: &mut gpui::VisualTestContext,
  ) {
    loop {
      let (load_task, notifications_task) = github_page.update_in(cx, |this, _window, _| {
        (this.load_task.take(), this.notifications_task.take())
      });

      let mut had_task = false;
      if let Some(task) = load_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = notifications_task {
        had_task = true;
        task.await;
      }

      if !had_task {
        break;
      }
    }
  }

  fn start_path_response_server(
    responses: Vec<(&str, &str, &str)>,
  ) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let address = format!("http://{}", listener.local_addr().expect("local addr"));
    let mut responses = responses
      .into_iter()
      .map(|(path, status, body)| (path.to_string(), status.to_string(), body.to_string()))
      .collect::<Vec<_>>();
    let expected_requests = responses.len();

    let handle = thread::spawn(move || {
      for _ in 0..expected_requests {
        let (mut stream, _) = listener.accept().expect("accept connection");
        let mut request_buffer = [0u8; 4096];
        let bytes_read = stream.read(&mut request_buffer).unwrap_or(0);
        let request = String::from_utf8_lossy(&request_buffer[..bytes_read]);
        let request_path = request
          .lines()
          .next()
          .and_then(|line| line.split_whitespace().nth(1))
          .and_then(|target| target.split('?').next())
          .unwrap_or_default();

        let (status, body) = if let Some(index) = responses
          .iter()
          .position(|(path, _, _)| path == request_path)
        {
          let (_, status, body) = responses.remove(index);
          (status, body)
        } else {
          (
            "404 Not Found".to_string(),
            format!(r#"{{"error":"unexpected path: {}"}}"#, request_path),
          )
        };

        let response = format!(
          "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
          body.as_bytes().len(),
          body
        );
        stream
          .write_all(response.as_bytes())
          .expect("write response");
      }
    });

    (address, handle)
  }

  #[test]
  fn compact_datetime_formats_rfc3339_values() {
    assert_eq!(
      format_compact_datetime("2026-02-15T12:34:56Z").as_ref(),
      "Feb 15, 2026 12:34"
    );
    assert_eq!(
      format_compact_datetime("2026-02-15T12:34:56+02:00").as_ref(),
      "Feb 15, 2026 12:34"
    );
  }

  #[test]
  fn compact_datetime_preserves_non_iso_inputs() {
    assert_eq!(format_compact_datetime("2026-02-15").as_ref(), "2026-02-15");
  }

  #[test]
  fn pull_request_row_matches_title_or_repo_case_insensitive() {
    let row = make_pull_request_row("Fix Login Bug", "Acme", "Portal");
    assert!(row.matches("login"));
    assert!(row.matches("acme/portal"));
    assert!(!row.matches("missing"));
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
    let mut delegate = GithubNotificationListDelegate::new();
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

    delegate.prepare("backend");
    assert_eq!(delegate.matched_rows.len(), 1);
    assert_eq!(
      delegate.matched_rows[0].notification.repository.full_name,
      "acme/backend"
    );
  }

  #[gpui::test]
  async fn refresh_pull_requests_sets_unauthorized_errors(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (base_url, handle) = start_path_response_server(vec![
      ("/github/pr/latest", "401 Unauthorized", "{}"),
      ("/github/notifications", "401 Unauthorized", "{}"),
    ]);
    let api = ApiClient::new_with_base_url(base_url);
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));
    cx.executor().allow_parking();

    let (load_task, notifications_task) = github_page.update_in(cx, |this, _window, cx| {
      this.refresh_pull_requests(cx);
      (
        this.load_task.take().expect("pull request load task"),
        this.notifications_task.take().expect("notifications task"),
      )
    });
    load_task.await;
    notifications_task.await;
    await_github_page_background_tasks(github_page.clone(), cx).await;

    let (
      error,
      notifications_error,
      pr_count,
      notifications_count,
      pr_loading,
      notifications_loading,
    ) = github_page.read_with(cx, |this, cx| {
      let pull_requests = this.pull_requests.read(cx);
      let notifications = this.notifications.read(cx);
      (
        this.error.clone(),
        this.notifications_error.clone(),
        pull_requests.delegate().matched_rows.len(),
        notifications.delegate().matched_rows.len(),
        pull_requests.delegate().loading,
        notifications.delegate().loading,
      )
    });

    assert!(
      error
        .as_ref()
        .is_some_and(|value| value.as_ref().contains("unauthorized"))
    );
    assert!(
      notifications_error
        .as_ref()
        .is_some_and(|value| value.as_ref().contains("unauthorized"))
    );
    assert_eq!(pr_count, 0);
    assert_eq!(notifications_count, 0);
    assert!(!pr_loading);
    assert!(!notifications_loading);

    handle.join().expect("join server thread");
  }

  #[gpui::test]
  async fn refresh_pull_requests_populates_pull_requests_and_notifications_on_success(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let pull_requests_body = r#"{"pullRequests":[{"number":42,"title":"Fix login","state":"open","mergedAt":null,"draft":false,"updatedAt":"2026-02-15T12:00:00Z","labels":[{"name":"bug"}],"repository":{"owner":"acme","repo":"portal"}}]}"#;
    let notifications_body = r#"{"notifications":[{"id":"n1","repository":{"name":"portal","fullName":"acme/portal","owner":null},"subject":{"title":"Please review","type":"PullRequest","url":null,"latestCommentUrl":null},"reason":"mention","unread":true,"updatedAt":"2026-02-15T12:10:00Z","lastReadAt":null,"url":"https://api.github.test/notif/1","subscriptionUrl":"https://api.github.test/sub/1"}]}"#;
    let (base_url, handle) = start_path_response_server(vec![
      ("/github/pr/latest", "200 OK", pull_requests_body),
      ("/github/notifications", "200 OK", notifications_body),
    ]);
    let api = ApiClient::new_with_base_url(base_url);
    let (github_page, cx) =
      cx.add_window_view(|window, cx| GithubPage::new_for_test(api, window, cx));
    cx.executor().allow_parking();

    let (load_task, notifications_task) = github_page.update_in(cx, |this, _window, cx| {
      this.refresh_pull_requests(cx);
      (
        this.load_task.take().expect("pull request load task"),
        this.notifications_task.take().expect("notifications task"),
      )
    });
    load_task.await;
    notifications_task.await;
    await_github_page_background_tasks(github_page.clone(), cx).await;

    let (error, notifications_error, pr_titles, notification_titles, unread_count) = github_page
      .read_with(cx, |this, cx| {
        let pull_requests = this.pull_requests.read(cx);
        let notifications = this.notifications.read(cx);
        (
          this.error.clone(),
          this.notifications_error.clone(),
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
        )
      });

    assert!(error.is_none());
    assert!(notifications_error.is_none());
    assert_eq!(pr_titles, vec!["Fix login".to_string()]);
    assert_eq!(notification_titles, vec!["Please review".to_string()]);
    assert_eq!(unread_count, 1);

    handle.join().expect("join server thread");
  }
}
