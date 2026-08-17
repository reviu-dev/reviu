//! What a crash report and its breadcrumbs need to know about the repository the
//! user was working in when things went wrong.

use std::path::{Path, PathBuf};

use editor::DiffViewMode;
use sentry::protocol::{Map, Value};

use crate::dock_panel::DockPanelTab;
use crate::repo_command::RepoCommandOutcome;
use crate::sentry_context;

pub(crate) fn dock_tab_tag(tab: DockPanelTab) -> &'static str {
  match tab {
    DockPanelTab::Changes => "changes",
    DockPanelTab::Files => "files",
    DockPanelTab::History => "history",
    DockPanelTab::PullRequest => "pull_request",
    DockPanelTab::Terminal => "terminal",
  }
}

pub(crate) fn diff_view_tag(diff_view: DiffViewMode, previewing: bool) -> &'static str {
  if previewing {
    return "markdown_preview";
  }
  match diff_view {
    DiffViewMode::Inline => "inline",
    DiffViewMode::Split => "split",
  }
}

/// What an outcome is worth reporting: a conflict is business as usual, an error
/// is not, and success needs no report at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OutcomeReport {
  Nothing,
  Expected {
    reason: &'static str,
    file: Option<PathBuf>,
  },
  Unexpected {
    error: String,
  },
}

pub(crate) fn outcome_report(outcome: &anyhow::Result<RepoCommandOutcome>) -> OutcomeReport {
  match outcome {
    Ok(RepoCommandOutcome::Done { .. }) | Ok(RepoCommandOutcome::UpToDate { .. }) => {
      OutcomeReport::Nothing
    }
    Ok(RepoCommandOutcome::Conflicted { path, .. }) => OutcomeReport::Expected {
      reason: "conflict",
      file: Some(path.clone()),
    },
    Err(error) => OutcomeReport::Unexpected {
      error: error.to_string(),
    },
  }
}

/// The repository paths are never sent as-is: only a name and a stable hash.
pub(crate) struct GitTelemetry<'a> {
  pub(crate) repo_root: Option<&'a Path>,
  pub(crate) selected_file: Option<&'a Path>,
  pub(crate) branch: Option<&'a str>,
  pub(crate) tab: &'static str,
  pub(crate) diff_view: &'static str,
}

impl GitTelemetry<'_> {
  pub(crate) fn data(&self) -> Map<String, Value> {
    let mut data = Map::new();
    if let Some(repo_root) = self.repo_root {
      let (repo_name, repo_hash) = sentry_context::sanitize_repo_path(repo_root);
      data.insert("repo_name".into(), repo_name.into());
      data.insert("repo_hash".into(), repo_hash.into());
    }
    if let Some(selected_file) = self.selected_file {
      let file = selected_file.to_string_lossy().replace(['\n', '\r'], "");
      data.insert("selected_file".into(), file.into());
    }
    if let Some(branch) = self.branch {
      data.insert("branch".into(), branch.to_string().into());
    }
    data.insert("sidebar_mode".into(), self.tab.to_string().into());
    data.insert("diff_view".into(), self.diff_view.to_string().into());
    data
  }

  /// The context that stays attached to whatever happens next.
  pub(crate) fn sync(&self) {
    sentry_context::sync_git_context(
      self.repo_root,
      self.selected_file,
      self.branch,
      self.tab,
      self.diff_view,
    );
  }

  pub(crate) fn breadcrumb(&self, message: &str, extra: Map<String, Value>) {
    sentry_context::add_breadcrumb("git.action", message, self.with_context(extra));
  }

  /// Git refused for a reason we know about, a conflict being the usual one.
  pub(crate) fn expected_error(&self, operation: &str, reason: &str, extra: Map<String, Value>) {
    sentry_context::record_expected_error(operation, reason, self.with_context(extra));
  }

  pub(crate) fn unexpected_error(
    &self,
    operation: &'static str,
    error: &str,
    extra: Map<String, Value>,
  ) {
    let io_error = std::io::Error::other(error.to_string());
    sentry_context::capture_unexpected_error(operation, &io_error, self.with_context(extra));
  }

  /// Sends whatever the outcome deserves, under the command's own key.
  pub(crate) fn report_outcome(&self, operation: &'static str, report: OutcomeReport) {
    match report {
      OutcomeReport::Nothing => {}
      OutcomeReport::Expected { reason, file } => {
        let mut data = Map::new();
        if let Some(file) = file {
          data.insert(
            "file".into(),
            file.to_string_lossy().replace(['\n', '\r'], "").into(),
          );
        }
        self.expected_error(operation, reason, data);
      }
      OutcomeReport::Unexpected { error } => {
        let mut data = Map::new();
        data.insert("error".into(), error.clone().into());
        self.breadcrumb(&format!("{operation} failed"), data.clone());
        self.unexpected_error(operation, error.as_str(), data);
      }
    }
  }

  fn with_context(&self, mut extra: Map<String, Value>) -> Map<String, Value> {
    for (key, value) in self.data() {
      extra.entry(key).or_insert(value);
    }
    extra
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn value(data: &Map<String, Value>, key: &str) -> Option<String> {
    data
      .get(key)
      .and_then(|value| value.as_str())
      .map(Into::into)
  }

  #[test]
  fn the_repository_path_never_leaves_the_machine() {
    let telemetry = GitTelemetry {
      repo_root: Some(Path::new("/home/someone/secret-project")),
      selected_file: None,
      branch: None,
      tab: "changes",
      diff_view: "inline",
    };

    let data = telemetry.data();
    assert_eq!(value(&data, "repo_name").as_deref(), Some("secret-project"));
    assert_eq!(
      value(&data, "repo_hash").map(|hash| hash.len()),
      Some(12),
      "the full path is replaced by a stable short hash"
    );
    assert!(
      !format!("{data:?}").contains("/home/someone"),
      "no absolute path anywhere in the payload"
    );
  }

  #[test]
  fn the_context_says_which_surface_and_file_were_open() {
    let telemetry = GitTelemetry {
      repo_root: None,
      selected_file: Some(Path::new("src/ma\nin.rs")),
      branch: Some("feature"),
      tab: "history",
      diff_view: "split",
    };

    let data = telemetry.data();
    assert_eq!(
      value(&data, "selected_file").as_deref(),
      Some("src/main.rs"),
      "newlines would break the breadcrumb"
    );
    assert_eq!(value(&data, "branch").as_deref(), Some("feature"));
    assert_eq!(value(&data, "sidebar_mode").as_deref(), Some("history"));
    assert_eq!(value(&data, "diff_view").as_deref(), Some("split"));
  }

  #[test]
  fn the_caller_keeps_the_last_word_on_a_key() {
    let telemetry = GitTelemetry {
      repo_root: None,
      selected_file: None,
      branch: Some("main"),
      tab: "changes",
      diff_view: "inline",
    };

    let mut extra = Map::new();
    extra.insert("branch".into(), "the-one-being-deleted".into());
    let data = telemetry.with_context(extra);

    assert_eq!(
      value(&data, "branch").as_deref(),
      Some("the-one-being-deleted"),
      "a command that names a branch means that branch, not the current one"
    );
  }

  #[test]
  fn only_what_is_worth_reporting_is_reported() {
    assert_eq!(
      outcome_report(&Ok(RepoCommandOutcome::Done {
        message: "Pushed".into()
      })),
      OutcomeReport::Nothing,
      "a command that worked is not news"
    );
    assert_eq!(
      outcome_report(&Ok(RepoCommandOutcome::UpToDate {
        message: "Already up to date".into()
      })),
      OutcomeReport::Nothing
    );
    assert_eq!(
      outcome_report(&Ok(RepoCommandOutcome::Conflicted {
        path: PathBuf::from("src/main.rs"),
        commit_message: None,
        error: "conflict".into(),
      })),
      OutcomeReport::Expected {
        reason: "conflict",
        file: Some(PathBuf::from("src/main.rs")),
      },
      "a conflict is expected: it must not be captured as a crash"
    );
    assert_eq!(
      outcome_report(&Err(anyhow::anyhow!("remote hung up"))),
      OutcomeReport::Unexpected {
        error: "remote hung up".to_string(),
      }
    );
  }

  #[test]
  fn every_tab_and_view_has_a_tag() {
    assert_eq!(dock_tab_tag(DockPanelTab::Changes), "changes");
    assert_eq!(dock_tab_tag(DockPanelTab::Files), "files");
    assert_eq!(dock_tab_tag(DockPanelTab::History), "history");
    assert_eq!(dock_tab_tag(DockPanelTab::PullRequest), "pull_request");
    assert_eq!(dock_tab_tag(DockPanelTab::Terminal), "terminal");

    assert_eq!(diff_view_tag(DiffViewMode::Inline, false), "inline");
    assert_eq!(diff_view_tag(DiffViewMode::Split, false), "split");
    assert_eq!(
      diff_view_tag(DiffViewMode::Split, true),
      "markdown_preview",
      "a preview is what the user reads, whatever the diff mode says"
    );
  }
}
