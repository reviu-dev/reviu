use std::fs;
use std::path::{Path, PathBuf};

use git2::{
  IndexAddOption, Repository as GitRepository, ResetType, Status, StatusOptions, Tree,
};
use git2::build::CheckoutBuilder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatusKind {
  Added,
  Modified,
  Deleted,
  Untracked,
  Renamed,
  Typechange,
  Conflicted,
}

#[derive(Clone, Debug)]
pub struct RepositoryFile {
  pub path: PathBuf,
  pub status: FileStatusKind,
  pub base_content: Option<String>,
  pub current_content: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Repository {
  pub root: PathBuf,
  pub changes: Vec<RepositoryFile>,
  pub staged: Vec<RepositoryFile>,
}

pub fn open_repository(path: &Path) -> Result<Repository, git2::Error> {
  let repo = GitRepository::discover(path)?;
  let root = repo
    .workdir()
    .map(Path::to_path_buf)
    .unwrap_or_else(|| repo.path().to_path_buf());

  let head_tree = repo
    .head()
    .ok()
    .and_then(|head| head.peel_to_commit().ok())
    .and_then(|commit| commit.tree().ok());

  let mut options = StatusOptions::new();
  options
    .include_untracked(true)
    .recurse_untracked_dirs(true)
    .include_ignored(false)
    .renames_head_to_index(true)
    .renames_index_to_workdir(true);

  let statuses = repo.statuses(Some(&mut options))?;
  let mut changes = Vec::new();
  let mut staged = Vec::new();

  for entry in statuses.iter() {
    let status = entry.status();
    let (old_path, new_path) = status_paths(&entry);
    let Some(display_path) = new_path.as_ref().or(old_path.as_ref()) else {
      continue;
    };

    let base_content = match (head_tree.as_ref(), old_path.as_ref()) {
      (Some(tree), Some(path)) => read_head_blob(&repo, tree, path),
      _ => None,
    };

    let index_content = read_index_blob(&repo, display_path);

    let mut workdir_content = new_path
      .as_ref()
      .and_then(|path| read_workdir_file(&root, path));

    if workdir_content.is_none() && status.is_wt_deleted() {
      workdir_content = Some(String::new());
    }

    if status.is_conflicted() {
      changes.push(RepositoryFile {
        path: display_path.to_path_buf(),
        status: FileStatusKind::Conflicted,
        base_content: base_content.clone(),
        current_content: workdir_content.clone(),
      });
      continue;
    }

    if let Some(kind) = status_to_kind_for_scope(status, StatusScope::Workdir) {
      let base_for_changes = index_content.clone().or_else(|| base_content.clone());
      changes.push(RepositoryFile {
        path: display_path.to_path_buf(),
        status: kind,
        base_content: base_for_changes,
        current_content: workdir_content.clone(),
      });
    }

    if let Some(kind) = status_to_kind_for_scope(status, StatusScope::Index) {
      let mut staged_content = index_content.clone();
      if staged_content.is_none() && status.is_index_deleted() {
        staged_content = Some(String::new());
      }
      staged.push(RepositoryFile {
        path: display_path.to_path_buf(),
        status: kind,
        base_content: base_content.clone(),
        current_content: staged_content,
      });
    }
  }

  changes.sort_by(|a, b| a.path.cmp(&b.path));
  staged.sort_by(|a, b| a.path.cmp(&b.path));

  Ok(Repository {
    root,
    changes,
    staged,
  })
}

fn read_workdir_file(root: &Path, rel_path: &Path) -> Option<String> {
  let bytes = fs::read(root.join(rel_path)).ok()?;
  Some(String::from_utf8_lossy(&bytes).to_string())
}

fn read_head_blob(repo: &GitRepository, tree: &Tree, path: &Path) -> Option<String> {
  let entry = tree.get_path(path).ok()?;
  let object = entry.to_object(repo).ok()?;
  let blob = object.as_blob()?;
  Some(String::from_utf8_lossy(blob.content()).to_string())
}

fn read_index_blob(repo: &GitRepository, path: &Path) -> Option<String> {
  let index = repo.index().ok()?;
  let entry = index.get_path(path, 0)?;
  let blob = repo.find_blob(entry.id).ok()?;
  Some(String::from_utf8_lossy(blob.content()).to_string())
}

fn status_paths(entry: &git2::StatusEntry<'_>) -> (Option<PathBuf>, Option<PathBuf>) {
  if let Some(delta) = entry.index_to_workdir().or_else(|| entry.head_to_index()) {
    let old_path = delta.old_file().path().map(Path::to_path_buf);
    let new_path = delta.new_file().path().map(Path::to_path_buf);
    return (old_path, new_path);
  }

  let path = entry.path().map(PathBuf::from);
  (path.clone(), path)
}

#[derive(Copy, Clone, Debug)]
enum StatusScope {
  Index,
  Workdir,
}

fn status_to_kind_for_scope(status: Status, scope: StatusScope) -> Option<FileStatusKind> {
  match scope {
    StatusScope::Index => {
      if status.is_index_new() {
        Some(FileStatusKind::Added)
      } else if status.is_index_deleted() {
        Some(FileStatusKind::Deleted)
      } else if status.is_index_modified() {
        Some(FileStatusKind::Modified)
      } else if status.is_index_renamed() {
        Some(FileStatusKind::Renamed)
      } else if status.is_index_typechange() {
        Some(FileStatusKind::Typechange)
      } else {
        None
      }
    }
    StatusScope::Workdir => {
      if status.is_wt_new() {
        Some(FileStatusKind::Untracked)
      } else if status.is_wt_deleted() {
        Some(FileStatusKind::Deleted)
      } else if status.is_wt_modified() {
        Some(FileStatusKind::Modified)
      } else if status.is_wt_renamed() {
        Some(FileStatusKind::Renamed)
      } else if status.is_wt_typechange() {
        Some(FileStatusKind::Typechange)
      } else {
        None
      }
    }
  }
}

fn repo_relative_path(repo_root: &Path, path: &Path) -> PathBuf {
  if path.is_absolute() {
    path.strip_prefix(repo_root).unwrap_or(path).to_path_buf()
  } else {
    path.to_path_buf()
  }
}

pub fn stage_path(repo_root: &Path, path: &Path) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let rel_path = repo_relative_path(&repo_root_path(&repo)?, path);
  let abs_path = repo_root_path(&repo)?.join(&rel_path);
  let mut index = repo.index()?;
  if abs_path.exists() {
    index.add_path(&rel_path)?;
  } else {
    let _ = index.remove_path(&rel_path);
  }
  index.write()?;
  Ok(())
}

pub fn stage_all(repo_root: &Path) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let mut index = repo.index()?;
  index.add_all(["*"], IndexAddOption::DEFAULT, None)?;
  index.write()?;
  Ok(())
}

pub fn unstage_path(repo_root: &Path, path: &Path) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let rel_path = repo_relative_path(&repo_root_path(&repo)?, path);
  let target_commit = repo
    .head()
    .ok()
    .and_then(|head| head.peel_to_commit().ok());
  match target_commit {
    Some(commit) => repo.reset_default(Some(commit.as_object()), &[rel_path.as_path()])?,
    None => repo.reset_default(None, &[rel_path.as_path()])?,
  }
  Ok(())
}

pub fn unstage_all(repo_root: &Path) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  match repo.head().ok().and_then(|head| head.peel_to_commit().ok()) {
    Some(commit) => {
      repo.reset(commit.as_object(), ResetType::Mixed, None)?;
    }
    None => {
      let mut index = repo.index()?;
      let _ = index.clear();
      index.write()?;
    }
  }
  Ok(())
}

pub fn discard_change(
  repo_root: &Path,
  path: &Path,
  status: FileStatusKind,
) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let rel_path = repo_relative_path(&repo_root_path(&repo)?, path);
  let abs_path = repo_root_path(&repo)?.join(&rel_path);
  if status == FileStatusKind::Untracked {
    if let Ok(metadata) = fs::metadata(&abs_path) {
      if metadata.is_dir() {
        let _ = fs::remove_dir_all(&abs_path);
      } else {
        let _ = fs::remove_file(&abs_path);
      }
    }
    return Ok(());
  }

  let mut options = CheckoutBuilder::new();
  options.force().path(&rel_path);
  repo.checkout_index(None, Some(&mut options))?;
  Ok(())
}

fn repo_root_path(repo: &GitRepository) -> Result<PathBuf, git2::Error> {
  repo
    .workdir()
    .map(|path| path.to_path_buf())
    .ok_or_else(|| git2::Error::from_str("Repository has no working directory"))
}
