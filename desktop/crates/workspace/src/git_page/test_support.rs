//! Shared fixtures for the git page tests.

use super::*;
use git::RepoStage;

pub(super) use crate::history_list::test_support::{make_commit, make_history_file};
pub(super) use crate::test_support::{
  TempBareRepo, TempRepo, commit_text_file, force_checkout_head, head_oid, push_branch_to_remote,
  remote_branch_oid, set_remote_head, set_upstream,
};
pub(super) use git::{restore_file, stage_file};

use gpui::TestAppContext;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api::{ApiClient, User, UserRole, UserSubscription};

pub(super) struct TempDir {
  pub(super) path: PathBuf,
}

impl TempDir {
  pub(super) fn new(prefix: &str) -> Self {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system clock before unix epoch")
      .as_nanos();
    path.push(format!("reviu-{prefix}-dir-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&path).expect("create temp dir");
    Self { path }
  }
}

impl Drop for TempDir {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.path);
  }
}

pub(super) fn make_branch_status(
  name: &str,
  ahead: usize,
  behind: usize,
  has_upstream: bool,
) -> BranchStatus {
  BranchStatus {
    name: name.to_string(),
    ahead,
    behind,
    has_upstream,
  }
}

pub(super) fn make_branch_pull_request(number: u64) -> GithubPullRequest {
  GithubPullRequest {
    number,
    title: format!("Pull request {number}"),
    state: crate::api::GithubPullRequestState::Open,
    merged_at: None,
    draft: false,
    comments_count: 0,
    repository: crate::api::GithubRepository {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
    },
  }
}

pub(super) fn make_test_api_client(base_url: impl Into<String>) -> ApiClient {
  ApiClient::new_with_base_url(base_url)
}

pub(super) fn start_matching_response_server(
  responses: Vec<(String, String, String)>,
) -> (String, std::thread::JoinHandle<()>) {
  let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
  let address = format!("http://{}", listener.local_addr().expect("local addr"));

  let handle = std::thread::spawn(move || {
    for _ in 0..responses.len() {
      let (mut stream, _) = listener.accept().expect("accept connection");
      let mut request_buffer = [0u8; 4096];
      let bytes_read = stream.read(&mut request_buffer).expect("read request");
      let request = String::from_utf8_lossy(&request_buffer[..bytes_read]);

      let (_, status, body) = responses
        .iter()
        .find(|(pattern, _, _)| request.contains(pattern))
        .unwrap_or_else(|| panic!("unexpected request: {request}"));

      let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
        body.as_bytes().len(),
        body,
      );
      stream
        .write_all(response.as_bytes())
        .expect("write response");
      stream.flush().expect("flush response");
    }
  });

  (address, handle)
}

pub(super) fn make_authenticated_test_user(role: UserRole) -> User {
  User {
    id: "user_123".to_string(),
    name: "Joris".to_string(),
    email: "joris@example.com".to_string(),
    email_verified: true,
    image: None,
    github_login: Some("joris-gallot".to_string()),
    role,
    subscription: UserSubscription::default(),
  }
}

pub(super) fn isolate_config_store_for_test() {
  static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);
  let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
  let db_path = std::env::temp_dir().join(format!(
    "reviu-git-page-test-config-{}-{id}.sqlite",
    std::process::id()
  ));
  let _ = std::fs::remove_file(&db_path);
  ConfigStore::set_test_db_path(Some(db_path));
}

pub(super) fn init_gpui_test(cx: &mut TestAppContext) {
  isolate_config_store_for_test();
  cx.update(|cx| {
    gpui_component::init(cx);
    if !cx.has_global::<WorkspaceApi>() {
      cx.set_global(WorkspaceApi::new());
    }
    if !cx.has_global::<AuthStateStore>() {
      cx.set_global(AuthStateStore::default());
    }
    if !cx.has_global::<ActiveLocalRepoStore>() {
      cx.set_global(ActiveLocalRepoStore::default());
    }
    if !cx.has_global::<crate::config::AppSettings>() {
      cx.set_global(crate::config::AppSettings::default());
    }
    ActiveLocalRepoStore::set(cx, None);
  });
}

pub(super) fn add_git_page_window_with_root(
  cx: &mut TestAppContext,
) -> (Entity<GitPage>, &mut gpui::VisualTestContext) {
  let mut mounted_git_page: Option<Entity<GitPage>> = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
    mounted_git_page = Some(git_page.clone());
    gpui_component::Root::new(git_page, window, cx)
  });
  let git_page = mounted_git_page.expect("git page");
  (git_page, cx)
}

pub(super) fn tiny_png_bytes() -> Vec<u8> {
  vec![
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0,
    0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3, 2, 0,
    239, 154, 63, 71, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
  ]
}

pub(super) fn make_status_entry(path: &str, stage: RepoStage) -> RepoStatusEntry {
  RepoStatusEntry {
    path: PathBuf::from(path),
    old_path: None,
    status: RepoStatusKind::Modified,
    stage,
  }
}

pub(super) fn selected_branch_from_dropdown(this: &GitPage) -> Option<BranchRef> {
  this
    .branch_dropdown_items
    .iter()
    .find(|item| item.is_current)
    .map(|item| item.branch.clone())
}

pub(super) async fn await_git_page_background_tasks(
  git_page: Entity<GitPage>,
  cx: &mut gpui::VisualTestContext,
) {
  loop {
    let (
      branch_pr_lookup_task,
      open_file_task,
      status_task,
      branch_task,
      history_task,
      history_files_task,
      history_open_file_task,
    ) = git_page.update_in(cx, |this, _window, _| {
      (
        this.branch_pr_lookup_task.take(),
        this.open_file_task.take(),
        this.status_task.take(),
        this.branch_task.take(),
        this.history_task.take(),
        this.history_files_task.take(),
        this.history_open_file_task.take(),
      )
    });
    let editor = git_page.read_with(cx, |this, _| this.editor.clone());
    let (editor_bases_task, editor_diff_task) = if let Some(editor) = editor {
      editor.update(cx, |editor, _| {
        (editor.bases_task.take(), editor.diff_task.take())
      })
    } else {
      (None, None)
    };

    let mut had_task = false;
    if let Some(task) = branch_pr_lookup_task {
      had_task = true;
      task.await;
    }
    if let Some(task) = open_file_task {
      had_task = true;
      task.await;
    }
    if let Some(task) = status_task {
      had_task = true;
      task.await;
    }
    if let Some(task) = branch_task {
      had_task = true;
      task.await;
    }
    if let Some(task) = history_task {
      had_task = true;
      task.await;
    }
    if let Some(task) = history_files_task {
      had_task = true;
      task.await;
    }
    if let Some(task) = history_open_file_task {
      had_task = true;
      task.await;
    }
    if let Some(task) = editor_bases_task {
      had_task = true;
      task.await;
    }
    if let Some(task) = editor_diff_task {
      had_task = true;
      task.await;
    }

    if !had_task {
      break;
    }
  }
}

pub(super) fn seed_repo_branch_state(
  this: &mut GitPage,
  repo_root: &Path,
  cx: &mut Context<GitPage>,
) {
  this.selected_repo = Some(repo_root.to_path_buf());
  let branch_status = current_branch_status(repo_root).expect("read initial branch status");
  let selected = GitPage::selected_branch_from_status(Some(&branch_status));
  let detached_label = if crate::repo_state::is_detached_head(Some(&branch_status)) {
    detached_head_label(repo_root).ok()
  } else {
    None
  };
  let items = GitPage::branch_select_items(
    list_branches(repo_root).expect("list branches"),
    selected.as_ref(),
    detached_label.as_deref(),
  );
  this.branch_status = Some(branch_status);
  this.branch_dropdown_items = items;
  cx.notify();
}
