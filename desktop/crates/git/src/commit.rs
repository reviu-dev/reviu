use std::path::Path;

use anyhow::{Context, Result, bail};
use git2::{BranchType, Cred, PushOptions, RemoteCallbacks, Repository, ResetType, Signature};

#[derive(Debug, Clone, Copy)]
pub struct HeadCommitStatus {
  pub has_head_commit: bool,
  pub can_undo_last_commit: bool,
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
  let mut index = repo.index()?;
  let tree_id = index.write_tree()?;
  let tree = repo.find_tree(tree_id)?;

  let signature = repo
    .signature()
    .or_else(|_| Signature::now("reviu", "reviu@contact"))?;

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

  let signature = repo
    .signature()
    .or_else(|_| Signature::now("reviu", "reviu@contact"))?;

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
  let Some(info) = upstream_info(&repo)? else {
    bail!("no upstream configured");
  };

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
  Ok(())
}

struct UpstreamInfo {
  remote: String,
  remote_branch: String,
  local_branch: String,
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
      Repository::init(&path).expect("init git repository");
      Self { path }
    }
  }

  impl Drop for TempRepo {
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
  fn head_commit_status_is_false_for_repo_without_commits() {
    let repo = TempRepo::init("commit-head-empty");
    let status = head_commit_status(&repo.path).expect("head status");
    assert!(!status.has_head_commit);
    assert!(!status.can_undo_last_commit);
  }

  #[test]
  fn push_fails_without_upstream_configuration() {
    let repo = TempRepo::init("commit-push-upstream");
    commit_text_file(&repo.path, Path::new("README.md"), "hello\n", "initial");

    let err = push(&repo.path, false).err();
    assert!(err.is_some());
    assert!(
      err
        .expect("push error")
        .to_string()
        .contains("no upstream configured")
    );
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
