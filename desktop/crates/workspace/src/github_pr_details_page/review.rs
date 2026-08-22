//! Reviewing the pull request: the review form and its decision, the comments
//! on the diff, and committing a suggested change.

use super::*;

fn code_reference_requests_from_markdown(markdown: &str) -> Vec<GithubBlobLineReference> {
  extract_github_blob_line_references(markdown)
}

fn next_review_comment_navigation_index(
  comment_ids: &[u64],
  active_comment_id: Option<u64>,
  direction: ReviewCommentNavigationDirection,
) -> Option<usize> {
  if comment_ids.is_empty() {
    return None;
  }

  let active_index =
    active_comment_id.and_then(|id| comment_ids.iter().position(|value| *value == id));

  Some(match direction {
    ReviewCommentNavigationDirection::Next => active_index
      .map(|ix| (ix + 1) % comment_ids.len())
      .unwrap_or(0),
    ReviewCommentNavigationDirection::Previous => active_index
      .map(|ix| {
        if ix == 0 {
          comment_ids.len() - 1
        } else {
          ix - 1
        }
      })
      .unwrap_or(comment_ids.len() - 1),
  })
}

fn parse_github_commit_url(url: &str) -> Option<(String, String, String)> {
  let url = url.trim();
  let tail = url
    .strip_prefix("https://github.com/")
    .or_else(|| url.strip_prefix("http://github.com/"))
    .or_else(|| url.strip_prefix("github.com/"))?;
  let tail = tail
    .split('#')
    .next()
    .unwrap_or(tail)
    .split('?')
    .next()
    .unwrap_or(tail);

  let parts = tail
    .split('/')
    .map(str::trim)
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>();
  if parts.len() < 4 {
    return None;
  }

  let owner = parts[0].to_string();
  let repo = parts[1].to_string();
  let sha = match parts[2] {
    "commit" => parts.get(3)?,
    "pull" if parts.get(4).copied() == Some("commits") => parts.get(5)?,
    _ => return None,
  };

  Some((owner, repo, (*sha).to_string()))
}

fn resolve_same_pr_commit_link_sha(
  current_pr_context: Option<&CurrentPrContext>,
  commits: &[GithubPullRequestCommit],
  url: &str,
) -> Option<String> {
  let (owner, repo, linked_sha) = parse_github_commit_url(url)?;
  let context = current_pr_context?;
  if !context.owner.eq_ignore_ascii_case(&owner) || !context.repo.eq_ignore_ascii_case(&repo) {
    return None;
  }

  let linked_sha = linked_sha.trim();
  if linked_sha.is_empty() {
    return None;
  }

  if let Some(commit) = commits
    .iter()
    .find(|commit| commit.sha.eq_ignore_ascii_case(linked_sha))
  {
    return Some(commit.sha.clone());
  }

  let linked_sha = linked_sha.to_ascii_lowercase();
  let mut matches = commits.iter().filter(|commit| {
    commit
      .sha
      .to_ascii_lowercase()
      .starts_with(linked_sha.as_str())
  });
  let first_match = matches.next()?;
  if matches.next().is_some() {
    return None;
  }

  Some(first_match.sha.clone())
}

fn normalize_line_range(start: Option<i64>, end: Option<i64>) -> Option<(usize, usize)> {
  let start = positive_line_number(start);
  let end = positive_line_number(end);
  let (start, end) = match (start, end) {
    (Some(start), Some(end)) => (start, end),
    (Some(start), None) => (start, start),
    (None, Some(end)) => (end, end),
    (None, None) => return None,
  };

  Some(if start <= end {
    (start, end)
  } else {
    (end, start)
  })
}

fn review_comment_preview_line_range(
  comment: &GithubPullRequestReviewComment,
) -> Option<(usize, usize)> {
  normalize_line_range(
    comment.start_line.or(comment.line),
    comment.line.or(comment.start_line),
  )
  .or_else(|| {
    normalize_line_range(
      comment.original_start_line.or(comment.original_line),
      comment.original_line.or(comment.original_start_line),
    )
  })
}

fn review_comment_targets_file(
  comment: &GithubPullRequestReviewComment,
  file: &GithubPrFileDiff,
) -> bool {
  comment.path == file.path
    || file
      .old_path
      .as_ref()
      .is_some_and(|old_path| old_path.as_ref() == comment.path)
}

fn review_comment_to_editor_comment(
  comment: &GithubPullRequestReviewComment,
  comments_by_id: &HashMap<u64, &GithubPullRequestReviewComment>,
) -> Option<ReviewComment> {
  let (line, side, resolved_line) = resolve_review_comment_display_anchor(comment, comments_by_id)?;

  let line_label = {
    let line_label = if let Some(start) = comment.start_line
      && let Some(end) = comment.line
      && start != end
    {
      Some(format!("L{}-{}", start, end))
    } else {
      comment
        .line
        .or(comment.start_line)
        .or(resolved_line)
        .map(|value| format!("L{}", value))
    };
    line_label.map(|label| Arc::from(label.as_str()))
  };

  Some(ReviewComment {
    id: comment.id,
    in_reply_to_id: comment.in_reply_to_id,
    line,
    side,
    author: Arc::from(comment.user.login.as_str()),
    avatar_url: comment.user.avatar_url.as_deref().map(Arc::from),
    line_label,
    body: Arc::from(comment.body.as_str()),
    suggestion_context: suggestion_context_from_review_comment(comment),
    created_at: Arc::from(format_relative_time(&comment.created_at).to_string()),
    thread_id: (!comment.thread_id.is_empty())
      .then(|| Arc::<str>::from(comment.thread_id.as_str())),
    is_resolved: comment.is_resolved,
    is_outdated: comment.is_outdated,
    viewer_can_resolve: comment.viewer_can_resolve,
    viewer_can_unresolve: comment.viewer_can_unresolve,
    is_pending: comment.is_pending,
  })
}

fn resolve_review_comment_thread_root_id(
  comment: &GithubPullRequestReviewComment,
  comments_by_id: &HashMap<u64, &GithubPullRequestReviewComment>,
) -> u64 {
  let mut root_id = comment.id;
  let mut parent = comment.in_reply_to_id;
  for _ in 0..64 {
    let Some(parent_id) = parent else {
      break;
    };
    if parent_id == root_id {
      break;
    }
    root_id = parent_id;
    parent = comments_by_id
      .get(&parent_id)
      .and_then(|value| value.in_reply_to_id);
  }
  if comments_by_id.contains_key(&root_id) {
    root_id
  } else {
    comment.id
  }
}

fn overview_root_review_comment_ids(
  review_comments: &[GithubPullRequestReviewComment],
) -> Vec<u64> {
  let comments_by_id: HashMap<u64, &GithubPullRequestReviewComment> = review_comments
    .iter()
    .map(|comment| (comment.id, comment))
    .collect();
  let mut root_ids = Vec::new();
  let mut seen = HashSet::new();

  for comment in review_comments {
    let root_id = resolve_review_comment_thread_root_id(comment, &comments_by_id);
    if seen.insert(root_id) {
      root_ids.push(root_id);
    }
  }

  root_ids
}

fn suggestion_context_from_review_comment(
  comment: &GithubPullRequestReviewComment,
) -> Option<SuggestionContext> {
  let (_, line) = review_comment_preview_line_range(comment)?;
  let start_line = comment
    .start_line
    .or(comment.line)
    .or(comment.original_start_line)
    .or(comment.original_line);
  let original_range = github_shared::extract_original_line_range_from_diff_hunk(
    &comment.diff_hunk,
    start_line,
    line as i64,
  )?;
  Some(SuggestionContext {
    original_start_line: Some(original_range.start_line),
    suggested_start_line: Some(original_range.start_line),
    original_lines: original_range.lines,
    path: Arc::from(comment.path.as_str()),
  })
}

fn review_comment_owned_by_login(comment: &GithubPullRequestReviewComment, login: &str) -> bool {
  github_shared::logins_match_case_insensitive(comment.user.login.as_str(), login)
}

fn upsert_review_local(
  reviews: &mut Vec<GithubPullRequestReview>,
  mut review: GithubPullRequestReview,
) {
  if let Some(existing) = reviews.iter_mut().find(|existing| existing.id == review.id) {
    if review.node_id.is_empty() {
      review.node_id.clone_from(&existing.node_id);
    }
    *existing = review;
    return;
  }

  reviews.push(review);
}

impl GithubPrDetailsPage {
  pub(super) fn review_decision_to_api_event(
    decision: GithubPrReviewDecision,
  ) -> GithubPullRequestReviewEvent {
    match decision {
      GithubPrReviewDecision::Comment => GithubPullRequestReviewEvent::Comment,
      GithubPrReviewDecision::Approve => GithubPullRequestReviewEvent::Approve,
      GithubPrReviewDecision::RequestChanges => GithubPullRequestReviewEvent::RequestChanges,
    }
  }

  pub(super) fn review_decision_requires_body(decision: GithubPrReviewDecision) -> bool {
    matches!(
      decision,
      GithubPrReviewDecision::Comment | GithubPrReviewDecision::RequestChanges
    )
  }

  pub(super) fn review_decision_from_index(index: usize) -> GithubPrReviewDecision {
    match index {
      1 => GithubPrReviewDecision::Approve,
      2 => GithubPrReviewDecision::RequestChanges,
      _ => GithubPrReviewDecision::Comment,
    }
  }

  pub(super) fn review_decision_index(decision: GithubPrReviewDecision) -> usize {
    match decision {
      GithubPrReviewDecision::Comment => 0,
      GithubPrReviewDecision::Approve => 1,
      GithubPrReviewDecision::RequestChanges => 2,
    }
  }

  pub(super) fn validate_review_submission(
    decision: GithubPrReviewDecision,
    body: &str,
  ) -> Option<SharedString> {
    if Self::review_decision_requires_body(decision) && body.trim().is_empty() {
      return Some("A review comment is required for this review type".into());
    }

    None
  }

  pub(super) fn mark_review_form_reset_pending(&mut self) {
    self.review_form_reset_pending = true;
    self.review_decision = GithubPrReviewDecision::Comment;
    self.submit_review_error = None;
  }

  pub(super) fn reset_review_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.review_form_reset_pending = false;
    self.review_decision = GithubPrReviewDecision::Comment;
    self.submit_review_error = None;
    self.review_preview_open = false;
    self.review_input.update(cx, |input, cx| {
      input.set_value("", window, cx);
    });
  }

  pub(super) fn focus_review_input(&self, window: &mut Window) {
    let review_input = self.review_input.clone();
    window.on_next_frame(move |window, cx| {
      review_input.update(cx, |input, cx| {
        input.focus(window, cx);
      });
    });
  }

  pub(super) fn subscribe_to_review_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    cx.subscribe_in(
      &self.review_input,
      window,
      |this, state, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter {
          secondary: true, ..
        } = event
        {
          if !this.review_popover_open || this.submit_review_loading || this.pull_request.is_none()
          {
            return;
          }
          if this.is_current_user_pr_author(cx)
            && !matches!(this.review_decision, GithubPrReviewDecision::Comment)
          {
            return;
          }
          let raw_body = state.read(cx).value().to_string();
          let trimmed = raw_body.trim_end_matches('\n').to_string();
          if trimmed != raw_body {
            state.update(cx, |input, cx| {
              input.set_value(trimmed.clone(), window, cx);
            });
          }
          if Self::validate_review_submission(this.review_decision, trimmed.as_str()).is_some() {
            return;
          }
          this.submit_pull_request_review(window, cx);
        }
      },
    )
    .detach();
  }

  pub(super) fn suggested_change_default_commit_title() -> &'static str {
    "Apply suggestion from code review"
  }

  pub(super) fn suggested_change_commit_available(&self) -> bool {
    self.selected_commit_sha.is_none()
      && self
        .pull_request
        .as_ref()
        .is_some_and(|pr| matches!(pr.state, GithubPullRequestState::Open))
  }

  pub(super) fn normalized_suggested_change_line(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
  }

  pub(super) fn suggested_change_original_lines_match_current_head(
    file_contents: &HashMap<String, GithubPrFileContents>,
    path: &str,
    original_start_line: Option<usize>,
    original_lines: &[String],
  ) -> Option<bool> {
    let original_start_line = original_start_line?;
    let contents = file_contents.get(path)?;
    let head = contents.head.as_ref()?;
    let normalized_head = head.replace("\r\n", "\n");
    let mut lines: Vec<&str> = normalized_head.split('\n').collect();
    if normalized_head.ends_with('\n') {
      lines.pop();
    }
    let start_index = original_start_line.checked_sub(1)?;
    let actual = lines.get(start_index..start_index + original_lines.len())?;

    Some(
      actual
        .iter()
        .map(|line| Self::normalized_suggested_change_line(line))
        .eq(
          original_lines
            .iter()
            .map(|line| Self::normalized_suggested_change_line(line.as_str())),
        ),
    )
  }

  pub(super) fn suggested_change_target_matches_context(
    target: &SuggestedChangeCommitTarget,
    comment_id: u64,
    context: &SuggestionActionContext,
  ) -> bool {
    target.comment_id == comment_id
      && target.path == context.path
      && Some(target.original_start_line) == context.original_start_line
      && target.original_lines == context.original_lines
      && target.suggested_lines == context.suggested_lines
  }

  pub(super) fn open_suggested_change_commit(
    &mut self,
    target: SuggestedChangeCommitTarget,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.suggested_change_commit_target = Some(target);
    self.suggested_change_include_co_author = true;
    self.suggested_change_commit_error = None;
    self
      .suggested_change_commit_title_input
      .update(cx, |input, cx| {
        input.set_value(Self::suggested_change_default_commit_title(), window, cx);
      });
    self
      .suggested_change_commit_message_input
      .update(cx, |input, cx| input.set_value("", window, cx));

    let title_input = self.suggested_change_commit_title_input.clone();
    window.on_next_frame(move |window, cx| {
      title_input.update(cx, |input, cx| input.focus(window, cx));
    });
    cx.notify();
  }

  pub(super) fn close_suggested_change_commit(&mut self, cx: &mut Context<Self>) {
    if self.suggested_change_commit_loading {
      return;
    }
    self.suggested_change_commit_target = None;
    self.suggested_change_commit_error = None;
    cx.notify();
  }

  pub(super) fn submit_suggested_change_commit(&mut self, cx: &mut Context<Self>) {
    if self.suggested_change_commit_loading {
      return;
    }

    let Some(target) = self.suggested_change_commit_target.clone() else {
      self.suggested_change_commit_error = Some("No suggested change selected.".into());
      cx.notify();
      return;
    };

    let Some(pull_request) = self.pull_request.as_ref() else {
      self.suggested_change_commit_error = Some("No pull request selected.".into());
      cx.notify();
      return;
    };

    if !matches!(pull_request.state, GithubPullRequestState::Open) {
      self.suggested_change_commit_error =
        Some("Suggested changes can only be committed on open pull requests.".into());
      cx.notify();
      return;
    }

    if self.selected_commit_sha.is_some() {
      self.suggested_change_commit_error =
        Some("Switch back to the latest changes before committing a suggestion.".into());
      cx.notify();
      return;
    }

    let commit_title = self
      .suggested_change_commit_title_input
      .read(cx)
      .value()
      .trim()
      .to_string();
    if commit_title.is_empty() {
      self.suggested_change_commit_error = Some("Commit title is required.".into());
      cx.notify();
      return;
    }

    let commit_message = self
      .suggested_change_commit_message_input
      .read(cx)
      .value()
      .to_string();
    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let expected_head_sha = pull_request.head_sha.clone();
    let include_co_author = self.suggested_change_include_co_author;
    let comment_id = target.comment_id;
    let modified_path = target.path.clone();
    let api = self.api.clone();

    self.suggested_change_commit_loading = true;
    self.suggested_change_commit_error = None;
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          api.apply_pull_request_suggested_change(
            &owner,
            &repo,
            number,
            target.comment_id,
            &commit_title,
            Some(commit_message.as_str()),
            &expected_head_sha,
            target.path.as_ref(),
            target.original_start_line,
            &target.original_lines,
            &target.suggested_lines,
            include_co_author,
            Some(target.author_login.as_ref()),
          )
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.suggested_change_commit_loading = false;
        this.suggested_change_commit_task = None;
        match result {
          Ok(_) => {
            this.suggested_change_commit_target = None;
            this.suggested_change_commit_error = None;
            this.add_pr_breadcrumb("Commit suggested change succeeded", Map::new());
            this.file_contents.remove(modified_path.as_ref());
            this.refresh_current_page(cx);
            cx.refresh_windows();
          }
          Err(error) => {
            let error_message = error.to_string();
            if error_message.contains("Suggested change no longer matches the file") {
              this.stale_suggested_change_comment_ids.insert(comment_id);
            }
            this.suggested_change_commit_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Commit suggested change failed", Map::new());
            this.record_pr_error(
              "github.pr.suggested_change.commit",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        cx.notify();
      });
    });

    self.suggested_change_commit_task = Some(task);
  }

  pub(super) fn suggested_change_commit_action_renderer(
    &self,
    page: Entity<Self>,
    comment_id: u64,
    author_login: Arc<str>,
    is_outdated: bool,
    _cx: &App,
  ) -> Arc<dyn Fn(SuggestionActionContext, &App) -> AnyElement + Send + Sync> {
    let can_commit = self.suggested_change_commit_available();
    let current_target = self.suggested_change_commit_target.clone();
    let title_input = self.suggested_change_commit_title_input.clone();
    let message_input = self.suggested_change_commit_message_input.clone();
    let include_co_author = self.suggested_change_include_co_author;
    let loading = self.suggested_change_commit_loading;
    let error = self.suggested_change_commit_error.clone();
    let file_contents = self.file_contents.clone();
    let locally_stale = self
      .stale_suggested_change_comment_ids
      .contains(&comment_id);

    Arc::new(move |context: SuggestionActionContext, cx: &App| {
      let theme = cx.theme().clone();
      let is_open = current_target.as_ref().is_some_and(|target| {
        Self::suggested_change_target_matches_context(target, comment_id, &context)
      });
      let has_line_anchor = context.original_start_line.is_some();
      let lines_match_current_head = Self::suggested_change_original_lines_match_current_head(
        &file_contents,
        context.path.as_ref(),
        context.original_start_line,
        &context.original_lines,
      )
      .unwrap_or(true);
      let suggestion_is_outdated = is_outdated || locally_stale || !lines_match_current_head;
      let can_submit = !loading
        && has_line_anchor
        && !suggestion_is_outdated
        && github_shared::normalize_non_empty_text(title_input.read(cx).value().as_str()).is_some();
      let disabled_reason = if suggestion_is_outdated {
        Some("Outdated suggestions cannot be applied.")
      } else if can_commit {
        None
      } else {
        Some("Switch back to the latest open pull request changes before committing a suggestion.")
      };

      div()
        .relative()
        .child({
          let button = Button::new(format!(
            "pr-commit-suggestion-{comment_id}-{}",
            context.original_start_line.unwrap_or_default()
          ))
          .xsmall()
          .compact()
          .label("Commit suggestion")
          .disabled(!can_commit || !has_line_anchor || suggestion_is_outdated || loading)
          .on_click({
            let page = page.clone();
            let author_login = author_login.clone();
            let context = context.clone();
            move |_, window, cx| {
              if is_open {
                page.update(cx, |this, cx| {
                  this.close_suggested_change_commit(cx);
                });
                return;
              }

              let Some(original_start_line) = context.original_start_line else {
                return;
              };
              let target = SuggestedChangeCommitTarget {
                comment_id,
                author_login: author_login.clone(),
                path: context.path.clone(),
                original_start_line,
                original_lines: context.original_lines.clone(),
                suggested_lines: context.suggested_lines.clone(),
              };
              page.update(cx, |this, cx| {
                this.open_suggested_change_commit(target, window, cx);
              });
            }
          });

          if let Some(disabled_reason) = disabled_reason {
            button.tooltip(disabled_reason)
          } else {
            button
          }
        })
        .when(is_open, |this| {
          this.child(deferred(
            v_flex()
              .id(format!("pr-commit-suggestion-popover-{comment_id}"))
              .absolute()
              .top_full()
              .right_0()
              .mt_1()
              .w(px(360.0))
              .p_3()
              .gap_3()
              .border_1()
              .border_color(theme.border)
              .rounded_md()
              .occlude()
              .bg(theme.popover)
              .text_color(theme.popover_foreground)
              .shadow_lg()
              .child(
                div()
                  .text_sm()
                  .font_medium()
                  .text_color(theme.foreground)
                  .child("Commit suggested change"),
              )
              .child(div().w_full().child(Input::new(&title_input).w_full()))
              .child(
                div()
                  .w_full()
                  .child(Textarea::new(&message_input).w_full().h(px(86.0))),
              )
              .child(
                Switch::new(format!("pr-commit-suggestion-coauthor-{comment_id}"))
                  .small()
                  .checked(include_co_author)
                  .label(format!("Co-authored-by @{}", author_login))
                  .disabled(loading)
                  .on_click({
                    let page = page.clone();
                    move |checked, _, cx| {
                      page.update(cx, |this, cx| {
                        this.suggested_change_include_co_author = *checked;
                        cx.notify();
                      });
                    }
                  }),
              )
              .when_some(error.clone(), |this, error| {
                this.child(div().text_xs().text_color(theme.status_red()).child(error))
              })
              .child(
                h_flex()
                  .items_center()
                  .justify_end()
                  .gap_2()
                  .child(
                    Button::new(format!("pr-commit-suggestion-cancel-{comment_id}"))
                      .ghost()
                      .xsmall()
                      .compact()
                      .label("Cancel")
                      .disabled(loading)
                      .on_click({
                        let page = page.clone();
                        move |_, _, cx| {
                          page.update(cx, |this, cx| {
                            this.close_suggested_change_commit(cx);
                          });
                        }
                      }),
                  )
                  .child(
                    Button::new(format!("pr-commit-suggestion-submit-{comment_id}"))
                      .primary()
                      .xsmall()
                      .compact()
                      .label("Commit")
                      .loading(loading)
                      .disabled(!can_submit)
                      .on_click({
                        let page = page.clone();
                        move |_, _, cx| {
                          page.update(cx, |this, cx| {
                            this.submit_suggested_change_commit(cx);
                          });
                        }
                      }),
                  ),
              ),
          ))
        })
        .into_any_element()
    })
  }

  pub(super) fn toggle_review_thread_resolution(
    &mut self,
    thread_id: String,
    root_comment_id: u64,
    currently_resolved: bool,
    cx: &mut Context<Self>,
  ) {
    if thread_id.is_empty() || self.resolve_thread_in_flight.contains(&thread_id) {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let api = self.api.clone();
    let target = thread_id.clone();

    self.resolve_thread_in_flight.insert(thread_id.clone());
    self.resolve_thread_errors.remove(&thread_id);
    self.expanded_resolved_threads.remove(&root_comment_id);
    cx.notify();

    let thread_id_for_result = thread_id.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          if currently_resolved {
            api.unresolve_pull_request_review_thread(&owner, &repo, number, target.as_str())
          } else {
            api.resolve_pull_request_review_thread(&owner, &repo, number, target.as_str())
          }
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.resolve_thread_in_flight.remove(&thread_id_for_result);
        this.resolve_thread_tasks.remove(&thread_id_for_result);
        let editor_thread_id = thread_id_for_result.clone();
        this.diff_editor.update(cx, |editor, cx| {
          editor.clear_review_comment_resolve_in_flight(editor_thread_id.as_str(), cx);
        });
        match result {
          Ok(()) => {
            this.resolve_thread_errors.remove(&thread_id_for_result);
            let breadcrumb = if currently_resolved {
              "Unresolve review thread succeeded"
            } else {
              "Resolve review thread succeeded"
            };
            this.add_pr_breadcrumb(breadcrumb, Map::new());
            this.refresh_pull_request_conversation_for_current_pull_request(false, cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            this
              .resolve_thread_errors
              .insert(thread_id_for_result.clone(), error_message.clone().into());
            let breadcrumb = if currently_resolved {
              "Unresolve review thread failed"
            } else {
              "Resolve review thread failed"
            };
            this.add_pr_breadcrumb(breadcrumb, Map::new());
            this.record_pr_error(
              "github.pr.review_thread.resolve",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        cx.notify();
      });
    });

    self.resolve_thread_tasks.insert(thread_id, task);
  }

  pub(super) fn submit_pull_request_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.submit_review_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      self.submit_review_error = Some("No pull request selected".into());
      cx.notify();
      return;
    };

    let body = self.review_input.read(cx).value().to_string();
    let decision = self.review_decision;
    let author_restricted_decision =
      self.is_current_user_pr_author(cx) && !matches!(decision, GithubPrReviewDecision::Comment);
    if author_restricted_decision {
      self.submit_review_error = Some(
        "Pull request authors cannot approve or request changes on their own pull requests.".into(),
      );
      cx.notify();
      return;
    }
    if let Some(error) = Self::validate_review_submission(decision, body.as_str()) {
      self.submit_review_error = Some(error);
      cx.notify();
      return;
    }

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let event = Self::review_decision_to_api_event(decision);
    let api = self.api.clone();
    let pending_review_id = self.pending_review_id.clone();

    self.submit_review_loading = true;
    self.submit_review_error = None;
    crate::analytics::track(cx, "github_pr_review_submitted");
    cx.notify();

    let task = cx.spawn_in(window, async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          if let Some(review_id) = pending_review_id {
            api.submit_pending_review(&owner, &repo, number, &review_id, event, &body)
          } else {
            api.submit_pull_request_review(&owner, &repo, number, event, &body)
          }
        })
        .await;

      let _ = this.update_in(cx, |this, window, cx| {
        this.submit_review_loading = false;

        match result {
          Ok(review) => {
            this.review_popover_open = false;
            this.pending_review_id = None;
            this.pending_review_pull_request_id = None;
            this.reset_review_form(window, cx);
            this.refocus_page_shortcuts(window, cx);
            upsert_review_local(&mut this.reviews, review);
            if let Some(pr) = this.pull_request.as_mut()
              && let Some(login) = Self::current_github_login(cx)
            {
              pr.requested_reviewers
                .retain(|r| !r.login.eq_ignore_ascii_case(&login));
            }
            this.refresh_pull_request_conversation_for_current_pull_request(false, cx);
            this.add_pr_breadcrumb("Submit PR review succeeded", Map::new());
            cx.refresh_windows();
          }
          Err(error) => {
            let error_message = error.to_string();
            this.submit_review_error = Some(error_message.clone().into());
            this.add_pr_breadcrumb("Submit PR review failed", Map::new());
            this.record_pr_error(
              "github.pr.review.submit",
              error_message.as_str(),
              Map::new(),
            );
          }
        }
        cx.notify();
      });
    });

    self.submit_review_task = Some(task);
  }

  pub(super) fn install_diff_editor_review_comment_handlers(&mut self, cx: &mut Context<Self>) {
    if self.selected_commit_sha.is_some() {
      return;
    }
    let view = cx.entity().downgrade();

    let edit: ReviewCommentEditHandler = Arc::new({
      let view = view.clone();
      move |comment_id, body, _window, cx| {
        let _ = view.update(cx, |this, cx| {
          this.submit_review_comment_edit(comment_id, body.as_ref().to_string(), cx);
        });
      }
    });

    let create: ReviewCommentCreateHandler = Arc::new({
      let view = view.clone();
      move |request, _window, cx| {
        let _ = view.update(cx, |this, cx| {
          this.submit_review_comment_create(request, cx);
        });
      }
    });

    let cancel: ReviewCommentCancelHandler = Arc::new({
      let view = view.clone();
      move |window, _cx| {
        let view = view.clone();
        window.on_next_frame(move |window, cx| {
          let _ = view.update(cx, |this, cx| {
            if this.active_tab_ix == PR_TAB_CHANGES_IX {
              this.focus_changes_tree(window, cx);
            }
          });
        });
      }
    });

    let delete: ReviewCommentDeleteHandler = Arc::new({
      let view = view.clone();
      move |comment_id, window, cx| {
        let _ = view.update(cx, |this, cx| {
          this.confirm_review_comment_delete(comment_id, window, cx);
        });
      }
    });

    let resolve: ReviewCommentResolveHandler = Arc::new({
      let view = view.clone();
      move |thread_id: Arc<str>, root_comment_id: u64, currently_resolved, _window, cx| {
        let _ = view.update(cx, |this, cx| {
          this.toggle_review_thread_resolution(
            thread_id.as_ref().to_string(),
            root_comment_id,
            currently_resolved,
            cx,
          );
        });
      }
    });

    let suggestion_action_factory: ReviewCommentSuggestionActionFactory = Arc::new({
      let view = view.clone();
      move |comment_id, author_login, is_outdated, cx| {
        let renderer = view.upgrade().map(|page| {
          page.read(cx).suggested_change_commit_action_renderer(
            page.clone(),
            comment_id,
            author_login,
            is_outdated,
            cx,
          )
        });
        match renderer {
          Some(renderer) => renderer,
          None => Arc::new(|_ctx, _cx| div().into_any_element()),
        }
      }
    });

    let link: ReviewCommentLinkHandler = Arc::new({
      let view = view.clone();
      move |url, window, cx| {
        view
          .update(cx, |this, cx| this.handle_gfm_link(url, window, cx))
          .unwrap_or(false)
      }
    });

    let image_upload: ReviewCommentImageUploadHandler = Arc::new({
      let view = view.clone();
      move |paths, input, window, cx| {
        let paths = paths.clone();
        let _ = view.update(cx, |this, cx| {
          this.handle_diff_editor_review_comment_drop(&paths, input, window, cx);
        });
      }
    });

    let preview_renderer: ReviewCommentPreviewRenderer = Arc::new({
      let view = view.clone();
      move |text: &str,
            suggestion_context: Option<SuggestionContext>,
            _window: &mut Window,
            cx: &mut App|
            -> AnyElement {
        let mut options = view
          .update(cx, |this, cx| {
            this.build_overview_composer_markdown_options(4_444, cx)
          })
          .unwrap_or_default();
        if let Some(mut ctx) = suggestion_context {
          if ctx.path.as_ref().is_empty()
            && let Some(path) = view
              .update(cx, |this, _| {
                this
                  .selected_file
                  .as_ref()
                  .map(|file| Arc::<str>::from(file.path.as_ref()))
              })
              .ok()
              .flatten()
          {
            ctx.path = path;
          }
          options = options.with_suggestion_context(ctx);
        }
        render_markdown(text, &options, cx)
      }
    });

    configure_review(
      &self.diff_editor.clone(),
      ReviewDestination::Github(Box::new(GithubReviewHandlers {
        create,
        edit,
        delete,
        cancel,
        resolve,
        asset_url_resolver: github_shared::make_asset_url_resolver(&self.api),
        preview_renderer: Some(preview_renderer),
        link: Some(link),
        image_upload: Some(image_upload),
        suggestion_action_factory: Some(suggestion_action_factory),
      })),
      cx,
    );
  }

  pub(super) fn sync_review_comment_handlers(&mut self, cx: &mut Context<Self>) {
    let should_enable = self.selected_commit_sha.is_none();
    if self.review_comment_handlers_enabled == should_enable {
      return;
    }

    self.review_comment_handlers_enabled = should_enable;
    if should_enable {
      self.install_diff_editor_review_comment_handlers(cx);
      return;
    }

    configure_review(&self.diff_editor.clone(), ReviewDestination::None, cx);
  }

  pub(super) fn handle_gfm_link(
    &mut self,
    url: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> bool {
    if should_open_externally(window) {
      return github_shared::try_open_github_asset_url(url, &self.api, cx);
    }

    if github_shared::try_open_github_asset_url(url, &self.api, cx) {
      return true;
    }

    if let Some(commit_sha) =
      resolve_same_pr_commit_link_sha(self.current_pr_context.as_ref(), &self.commits, url)
    {
      if self.active_tab_ix != PR_TAB_CHANGES_IX {
        self.set_active_tab(PR_TAB_CHANGES_IX, window, cx);
      }
      self.select_commit_filter(Some(commit_sha), cx);
      return true;
    }

    let Some(action) = parse_github_url_action(url) else {
      return false;
    };

    match action {
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
        open_changes_tab: _,
        review_comment_id,
      } => {
        let same_target = self.current_pr_context.as_ref().is_some_and(|context| {
          context.number == number
            && context.owner.eq_ignore_ascii_case(&owner)
            && context.repo.eq_ignore_ascii_case(&repo)
        });

        if same_target {
          match same_pr_gfm_navigation(self.active_tab_ix, review_comment_id) {
            SamePrGfmNavigation::ShowOverview { switch_to_overview } => {
              if switch_to_overview {
                self.set_active_tab(PR_TAB_OVERVIEW_IX, window, cx);
              }
              return true;
            }
            SamePrGfmNavigation::ScrollComment { switch_to_changes } => {
              if switch_to_changes {
                self.set_active_tab(PR_TAB_CHANGES_IX, window, cx);
              }
            }
          }
          return review_comment_id.is_some_and(|comment_id| {
            self.handle_review_comment_link_target(number, comment_id, cx)
          });
        }

        self.load_pull_request(
          owner.clone(),
          repo.clone(),
          number,
          GithubPrOpenTarget {
            open_changes_tab: review_comment_id.is_some(),
            review_comment_id,
          },
          cx,
        );
        NavigationHistory::navigate(crate::navigation::build_pr_path(&owner, &repo, number), cx);
        true
      }
      CommandPaletteAction::OpenGithubRepoDetails {
        owner,
        repo,
        tab,
        issue_number,
        issue_comment_id,
      } => {
        if tab == Some(ui::CommandPaletteGithubRepoTab::Issues)
          && let Some(issue_number) = issue_number
        {
          self.open_resolved_issue_reference(owner, repo, issue_number, issue_comment_id, cx);
          return true;
        }

        open_repo_target(owner, repo, tab, issue_number, issue_comment_id, cx);
        true
      }
      CommandPaletteAction::OpenGithubCommitDetails { owner, repo, sha } => {
        open_commit_target(owner, repo, sha, cx);
        true
      }
      CommandPaletteAction::OpenGithubProfile { login } => {
        open_profile_target(login, cx);
        true
      }
      _ => false,
    }
  }

  pub(super) fn open_resolved_issue_reference(
    &mut self,
    owner: String,
    repo: String,
    issue_number: u64,
    issue_comment_id: Option<u64>,
    cx: &mut Context<Self>,
  ) {
    let api = self.api.clone();
    let fallback_owner = owner.clone();
    let fallback_repo = repo.clone();
    cx.spawn(async move |_, cx| {
      let result = cx
        .background_spawn(async move {
          api.resolve_github_issue_reference_target(&owner, &repo, issue_number)
        })
        .await;

      cx.update(|cx| match result {
        Ok(target) if target.kind == GithubIssueReferenceTargetKind::PullRequest => {
          open_pr_target(
            fallback_owner,
            fallback_repo,
            target.number,
            false,
            None,
            cx,
          );
        }
        _ => {
          open_repo_target(
            fallback_owner,
            fallback_repo,
            Some(ui::CommandPaletteGithubRepoTab::Issues),
            Some(issue_number),
            issue_comment_id,
            cx,
          );
        }
      });
    })
    .detach();
  }

  pub(super) fn handle_review_comment_link_target(
    &mut self,
    pr_number: u64,
    comment_id: u64,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(pull_request) = self.pull_request.as_ref() else {
      return false;
    };
    if pull_request.number != pr_number {
      return false;
    }

    self.pending_review_comment_link_comment_id = Some(comment_id);
    self.resolve_pending_review_comment_link(cx);

    if self.pending_review_comment_link_comment_id == Some(comment_id)
      && !self.review_comments_loading
      && !self.files_loading
      && !self.file_loading
      && !self.file_lookup.is_empty()
    {
      self.pending_review_comment_link_comment_id = None;
      return false;
    }

    true
  }

  pub(super) fn file_for_review_comment_path(&self, path: &str) -> Option<Rc<GithubPrFileDiff>> {
    file_for_review_comment_path(&self.file_lookup, path)
  }

  pub(super) fn try_scroll_to_pending_review_comment(&mut self, cx: &mut Context<Self>) -> bool {
    let Some(comment_id) = self.pending_review_comment_link_comment_id else {
      return false;
    };

    let did_scroll = self.diff_editor.update(cx, |editor, cx| {
      editor.scroll_to_review_comment(comment_id, editor.measured_editor_line_height(), cx)
    });

    if did_scroll {
      self.pending_review_comment_link_comment_id = None;
      self.active_review_comment_id = Some(comment_id);
    }

    did_scroll
  }

  pub(super) fn resolve_pending_review_comment_link(&mut self, cx: &mut Context<Self>) {
    let Some(comment_id) = self.pending_review_comment_link_comment_id else {
      return;
    };

    let Some(comment_path) = self
      .review_comments
      .iter()
      .find(|comment| comment.id == comment_id)
      .map(|comment| comment.path.clone())
    else {
      return;
    };

    let Some(target_file) = self.file_for_review_comment_path(comment_path.as_str()) else {
      return;
    };

    let selected_matches_target = self
      .selected_file
      .as_ref()
      .is_some_and(|file| file.path == target_file.path);

    if !selected_matches_target {
      self.set_selected_file(Some(target_file), cx);
      return;
    }

    let _ = self.try_scroll_to_pending_review_comment(cx);
  }

  pub(super) fn navigate_review_comment(
    &mut self,
    direction: ReviewCommentNavigationDirection,
    cx: &mut Context<Self>,
  ) {
    let Some(index) = next_review_comment_navigation_index(
      &self.selected_file_review_comment_ids,
      self.active_review_comment_id,
      direction,
    ) else {
      return;
    };
    let Some(comment_id) = self.selected_file_review_comment_ids.get(index).copied() else {
      return;
    };

    let did_scroll = self.diff_editor.update(cx, |editor, cx| {
      editor.scroll_to_review_comment(comment_id, editor.measured_editor_line_height(), cx)
    });
    if !did_scroll {
      self.pending_review_comment_link_comment_id = Some(comment_id);
      self.resolve_pending_review_comment_link(cx);
    }
    self.active_review_comment_id = Some(comment_id);
    cx.notify();
  }

  pub(super) fn submit_review_comment_edit(
    &mut self,
    comment_id: u64,
    body: String,
    cx: &mut Context<Self>,
  ) {
    if self.selected_commit_sha.is_some() {
      let message = Arc::<str>::from("Review comments are disabled for commit-level diffs");
      self.review_comments_error = Some(message.to_string().into());
      self.diff_editor.update(cx, |editor, cx| {
        editor.finish_review_comment_edit_submission(comment_id, Some(message.clone()), cx);
      });
      cx.notify();
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      self.review_comments_error = Some("No pull request selected".into());
      self.diff_editor.update(cx, |editor, cx| {
        editor.finish_review_comment_edit_submission(
          comment_id,
          Some(Arc::from("No pull request selected")),
          cx,
        );
      });
      cx.notify();
      return;
    };

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    // Draft edits go through the pending-review GraphQL endpoint (by node id).
    let pending_comment_node_id = self
      .review_comments
      .iter()
      .find(|comment| comment.id == comment_id)
      .filter(|comment| comment.is_pending)
      .map(|comment| comment.node_id.clone());
    let api = self.api.clone();
    let body_for_api = body.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          if let Some(node_id) = pending_comment_node_id {
            api.update_pending_review_comment(&owner, &repo, number, &node_id, &body_for_api)?;
            Ok::<_, anyhow::Error>(None)
          } else {
            api
              .update_pull_request_review_comment(&owner, &repo, number, comment_id, &body_for_api)
              .map(Some)
          }
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        let mut error_message: Option<Arc<str>> = None;
        match result {
          Ok(Some(updated_comment)) => {
            if let Some(existing) = this
              .review_comments
              .iter_mut()
              .find(|comment| comment.id == updated_comment.id)
            {
              *existing = updated_comment;
            } else {
              this.review_comments.push(updated_comment);
            }
            this.review_comments_error = None;
            this.sync_review_comments(cx);
          }
          Ok(None) => {
            if let Some(existing) = this
              .review_comments
              .iter_mut()
              .find(|comment| comment.id == comment_id)
            {
              existing.body = body;
            }
            this.review_comments_error = None;
            this.sync_review_comments(cx);
          }
          Err(error) => {
            let error_message_text = error.to_string();
            this.review_comments_error = Some(error_message_text.clone().into());
            this.add_pr_breadcrumb("Update review comment failed", Map::new());
            this.record_pr_error(
              "github.pr.review_comment.update",
              error_message_text.as_str(),
              Map::new(),
            );
            error_message = Some(Arc::from(error_message_text));
          }
        }
        let success = error_message.is_none();
        this.diff_editor.update(cx, |editor, cx| {
          editor.finish_review_comment_edit_submission(comment_id, error_message, cx);
        });
        if success {
          this.focus_changes_tree_via_window_handle(cx);
        }
        cx.notify();
      });
    });
    self.review_comments_task = Some(task);
  }

  pub(super) fn submit_review_comment_create(
    &mut self,
    request: ReviewCommentCreateRequest,
    cx: &mut Context<Self>,
  ) {
    if self.selected_commit_sha.is_some() {
      let message = Arc::<str>::from("Review comments are disabled for commit-level diffs");
      self.review_comments_error = Some(message.to_string().into());
      self.diff_editor.update(cx, |editor, cx| {
        editor.finish_review_comment_create_submission(Some(message.clone()), cx);
      });
      cx.notify();
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      self.review_comments_error = Some("No pull request selected".into());
      self.diff_editor.update(cx, |editor, cx| {
        editor
          .finish_review_comment_create_submission(Some(Arc::from("No pull request selected")), cx);
      });
      cx.notify();
      return;
    };
    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let number = pull_request.number;
    let pull_request_node_id = pull_request.node_id.clone();
    let in_reply_to_id = request.in_reply_to_id;
    let line_comment_payload = if in_reply_to_id.is_none() {
      let Some(selected_file) = self.selected_file.as_ref() else {
        self.review_comments_error = Some("No selected file".into());
        self.diff_editor.update(cx, |editor, cx| {
          editor.finish_review_comment_create_submission(Some(Arc::from("No selected file")), cx);
        });
        cx.notify();
        return;
      };

      let side = match request.side {
        ReviewCommentSide::Left => "LEFT".to_string(),
        ReviewCommentSide::Right => "RIGHT".to_string(),
      };
      let start_side = request.start_side.map(|value| match value {
        ReviewCommentSide::Left => "LEFT".to_string(),
        ReviewCommentSide::Right => "RIGHT".to_string(),
      });
      let line = request.line.saturating_add(1) as u64;
      let start_line = request
        .start_line
        .map(|value| value.saturating_add(1) as u64);

      Some((
        selected_file.path.to_string(),
        pull_request.head_sha.clone(),
        line,
        side,
        start_line,
        start_side,
      ))
    } else {
      None
    };
    let body = request.body.as_ref().to_string();
    let api = self.api.clone();

    // Pending-review path: draft a new top-level comment into the viewer's pending review,
    // starting one on GitHub if none exists yet. Replies stay immediate (single comments).
    if matches!(request.mode, ReviewCommentMode::PendingReview)
      && in_reply_to_id.is_none()
      && let Some((path, _commit_id, line, side, start_line, start_side)) = line_comment_payload
    {
      let pull_request_id = self
        .pending_review_pull_request_id
        .clone()
        .unwrap_or(pull_request_node_id);
      let existing_review_id = self.pending_review_id.clone();
      let task = cx.spawn(async move |this, cx| {
        let result = cx
          .background_spawn(async move {
            let review_id = match existing_review_id {
              Some(id) => id,
              None => {
                api
                  .start_pending_review(&owner, &repo, number, &pull_request_id)?
                  .node_id
              }
            };
            let comment = api.add_pending_review_thread(
              &owner,
              &repo,
              number,
              &pull_request_id,
              &review_id,
              &path,
              &body,
              "LINE",
              Some(line),
              Some(side.as_str()),
              start_line,
              start_side.as_deref(),
            )?;
            Ok::<_, anyhow::Error>((review_id, pull_request_id, comment))
          })
          .await;

        let _ = this.update(cx, |this, cx| {
          let mut error_message: Option<Arc<str>> = None;
          match result {
            Ok((review_id, pull_request_id, mut created_comment)) => {
              this.pending_review_id = Some(review_id);
              this.pending_review_pull_request_id = Some(pull_request_id);
              created_comment.is_pending = true;
              if let Some(existing) = this
                .review_comments
                .iter_mut()
                .find(|comment| comment.id == created_comment.id)
              {
                *existing = created_comment;
              } else {
                this.review_comments.push(created_comment);
              }
              this.review_comments_error = None;
              this.sync_review_comments(cx);
            }
            Err(error) => {
              let error_message_text = error.to_string();
              this.review_comments_error = Some(error_message_text.clone().into());
              this.add_pr_breadcrumb("Add pending review comment failed", Map::new());
              this.record_pr_error(
                "github.pr.pending_review.add_comment",
                error_message_text.as_str(),
                Map::new(),
              );
              error_message = Some(Arc::from(error_message_text));
            }
          }
          let success = error_message.is_none();
          this.diff_editor.update(cx, |editor, cx| {
            editor.finish_review_comment_create_submission(error_message, cx);
          });
          if success {
            this.focus_changes_tree_via_window_handle(cx);
          }
          cx.notify();
        });
      });
      self.review_comments_task = Some(task);
      return;
    }

    // Pending-review path: a reply joins the pending review when one is in progress.
    if let Some(in_reply_to_id) = in_reply_to_id
      && let Some(review_id) = self.pending_review_id.clone()
      && let Some(thread_node_id) = self
        .review_comments
        .iter()
        .find(|comment| comment.id == in_reply_to_id)
        .map(|comment| comment.thread_id.clone())
        .filter(|thread_id| !thread_id.is_empty())
    {
      let task = cx.spawn(async move |this, cx| {
        let result = cx
          .background_spawn(async move {
            api.reply_pending_review_thread(
              &owner,
              &repo,
              number,
              &review_id,
              &thread_node_id,
              &body,
            )
          })
          .await;

        let _ = this.update(cx, |this, cx| {
          let mut error_message: Option<Arc<str>> = None;
          match result {
            Ok(_reply_node_id) => {
              this.review_comments_error = None;
              // Reply returns a node id only; refetch to surface the draft reply with full data.
              this.refresh_pull_request_conversation_for_current_pull_request(false, cx);
            }
            Err(error) => {
              let error_message_text = error.to_string();
              this.review_comments_error = Some(error_message_text.clone().into());
              this.add_pr_breadcrumb("Add pending review reply failed", Map::new());
              this.record_pr_error(
                "github.pr.pending_review.reply",
                error_message_text.as_str(),
                Map::new(),
              );
              error_message = Some(Arc::from(error_message_text));
            }
          }
          let success = error_message.is_none();
          this.diff_editor.update(cx, |editor, cx| {
            editor.finish_review_comment_create_submission(error_message, cx);
          });
          if success {
            this.focus_changes_tree_via_window_handle(cx);
          }
          cx.notify();
        });
      });
      self.review_comments_task = Some(task);
      return;
    }

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          if let Some(in_reply_to_id) = in_reply_to_id {
            api.reply_pull_request_review_comment(&owner, &repo, number, in_reply_to_id, &body)
          } else {
            let (path, commit_id, line, side, start_line, start_side) = line_comment_payload
              .expect("line comment payload should exist when creating a top-level comment");
            api.create_pull_request_review_comment(
              &owner,
              &repo,
              number,
              &path,
              &commit_id,
              line,
              &side,
              start_line,
              start_side.as_deref(),
              &body,
            )
          }
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        let mut error_message: Option<Arc<str>> = None;
        match result {
          Ok(created_comment) => {
            if let Some(existing) = this
              .review_comments
              .iter_mut()
              .find(|comment| comment.id == created_comment.id)
            {
              *existing = created_comment;
            } else {
              this.review_comments.push(created_comment);
            }
            if let Some(pr) = this.pull_request.as_mut()
              && let Some(login) = Self::current_github_login(cx)
            {
              pr.requested_reviewers
                .retain(|r| !r.login.eq_ignore_ascii_case(&login));
            }
            this.review_comments_error = None;
            this.sync_review_comments(cx);
          }
          Err(error) => {
            let error_message_text = error.to_string();
            this.review_comments_error = Some(error_message_text.clone().into());
            this.add_pr_breadcrumb("Create review comment failed", Map::new());
            this.record_pr_error(
              "github.pr.review_comment.create",
              error_message_text.as_str(),
              Map::new(),
            );
            error_message = Some(Arc::from(error_message_text));
          }
        }
        let success = error_message.is_none();
        this.diff_editor.update(cx, |editor, cx| {
          editor.finish_review_comment_create_submission(error_message, cx);
        });
        if success {
          this.focus_changes_tree_via_window_handle(cx);
        }
        cx.notify();
      });
    });
    self.review_comments_task = Some(task);
  }

  pub(super) fn confirm_review_comment_delete(
    &mut self,
    comment_id: u64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_commit_sha.is_some() {
      self.review_comments_error =
        Some("Review comments are disabled for commit-level diffs".into());
      cx.notify();
      return;
    }

    if !self.editable_review_comment_ids(cx).contains(&comment_id) {
      return;
    }

    let title: SharedString = "Delete comment?".into();
    let message: SharedString = "This review comment will be permanently deleted.".into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Delete")
        .cancel_text("Cancel")
        .destructive()
        .on_confirm(move |_, _, cx| {
          view.update(cx, |this, cx| {
            this.submit_review_comment_delete(comment_id, cx);
          });
          true
        })
        .build(alert)
    });
  }

  pub(super) fn submit_review_comment_delete(&mut self, comment_id: u64, cx: &mut Context<Self>) {
    if self.selected_commit_sha.is_some() {
      self.review_comments_error =
        Some("Review comments are disabled for commit-level diffs".into());
      self.diff_editor.update(cx, |editor, cx| {
        editor.finish_review_comment_delete_submission(comment_id, cx);
      });
      cx.notify();
      return;
    }

    let Some((owner, repo, number)) = self.pull_request.as_ref().map(|pull_request| {
      (
        pull_request.repository.owner.clone(),
        pull_request.repository.repo.clone(),
        pull_request.number,
      )
    }) else {
      self.review_comments_error = Some("No pull request selected".into());
      self.diff_editor.update(cx, |editor, cx| {
        editor.finish_review_comment_delete_submission(comment_id, cx);
      });
      cx.notify();
      return;
    };
    let Some((removed_index, removed_comment)) = self
      .review_comments
      .iter()
      .enumerate()
      .find(|(_, comment)| comment.id == comment_id)
      .map(|(index, comment)| (index, comment.clone()))
    else {
      return;
    };

    self.diff_editor.update(cx, |editor, cx| {
      editor.start_review_comment_delete_submission(comment_id, cx);
    });
    self.review_comments.remove(removed_index);
    self.review_comments_error = None;
    self.sync_review_comments(cx);
    cx.notify();

    // Draft deletes go through the pending-review GraphQL endpoint (by node id).
    let pending_node_id = removed_comment
      .is_pending
      .then(|| removed_comment.node_id.clone());
    let api = self.api.clone();

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          if let Some(node_id) = pending_node_id {
            api.delete_pending_review_comment(&owner, &repo, number, &node_id)
          } else {
            api.delete_pull_request_review_comment(&owner, &repo, number, comment_id)
          }
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        if let Err(error) = result {
          if !this
            .review_comments
            .iter()
            .any(|comment| comment.id == removed_comment.id)
          {
            let insert_index = removed_index.min(this.review_comments.len());
            this
              .review_comments
              .insert(insert_index, removed_comment.clone());
          }
          let error_message_text = error.to_string();
          this.review_comments_error = Some(error_message_text.clone().into());
          this.add_pr_breadcrumb("Delete review comment failed", Map::new());
          this.record_pr_error(
            "github.pr.review_comment.delete",
            error_message_text.as_str(),
            Map::new(),
          );
          this.sync_review_comments(cx);
        } else {
          this.review_comments_error = None;
        }

        this.diff_editor.update(cx, |editor, cx| {
          editor.finish_review_comment_delete_submission(comment_id, cx);
        });
        cx.notify();
      });
    });
    self.review_comments_task = Some(task);
  }

  pub(super) fn upload_dropped_images(
    &mut self,
    paths: &ExternalPaths,
    input: Entity<TextareaState>,
    on_error: impl Fn(&mut Self, String, &mut Context<Self>) + Send + 'static + Clone,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    github_shared::upload_dropped_images(paths, input, self.api.clone(), on_error, window, cx);
  }

  // Uploads run fire-and-forget: no per-composer error field to surface into.
  pub(super) fn handle_diff_editor_review_comment_drop(
    &mut self,
    paths: &ExternalPaths,
    input: Entity<TextareaState>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.upload_dropped_images(
      paths,
      input,
      |this, message, _cx| {
        this.add_pr_breadcrumb("Diff editor image upload failed", {
          let mut map = Map::new();
          map.insert("error".into(), Value::String(message));
          map
        });
      },
      window,
      cx,
    );
  }

  pub(super) fn review_comments_for_selected_file(&self) -> Vec<ReviewComment> {
    let Some(file) = self.selected_file.as_ref() else {
      return Vec::new();
    };

    let comments_for_file: Vec<&GithubPullRequestReviewComment> = self
      .review_comments
      .iter()
      .filter(|comment| comment.path == file.path)
      .collect();
    let comments_by_id: HashMap<u64, &GithubPullRequestReviewComment> = comments_for_file
      .iter()
      .map(|comment| (comment.id, *comment))
      .collect();

    self
      .review_comments
      .iter()
      .filter(|comment| review_comment_targets_file(comment, file))
      .filter_map(|comment| review_comment_to_editor_comment(comment, &comments_by_id))
      .collect()
  }

  pub(super) fn review_comment_code_reference_requests_for_comments(
    &self,
    comments: &[ReviewComment],
  ) -> HashMap<u64, Vec<GithubBlobLineReference>> {
    comments
      .iter()
      .filter_map(|comment| {
        let references = code_reference_requests_from_markdown(comment.body.as_ref());
        if references.is_empty() {
          None
        } else {
          Some((comment.id, references))
        }
      })
      .collect()
  }

  pub(super) fn description_code_reference_requests_for_pull_request(
    pull_request: &GithubPullRequestDetails,
  ) -> Vec<GithubBlobLineReference> {
    pull_request
      .body
      .as_deref()
      .map(code_reference_requests_from_markdown)
      .unwrap_or_default()
  }

  pub(super) fn prefetch_overview_root_review_comment_files(&mut self, cx: &mut Context<Self>) {
    if self.review_comments.is_empty() || self.file_lookup.is_empty() {
      return;
    }

    let mut seen_paths = HashSet::new();
    for root_id in overview_root_review_comment_ids(&self.review_comments) {
      let Some(comment) = self
        .review_comments
        .iter()
        .find(|comment| comment.id == root_id)
      else {
        continue;
      };
      let Some(file) = self.file_for_review_comment_path(comment.path.as_str()) else {
        continue;
      };
      let canonical_path = file.path.to_string();
      if !seen_paths.insert(canonical_path) {
        continue;
      }
      self.maybe_fetch_file_contents(file, cx);
    }
  }

  pub(super) fn cached_review_comment_code_reference_previews(
    &self,
    requests: &HashMap<u64, Vec<GithubBlobLineReference>>,
  ) -> HashMap<u64, Vec<ReviewCommentCodeReferencePreview>> {
    requests
      .iter()
      .filter_map(|(comment_id, references)| {
        let previews: Vec<ReviewCommentCodeReferencePreview> = references
          .iter()
          .filter_map(|reference| {
            self
              .review_comment_code_reference_cache
              .get(&reference.url)
              .and_then(|preview| preview.clone())
          })
          .collect();
        if previews.is_empty() {
          None
        } else {
          Some((*comment_id, previews))
        }
      })
      .collect()
  }

  pub(super) fn schedule_code_reference_fetches<'a, I>(
    &mut self,
    references: I,
    cx: &mut Context<Self>,
  ) where
    I: IntoIterator<Item = &'a GithubBlobLineReference>,
  {
    for reference in references {
      if self
        .review_comment_code_reference_cache
        .contains_key(&reference.url)
        || self
          .review_comment_code_reference_tasks
          .contains_key(&reference.url)
      {
        continue;
      }

      let cache_key = reference.url.clone();
      let api = self.api.clone();
      let owner = reference.owner.clone();
      let repo = reference.repo.clone();
      let path = reference.path.clone();
      let revision = reference.reference.clone();
      let start_line = reference.start_line;
      let end_line = reference.end_line;
      let repo_label = github_shared::repo_label(&owner, &repo);
      let url = Arc::<str>::from(reference.url.as_str());
      let path_arc = Arc::<str>::from(path.as_str());
      let reference_arc = Arc::<str>::from(revision.as_str());
      let repo_arc = Arc::<str>::from(repo_label.as_str());

      let task = cx.spawn(async move |this, cx| {
        let result = cx
          .background_spawn(async move {
            api.fetch_github_file_content(&owner, &repo, &path, &revision)
          })
          .await;

        let preview = match result {
          Ok(Some(content)) => github_shared::line_snippets_from_content(
            &content, start_line, end_line,
          )
          .map(|snippets| {
            let actual_end_line = start_line.saturating_add(snippets.len().saturating_sub(1));
            ReviewCommentCodeReferencePreview {
              url: url.clone(),
              repo: repo_arc.clone(),
              path: path_arc.clone(),
              reference: reference_arc.clone(),
              start_line,
              end_line: actual_end_line,
              snippets: snippets.into_iter().map(Arc::<str>::from).collect(),
              full_content: Some(Arc::<str>::from(content.as_str())),
            }
          }),
          _ => None,
        };

        let _ = this.update(cx, |this, cx| {
          this
            .review_comment_code_reference_cache
            .insert(cache_key.clone(), preview);
          this.review_comment_code_reference_tasks.remove(&cache_key);
          this.sync_review_comments(cx);
          cx.notify();
        });
      });

      self
        .review_comment_code_reference_tasks
        .insert(reference.url.clone(), task);
    }
  }

  pub(super) fn sync_review_comments(&mut self, cx: &mut Context<Self>) {
    self.sync_review_comment_handlers(cx);
    if self.selected_commit_sha.is_some() {
      self.selected_file_review_comment_ids.clear();
      self.active_review_comment_id = None;
      self.diff_editor.update(cx, |editor, cx| {
        editor.set_review_comment_pr_number(None, cx);
        editor.set_editable_review_comment_ids(std::iter::empty::<u64>(), cx);
        editor.set_review_comments(Vec::new(), cx);
        editor.set_review_comment_code_reference_previews(HashMap::new(), cx);
      });
      self.pending_review_comment_link_comment_id = None;
      return;
    }

    let comments = self.review_comments_for_selected_file();
    self.selected_file_review_comment_ids = comments.iter().map(|comment| comment.id).collect();
    if self
      .active_review_comment_id
      .is_some_and(|id| !self.selected_file_review_comment_ids.contains(&id))
    {
      self.active_review_comment_id = None;
    }
    let preview_requests = self.review_comment_code_reference_requests_for_comments(&comments);
    let preview_map = self.cached_review_comment_code_reference_previews(&preview_requests);
    let pr_number = self.pull_request.as_ref().map(|pr| pr.number);
    let editable_comment_ids = self.editable_review_comment_ids(cx);
    let has_pending_review = self.pending_review_id.is_some();
    self.diff_editor.update(cx, move |editor, cx| {
      editor.set_review_comment_pr_number(pr_number, cx);
      editor.set_editable_review_comment_ids(editable_comment_ids.iter().copied(), cx);
      editor.set_review_comments(comments, cx);
      editor.set_has_pending_review(has_pending_review, cx);
      editor.set_review_comment_code_reference_previews(preview_map, cx);
    });
    self.schedule_code_reference_fetches(
      preview_requests.values().flat_map(|items| items.iter()),
      cx,
    );
    self.resolve_pending_review_comment_link(cx);
  }

  pub(super) fn editable_review_comment_ids(&self, cx: &App) -> HashSet<u64> {
    let Some(login) = Self::current_github_login(cx) else {
      return HashSet::new();
    };

    self
      .review_comments
      .iter()
      .filter(|comment| review_comment_owned_by_login(comment, &login))
      .map(|comment| comment.id)
      .collect()
  }

  pub(super) fn render_review_popover(
    &mut self,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let author_cannot_approve_tooltip =
      "Pull request authors cannot approve their own pull requests.".to_string();
    let author_cannot_request_changes_tooltip =
      "Pull request authors cannot request changes on their own pull requests.".to_string();
    let is_current_user_pr_author = self.is_current_user_pr_author(cx);
    let review_body = self.review_input.read(cx).value().to_string();
    let submit_review_disabled = self.submit_review_loading
      || self.pull_request.is_none()
      || (is_current_user_pr_author
        && !matches!(self.review_decision, GithubPrReviewDecision::Comment))
      || Self::validate_review_submission(self.review_decision, review_body.as_str()).is_some();
    let review_decision_index = Self::review_decision_index(self.review_decision);
    let review_button_disabled = self.pull_request.is_none();
    let review_preview_open = self.review_preview_open;
    let review_markdown_options = self.build_overview_composer_markdown_options(5_555, cx);
    let page_for_review_toggle = cx.entity().clone();
    let pending_comment_count = self
      .review_comments
      .iter()
      .filter(|comment| comment.is_pending)
      .count();

    Popover::new("pr-review-popover")
      .anchor(Anchor::TopRight)
      .w(px(PR_REVIEW_POPOVER_WIDTH))
      .open(self.review_popover_open)
      .on_open_change(cx.listener(|this, open, window, cx| {
        this.review_popover_open = *open;
        if *open {
          if this.review_form_reset_pending {
            this.reset_review_form(window, cx);
          }
          if this.is_current_user_pr_author(cx)
            && !matches!(this.review_decision, GithubPrReviewDecision::Comment)
          {
            this.review_decision = GithubPrReviewDecision::Comment;
          }
          this.focus_review_input(window);
        }
        cx.notify();
      }))
      .trigger(
        Button::new("pr-review-button")
          .child(
            h_flex()
              .items_center()
              .child(Icon::new(UiIconName::Eye).size_3p5().mr_1p5())
              .child("Review")
              .child(Icon::new(IconName::ChevronDown).size_3p5().ml_2()),
          )
          .with_variant(ButtonVariant::Secondary)
          .outline()
          .small()
          .disabled(review_button_disabled),
      )
      .child(
        v_flex()
          .id("pr-review-popover-content")
          .w_full()
          .gap_3()
          .child(
            div()
              .text_sm()
              .font_medium()
              .text_color(theme.foreground)
              .child("Submit review"),
          )
          .when(pending_comment_count > 0, |this| {
            let label = if pending_comment_count == 1 {
              "1 pending comment will be published".to_string()
            } else {
              format!("{pending_comment_count} pending comments will be published")
            };
            this.child(
              div()
                .text_xs()
                .text_color(theme.status_yellow())
                .child(label),
            )
          })
          .child(
            div().w_full().child(
              MarkdownComposer::new(&self.review_input)
                .w_full()
                .h(px(PR_REVIEW_INPUT_HEIGHT_PX))
                .preview_open(review_preview_open)
                .on_toggle_preview(move |_, cx| {
                  page_for_review_toggle.update(cx, |this, cx| {
                    this.review_preview_open = !this.review_preview_open;
                    cx.notify();
                  });
                })
                .preview(move |text, _, cx| render_markdown(text, &review_markdown_options, cx)),
            ),
          )
          .child(
            RadioGroup::vertical("pr-review-decision-group")
              .selected_index(Some(review_decision_index))
              .on_click(cx.listener(|this, index: &usize, _, cx| {
                let next_decision = Self::review_decision_from_index(*index);
                if this.is_current_user_pr_author(cx)
                  && !matches!(next_decision, GithubPrReviewDecision::Comment)
                {
                  this.review_decision = GithubPrReviewDecision::Comment;
                  this.submit_review_error = Some(
                    "Pull request authors cannot approve or request changes on their own pull requests."
                      .into(),
                  );
                  cx.notify();
                  return;
                }
                this.review_decision = next_decision;
                this.submit_review_error = None;
                cx.notify();
              }))
              .child(Radio::new("pr-review-decision-comment").label("Comment"))
              .child(
                Radio::new("pr-review-decision-approve")
                  .label("Approve")
                  .disabled(is_current_user_pr_author)
                  .when(is_current_user_pr_author, |this| {
                    this.tooltip(author_cannot_approve_tooltip.clone())
                  }),
              )
              .child(
                Radio::new("pr-review-decision-request-changes")
                  .label("Request changes")
                  .disabled(is_current_user_pr_author)
                  .when(is_current_user_pr_author, |this| {
                    this.tooltip(author_cannot_request_changes_tooltip.clone())
                  }),
              ),
          )
          .when_some(self.submit_review_error.clone(), |this, error| {
            this.child(
              div()
                .text_xs()
                .text_color(theme.status_red())
                .overflow_hidden()
                .text_ellipsis_start()
                .child(error),
            )
          })
          .child(
            h_flex()
              .items_center()
              .justify_end()
              .gap_2()
              .child(
                Button::new("pr-review-cancel")
                  .ghost()
                  .small()
                  .label("Cancel")
                  .disabled(self.submit_review_loading)
                  .on_click(cx.listener(|this, _, window, cx| {
                    this.review_popover_open = false;
                    this.reset_review_form(window, cx);
                    cx.notify();
                  })),
              )
              .child(
                Button::new("pr-review-submit")
                  .primary()
                  .small()
                  .label("Submit review")
                  .child(Kbd::new(Keystroke::parse("cmd-enter").unwrap()).ml_1())
                  .loading(self.submit_review_loading)
                  .disabled(submit_review_disabled)
                  .on_click(cx.listener(|this, _, window, cx| {
                    this.submit_pull_request_review(window, cx);
                  })),
              ),
          ),
      )
      .into_any_element()
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::super::*;
  use super::*;

  #[test]
  fn suggested_change_original_lines_detects_stale_head_content() {
    let file_contents = HashMap::from([(
      "src/main.rs".to_string(),
      GithubPrFileContents {
        base: None,
        head: Some("fn main() {\n  println!(\"new\");\n}\n".to_string()),
      },
    )]);
    let original_lines = vec!["  println!(\"old\");".to_string()];

    assert_eq!(
      GithubPrDetailsPage::suggested_change_original_lines_match_current_head(
        &file_contents,
        "src/main.rs",
        Some(2),
        &original_lines,
      ),
      Some(false)
    );
  }

  #[test]
  fn suggested_change_original_lines_match_current_head_content() {
    let file_contents = HashMap::from([(
      "src/main.rs".to_string(),
      GithubPrFileContents {
        base: None,
        head: Some("fn main() {\r\n  println!(\"old\");\r\n}\r\n".to_string()),
      },
    )]);
    let original_lines = vec!["  println!(\"old\");".to_string()];

    assert_eq!(
      GithubPrDetailsPage::suggested_change_original_lines_match_current_head(
        &file_contents,
        "src/main.rs",
        Some(2),
        &original_lines,
      ),
      Some(true)
    );
  }

  #[test]
  fn review_decision_to_api_event_maps_all_variants() {
    assert_eq!(
      GithubPrDetailsPage::review_decision_to_api_event(GithubPrReviewDecision::Comment),
      GithubPullRequestReviewEvent::Comment
    );
    assert_eq!(
      GithubPrDetailsPage::review_decision_to_api_event(GithubPrReviewDecision::Approve),
      GithubPullRequestReviewEvent::Approve
    );
    assert_eq!(
      GithubPrDetailsPage::review_decision_to_api_event(GithubPrReviewDecision::RequestChanges),
      GithubPullRequestReviewEvent::RequestChanges
    );
  }

  #[test]
  fn validate_review_submission_requires_body_for_comment_and_request_changes() {
    assert!(
      GithubPrDetailsPage::validate_review_submission(GithubPrReviewDecision::Comment, "   ")
        .is_some()
    );
    assert!(
      GithubPrDetailsPage::validate_review_submission(GithubPrReviewDecision::RequestChanges, "")
        .is_some()
    );
  }

  #[test]
  fn validate_review_submission_allows_empty_body_for_approve() {
    assert!(
      GithubPrDetailsPage::validate_review_submission(GithubPrReviewDecision::Approve, "   ")
        .is_none()
    );
  }

  #[test]
  fn review_decision_defaults_to_comment() {
    assert_eq!(
      GithubPrReviewDecision::default(),
      GithubPrReviewDecision::Comment
    );
  }

  #[test]
  fn file_for_review_comment_path_prefers_direct_match() {
    let files = files_from_api(vec![make_api_file("src/main.rs", "modified", None)]);
    let lookup: HashMap<String, Rc<GithubPrFileDiff>> = files
      .into_iter()
      .map(|file| (file.path.as_ref().to_string(), file))
      .collect();

    let resolved = file_for_review_comment_path(&lookup, "src/main.rs");
    assert_eq!(
      resolved.as_ref().map(|file| file.path.as_ref()),
      Some("src/main.rs")
    );
  }

  #[test]
  fn file_for_review_comment_path_falls_back_to_renamed_old_path() {
    let files = files_from_api(vec![make_api_file(
      "src/new.rs",
      "renamed",
      Some("src/old.rs"),
    )]);
    let lookup: HashMap<String, Rc<GithubPrFileDiff>> = files
      .into_iter()
      .map(|file| (file.path.as_ref().to_string(), file))
      .collect();

    let resolved = file_for_review_comment_path(&lookup, "src/old.rs");
    assert_eq!(
      resolved.as_ref().map(|file| file.path.as_ref()),
      Some("src/new.rs")
    );
    assert!(file_for_review_comment_path(&lookup, "missing.rs").is_none());
  }

  #[test]
  fn code_reference_requests_from_markdown_extracts_blob_links() {
    let body = "[compose](https://github.com/acme/widget/blob/main/docker-compose.yml#L7)";
    let references = code_reference_requests_from_markdown(body);
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].owner, "acme");
    assert_eq!(references[0].repo, "widget");
  }

  #[test]
  fn next_review_comment_navigation_index_falls_back_when_active_comment_is_missing() {
    let comment_ids = [11, 22, 33];
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        Some(99),
        ReviewCommentNavigationDirection::Next
      ),
      Some(0)
    );
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        Some(99),
        ReviewCommentNavigationDirection::Previous
      ),
      Some(2)
    );
  }

  #[test]
  fn next_review_comment_navigation_index_handles_empty_list() {
    assert_eq!(
      next_review_comment_navigation_index(&[], None, ReviewCommentNavigationDirection::Next),
      None
    );
  }

  #[test]
  fn next_review_comment_navigation_index_uses_first_or_last_without_active_selection() {
    let comment_ids = [11, 22, 33];
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        None,
        ReviewCommentNavigationDirection::Next
      ),
      Some(0)
    );
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        None,
        ReviewCommentNavigationDirection::Previous
      ),
      Some(2)
    );
  }

  #[test]
  fn next_review_comment_navigation_index_wraps_in_both_directions() {
    let comment_ids = [11, 22, 33];
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        Some(33),
        ReviewCommentNavigationDirection::Next
      ),
      Some(0)
    );
    assert_eq!(
      next_review_comment_navigation_index(
        &comment_ids,
        Some(11),
        ReviewCommentNavigationDirection::Previous
      ),
      Some(2)
    );
  }

  #[test]
  fn overview_root_review_comment_ids_collapses_threads_to_root_only() {
    let review_comments = vec![
      make_review_comment(1, "2026-02-28T10:00:00Z", None),
      make_review_comment(2, "2026-02-28T10:01:00Z", Some(1)),
      make_review_comment(3, "2026-02-28T10:02:00Z", Some(2)),
    ];

    let roots = overview_root_review_comment_ids(&review_comments);
    assert_eq!(roots, vec![1]);
    assert!(!roots.contains(&2));
    assert!(!roots.contains(&3));
  }

  #[test]
  fn overview_root_review_comment_ids_keeps_distinct_thread_roots() {
    let review_comments = vec![
      make_review_comment(1, "2026-02-28T10:00:00Z", None),
      make_review_comment(2, "2026-02-28T10:01:00Z", Some(1)),
      make_review_comment(10, "2026-02-28T10:02:00Z", None),
      make_review_comment(11, "2026-02-28T10:03:00Z", Some(10)),
    ];

    let roots = overview_root_review_comment_ids(&review_comments);
    assert_eq!(roots, vec![1, 10]);
  }

  #[test]
  fn overview_root_review_comment_ids_uses_orphan_reply_as_its_own_root() {
    let review_comments = vec![make_review_comment(7, "2026-02-28T10:00:00Z", Some(999))];
    let roots = overview_root_review_comment_ids(&review_comments);
    assert_eq!(roots, vec![7]);
  }

  #[test]
  fn parse_github_commit_url_accepts_pull_request_commit_urls() {
    let parsed = parse_github_commit_url(
      "https://github.com/acme/widget/pull/42/commits/abcdef1234567890?diff=split",
    );
    assert_eq!(
      parsed,
      Some((
        "acme".to_string(),
        "widget".to_string(),
        "abcdef1234567890".to_string(),
      ))
    );
  }

  #[test]
  fn parse_github_commit_url_accepts_repository_commit_urls() {
    let parsed = parse_github_commit_url("https://github.com/acme/widget/commit/abcdef1234567890");
    assert_eq!(
      parsed,
      Some((
        "acme".to_string(),
        "widget".to_string(),
        "abcdef1234567890".to_string(),
      ))
    );
  }

  #[test]
  fn resolve_same_pr_commit_link_sha_matches_exact_and_unique_prefix_links() {
    let commits = vec![
      make_api_commit(
        "abcdef1234567890abcdef1234567890abcdef12",
        "first",
        Some("2026-02-20T10:00:00Z"),
        Some("p1"),
      ),
      make_api_commit(
        "fedcba9876543210fedcba9876543210fedcba98",
        "second",
        Some("2026-02-21T10:00:00Z"),
        Some("p2"),
      ),
    ];
    let context = CurrentPrContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      number: 42,
    };

    let exact = resolve_same_pr_commit_link_sha(
      Some(&context),
      &commits,
      "https://github.com/acme/widget/commit/abcdef1234567890abcdef1234567890abcdef12",
    );
    assert_eq!(
      exact.as_deref(),
      Some("abcdef1234567890abcdef1234567890abcdef12")
    );

    let prefix = resolve_same_pr_commit_link_sha(
      Some(&context),
      &commits,
      "https://github.com/acme/widget/commit/fedcba9",
    );
    assert_eq!(
      prefix.as_deref(),
      Some("fedcba9876543210fedcba9876543210fedcba98")
    );
  }

  #[test]
  fn resolve_same_pr_commit_link_sha_rejects_other_repos_and_ambiguous_prefixes() {
    let commits = vec![
      make_api_commit(
        "abcdef1234567890abcdef1234567890abcdef12",
        "first",
        Some("2026-02-20T10:00:00Z"),
        Some("p1"),
      ),
      make_api_commit(
        "abcdef9999999999abcdef9999999999abcdef99",
        "second",
        Some("2026-02-21T10:00:00Z"),
        Some("p2"),
      ),
    ];
    let context = CurrentPrContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      number: 42,
    };

    let other_repo = resolve_same_pr_commit_link_sha(
      Some(&context),
      &commits,
      "https://github.com/acme/other/commit/abcdef1234567890abcdef1234567890abcdef12",
    );
    assert!(other_repo.is_none());

    let ambiguous = resolve_same_pr_commit_link_sha(
      Some(&context),
      &commits,
      "https://github.com/acme/widget/commit/abcdef",
    );
    assert!(ambiguous.is_none());
  }

  #[test]
  fn review_comment_owned_by_login_is_case_insensitive() {
    let comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    assert!(review_comment_owned_by_login(&comment, "OCTOCAT"));
    assert!(!review_comment_owned_by_login(&comment, "hubot"));
  }

  #[test]
  fn review_comment_preview_line_range_falls_back_to_original_fields() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.start_line = None;
    comment.line = None;
    comment.original_start_line = Some(14);
    comment.original_line = Some(16);

    assert_eq!(review_comment_preview_line_range(&comment), Some((14, 16)));
  }

  #[test]
  fn review_comment_preview_line_range_normalizes_order_and_rejects_non_positive_values() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.start_line = Some(21);
    comment.line = Some(19);
    assert_eq!(review_comment_preview_line_range(&comment), Some((19, 21)));

    comment.start_line = Some(0);
    comment.line = Some(-2);
    comment.original_start_line = Some(0);
    comment.original_line = Some(-1);
    assert_eq!(review_comment_preview_line_range(&comment), None);
  }

  #[test]
  fn review_comment_preview_line_range_prefers_primary_fields() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.start_line = Some(8);
    comment.line = Some(11);
    comment.original_start_line = Some(2);
    comment.original_line = Some(4);

    assert_eq!(review_comment_preview_line_range(&comment), Some((8, 11)));
  }

  #[test]
  fn review_comment_targets_file_matches_renamed_old_path() {
    let file = GithubPrFileDiff {
      path: "src/new.rs".into(),
      old_path: Some("src/old.rs".into()),
      status: GithubPrFileStatus::Renamed,
    };
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.path = "src/old.rs".to_string();

    assert!(review_comment_targets_file(&comment, &file));
  }

  #[test]
  fn review_comment_to_editor_comment_returns_none_without_current_anchor() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.line = None;
    comment.start_line = None;
    comment.original_line = Some(4);
    comment.original_start_line = Some(4);

    let comments_by_id = HashMap::from([(comment.id, &comment)]);

    assert!(review_comment_to_editor_comment(&comment, &comments_by_id).is_none());
  }

  #[test]
  fn suggestion_context_from_review_comment_falls_back_to_original_line_fields() {
    let mut comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    comment.diff_hunk = "@@ -10,3 +10,3 @@\n keep\n current\n keep".to_string();
    comment.start_line = None;
    comment.line = None;
    comment.original_start_line = None;
    comment.original_line = Some(11);

    let ctx = suggestion_context_from_review_comment(&comment).expect("suggestion context");

    assert_eq!(ctx.original_start_line, Some(11));
    assert_eq!(ctx.suggested_start_line, Some(11));
    assert_eq!(ctx.original_lines, vec!["current".to_string()]);
  }
}
