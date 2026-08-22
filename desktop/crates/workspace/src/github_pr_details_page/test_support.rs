//! Fixtures shared by the tests of the page and of its modules.

use super::*;
pub(super) use crate::api::{
  GithubPullRequestChecksRollupState, GithubPullRequestChecksSummary, GithubPullRequestCommit,
  GithubPullRequestDetails, GithubPullRequestFile, GithubPullRequestMergeMethod,
  GithubPullRequestMergeReadiness, GithubPullRequestMergeReadinessStatus,
  GithubPullRequestReviewComment, GithubPullRequestReviewCommentUser, GithubPullRequestState,
  GithubRepository,
};
pub(super) use crate::workspace::WorkspaceApi;
pub(super) use git2::{BranchType, Repository, Signature};
pub(super) use gpui::TestAppContext;
pub(super) use std::{
  io::{Read, Write},
  net::TcpListener,
  sync::atomic::{AtomicU64, Ordering},
  thread,
};

pub(super) fn init_gpui_test(cx: &mut TestAppContext) {
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
    if !cx.has_global::<AppSettings>() {
      cx.set_global(AppSettings::default());
    }
    ActiveLocalRepoStore::set(cx, None);
  });
}

pub(super) fn unique_test_db_path(label: &str) -> PathBuf {
  static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);
  let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
  std::env::temp_dir().join(format!(
    "reviu-pr-details-{label}-{}-{id}.sqlite",
    std::process::id()
  ))
}

pub(super) fn make_test_api_client(base_url: impl Into<String>) -> ApiClient {
  ApiClient::new_with_base_url(base_url)
}

pub(super) fn start_response_server(
  responses: Vec<(String, String)>,
) -> (String, thread::JoinHandle<()>) {
  let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
  let address = format!("http://{}", listener.local_addr().expect("local addr"));

  let handle = thread::spawn(move || {
    for (status, body) in responses {
      let (mut stream, _) = listener.accept().expect("accept connection");
      let mut request_buffer = [0u8; 4096];
      let _ = stream.read(&mut request_buffer).expect("read request");

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

pub(super) fn start_single_response_server(
  status: &str,
  body: &str,
) -> (String, thread::JoinHandle<()>) {
  start_response_server(vec![(status.to_string(), body.to_string())])
}

pub(super) fn make_active_local_repo(
  head_sha: &str,
  has_uncommitted_changes: bool,
) -> ActiveLocalRepo {
  make_active_local_repo_for_branch("feature", head_sha, has_uncommitted_changes)
}

pub(super) fn make_active_local_repo_for_branch(
  current_branch: &str,
  head_sha: &str,
  has_uncommitted_changes: bool,
) -> ActiveLocalRepo {
  ActiveLocalRepo {
    repo_root: PathBuf::from("/tmp/reviu-tests/acme-widget"),
    github_owner: Some("acme".to_string()),
    github_repo: Some("widget".to_string()),
    current_branch: Some(current_branch.to_string()),
    head_sha: Some(head_sha.to_string()),
    has_uncommitted_changes,
  }
}

pub(super) fn commit_local_project_file(
  repo_root: &Path,
  rel_path: &Path,
  contents: &str,
  message: &str,
) {
  let repo = Repository::open(repo_root).expect("open repo");
  std::fs::write(repo_root.join(rel_path), contents).expect("write project file");

  let mut index = repo.index().expect("open git index");
  index.add_path(rel_path).expect("stage project file");
  index.write().expect("write git index");
  let tree_id = index.write_tree().expect("write git tree");
  let tree = repo.find_tree(tree_id).expect("find git tree");
  let signature = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
  let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());

  match parent {
    Some(parent) => {
      repo
        .commit(
          Some("HEAD"),
          &signature,
          &signature,
          message,
          &tree,
          &[&parent],
        )
        .expect("commit with parent");
    }
    None => {
      repo
        .commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
        .expect("initial commit");
    }
  }
}

pub(super) fn create_local_repo_with_github_remote(
  owner: &str,
  repo_name: &str,
  current_branch: &str,
  additional_branches: &[&str],
) -> (PathBuf, ActiveLocalRepo) {
  let repo_root = crate::test_support::temp_path("pr-details-local-repo");

  std::fs::create_dir_all(repo_root.join("src")).expect("create repo directories");
  Repository::init(&repo_root).expect("init local repo");
  commit_local_project_file(
    &repo_root,
    Path::new("src/main.rs"),
    "fn main() {}\n",
    "initial",
  );

  let repo = Repository::open(&repo_root).expect("open local repo");
  repo
    .remote(
      "origin",
      format!("https://github.com/{owner}/{repo_name}.git").as_str(),
    )
    .expect("create origin remote");
  {
    let head_commit = repo
      .head()
      .expect("repo head")
      .peel_to_commit()
      .expect("head commit");

    for branch_name in additional_branches
      .iter()
      .copied()
      .chain(std::iter::once(current_branch))
    {
      if repo.find_branch(branch_name, BranchType::Local).is_err() {
        repo
          .branch(branch_name, &head_commit, false)
          .expect("create local branch");
      }
    }
  }
  drop(repo);

  switch_to_branch_name(&repo_root, current_branch).expect("switch to current branch");
  let snapshot =
    local_repo_snapshot(&repo_root, None).expect("snapshot local repo with GitHub remote");
  (repo_root, snapshot)
}

pub(super) fn make_pr_details_for_local_repo(
  head_sha: &str,
  head_ref_name: &str,
) -> GithubPullRequestDetails {
  let mut pull_request = make_pr_details_for_stats();
  pull_request.head_sha = head_sha.to_string();
  pull_request.head_ref_name = head_ref_name.to_string();
  pull_request
}

pub(super) fn make_api_file(
  filename: &str,
  status: &str,
  previous_filename: Option<&str>,
) -> GithubPullRequestFile {
  GithubPullRequestFile {
    filename: filename.to_string(),
    status: status.to_string(),
    patch: None,
    previous_filename: previous_filename.map(str::to_string),
  }
}

pub(super) fn make_api_commit(
  sha: &str,
  message: &str,
  committed_at: Option<&str>,
  parent_sha: Option<&str>,
) -> GithubPullRequestCommit {
  GithubPullRequestCommit {
    sha: sha.to_string(),
    message: message.to_string(),
    authored_at: committed_at.map(str::to_string),
    committed_at: committed_at.map(str::to_string),
    parent_sha: parent_sha.map(str::to_string),
  }
}

pub(super) fn make_merge_readiness(
  status: GithubPullRequestMergeReadinessStatus,
  methods: Vec<GithubPullRequestMergeMethod>,
) -> GithubPullRequestMergeReadiness {
  GithubPullRequestMergeReadiness {
    status,
    message: match status {
      GithubPullRequestMergeReadinessStatus::Ready => {
        "This pull request is ready to merge.".to_string()
      }
      GithubPullRequestMergeReadinessStatus::Blocked => {
        "This pull request is blocked by required checks.".to_string()
      }
      GithubPullRequestMergeReadinessStatus::Checking => {
        "GitHub is still computing whether this pull request can be merged.".to_string()
      }
      GithubPullRequestMergeReadinessStatus::Forbidden => {
        "You do not have permission to merge this pull request.".to_string()
      }
      GithubPullRequestMergeReadinessStatus::Draft => {
        "This pull request is still marked as a draft.".to_string()
      }
      GithubPullRequestMergeReadinessStatus::Closed => "This pull request is closed.".to_string(),
      GithubPullRequestMergeReadinessStatus::Merged => {
        "This pull request has already been merged.".to_string()
      }
    },
    current_head_sha: "head123".to_string(),
    default_method: methods.first().copied(),
    can_merge_now: status == GithubPullRequestMergeReadinessStatus::Ready && !methods.is_empty(),
    viewer_can_merge: true,
    mergeable_state: Some("clean".to_string()),
    rebaseable: Some(true),
    available_methods: methods,
  }
}

pub(super) fn make_merge_readiness_with_state(
  status: GithubPullRequestMergeReadinessStatus,
  mergeable_state: Option<&str>,
  message: &str,
) -> GithubPullRequestMergeReadiness {
  let mut readiness = make_merge_readiness(status, vec![GithubPullRequestMergeMethod::Merge]);
  readiness.mergeable_state = mergeable_state.map(ToString::to_string);
  readiness.message = message.to_string();
  readiness
}

pub(super) fn make_checks_summary() -> GithubPullRequestChecksSummary {
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

pub(super) fn make_review_comment(
  id: u64,
  created_at: &str,
  in_reply_to_id: Option<u64>,
) -> GithubPullRequestReviewComment {
  GithubPullRequestReviewComment {
    node_id: format!("PRRC_{id}"),
    is_outdated: false,
    thread_id: String::new(),
    is_resolved: false,
    is_collapsed: false,
    viewer_can_resolve: false,
    viewer_can_unresolve: false,
    id,
    pull_request_review_id: Some(12),
    diff_hunk: "@@ -1 +1 @@".to_string(),
    path: "src/main.rs".to_string(),
    position: Some(1),
    original_position: Some(1),
    commit_id: "head123".to_string(),
    original_commit_id: "base123".to_string(),
    in_reply_to_id,
    user: GithubPullRequestReviewCommentUser {
      login: "octocat".to_string(),
      avatar_url: None,
    },
    body: "Looks good".to_string(),
    created_at: created_at.to_string(),
    updated_at: created_at.to_string(),
    start_line: None,
    original_start_line: None,
    start_side: None,
    line: Some(1),
    original_line: Some(1),
    side: Some("RIGHT".to_string()),
    is_pending: false,
    pull_request_review_node_id: None,
  }
}

pub(super) fn make_pr_details_for_stats() -> GithubPullRequestDetails {
  GithubPullRequestDetails {
    node_id: "PR_kwDOExample".to_string(),
    number: 42,
    title: "Example PR".to_string(),
    state: GithubPullRequestState::Open,
    draft: false,
    created_at: "2026-02-28T10:00:00Z".to_string(),
    updated_at: "2026-02-28T10:00:00Z".to_string(),
    merged_at: None,
    merge_base_sha: "base".to_string(),
    base_sha: "base".to_string(),
    head_sha: "head".to_string(),
    base_ref_name: "main".to_string(),
    head_ref_name: "feature".to_string(),
    body: Some("Body".to_string()),
    author: crate::api::GithubPullRequestAuthor {
      login: "author".to_string(),
      avatar_url: None,
    },
    assignees: Vec::new(),
    requested_reviewers: Vec::new(),
    comments: 10,
    review_comments: 11,
    commits: 3,
    additions: 20,
    deletions: 4,
    changed_files: 2,
    labels: Vec::new(),
    repository: GithubRepository {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
    },
    head_repository: Some(GithubRepository {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
    }),
  }
}

pub(super) fn make_repo_branch(name: &str) -> GithubRepositoryBranch {
  GithubRepositoryBranch {
    name: name.to_string(),
    commit: crate::api::GithubRepositoryBranchCommit {
      sha: format!("{name}-sha"),
      url: format!("https://api.github.com/repos/acme/widget/commits/{name}"),
    },
    protected: false,
  }
}
