use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::build::CheckoutBuilder;
use git2::{ErrorCode, IndexAddOption, ObjectType, Repository, Status, StatusOptions, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoStage {
  Staged,
  Unstaged,
  PartiallyStaged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoStatusKind {
  Added,
  Modified,
  Deleted,
  Renamed,
  TypeChange,
  Untracked,
  Conflicted,
}

impl RepoStatusKind {
  pub fn short_code(self) -> &'static str {
    match self {
      RepoStatusKind::Added => "A",
      RepoStatusKind::Modified => "M",
      RepoStatusKind::Deleted => "D",
      RepoStatusKind::Renamed => "R",
      RepoStatusKind::TypeChange => "T",
      RepoStatusKind::Untracked => "U",
      RepoStatusKind::Conflicted => "U",
    }
  }
}

#[derive(Clone, Debug)]
pub struct RepoStatusEntry {
  pub path: PathBuf,
  pub old_path: Option<PathBuf>,
  pub status: RepoStatusKind,
  pub stage: RepoStage,
}

pub fn list_repo_status(repo_root: &Path) -> Result<Vec<RepoStatusEntry>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut opts = StatusOptions::new();
  opts
    .include_untracked(true)
    .recurse_untracked_dirs(true)
    .update_index(true)
    .renames_head_to_index(true)
    .renames_index_to_workdir(true)
    .include_ignored(false);

  let statuses = repo.statuses(Some(&mut opts))?;
  let mut entries = Vec::new();

  for entry in statuses.iter() {
    let status = entry.status();
    if status.is_empty() {
      continue;
    }

    let stage = stage_from_status(status);
    let kind = kind_from_status(status);
    let Some((path, old_path)) = entry_paths(&entry, kind) else {
      continue;
    };

    entries.push(RepoStatusEntry {
      path,
      old_path,
      status: kind,
      stage,
    });
  }

  entries.sort_by(|a, b| a.path.cmp(&b.path));
  Ok(entries)
}

pub fn list_repo_worktree_files(repo_root: &Path) -> Result<Vec<PathBuf>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut files = BTreeSet::new();

  let index = repo.index().context("open git index")?;
  for entry in index.iter() {
    let rel_path = PathBuf::from(String::from_utf8_lossy(entry.path.as_ref()).into_owned());
    if repo_root.join(&rel_path).is_file() {
      files.insert(rel_path);
    }
  }

  let mut opts = StatusOptions::new();
  opts
    .include_untracked(true)
    .recurse_untracked_dirs(true)
    .update_index(true)
    .include_ignored(false);
  let statuses = repo
    .statuses(Some(&mut opts))
    .context("read repository statuses")?;
  for entry in statuses.iter() {
    let Some(path) = entry.path() else {
      continue;
    };
    let rel_path = PathBuf::from(path);
    if repo_root.join(&rel_path).is_file() {
      files.insert(rel_path);
    }
  }

  Ok(files.into_iter().collect())
}

pub fn list_repo_head_files(repo_root: &Path) -> Result<Vec<PathBuf>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head = match repo.head() {
    Ok(head) => head,
    Err(_) => return Ok(Vec::new()),
  };
  let tree = match head.peel_to_tree() {
    Ok(tree) => tree,
    Err(_) => return Ok(Vec::new()),
  };

  let mut files = BTreeSet::new();
  collect_tree_files(&repo, &tree, Path::new(""), &mut files)?;
  Ok(files.into_iter().collect())
}

fn collect_tree_files(
  repo: &Repository,
  tree: &Tree<'_>,
  prefix: &Path,
  files: &mut BTreeSet<PathBuf>,
) -> Result<()> {
  for entry in tree.iter() {
    let Some(name) = entry.name() else {
      continue;
    };
    let path = if prefix.as_os_str().is_empty() {
      PathBuf::from(name)
    } else {
      prefix.join(name)
    };

    match entry.kind() {
      Some(ObjectType::Blob) => {
        files.insert(path);
      }
      Some(ObjectType::Tree) => {
        let subtree = repo.find_tree(entry.id()).context("find subtree")?;
        collect_tree_files(repo, &subtree, &path, files)?;
      }
      _ => {}
    }
  }

  Ok(())
}

pub fn stage_file(repo_root: &Path, rel_path: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut index = repo.index()?;
  let workdir_path = repo_root.join(rel_path);
  if workdir_path.exists() {
    index.add_path(rel_path)?;
  } else if let Err(err) = index.remove_path(rel_path)
    && err.code() != ErrorCode::NotFound
  {
    return Err(err.into());
  }
  index.write()?;
  Ok(())
}

pub fn stage_all(repo_root: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut index = repo.index()?;
  index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
  index.write()?;
  Ok(())
}

pub fn unstage_all(repo_root: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let pathspecs = [Path::new("*")];
  let target = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
  match target {
    Some(commit) => repo.reset_default(Some(commit.as_object()), pathspecs)?,
    None => repo.reset_default(None, pathspecs)?,
  }
  Ok(())
}

pub fn unstage_file(repo_root: &Path, rel_path: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let target = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
  match target {
    Some(commit) => repo.reset_default(Some(commit.as_object()), [rel_path])?,
    None => repo.reset_default(None, [rel_path])?,
  }
  Ok(())
}

pub fn restore_file(repo_root: &Path, rel_path: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut checkout = CheckoutBuilder::new();
  checkout.force().path(rel_path);
  if repo.head().is_ok() {
    repo.checkout_head(Some(&mut checkout))?;
  } else {
    repo.checkout_index(None, Some(&mut checkout))?;
  }
  Ok(())
}

pub fn restore_renamed_file(repo_root: &Path, old_path: &Path, new_path: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  if let Ok(head) = repo.head().and_then(|head| head.peel_to_commit()) {
    let head_object = head.into_object();
    let paths = [old_path, new_path];
    repo
      .reset_default(Some(&head_object), paths)
      .with_context(|| format!("reset index for rename {old_path:?} -> {new_path:?}"))?;
  }
  let new_target = repo_root.join(new_path);
  if new_target.exists() {
    fs::remove_file(&new_target)
      .with_context(|| format!("remove renamed file {:?}", new_target))?;
  }
  restore_file(repo_root, old_path)
}

pub fn delete_untracked_file(repo_root: &Path, rel_path: &Path) -> Result<()> {
  let target = repo_root.join(rel_path);
  let meta = fs::metadata(&target).with_context(|| format!("metadata {:?}", target))?;
  if meta.is_dir() {
    fs::remove_dir_all(&target).with_context(|| format!("remove dir {:?}", target))?;
  } else {
    fs::remove_file(&target).with_context(|| format!("remove file {:?}", target))?;
  }
  Ok(())
}

fn stage_from_status(status: Status) -> RepoStage {
  let index = status.is_index_new()
    || status.is_index_modified()
    || status.is_index_deleted()
    || status.is_index_renamed()
    || status.is_index_typechange();
  let worktree = status.is_wt_new()
    || status.is_wt_modified()
    || status.is_wt_deleted()
    || status.is_wt_renamed()
    || status.is_wt_typechange();

  match (index, worktree) {
    (true, true) => RepoStage::PartiallyStaged,
    (true, false) => RepoStage::Staged,
    (false, true) => RepoStage::Unstaged,
    (false, false) => RepoStage::Unstaged,
  }
}

fn kind_from_status(status: Status) -> RepoStatusKind {
  if status.is_conflicted() {
    return RepoStatusKind::Conflicted;
  }

  let index = status.is_index_new()
    || status.is_index_modified()
    || status.is_index_deleted()
    || status.is_index_renamed()
    || status.is_index_typechange();
  let worktree = status.is_wt_new()
    || status.is_wt_modified()
    || status.is_wt_deleted()
    || status.is_wt_renamed()
    || status.is_wt_typechange();

  if status.is_wt_new() && !index {
    return RepoStatusKind::Untracked;
  }

  if status.is_index_new() || status.is_wt_new() {
    return RepoStatusKind::Added;
  }

  if status.is_index_deleted() || status.is_wt_deleted() {
    return RepoStatusKind::Deleted;
  }

  if status.is_index_renamed() || status.is_wt_renamed() {
    return RepoStatusKind::Renamed;
  }

  if status.is_index_typechange() || status.is_wt_typechange() {
    return RepoStatusKind::TypeChange;
  }

  if status.is_index_modified() || status.is_wt_modified() || index || worktree {
    return RepoStatusKind::Modified;
  }

  RepoStatusKind::Modified
}

fn entry_paths(
  entry: &git2::StatusEntry<'_>,
  kind: RepoStatusKind,
) -> Option<(PathBuf, Option<PathBuf>)> {
  let delta = entry.index_to_workdir().or_else(|| entry.head_to_index());

  let (path, old_path) = if let Some(delta) = delta {
    let old_path = delta.old_file().path().map(Path::to_path_buf);
    let new_path = delta.new_file().path().map(Path::to_path_buf);
    let path = if kind == RepoStatusKind::Deleted {
      old_path.clone().or(new_path.clone())
    } else {
      new_path.clone().or(old_path.clone())
    }?;
    let old_path = if kind == RepoStatusKind::Renamed {
      old_path.filter(|old| old != &path)
    } else {
      None
    };
    (path, old_path)
  } else {
    (PathBuf::from(entry.path()?), None)
  };

  Some((path, old_path))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::{TempDir, commit_text_file as commit_file};
  use git2::{Repository, Status};

  fn init_repo(path: &Path) {
    Repository::init(path).expect("init git repository");
  }

  #[test]
  fn maps_untracked_worktree_status() {
    let status = Status::WT_NEW;
    assert_eq!(stage_from_status(status), RepoStage::Unstaged);
    assert_eq!(kind_from_status(status), RepoStatusKind::Untracked);
  }

  #[test]
  fn maps_partially_staged_modified_status() {
    let status = Status::INDEX_MODIFIED | Status::WT_MODIFIED;
    assert_eq!(stage_from_status(status), RepoStage::PartiallyStaged);
    assert_eq!(kind_from_status(status), RepoStatusKind::Modified);
  }

  #[test]
  fn maps_conflicted_status() {
    let status = Status::CONFLICTED;
    assert_eq!(kind_from_status(status), RepoStatusKind::Conflicted);
  }

  #[test]
  fn maps_renamed_status() {
    let status = Status::INDEX_RENAMED;
    assert_eq!(stage_from_status(status), RepoStage::Staged);
    assert_eq!(kind_from_status(status), RepoStatusKind::Renamed);
  }

  #[test]
  fn delete_untracked_file_removes_file() {
    let temp = TempDir::new("status-file");
    let rel_path = Path::new("note.txt");
    let absolute = temp.path.join(rel_path);
    std::fs::write(&absolute, "hello").expect("write file");

    delete_untracked_file(&temp.path, rel_path).expect("delete file");
    assert!(!absolute.exists());
  }

  #[test]
  fn delete_untracked_file_removes_directory() {
    let temp = TempDir::new("status-dir");
    let rel_path = Path::new("folder");
    let absolute = temp.path.join(rel_path);
    std::fs::create_dir_all(absolute.join("nested")).expect("create directory");

    delete_untracked_file(&temp.path, rel_path).expect("delete directory");
    assert!(!absolute.exists());
  }

  #[test]
  fn delete_untracked_file_returns_error_for_missing_path() {
    let temp = TempDir::new("status-missing");
    let err = delete_untracked_file(&temp.path, Path::new("missing.txt")).err();
    assert!(err.is_some());
  }

  #[test]
  fn stage_file_marks_modified_file_as_staged() {
    let temp = TempDir::new("status-stage-file");
    init_repo(&temp.path);
    let rel_path = Path::new("README.md");
    commit_file(&temp.path, rel_path, "v1\n", "initial");
    std::fs::write(temp.path.join(rel_path), "v2\n").expect("modify file");

    stage_file(&temp.path, rel_path).expect("stage file");

    let entries = list_repo_status(&temp.path).expect("list status");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].status, RepoStatusKind::Modified);
    assert_eq!(entries[0].stage, RepoStage::Staged);
  }

  #[test]
  fn unstage_file_moves_change_back_to_unstaged() {
    let temp = TempDir::new("status-unstage-file");
    init_repo(&temp.path);
    let rel_path = Path::new("README.md");
    commit_file(&temp.path, rel_path, "v1\n", "initial");
    std::fs::write(temp.path.join(rel_path), "v2\n").expect("modify file");
    stage_file(&temp.path, rel_path).expect("stage file");

    unstage_file(&temp.path, rel_path).expect("unstage file");

    let entries = list_repo_status(&temp.path).expect("list status");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].status, RepoStatusKind::Modified);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
  }

  #[test]
  fn restore_file_reverts_worktree_to_head() {
    let temp = TempDir::new("status-restore-file");
    init_repo(&temp.path);
    let rel_path = Path::new("README.md");
    commit_file(&temp.path, rel_path, "v1\n", "initial");
    std::fs::write(temp.path.join(rel_path), "v2\n").expect("modify file");

    restore_file(&temp.path, rel_path).expect("restore file");

    let contents = std::fs::read_to_string(temp.path.join(rel_path)).expect("read restored file");
    assert_eq!(contents, "v1\n");
    let entries = list_repo_status(&temp.path).expect("list status");
    assert!(entries.is_empty());
  }

  #[test]
  fn restore_renamed_file_restores_old_and_deletes_new() {
    let temp = TempDir::new("status-restore-renamed");
    init_repo(&temp.path);
    let old_path = Path::new("old.txt");
    let new_path = Path::new("new.txt");
    commit_file(&temp.path, old_path, "v1\n", "initial");
    std::fs::rename(temp.path.join(old_path), temp.path.join(new_path))
      .expect("rename file in worktree");

    restore_renamed_file(&temp.path, old_path, new_path).expect("restore renamed file");

    assert!(temp.path.join(old_path).exists());
    assert!(!temp.path.join(new_path).exists());
    let contents = std::fs::read_to_string(temp.path.join(old_path)).expect("read restored file");
    assert_eq!(contents, "v1\n");
    let entries = list_repo_status(&temp.path).expect("list status");
    assert!(entries.is_empty());
  }

  #[test]
  fn restore_renamed_file_when_new_path_is_missing_restores_old() {
    let temp = TempDir::new("status-restore-renamed-missing-new");
    init_repo(&temp.path);
    let old_path = Path::new("old.txt");
    let new_path = Path::new("new.txt");
    commit_file(&temp.path, old_path, "v1\n", "initial");
    std::fs::remove_file(temp.path.join(old_path)).expect("remove file in worktree");

    restore_renamed_file(&temp.path, old_path, new_path).expect("restore when new is missing");

    assert!(temp.path.join(old_path).exists());
    let contents = std::fs::read_to_string(temp.path.join(old_path)).expect("read restored file");
    assert_eq!(contents, "v1\n");
  }

  #[test]
  fn stage_all_marks_all_modified_files_as_staged() {
    let temp = TempDir::new("status-stage-all");
    init_repo(&temp.path);
    let first = Path::new("a.txt");
    let second = Path::new("b.txt");
    commit_file(&temp.path, first, "a1\n", "initial");
    commit_file(&temp.path, second, "b1\n", "second");
    std::fs::write(temp.path.join(first), "a2\n").expect("modify first file");
    std::fs::write(temp.path.join(second), "b2\n").expect("modify second file");

    stage_all(&temp.path).expect("stage all");

    let entries = list_repo_status(&temp.path).expect("list status");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.stage == RepoStage::Staged));
    assert!(
      entries
        .iter()
        .all(|entry| entry.status == RepoStatusKind::Modified)
    );
  }

  #[test]
  fn unstage_all_moves_staged_changes_back_to_unstaged() {
    let temp = TempDir::new("status-unstage-all");
    init_repo(&temp.path);
    let first = Path::new("a.txt");
    let second = Path::new("b.txt");
    commit_file(&temp.path, first, "a1\n", "initial");
    commit_file(&temp.path, second, "b1\n", "second");
    std::fs::write(temp.path.join(first), "a2\n").expect("modify first file");
    std::fs::write(temp.path.join(second), "b2\n").expect("modify second file");
    stage_all(&temp.path).expect("stage all");

    unstage_all(&temp.path).expect("unstage all");

    let entries = list_repo_status(&temp.path).expect("list status");
    assert_eq!(entries.len(), 2);
    assert!(
      entries
        .iter()
        .all(|entry| entry.stage == RepoStage::Unstaged)
    );
    assert!(
      entries
        .iter()
        .all(|entry| entry.status == RepoStatusKind::Modified)
    );
  }

  #[test]
  fn stage_file_marks_deleted_file_as_staged_deletion() {
    let temp = TempDir::new("status-stage-delete");
    init_repo(&temp.path);
    let rel_path = Path::new("delete.txt");
    commit_file(&temp.path, rel_path, "to delete\n", "initial");
    std::fs::remove_file(temp.path.join(rel_path)).expect("remove file from worktree");

    stage_file(&temp.path, rel_path).expect("stage deleted file");

    let entries = list_repo_status(&temp.path).expect("list status");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].status, RepoStatusKind::Deleted);
    assert_eq!(entries[0].stage, RepoStage::Staged);
  }

  #[test]
  fn list_repo_worktree_files_includes_tracked_and_untracked_but_skips_ignored() {
    let temp = TempDir::new("status-worktree-files");
    init_repo(&temp.path);
    std::fs::write(temp.path.join(".gitignore"), "ignored/\n").expect("write gitignore");
    let tracked = Path::new("tracked.txt");
    commit_file(&temp.path, tracked, "tracked\n", "initial");

    std::fs::create_dir_all(temp.path.join("src")).expect("create untracked dir");
    std::fs::write(temp.path.join("src/lib.rs"), "pub fn lib() {}\n").expect("write untracked");
    std::fs::create_dir_all(temp.path.join("ignored")).expect("create ignored dir");
    std::fs::write(temp.path.join("ignored/secret.txt"), "ignored\n").expect("write ignored");

    let files = list_repo_worktree_files(temp.path.as_path()).expect("list repo worktree files");

    assert_eq!(
      files,
      vec![
        PathBuf::from(".gitignore"),
        PathBuf::from("src/lib.rs"),
        PathBuf::from("tracked.txt"),
      ]
    );
  }

  #[test]
  fn list_repo_head_files_uses_head_tree_and_excludes_local_untracked_files() {
    let temp = TempDir::new("status-head-files");
    init_repo(&temp.path);
    let tracked = Path::new("tracked.txt");
    let nested = Path::new("src/lib.rs");
    commit_file(&temp.path, tracked, "tracked\n", "initial tracked");
    commit_file(&temp.path, nested, "pub fn clean() {}\n", "initial nested");

    std::fs::write(temp.path.join(tracked), "local change\n").expect("write local change");
    std::fs::create_dir_all(temp.path.join("scratch")).expect("create untracked dir");
    std::fs::write(temp.path.join("scratch/tmp.rs"), "untracked\n").expect("write untracked");

    let files = list_repo_head_files(temp.path.as_path()).expect("list repo head files");

    assert_eq!(
      files,
      vec![PathBuf::from("src/lib.rs"), PathBuf::from("tracked.txt"),]
    );
  }
}
