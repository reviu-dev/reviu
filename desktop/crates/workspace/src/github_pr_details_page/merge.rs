//! Merging the pull request, the status actions around it (close, reopen, ready
//! for review) and the target branch it merges into.

use super::*;

impl GithubPrDetailsPage {
  pub(super) fn mark_merge_form_reset_pending(&mut self) {
    self.merge_form_reset_pending = true;
    self.merge_submit_error = None;
  }

  pub(super) fn sync_merge_method_with_readiness(&mut self) {
    let Some(readiness) = self.merge_readiness.as_ref() else {
      return;
    };

    let method_available = readiness.available_methods.contains(&self.merge_method);

    if !method_available {
      self.merge_method = readiness
        .default_method
        .or_else(|| readiness.available_methods.first().copied())
        .unwrap_or(GithubPullRequestMergeMethod::Merge);
    }
  }

  pub(super) fn reset_merge_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.merge_form_reset_pending = false;
    self.sync_merge_method_with_readiness();
    self.merge_submit_error = None;
    self.merge_commit_title_input.update(cx, |input, cx| {
      input.set_value("", window, cx);
    });
    self.merge_commit_message_input.update(cx, |input, cx| {
      input.set_value("", window, cx);
    });
  }

  pub(super) fn selected_merge_method(&self) -> Option<GithubPullRequestMergeMethod> {
    self.merge_readiness.as_ref().and_then(|readiness| {
      readiness
        .available_methods
        .contains(&self.merge_method)
        .then_some(self.merge_method)
    })
  }

  pub(super) fn fetch_merge_readiness_for_context(
    &mut self,
    owner: String,
    repo: String,
    number: u64,
    cx: &mut Context<Self>,
  ) {
    self.merge_readiness_loading = true;
    self.merge_readiness_error = None;
    self.merge_readiness = None;
    let merge_api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          merge_api.fetch_pull_request_merge_readiness(&owner, &repo, number)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(readiness) => {
            this.merge_readiness_loading = false;
            this.merge_readiness_error = None;
            this.merge_readiness = Some(readiness);
            this.sync_merge_method_with_readiness();
            this.add_pr_breadcrumb("Load PR merge readiness succeeded", Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            this.merge_readiness_loading = false;
            this.merge_readiness_error = Some(error_message.clone().into());
            this.merge_readiness = None;
            this.add_pr_breadcrumb("Load PR merge readiness failed", Map::new());
            this.record_pr_error(
              "github.pr.merge_readiness",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        this.merge_readiness_task = None;
        cx.notify();
      });
    });
    self.merge_readiness_task = Some(task);
  }

  pub(super) fn submit_pull_request_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.merge_submit_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      self.merge_submit_error = Some("No pull request selected".into());
      return;
    };

    let Some(readiness) = self.merge_readiness.as_ref() else {
      self.merge_submit_error = Some("Merge readiness is not available yet.".into());
      return;
    };

    let Some(method) = self.selected_merge_method() else {
      self.merge_submit_error = Some("No merge method is available.".into());
      return;
    };

    if !readiness.can_merge_now {
      self.merge_submit_error = Some(readiness.message.clone().into());
      return;
    }

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let expected_head_sha = readiness.current_head_sha.clone();
    let commit_title = self.merge_commit_title_input.read(cx).value().to_string();
    let commit_message = self.merge_commit_message_input.read(cx).value().to_string();
    let api = self.api.clone();
    self.merge_submit_loading = true;
    self.merge_submit_error = None;

    let task = cx.spawn_in(window, async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          api.merge_pull_request(
            &owner,
            &repo,
            number,
            method,
            &expected_head_sha,
            Some(commit_title.as_str()),
            Some(commit_message.as_str()),
          )
        })
        .await;

      let _ = this.update_in(cx, |this, window, cx| {
        this.merge_submit_loading = false;
        match result {
          Ok(GithubPullRequestMergeResult { merged: true, .. }) => {
            this.merge_popover_open = false;
            this.mark_merge_form_reset_pending();
            this.refocus_page_shortcuts(window, cx);
            this.add_pr_breadcrumb("Merge pull request succeeded", Map::new());
            this.reload_current_pull_request(cx);
            cx.refresh_windows();
          }
          Ok(result) => {
            this.merge_submit_error = Some(result.message.into());
          }
          Err(error) => {
            let should_reload_merge_readiness = error
              .downcast_ref::<ApiError>()
              .and_then(ApiError::status_code_u16)
              .is_some_and(|status| status == 405 || status == 409);
            let error_message = error.to_string();
            this.merge_submit_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Merge pull request failed", Map::new());
            this.record_pr_error("github.pr.merge", error_message.as_str(), Map::new());
            if should_reload_merge_readiness {
              this.reload_merge_readiness_for_current_pull_request(cx);
            }
          }
        }
        cx.notify();
      });
    });

    self.merge_submit_task = Some(task);
  }

  pub(super) fn is_current_user_pr_author(&self, cx: &App) -> bool {
    let Some(pull_request) = self.pull_request.as_ref() else {
      return false;
    };
    let Some(login) = Self::current_github_login(cx) else {
      return false;
    };

    pull_request
      .author
      .login
      .eq_ignore_ascii_case(login.as_str())
  }

  pub(super) fn pull_request_status_action(&self) -> Option<GithubPrStatusAction> {
    let pull_request = self.pull_request.as_ref()?;
    if !matches!(pull_request.state, GithubPullRequestState::Open) {
      return None;
    }

    Some(if pull_request.draft {
      GithubPrStatusAction::ReadyForReview
    } else {
      GithubPrStatusAction::ConvertToDraft
    })
  }

  pub(super) fn push_pr_status_action_error_notification(
    &self,
    title: impl Into<SharedString>,
    error: SharedString,
    cx: &mut Context<Self>,
  ) {
    let title = title.into();
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      window.push_notification(
        Notification::error(error)
          .id::<GithubPrStatusActionNotificationId>()
          .title(title),
        cx,
      );
    });
  }

  pub(super) fn local_draft_merge_readiness(&self) -> Option<GithubPullRequestMergeReadiness> {
    let pull_request = self.pull_request.as_ref()?;
    let existing = self.merge_readiness.as_ref();

    Some(GithubPullRequestMergeReadiness {
      status: GithubPullRequestMergeReadinessStatus::Draft,
      message: "This pull request is still marked as a draft.".to_string(),
      current_head_sha: pull_request.head_sha.clone(),
      available_methods: Vec::new(),
      default_method: None,
      can_merge_now: false,
      viewer_can_merge: existing
        .map(|readiness| readiness.viewer_can_merge)
        .unwrap_or(true),
      mergeable_state: Some("draft".to_string()),
      rebaseable: existing.and_then(|readiness| readiness.rebaseable),
    })
  }

  pub(super) fn apply_pull_request_status_action_success(
    &mut self,
    action: GithubPrStatusAction,
    cx: &mut Context<Self>,
  ) {
    let Some(pull_request) = self.pull_request.as_mut() else {
      return;
    };

    pull_request.draft = matches!(action, GithubPrStatusAction::ConvertToDraft);

    match action {
      GithubPrStatusAction::ReadyForReview => {
        self.merge_popover_open = false;
        self.mark_merge_form_reset_pending();
        self.reload_merge_readiness_for_current_pull_request(cx);
      }
      GithubPrStatusAction::ConvertToDraft => {
        self.merge_popover_open = false;
        self.mark_merge_form_reset_pending();
        self.merge_readiness_loading = false;
        self.merge_readiness_error = None;
        self.merge_readiness_task = None;
        self.merge_readiness = self.local_draft_merge_readiness();
        self.sync_merge_method_with_readiness();
      }
    }
  }

  pub(super) fn submit_pull_request_status_action(
    &mut self,
    action: GithubPrStatusAction,
    cx: &mut Context<Self>,
  ) {
    if self.status_action_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let pull_request_id = pull_request.node_id.clone();
    let api = self.api.clone();
    self.status_action_loading = true;

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          match action {
            GithubPrStatusAction::ReadyForReview => {
              api.mark_pull_request_ready_for_review(&owner, &repo, number, &pull_request_id)
            }
            GithubPrStatusAction::ConvertToDraft => {
              api.convert_pull_request_to_draft(&owner, &repo, number, &pull_request_id)
            }
          }
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.status_action_loading = false;

        match result {
          Ok(()) => {
            this.add_pr_breadcrumb(action.success_breadcrumb(), Map::new());
            this.apply_pull_request_status_action_success(action, cx);
            cx.refresh_windows();
          }
          Err(error) => {
            let error_message = error.to_string();
            this.add_pr_breadcrumb(action.failure_breadcrumb(), Map::new());
            this.record_pr_error(
              action.sentry_operation(),
              error_message.as_str(),
              Map::new(),
            );
            this.push_pr_status_action_error_notification(
              action.error_title(),
              error_message.into(),
              cx,
            );
          }
        }

        cx.notify();
      });
    });

    self.status_action_task = Some(task);
    cx.notify();
  }

  pub(super) fn submit_update_branch(&mut self, cx: &mut Context<Self>) {
    if self.update_branch_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let api = self.api.clone();
    self.update_branch_loading = true;

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move { api.update_pull_request_branch(&owner, &repo, number) })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.update_branch_loading = false;

        match result {
          Ok(()) => {
            this.refresh_current_page(cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            let is_already_up_to_date = error_message
              .to_ascii_lowercase()
              .contains("no new commits");
            let _ = cx.update_window(this.window_handle, move |_, window, cx| {
              if is_already_up_to_date {
                window.push_notification(Notification::info("Branch is already up to date"), cx);
              } else {
                window.push_notification(
                  Notification::error(SharedString::from(error_message))
                    .title("Update branch failed"),
                  cx,
                );
              }
            });
          }
        }

        cx.notify();
      });
    });

    self.update_branch_task = Some(task);
    cx.notify();
  }

  pub(super) fn can_edit_target_branch(&self, pr: &GithubPullRequestDetails) -> bool {
    pr.state == GithubPullRequestState::Open
      && pr.merged_at.is_none()
      && !self.target_branch_update_loading
  }

  pub(super) fn refresh_target_branch_options(&mut self, cx: &mut Context<Self>) {
    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let current_base = pull_request.base_ref_name.clone();
    let head_ref = pull_request.head_ref_name.clone();
    self.target_branch_loading = true;
    self.target_branch_error = None;
    self.target_branch_request_generation = self.target_branch_request_generation.wrapping_add(1);
    let request_generation = self.target_branch_request_generation;
    let api = self.api.clone();

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move { api.fetch_github_repository_branches(&owner, &repo) })
        .await;

      let _ = this.update(cx, |this, cx| {
        if request_generation != this.target_branch_request_generation {
          return;
        }

        this.target_branch_task = None;
        this.target_branch_loading = false;

        match result {
          Ok(branches) => {
            let items = build_target_branch_select_items(branches, &current_base, &head_ref);
            this.set_target_branch_select_items(items, Some(current_base), cx);
            this.target_branch_error = None;
          }
          Err(error) => {
            let message = error.to_string();
            this.set_target_branch_select_items(Vec::new(), None, cx);
            this.target_branch_error = Some(message.clone().into());
            this.record_pr_error("github.pr.target_branch_options", &message, Map::new());
          }
        }

        cx.notify();
      });
    });

    self.target_branch_task = Some(task);
    cx.notify();
  }

  pub(super) fn set_target_branch_select_items(
    &mut self,
    items: Vec<PrTargetBranchSelectItem>,
    selected_branch: Option<String>,
    cx: &mut Context<Self>,
  ) {
    let target_branch_select = self.target_branch_select.clone();
    let window_handle = self.window_handle;
    let _ = cx.update_window(window_handle, move |_, window, cx| {
      target_branch_select.update(cx, |state, cx| {
        state.set_items(SearchableVec::new(items), window, cx);
        if let Some(selected_branch) = selected_branch.as_ref() {
          state.set_selected_value(selected_branch, window, cx);
        } else {
          state.set_selected_index(None, window, cx);
        }
      });
    });
  }

  pub(super) fn submit_target_branch_update(&mut self, branch: String, cx: &mut Context<Self>) {
    if self.target_branch_update_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    if branch.eq_ignore_ascii_case(pull_request.base_ref_name.as_str()) {
      return;
    }

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let api = self.api.clone();
    self.target_branch_update_loading = true;
    self.target_branch_update_error = None;

    let task = cx.spawn(async move |this, cx| {
      let branch_for_request = branch.clone();
      let result = cx
        .background_spawn(async move {
          api.update_pull_request_base(&owner, &repo, number, &branch_for_request)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.target_branch_update_loading = false;
        this.target_branch_update_task = None;

        match result {
          Ok(()) => {
            this.target_branch_update_error = None;
            this.add_pr_breadcrumb("Update PR target branch succeeded", Map::new());
            if let Some(pull_request) = this.pull_request.as_mut() {
              pull_request.base_ref_name = branch;
            }
            this.reload_current_pull_request(cx);
          }
          Err(error) => {
            let message = error.to_string();
            this.target_branch_update_error = Some(message.clone().into());
            this.add_pr_breadcrumb("Update PR target branch failed", Map::new());
            this.record_pr_error("github.pr.target_branch_update", &message, Map::new());
            let restore_base = this
              .pull_request
              .as_ref()
              .map(|pr| pr.base_ref_name.clone());
            if let Some(base) = restore_base {
              let select = this.target_branch_select.clone();
              let window_handle = this.window_handle;
              let _ = cx.update_window(window_handle, move |_, window, cx| {
                select.update(cx, |state, cx| {
                  state.set_selected_value(&base, window, cx);
                });
              });
            }
          }
        }

        cx.notify();
      });
    });

    self.target_branch_update_task = Some(task);
    cx.notify();
  }

  pub(super) fn can_submit_merge_from_input(&self) -> bool {
    if !self.merge_popover_open || self.merge_submit_loading {
      return false;
    }
    self.merge_readiness.as_ref().is_some_and(|readiness| {
      readiness.can_merge_now
        && self
          .selected_merge_method()
          .is_some_and(|method| readiness.available_methods.contains(&method))
    })
  }

  pub(super) fn subscribe_to_merge_commit_inputs(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let title_input = self.merge_commit_title_input.clone();
    let message_input = self.merge_commit_message_input.clone();
    cx.subscribe_in(
      &title_input,
      window,
      |this, state, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter {
          secondary: true, ..
        } = event
        {
          if !this.can_submit_merge_from_input() {
            return;
          }
          let raw = state.read(cx).value().to_string();
          let trimmed = raw.trim_end_matches('\n').to_string();
          if trimmed != raw {
            state.update(cx, |input, cx| {
              input.set_value(trimmed, window, cx);
            });
          }
          this.submit_pull_request_merge(window, cx);
        }
      },
    )
    .detach();
    cx.subscribe_in(
      &message_input,
      window,
      |this, state, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter {
          secondary: true, ..
        } = event
        {
          if !this.can_submit_merge_from_input() {
            return;
          }
          let raw = state.read(cx).value().to_string();
          let trimmed = raw.trim_end_matches('\n').to_string();
          if trimmed != raw {
            state.update(cx, |input, cx| {
              input.set_value(trimmed, window, cx);
            });
          }
          this.submit_pull_request_merge(window, cx);
        }
      },
    )
    .detach();
  }

  pub(super) fn render_merge_popover(
    &mut self,
    theme: &gpui_component::Theme,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let merge_readiness = self.merge_readiness.clone();
    let merge_status = merge_readiness
      .as_ref()
      .map(|readiness| readiness.status)
      .unwrap_or(GithubPullRequestMergeReadinessStatus::Checking);
    let available_methods = merge_readiness
      .as_ref()
      .map(|readiness| readiness.available_methods.clone())
      .unwrap_or_default();
    let selected_method = self.selected_merge_method();
    let selected_method_index = selected_method.and_then(|method| {
      available_methods
        .iter()
        .position(|candidate| *candidate == method)
    });
    let can_submit_merge = self.merge_readiness.as_ref().is_some_and(|readiness| {
      readiness.can_merge_now
        && readiness
          .available_methods
          .iter()
          .any(|method| Some(*method) == selected_method)
    });
    let show_commit_fields = selected_method.is_some_and(merge_method_supports_commit_message);
    let merge_button_disabled = self.pull_request.is_none();
    let merge_message = self
      .merge_submit_error
      .clone()
      .or_else(|| self.merge_readiness_error.clone())
      .or_else(|| {
        merge_readiness
          .as_ref()
          .map(|readiness| readiness.message.clone().into())
      });

    Popover::new("pr-merge-popover")
      .anchor(Anchor::TopRight)
      .w(px(PR_MERGE_POPOVER_WIDTH))
      .open(self.merge_popover_open)
      .on_open_change(cx.listener(|this, open, window, cx| {
        this.merge_popover_open = *open;
        if *open && this.merge_form_reset_pending {
          this.reset_merge_form(window, cx);
        }
        cx.notify();
      }))
      .trigger(
        Button::new("pr-merge-button")
          .with_variant(ButtonVariant::Secondary)
          .outline()
          .child(
            h_flex()
              .items_center()
              .child(Icon::new(UiIconName::GitMerge).size_3p5().mr_1p5())
              .child("Merge")
              .child(Icon::new(IconName::ChevronDown).size_3p5().ml_2()),
          )
          .small()
          .disabled(merge_button_disabled),
      )
      .child(
        v_flex()
          .id("pr-merge-popover-content")
          .w_full()
          .gap_3()
          .child(
            div()
              .text_sm()
              .font_medium()
              .text_color(theme.foreground)
              .child("Merge pull request"),
          )
          .when(self.merge_readiness_loading && self.merge_readiness.is_none(), |this| {
            this.child(
              h_flex()
                .items_center()
                .gap_2()
                .child(Spinner::new().small())
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Checking merge readiness..."),
                ),
            )
          })
          .when(
            !available_methods.is_empty()
              && matches!(
                merge_status,
                GithubPullRequestMergeReadinessStatus::Ready
                  | GithubPullRequestMergeReadinessStatus::Blocked
              ),
            |this| {
              let methods_for_click = available_methods.clone();
              let mut group = RadioGroup::vertical("pr-merge-method-group")
                .selected_index(selected_method_index)
                .on_click(cx.listener(move |this, index: &usize, _, cx| {
                  if let Some(method) = methods_for_click.get(*index).copied() {
                    this.merge_method = method;
                    this.merge_submit_error = None;
                    cx.notify();
                  }
                }));

              for method in &available_methods {
                let id = match method {
                  GithubPullRequestMergeMethod::Merge => "pr-merge-method-merge",
                  GithubPullRequestMergeMethod::Squash => "pr-merge-method-squash",
                  GithubPullRequestMergeMethod::Rebase => "pr-merge-method-rebase",
                };
                group = group.child(Radio::new(id).label(merge_method_label(*method)));
              }

              let focus_handle = self.merge_method_focus_handle.clone();
              let is_focused = focus_handle.is_focused(window);
              let ring_color = if is_focused {
                theme.ring
              } else {
                gpui::transparent_black()
              };
              this
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Choose how GitHub should merge this pull request."),
                )
                .child(
                  div()
                    .track_focus(&focus_handle)
                    .p_1()
                    .rounded(theme.radius)
                    .border_2()
                    .border_color(ring_color)
                    .child(group),
                )
            },
          )
          .when(show_commit_fields, |this| {
            this
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child("Leave the fields empty to let GitHub generate the default commit title and message."),
              )
              .child(
                div()
                  .w_full()
                  .debug_selector(|| "github-pr-merge-title-input".to_string())
                  .child(Input::new(&self.merge_commit_title_input).w_full()),
              )
              .child(
                div()
                  .w_full()
                  .debug_selector(|| "github-pr-merge-message-input".to_string())
                  .child(
                    Textarea::new(&self.merge_commit_message_input)
                      .w_full()
                      .h(px(PR_MERGE_MESSAGE_INPUT_HEIGHT_PX)),
                  ),
              )
          })
          .when_some(merge_message, |this, message| {
            let color = if self.merge_submit_error.is_some() || self.merge_readiness_error.is_some()
            {
              theme.status_red()
            } else if matches!(merge_status, GithubPullRequestMergeReadinessStatus::Ready) {
              theme.muted_foreground
            } else {
              theme.status_orange()
            };

            this.child(
              div()
                .text_xs()
                .text_color(color)
                .child(message),
            )
          })
          .child(
            h_flex()
              .items_center()
              .justify_end()
              .gap_2()
              .child(
                Button::new("pr-merge-cancel")
                  .ghost()
                  .small()
                  .label("Cancel")
                  .disabled(self.merge_submit_loading)
                  .on_click(cx.listener(|this, _, window, cx| {
                    this.merge_popover_open = false;
                    this.reset_merge_form(window, cx);
                    cx.notify();
                  })),
              )
              .child(
                Button::new("pr-merge-submit")
                  .primary()
                  .small()
                  .label("Merge pull request")
                  .child(Kbd::new(Keystroke::parse("cmd-enter").unwrap()).ml_1())
                  .loading(self.merge_submit_loading)
                  .disabled(!can_submit_merge)
                  .on_click(cx.listener(|this, _, window, cx| {
                    this.submit_pull_request_merge(window, cx);
                  })),
              ),
          ),
      )
      .into_any_element()
  }

  pub(super) fn render_pr_actions_menu(&self, cx: &mut Context<Self>) -> AnyElement {
    let status_action = self.pull_request_status_action();
    let status_loading = self.status_action_loading;
    let update_branch_loading = self.update_branch_loading;
    let has_conflicts = self
      .merge_readiness
      .as_ref()
      .and_then(|r| r.mergeable_state.as_deref())
      .is_some_and(|s| s.trim().eq_ignore_ascii_case("dirty"));
    let is_open = self
      .pull_request
      .as_ref()
      .is_some_and(|pr| matches!(pr.state, GithubPullRequestState::Open));
    let view = cx.entity().clone();
    let pr_url = self
      .pull_request
      .as_ref()
      .map(|pr| github_shared::pr_url(&pr.repository.owner, &pr.repository.repo, pr.number));

    Button::new("pr-actions-menu")
      .icon(UiIconName::EllipsisVertical)
      .ghost()
      .small()
      .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
        let view = view.clone();
        let mut menu = menu;

        if let Some(url) = pr_url.clone() {
          menu = menu.item(
            PopupMenuItem::new("View on GitHub")
              .icon(Icon::new(IconName::ExternalLink))
              .on_click(move |_, _, cx| {
                cx.open_url(&url);
              }),
          );
        }

        if is_open {
          let update_view = view.clone();

          menu = menu.item(
            PopupMenuItem::new("Update branch")
              .icon(Icon::new(UiIconName::GitMerge))
              .disabled(update_branch_loading || has_conflicts)
              .on_click(move |_, _, cx| {
                update_view.update(cx, |this, cx| {
                  this.submit_update_branch(cx);
                });
              }),
          );
        }

        if let Some(action) = status_action {
          let icon = match action {
            GithubPrStatusAction::ConvertToDraft => UiIconName::GitPullRequestDraft,
            GithubPrStatusAction::ReadyForReview => UiIconName::GitPullRequestArrow,
          };
          menu = menu.item(
            PopupMenuItem::new(action.button_label())
              .icon(Icon::new(icon))
              .disabled(status_loading)
              .on_click(move |_, _, cx| {
                view.update(cx, |this, cx| {
                  this.submit_pull_request_status_action(action, cx);
                });
              }),
          );
        }

        menu
      })
      .into_any_element()
  }

  pub(super) fn is_pull_request_merged(&self) -> bool {
    self
      .pull_request
      .as_ref()
      .is_some_and(|pull_request| pull_request.merged_at.is_some())
  }

  pub(super) fn reload_merge_readiness_for_current_pull_request(&mut self, cx: &mut Context<Self>) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };
    self.fetch_merge_readiness_for_context(context.owner, context.repo, context.number, cx);
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::super::*;

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
}
