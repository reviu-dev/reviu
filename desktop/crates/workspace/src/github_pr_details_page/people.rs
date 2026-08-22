//! Assignees, requested reviewers and labels: the token rows of the overview
//! and everything that adds to or removes from them.

use super::*;

fn reviewer_status_tooltip(status: ReviewerStatus, login: &str) -> SharedString {
  match status {
    ReviewerStatus::Awaiting => format!("Awaiting requested review from {login}").into(),
    ReviewerStatus::Approved => format!("{login} approved").into(),
    ReviewerStatus::Commented => format!("{login} left review comments").into(),
    ReviewerStatus::ChangesRequested => format!("{login} requested changes").into(),
  }
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

impl GithubPrDetailsPage {
  pub(super) fn render_people_token_row(
    id_prefix: &'static str,
    users: &[GithubPullRequestFilterOptionUser],
    _can_remove: bool,
    _on_remove: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
  ) -> impl IntoElement {
    let mut row = h_flex().gap_1().items_center();
    for user in users {
      let login = user.login.clone();
      let avatar = Avatar::new()
        .name(login.clone())
        .when_some(user.avatar_url.clone(), |this, url| this.src(url))
        .small();
      let wrapper = div()
        .id(format!("{id_prefix}-{}", login))
        .child(avatar)
        .hoverable_tooltip({
          let tooltip = login.clone();
          move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)
        });
      row = row.child(wrapper);
    }

    h_flex().gap_2().items_center().ml_auto().child(row)
  }

  pub(super) fn render_people_skeleton_row(count: usize) -> impl IntoElement {
    let mut row = h_flex().gap_1().items_center();
    for _ in 0..count {
      row = row.child(Skeleton::new().size(px(24.0)).rounded_full());
    }
    h_flex().gap_2().items_center().ml_auto().child(row)
  }

  pub(super) fn render_label_skeleton_row(count: usize) -> impl IntoElement {
    let mut row = h_flex().gap_1().items_center();
    for _ in 0..count {
      row = row.child(Skeleton::new().w(px(85.0)).h(px(17.0)).rounded_full());
    }
    h_flex().gap_2().items_center().child(row)
  }

  pub(super) fn render_requested_reviewer_row(
    users: &[GithubPullRequestFilterOptionUser],
    reviews: &[GithubPullRequestReview],
    requested_reviewers: &[GithubPullRequestFilterOptionUser],
    theme: &gpui_component::Theme,
  ) -> impl IntoElement {
    let mut row = h_flex().gap_1().items_center();

    for user in users {
      let login = user.login.clone();
      let avatar = Avatar::new()
        .name(login.clone())
        .when_some(user.avatar_url.clone(), |this, url| this.src(url))
        .small();

      let badge_size = px(12.0);
      let status = reviewer_status_for_login(reviews, login.as_str(), requested_reviewers);
      let tooltip = reviewer_status_tooltip(status, login.as_str());
      let status_marker = match status {
        ReviewerStatus::Awaiting => div().size(px(9.0)).rounded_full().bg(theme.status_yellow()),
        ReviewerStatus::Approved => div()
          .size(badge_size)
          .rounded_full()
          .bg(theme.status_green())
          .flex()
          .items_center()
          .justify_center()
          .child(Icon::new(UiIconName::Check).size_3().text_color(white())),
        ReviewerStatus::Commented => div()
          .size(badge_size)
          .rounded_full()
          .bg(theme.background)
          .flex()
          .items_center()
          .justify_center()
          .child(
            Icon::new(UiIconName::MessageCircle)
              .size_3()
              .text_color(theme.muted_foreground),
          ),
        ReviewerStatus::ChangesRequested => div()
          .size(badge_size)
          .rounded_full()
          .bg(theme.status_red())
          .flex()
          .items_center()
          .justify_center()
          .child(Icon::new(UiIconName::FileDiff).size_3().text_color(white())),
      };
      let status_overlay = div()
        .absolute()
        .right_0()
        .bottom_0()
        .flex()
        .items_center()
        .justify_center()
        .child(status_marker);
      let wrapper = div()
        .relative()
        .id(format!("github-pr-reviewer-{}", login))
        .child(avatar)
        .child(status_overlay)
        .hoverable_tooltip({
          let tooltip = tooltip.clone();
          move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)
        });
      row = row.child(wrapper);
    }

    h_flex().gap_2().items_center().ml_auto().child(row)
  }

  pub(super) fn subscribe_to_assignee_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    cx.subscribe_in(
      &self.assignee_input,
      window,
      |this, state, event: &InputEvent, window, cx| match event {
        InputEvent::Change => {
          this.people_mutation_error = None;
          cx.notify();
        }
        InputEvent::PressEnter { .. } => {
          this.add_assignee_value(state.read(cx).value().as_str(), window, cx);
        }
        _ => {}
      },
    )
    .detach();
  }

  pub(super) fn subscribe_to_requested_reviewer_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    cx.subscribe_in(
      &self.requested_reviewer_input,
      window,
      |this, state, event: &InputEvent, window, cx| match event {
        InputEvent::Change => {
          this.people_mutation_error = None;
          cx.notify();
        }
        InputEvent::PressEnter { .. } => {
          this.add_requested_reviewer_value(state.read(cx).value().as_str(), window, cx);
        }
        _ => {}
      },
    )
    .detach();
  }

  pub(super) fn subscribe_to_label_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    cx.subscribe_in(
      &self.label_input,
      window,
      |this, state, event: &InputEvent, window, cx| match event {
        InputEvent::Change => {
          this.label_mutation_error = None;
          cx.notify();
        }
        InputEvent::PressEnter { .. } => {
          this.add_label_value(state.read(cx).value().as_str(), window, cx);
        }
        _ => {}
      },
    )
    .detach();
  }

  pub(super) fn assignee_query(&self, cx: &App) -> String {
    self.assignee_input.read(cx).value().trim().to_string()
  }

  pub(super) fn requested_reviewer_query(&self, cx: &App) -> String {
    self
      .requested_reviewer_input
      .read(cx)
      .value()
      .trim()
      .to_string()
  }

  pub(super) fn label_query(&self, cx: &App) -> String {
    self.label_input.read(cx).value().trim().to_string()
  }

  pub(super) fn can_edit_people(&self, pr: &GithubPullRequestDetails) -> bool {
    pr.state == GithubPullRequestState::Open
      && pr.merged_at.is_none()
      && !self.people_mutation_loading
  }

  pub(super) fn can_edit_labels(&self, pr: &GithubPullRequestDetails) -> bool {
    pr.state == GithubPullRequestState::Open
      && pr.merged_at.is_none()
      && !self.label_mutation_loading
  }

  pub(super) fn clear_input(
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    input.update(cx, |input, cx| input.set_value("", window, cx));
  }

  pub(super) fn refresh_review_people_options_for_current_context(
    &mut self,
    cx: &mut Context<Self>,
  ) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };

    self.review_people_options_loading = true;
    self.review_people_options_error = None;
    self.label_options_loading = true;
    self.label_options_error = None;

    let api = self.api.clone();
    let full_name = format!("{}/{}", context.owner, context.repo);
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move { api.fetch_github_pull_request_filter_options(&[full_name]) })
        .await;

      let _ = cx.update_window(window_handle, |_, _, cx| {
        let _ = this.update(cx, |this, cx| {
          match result {
            Ok(options) => {
              this.review_people_options = options.assignees;
              this.label_options = options.labels;
              this.review_people_options_loading = false;
              this.review_people_options_error = None;
              this.label_options_loading = false;
              this.label_options_error = None;
            }
            Err(error) => {
              this.review_people_options = Vec::new();
              this.label_options = Vec::new();
              this.review_people_options_loading = false;
              this.review_people_options_error = Some(error.to_string().into());
              this.label_options_loading = false;
              this.label_options_error = Some(error.to_string().into());
            }
          }
          this.review_people_options_task = None;
          cx.notify();
        });
      });
    });

    self.review_people_options_task = Some(task);
  }

  pub(super) fn add_assignee_value(
    &mut self,
    raw_value: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.people_mutation_loading {
      return;
    }
    let Some(pull_request) = self.pull_request.as_ref() else {
      self.people_mutation_error = Some("No pull request selected".into());
      return;
    };
    let login = raw_value.trim();
    if login.is_empty() {
      return;
    }
    if pull_request
      .assignees
      .iter()
      .any(|user| github_shared::logins_match_case_insensitive(user.login.as_str(), login))
    {
      Self::clear_input(&self.assignee_input, window, cx);
      return;
    }

    self.people_mutation_loading = true;
    self.people_mutation_error = None;
    let api = self.api.clone();
    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let login_string = login.to_string();
    let user = find_filter_option_user(&self.review_people_options, login);
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          api.add_pull_request_assignee(&owner, &repo, number, &login_string)
        })
        .await;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          match result {
            Ok(()) => {
              if let Some(pr) = this.pull_request.as_mut() {
                upsert_filter_option_user(&mut pr.assignees, user.clone());
              }
              Self::clear_input(&this.assignee_input, window, cx);
              this.people_mutation_error = None;
            }
            Err(error) => {
              this.people_mutation_error = Some(error.to_string().into());
            }
          }
          this.people_mutation_loading = false;
          this.people_mutation_task = None;
          cx.notify();
        });
      });
    });

    self.people_mutation_task = Some(task);
  }

  pub(super) fn remove_assignee(&mut self, login: &str, cx: &mut Context<Self>) {
    if self.people_mutation_loading {
      return;
    }
    let Some(pull_request) = self.pull_request.as_ref() else {
      self.people_mutation_error = Some("No pull request selected".into());
      return;
    };
    let login = login.trim();
    if login.is_empty() {
      return;
    }

    self.people_mutation_loading = true;
    self.people_mutation_error = None;
    let api = self.api.clone();
    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let login_string = login.to_string();
    let login_for_request = login_string.clone();

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          api.remove_pull_request_assignee(&owner, &repo, number, &login_for_request)
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            if let Some(pr) = this.pull_request.as_mut() {
              remove_filter_option_user(&mut pr.assignees, &login_string);
            }
            this.people_mutation_error = None;
          }
          Err(error) => {
            this.people_mutation_error = Some(error.to_string().into());
          }
        }
        this.people_mutation_loading = false;
        this.people_mutation_task = None;
        cx.notify();
      });
    });

    self.people_mutation_task = Some(task);
  }

  pub(super) fn add_requested_reviewer_value(
    &mut self,
    raw_value: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.people_mutation_loading {
      return;
    }
    let Some(pull_request) = self.pull_request.as_ref() else {
      self.people_mutation_error = Some("No pull request selected".into());
      return;
    };
    let login = raw_value.trim();
    if login.is_empty() {
      return;
    }
    if github_shared::logins_match_case_insensitive(login, pull_request.author.login.as_str()) {
      self.people_mutation_error =
        Some("You cannot request a review from the pull request author.".into());
      return;
    }
    if pull_request
      .requested_reviewers
      .iter()
      .any(|user| github_shared::logins_match_case_insensitive(user.login.as_str(), login))
    {
      Self::clear_input(&self.requested_reviewer_input, window, cx);
      return;
    }

    self.people_mutation_loading = true;
    self.people_mutation_error = None;
    let api = self.api.clone();
    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let login_string = login.to_string();
    let user = find_filter_option_user(&self.review_people_options, login);
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          api.request_pull_request_reviewer(&owner, &repo, number, &login_string)
        })
        .await;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          match result {
            Ok(()) => {
              if let Some(pr) = this.pull_request.as_mut() {
                upsert_filter_option_user(&mut pr.requested_reviewers, user.clone());
              }
              Self::clear_input(&this.requested_reviewer_input, window, cx);
              this.people_mutation_error = None;
            }
            Err(error) => {
              this.people_mutation_error = Some(error.to_string().into());
            }
          }
          this.people_mutation_loading = false;
          this.people_mutation_task = None;
          cx.notify();
        });
      });
    });

    self.people_mutation_task = Some(task);
  }

  pub(super) fn remove_requested_reviewer(&mut self, login: &str, cx: &mut Context<Self>) {
    if self.people_mutation_loading {
      return;
    }
    let Some(pull_request) = self.pull_request.as_ref() else {
      self.people_mutation_error = Some("No pull request selected".into());
      return;
    };
    let login = login.trim();
    if login.is_empty() {
      return;
    }

    self.people_mutation_loading = true;
    self.people_mutation_error = None;
    let api = self.api.clone();
    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let login_string = login.to_string();
    let login_for_request = login_string.clone();

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          api.remove_pull_request_reviewer(&owner, &repo, number, &login_for_request)
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            if let Some(pr) = this.pull_request.as_mut() {
              remove_filter_option_user(&mut pr.requested_reviewers, &login_string);
            }
            this.people_mutation_error = None;
          }
          Err(error) => {
            this.people_mutation_error = Some(error.to_string().into());
          }
        }
        this.people_mutation_loading = false;
        this.people_mutation_task = None;
        cx.notify();
      });
    });

    self.people_mutation_task = Some(task);
  }

  pub(super) fn add_label_value(
    &mut self,
    raw_value: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.label_mutation_loading {
      return;
    }
    let Some(pull_request) = self.pull_request.as_ref() else {
      self.label_mutation_error = Some("No pull request selected".into());
      return;
    };
    let label = raw_value.trim();
    if label.is_empty() {
      return;
    }
    if labels_contains(&pull_request.labels, label) {
      Self::clear_input(&self.label_input, window, cx);
      return;
    }

    self.label_mutation_loading = true;
    self.label_mutation_error = None;
    let api = self.api.clone();
    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let label_string = label.to_string();
    let label_for_request = label_string.clone();
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          api.add_pull_request_label(&owner, &repo, number, &label_for_request)
        })
        .await;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          match result {
            Ok(()) => {
              if let Some(pr) = this.pull_request.as_mut() {
                upsert_label(
                  &mut pr.labels,
                  GithubPullRequestLabel {
                    name: label_string.clone(),
                    color: None,
                  },
                );
              }
              Self::clear_input(&this.label_input, window, cx);
              this.label_mutation_error = None;
              this.refresh_pull_request_details_for_current_context(cx);
            }
            Err(error) => {
              this.label_mutation_error = Some(error.to_string().into());
            }
          }
          this.label_mutation_loading = false;
          this.label_mutation_task = None;
          cx.notify();
        });
      });
    });

    self.label_mutation_task = Some(task);
  }

  pub(super) fn remove_label(&mut self, name: &str, cx: &mut Context<Self>) {
    if self.label_mutation_loading {
      return;
    }
    let Some(pull_request) = self.pull_request.as_ref() else {
      self.label_mutation_error = Some("No pull request selected".into());
      return;
    };
    let name = name.trim();
    if name.is_empty() {
      return;
    }

    self.label_mutation_loading = true;
    self.label_mutation_error = None;
    let api = self.api.clone();
    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let name_string = name.to_string();
    let name_for_request = name_string.clone();
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          api.remove_pull_request_label(&owner, &repo, number, &name_for_request)
        })
        .await;
      let _ = cx.update_window(window_handle, |_, _, cx| {
        let _ = this.update(cx, |this, cx| {
          match result {
            Ok(()) => {
              if let Some(pr) = this.pull_request.as_mut() {
                remove_label(&mut pr.labels, &name_string);
              }
              this.label_mutation_error = None;
              this.refresh_pull_request_details_for_current_context(cx);
            }
            Err(error) => {
              this.label_mutation_error = Some(error.to_string().into());
            }
          }
          this.label_mutation_loading = false;
          this.label_mutation_task = None;
          cx.notify();
        });
      });
    });

    self.label_mutation_task = Some(task);
  }
}
