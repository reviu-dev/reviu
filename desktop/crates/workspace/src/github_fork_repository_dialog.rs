use gpui::{
  AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, ParentElement, Render,
  SharedString, Styled, Subscription, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, IndexPath, Sizable as _,
  button::{Button, ButtonVariants as _},
  checkbox::Checkbox,
  dialog::{DialogDescription, DialogFooter, DialogHeader, DialogTitle},
  h_flex,
  notification::Notification,
  select::{Select, SelectEvent, SelectState},
  spinner::Spinner,
  v_flex,
};
use smol::unblock;
use ui::{Input, InputState, StatusThemeExt, UiIconName, WindowExt};

use crate::{
  api::{ApiClient, CreateRepositoryOwner, GithubUserOrganization},
  auth_state::AuthStateStore,
  github_navigation::open_repo_target,
};

const REPOSITORY_NAME_MAX_LENGTH: usize = 100;

fn is_valid_repository_name_char(c: char) -> bool {
  c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'
}

struct ForkRepositoryErrorNotificationId;

pub struct ForkRepositoryDialog {
  api: ApiClient,
  window_handle: AnyWindowHandle,
  source_owner: String,
  source_repo: String,
  name_input: Entity<InputState>,
  owner_select: Entity<SelectState<Vec<String>>>,
  owners: Vec<OwnerChoice>,
  selected_owner_index: Option<usize>,
  default_branch_only: bool,
  owners_loading: bool,
  owners_task: Option<Task<()>>,
  submit_loading: bool,
  submit_task: Option<Task<()>>,
  validation_error: Option<SharedString>,
  _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug)]
struct OwnerChoice {
  label: String,
  owner: CreateRepositoryOwner,
}

impl ForkRepositoryDialog {
  fn new(
    api: ApiClient,
    window_handle: AnyWindowHandle,
    source_owner: String,
    source_repo: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let placeholder_name = source_repo.clone();
    let initial_name = source_repo.clone();
    let name_input = cx.new(|cx| {
      let mut state = InputState::new(window, cx).placeholder(placeholder_name);
      state.set_value(initial_name, window, cx);
      state
    });
    let owner_select = cx.new(|cx| SelectState::new(Vec::<String>::new(), None, window, cx));

    let subscription = cx.subscribe(
      &owner_select,
      move |this, _, event: &SelectEvent<Vec<String>>, cx| {
        if let SelectEvent::Confirm(Some(label)) = event {
          let new_index = this.owners.iter().position(|choice| choice.label == *label);
          if new_index != this.selected_owner_index {
            this.selected_owner_index = new_index;
            cx.notify();
          }
        }
      },
    );

    let viewer_login = AuthStateStore::get(cx).github_login();
    let mut owners = Vec::new();
    if let Some(login) = viewer_login {
      owners.push(OwnerChoice {
        label: login,
        owner: CreateRepositoryOwner::Viewer,
      });
    }

    let mut this = Self {
      api,
      window_handle,
      source_owner,
      source_repo,
      name_input,
      owner_select,
      owners,
      selected_owner_index: None,
      default_branch_only: true,
      owners_loading: false,
      owners_task: None,
      submit_loading: false,
      submit_task: None,
      validation_error: None,
      _subscriptions: vec![subscription],
    };

    this.refresh_owner_select(window, cx);
    this.load_organizations(cx);
    this
  }

  fn refresh_owner_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let labels: Vec<String> = self.owners.iter().map(|o| o.label.clone()).collect();
    let selected = if labels.is_empty() {
      None
    } else {
      let idx = self.selected_owner_index.unwrap_or(0).min(labels.len() - 1);
      Some(idx)
    };
    self.selected_owner_index = selected;
    let index_path = selected.map(IndexPath::new);
    self.owner_select.update(cx, |state, cx| {
      state.set_items(labels, window, cx);
      state.set_selected_index(index_path, window, cx);
    });
  }

  fn load_organizations(&mut self, cx: &mut Context<Self>) {
    if self.owners_loading {
      return;
    }
    self.owners_loading = true;

    let api = self.api.clone();
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_github_user_organizations()).await;

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.owners_loading = false;
          if let Ok(orgs) = result {
            this.append_organizations(orgs);
            this.refresh_owner_select(window, cx);
          }
          cx.notify();
        });
      });
    });

    self.owners_task = Some(task);
    cx.notify();
  }

  fn append_organizations(&mut self, orgs: Vec<GithubUserOrganization>) {
    for org in orgs {
      if self
        .owners
        .iter()
        .any(|existing| existing.label.eq_ignore_ascii_case(&org.login))
      {
        continue;
      }
      self.owners.push(OwnerChoice {
        label: org.login.clone(),
        owner: CreateRepositoryOwner::Organization(org.login),
      });
    }
  }

  fn toggle_default_branch_only_action(
    &mut self,
    checked: bool,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.default_branch_only = checked;
    cx.notify();
  }

  fn validate_repository_name(name: &str) -> Option<SharedString> {
    if name.is_empty() {
      return Some("Repository name is required.".into());
    }
    if name.len() > REPOSITORY_NAME_MAX_LENGTH {
      return Some(
        format!("Repository name must be at most {REPOSITORY_NAME_MAX_LENGTH} characters.").into(),
      );
    }
    if !name.chars().all(is_valid_repository_name_char) {
      return Some("Name may only contain letters, numbers, dots, hyphens and underscores.".into());
    }
    None
  }

  fn submit_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    if self.submit_loading {
      return;
    }

    let name = self.name_input.read(cx).value().trim().to_string();
    if let Some(error) = Self::validate_repository_name(&name) {
      self.validation_error = Some(error);
      cx.notify();
      return;
    }

    let Some(index) = self.selected_owner_index else {
      self.validation_error = Some("Pick an owner for the forked repository.".into());
      cx.notify();
      return;
    };

    let Some(owner_choice) = self.owners.get(index).cloned() else {
      self.validation_error = Some("Selected owner is no longer available.".into());
      cx.notify();
      return;
    };

    self.validation_error = None;
    self.submit_loading = true;

    let api = self.api.clone();
    let source_owner = self.source_owner.clone();
    let source_repo = self.source_repo.clone();
    let default_branch_only = self.default_branch_only;
    let window_handle = self.window_handle;
    let name_for_task = if name == self.source_repo {
      None
    } else {
      Some(name)
    };

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        api.fork_github_repository(
          &source_owner,
          &source_repo,
          &owner_choice.owner,
          name_for_task.as_deref(),
          default_branch_only,
        )
      })
      .await;

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.submit_loading = false;
          cx.notify();
        });

        match result {
          Ok(repository) => {
            window.close_dialog(cx);
            open_repo_target(repository.owner, repository.repo, None, None, None, cx);
          }
          Err(error) => {
            window.push_notification(
              Notification::error(error.to_string())
                .id::<ForkRepositoryErrorNotificationId>()
                .title("Fork repository failed"),
              cx,
            );
          }
        }
      });
    });

    self.submit_task = Some(task);
    cx.notify();
  }
}

impl Focusable for ForkRepositoryDialog {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    self.name_input.read(cx).focus_handle(cx)
  }
}

impl Render for ForkRepositoryDialog {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let owner_missing = self.owners.is_empty();
    let submit_disabled = self.submit_loading || owner_missing;
    let source_label = format!("{}/{}", self.source_owner, self.source_repo);

    div()
      .id("github-fork-repository-dialog")
      .flex()
      .flex_col()
      .child(
        DialogHeader::new()
          .p_4()
          .child(DialogTitle::new().child("Fork repository"))
          .child(DialogDescription::new().child(format!(
            "Create a fork of {source_label} under your account or an organization."
          ))),
      )
      .child(
        v_flex()
          .px_4()
          .pb_4()
          .gap_3()
          .child(
            v_flex()
              .gap_1()
              .child(
                h_flex()
                  .justify_between()
                  .items_center()
                  .child(div().text_sm().child("Owner"))
                  .when(self.owners_loading, |this| {
                    this.child(Spinner::new().xsmall())
                  }),
              )
              .child(
                Select::new(&self.owner_select)
                  .placeholder("Select an owner")
                  .disabled(self.submit_loading || owner_missing),
              ),
          )
          .child(
            v_flex()
              .gap_1()
              .child(div().text_sm().child("Repository name"))
              .child(Input::new(&self.name_input).w_full()),
          )
          .child(
            Checkbox::new("github-fork-default-branch-only")
              .checked(self.default_branch_only)
              .label("Copy the default branch only")
              .on_click(cx.listener(|this, checked, window, cx| {
                this.toggle_default_branch_only_action(*checked, window, cx);
              })),
          )
          .when(self.validation_error.is_some(), |this| {
            let error = self.validation_error.clone().unwrap_or_default();
            this.child(div().text_xs().text_color(theme.status_red()).child(error))
          }),
      )
      .child(
        DialogFooter::new()
          .px_4()
          .pb_4()
          .pt_1()
          .justify_end()
          .child(
            Button::new("cancel-fork-repository")
              .label("Cancel")
              .outline()
              .disabled(self.submit_loading)
              .on_click(|_, window, cx| {
                window.close_dialog(cx);
              }),
          )
          .child(
            Button::new("submit-fork-repository")
              .label("Create fork")
              .icon(UiIconName::GitFork)
              .primary()
              .loading(self.submit_loading)
              .disabled(submit_disabled)
              .on_click(cx.listener(Self::submit_action)),
          ),
      )
  }
}

pub fn open_fork_repository_dialog(
  api: ApiClient,
  source_owner: String,
  source_repo: String,
  window: &mut Window,
  _cx: &mut App,
) {
  // Defer to next frame so any currently-open dialog closes first
  window.on_next_frame(move |window, cx| {
    open_fork_repository_dialog_inner(
      api.clone(),
      source_owner.clone(),
      source_repo.clone(),
      window,
      cx,
    );
  });
}

fn open_fork_repository_dialog_inner(
  api: ApiClient,
  source_owner: String,
  source_repo: String,
  window: &mut Window,
  cx: &mut App,
) {
  let window_handle = window.window_handle();
  let dialog = cx
    .new(|cx| ForkRepositoryDialog::new(api, window_handle, source_owner, source_repo, window, cx));
  let dialog_for_overlay = dialog.clone();
  let dialog_for_focus = dialog.clone();

  window.open_dialog(cx, move |overlay, _, _| {
    overlay.p_0().w(px(520.0)).child(dialog_for_overlay.clone())
  });

  window.on_next_frame(move |window, cx| {
    let focus_handle = dialog_for_focus.read(cx).focus_handle(cx);
    window.focus(&focus_handle, cx);
  });
}
