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
