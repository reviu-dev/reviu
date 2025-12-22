use crate::error::{Error, Result};
use git2::{BranchType, Remote, Repository};
use std::path::{Path, PathBuf};

/// Wrapper around git2::Repository with additional metadata
pub struct GitRepository {
  repo: Repository,
  path: PathBuf,
  info: RepositoryInfo,
}

impl GitRepository {
  /// Open a git repository at the given path
  pub fn open(path: &Path) -> Result<Self> {
    let repo = Repository::open(path)?;
    let path = repo
      .workdir()
      .ok_or_else(|| Error::InvalidRepositoryPath(path.to_string_lossy().to_string()))?
      .to_path_buf();

    let info = Self::collect_info(&repo)?;

    Ok(Self { repo, path, info })
  }

  /// Discover and open a git repository from any path within it
  pub fn discover(path: &Path) -> Result<Self> {
    let repo = Repository::discover(path)?;
    let path = repo
      .workdir()
      .ok_or_else(|| Error::InvalidRepositoryPath(path.to_string_lossy().to_string()))?
      .to_path_buf();

    let info = Self::collect_info(&repo)?;

    Ok(Self { repo, path, info })
  }

  /// Collect repository information
  fn collect_info(repo: &Repository) -> Result<RepositoryInfo> {
    let name = repo
      .workdir()
      .and_then(|p| p.file_name())
      .and_then(|n| n.to_str())
      .unwrap_or("Unknown")
      .to_string();

    let head = repo.head().ok();
    let branch = head
      .as_ref()
      .and_then(|h| h.shorthand())
      .map(|s| s.to_string());

    let remote = Self::get_default_remote_name(repo);

    Ok(RepositoryInfo {
      name,
      head: branch,
      remote,
    })
  }

  /// Get the default remote (usually "origin")
  fn get_default_remote_name(repo: &Repository) -> Option<String> {
    // Try to get the upstream remote for the current branch
    if let Ok(head) = repo.head() {
      if let Some(branch_name) = head.shorthand() {
        if let Ok(branch) = repo.find_branch(branch_name, BranchType::Local) {
          if let Ok(upstream) = branch.upstream() {
            if let Some(upstream_name) = upstream.name().ok().flatten() {
              // Extract remote name from upstream ref (e.g., "refs/remotes/origin/main" -> "origin")
              if let Some(remote_name) = upstream_name.strip_prefix("refs/remotes/") {
                if let Some(remote) = remote_name.split('/').next() {
                  return Some(remote.to_string());
                }
              }
            }
          }
        }
      }
    }

    // Fallback to "origin" if it exists
    if repo.find_remote("origin").is_ok() {
      return Some("origin".to_string());
    }

    // Return first available remote
    if let Ok(remotes) = repo.remotes() {
      if let Some(first_remote) = remotes.get(0) {
        return Some(first_remote.to_string());
      }
    }

    None
  }

  /// Get the underlying git2::Repository
  pub fn repo(&self) -> &Repository {
    &self.repo
  }

  /// Get the repository path
  pub fn path(&self) -> &Path {
    &self.path
  }

  /// Get repository information
  pub fn info(&self) -> &RepositoryInfo {
    &self.info
  }

  /// Refresh repository information
  pub fn refresh_info(&mut self) -> Result<()> {
    self.info = Self::collect_info(&self.repo)?;
    Ok(())
  }

  /// Get the current branch name
  pub fn current_branch(&self) -> Result<Option<String>> {
    let head = self.repo.head()?;
    Ok(head.shorthand().map(|s| s.to_string()))
  }

  /// Check if the repository has uncommitted changes
  pub fn has_changes(&self) -> Result<bool> {
    let statuses = self.repo.statuses(None)?;
    Ok(!statuses.is_empty())
  }

  /// Check if the repository is bare
  pub fn is_bare(&self) -> bool {
    self.repo.is_bare()
  }

  /// Get the remote by name
  pub fn get_remote(&self, name: &str) -> Result<Remote> {
    Ok(self.repo.find_remote(name)?)
  }

  /// Get the default remote
  pub fn get_default_remote(&self) -> Result<Remote> {
    let remote_name = self
      .info
      .remote
      .as_ref()
      .ok_or_else(|| Error::Unknown("No remote configured".into()))?;
    self.get_remote(remote_name)
  }

  /// Get the URL of the default remote
  pub fn remote_url(&self) -> Result<Option<String>> {
    if let Some(remote_name) = &self.info.remote {
      let remote = self.repo.find_remote(remote_name)?;
      Ok(remote.url().map(|s| s.to_string()))
    } else {
      Ok(None)
    }
  }

  /// Check if the repository has a remote configured
  pub fn has_remote(&self) -> bool {
    self.info.remote.is_some()
  }
}

/// Repository information
#[derive(Debug, Clone)]
pub struct RepositoryInfo {
  pub name: String,
  pub head: Option<String>,
  pub remote: Option<String>,
}

#[cfg(test)]
mod tests {
  use super::*;
  use git2::Signature;
  use std::fs;
  use tempfile::tempdir;

  #[test]
  fn test_open_repository() {
    let dir = tempdir().unwrap();
    Repository::init(dir.path()).unwrap();

    let git_repo = GitRepository::open(dir.path());
    assert!(git_repo.is_ok());
  }

  #[test]
  fn test_discover_repository() {
    let dir = tempdir().unwrap();
    Repository::init(dir.path()).unwrap();

    let subdir = dir.path().join("subdir");
    fs::create_dir(&subdir).unwrap();

    let git_repo = GitRepository::discover(&subdir);
    assert!(git_repo.is_ok());
  }

  #[test]
  fn test_repository_info() {
    let dir = tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();

    // Create initial commit
    let sig = Signature::now("Test", "test@example.com").unwrap();
    let tree_id = {
      let mut index = repo.index().unwrap();
      index.write_tree().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    repo
      .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
      .unwrap();

    let git_repo = GitRepository::open(dir.path()).unwrap();
    let info = git_repo.info();

    assert!(!info.name.is_empty());
    assert!(info.head.is_some());
  }

  #[test]
  fn test_has_changes() {
    let dir = tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();

    let git_repo = GitRepository::open(dir.path()).unwrap();

    // No changes initially
    assert!(!git_repo.has_changes().unwrap());

    // Create a file
    fs::write(dir.path().join("test.txt"), "content").unwrap();

    // Should detect changes
    assert!(git_repo.has_changes().unwrap());
  }
}
