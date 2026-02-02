use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::{IndexAddOption, Repository, Status, StatusOptions};
use git2::build::CheckoutBuilder;

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

    let Some(path) = entry.path() else {
      continue;
    };

    let stage = stage_from_status(status);
    let kind = kind_from_status(status);

    entries.push(RepoStatusEntry {
      path: PathBuf::from(path),
      status: kind,
      stage,
    });
  }

  entries.sort_by(|a, b| a.path.cmp(&b.path));
  Ok(entries)
}

pub fn stage_file(repo_root: &Path, rel_path: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut index = repo.index()?;
  index.add_path(rel_path)?;
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

pub fn unstage_file(repo_root: &Path, rel_path: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let target = repo
    .head()
    .ok()
    .and_then(|head| head.peel_to_commit().ok());
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
