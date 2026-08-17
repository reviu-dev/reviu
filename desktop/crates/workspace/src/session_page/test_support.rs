//! Shared fixtures for the shell tests.

use super::*;
use crate::workspace::WorkspaceApi;
use editor::{ReviewCommentMode, ReviewCommentSide};
use gpui::TestAppContext;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn isolate_config_store_for_test() {
  static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);
  let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
  let db_path = std::env::temp_dir().join(format!(
    "reviu-session-page-test-config-{}-{id}.sqlite",
    std::process::id()
  ));
  let _ = std::fs::remove_file(&db_path);
  ConfigStore::set_test_db_path(Some(db_path));
}

/// Mounts the page for real. The agent is only connected by `activate`, which
/// the workspace calls when it routes here, so rendering spawns no process.
pub(super) fn add_session_page_window(
  repo_root: PathBuf,
  cx: &mut TestAppContext,
) -> (Entity<SessionPage>, &mut gpui::VisualTestContext) {
  // The recent-repository store is process-global, so parallel tests would race
  // over it; the repo is set on the page explicitly below instead.
  isolate_config_store_for_test();
  cx.update(|cx| {
    gpui_component::init(cx);
    if !cx.has_global::<crate::config::AppSettings>() {
      cx.set_global(crate::config::AppSettings::default());
    }
    if !cx.has_global::<AuthStateStore>() {
      cx.set_global(AuthStateStore::default());
    }
    if !cx.has_global::<WorkspaceApi>() {
      cx.set_global(WorkspaceApi::new());
    }
  });

  let mut mounted: Option<Entity<SessionPage>> = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let page = cx.new(|cx| SessionPage::new(window, cx));
    mounted = Some(page.clone());
    gpui_component::Root::new(page, window, cx)
  });
  let page = mounted.expect("session page");
  page.update(cx, |page, cx| {
    page.selected_repo = Some(repo_root.clone());
    page.dock_panel.update(cx, |panel, cx| {
      panel.set_repo_root(Some(repo_root.clone()), cx)
    });
  });
  (page, cx)
}

/// The shell as a fresh install sees it: no repository anywhere.
pub(super) fn add_session_page_window_without_repo(
  cx: &mut TestAppContext,
) -> (Entity<SessionPage>, &mut gpui::VisualTestContext) {
  isolate_config_store_for_test();
  cx.update(|cx| {
    gpui_component::init(cx);
    if !cx.has_global::<crate::config::AppSettings>() {
      cx.set_global(crate::config::AppSettings::default());
    }
    if !cx.has_global::<AuthStateStore>() {
      cx.set_global(AuthStateStore::default());
    }
    if !cx.has_global::<WorkspaceApi>() {
      cx.set_global(WorkspaceApi::new());
    }
  });

  let mut mounted: Option<Entity<SessionPage>> = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let page = cx.new(|cx| SessionPage::new(window, cx));
    mounted = Some(page.clone());
    gpui_component::Root::new(page, window, cx)
  });
  (mounted.expect("session page"), cx)
}

pub(super) async fn await_open_file(page: &Entity<SessionPage>, cx: &mut gpui::VisualTestContext) {
  let task = page.update(cx, |page, _| page.open_file_task.take());
  if let Some(task) = task {
    task.await;
  }
  cx.run_until_parked();
}

/// The diff lands after the file load: bases, then diff, then projection.
pub(super) async fn await_editor_diff(
  page: &Entity<SessionPage>,
  cx: &mut gpui::VisualTestContext,
) {
  loop {
    let Some(editor) = page.read_with(cx, |page, _| page.editor.clone()) else {
      return;
    };
    let (bases_task, diff_task, git_task) = editor.update(cx, |editor, _| {
      (
        editor.bases_task.take(),
        editor.diff_task.take(),
        editor.git_task.take(),
      )
    });
    let mut had_task = false;
    if let Some(task) = bases_task {
      had_task = true;
      task.await;
    }
    if let Some(task) = diff_task {
      had_task = true;
      task.await;
    }
    if let Some(task) = git_task {
      had_task = true;
      task.await;
    }
    cx.run_until_parked();
    if !had_task {
      return;
    }
  }
}

pub(super) fn create_request(line: usize, body: &str) -> ReviewCommentCreateRequest {
  ReviewCommentCreateRequest {
    line,
    side: ReviewCommentSide::Right,
    start_line: None,
    start_side: None,
    in_reply_to_id: None,
    body: Arc::from(body),
    mode: ReviewCommentMode::SingleComment,
  }
}

pub(super) fn init_bare_repo(prefix: &str) -> PathBuf {
  let mut path = std::env::temp_dir();
  let nanos = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .expect("system clock before unix epoch")
    .as_nanos();
  path.push(format!(
    "reviu-{prefix}-bare-{}-{nanos}",
    std::process::id()
  ));
  std::fs::create_dir_all(&path).expect("create temp dir");
  git2::Repository::init_bare(&path).expect("init bare repository");
  path
}

/// Publishes the current branch to a fresh bare remote and tracks it, so the
/// ahead/behind counters have something to count.
pub(super) fn publish_to_new_remote(repo_root: &Path, prefix: &str) -> PathBuf {
  let remote_root = init_bare_repo(prefix);
  let repo = git2::Repository::open(repo_root).expect("open repo");
  repo
    .remote("origin", &remote_root.to_string_lossy())
    .expect("add remote");

  let head = repo.head().expect("head");
  let branch = head.shorthand().expect("branch name").to_string();
  let mut remote = repo.find_remote("origin").expect("find remote");
  let mut callbacks = git2::RemoteCallbacks::new();
  callbacks.credentials(|_, _, _| git2::Cred::default());
  let mut options = git2::PushOptions::new();
  options.remote_callbacks(callbacks);
  remote
    .push(
      &[format!("refs/heads/{branch}:refs/heads/{branch}")],
      Some(&mut options),
    )
    .expect("push branch");

  repo
    .find_branch(&branch, git2::BranchType::Local)
    .expect("find local branch")
    .set_upstream(Some(&format!("origin/{branch}")))
    .expect("set upstream");

  remote_root
}

pub(super) async fn await_branch_refresh(
  page: &Entity<SessionPage>,
  cx: &mut gpui::VisualTestContext,
) {
  let task = page.update(cx, |page, _| page._branch_task.take());
  if let Some(task) = task {
    task.await;
  }
  cx.run_until_parked();
}
