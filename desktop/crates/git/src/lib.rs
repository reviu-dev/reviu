use std::fs;
use std::path::{Path, PathBuf};

use git2::{Repository as GitRepository, Status, StatusOptions, Tree};

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
  pub entries: Vec<RepositoryFile>,
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
  let mut entries = Vec::new();

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

    let mut current_content = new_path
      .as_ref()
      .and_then(|path| read_workdir_file(&root, path));

    if current_content.is_none() && (status.is_wt_deleted() || status.is_index_deleted()) {
      current_content = Some(String::new());
    }

    entries.push(RepositoryFile {
      path: display_path.to_path_buf(),
      status: status_to_kind(status),
      base_content,
      current_content,
    });
  }

  entries.sort_by(|a, b| a.path.cmp(&b.path));

  Ok(Repository { root, entries })
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

fn status_paths(entry: &git2::StatusEntry<'_>) -> (Option<PathBuf>, Option<PathBuf>) {
  if let Some(delta) = entry.index_to_workdir().or_else(|| entry.head_to_index()) {
    let old_path = delta.old_file().path().map(Path::to_path_buf);
    let new_path = delta.new_file().path().map(Path::to_path_buf);
    return (old_path, new_path);
  }

  let path = entry.path().map(PathBuf::from);
  (path.clone(), path)
}

fn status_to_kind(status: Status) -> FileStatusKind {
  if status.is_conflicted() {
    FileStatusKind::Conflicted
  } else if status.is_wt_deleted() || status.is_index_deleted() {
    FileStatusKind::Deleted
  } else if status.is_index_new() {
    FileStatusKind::Added
  } else if status.is_wt_new() {
    FileStatusKind::Untracked
  } else if status.is_wt_modified() || status.is_index_modified() {
    FileStatusKind::Modified
  } else if status.is_wt_renamed() || status.is_index_renamed() {
    FileStatusKind::Renamed
  } else if status.is_wt_typechange() || status.is_index_typechange() {
    FileStatusKind::Typechange
  } else {
    FileStatusKind::Modified
  }
}
