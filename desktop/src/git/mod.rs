pub mod diff;
mod operations;
mod repository;

pub use diff::DiffEngine;
pub use repository::GitRepository;

use crate::error::{Error, Result};
use crate::state::{FileStatus, FileStatusKind, GitStatus};
use git2::{Repository, StatusOptions};
use std::path::{Path, PathBuf};

/// Detect if a path contains a git repository
pub fn is_git_repository(path: &Path) -> bool {
  Repository::discover(path).is_ok()
}

/// Open a git repository at the given path
pub fn open_repository(path: &Path) -> Result<GitRepository> {
  GitRepository::open(path)
}

/// Get the status of all files in the repository
pub fn get_repository_status(repo: &Repository) -> Result<GitStatus> {
  let mut opts = StatusOptions::new();
  opts.include_untracked(true);
  opts.recurse_untracked_dirs(false);

  let statuses = repo.statuses(Some(&mut opts))?;
  let mut files = Vec::new();

  for entry in statuses.iter() {
    if let Some(path) = entry.path() {
      let status = entry.status();
      let path_buf = PathBuf::from(path);

      // Determine file status kind
      let (status_kind, staged) = if status.is_index_new() {
        (FileStatusKind::Added, true)
      } else if status.is_index_modified() {
        (FileStatusKind::Modified, true)
      } else if status.is_index_deleted() {
        (FileStatusKind::Deleted, true)
      } else if status.is_index_renamed() {
        (FileStatusKind::Renamed { from: None }, true)
      } else if status.is_wt_new() {
        (FileStatusKind::Untracked, false)
      } else if status.is_wt_modified() {
        (FileStatusKind::Modified, false)
      } else if status.is_wt_deleted() {
        (FileStatusKind::Deleted, false)
      } else if status.is_wt_renamed() {
        (FileStatusKind::Renamed { from: None }, false)
      } else {
        continue;
      };

      files.push(FileStatus {
        path: path_buf,
        status: status_kind,
        staged,
      });
    }
  }

  // Get branch information
  let head = repo.head().ok();
  let branch = head
    .as_ref()
    .and_then(|h| h.shorthand())
    .map(|s| s.to_string());

  // Count ahead/behind commits
  let (ahead, behind) = if let Some(head_ref) = head {
    if let (Ok(local_oid), Ok(upstream)) = (
      head_ref
        .target()
        .ok_or(Error::Unknown("No head target".into())),
      repo.branch_upstream_name(head_ref.name().unwrap_or("HEAD")),
    ) {
      if let Ok(upstream_ref) = repo.find_reference(&upstream.as_str().unwrap_or("")) {
        if let Some(upstream_oid) = upstream_ref.target() {
          match repo.graph_ahead_behind(local_oid, upstream_oid) {
            Ok((ahead, behind)) => (ahead, behind),
            Err(_) => (0, 0),
          }
        } else {
          (0, 0)
        }
      } else {
        (0, 0)
      }
    } else {
      (0, 0)
    }
  } else {
    (0, 0)
  };

  Ok(GitStatus {
    files,
    branch,
    ahead,
    behind,
  })
}

/// Find the root of a git repository from any path within it
pub fn find_repository_root(path: &Path) -> Result<PathBuf> {
  let repo = Repository::discover(path)?;
  let workdir = repo
    .workdir()
    .ok_or_else(|| Error::InvalidRepositoryPath(path.to_string_lossy().to_string()))?;
  Ok(workdir.to_path_buf())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use tempfile::tempdir;

  #[test]
  fn test_is_git_repository() {
    let dir = tempdir().unwrap();
    assert!(!is_git_repository(dir.path()));

    Repository::init(dir.path()).unwrap();
    assert!(is_git_repository(dir.path()));
  }

  #[test]
  fn test_open_repository() {
    let dir = tempdir().unwrap();
    Repository::init(dir.path()).unwrap();

    let repo = open_repository(dir.path());
    assert!(repo.is_ok());
  }

  #[test]
  fn test_find_repository_root() {
    let dir = tempdir().unwrap();
    Repository::init(dir.path()).unwrap();

    let subdir = dir.path().join("subdir");
    fs::create_dir(&subdir).unwrap();

    let root = find_repository_root(&subdir).unwrap();
    assert_eq!(root, dir.path());
  }
}
