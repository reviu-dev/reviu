//! The Overview tab: the pull request's header, its status, its checks, and the
//! skeletons shown while any of it is still loading.

use super::*;

fn merged_reviewers(
  requested_reviewers: &[GithubPullRequestFilterOptionUser],
  reviews: &[GithubPullRequestReview],
  author_login: &str,
) -> Vec<GithubPullRequestFilterOptionUser> {
  let mut reviewers = Vec::new();
  let mut seen: HashSet<String> = HashSet::new();

  for reviewer in requested_reviewers {
    let key = reviewer.login.to_lowercase();
    if github_shared::logins_match_case_insensitive(reviewer.login.as_str(), author_login) {
      continue;
    }
    if seen.insert(key) {
      reviewers.push(reviewer.clone());
    }
  }

  for review in reviews {
    let Some(user) = review.user.as_ref() else {
      continue;
    };
    let key = user.login.to_lowercase();
    if github_shared::logins_match_case_insensitive(user.login.as_str(), author_login) {
      continue;
    }
    if seen.insert(key) {
      reviewers.push(GithubPullRequestFilterOptionUser {
        login: user.login.clone(),
        avatar_url: user.avatar_url.clone(),
      });
    }
  }

  reviewers
}

fn non_empty_owned(value: &str) -> Option<String> {
  let value = value.trim();
  if value.is_empty() {
    None
  } else {
    Some(value.to_string())
  }
}

fn singular_or_plural(count: u64, singular: &'static str, plural: &'static str) -> &'static str {
  if count == 1 { singular } else { plural }
}

fn overview_checks_summary_title(checks: &GithubPullRequestChecksSummary) -> String {
  if checks.total_checks == 0 {
    return "No checks have run".to_string();
  }

  match checks.overall_state {
    GithubPullRequestChecksRollupState::Success => "All checks have passed".to_string(),
    GithubPullRequestChecksRollupState::Skipped => "All checks were skipped".to_string(),
    GithubPullRequestChecksRollupState::Pending => {
      if checks.pending_checks == 0 {
        "Checks are pending".to_string()
      } else {
        "Checks".to_string()
      }
    }
    GithubPullRequestChecksRollupState::Failure => {
      if checks.failed_checks == 0 {
        "Checks need attention".to_string()
      } else {
        "Checks".to_string()
      }
    }
  }
}

fn overview_checks_summary_subtitle(checks: &GithubPullRequestChecksSummary) -> String {
  if checks.total_checks == 0 {
    return "No checks reported".to_string();
  }

  let mut parts = Vec::new();
  if checks.failed_checks > 0 {
    parts.push(format!("{} failing", checks.failed_checks));
  }
  if checks.pending_checks > 0 {
    parts.push(format!("{} pending", checks.pending_checks));
  }
  if checks.skipped_checks > 0 {
    parts.push(format!("{} skipped", checks.skipped_checks));
  }
  if checks.successful_checks > 0 {
    parts.push(format!(
      "{} successful {}",
      checks.successful_checks,
      singular_or_plural(checks.successful_checks, "check", "checks")
    ));
  }

  parts.join(", ")
}

fn overview_checks_uniform_state(
  checks: &GithubPullRequestChecksSummary,
) -> Option<GithubPullRequestChecksRollupState> {
  if checks.total_checks == 0 {
    return None;
  }

  if checks.successful_checks == checks.total_checks {
    Some(GithubPullRequestChecksRollupState::Success)
  } else if checks.pending_checks == checks.total_checks {
    Some(GithubPullRequestChecksRollupState::Pending)
  } else if checks.failed_checks == checks.total_checks {
    Some(GithubPullRequestChecksRollupState::Failure)
  } else if checks.skipped_checks == checks.total_checks {
    Some(GithubPullRequestChecksRollupState::Skipped)
  } else {
    None
  }
}

fn overview_checks_summary_slices(
  checks: &GithubPullRequestChecksSummary,
  theme: &gpui_component::Theme,
) -> Vec<OverviewChecksSummarySlice> {
  let mut slices = Vec::new();

  if checks.failed_checks > 0 {
    slices.push(OverviewChecksSummarySlice {
      value: checks.failed_checks as f32,
      color: theme.status_red(),
    });
  }

  if checks.pending_checks > 0 {
    slices.push(OverviewChecksSummarySlice {
      value: checks.pending_checks as f32,
      color: theme.status_orange(),
    });
  }

  if checks.skipped_checks > 0 {
    slices.push(OverviewChecksSummarySlice {
      value: checks.skipped_checks as f32,
      color: theme.status_gray(),
    });
  }

  if checks.successful_checks > 0 {
    slices.push(OverviewChecksSummarySlice {
      value: checks.successful_checks as f32,
      color: theme.status_green(),
    });
  }

  if slices.is_empty() {
    slices.push(OverviewChecksSummarySlice {
      value: 1.0,
      color: theme.muted_foreground.opacity(0.3),
    });
  }

  slices
}

fn overview_checks_summary_segments(
  slices: &[OverviewChecksSummarySlice],
) -> Vec<OverviewChecksSummarySegment> {
  let total: f32 = slices.iter().map(|slice| slice.value.max(0.0)).sum();
  if total <= 0.0 {
    return Vec::new();
  }

  let mut start_angle = 0.0;
  let mut segments = Vec::new();

  for slice in slices {
    let span = (slice.value.max(0.0) / total) * std::f32::consts::TAU;
    if span <= 0.0 {
      continue;
    }

    let half_gap = if slices.len() > 1 {
      (OVERVIEW_CHECKS_SUMMARY_RING_GAP_ANGLE / 2.0).min(span / 4.0)
    } else {
      0.0
    };
    let segment_start_angle = start_angle + half_gap;
    let segment_end_angle = start_angle + span - half_gap;
    if segment_end_angle > segment_start_angle {
      segments.push(OverviewChecksSummarySegment {
        start_angle: segment_start_angle,
        end_angle: segment_end_angle,
        color: slice.color,
      });
    }

    start_angle += span;
  }

  segments
}

fn overview_checks_summary_caps(
  segments: &[OverviewChecksSummarySegment],
) -> Vec<OverviewChecksSummaryCap> {
  if segments.len() <= 1 {
    return Vec::new();
  }

  let center = OVERVIEW_CHECKS_SUMMARY_RING_SIZE / 2.0;
  let mut caps = Vec::new();

  for segment in segments {
    for angle in [segment.start_angle, segment.end_angle] {
      let visual_angle = angle - std::f32::consts::FRAC_PI_2;
      let x = center + OVERVIEW_CHECKS_SUMMARY_RING_RADIUS * visual_angle.cos();
      let y = center + OVERVIEW_CHECKS_SUMMARY_RING_RADIUS * visual_angle.sin();
      caps.push(OverviewChecksSummaryCap {
        left: x - OVERVIEW_CHECKS_SUMMARY_RING_STROKE_WIDTH / 2.0,
        top: y - OVERVIEW_CHECKS_SUMMARY_RING_STROKE_WIDTH / 2.0,
        size: OVERVIEW_CHECKS_SUMMARY_RING_STROKE_WIDTH,
        color: segment.color,
      });
    }
  }

  caps
}

fn format_overview_check_duration(total_seconds: u64) -> String {
  if total_seconds < 60 {
    return format!("{total_seconds}s");
  }

  let total_minutes = total_seconds / 60;
  let seconds = total_seconds % 60;
  if total_minutes < 60 {
    return if seconds == 0 {
      format!("{total_minutes}m")
    } else {
      format!("{total_minutes}m {seconds}s")
    };
  }

  let total_hours = total_minutes / 60;
  let minutes = total_minutes % 60;
  if total_hours < 24 {
    return if minutes == 0 {
      format!("{total_hours}h")
    } else {
      format!("{total_hours}h {minutes}m")
    };
  }

  let days = total_hours / 24;
  let hours = total_hours % 24;
  if hours == 0 {
    format!("{days}d")
  } else {
    format!("{days}d {hours}h")
  }
}

fn overview_check_duration_label(
  started_at: Option<&str>,
  finished_at: Option<&str>,
  state: GithubPullRequestChecksRollupState,
) -> Option<String> {
  let started_at = parse_rfc3339(started_at?)?;
  let finished_at = finished_at.and_then(parse_rfc3339).or_else(|| {
    (state == GithubPullRequestChecksRollupState::Pending).then(time::OffsetDateTime::now_utc)
  })?;
  let elapsed_seconds = (finished_at - started_at).whole_seconds();
  if elapsed_seconds < 0 {
    return None;
  }

  Some(format_overview_check_duration(elapsed_seconds as u64))
}

fn overview_check_status_label(
  state: GithubPullRequestChecksRollupState,
  started_at: Option<&str>,
  finished_at: Option<&str>,
) -> Option<String> {
  match state {
    GithubPullRequestChecksRollupState::Success => Some(
      overview_check_duration_label(started_at, finished_at, state)
        .map(|d| format!("Successful in {d}"))
        .unwrap_or_else(|| "Successful".to_string()),
    ),
    GithubPullRequestChecksRollupState::Failure => Some(
      overview_check_duration_label(started_at, finished_at, state)
        .map(|d| format!("Failed in {d}"))
        .unwrap_or_else(|| "Failed".to_string()),
    ),
    GithubPullRequestChecksRollupState::Skipped => Some(
      finished_at
        .or(started_at)
        .map(|value| format!("Skipped {}", format_relative_time(value)))
        .unwrap_or_else(|| "Skipped".to_string()),
    ),
    GithubPullRequestChecksRollupState::Pending => Some(
      overview_check_duration_label(started_at, finished_at, state)
        .map(|d| format!("In progress - {d}"))
        .unwrap_or_else(|| "In progress".to_string()),
    ),
  }
}

fn overview_check_state_sort_key(row: &OverviewCheckRow) -> u8 {
  match row.state {
    GithubPullRequestChecksRollupState::Failure => 0,
    GithubPullRequestChecksRollupState::Pending => 1,
    GithubPullRequestChecksRollupState::Skipped => 2,
    GithubPullRequestChecksRollupState::Success => 3,
  }
}

fn overview_check_rows(checks: &GithubPullRequestChecksSummary) -> Vec<OverviewCheckRow> {
  let mut rows = Vec::new();

  for (ix, context) in checks.missing_required_contexts.iter().enumerate() {
    rows.push(OverviewCheckRow {
      id: format!("missing-required-context-{ix}"),
      state: GithubPullRequestChecksRollupState::Pending,
      title: context.clone(),
      status_label: Some("Required check has not reported yet".to_string()),
      app_label: None,
      app_slug: None,
      app_avatar_url: None,
      open_url: None,
    });
  }

  for run in &checks.actions_runs {
    let run_name = run
      .name
      .as_deref()
      .and_then(non_empty_owned)
      .unwrap_or_else(|| "GitHub Actions".to_string());
    let event_suffix = non_empty_owned(&run.event).map(|event| format!(" ({event})"));
    let run_started_at = run
      .run_started_at
      .as_deref()
      .or(Some(run.created_at.as_str()));
    let run_finished_at =
      (run.state != GithubPullRequestChecksRollupState::Pending).then_some(run.updated_at.as_str());

    if run.jobs.is_empty() {
      let title = match event_suffix.as_deref() {
        Some(suffix) => format!("{run_name}{suffix}"),
        None => run_name.clone(),
      };
      rows.push(OverviewCheckRow {
        id: format!("workflow-run-{}", run.id),
        state: run.state,
        title,
        status_label: overview_check_status_label(run.state, run_started_at, run_finished_at),
        app_label: Some("GitHub Actions".to_string()),
        app_slug: Some("github-actions".to_string()),
        app_avatar_url: None,
        open_url: run.html_url.clone(),
      });
      continue;
    }

    for job in &run.jobs {
      let job_name = non_empty_owned(&job.name).unwrap_or_else(|| run_name.clone());
      let title = match event_suffix.as_deref() {
        Some(suffix) => format!("{run_name} / {job_name}{suffix}"),
        None => format!("{run_name} / {job_name}"),
      };
      let job_started_at = job.started_at.as_deref().or(run_started_at);
      let job_finished_at = if job.state == GithubPullRequestChecksRollupState::Pending {
        None
      } else {
        job.completed_at.as_deref().or(run_finished_at)
      };

      rows.push(OverviewCheckRow {
        id: format!("workflow-job-{}", job.id),
        state: job.state,
        title,
        status_label: overview_check_status_label(job.state, job_started_at, job_finished_at),
        app_label: job
          .app_name
          .as_deref()
          .and_then(non_empty_owned)
          .or_else(|| Some("GitHub Actions".to_string())),
        app_slug: job
          .app_slug
          .as_deref()
          .and_then(non_empty_owned)
          .or_else(|| Some("github-actions".to_string())),
        app_avatar_url: job.app_avatar_url.as_deref().and_then(non_empty_owned),
        open_url: job.html_url.clone().or_else(|| run.html_url.clone()),
      });
    }
  }

  for check in &checks.other_checks {
    let title = non_empty_owned(&check.name).unwrap_or_else(|| "Check run".to_string());
    let finished_at = (check.state != GithubPullRequestChecksRollupState::Pending)
      .then_some(check.completed_at.as_deref())
      .flatten();

    rows.push(OverviewCheckRow {
      id: format!("check-run-{}", check.id),
      state: check.state,
      title,
      status_label: overview_check_status_label(
        check.state,
        check.started_at.as_deref(),
        finished_at,
      ),
      app_label: check
        .app_name
        .as_deref()
        .and_then(non_empty_owned)
        .or_else(|| check.app_slug.as_deref().and_then(non_empty_owned)),
      app_slug: check.app_slug.as_deref().and_then(non_empty_owned),
      app_avatar_url: check.app_avatar_url.as_deref().and_then(non_empty_owned),
      open_url: check.details_url.clone().or_else(|| check.html_url.clone()),
    });
  }

  for status in &checks.legacy_statuses {
    let title = non_empty_owned(&status.context).unwrap_or_else(|| "Status check".to_string());
    let finished_at = (status.state != GithubPullRequestChecksRollupState::Pending)
      .then_some(status.updated_at.as_str());
    rows.push(OverviewCheckRow {
      id: format!("legacy-status-{}", status.id),
      state: status.state,
      title,
      status_label: overview_check_status_label(
        status.state,
        Some(status.created_at.as_str()),
        finished_at,
      ),
      app_label: None,
      app_slug: None,
      app_avatar_url: status.avatar_url.as_deref().and_then(non_empty_owned),
      open_url: status.target_url.clone(),
    });
  }

  rows.sort_by_key(overview_check_state_sort_key);
  rows
}

fn overview_check_app_initial(row: &OverviewCheckRow) -> String {
  row
    .app_label
    .as_deref()
    .or(row.app_slug.as_deref())
    .unwrap_or(row.title.as_str())
    .chars()
    .next()
    .map(|c| c.to_uppercase().collect::<String>())
    .filter(|initial| !initial.is_empty())
    .unwrap_or_else(|| "C".to_string())
}

fn overview_pr_alert_kind(
  merge_readiness: Option<&GithubPullRequestMergeReadiness>,
  checks: Option<&GithubPullRequestChecksSummary>,
) -> Option<OverviewPrAlertKind> {
  if let Some(readiness) = merge_readiness {
    match readiness
      .mergeable_state
      .as_deref()
      .map(str::trim)
      .map(str::to_ascii_lowercase)
      .as_deref()
    {
      Some("dirty") => return Some(OverviewPrAlertKind::Conflicts),
      Some("behind") => return Some(OverviewPrAlertKind::OutOfDate),
      _ => {}
    }

    if matches!(
      readiness.status,
      GithubPullRequestMergeReadinessStatus::Blocked
    ) {
      return Some(OverviewPrAlertKind::Blocked);
    }
  }

  if checks.is_some_and(|checks| checks.requires_up_to_date_branch) {
    return Some(OverviewPrAlertKind::OutOfDate);
  }

  None
}

fn overview_review_status_info(
  merge_readiness: Option<&GithubPullRequestMergeReadiness>,
  requested_reviewers: &[GithubPullRequestFilterOptionUser],
  reviews: &[GithubPullRequestReview],
  author_login: &str,
) -> Option<OverviewReviewStatusInfo> {
  // Branch protection explicitly blocks the merge for review requirements.
  if let Some(readiness) = merge_readiness
    && matches!(
      readiness.status,
      GithubPullRequestMergeReadinessStatus::Blocked
    )
  {
    let state = readiness
      .mergeable_state
      .as_deref()
      .map(str::trim)
      .map(str::to_ascii_lowercase);
    if !matches!(state.as_deref(), Some("dirty") | Some("behind")) {
      return Some(OverviewReviewStatusInfo {
        title: "Review required",
        message: readiness.message.clone(),
      });
    }
  }

  // Derive status from requested reviewers and submitted reviews.
  let reviewers = merged_reviewers(requested_reviewers, reviews, author_login);
  if reviewers.is_empty() {
    return None;
  }

  let has_changes_requested = reviewers.iter().any(|r| {
    matches!(
      reviewer_status_for_login(reviews, &r.login, requested_reviewers),
      ReviewerStatus::ChangesRequested
    )
  });
  let has_approval = reviewers.iter().any(|r| {
    matches!(
      reviewer_status_for_login(reviews, &r.login, requested_reviewers),
      ReviewerStatus::Approved
    )
  });

  if has_changes_requested {
    Some(OverviewReviewStatusInfo {
      title: "Changes requested",
      message: "Some reviewers have requested changes.".to_string(),
    })
  } else if has_approval {
    Some(OverviewReviewStatusInfo {
      title: "Changes approved",
      message: "Pull request has been approved.".to_string(),
    })
  } else {
    Some(OverviewReviewStatusInfo {
      title: "Review required",
      message: "Awaiting review from requested reviewers.".to_string(),
    })
  }
}

fn overview_conflicts_info(
  merge_readiness: Option<&GithubPullRequestMergeReadiness>,
  checks: Option<&GithubPullRequestChecksSummary>,
) -> Option<OverviewConflictsInfo> {
  if let Some(readiness) = merge_readiness {
    let state = readiness
      .mergeable_state
      .as_deref()
      .map(str::trim)
      .map(str::to_ascii_lowercase);
    match state.as_deref() {
      Some("dirty") => {
        return Some(OverviewConflictsInfo {
          kind: OverviewConflictsKind::Conflicts,
          title: "Merge conflicts detected",
          message: "Resolve conflicts before continuing.".to_string(),
        });
      }
      Some("behind") => {
        return Some(OverviewConflictsInfo {
          kind: OverviewConflictsKind::OutOfDate,
          title: "Branch is out of date",
          message: "Update this branch before merging.".to_string(),
        });
      }
      _ => {
        return Some(OverviewConflictsInfo {
          kind: OverviewConflictsKind::NoConflicts,
          title: "No conflicts with base branch",
          message: "Merging can be performed automatically.".to_string(),
        });
      }
    }
  }

  if checks.is_some_and(|c| c.requires_up_to_date_branch) {
    return Some(OverviewConflictsInfo {
      kind: OverviewConflictsKind::OutOfDate,
      title: "Branch is out of date",
      message: "The base branch rules require this pull request to be up to date before merging."
        .to_string(),
    });
  }

  None
}

fn filter_option_users_contains(
  users: &[GithubPullRequestFilterOptionUser],
  candidate: &str,
) -> bool {
  users
    .iter()
    .any(|user| github_shared::logins_match_case_insensitive(user.login.as_str(), candidate))
}

fn matching_filter_option_users(
  options: &[GithubPullRequestFilterOptionUser],
  query: &str,
  selected: &[GithubPullRequestFilterOptionUser],
) -> Vec<GithubPullRequestFilterOptionUser> {
  let normalized_query = query.trim().to_lowercase();
  options
    .iter()
    .filter(|option| !filter_option_users_contains(selected, option.login.as_str()))
    .filter(|option| {
      normalized_query.is_empty() || option.login.to_lowercase().contains(&normalized_query)
    })
    .take(6)
    .cloned()
    .collect()
}

fn matching_label_options(
  options: &[GithubPullRequestFilterOptionLabel],
  query: &str,
  selected: &[GithubPullRequestLabel],
) -> Vec<GithubPullRequestFilterOptionLabel> {
  let normalized_query = query.trim().to_lowercase();
  options
    .iter()
    .filter(|option| !labels_contains(selected, option.name.as_str()))
    .filter(|option| {
      normalized_query.is_empty() || option.name.to_lowercase().contains(&normalized_query)
    })
    .take(8)
    .cloned()
    .collect()
}

fn overview_change_stat_labels(pr: &GithubPullRequestDetails) -> [String; 2] {
  [format!("+{}", pr.additions), format!("-{}", pr.deletions)]
}

fn pr_changes_tab_count_label(changed_files: u64) -> SharedString {
  changed_files.to_string().into()
}

fn overview_change_stats(
  pr: &GithubPullRequestDetails,
  theme: &gpui_component::Theme,
) -> Vec<gpui::AnyElement> {
  let [additions, deletions] = overview_change_stat_labels(pr);
  vec![
    div()
      .text_sm()
      .font_medium()
      .text_color(theme.status_green())
      .child(additions)
      .into_any_element(),
    div()
      .text_sm()
      .font_medium()
      .text_color(theme.status_red())
      .child(deletions)
      .into_any_element(),
  ]
}

fn local_repo_has_active_conflict_resolution(repo_root: &Path) -> bool {
  if is_merge_in_progress(repo_root).unwrap_or(false)
    || is_rebase_in_progress(repo_root).unwrap_or(false)
  {
    return true;
  }

  list_repo_status(repo_root)
    .map(|entries| {
      entries
        .iter()
        .any(|entry| entry.status == RepoStatusKind::Conflicted)
    })
    .unwrap_or(false)
}

fn should_prepare_local_branch_before_merging_base(
  repo_root: &Path,
  has_uncommitted_changes: bool,
) -> bool {
  has_uncommitted_changes && !local_repo_has_active_conflict_resolution(repo_root)
}

impl GithubPrDetailsPage {
  pub(super) fn render_header(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let changes_tab = if let Some(pull_request) = self.pull_request.as_ref() {
      Tab::new().child(
        h_flex().items_center().gap_2().child("Changes").child(
          div()
            .debug_selector(|| "github-pr-changes-tab-count".to_string())
            .child(
              Tag::secondary()
                .small()
                .rounded_full()
                .child(pr_changes_tab_count_label(pull_request.changed_files)),
            ),
        ),
      )
    } else {
      Tab::new().label("Changes")
    };

    let tab_bar = TabBar::new("pr-details-tabs")
      .segmented()
      .selected_index(self.active_tab_ix)
      .on_click(cx.listener(|this, ix: &usize, window, cx| {
        this.set_active_tab(*ix, window, cx);
      }))
      .child(Tab::new().label("Overview"))
      .child(changes_tab);

    let header_tools = self.render_pr_header_tools(cx);

    let back_button = || {
      Button::new("pr-back")
        .icon(IconName::ArrowLeft)
        .ghost()
        .compact()
        .on_click(cx.listener(|this, _, _, cx| {
          this.navigate_back(cx);
        }))
    };

    let left_area = if let Some(pr) = self.pull_request.as_ref() {
      let status_tag = github_shared::pull_request_status_tag(pr.status(), &theme);

      let title = div()
        .min_w_0()
        .text_sm()
        .font_medium()
        .text_color(theme.foreground)
        .overflow_hidden()
        .text_ellipsis_start()
        .child(pr.title.clone());

      let meta = h_flex()
        .items_center()
        .gap_2()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(format!("#{}", pr.number))
        .child(status_tag);

      h_flex()
        .items_center()
        .gap_2()
        .child(back_button())
        .child(div().flex().items_center().gap_2().child(title).child(meta))
    } else {
      let title_skeleton = Skeleton::new().w(px(220.)).h_4().rounded_md();
      let meta_skeleton = Skeleton::new().w(px(110.)).h_4().rounded_md().secondary();
      h_flex().items_center().gap_2().child(back_button()).child(
        div()
          .flex()
          .items_center()
          .gap_3()
          .child(title_skeleton)
          .child(meta_skeleton),
      )
    };
    let right_area = h_flex()
      .items_center()
      .gap_2()
      .when(!self.is_pull_request_merged(), |this| {
        this
          .when(
            !self
              .pull_request
              .as_ref()
              .is_some_and(|pull_request| pull_request.draft),
            |this| {
              this.child(
                div()
                  .debug_selector(|| "github-pr-merge-button".to_string())
                  .child(self.render_merge_popover(&theme, window, cx)),
              )
            },
          )
          .child(
            div()
              .debug_selector(|| "github-pr-review-button".to_string())
              .child(self.render_review_popover(&theme, cx)),
          )
      })
      .child(self.render_pr_actions_menu(cx));

    div()
      .px_3()
      .pt_2()
      .pb_3()
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
          .child(left_area)
          .child(right_area),
      )
      .child(
        h_flex()
          .items_center()
          .gap_4()
          .child(tab_bar)
          .when_some(header_tools, |this, tools| this.child(tools)),
      )
  }

  pub(super) fn render_pr_header_tools(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
    self.pull_request.as_ref()?;
    let theme = cx.theme().clone();
    let show_file_search = self.active_tab_ix == PR_TAB_CHANGES_IX;
    let show_local_project_controls =
      matches!(self.active_tab_ix, PR_TAB_OVERVIEW_IX | PR_TAB_CHANGES_IX);
    let tree_search_active = self.tree_search_query_normalized().is_some();
    let local_project_controls = show_local_project_controls
      .then(|| self.render_local_project_controls(&theme, cx, show_file_search))
      .flatten();

    if !show_file_search && local_project_controls.is_none() {
      return None;
    }

    Some(
      h_flex()
        .items_center()
        .gap_3()
        .when(show_file_search, |this| {
          this.child(
            div()
              .id("github-pr-file-contents-search")
              .debug_selector(|| "github-pr-file-contents-search".to_string())
              .relative()
              .w(px(280.0))
              .child(Input::new(&self.tree_search_input).w_full().pr(px(28.0)))
              .when(tree_search_active && self.tree_search_loading, |this| {
                this.child(
                  h_flex()
                    .absolute()
                    .top_0()
                    .right_2()
                    .bottom_0()
                    .items_center()
                    .child(Spinner::new().small()),
                )
              }),
          )
        })
        .when_some(local_project_controls, |this, controls| {
          this.child(
            h_flex()
              .debug_selector(|| "github-pr-local-project-controls".to_string())
              .child(controls),
          )
        })
        .into_any_element(),
    )
  }

  pub(super) fn render_review_requested_alert(&self, cx: &App) -> Option<AnyElement> {
    let pr = self.pull_request.as_ref()?;
    if pr.state != GithubPullRequestState::Open {
      return None;
    }
    let login = Self::current_github_login(cx)?;
    let is_requested = pr
      .requested_reviewers
      .iter()
      .any(|r| r.login.eq_ignore_ascii_case(&login));
    if !is_requested {
      return None;
    }

    let theme = cx.theme();
    Some(
      h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .px_3()
        .py_2()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.status_yellow().opacity(0.4))
        .bg(theme.status_yellow().opacity(0.08))
        .child(
          Icon::new(UiIconName::Eye)
            .size_4()
            .text_color(theme.status_yellow()),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme.foreground)
            .child("Your review has been requested on this pull request."),
        )
        .into_any_element(),
    )
  }

  pub(super) fn render_overview_status_section(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();

    let author_login = self
      .pull_request
      .as_ref()
      .map(|pr| pr.author.login.as_str())
      .unwrap_or("");
    let requested_reviewers = self
      .pull_request
      .as_ref()
      .map(|pr| pr.requested_reviewers.as_slice())
      .unwrap_or(&[]);
    let review_info = overview_review_status_info(
      self.merge_readiness.as_ref(),
      requested_reviewers,
      &self.reviews,
      author_login,
    );
    let conflicts_info = if self.is_pull_request_merged() {
      Some(OverviewConflictsInfo {
        kind: OverviewConflictsKind::Merged,
        title: "Pull request successfully merged and closed",
        message: "You're all set - the branch has been merged.".to_string(),
      })
    } else {
      overview_conflicts_info(self.merge_readiness.as_ref(), self.checks.as_ref())
    };
    let has_checks_content =
      self.checks.is_some() || self.checks_loading || self.checks_error.is_some();

    let is_loading = self.merge_readiness_loading || self.checks_loading || self.reviews_loading;

    if review_info.is_none() && !has_checks_content && conflicts_info.is_none() {
      if is_loading {
        return Self::render_overview_status_skeleton(&theme);
      }
      return div().into_any_element();
    }

    let mut card = v_flex()
      .id("github-pr-overview-status")
      .w_full()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius_lg)
      .overflow_hidden();

    let mut has_previous = false;

    if let Some(review) = &review_info
      && !self.is_pull_request_merged()
    {
      let review_icon: AnyElement = match review.title {
        "Changes approved" => Icon::new(UiIconName::CircleCheck)
          .size_5()
          .text_color(theme.status_green())
          .into_any_element(),
        "Changes requested" => Icon::new(IconName::CircleX)
          .size_5()
          .text_color(theme.status_red())
          .into_any_element(),
        _ => Self::render_status_icon_dot(theme.status_yellow()),
      };
      card = card.child(Self::render_overview_status_row(
        "github-pr-overview-review-status",
        review.title,
        &review.message,
        review_icon,
        has_previous,
        None::<AnyElement>,
        &theme,
      ));
      has_previous = true;
    }

    if has_checks_content {
      card = card.child(self.render_overview_checks_inner(has_previous, &theme, cx));
      has_previous = true;
    }

    if let Some(conflicts) = &conflicts_info {
      let icon = match conflicts.kind {
        OverviewConflictsKind::NoConflicts => Icon::new(UiIconName::CircleCheck)
          .size_5()
          .text_color(theme.status_green())
          .into_any_element(),
        OverviewConflictsKind::Conflicts => Icon::new(IconName::CircleX)
          .size_5()
          .text_color(theme.status_red())
          .into_any_element(),
        OverviewConflictsKind::OutOfDate => {
          Self::render_status_icon_dot(theme.status_yellow()).into_any_element()
        }
        OverviewConflictsKind::Merged => Icon::new(UiIconName::GitMerge)
          .size_5()
          .text_color(theme.status_violet())
          .into_any_element(),
      };

      let action = self.render_overview_conflicts_action(&conflicts.kind, cx);

      card = card.child(Self::render_overview_status_row(
        "github-pr-overview-conflicts-status",
        conflicts.title,
        &conflicts.message,
        icon,
        has_previous,
        action,
        &theme,
      ));
    }

    card.into_any_element()
  }

  pub(super) fn render_overview_status_row(
    id: &'static str,
    title: &str,
    message: &str,
    icon: impl IntoElement,
    has_border_top: bool,
    action: Option<impl IntoElement>,
    theme: &gpui_component::Theme,
  ) -> AnyElement {
    h_flex()
      .id(id)
      .w_full()
      .items_start()
      .justify_between()
      .gap_3()
      .p_3()
      .when(has_border_top, |this| {
        this.border_t_1().border_color(theme.border)
      })
      .child(
        h_flex()
          .min_w_0()
          .flex_1()
          .items_center()
          .gap_3()
          .child(
            div()
              .size(px(OVERVIEW_CHECKS_SUMMARY_RING_SIZE))
              .flex_shrink_0()
              .flex()
              .items_center()
              .justify_center()
              .child(icon),
          )
          .child(
            v_flex()
              .min_w_0()
              .flex_1()
              .child(
                div()
                  .font_medium()
                  .text_color(theme.foreground)
                  .child(title.to_string()),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .child(message.to_string()),
              ),
          ),
      )
      .when_some(action, |this, action| this.child(action))
      .into_any_element()
  }

  pub(super) fn render_status_icon_dot(color: Hsla) -> AnyElement {
    div()
      .size(px(16.0))
      .rounded_full()
      .border_2()
      .border_color(color)
      .into_any_element()
  }

  pub(super) fn render_overview_status_skeleton(theme: &gpui_component::Theme) -> AnyElement {
    let skeleton_row = |has_border_top: bool| {
      h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .p_3()
        .when(has_border_top, |this| {
          this.border_t_1().border_color(theme.border)
        })
        .child(
          Skeleton::new()
            .size(px(OVERVIEW_CHECKS_SUMMARY_RING_SIZE))
            .rounded_full(),
        )
        .child(
          v_flex()
            .gap_1()
            .child(
              Skeleton::new()
                .w(px(160.0))
                .h(px(16.0))
                .rounded(theme.radius),
            )
            .child(
              Skeleton::new()
                .w(px(120.0))
                .h(px(14.0))
                .rounded(theme.radius),
            ),
        )
    };

    v_flex()
      .id("github-pr-overview-status-skeleton")
      .w_full()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius_lg)
      .overflow_hidden()
      .child(skeleton_row(false))
      .child(skeleton_row(true))
      .into_any_element()
  }

  pub(super) fn render_overview_conflicts_action(
    &self,
    kind: &OverviewConflictsKind,
    cx: &Context<Self>,
  ) -> Option<AnyElement> {
    if matches!(
      self.local_project_availability(cx),
      GithubPrLocalProjectAvailability::Hidden
    ) {
      return None;
    }

    let label = match kind {
      OverviewConflictsKind::Conflicts => "Resolve conflicts",
      OverviewConflictsKind::OutOfDate => "Update branch",
      OverviewConflictsKind::NoConflicts | OverviewConflictsKind::Merged => return None,
    };

    let view = cx.entity().clone();
    Some(
      div()
        .flex_shrink_0()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
          Button::new("github-pr-overview-conflicts-action")
            .small()
            .primary()
            .with_variant(ButtonVariant::Secondary)
            .label(label)
            .disabled(self.local_branch_switch_loading || self.local_project_update_loading)
            .on_click(move |_, window, cx| {
              view.update(cx, |this, cx| {
                this.open_overview_pr_alert_in_shell(window, cx);
              });
            }),
        )
        .into_any_element(),
    )
  }

  pub(super) fn render_overview_checks_inner(
    &self,
    has_border_top: bool,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    if self.checks_loading && self.checks.is_none() {
      return h_flex()
        .id("github-pr-overview-checks-loading")
        .w_full()
        .items_center()
        .gap_3()
        .p_3()
        .when(has_border_top, |this| {
          this.border_t_1().border_color(theme.border)
        })
        .child(
          Skeleton::new()
            .size(px(OVERVIEW_CHECKS_SUMMARY_RING_SIZE))
            .rounded_full(),
        )
        .child(
          v_flex()
            .gap_1()
            .child(
              Skeleton::new()
                .w(px(180.0))
                .h(px(16.0))
                .rounded(theme.radius),
            )
            .child(
              Skeleton::new()
                .w(px(130.0))
                .h(px(14.0))
                .rounded(theme.radius),
            ),
        )
        .into_any_element();
    }

    if let Some(error) = self.checks_error.as_ref() {
      return h_flex()
        .id("github-pr-overview-checks-error")
        .w_full()
        .items_center()
        .gap_2()
        .p_4()
        .when(has_border_top, |this| {
          this.border_t_1().border_color(theme.border)
        })
        .child(
          Icon::new(IconName::CircleX)
            .size_4()
            .text_color(theme.status_red()),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme.status_red())
            .child(error.clone()),
        )
        .into_any_element();
    }

    if let Some(checks) = self.checks.as_ref() {
      return self.render_overview_checks_card(checks, has_border_top, theme, cx);
    }

    div().into_any_element()
  }

  pub(super) fn render_overview_checks_card(
    &self,
    checks: &GithubPullRequestChecksSummary,
    has_border_top: bool,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let rows = overview_check_rows(checks);
    let row_count = rows.len();
    let toggle_icon = if self.overview_checks_open {
      IconName::ChevronUp
    } else {
      IconName::ChevronDown
    };
    let toggle_label = if self.overview_checks_open {
      "Hide"
    } else {
      "Details"
    };
    let header_hover_bg = theme.muted.opacity(0.35);

    let inner = restrict_scroll_to_wheel_axis(
      v_flex()
        .id("github-pr-overview-checks-scroll")
        .w_full()
        .max_h(px(320.0))
        .overflow_y_scroll(),
    )
    .track_scroll(&self.overview_checks_scroll_handle)
    .when(row_count == 0, |this| {
      this.child(
        div()
          .px_4()
          .py_3()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child("GitHub has not reported individual check details."),
      )
    })
    .children(
      rows
        .into_iter()
        .enumerate()
        .map(|(ix, row)| self.render_overview_check_row(row, ix, row_count, theme)),
    );
    let inner_guarded = scrollable_node(
      inner,
      &self.overview_checks_scroll_handle,
      ScrollAxes::vertical(),
      OVERVIEW_CHECKS_SCROLL_GUARD_ID,
    );

    let content = div()
      .relative()
      .w_full()
      .border_t_1()
      .border_color(theme.border)
      .bg(theme.muted.opacity(0.45))
      .child(inner_guarded)
      .vertical_scrollbar(&self.overview_checks_scroll_handle);

    let header = h_flex()
      .id("github-pr-overview-checks-header")
      .w_full()
      .items_center()
      .justify_between()
      .gap_3()
      .p_3()
      .when(has_border_top, |this| {
        this.border_t_1().border_color(theme.border)
      })
      .cursor_pointer()
      .hover(move |this| this.bg(header_hover_bg))
      .on_click(cx.listener(|this, _, _, cx| {
        this.toggle_overview_checks(cx);
      }))
      .child(
        h_flex()
          .min_w_0()
          .items_center()
          .gap_3()
          .child(Self::render_overview_checks_summary_icon(checks, theme))
          .child(
            v_flex()
              .min_w_0()
              .child(
                div()
                  .font_medium()
                  .text_color(theme.foreground)
                  .overflow_hidden()
                  .text_ellipsis()
                  .child(overview_checks_summary_title(checks)),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .overflow_hidden()
                  .text_ellipsis()
                  .child(overview_checks_summary_subtitle(checks)),
              ),
          ),
      )
      .child(
        div()
          .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
          .child(
            Button::new("github-pr-overview-checks-toggle")
              .ghost()
              .xsmall()
              .compact()
              .label(toggle_label)
              .icon(toggle_icon)
              .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_overview_checks(cx);
              })),
          ),
      );

    v_flex()
      .id("github-pr-overview-checks")
      .w_full()
      .child(
        Collapsible::new()
          .open(self.overview_checks_open)
          .child(header)
          .content(content),
      )
      .into_any_element()
  }

  pub(super) fn toggle_overview_checks(&mut self, cx: &mut Context<Self>) {
    self.overview_checks_open = !self.overview_checks_open;
    cx.notify();
  }

  pub(super) fn render_overview_checks_summary_icon(
    checks: &GithubPullRequestChecksSummary,
    theme: &gpui_component::Theme,
  ) -> AnyElement {
    let slices = overview_checks_summary_slices(checks, theme);
    let segments = overview_checks_summary_segments(&slices);
    let caps = overview_checks_summary_caps(&segments);
    let segments_for_canvas = segments.clone();
    let center_icon = overview_checks_uniform_state(checks).map(|state| match state {
      GithubPullRequestChecksRollupState::Success => Icon::new(UiIconName::Check)
        .size_3()
        .text_color(theme.status_green())
        .into_any_element(),
      GithubPullRequestChecksRollupState::Pending => div()
        .size(px(6.0))
        .rounded_full()
        .bg(theme.status_orange())
        .into_any_element(),
      GithubPullRequestChecksRollupState::Failure => Icon::new(UiIconName::X)
        .size_3()
        .text_color(theme.status_red())
        .into_any_element(),
      GithubPullRequestChecksRollupState::Skipped => Icon::new(UiIconName::CircleSlash)
        .size_3()
        .text_color(theme.status_gray())
        .into_any_element(),
    });

    let ring_bg_color = theme.background;

    div()
      .relative()
      .size(px(OVERVIEW_CHECKS_SUMMARY_RING_SIZE))
      .flex_shrink_0()
      .child(
        canvas(
          |bounds, _, _| bounds,
          move |bounds, _, window, _| {
            let center_x = bounds.origin.x.as_f32() + bounds.size.width.as_f32() / 2.0;
            let center_y = bounds.origin.y.as_f32() + bounds.size.height.as_f32() / 2.0;

            // Single segment: draw a perfect ring with paint_quad to avoid
            // visible round cap seam at the stroke join point.
            if segments_for_canvas.len() == 1 {
              let segment = &segments_for_canvas[0];
              let stroke = px(OVERVIEW_CHECKS_SUMMARY_RING_STROKE_WIDTH);
              let radius = px(OVERVIEW_CHECKS_SUMMARY_RING_RADIUS);
              let center = point(px(center_x), px(center_y));
              let outer_r = radius + stroke / 2.;
              let inner_r = radius - stroke / 2.;
              let outer_bounds = gpui::Bounds::new(
                point(center.x - outer_r, center.y - outer_r),
                gpui::size(outer_r * 2., outer_r * 2.),
              );
              let inner_bounds = gpui::Bounds::new(
                point(center.x - inner_r, center.y - inner_r),
                gpui::size(inner_r * 2., inner_r * 2.),
              );
              window.paint_quad(gpui::fill(outer_bounds, segment.color).corner_radii(outer_r));
              window.paint_quad(gpui::fill(inner_bounds, ring_bg_color).corner_radii(inner_r));
              return;
            }

            for segment in &segments_for_canvas {
              let mut builder = PathBuilder::stroke(px(OVERVIEW_CHECKS_SUMMARY_RING_STROKE_WIDTH));
              let start_visual_angle = segment.start_angle - std::f32::consts::FRAC_PI_2;
              let end_visual_angle = segment.end_angle - std::f32::consts::FRAC_PI_2;
              let start = point(
                px(center_x + OVERVIEW_CHECKS_SUMMARY_RING_RADIUS * start_visual_angle.cos()),
                px(center_y + OVERVIEW_CHECKS_SUMMARY_RING_RADIUS * start_visual_angle.sin()),
              );
              let end = point(
                px(center_x + OVERVIEW_CHECKS_SUMMARY_RING_RADIUS * end_visual_angle.cos()),
                px(center_y + OVERVIEW_CHECKS_SUMMARY_RING_RADIUS * end_visual_angle.sin()),
              );
              let span = segment.end_angle - segment.start_angle;

              builder.move_to(start);
              if span >= std::f32::consts::TAU - 0.001 {
                let mid_angle = segment.start_angle + span / 2.0;
                let mid_visual_angle = mid_angle - std::f32::consts::FRAC_PI_2;
                let mid = point(
                  px(center_x + OVERVIEW_CHECKS_SUMMARY_RING_RADIUS * mid_visual_angle.cos()),
                  px(center_y + OVERVIEW_CHECKS_SUMMARY_RING_RADIUS * mid_visual_angle.sin()),
                );
                builder.arc_to(
                  point(
                    px(OVERVIEW_CHECKS_SUMMARY_RING_RADIUS),
                    px(OVERVIEW_CHECKS_SUMMARY_RING_RADIUS),
                  ),
                  px(0.0),
                  false,
                  true,
                  mid,
                );
                builder.arc_to(
                  point(
                    px(OVERVIEW_CHECKS_SUMMARY_RING_RADIUS),
                    px(OVERVIEW_CHECKS_SUMMARY_RING_RADIUS),
                  ),
                  px(0.0),
                  false,
                  true,
                  end,
                );
              } else {
                builder.arc_to(
                  point(
                    px(OVERVIEW_CHECKS_SUMMARY_RING_RADIUS),
                    px(OVERVIEW_CHECKS_SUMMARY_RING_RADIUS),
                  ),
                  px(0.0),
                  span.abs() > std::f32::consts::PI,
                  true,
                  end,
                );
              }

              if let Ok(path) = builder.build() {
                window.paint_path(path, segment.color);
              }
            }
          },
        )
        .absolute()
        .size_full(),
      )
      .children(caps.into_iter().map(|cap| {
        div()
          .absolute()
          .left(px(cap.left))
          .top(px(cap.top))
          .size(px(cap.size))
          .rounded_full()
          .bg(cap.color)
      }))
      .child(
        div()
          .absolute()
          .inset_0()
          .flex()
          .items_center()
          .justify_center()
          .child(
            div()
              .size(px(20.0))
              .rounded_full()
              .bg(theme.background)
              .flex()
              .items_center()
              .justify_center()
              .when_some(center_icon, |this, center_icon| this.child(center_icon)),
          ),
      )
      .into_any_element()
  }

  pub(super) fn render_overview_check_status_icon(
    state: GithubPullRequestChecksRollupState,
    theme: &gpui_component::Theme,
  ) -> AnyElement {
    match state {
      GithubPullRequestChecksRollupState::Success => div()
        .w(px(22.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
          Icon::new(UiIconName::CircleCheck)
            .size_4()
            .text_color(theme.status_green()),
        )
        .into_any_element(),
      GithubPullRequestChecksRollupState::Pending => div()
        .w(px(22.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(div().size(px(9.0)).rounded_full().bg(theme.status_orange()))
        .into_any_element(),
      GithubPullRequestChecksRollupState::Failure => div()
        .w(px(22.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
          Icon::new(IconName::CircleX)
            .size_4()
            .text_color(theme.status_red()),
        )
        .into_any_element(),
      GithubPullRequestChecksRollupState::Skipped => div()
        .w(px(22.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
          Icon::new(UiIconName::CircleSlash)
            .size_4()
            .text_color(theme.status_gray()),
        )
        .into_any_element(),
    }
  }

  pub(super) fn render_overview_check_app_icon(
    row: &OverviewCheckRow,
    theme: &gpui_component::Theme,
  ) -> AnyElement {
    let is_github_app = row
      .app_slug
      .as_deref()
      .is_some_and(|slug| slug.to_ascii_lowercase().contains("github"))
      || row
        .app_label
        .as_deref()
        .is_some_and(|label| label.to_ascii_lowercase().contains("github"));

    let base = div()
      .size(px(26.0))
      .flex_shrink_0()
      .flex()
      .items_center()
      .justify_center()
      .rounded(theme.radius)
      .border_1()
      .border_color(theme.border)
      .overflow_hidden()
      .bg(theme.background);

    if let Some(url) = row.app_avatar_url.as_deref().and_then(non_empty_owned) {
      base
        .child(img(url).size(px(24.0)).rounded(theme.radius))
        .into_any_element()
    } else if is_github_app {
      base
        .child(
          Icon::new(IconName::Github)
            .size_4()
            .text_color(theme.foreground),
        )
        .into_any_element()
    } else {
      base
        .child(
          div()
            .text_xs()
            .font_medium()
            .text_color(theme.muted_foreground)
            .child(overview_check_app_initial(row)),
        )
        .into_any_element()
    }
  }

  pub(super) fn render_overview_check_row(
    &self,
    row: OverviewCheckRow,
    ix: usize,
    row_count: usize,
    theme: &gpui_component::Theme,
  ) -> AnyElement {
    let row_id = row.id.clone();
    let title = row.title.clone();
    let status_label = row.status_label.clone();
    let open_url = row.open_url.clone();
    let hover_bg = theme.accent.opacity(0.55);

    let mut item = h_flex()
      .id(format!("github-pr-overview-check-row-{row_id}"))
      .w_full()
      .min_w_0()
      .items_center()
      .gap_3()
      .px_4()
      .py_2()
      .when(ix + 1 < row_count, |this| {
        this.border_b_1().border_color(theme.border.opacity(0.65))
      })
      .child(Self::render_overview_check_status_icon(row.state, theme))
      .child(Self::render_overview_check_app_icon(&row, theme))
      .child(
        h_flex()
          .min_w_0()
          .flex_1()
          .items_center()
          .justify_between()
          .gap_3()
          .overflow_hidden()
          .child(
            div()
              .min_w_0()
              .overflow_hidden()
              .text_ellipsis()
              .text_sm()
              .font_medium()
              .text_color(theme.foreground)
              .child(title),
          )
          .when_some(status_label, |this, status_label| {
            this.child(
              div()
                .flex_shrink_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(status_label),
            )
          }),
      );

    if let Some(url) = open_url
      .map(|value| value.trim().to_string())
      .filter(|value| !value.is_empty())
    {
      item = item
        .cursor_pointer()
        .hover(move |this| this.bg(hover_bg))
        .on_click(move |_, _, cx| {
          cx.open_url(&url);
        });
    }

    item.into_any_element()
  }

  pub(super) fn build_overview_composer_markdown_options(
    &self,
    scope_offset: usize,
    cx: &mut Context<Self>,
  ) -> MarkdownRenderOptions {
    let pr_page_for_links = cx.entity().clone();
    let link_handler = Arc::new(move |url: &str, window: &mut Window, cx: &mut App| {
      let handled = pr_page_for_links.update(cx, |this, cx| this.handle_gfm_link(url, window, cx));
      if handled {
        LinkAction::Handled
      } else {
        LinkAction::Open
      }
    });

    let mut options = MarkdownRenderOptions::with_on_link(link_handler)
      .with_state(self.description_markdown_state.clone())
      .with_syntax_cache(self.syntax_highlight_cache.clone())
      .with_asset_url_resolver(github_shared::make_asset_url_resolver(&self.api))
      .with_hardbreaks();

    if let Some(pr) = self.pull_request.as_ref() {
      let scope_id = (pr.number as usize)
        .wrapping_mul(1_000_003)
        .wrapping_add(scope_offset);
      options = options
        .with_github_issue_reference_context(
          pr.repository.owner.as_str(),
          pr.repository.repo.as_str(),
        )
        .with_scope_id(scope_id);
    }

    options
  }

  pub(super) fn render_overview_skeleton(&self, theme: &gpui_component::Theme) -> AnyElement {
    let max_w = px(1100.);

    // Author row skeleton
    let author_row = h_flex()
      .gap_2()
      .items_center()
      .child(Skeleton::new().size(px(24.0)).rounded_full())
      .child(
        Skeleton::new()
          .w(px(100.0))
          .h(px(16.0))
          .rounded(theme.radius),
      )
      .child(
        Skeleton::new()
          .w(px(120.0))
          .h(px(16.0))
          .rounded(theme.radius),
      )
      .child(
        Skeleton::new()
          .w(px(110.0))
          .h(px(16.0))
          .rounded(theme.radius),
      );

    // Created / Updated row skeleton
    let created_updated = h_flex()
      .gap_2()
      .items_center()
      .child(
        Skeleton::new()
          .w(px(50.0))
          .h(px(14.0))
          .rounded(theme.radius),
      )
      .child(
        Skeleton::new()
          .w(px(80.0))
          .h(px(16.0))
          .rounded(theme.radius),
      )
      .child(
        div()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child("•"),
      )
      .child(
        Skeleton::new()
          .w(px(55.0))
          .h(px(14.0))
          .rounded(theme.radius),
      )
      .child(
        Skeleton::new()
          .w(px(60.0))
          .h(px(16.0))
          .rounded(theme.radius),
      )
      .child(
        div()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child("•"),
      )
      .child(
        Skeleton::new()
          .w(px(50.0))
          .h(px(16.0))
          .rounded(theme.radius),
      );

    // Source → Target row skeleton
    let source_target = h_flex()
      .gap_2()
      .items_center()
      .child(
        Skeleton::new()
          .w(px(40.0))
          .h(px(14.0))
          .rounded(theme.radius),
      )
      .child(
        Skeleton::new()
          .w(px(160.0))
          .h(px(16.0))
          .rounded(theme.radius),
      )
      .child(
        Icon::new(IconName::ArrowRight)
          .size_3()
          .text_color(theme.muted_foreground),
      )
      .child(
        Skeleton::new()
          .w(px(40.0))
          .h(px(14.0))
          .rounded(theme.radius),
      )
      .child(
        Skeleton::new()
          .w(px(60.0))
          .h(px(16.0))
          .rounded(theme.radius),
      );

    // Description skeleton
    let description = v_flex()
      .gap_2()
      .child(
        Skeleton::new()
          .w(px(90.0))
          .h(px(16.0))
          .rounded(theme.radius),
      )
      .child(
        v_flex()
          .gap_2()
          .border_1()
          .border_color(theme.border)
          .rounded(theme.radius)
          .p_3()
          .child(Skeleton::new().w_full().h(px(14.0)).rounded(theme.radius))
          .child(
            Skeleton::new()
              .w(px(300.0))
              .h(px(14.0))
              .rounded(theme.radius),
          )
          .child(
            Skeleton::new()
              .w(px(200.0))
              .h(px(14.0))
              .rounded(theme.radius),
          ),
      );

    // Status section skeleton
    let status_section = v_flex()
      .w_full()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius_lg)
      .overflow_hidden()
      .child(
        h_flex()
          .w_full()
          .items_center()
          .gap_3()
          .p_3()
          .child(Skeleton::new().size(px(36.0)).rounded_full())
          .child(
            v_flex()
              .gap_1()
              .child(
                Skeleton::new()
                  .w(px(160.0))
                  .h(px(16.0))
                  .rounded(theme.radius),
              )
              .child(
                Skeleton::new()
                  .w(px(120.0))
                  .h(px(14.0))
                  .rounded(theme.radius),
              ),
          ),
      )
      .child(
        h_flex()
          .w_full()
          .items_center()
          .gap_3()
          .p_3()
          .border_t_1()
          .border_color(theme.border)
          .child(
            div()
              .size(px(36.0))
              .flex_shrink_0()
              .flex()
              .items_center()
              .justify_center()
              .child(Skeleton::new().size(px(20.0)).rounded_full()),
          )
          .child(
            v_flex()
              .gap_1()
              .child(
                Skeleton::new()
                  .w(px(200.0))
                  .h(px(16.0))
                  .rounded(theme.radius),
              )
              .child(
                Skeleton::new()
                  .w(px(260.0))
                  .h(px(14.0))
                  .rounded(theme.radius),
              ),
          ),
      );

    let ai_brief_section = v_flex()
      .w_full()
      .gap_3()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius_lg)
      .p_3()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_3()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(Skeleton::new().size(px(16.0)).rounded(theme.radius))
              .child(
                Skeleton::new()
                  .w(px(60.0))
                  .h(px(14.0))
                  .rounded(theme.radius),
              ),
          )
          .child(
            Skeleton::new()
              .w(px(80.0))
              .h(px(24.0))
              .rounded(theme.radius),
          ),
      );

    div()
      .px_10()
      .pb_4()
      .child(
        v_flex()
          .mx_auto()
          .max_w(max_w)
          .pt(px(30.))
          .gap_4()
          .child({
            let left_meta = v_flex()
              .gap_2()
              .child(author_row)
              .child(created_updated)
              .child(source_target);

            let right_people = v_flex()
              .gap_3()
              .items_end()
              .child(
                v_flex()
                  .gap_1()
                  .items_end()
                  .child(
                    Skeleton::new()
                      .w(px(80.0))
                      .h(px(14.0))
                      .rounded(theme.radius),
                  )
                  .child(
                    Skeleton::new()
                      .w(px(100.0))
                      .h(px(14.0))
                      .rounded(theme.radius),
                  ),
              )
              .child(
                v_flex()
                  .gap_1()
                  .items_end()
                  .child(
                    Skeleton::new()
                      .w(px(75.0))
                      .h(px(14.0))
                      .rounded(theme.radius),
                  )
                  .child(
                    Skeleton::new()
                      .w(px(100.0))
                      .h(px(14.0))
                      .rounded(theme.radius),
                  ),
              );

            div()
              .flex()
              .justify_between()
              .gap_6()
              .child(left_meta)
              .child(right_people)
          })
          .child(description)
          .child(ai_brief_section)
          .child(status_section),
      )
      .into_any_element()
  }

  pub(super) fn render_details(
    &mut self,
    pr: &GithubPullRequestDetails,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let repo_label = github_shared::repo_label(&pr.repository.owner, &pr.repository.repo);
    let repo_owner = pr.repository.owner.clone();
    let repo_name = pr.repository.repo.clone();
    let updated_at = format_relative_time(&pr.updated_at);
    let created_at = format_relative_time(&pr.created_at);
    let merged_at = pr.merged_at.as_deref().map(format_relative_time);

    let pr_page = cx.entity().clone();
    let can_edit_people = self.can_edit_people(pr);
    let can_edit_labels = self.can_edit_labels(pr);
    let author_login = pr.author.login.clone();
    let assignee_suggestions = matching_filter_option_users(
      &self.review_people_options,
      &self.assignee_query(cx),
      &pr.assignees,
    );
    let reviewer_suggestions = matching_filter_option_users(
      &self.review_people_options,
      &self.requested_reviewer_query(cx),
      &pr.requested_reviewers,
    )
    .into_iter()
    .filter(|user| {
      !github_shared::logins_match_case_insensitive(user.login.as_str(), author_login.as_str())
    })
    .collect::<Vec<_>>();
    let label_suggestions =
      matching_label_options(&self.label_options, &self.label_query(cx), &pr.labels);
    let can_edit_target_branch = self.can_edit_target_branch(pr);

    let content =
      v_flex()
        .w_full()
        .gap_4()
        .child({
          let author_row = h_flex()
            .gap_2()
            .items_center()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(
              h_flex()
                .items_center()
                .gap_2()
                .child(
                  Avatar::new()
                    .name(pr.author.login.clone())
                    .when_some(pr.author.avatar_url.clone(), |this, url| this.src(url))
                    .small(),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(pr.author.login.clone()),
                ),
            )
            .child(
              Button::new("open-pr-repo-details")
                .ghost()
                .small()
                .compact()
                .label(repo_label)
                .on_click(move |_, _, cx| {
                  open_repo_target(repo_owner.clone(), repo_name.clone(), None, None, None, cx);
                }),
            );

          let created_updated = h_flex()
            .gap_2()
            .flex_wrap()
            .items_center()
            .child(
              h_flex()
                .items_center()
                .gap_2()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Created"),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(created_at),
                ),
            )
            .child(
              div()
                .debug_selector(|| "github-pr-overview-created-updated-separator".to_string())
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("•"),
            )
            .child(
              h_flex()
                .items_center()
                .gap_2()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Updated"),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(updated_at),
                )
                .child(
                  div()
                    .debug_selector(|| {
                      "github-pr-overview-updated-change-stats-separator".to_string()
                    })
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("•"),
                )
                .child(
                  h_flex()
                    .debug_selector(|| "github-pr-overview-updated-change-stats".to_string())
                    .items_center()
                    .gap_2()
                    .children(overview_change_stats(pr, &theme)),
                ),
            )
            .when_some(merged_at.clone(), |this, merged| {
              this.child(
                h_flex()
                  .items_center()
                  .gap_2()
                  .child(
                    div()
                      .text_sm()
                      .text_color(theme.muted_foreground)
                      .child("Merged"),
                  )
                  .child(
                    div()
                      .text_sm()
                      .text_color(theme.foreground)
                      .child(merged.to_string()),
                  ),
              )
            });

          let target_branch_select = v_flex()
            .gap_1()
            .child(
              Select::new(&self.target_branch_select)
                .placeholder(pr.base_ref_name.clone())
                .search_placeholder("Search branches...")
                .xsmall()
                .w(px(220.0))
                .menu_width(px(280.0))
                .disabled(!can_edit_target_branch),
            )
            .when_some(self.target_branch_error.clone(), |this, error| {
              this.child(div().text_xs().text_color(theme.status_red()).child(error))
            })
            .when_some(self.target_branch_update_error.clone(), |this, error| {
              this.child(div().text_xs().text_color(theme.status_red()).child(error))
            });

          let source_target = h_flex().items_center().gap_2().child(
            h_flex()
              .items_center()
              .gap_2()
              .child(
                h_flex()
                  .items_center()
                  .gap_1()
                  .child(
                    div()
                      .text_xs()
                      .text_color(theme.muted_foreground)
                      .child("Source"),
                  )
                  .child(
                    div()
                      .text_sm()
                      .text_color(theme.foreground)
                      .child(pr.head_ref_name.clone()),
                  )
                  .child(Clipboard::new("copy-pr-branch-source").value(pr.head_ref_name.clone())),
              )
              .child(
                Icon::new(IconName::ArrowRight)
                  .size_3()
                  .text_color(theme.muted_foreground),
              )
              .child(
                h_flex()
                  .items_center()
                  .gap_1()
                  .child(
                    div()
                      .text_xs()
                      .text_color(theme.muted_foreground)
                      .child("Target"),
                  )
                  .child(target_branch_select),
              ),
          );

          let labels_popover =
            |trigger| {
              Popover::new("pr-labels-popover")
                .anchor(Anchor::TopRight)
                .open(self.label_popover_open)
                .on_open_change(cx.listener(|this, open, _window, cx| {
                  this.label_popover_open = *open;
                  if *open {
                    this.label_mutation_error = None;
                  }
                  cx.notify();
                }))
                .trigger(trigger)
                .child(
                  v_flex()
                    .w(px(260.0))
                    .gap_2()
                    .when(!pr.labels.is_empty(), |this| {
                      this.child(h_flex().gap_1().flex_wrap().children(pr.labels.iter().map(
                        |label| {
                          let page = pr_page.clone();
                          let label_name = label.name.clone();
                          h_flex()
                            .items_center()
                            .gap_1()
                            .child(github_shared::github_label_tag(label, &theme))
                            .child(
                              Button::new(format!("pr-label-remove-{}", label_name))
                                .ghost()
                                .xsmall()
                                .compact()
                                .icon(IconName::Close)
                                .disabled(!can_edit_labels)
                                .on_click(move |_, _, cx| {
                                  page.update(cx, |this, cx| {
                                    this.remove_label(&label_name, cx);
                                  });
                                }),
                            )
                        },
                      )))
                    })
                    .child(
                      Input::new(&self.label_input)
                        .w_full()
                        .disabled(!can_edit_labels || self.label_options_loading),
                    )
                    .when(!label_suggestions.is_empty(), |this| {
                      this.child(h_flex().gap_1().flex_wrap().children(
                        label_suggestions.iter().map(|label| {
                          let page = pr_page.clone();
                          let label_name = label.name.clone();
                          Button::new(format!("pr-label-suggestion-{}", label.name))
                            .label(label.name.clone())
                            .xsmall()
                            .outline()
                            .on_click(move |_, window, cx| {
                              page.update(cx, |this, cx| {
                                this.add_label_value(&label_name, window, cx);
                              });
                            })
                        }),
                      ))
                    }),
                )
            };

          let label_block = v_flex()
            .pt_2()
            .gap_1()
            .when(self.label_options_loading, |this| {
              this.child(Self::render_label_skeleton_row(3))
            })
            .when(
              !self.label_options_loading && !pr.labels.is_empty(),
              |this| {
                this.child(
                  h_flex()
                    .debug_selector(|| "github-pr-overview-labels".to_string())
                    .gap_1()
                    .flex_wrap()
                    .children(
                      pr.labels
                        .iter()
                        .map(|label| github_shared::github_label_tag(label, &theme)),
                    )
                    .child(labels_popover(
                      Button::new("pr-labels-edit-inline")
                        .label("Edit labels")
                        .xsmall()
                        .outline()
                        .icon(UiIconName::SquarePen)
                        .disabled(!can_edit_labels),
                    )),
                )
              },
            )
            .when(
              !self.label_options_loading && pr.labels.is_empty(),
              |this| {
                this.child(
                  div().flex().child(labels_popover(
                    Button::new("pr-labels-edit-inline-empty")
                      .label("Add labels")
                      .xsmall()
                      .outline()
                      .icon(UiIconName::SquarePen)
                      .disabled(!can_edit_labels),
                  )),
                )
              },
            )
            .when_some(self.label_mutation_error.clone(), |this, error| {
              this.child(div().text_xs().text_color(theme.status_red()).child(error))
            });
          let label_block = label_block
            .when_some(self.label_options_error.clone(), |this, error| {
              this.child(div().text_xs().text_color(theme.status_red()).child(error))
            });

          let left_meta = v_flex()
            .gap_2()
            .child(author_row)
            .child(created_updated)
            .child(source_target)
            .child(label_block);

          let reviewers_popover = Popover::new("pr-reviewers-popover")
            .anchor(Anchor::TopRight)
            .open(self.reviewer_popover_open)
            .on_open_change(cx.listener(|this, open, _window, cx| {
              this.reviewer_popover_open = *open;
              if *open {
                this.people_mutation_error = None;
              }
              cx.notify();
            }))
            .trigger(
              Button::new("pr-reviewers-edit")
                .ghost()
                .xsmall()
                .compact()
                .icon(UiIconName::SquarePen)
                .disabled(!can_edit_people),
            )
            .child(
              v_flex()
                .w(px(260.0))
                .gap_2()
                .when(!pr.requested_reviewers.is_empty(), |this| {
                  this.child(h_flex().gap_1().flex_wrap().children(
                    pr.requested_reviewers.iter().map(|user| {
                      let page = pr_page.clone();
                      let login = user.login.clone();
                      h_flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py_1()
                        .rounded_full()
                        .child(
                          Avatar::new()
                            .name(login.clone())
                            .when_some(user.avatar_url.clone(), |this, url| this.src(url))
                            .xsmall(),
                        )
                        .child(div().text_sm().child(login.clone()))
                        .child(
                          Button::new(format!("pr-reviewer-remove-{}", login))
                            .ghost()
                            .xsmall()
                            .compact()
                            .icon(IconName::Close)
                            .on_click(move |_, _, cx| {
                              page.update(cx, |this, cx| {
                                this.remove_requested_reviewer(&login, cx);
                              });
                            }),
                        )
                    }),
                  ))
                })
                .child(
                  Input::new(&self.requested_reviewer_input)
                    .w_full()
                    .disabled(!can_edit_people || self.review_people_options_loading),
                )
                .when(!reviewer_suggestions.is_empty(), |this| {
                  this.child(h_flex().gap_1().flex_wrap().children(
                    reviewer_suggestions.into_iter().map(|reviewer| {
                      Button::new(format!("pr-reviewer-suggestion-{}", reviewer.login))
                        .label(reviewer.login.clone())
                        .xsmall()
                        .outline()
                        .on_click({
                          let page = pr_page.clone();
                          let login = reviewer.login.clone();
                          move |_, window, cx| {
                            page.update(cx, |this, cx| {
                              this.add_requested_reviewer_value(&login, window, cx);
                            });
                          }
                        })
                    }),
                  ))
                }),
            );

          let assignees_popover = Popover::new("pr-assignees-popover")
            .anchor(Anchor::TopRight)
            .open(self.assignee_popover_open)
            .on_open_change(cx.listener(|this, open, _window, cx| {
              this.assignee_popover_open = *open;
              if *open {
                this.people_mutation_error = None;
              }
              cx.notify();
            }))
            .trigger(
              Button::new("pr-assignees-edit")
                .ghost()
                .xsmall()
                .compact()
                .icon(UiIconName::SquarePen)
                .disabled(!can_edit_people),
            )
            .child(
              v_flex()
                .w(px(260.0))
                .gap_2()
                .when(!pr.assignees.is_empty(), |this| {
                  this.child(
                    h_flex()
                      .gap_1()
                      .flex_wrap()
                      .children(pr.assignees.iter().map(|user| {
                        let page = pr_page.clone();
                        let login = user.login.clone();
                        h_flex()
                          .items_center()
                          .gap_1()
                          .px_2()
                          .py_1()
                          .rounded_full()
                          .child(
                            Avatar::new()
                              .name(login.clone())
                              .when_some(user.avatar_url.clone(), |this, url| this.src(url))
                              .xsmall(),
                          )
                          .child(div().text_sm().child(login.clone()))
                          .child(
                            Button::new(format!("pr-assignee-remove-{}", login))
                              .ghost()
                              .xsmall()
                              .compact()
                              .icon(IconName::Close)
                              .on_click(move |_, _, cx| {
                                page.update(cx, |this, cx| {
                                  this.remove_assignee(&login, cx);
                                });
                              }),
                          )
                      })),
                  )
                })
                .child(
                  Input::new(&self.assignee_input)
                    .w_full()
                    .disabled(!can_edit_people || self.review_people_options_loading),
                )
                .when(!assignee_suggestions.is_empty(), |this| {
                  this.child(h_flex().gap_1().flex_wrap().children(
                    assignee_suggestions.into_iter().map(|assignee| {
                      Button::new(format!("pr-assignee-suggestion-{}", assignee.login))
                        .label(assignee.login.clone())
                        .xsmall()
                        .outline()
                        .on_click({
                          let page = pr_page.clone();
                          let login = assignee.login.clone();
                          move |_, window, cx| {
                            page.update(cx, |this, cx| {
                              this.add_assignee_value(&login, window, cx);
                            });
                          }
                        })
                    }),
                  ))
                }),
            );

          let merged_reviewers = merged_reviewers(
            &pr.requested_reviewers,
            &self.reviews,
            author_login.as_str(),
          );
          let reviewer_block = v_flex()
            .gap_1()
            .child(
              h_flex()
                .items_center()
                .gap_1()
                .justify_between()
                .child(div().text_sm().child("Reviewers"))
                .child(reviewers_popover),
            )
            .when(self.review_people_options_loading, |this| {
              this.child(Self::render_people_skeleton_row(3))
            })
            .when(
              !self.review_people_options_loading && !merged_reviewers.is_empty(),
              |this| {
                this.child(Self::render_requested_reviewer_row(
                  &merged_reviewers,
                  &self.reviews,
                  self
                    .pull_request
                    .as_ref()
                    .map(|pr| pr.requested_reviewers.as_slice())
                    .unwrap_or(&[]),
                  &theme,
                ))
              },
            )
            .when(
              !self.review_people_options_loading && merged_reviewers.is_empty(),
              |this| {
                this.child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("No reviewers"),
                )
              },
            );

          let assignee_block = v_flex()
            .gap_1()
            .child(
              h_flex()
                .items_center()
                .gap_1()
                .justify_between()
                .child(div().text_sm().child("Assignees"))
                .child(assignees_popover),
            )
            .when(self.review_people_options_loading, |this| {
              this.child(Self::render_people_skeleton_row(3))
            })
            .when(
              !self.review_people_options_loading && !pr.assignees.is_empty(),
              |this| {
                this.child(Self::render_people_token_row(
                  "github-pr-assignee",
                  &pr.assignees,
                  false,
                  |_login, _, _| {},
                ))
              },
            )
            .when(
              !self.review_people_options_loading && pr.assignees.is_empty(),
              |this| {
                this.child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("No assignees"),
                )
              },
            );

          let right_people = v_flex()
            .gap_3()
            .items_end()
            .child(reviewer_block)
            .child(assignee_block)
            .when_some(self.review_people_options_error.clone(), |this, error| {
              this.child(div().text_xs().text_color(theme.status_red()).child(error))
            })
            .when_some(self.people_mutation_error.clone(), |this, error| {
              this.child(div().text_xs().text_color(theme.status_red()).child(error))
            });

          div()
            .flex()
            .justify_between()
            .gap_6()
            .child(left_meta)
            .child(right_people)
        })
        .child(
          h_flex()
            .items_center()
            .justify_between()
            .gap_2()
            .p_3()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Description and conversation live on GitHub."),
            )
            .child(
              Button::new("github-pr-overview-open-on-github")
                .debug_selector(|| "github-pr-overview-open-on-github".to_string())
                .with_variant(ButtonVariant::Secondary)
                .outline()
                .small()
                .icon(IconName::ExternalLink)
                .label("Open on GitHub")
                .on_click({
                  let url =
                    github_shared::pr_url(&pr.repository.owner, &pr.repository.repo, pr.number);
                  move |_, _, cx| {
                    cx.open_url(&url);
                  }
                }),
            ),
        )
        .when_some(self.render_review_requested_alert(cx), |this, alert| {
          this.child(alert)
        })
        .child(self.render_overview_status_section(cx));

    div()
      .id("github-pr-overview-scroll")
      .size_full()
      .overflow_y_scroll()
      .px_10()
      .py_8()
      .child(div().mx_auto().max_w(px(1100.)).child(content))
      .into_any_element()
  }

  pub(super) fn open_overview_pr_alert_in_shell(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(kind) = overview_pr_alert_kind(self.merge_readiness.as_ref(), self.checks.as_ref())
    else {
      return;
    };
    if matches!(kind, OverviewPrAlertKind::Blocked) {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let post_action = GithubPrLocalProjectPostAction::MergeBaseInWorkspace {
      base_branch_name: pull_request.base_ref_name.clone(),
    };
    let has_uncommitted_changes = self.effective_local_repo_has_uncommitted_changes(cx);

    match self.local_project_availability(cx) {
      GithubPrLocalProjectAvailability::Hidden => {}
      GithubPrLocalProjectAvailability::NeedsBranchSwitch {
        has_uncommitted_changes,
        ..
      } => {
        let post_action = Some(
          GithubPrLocalProjectPostAction::EnsurePrHeadThenMergeBaseInWorkspace {
            base_branch_name: pull_request.base_ref_name.clone(),
          },
        );
        if has_uncommitted_changes {
          self.confirm_switch_local_branch_with_stash(post_action, window, cx);
        } else {
          self.switch_local_branch_to_pr_branch(false, post_action, window, cx);
        }
      }
      GithubPrLocalProjectAvailability::Ready { repo_root } => {
        if should_prepare_local_branch_before_merging_base(
          repo_root.as_path(),
          has_uncommitted_changes,
        ) {
          self.confirm_prepare_local_branch_with_stash(repo_root, post_action, window, cx);
        } else {
          self.execute_local_project_post_action(post_action, repo_root, cx);
        }
      }
      GithubPrLocalProjectAvailability::NeedsUpdate { .. } => {
        self.update_local_branch_to_pr_head(false, Some(post_action), window, cx);
      }
      GithubPrLocalProjectAvailability::Dirty { repo_root } => {
        if should_prepare_local_branch_before_merging_base(repo_root.as_path(), true) {
          self.confirm_prepare_local_branch_with_stash(repo_root, post_action, window, cx);
        } else {
          self.execute_local_project_post_action(post_action, repo_root, cx);
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::super::*;
  use super::*;
  use crate::api::{
    GithubPullRequestCheckRun, GithubPullRequestLegacyStatus, GithubPullRequestReviewUser,
    GithubPullRequestWorkflowJob, GithubPullRequestWorkflowRun,
  };
  use git::{BranchKind, BranchRef, merge_branch};

  #[test]
  fn overview_change_stat_labels_are_compact() {
    let pr = make_pr_details_for_stats();
    let labels = overview_change_stat_labels(&pr);

    assert_eq!(labels, ["+20".to_string(), "-4".to_string()]);
  }

  #[test]
  fn overview_check_rows_keep_provider_avatar_urls() {
    let mut checks = make_checks_summary();
    checks.missing_required_contexts.clear();
    checks.actions_runs = vec![GithubPullRequestWorkflowRun {
      id: 100,
      name: Some("CI".to_string()),
      display_title: Some("build branch".to_string()),
      event: "pull_request".to_string(),
      status: Some("completed".to_string()),
      conclusion: Some("success".to_string()),
      state: GithubPullRequestChecksRollupState::Success,
      created_at: "2026-03-19T10:00:00Z".to_string(),
      updated_at: "2026-03-19T10:02:00Z".to_string(),
      run_started_at: Some("2026-03-19T10:00:00Z".to_string()),
      run_number: 12,
      run_attempt: Some(1),
      html_url: Some("https://github.com/acme/widget/actions/runs/100".to_string()),
      jobs: vec![GithubPullRequestWorkflowJob {
        id: 200,
        name: "build".to_string(),
        status: Some("completed".to_string()),
        conclusion: Some("success".to_string()),
        state: GithubPullRequestChecksRollupState::Success,
        started_at: Some("2026-03-19T10:00:00Z".to_string()),
        completed_at: Some("2026-03-19T10:02:00Z".to_string()),
        html_url: Some("https://github.com/acme/widget/actions/runs/100/job/200".to_string()),
        required: true,
        app_name: Some("GitHub Actions".to_string()),
        app_slug: Some("github-actions".to_string()),
        app_avatar_url: Some("https://avatars.githubusercontent.com/in/15368?v=4".to_string()),
        steps: Vec::new(),
      }],
    }];
    checks.other_checks = vec![GithubPullRequestCheckRun {
      id: 301,
      name: "lint".to_string(),
      status: Some("completed".to_string()),
      conclusion: Some("failure".to_string()),
      state: GithubPullRequestChecksRollupState::Failure,
      started_at: Some("2026-03-19T10:01:00Z".to_string()),
      completed_at: Some("2026-03-19T10:03:00Z".to_string()),
      html_url: Some("https://github.com/acme/widget/runs/301".to_string()),
      details_url: Some("https://github.com/acme/widget/runs/301".to_string()),
      required: true,
      app_name: Some("Reviewdog".to_string()),
      app_slug: Some("reviewdog".to_string()),
      app_avatar_url: Some("https://avatars.githubusercontent.com/u/15138054?v=4".to_string()),
      title: Some("Lint".to_string()),
      summary: Some("Lint failed".to_string()),
      text: None,
      annotations_count: 2,
    }];
    checks.legacy_statuses = vec![GithubPullRequestLegacyStatus {
      id: 401,
      context: "security/brakeman".to_string(),
      status: "success".to_string(),
      state: GithubPullRequestChecksRollupState::Success,
      description: Some("Security checks passed".to_string()),
      target_url: Some("https://ci.example.com/401".to_string()),
      avatar_url: Some("https://ci.example.com/avatar.png".to_string()),
      created_at: "2026-03-19T10:00:00Z".to_string(),
      updated_at: "2026-03-19T10:04:00Z".to_string(),
      required: false,
    }];

    let rows = overview_check_rows(&checks);

    assert_eq!(
      rows
        .iter()
        .find(|row| row.id == "workflow-job-200")
        .and_then(|row| row.app_avatar_url.as_deref()),
      Some("https://avatars.githubusercontent.com/in/15368?v=4")
    );
    assert_eq!(
      rows
        .iter()
        .find(|row| row.id == "check-run-301")
        .and_then(|row| row.app_avatar_url.as_deref()),
      Some("https://avatars.githubusercontent.com/u/15138054?v=4")
    );
    assert_eq!(
      rows
        .iter()
        .find(|row| row.id == "legacy-status-401")
        .and_then(|row| row.app_avatar_url.as_deref()),
      Some("https://ci.example.com/avatar.png")
    );
  }

  #[test]
  fn overview_check_rows_prefix_workflow_name_with_event_suffix() {
    let mut checks = make_checks_summary();
    checks.missing_required_contexts.clear();
    checks.actions_runs = vec![GithubPullRequestWorkflowRun {
      id: 100,
      name: Some("CI".to_string()),
      display_title: Some("CI".to_string()),
      event: "pull_request".to_string(),
      status: Some("completed".to_string()),
      conclusion: Some("success".to_string()),
      state: GithubPullRequestChecksRollupState::Success,
      created_at: "2026-04-25T10:00:00Z".to_string(),
      updated_at: "2026-04-25T10:02:00Z".to_string(),
      run_started_at: Some("2026-04-25T10:00:00Z".to_string()),
      run_number: 12,
      run_attempt: Some(1),
      html_url: Some("https://github.com/acme/widget/actions/runs/100".to_string()),
      jobs: vec![GithubPullRequestWorkflowJob {
        id: 200,
        name: "Frontend (build)".to_string(),
        status: Some("completed".to_string()),
        conclusion: Some("success".to_string()),
        state: GithubPullRequestChecksRollupState::Success,
        started_at: Some("2026-04-25T10:00:00Z".to_string()),
        completed_at: Some("2026-04-25T10:02:00Z".to_string()),
        html_url: None,
        required: false,
        app_name: Some("GitHub Actions".to_string()),
        app_slug: Some("github-actions".to_string()),
        app_avatar_url: None,
        steps: Vec::new(),
      }],
    }];

    let rows = overview_check_rows(&checks);
    let row = rows
      .iter()
      .find(|row| row.id == "workflow-job-200")
      .expect("workflow job row");

    assert_eq!(row.title, "CI / Frontend (build) (pull_request)");
    assert_eq!(row.status_label.as_deref(), Some("Successful in 2m"));
  }

  #[test]
  fn overview_check_status_label_formats_skipped_and_success_states() {
    assert_eq!(
      overview_check_status_label(
        GithubPullRequestChecksRollupState::Success,
        Some("2026-04-25T10:00:00Z"),
        Some("2026-04-25T10:00:07Z"),
      )
      .as_deref(),
      Some("Successful in 7s"),
    );
    assert!(
      overview_check_status_label(
        GithubPullRequestChecksRollupState::Skipped,
        Some("2026-04-24T10:00:00Z"),
        Some("2026-04-24T10:00:00Z"),
      )
      .unwrap()
      .starts_with("Skipped "),
    );
  }

  #[test]
  fn overview_checks_summary_subtitle_lists_skipped_alongside_success() {
    let mut checks = make_checks_summary();
    checks.total_checks = 31;
    checks.successful_checks = 15;
    checks.skipped_checks = 16;
    checks.failed_checks = 0;
    checks.pending_checks = 0;
    checks.overall_state = GithubPullRequestChecksRollupState::Success;

    assert_eq!(
      overview_checks_summary_subtitle(&checks),
      "16 skipped, 15 successful checks"
    );
  }

  #[gpui::test]
  fn overview_conflicts_action_lands_in_the_shell_when_conflict_resolution_is_already_active(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/github/acme/widget/pull/42", cx);
    });

    let (repo_root, _) =
      create_local_repo_with_github_remote("acme", "widget", "feature", &["main"]);
    let rel_path = Path::new("src/main.rs");
    commit_local_project_file(
      &repo_root,
      rel_path,
      "fn main() {\n  println!(\"feature\");\n}\n",
      "feature change",
    );
    switch_to_branch_name(&repo_root, "main").expect("switch to main");
    commit_local_project_file(
      &repo_root,
      rel_path,
      "fn main() {\n  println!(\"main\");\n}\n",
      "main change",
    );
    switch_to_branch_name(&repo_root, "feature").expect("switch back to feature");

    let merge_result = merge_branch(
      &repo_root,
      &BranchRef {
        name: "main".to_string(),
        kind: BranchKind::Local,
      },
    );
    assert!(merge_result.is_err(), "merge should stop on conflicts");
    assert!(local_repo_has_active_conflict_resolution(&repo_root));

    let snapshot = local_repo_snapshot(&repo_root, None).expect("snapshot conflicted repo");
    let head_sha = snapshot
      .head_sha
      .clone()
      .expect("feature branch head should stay available");
    assert!(snapshot.has_uncommitted_changes);

    cx.update(|cx| {
      ActiveLocalRepoStore::set(cx, Some(snapshot));
    });
    let (page, cx) = cx.add_window_view(|window, cx| GithubPrDetailsPage::new(window, cx));

    page.update_in(cx, |this, _window, cx| {
      this.pull_request = Some(make_pr_details_for_local_repo(&head_sha, "feature"));
      this.merge_readiness = Some(make_merge_readiness_with_state(
        GithubPullRequestMergeReadinessStatus::Blocked,
        Some("dirty"),
        "This pull request has merge conflicts that must be resolved before it can be merged.",
      ));
      cx.notify();
    });

    page.update_in(cx, |this, window, cx| {
      this.open_overview_pr_alert_in_shell(window, cx);
    });

    cx.update(|_, cx| {
      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/session");
    });

    std::fs::remove_dir_all(&repo_root).ok();
  }

  #[test]
  fn overview_conflicts_info_falls_back_to_checks_requirement() {
    let info = overview_conflicts_info(None, Some(&make_checks_summary())).expect("conflicts info");

    assert_eq!(info.kind, OverviewConflictsKind::OutOfDate);
    assert_eq!(info.title, "Branch is out of date");
  }

  #[test]
  fn overview_conflicts_info_returns_conflicts_for_dirty_mergeable_state() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("dirty"),
      "This pull request has merge conflicts that must be resolved before it can be merged.",
    );

    let info = overview_conflicts_info(Some(&readiness), None).expect("conflicts info");

    assert_eq!(info.kind, OverviewConflictsKind::Conflicts);
    assert_eq!(info.title, "Merge conflicts detected");
  }

  #[test]
  fn overview_conflicts_info_returns_no_conflicts_when_pr_is_ready() {
    let mut checks = make_checks_summary();
    checks.requires_up_to_date_branch = false;
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );

    let info = overview_conflicts_info(Some(&readiness), Some(&checks)).expect("conflicts info");
    assert_eq!(info.kind, OverviewConflictsKind::NoConflicts);
  }

  #[test]
  fn overview_conflicts_info_returns_out_of_date_for_behind_mergeable_state() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("behind"),
      "This pull request branch is out of date with the base branch.",
    );

    let info = overview_conflicts_info(Some(&readiness), None).expect("conflicts info");

    assert_eq!(info.kind, OverviewConflictsKind::OutOfDate);
    assert_eq!(info.title, "Branch is out of date");
  }

  #[test]
  fn overview_conflicts_status_is_built_when_pr_has_merge_conflicts() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("dirty"),
      "This pull request has merge conflicts that must be resolved before it can be merged.",
    );

    let info = overview_conflicts_info(Some(&readiness), None).expect("conflicts info");
    assert_eq!(info.kind, OverviewConflictsKind::Conflicts);
  }

  #[test]
  fn overview_review_status_changes_requested_overrides_approval() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );
    let reviews = vec![
      GithubPullRequestReview {
        node_id: "PRR_1".to_string(),
        id: 1,
        body: None,
        state: GithubPullRequestReviewState::Approved,
        submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
        commit_id: None,
        html_url: String::new(),
        user: Some(GithubPullRequestReviewUser {
          login: "reviewer1".to_string(),
          avatar_url: None,
        }),
      },
      GithubPullRequestReview {
        node_id: "PRR_2".to_string(),
        id: 2,
        body: None,
        state: GithubPullRequestReviewState::RequestChanges,
        submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
        commit_id: None,
        html_url: String::new(),
        user: Some(GithubPullRequestReviewUser {
          login: "reviewer2".to_string(),
          avatar_url: None,
        }),
      },
    ];

    let info =
      overview_review_status_info(Some(&readiness), &[], &reviews, "author").expect("review info");
    assert_eq!(info.title, "Changes requested");
  }

  #[test]
  fn overview_review_status_returns_approved_when_all_reviewers_approved() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );
    let reviews = vec![GithubPullRequestReview {
      node_id: "PRR_1".to_string(),
      id: 1,
      body: None,
      state: GithubPullRequestReviewState::Approved,
      submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
      commit_id: None,
      html_url: String::new(),
      user: Some(GithubPullRequestReviewUser {
        login: "reviewer1".to_string(),
        avatar_url: None,
      }),
    }];

    let info =
      overview_review_status_info(Some(&readiness), &[], &reviews, "author").expect("review info");
    assert_eq!(info.title, "Changes approved");
  }

  #[test]
  fn overview_review_status_returns_approved_when_one_approved_and_one_commented() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );
    let reviews = vec![
      GithubPullRequestReview {
        node_id: "PRR_1".to_string(),
        id: 1,
        body: None,
        state: GithubPullRequestReviewState::Approved,
        submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
        commit_id: None,
        html_url: String::new(),
        user: Some(GithubPullRequestReviewUser {
          login: "reviewer1".to_string(),
          avatar_url: None,
        }),
      },
      GithubPullRequestReview {
        node_id: "PRR_2".to_string(),
        id: 2,
        body: None,
        state: GithubPullRequestReviewState::Commented,
        submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
        commit_id: None,
        html_url: String::new(),
        user: Some(GithubPullRequestReviewUser {
          login: "reviewer2".to_string(),
          avatar_url: None,
        }),
      },
    ];

    let info =
      overview_review_status_info(Some(&readiness), &[], &reviews, "author").expect("review info");
    assert_eq!(info.title, "Changes approved");
  }

  #[test]
  fn overview_review_status_returns_awaiting_when_reviewer_has_not_approved() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );
    let reviewers = vec![GithubPullRequestFilterOptionUser {
      login: "reviewer1".to_string(),
      avatar_url: None,
    }];

    let info = overview_review_status_info(Some(&readiness), &reviewers, &[], "author")
      .expect("review info");
    assert_eq!(info.title, "Review required");
  }

  #[test]
  fn overview_review_status_returns_changes_requested() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );
    let reviews = vec![GithubPullRequestReview {
      node_id: "PRR_1".to_string(),
      id: 1,
      body: None,
      state: GithubPullRequestReviewState::RequestChanges,
      submitted_at: Some("2025-01-01T00:00:00Z".to_string()),
      commit_id: None,
      html_url: String::new(),
      user: Some(GithubPullRequestReviewUser {
        login: "reviewer1".to_string(),
        avatar_url: None,
      }),
    }];

    let info =
      overview_review_status_info(Some(&readiness), &[], &reviews, "author").expect("review info");
    assert_eq!(info.title, "Changes requested");
  }

  #[test]
  fn overview_review_status_returns_none_for_dirty_state() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("dirty"),
      "Merge conflicts.",
    );

    assert!(overview_review_status_info(Some(&readiness), &[], &[], "author").is_none());
  }

  #[test]
  fn overview_review_status_returns_none_when_ready_and_no_reviewers() {
    let readiness = make_merge_readiness(
      GithubPullRequestMergeReadinessStatus::Ready,
      vec![GithubPullRequestMergeMethod::Merge],
    );

    assert!(overview_review_status_info(Some(&readiness), &[], &[], "author").is_none());
  }

  #[test]
  fn overview_review_status_returns_review_required_when_blocked() {
    let readiness = make_merge_readiness_with_state(
      GithubPullRequestMergeReadinessStatus::Blocked,
      Some("blocked"),
      "Review is required by reviewers with write access.",
    );

    let info =
      overview_review_status_info(Some(&readiness), &[], &[], "author").expect("review info");
    assert_eq!(info.title, "Review required");
  }
}
