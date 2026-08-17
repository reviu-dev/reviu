//! What a crash report and its breadcrumbs need to know about the repository the
//! user was working in when things went wrong.

use std::path::{Path, PathBuf};

use editor::DiffViewMode;
use sentry::protocol::{Map, Value};

use crate::dock_panel::DockPanelTab;
use crate::repo_command::RepoCommandOutcome;
use crate::sentry_context;

/// Where the reports go. Sentry is not observable from a test, this is.
pub(crate) trait TelemetrySink: Send + Sync {
  fn breadcrumb(&self, message: &str, data: Map<String, Value>);
  fn expected_error(&self, operation: &str, reason: &str, data: Map<String, Value>);
  fn unexpected_error(&self, operation: &'static str, error: &str, data: Map<String, Value>);
  fn sync_context(
    &self,
    repo_root: Option<&Path>,
    selected_file: Option<&Path>,
    branch: Option<&str>,
    tab: &'static str,
    diff_view: &'static str,
  );
  fn clear_context(&self);
}

struct SentrySink;

impl TelemetrySink for SentrySink {
  fn breadcrumb(&self, message: &str, data: Map<String, Value>) {
    sentry_context::add_breadcrumb("git.action", message, data);
  }

  fn expected_error(&self, operation: &str, reason: &str, data: Map<String, Value>) {
    sentry_context::record_expected_error(operation, reason, data);
  }

  fn unexpected_error(&self, operation: &'static str, error: &str, data: Map<String, Value>) {
    let io_error = std::io::Error::other(error.to_string());
    sentry_context::capture_unexpected_error(operation, &io_error, data);
  }

  fn sync_context(
    &self,
    repo_root: Option<&Path>,
    selected_file: Option<&Path>,
    branch: Option<&str>,
    tab: &'static str,
    diff_view: &'static str,
  ) {
    sentry_context::sync_git_context(repo_root, selected_file, branch, tab, diff_view);
  }

  fn clear_context(&self) {
    sentry_context::clear_git_context();
  }
}

// Tests run in parallel threads, so the override is per thread.
#[cfg(test)]
thread_local! {
  static TEST_SINK: std::cell::RefCell<Option<std::sync::Arc<dyn TelemetrySink>>> =
    const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_test_sink(sink: Option<std::sync::Arc<dyn TelemetrySink>>) {
  TEST_SINK.with(|cell| *cell.borrow_mut() = sink);
}

fn sink() -> std::sync::Arc<dyn TelemetrySink> {
  #[cfg(test)]
  if let Some(sink) = TEST_SINK.with(|cell| cell.borrow().clone()) {
    return sink;
  }
  std::sync::Arc::new(SentrySink)
}

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
    sink().sync_context(
      self.repo_root,
      self.selected_file,
      self.branch,
      self.tab,
      self.diff_view,
    );
  }

  /// Without a repository there is nothing to describe, and a stale context would
  /// point a later crash at the repository the user just left.
  pub(crate) fn sync_or_clear(&self) {
    if self.repo_root.is_none() {
      sink().clear_context();
      return;
    }
    self.sync();
  }

  pub(crate) fn breadcrumb(&self, message: &str, extra: Map<String, Value>) {
    sink().breadcrumb(message, self.with_context(extra));
  }

  /// Git refused for a reason we know about, a conflict being the usual one.
  pub(crate) fn expected_error(&self, operation: &str, reason: &str, extra: Map<String, Value>) {
    sink().expected_error(operation, reason, self.with_context(extra));
  }

  pub(crate) fn unexpected_error(
    &self,
    operation: &'static str,
    error: &str,
    extra: Map<String, Value>,
  ) {
    sink().unexpected_error(operation, error, self.with_context(extra));
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
pub(crate) mod test_support {
  use super::*;
  use std::sync::{Arc, Mutex};

  #[derive(Debug, PartialEq, Eq)]
  pub(crate) enum Report {
    Breadcrumb(String),
    Expected { operation: String, reason: String },
    Unexpected { operation: String, error: String },
    ContextSynced { tab: String, diff_view: String },
    ContextCleared,
  }

  #[derive(Default)]
  pub(crate) struct RecordingSink {
    reports: Mutex<Vec<Report>>,
    data: Mutex<Vec<Map<String, Value>>>,
  }

  impl RecordingSink {
    /// Installs itself for the duration of the test.
    pub(crate) fn install() -> Arc<Self> {
      let sink = Arc::new(Self::default());
      set_test_sink(Some(sink.clone()));
      sink
    }

    pub(crate) fn reports(&self) -> Vec<Report> {
      self.reports.lock().expect("reports").drain(..).collect()
    }

    pub(crate) fn last_data(&self) -> Option<Map<String, Value>> {
      self.data.lock().expect("data").last().cloned()
    }

    fn record(&self, report: Report, data: Map<String, Value>) {
      self.reports.lock().expect("reports").push(report);
      self.data.lock().expect("data").push(data);
    }
  }

  impl TelemetrySink for RecordingSink {
    fn breadcrumb(&self, message: &str, data: Map<String, Value>) {
      self.record(Report::Breadcrumb(message.to_string()), data);
    }

    fn expected_error(&self, operation: &str, reason: &str, data: Map<String, Value>) {
      self.record(
        Report::Expected {
          operation: operation.to_string(),
          reason: reason.to_string(),
        },
        data,
      );
    }

    fn unexpected_error(&self, operation: &'static str, error: &str, data: Map<String, Value>) {
      self.record(
        Report::Unexpected {
          operation: operation.to_string(),
          error: error.to_string(),
        },
        data,
      );
    }

    fn sync_context(
      &self,
      _repo_root: Option<&Path>,
      _selected_file: Option<&Path>,
      _branch: Option<&str>,
      tab: &'static str,
      diff_view: &'static str,
    ) {
      self.record(
        Report::ContextSynced {
          tab: tab.to_string(),
          diff_view: diff_view.to_string(),
        },
        Map::new(),
      );
    }

    fn clear_context(&self) {
      self.record(Report::ContextCleared, Map::new());
    }
  }
}

#[cfg(test)]
mod tests {
  use super::test_support::{RecordingSink, Report};
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
  fn a_conflict_is_reported_as_expected_and_an_error_is_not() {
    let sink = RecordingSink::install();
    let telemetry = GitTelemetry {
      repo_root: Some(Path::new("/tmp/widget")),
      selected_file: None,
      branch: Some("feature"),
      tab: "changes",
      diff_view: "inline",
    };

    telemetry.report_outcome(
      "git.rebase",
      outcome_report(&Ok(RepoCommandOutcome::Conflicted {
        path: PathBuf::from("src/main.rs"),
        commit_message: None,
        error: "conflict".into(),
      })),
    );
    assert_eq!(
      sink.reports(),
      vec![Report::Expected {
        operation: "git.rebase".to_string(),
        reason: "conflict".to_string(),
      }],
      "a conflict never reaches Sentry as a crash"
    );
    assert_eq!(
      sink.last_data().and_then(|data| data
        .get("file")
        .and_then(|file| file.as_str())
        .map(String::from)),
      Some("src/main.rs".to_string())
    );

    telemetry.report_outcome(
      "git.push",
      outcome_report(&Err(anyhow::anyhow!("remote hung up"))),
    );
    assert_eq!(
      sink.reports(),
      vec![
        Report::Breadcrumb("git.push failed".to_string()),
        Report::Unexpected {
          operation: "git.push".to_string(),
          error: "remote hung up".to_string(),
        },
      ],
      "an error leaves a trail and is captured"
    );

    telemetry.report_outcome(
      "git.push",
      outcome_report(&Ok(RepoCommandOutcome::Done {
        message: "Pushed".into(),
      })),
    );
    assert_eq!(sink.reports(), vec![], "success sends nothing at all");
    set_test_sink(None);
  }

  #[test]
  fn leaving_the_last_repository_clears_the_context() {
    let sink = RecordingSink::install();

    GitTelemetry {
      repo_root: Some(Path::new("/tmp/widget")),
      selected_file: None,
      branch: None,
      tab: "changes",
      diff_view: "inline",
    }
    .sync_or_clear();
    assert_eq!(
      sink.reports(),
      vec![Report::ContextSynced {
        tab: "changes".to_string(),
        diff_view: "inline".to_string(),
      }]
    );

    GitTelemetry {
      repo_root: None,
      selected_file: None,
      branch: None,
      tab: "changes",
      diff_view: "inline",
    }
    .sync_or_clear();
    assert_eq!(
      sink.reports(),
      vec![Report::ContextCleared],
      "a later crash must not be blamed on the repository we just left"
    );
    set_test_sink(None);
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
