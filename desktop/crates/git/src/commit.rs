use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};
use git2::{
  BranchType, Cred, PushOptions, RemoteCallbacks, Repository, RepositoryState, ResetType, Signature,
};

#[derive(Debug, Clone, Copy)]
pub struct HeadCommitStatus {
  pub has_head_commit: bool,
  pub can_undo_last_commit: bool,
}

fn repo_signature(repo: &Repository) -> Result<Signature<'_>> {
  repo
    .signature()
    .context("git user identity is not configured (set user.name and user.email)")
}

fn command_output_details(output: &std::process::Output) -> String {
  let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
  let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
  [stderr, stdout]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

pub fn head_commit_status(repo_root: &Path) -> Result<HeadCommitStatus> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;

  let head = match repo.head() {
    Ok(head) => head,
    Err(_) => {
      return Ok(HeadCommitStatus {
        has_head_commit: false,
        can_undo_last_commit: false,
      });
    }
  };

  let commit = match head.peel_to_commit() {
    Ok(commit) => commit,
    Err(_) => {
      return Ok(HeadCommitStatus {
        has_head_commit: false,
        can_undo_last_commit: false,
      });
    }
  };

  Ok(HeadCommitStatus {
    has_head_commit: true,
    can_undo_last_commit: commit.parent_count() > 0,
  })
}

pub fn commit_changes(repo_root: &Path, message: &str) -> Result<()> {
  let message = message.trim();
  if message.is_empty() {
    bail!("commit message is empty");
  }

  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  if repo.state() == RepositoryState::Merge {
    if repo.index()?.has_conflicts() {
      bail!("merge has conflicts");
    }

    let output = Command::new("git")
      .current_dir(repo_root)
      .args(["commit", "-m", message])
      .env("GIT_EDITOR", ":")
      .output()
      .context("run git commit for merge")?;

    if output.status.success() {
      return Ok(());
    }

    let details = command_output_details(&output);
    if details.to_ascii_lowercase().contains("conflict") {
      bail!("merge has conflicts");
    }
    if details.is_empty() {
      bail!("commit failed");
    }
    bail!("commit failed: {details}");
  }

  let mut index = repo.index()?;
  let tree_id = index.write_tree()?;
  let tree = repo.find_tree(tree_id)?;

  let signature = repo_signature(&repo)?;

  let parent_commit = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
  if let Some(parent) = parent_commit.as_ref() {
    repo.commit(
      Some("HEAD"),
      &signature,
      &signature,
      message,
      &tree,
      &[parent],
    )?;
  } else {
    repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])?;
  }

  Ok(())
}

pub fn amend_commit(repo_root: &Path, message: Option<&str>) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head = repo
    .head()
    .and_then(|head| head.peel_to_commit())
    .context("read HEAD commit")?;

  let mut index = repo.index()?;
  let tree_id = index.write_tree()?;
  let tree = repo.find_tree(tree_id)?;

  let signature = repo_signature(&repo)?;

  let message = message.map(|msg| msg.trim()).filter(|msg| !msg.is_empty());

  head.amend(
    Some("HEAD"),
    Some(&signature),
    Some(&signature),
    None,
    message,
    Some(&tree),
  )?;

  Ok(())
}

pub fn undo_last_commit(repo_root: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head = repo
    .head()
    .and_then(|head| head.peel_to_commit())
    .context("read HEAD commit")?;
  let parent = head.parent(0).context("HEAD has no parent")?;

  repo.reset(parent.as_object(), ResetType::Soft, None)?;
  Ok(())
}

pub fn push(repo_root: &Path, force: bool) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let Some((info, should_set_upstream)) = push_target_info(&repo)? else {
    bail!("no upstream configured and no publish remote available");
  };

  {
    let mut remote = repo.find_remote(&info.remote)?;
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_, username_from_url, _| {
      if let Some(username) = username_from_url {
        Cred::ssh_key_from_agent(username).or_else(|_| Cred::default())
      } else {
        Cred::default()
      }
    });

    let mut options = PushOptions::new();
    options.remote_callbacks(callbacks);

    let local_ref = format!("refs/heads/{}", info.local_branch);
    let remote_ref = format!("refs/heads/{}", info.remote_branch);
    let refspec = if force {
      format!("+{}:{}", local_ref, remote_ref)
    } else {
      format!("{}:{}", local_ref, remote_ref)
    };

    remote.push(&[refspec], Some(&mut options))?;
  }

  if should_set_upstream {
    let mut local_branch = repo.find_branch(&info.local_branch, BranchType::Local)?;
    let upstream_ref = format!("{}/{}", info.remote, info.remote_branch);
    local_branch.set_upstream(Some(&upstream_ref))?;
  }
  Ok(())
}

struct UpstreamInfo {
  remote: String,
  remote_branch: String,
  local_branch: String,
}

fn push_target_info(repo: &Repository) -> Result<Option<(UpstreamInfo, bool)>> {
  if let Some(info) = upstream_info(repo)? {
    return Ok(Some((info, false)));
  }

  Ok(publish_info(repo)?.map(|info| (info, true)))
}

fn upstream_info(repo: &Repository) -> Result<Option<UpstreamInfo>> {
  let head = match repo.head() {
    Ok(head) => head,
    Err(_) => return Ok(None),
  };
  if !head.is_branch() {
    return Ok(None);
  }
  let local_name = head.shorthand().unwrap_or("HEAD");
  let local_branch = repo.find_branch(local_name, BranchType::Local)?;
  let upstream = match local_branch.upstream() {
    Ok(upstream) => upstream,
    Err(_) => return Ok(None),
  };
  let upstream_name = upstream.name()?.unwrap_or("");
  if upstream_name.is_empty() {
    return Ok(None);
  }

  let mut parts = upstream_name.splitn(2, '/');
  let remote = parts.next().unwrap_or("origin").to_string();
  let remote_branch = parts.next().unwrap_or(local_name).to_string();

  Ok(Some(UpstreamInfo {
    remote,
    remote_branch,
    local_branch: local_name.to_string(),
  }))
}

fn publish_info(repo: &Repository) -> Result<Option<UpstreamInfo>> {
  let head = match repo.head() {
    Ok(head) => head,
    Err(_) => return Ok(None),
  };
  if !head.is_branch() {
    return Ok(None);
  }

  let local_branch = head.shorthand().unwrap_or("HEAD").to_string();
  if local_branch == "HEAD" {
    return Ok(None);
  }
  let _ = repo.find_branch(&local_branch, BranchType::Local)?;

  let Some(remote) = default_publish_remote(repo)? else {
    return Ok(None);
  };

  Ok(Some(UpstreamInfo {
    remote,
    remote_branch: local_branch.clone(),
    local_branch,
  }))
}

fn default_publish_remote(repo: &Repository) -> Result<Option<String>> {
  if repo.find_remote("origin").is_ok() {
    return Ok(Some("origin".to_string()));
  }

  let remotes = repo.remotes().context("list remotes")?;
  let mut remote_names = remotes.iter().flatten().map(ToString::to_string);
  let first = remote_names.next();
  if first.is_none() || remote_names.next().is_some() {
    return Ok(None);
  }

  Ok(first)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
  };

  struct TempRepo {
    path: PathBuf,
  }

  impl TempRepo {
    fn init(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!("reviu-{prefix}-{}-{nanos}", std::process::id()));
      std::fs::create_dir_all(&path).expect("create temp dir");
      let repo = Repository::init(&path).expect("init git repository");
      let mut config = repo.config().expect("open git config");
      config
        .set_str("user.name", "Reviu Tests")
        .expect("set git user.name");
      config
        .set_str("user.email", "tests@reviu.local")
        .expect("set git user.email");
      Self { path }
    }
  }

  impl Drop for TempRepo {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  struct TempBareRepo {
    path: PathBuf,
  }

  impl TempBareRepo {
    fn init(prefix: &str) -> Self {
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
      Repository::init_bare(&path).expect("init bare git repository");
      Self { path }
    }
  }

  impl Drop for TempBareRepo {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  struct TempDir {
    path: PathBuf,
  }

  impl TempDir {
    fn new(prefix: &str) -> Self {
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

  fn commit_text_file(repo_root: &Path, rel_path: &Path, contents: &str, message: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::write(repo_root.join(rel_path), contents).expect("write worktree file");

    let mut index = repo.index().expect("open index");
    index.add_path(rel_path).expect("stage file");
    index.write().expect("write index");

    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("find tree");
    let sig = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());

    match parent {
      Some(parent) => {
        repo
          .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
          .expect("commit with parent");
      }
      None => {
        repo
          .commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
          .expect("initial commit");
      }
    }
  }

  fn stage_text_file(repo_root: &Path, rel_path: &Path, contents: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::write(repo_root.join(rel_path), contents).expect("write worktree file");

    let mut index = repo.index().expect("open index");
    index.add_path(rel_path).expect("stage file");
    index.write().expect("write index");
  }

  fn branch_name(repo_root: &Path) -> String {
    Repository::open(repo_root)
      .expect("open repo")
      .head()
      .ok()
      .and_then(|head| head.shorthand().map(ToString::to_string))
      .unwrap_or_else(|| "HEAD".to_string())
  }

  fn head_oid(repo_root: &Path) -> git2::Oid {
    Repository::open(repo_root)
      .expect("open repo")
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head commit")
      .id()
  }

  fn remote_branch_oid(repo_root: &Path, branch_name: &str) -> git2::Oid {
    let refname = format!("refs/heads/{branch_name}");
    Repository::open(repo_root)
      .expect("open repo")
      .refname_to_id(&refname)
      .expect("read remote branch oid")
  }

  fn push_branch_to_remote(repo_root: &Path, branch_name: &str, remote_name: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    let mut remote = repo.find_remote(remote_name).expect("find remote");
    let refspec = format!("refs/heads/{branch_name}:refs/heads/{branch_name}");
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_, _, _| Cred::default());
    let mut options = PushOptions::new();
    options.remote_callbacks(callbacks);
    remote
      .push(&[refspec], Some(&mut options))
      .expect("push branch");
  }

  fn set_upstream(repo_root: &Path, local_branch: &str, upstream_branch: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    let mut branch = repo
      .find_branch(local_branch, BranchType::Local)
      .expect("find local branch");
    branch
      .set_upstream(Some(upstream_branch))
      .expect("set upstream");
  }

  fn set_remote_head(remote_root: &Path, branch_name: &str) {
    let refname = format!("refs/heads/{branch_name}");
    Repository::open(remote_root)
      .expect("open remote")
      .set_head(&refname)
      .expect("set remote HEAD");
  }

  #[test]
  fn commit_changes_rejects_empty_message() {
    let repo = TempRepo::init("commit-empty");
    let err = commit_changes(&repo.path, "   ").err();
    assert!(err.is_some());
    assert!(
      err
        .expect("error")
        .to_string()
        .contains("commit message is empty")
    );
  }

  #[test]
  fn commit_changes_creates_initial_commit_with_trimmed_message() {
    let repo = TempRepo::init("commit-success-initial");
    let rel_path = Path::new("README.md");
    stage_text_file(&repo.path, rel_path, "hello\n");

    commit_changes(&repo.path, "  initial message  ").expect("commit changes");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.summary(), Some("initial message"));
    assert_eq!(head.parent_count(), 0);
  }

  #[test]
  fn commit_changes_creates_commit_with_parent_when_head_exists() {
    let repo = TempRepo::init("commit-success-parent");
    let rel_path = Path::new("README.md");
    commit_text_file(&repo.path, rel_path, "v1\n", "first");

    stage_text_file(&repo.path, rel_path, "v2\n");
    commit_changes(&repo.path, "second").expect("commit changes");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.summary(), Some("second"));
    assert_eq!(head.parent_count(), 1);
    assert_eq!(head.parent(0).expect("parent").summary(), Some("first"));
  }

  #[test]
  fn head_commit_status_is_false_for_repo_without_commits() {
    let repo = TempRepo::init("commit-head-empty");
    let status = head_commit_status(&repo.path).expect("head status");
    assert!(!status.has_head_commit);
    assert!(!status.can_undo_last_commit);
  }

  #[test]
  fn head_commit_status_tracks_undo_availability() {
    let repo = TempRepo::init("commit-head-status");
    let rel_path = Path::new("README.md");
    commit_text_file(&repo.path, rel_path, "v1\n", "first");

    let single = head_commit_status(&repo.path).expect("head status after first");
    assert!(single.has_head_commit);
    assert!(!single.can_undo_last_commit);

    commit_text_file(&repo.path, rel_path, "v2\n", "second");
    let double = head_commit_status(&repo.path).expect("head status after second");
    assert!(double.has_head_commit);
    assert!(double.can_undo_last_commit);
  }

  #[test]
  fn push_fails_without_upstream_or_publish_remote() {
    let repo = TempRepo::init("commit-push-upstream");
    commit_text_file(&repo.path, Path::new("README.md"), "hello\n", "initial");

    let err = push(&repo.path, false).err();
    assert!(err.is_some());
    assert!(
      err
        .expect("push error")
        .to_string()
        .contains("no upstream configured and no publish remote available")
    );
  }

  #[test]
  fn push_updates_remote_when_upstream_exists() {
    let local = TempRepo::init("commit-push-success-local");
    let remote = TempBareRepo::init("commit-push-success-remote");
    let rel_path = Path::new("README.md");

    commit_text_file(&local.path, rel_path, "v1\n", "initial");

    let local_repo = Repository::open(&local.path).expect("open local");
    local_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin");

    let local_branch = branch_name(&local.path);
    push_branch_to_remote(&local.path, &local_branch, "origin");
    set_upstream(
      &local.path,
      &local_branch,
      &format!("origin/{local_branch}"),
    );
    set_remote_head(&remote.path, &local_branch);

    commit_text_file(&local.path, rel_path, "v2-local\n", "local change");
    let expected_head = head_oid(&local.path);

    push(&local.path, false).expect("push without force");

    assert_eq!(
      remote_branch_oid(&remote.path, &local_branch),
      expected_head
    );
  }

  #[test]
  fn push_publishes_branch_and_sets_upstream_when_remote_exists() {
    let local = TempRepo::init("commit-push-publish-local");
    let remote = TempBareRepo::init("commit-push-publish-remote");
    let rel_path = Path::new("README.md");

    commit_text_file(&local.path, rel_path, "v1\n", "initial");

    let local_repo = Repository::open(&local.path).expect("open local");
    local_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin");

    let local_branch = branch_name(&local.path);
    let expected_head = head_oid(&local.path);

    push(&local.path, false).expect("publish branch");

    assert_eq!(
      remote_branch_oid(&remote.path, &local_branch),
      expected_head
    );
    let repo_handle = Repository::open(&local.path).expect("open local after publish");
    let branch = repo_handle
      .find_branch(&local_branch, BranchType::Local)
      .expect("find local branch");
    let upstream_name = branch
      .upstream()
      .expect("upstream configured")
      .name()
      .expect("read upstream name")
      .unwrap_or("")
      .to_string();
    assert_eq!(upstream_name, format!("origin/{local_branch}"));
  }

  #[test]
  fn push_force_overwrites_remote_after_non_fast_forward_rejection() {
    let source = TempRepo::init("commit-push-force-source");
    let remote = TempBareRepo::init("commit-push-force-remote");
    let peer_dir = TempDir::new("commit-push-force-peer");
    let rel_path = Path::new("README.md");

    commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin");

    let local_branch = branch_name(&source.path);
    push_branch_to_remote(&source.path, &local_branch, "origin");
    set_upstream(
      &source.path,
      &local_branch,
      &format!("origin/{local_branch}"),
    );
    set_remote_head(&remote.path, &local_branch);

    let _peer_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &peer_dir.path,
    )
    .expect("clone remote into peer");

    commit_text_file(&source.path, rel_path, "v2-source\n", "source change");
    let source_head = head_oid(&source.path);

    commit_text_file(&peer_dir.path, rel_path, "v2-peer\n", "peer change");
    push_branch_to_remote(&peer_dir.path, &local_branch, "origin");

    let err = push(&source.path, false).err();
    assert!(
      err.is_some(),
      "non-fast-forward push should fail without force"
    );

    push(&source.path, true).expect("force push");
    assert_eq!(remote_branch_oid(&remote.path, &local_branch), source_head);
  }

  #[test]
  fn amend_commit_keeps_message_when_none_and_updates_tree() {
    let repo = TempRepo::init("commit-amend-none");
    let rel_path = Path::new("README.md");
    commit_text_file(&repo.path, rel_path, "hello\n", "initial message");

    std::fs::write(repo.path.join(rel_path), "hello v2\n").expect("update file");
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let mut index = repo_handle.index().expect("open index");
    index.add_path(rel_path).expect("stage updated file");
    index.write().expect("write index");

    amend_commit(&repo.path, None).expect("amend commit");

    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head commit");
    assert_eq!(head.summary(), Some("initial message"));

    let tree = head.tree().expect("head tree");
    let entry = tree.get_path(rel_path).expect("entry in tree");
    let blob = repo_handle.find_blob(entry.id()).expect("blob");
    assert_eq!(
      String::from_utf8_lossy(blob.content()).as_ref(),
      "hello v2\n"
    );
  }

  #[test]
  fn amend_commit_trims_and_replaces_message() {
    let repo = TempRepo::init("commit-amend-message");
    let rel_path = Path::new("README.md");
    commit_text_file(&repo.path, rel_path, "hello\n", "initial message");

    amend_commit(&repo.path, Some("  updated message  ")).expect("amend commit");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head commit");
    assert_eq!(head.summary(), Some("updated message"));
  }

  #[test]
  fn commit_changes_completes_merge_after_conflict_resolution() {
    let repo = TempRepo::init("commit-merge-conflict-resolution");
    let rel_path = Path::new("README.md");
    commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = crate::current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    crate::create_branch(&repo.path, "feature").expect("create feature branch");

    commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    crate::switch_branch(
      &repo.path,
      &crate::BranchRef {
        name: "feature".to_string(),
        kind: crate::BranchKind::Local,
      },
    )
    .expect("switch to feature");
    commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    crate::switch_branch(
      &repo.path,
      &crate::BranchRef {
        name: base_branch.clone(),
        kind: crate::BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    repo_handle
      .checkout_head(Some(&mut checkout))
      .expect("force checkout head");

    let _ = crate::merge_branch(
      &repo.path,
      &crate::BranchRef {
        name: "feature".to_string(),
        kind: crate::BranchKind::Local,
      },
    )
    .expect_err("merge should fail with conflicts");
    assert!(
      crate::is_merge_in_progress(&repo.path).expect("read merge state"),
      "merge state should be active after conflict"
    );

    stage_text_file(&repo.path, rel_path, "resolved\n");

    commit_changes(&repo.path, "Merge branch 'feature' into main").expect("commit merge");

    assert!(
      !crate::is_merge_in_progress(&repo.path).expect("read merge state after commit"),
      "merge state should be cleaned after commit"
    );
    let repo_handle = Repository::open(&repo.path).expect("reopen repo after merge commit");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head after merge commit");
    assert_eq!(head.parent_count(), 2);
    assert_eq!(head.summary(), Some("Merge branch 'feature' into main"));
  }

  #[test]
  fn undo_last_commit_fails_when_head_has_no_parent() {
    let repo = TempRepo::init("commit-undo-one");
    commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "hello\n",
      "initial message",
    );

    let err = undo_last_commit(&repo.path).err();
    assert!(err.is_some());
    assert!(
      err
        .expect("undo error")
        .to_string()
        .contains("HEAD has no parent")
    );
  }

  #[test]
  fn undo_last_commit_moves_head_to_parent() {
    let repo = TempRepo::init("commit-undo-parent");
    let rel_path = Path::new("README.md");
    commit_text_file(&repo.path, rel_path, "v1\n", "first");
    commit_text_file(&repo.path, rel_path, "v2\n", "second");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head_before = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head before");
    let parent_oid = head_before.parent(0).expect("parent").id();

    undo_last_commit(&repo.path).expect("undo commit");

    let head_after = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head after");
    assert_eq!(head_after.id(), parent_oid);
  }
}
