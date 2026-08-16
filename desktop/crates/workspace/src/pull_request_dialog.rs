//! Create-pull-request dialog and its template lookup.

use std::rc::Rc;

use gpui::{
  AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
  Render, SharedString, Styled, Subscription, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Sizable as _,
  checkbox::Checkbox,
  dialog::{DialogDescription, DialogFooter, DialogHeader, DialogTitle},
  h_flex,
  input::{Input, InputState},
  notification::Notification,
  select::{Select, SelectEvent, SelectState},
  spinner::Spinner,
  v_flex,
};
use ui::{
  Button, ButtonVariants as _, StatusThemeExt as _, Textarea, TextareaState, UiIconName,
  WindowExt as _,
};

use crate::api::{ApiClient, GithubPullRequest};
use crate::github_pr_details_page::GithubPrDetailsPageHandle;
use crate::github_shared;

/// Errors of a git action share one notification slot, so they replace each other.
struct GitActionErrorNotificationId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GithubBranchContext {
  pub owner: String,
  pub repo: String,
  pub branch: String,
}

/// Invoked after the dialog successfully creates a pull request.
pub(crate) type PullRequestCreatedHandler =
  Rc<dyn Fn(&GithubBranchContext, &GithubPullRequest, &mut gpui::App)>;

struct CreatePullRequestDialog {
  api: ApiClient,
  window_handle: AnyWindowHandle,
  on_created: PullRequestCreatedHandler,
  branch_context: GithubBranchContext,
  title_input: Entity<InputState>,
  base_input: Entity<InputState>,
  body_input: Entity<TextareaState>,
  template_select: Entity<SelectState<Vec<String>>>,
  draft: bool,
  default_branch_loading: bool,
  template_loading: bool,
  template_options_count: usize,
  submit_loading: bool,
  validation_error: Option<SharedString>,
  default_branch_task: Option<Task<()>>,
  template_task: Option<Task<()>>,
  submit_task: Option<Task<()>>,
  _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PullRequestTemplateLoadResult {
  default_branch: Option<String>,
  template_paths: Vec<String>,
  template_body: Option<String>,
}

const PULL_REQUEST_TEMPLATE_SINGLE_PATHS: [&str; 3] = [
  ".github/pull_request_template.md",
  "pull_request_template.md",
  "docs/pull_request_template.md",
];
const PULL_REQUEST_TEMPLATE_DIRECTORY_PATHS: [&str; 3] = [
  ".github/PULL_REQUEST_TEMPLATE/",
  "PULL_REQUEST_TEMPLATE/",
  "docs/PULL_REQUEST_TEMPLATE/",
];

fn resolve_pull_request_template_paths(
  entries: &[crate::api::GithubRepositoryTreeEntry],
) -> Vec<String> {
  let mut paths = Vec::new();

  for candidate in PULL_REQUEST_TEMPLATE_SINGLE_PATHS {
    if entries
      .iter()
      .any(|entry| entry.entry_type == "blob" && entry.path == candidate)
    {
      paths.push(candidate.to_string());
    }
  }

  for directory in PULL_REQUEST_TEMPLATE_DIRECTORY_PATHS {
    let mut directory_paths = entries
      .iter()
      .filter(|entry| entry.entry_type == "blob")
      .filter_map(|entry| {
        let suffix = entry.path.strip_prefix(directory)?;
        if suffix.is_empty() || suffix.contains('/') {
          return None;
        }

        Some(entry.path.clone())
      })
      .collect::<Vec<_>>();
    directory_paths.sort();

    for path in directory_paths {
      if !paths.contains(&path) {
        paths.push(path);
      }
    }
  }

  paths
}

impl CreatePullRequestDialog {
  fn new(
    api: ApiClient,
    window_handle: AnyWindowHandle,
    on_created: PullRequestCreatedHandler,
    branch_context: GithubBranchContext,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let template_select = cx.new(|cx| SelectState::new(Vec::<String>::new(), None, window, cx));
    let subscription = cx.subscribe(
      &template_select,
      move |this, _, event: &SelectEvent<Vec<String>>, cx| {
        let SelectEvent::Confirm(Some(path)) = event else {
          return;
        };
        this.load_pull_request_template(path.clone(), cx);
      },
    );

    let mut this = Self {
      api,
      window_handle,
      on_created,
      branch_context,
      title_input: cx.new(|cx| InputState::new(window, cx).placeholder("Pull request title")),
      base_input: cx.new(|cx| InputState::new(window, cx).placeholder("Base branch")),
      body_input: cx.new(|cx| {
        TextareaState::new(window, cx)
          .auto_grow(4, 10)
          .placeholder("Add an optional description...")
      }),
      template_select,
      draft: false,
      default_branch_loading: false,
      template_loading: false,
      template_options_count: 0,
      submit_loading: false,
      validation_error: None,
      default_branch_task: None,
      template_task: None,
      submit_task: None,
      _subscriptions: vec![subscription],
    };
    this.load_repository_defaults(cx);
    this
  }

  fn load_repository_defaults(&mut self, cx: &mut Context<Self>) {
    if self.default_branch_loading {
      return;
    }

    self.default_branch_loading = true;
    self.template_loading = true;

    let api = self.api.clone();
    let owner = self.branch_context.owner.clone();
    let repo = self.branch_context.repo.clone();
    let base_input = self.base_input.clone();
    let body_input = self.body_input.clone();
    let template_select = self.template_select.clone();
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          let details = api.fetch_github_repository_details(&owner, &repo).ok();
          let default_branch = details
            .as_ref()
            .map(|details| details.default_branch.trim().to_string())
            .filter(|value| !value.is_empty());

          let template_paths = default_branch
            .as_ref()
            .and_then(|default_branch| {
              api
                .fetch_github_repository_tree(&owner, &repo, default_branch)
                .ok()
                .map(|tree| resolve_pull_request_template_paths(&tree.tree))
            })
            .unwrap_or_default();

          let template_body = if template_paths.len() == 1 {
            default_branch.as_ref().and_then(|default_branch| {
              api
                .fetch_github_file_content(&owner, &repo, &template_paths[0], default_branch)
                .ok()
                .flatten()
            })
          } else {
            None
          };

          PullRequestTemplateLoadResult {
            default_branch,
            template_paths,
            template_body,
          }
        })
        .await;

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.default_branch_loading = false;
          this.template_loading = false;
          this.template_options_count = result.template_paths.len();

          if let Some(default_branch) = result.default_branch.clone()
            && base_input.read(cx).value().trim().is_empty()
          {
            base_input.update(cx, |input, cx| {
              input.set_value(default_branch, window, cx);
            });
          }

          template_select.update(cx, |state, cx| {
            state.set_items(result.template_paths.clone(), window, cx);
            state.set_selected_index(None, window, cx);
          });

          if let Some(template_body) = result.template_body.clone() {
            body_input.update(cx, |input, cx| {
              input.set_value(template_body, window, cx);
            });
          }

          cx.notify();
        });
      });
    });

    self.default_branch_task = Some(task);
    cx.notify();
  }

  fn load_pull_request_template(&mut self, template_path: String, cx: &mut Context<Self>) {
    if self.template_loading {
      return;
    }

    let base_branch = self.base_input.read(cx).value().trim().to_string();
    if base_branch.is_empty() {
      return;
    }

    self.template_loading = true;

    let api = self.api.clone();
    let owner = self.branch_context.owner.clone();
    let repo = self.branch_context.repo.clone();
    let body_input = self.body_input.clone();
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          api.fetch_github_file_content(&owner, &repo, &template_path, &base_branch)
        })
        .await;

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.template_loading = false;

          if let Ok(Some(template_body)) = result {
            body_input.update(cx, |input, cx| {
              input.set_value(template_body, window, cx);
            });
          }

          cx.notify();
        });
      });
    });

    self.template_task = Some(task);
    cx.notify();
  }

  fn submit_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    if self.submit_loading {
      return;
    }

    let title = self.title_input.read(cx).value().trim().to_string();
    let base = self.base_input.read(cx).value().trim().to_string();
    let body = self.body_input.read(cx).value().to_string();

    if title.is_empty() {
      self.validation_error = Some("Pull request title is required.".into());
      cx.notify();
      return;
    }

    if base.is_empty() {
      self.validation_error = Some("Base branch is required.".into());
      cx.notify();
      return;
    }

    self.validation_error = None;
    self.submit_loading = true;

    let api = self.api.clone();
    let owner = self.branch_context.owner.clone();
    let repo = self.branch_context.repo.clone();
    let branch = self.branch_context.branch.clone();
    let branch_context = self.branch_context.clone();
    let on_created = self.on_created.clone();
    let draft = self.draft;
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          api.create_pull_request(
            &owner,
            &repo,
            &branch,
            &title,
            &base,
            Some(body.as_str()),
            draft,
          )
        })
        .await;

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.submit_loading = false;
          cx.notify();
        });

        match result {
          Ok(pull_request) => {
            on_created(&branch_context, &pull_request, cx);
            window.close_dialog(cx);
            GithubPrDetailsPageHandle::show_with_open_target(
              pull_request.repository.owner.into(),
              pull_request.repository.repo.into(),
              pull_request.number,
              false,
              None,
              cx,
            );
          }
          Err(error) => {
            window.push_notification(
              Notification::error(error.to_string())
                .id::<GitActionErrorNotificationId>()
                .title("Create pull request failed"),
              cx,
            );
          }
        }
      });
    });

    self.submit_task = Some(task);
    cx.notify();
  }

  fn toggle_draft_action(&mut self, checked: bool, _: &mut Window, cx: &mut Context<Self>) {
    self.draft = checked;
    cx.notify();
  }
}

impl Focusable for CreatePullRequestDialog {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    self.title_input.read(cx).focus_handle(cx)
  }
}

impl Render for CreatePullRequestDialog {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let branch_label = format!(
      "{}:{}",
      self.branch_context.owner, self.branch_context.branch
    );
    let repo_label = github_shared::repo_label(
      self.branch_context.owner.as_str(),
      self.branch_context.repo.as_str(),
    );

    div()
      .id("git-create-pull-request-dialog")
      .flex()
      .flex_col()
      .child(
        DialogHeader::new()
          .p_4()
          .child(DialogTitle::new().child("Create Pull Request"))
          .child(DialogDescription::new().child(format!(
            "Create a pull request from {branch_label} in {repo_label}."
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
              .child(div().text_sm().child("Title"))
              .child(Input::new(&self.title_input).w_full()),
          )
          .child(
            v_flex()
              .gap_1()
              .child(
                h_flex()
                  .justify_between()
                  .items_center()
                  .child(div().text_sm().child("Base Branch"))
                  .when(self.default_branch_loading, |this| {
                    this.child(Spinner::new().xsmall())
                  }),
              )
              .child(Input::new(&self.base_input).w_full()),
          )
          .child(
            v_flex()
              .gap_1()
              .when(self.template_options_count > 1, |this| {
                this.child(
                  v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Template"))
                    .child(
                      Select::new(&self.template_select)
                        .placeholder("Select a pull request template...")
                        .disabled(self.template_loading || self.submit_loading),
                    ),
                )
              })
              .child(
                h_flex()
                  .justify_between()
                  .items_center()
                  .child(div().text_sm().child("Description"))
                  .when(self.template_loading, |this| {
                    this.child(Spinner::new().xsmall())
                  }),
              )
              .child(
                Textarea::new(&self.body_input)
                  .w_full()
                  .disabled(self.template_loading),
              ),
          )
          .child(
            Checkbox::new("git-create-pull-request-draft")
              .checked(self.draft)
              .label("Create as draft")
              .on_click(cx.listener(|this, checked, window, cx| {
                this.toggle_draft_action(*checked, window, cx);
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
            Button::new("cancel-create-pull-request")
              .label("Cancel")
              .outline()
              .disabled(self.submit_loading)
              .on_click(|_, window, cx| {
                window.close_dialog(cx);
              }),
          )
          .child(
            Button::new("submit-create-pull-request")
              .label("Create pull request")
              .icon(UiIconName::GitPullRequestArrow)
              .primary()
              .loading(self.submit_loading)
              .disabled(self.submit_loading)
              .on_click(cx.listener(Self::submit_action)),
          ),
      )
  }
}

pub(crate) fn open_create_pull_request_dialog(
  api: ApiClient,
  window_handle: AnyWindowHandle,
  on_created: PullRequestCreatedHandler,
  branch_context: GithubBranchContext,
  window: &mut Window,
  cx: &mut App,
) {
  let dialog = cx.new(|cx| {
    CreatePullRequestDialog::new(
      api.clone(),
      window_handle,
      on_created,
      branch_context,
      window,
      cx,
    )
  });
  let dialog_for_overlay = dialog.clone();
  let dialog_for_focus = dialog.clone();

  window.open_dialog(cx, move |overlay, _, _| {
    overlay.p_0().w(px(600.0)).child(dialog_for_overlay.clone())
  });

  window.on_next_frame(move |window, cx| {
    let focus_handle = dialog_for_focus.read(cx).focus_handle(cx);
    window.focus(&focus_handle, cx);
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  fn make_repository_tree_entry(path: &str) -> crate::api::GithubRepositoryTreeEntry {
    crate::api::GithubRepositoryTreeEntry {
      path: path.to_string(),
      mode: "100644".to_string(),
      entry_type: "blob".to_string(),
      sha: "deadbeef".to_string(),
      size: Some(128),
      url: None,
    }
  }

  #[test]
  fn resolve_pull_request_template_paths_prefers_documented_single_template_locations() {
    let entries = vec![
      make_repository_tree_entry("docs/pull_request_template.md"),
      make_repository_tree_entry("pull_request_template.md"),
      make_repository_tree_entry(".github/pull_request_template.md"),
    ];

    assert_eq!(
      resolve_pull_request_template_paths(&entries),
      vec![
        ".github/pull_request_template.md".to_string(),
        "pull_request_template.md".to_string(),
        "docs/pull_request_template.md".to_string(),
      ]
    );
  }

  #[test]
  fn resolve_pull_request_template_paths_collects_direct_children_from_template_directories() {
    let entries = vec![
      make_repository_tree_entry(".github/PULL_REQUEST_TEMPLATE/bugfix.md"),
      make_repository_tree_entry(".github/PULL_REQUEST_TEMPLATE/feature.md"),
      make_repository_tree_entry(".github/PULL_REQUEST_TEMPLATE/nested/mobile/template.md"),
      make_repository_tree_entry("docs/PULL_REQUEST_TEMPLATE/release.md"),
    ];

    assert_eq!(
      resolve_pull_request_template_paths(&entries),
      vec![
        ".github/PULL_REQUEST_TEMPLATE/bugfix.md".to_string(),
        ".github/PULL_REQUEST_TEMPLATE/feature.md".to_string(),
        "docs/PULL_REQUEST_TEMPLATE/release.md".to_string(),
      ]
    );
  }
}
