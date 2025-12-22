use crate::error::{Error, Result};
use git2::{
  Cred, FetchOptions, IndexAddOption, PushOptions, RemoteCallbacks, Repository, Signature,
};

use std::path::Path;

/// Stage a file in the repository
pub fn stage_file(repo: &Repository, path: &Path) -> Result<()> {
  let mut index = repo.index()?;
  index.add_path(path)?;
  index.write()?;
  Ok(())
}

/// Stage all files matching a pathspec
pub fn stage_files(repo: &Repository, pathspecs: &[&Path]) -> Result<()> {
  let mut index = repo.index()?;
  for path in pathspecs {
    index.add_path(path)?;
  }
  index.write()?;
  Ok(())
}

/// Stage all changes in the repository
pub fn stage_all(repo: &Repository) -> Result<()> {
  let mut index = repo.index()?;
  index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
  index.write()?;
  Ok(())
}

/// Unstage a file in the repository
pub fn unstage_file(repo: &Repository, path: &Path) -> Result<()> {
  let head = repo.head()?;
  let head_commit = head.peel_to_commit()?;
  let head_tree = head_commit.tree()?;

  let mut index = repo.index()?;

  // Reset the file in the index to the HEAD version
  let entry = head_tree.get_path(path);
  if let Ok(entry) = entry {
    index.add(&git2::IndexEntry {
      ctime: git2::IndexTime::new(0, 0),
      mtime: git2::IndexTime::new(0, 0),
      dev: 0,
      ino: 0,
      mode: entry.filemode() as u32,
      uid: 0,
      gid: 0,
      file_size: 0,
      id: entry.id(),
      flags: 0,
      flags_extended: 0,
      path: path.to_string_lossy().as_bytes().to_vec(),
    })?;
  } else {
    // File doesn't exist in HEAD, remove it from index
    index.remove_path(path)?;
  }

  index.write()?;
  Ok(())
}

/// Unstage all changes
pub fn unstage_all(repo: &Repository) -> Result<()> {
  let head = repo.head()?;
  let head_commit = head.peel_to_commit()?;
  let _head_tree = head_commit.tree()?;

  repo.reset(head_commit.as_object(), git2::ResetType::Mixed, None)?;

  Ok(())
}

/// Commit staged changes
pub fn commit(repo: &Repository, message: &str) -> Result<git2::Oid> {
  let signature = get_signature(repo)?;
  let mut index = repo.index()?;
  let tree_id = index.write_tree()?;
  let tree = repo.find_tree(tree_id)?;

  let head = repo.head()?;
  let parent_commit = head.peel_to_commit()?;

  let oid = repo.commit(
    Some("HEAD"),
    &signature,
    &signature,
    message,
    &tree,
    &[&parent_commit],
  )?;

  Ok(oid)
}

/// Create an initial commit (for repositories with no commits)
pub fn initial_commit(repo: &Repository, message: &str) -> Result<git2::Oid> {
  let signature = get_signature(repo)?;
  let mut index = repo.index()?;
  let tree_id = index.write_tree()?;
  let tree = repo.find_tree(tree_id)?;

  let oid = repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])?;

  Ok(oid)
}

/// Get the git signature from config or use defaults
fn get_signature(repo: &Repository) -> Result<Signature<'static>> {
  let config = repo.config()?;

  let name = config
    .get_string("user.name")
    .unwrap_or_else(|_| "Unknown".to_string());

  let email = config
    .get_string("user.email")
    .unwrap_or_else(|_| "unknown@example.com".to_string());

  Ok(Signature::now(&name, &email)?)
}

/// Push changes to remote
pub fn push(repo: &Repository, remote_name: &str, branch: &str) -> Result<()> {
  let mut remote = repo.find_remote(remote_name)?;

  let mut callbacks = RemoteCallbacks::new();
  callbacks.credentials(|_url, username_from_url, _allowed_types| {
    Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
  });

  let mut push_options = PushOptions::new();
  push_options.remote_callbacks(callbacks);

  let refspec = format!("refs/heads/{}:refs/heads/{}", branch, branch);
  remote.push(&[&refspec], Some(&mut push_options))?;

  Ok(())
}

/// Push to the default remote (usually origin)
pub fn push_default(repo: &Repository) -> Result<()> {
  let head = repo.head()?;
  let branch = head
    .shorthand()
    .ok_or_else(|| Error::Unknown("Could not determine current branch".into()))?;

  push(repo, "origin", branch)
}

/// Pull changes from remote
pub fn pull(repo: &Repository, remote_name: &str, branch: &str) -> Result<()> {
  // Fetch first
  fetch(repo, remote_name, branch)?;

  // Then merge
  let fetch_head = repo.find_reference("FETCH_HEAD")?;
  let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;

  // Perform the merge
  let analysis = repo.merge_analysis(&[&fetch_commit])?;

  if analysis.0.is_up_to_date() {
    Ok(())
  } else if analysis.0.is_fast_forward() {
    // Fast-forward merge
    let refname = format!("refs/heads/{}", branch);
    let mut reference = repo.find_reference(&refname)?;
    reference.set_target(fetch_commit.id(), "Fast-forward")?;
    repo.set_head(&refname)?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
    Ok(())
  } else {
    // Normal merge - this is more complex and might require conflict resolution
    repo.merge(&[&fetch_commit], None, None)?;

    // Check if there are conflicts
    let index = repo.index()?;
    if index.has_conflicts() {
      return Err(Error::Unknown("Merge conflicts detected".into()));
    }

    // Create merge commit
    let signature = get_signature(repo)?;
    let mut index = repo.index()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let head = repo.head()?;
    let head_commit = head.peel_to_commit()?;

    let message = format!("Merge branch '{}'", branch);
    let fetch_oid = fetch_commit.id();
    let fetch_commit_obj = repo.find_commit(fetch_oid)?;
    repo.commit(
      Some("HEAD"),
      &signature,
      &signature,
      &message,
      &tree,
      &[&head_commit, &fetch_commit_obj],
    )?;

    // Clean up
    repo.cleanup_state()?;

    Ok(())
  }
}

/// Pull from the default remote (usually origin)
pub fn pull_default(repo: &Repository) -> Result<()> {
  let head = repo.head()?;
  let branch = head
    .shorthand()
    .ok_or_else(|| Error::Unknown("Could not determine current branch".into()))?;

  pull(repo, "origin", branch)
}

/// Fetch changes from remote
pub fn fetch(repo: &Repository, remote_name: &str, branch: &str) -> Result<()> {
  let mut remote = repo.find_remote(remote_name)?;

  let mut callbacks = RemoteCallbacks::new();
  callbacks.credentials(|_url, username_from_url, _allowed_types| {
    Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
  });

  let mut fetch_options = FetchOptions::new();
  fetch_options.remote_callbacks(callbacks);

  remote.fetch(&[branch], Some(&mut fetch_options), None)?;

  Ok(())
}

/// Check if there are unpushed commits
pub fn has_unpushed_commits(repo: &Repository, remote_name: &str) -> Result<bool> {
  let head = repo.head()?;
  let local_oid = head
    .target()
    .ok_or_else(|| Error::Unknown("No head target".into()))?;

  let branch_name = head
    .shorthand()
    .ok_or_else(|| Error::Unknown("No branch name".into()))?;

  let upstream_name = format!("refs/remotes/{}/{}", remote_name, branch_name);
  let upstream_ref = repo.find_reference(&upstream_name).ok();

  if let Some(upstream_ref) = upstream_ref {
    if let Some(upstream_oid) = upstream_ref.target() {
      let (ahead, _behind) = repo.graph_ahead_behind(local_oid, upstream_oid)?;
      return Ok(ahead > 0);
    }
  }

  Ok(false)
}

/// Check if there are unpulled commits
pub fn has_unpulled_commits(repo: &Repository, remote_name: &str) -> Result<bool> {
  let head = repo.head()?;
  let local_oid = head
    .target()
    .ok_or_else(|| Error::Unknown("No head target".into()))?;

  let branch_name = head
    .shorthand()
    .ok_or_else(|| Error::Unknown("No branch name".into()))?;

  let upstream_name = format!("refs/remotes/{}/{}", remote_name, branch_name);
  let upstream_ref = repo.find_reference(&upstream_name).ok();

  if let Some(upstream_ref) = upstream_ref {
    if let Some(upstream_oid) = upstream_ref.target() {
      let (_ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid)?;
      return Ok(behind > 0);
    }
  }

  Ok(false)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use tempfile::tempdir;

  fn setup_repo() -> (tempfile::TempDir, Repository) {
    let dir = tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();

    // Configure user
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Test User").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();

    (dir, repo)
  }

  #[test]
  fn test_stage_file() {
    let (dir, repo) = setup_repo();

    // Create a file
    let file_path = dir.path().join("test.txt");
    fs::write(&file_path, "content").unwrap();

    // Stage the file
    let result = stage_file(&repo, Path::new("test.txt"));
    assert!(result.is_ok());

    // Verify it's staged
    let statuses = repo.statuses(None).unwrap();
    assert_eq!(statuses.len(), 1);
    assert!(statuses.get(0).unwrap().status().is_index_new());
  }

  #[test]
  fn test_commit() {
    let (dir, repo) = setup_repo();

    // Create and stage a file
    let file_path = dir.path().join("test.txt");
    fs::write(&file_path, "content").unwrap();
    stage_file(&repo, Path::new("test.txt")).unwrap();

    // Commit
    let result = initial_commit(&repo, "Initial commit");
    assert!(result.is_ok());

    // Verify commit exists
    let head = repo.head().unwrap();
    let commit = head.peel_to_commit().unwrap();
    assert_eq!(commit.message().unwrap(), "Initial commit");
  }

  #[test]
  fn test_stage_unstage() {
    let (dir, repo) = setup_repo();

    // Create initial commit
    let file_path = dir.path().join("test.txt");
    fs::write(&file_path, "initial").unwrap();
    stage_file(&repo, Path::new("test.txt")).unwrap();
    initial_commit(&repo, "Initial commit").unwrap();

    // Modify the file
    fs::write(&file_path, "modified").unwrap();

    // Stage the modification
    stage_file(&repo, Path::new("test.txt")).unwrap();

    // Verify it's staged
    let statuses = repo.statuses(None).unwrap();
    assert_eq!(statuses.len(), 1);
    assert!(statuses.get(0).unwrap().status().is_index_modified());

    // Unstage
    unstage_file(&repo, Path::new("test.txt")).unwrap();

    // Verify it's unstaged
    let statuses = repo.statuses(None).unwrap();
    assert_eq!(statuses.len(), 1);
    assert!(statuses.get(0).unwrap().status().is_wt_modified());
  }
}
