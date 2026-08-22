//! What the CI of a pull request says, turned into rows a narrow column can
//! read: one line per workflow job, check run or status, with its state and how
//! long it took.

use crate::api::{GithubPullRequestChecksRollupState, GithubPullRequestChecksSummary};
use crate::date_format::{format_relative_time, parse_rfc3339};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CheckRow {
  pub id: String,
  pub state: GithubPullRequestChecksRollupState,
  pub title: String,
  pub status_label: Option<String>,
  pub app_label: Option<String>,
  pub app_slug: Option<String>,
  pub app_avatar_url: Option<String>,
  pub open_url: Option<String>,
}

pub(crate) fn non_empty_owned(value: &str) -> Option<String> {
  let value = value.trim();
  if value.is_empty() {
    None
  } else {
    Some(value.to_string())
  }
}

pub(crate) fn singular_or_plural(
  count: u64,
  singular: &'static str,
  plural: &'static str,
) -> &'static str {
  if count == 1 { singular } else { plural }
}

pub(crate) fn checks_summary_title(checks: &GithubPullRequestChecksSummary) -> String {
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

pub(crate) fn checks_summary_subtitle(checks: &GithubPullRequestChecksSummary) -> String {
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

pub(crate) fn format_check_duration(total_seconds: u64) -> String {
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

pub(crate) fn check_duration_label(
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

  Some(format_check_duration(elapsed_seconds as u64))
}

pub(crate) fn check_status_label(
  state: GithubPullRequestChecksRollupState,
  started_at: Option<&str>,
  finished_at: Option<&str>,
) -> Option<String> {
  match state {
    GithubPullRequestChecksRollupState::Success => Some(
      check_duration_label(started_at, finished_at, state)
        .map(|d| format!("Successful in {d}"))
        .unwrap_or_else(|| "Successful".to_string()),
    ),
    GithubPullRequestChecksRollupState::Failure => Some(
      check_duration_label(started_at, finished_at, state)
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
      check_duration_label(started_at, finished_at, state)
        .map(|d| format!("In progress - {d}"))
        .unwrap_or_else(|| "In progress".to_string()),
    ),
  }
}

pub(crate) fn check_state_sort_key(row: &CheckRow) -> u8 {
  match row.state {
    GithubPullRequestChecksRollupState::Failure => 0,
    GithubPullRequestChecksRollupState::Pending => 1,
    GithubPullRequestChecksRollupState::Skipped => 2,
    GithubPullRequestChecksRollupState::Success => 3,
  }
}

pub(crate) fn check_rows(checks: &GithubPullRequestChecksSummary) -> Vec<CheckRow> {
  let mut rows = Vec::new();

  for (ix, context) in checks.missing_required_contexts.iter().enumerate() {
    rows.push(CheckRow {
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
      rows.push(CheckRow {
        id: format!("workflow-run-{}", run.id),
        state: run.state,
        title,
        status_label: check_status_label(run.state, run_started_at, run_finished_at),
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

      rows.push(CheckRow {
        id: format!("workflow-job-{}", job.id),
        state: job.state,
        title,
        status_label: check_status_label(job.state, job_started_at, job_finished_at),
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

    rows.push(CheckRow {
      id: format!("check-run-{}", check.id),
      state: check.state,
      title,
      status_label: check_status_label(check.state, check.started_at.as_deref(), finished_at),
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
    rows.push(CheckRow {
      id: format!("legacy-status-{}", status.id),
      state: status.state,
      title,
      status_label: check_status_label(status.state, Some(status.created_at.as_str()), finished_at),
      app_label: None,
      app_slug: None,
      app_avatar_url: status.avatar_url.as_deref().and_then(non_empty_owned),
      open_url: status.target_url.clone(),
    });
  }

  rows.sort_by_key(check_state_sort_key);
  rows
}

/// A summary with something of every state in it: what a panel has to survive.
#[cfg(test)]
pub(crate) fn checks_summary_fixture() -> GithubPullRequestChecksSummary {
  GithubPullRequestChecksSummary {
    head_sha: "head123".to_string(),
    overall_state: GithubPullRequestChecksRollupState::Failure,
    required_state: GithubPullRequestChecksRollupState::Pending,
    total_checks: 4,
    successful_checks: 2,
    failed_checks: 1,
    pending_checks: 1,
    skipped_checks: 0,
    required_checks_total: 3,
    required_checks_passed: 1,
    required_checks_failed: 1,
    required_checks_pending: 1,
    required_checks_skipped: 0,
    required_contexts: vec![
      "build".to_string(),
      "lint".to_string(),
      "deploy".to_string(),
    ],
    missing_required_contexts: vec!["deploy".to_string()],
    requires_up_to_date_branch: true,
    actions_runs: Vec::new(),
    other_checks: Vec::new(),
    legacy_statuses: Vec::new(),
  }
}

#[cfg(test)]
mod tests {
  use super::checks_summary_fixture as make_checks_summary;
  use super::*;
  use crate::api::{
    GithubPullRequestCheckRun, GithubPullRequestLegacyStatus, GithubPullRequestWorkflowJob,
    GithubPullRequestWorkflowRun,
  };

  #[test]
  fn check_rows_keep_provider_avatar_urls() {
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

    let rows = check_rows(&checks);

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
  fn check_rows_prefix_workflow_name_with_event_suffix() {
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

    let rows = check_rows(&checks);
    let row = rows
      .iter()
      .find(|row| row.id == "workflow-job-200")
      .expect("workflow job row");

    assert_eq!(row.title, "CI / Frontend (build) (pull_request)");
    assert_eq!(row.status_label.as_deref(), Some("Successful in 2m"));
  }

  #[test]
  fn check_status_label_formats_skipped_and_success_states() {
    assert_eq!(
      check_status_label(
        GithubPullRequestChecksRollupState::Success,
        Some("2026-04-25T10:00:00Z"),
        Some("2026-04-25T10:00:07Z"),
      )
      .as_deref(),
      Some("Successful in 7s"),
    );
    assert!(
      check_status_label(
        GithubPullRequestChecksRollupState::Skipped,
        Some("2026-04-24T10:00:00Z"),
        Some("2026-04-24T10:00:00Z"),
      )
      .unwrap()
      .starts_with("Skipped "),
    );
  }

  #[test]
  fn checks_summary_subtitle_lists_skipped_alongside_success() {
    let mut checks = make_checks_summary();
    checks.total_checks = 31;
    checks.successful_checks = 15;
    checks.skipped_checks = 16;
    checks.failed_checks = 0;
    checks.pending_checks = 0;
    checks.overall_state = GithubPullRequestChecksRollupState::Success;

    assert_eq!(
      checks_summary_subtitle(&checks),
      "16 skipped, 15 successful checks"
    );
  }
}
