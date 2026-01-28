use std::fs;
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use git2::build::CheckoutBuilder;
use git2::{
  ApplyLocation, ApplyOptions, BranchType, Cred, DiffHunk, DiffOptions, Direction, IndexAddOption,
  Oid, Patch, ProxyOptions, PushOptions, RemoteCallbacks, Repository as GitRepository, ResetType,
  Signature, Status, StatusOptions, Tree,
};

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
  pub head_content: Option<String>,
  pub index_content: Option<String>,
  pub workdir_content: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Repository {
  pub root: PathBuf,
  pub changes: Vec<RepositoryFile>,
  pub staged: Vec<RepositoryFile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PushStatus {
  pub ahead: usize,
  pub behind: usize,
}

#[derive(Clone, Debug)]
pub struct BranchStatus {
  pub name: String,
  pub ahead: usize,
  pub behind: usize,
}

#[derive(Clone, Debug)]
pub struct HunkRange {
  pub base: Option<Range<usize>>,
  pub current: Option<Range<usize>>,
}

#[derive(Clone, Debug)]
pub struct DiffHunkInfo {
  pub old_start: usize,
  pub old_lines: usize,
  pub new_start: usize,
  pub new_lines: usize,
  pub old_changed: Vec<Range<usize>>,
  pub new_changed: Vec<Range<usize>>,
}

pub struct BufferDiff {
  pub patch: String,
  pub hunks: Vec<DiffHunkInfo>,
}

struct UpstreamInfo {
  head_ref: String,
  upstream_ref: String,
  remote_name: String,
  head_oid: Oid,
  upstream_oid: Oid,
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

    let head_content = match (head_tree.as_ref(), old_path.as_ref()) {
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
        head_content: head_content.clone(),
        index_content: index_content.clone(),
        workdir_content: workdir_content.clone(),
      });
      continue;
    }

    if let Some(kind) = status_to_kind_for_scope(status, StatusScope::Workdir) {
      changes.push(RepositoryFile {
        path: display_path.to_path_buf(),
        status: kind,
        head_content: head_content.clone(),
        index_content: index_content.clone(),
        workdir_content: workdir_content.clone(),
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
        head_content: head_content.clone(),
        index_content: staged_content,
        workdir_content: workdir_content.clone(),
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

fn push_line_range(ranges: &mut Vec<Range<usize>>, line: usize) {
  if let Some(last) = ranges.last_mut() {
    if last.end == line {
      last.end += 1;
      return;
    }
  }
  ranges.push(line..(line + 1));
}

pub fn diff_buffers_for_path(
  repo_root: &Path,
  path: &Path,
  old_text: &str,
  new_text: &str,
  context_lines: u32,
) -> Result<BufferDiff, git2::Error> {
  let rel_path = repo_relative_path(repo_root, path);
  diff_buffers_with_path(old_text, new_text, &rel_path, context_lines)
}

fn diff_buffers_with_path(
  old_text: &str,
  new_text: &str,
  rel_path: &Path,
  context_lines: u32,
) -> Result<BufferDiff, git2::Error> {
  let mut diff_opts = DiffOptions::new();
  diff_opts.context_lines(context_lines);
  diff_opts.interhunk_lines(0);
  diff_opts.patience(true);
  let mut patch = Patch::from_buffers(
    old_text.as_bytes(),
    Some(rel_path),
    new_text.as_bytes(),
    Some(rel_path),
    Some(&mut diff_opts),
  )?;

  let mut hunks = Vec::new();
  let hunk_count = patch.num_hunks();
  for hunk_idx in 0..hunk_count {
    let (hunk, line_count) = patch.hunk(hunk_idx)?;
    let old_start = hunk.old_start().saturating_sub(1) as usize;
    let new_start = hunk.new_start().saturating_sub(1) as usize;
    let mut info = DiffHunkInfo {
      old_start,
      old_lines: hunk.old_lines() as usize,
      new_start,
      new_lines: hunk.new_lines() as usize,
      old_changed: Vec::new(),
      new_changed: Vec::new(),
    };

    for line_idx in 0..line_count {
      let line = patch.line_in_hunk(hunk_idx, line_idx)?;
      match line.origin() {
        '+' | '>' => {
          if let Some(line_no) = line.new_lineno() {
            let line_idx = line_no.saturating_sub(1) as usize;
            push_line_range(&mut info.new_changed, line_idx);
          }
        }
        '-' | '<' => {
          if let Some(line_no) = line.old_lineno() {
            let line_idx = line_no.saturating_sub(1) as usize;
            push_line_range(&mut info.old_changed, line_idx);
          }
        }
        _ => {}
      }
    }

    hunks.push(info);
  }

  let patch_buf = patch.to_buf()?;
  let patch_text = String::from_utf8_lossy(patch_buf.as_ref()).to_string();
  Ok(BufferDiff {
    patch: patch_text,
    hunks,
  })
}

pub fn apply_patch_to_index(
  repo_root: &Path,
  path: &Path,
  patch: &str,
  targets: &[HunkRange],
) -> Result<(), git2::Error> {
  apply_patch(repo_root, path, patch, targets, ApplyLocation::Index)
}

pub fn apply_patch_to_workdir(
  repo_root: &Path,
  path: &Path,
  patch: &str,
  targets: &[HunkRange],
) -> Result<(), git2::Error> {
  apply_patch(repo_root, path, patch, targets, ApplyLocation::WorkDir)
}

pub fn write_index_content(
  repo_root: &Path,
  path: &Path,
  content: &str,
) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let rel_path = repo_relative_path(&repo_root_path(&repo)?, path);
  let mut index = repo.index()?;
  let entry = index
    .get_path(&rel_path, 0)
    .ok_or_else(|| git2::Error::from_str("Index entry not found"))?;
  let mut entry = git2::IndexEntry {
    ctime: entry.ctime,
    mtime: entry.mtime,
    dev: entry.dev,
    ino: entry.ino,
    mode: entry.mode,
    uid: entry.uid,
    gid: entry.gid,
    file_size: entry.file_size,
    id: entry.id,
    flags: entry.flags,
    flags_extended: entry.flags_extended,
    path: entry.path.clone(),
  };
  entry.path = rel_path.as_os_str().as_bytes().to_vec();
  index.add_frombuffer(&entry, content.as_bytes())?;
  index.write()?;
  Ok(())
}

fn apply_patch(
  repo_root: &Path,
  path: &Path,
  patch: &str,
  targets: &[HunkRange],
  location: ApplyLocation,
) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let rel_path = repo_relative_path(&repo_root_path(&repo)?, path);
  let diff = git2::Diff::from_buffer(patch.as_bytes())?;
  let targets = targets.to_vec();
  let rel_path_for_delta = rel_path.clone();
  let mut apply_opts = ApplyOptions::new();
  apply_opts.delta_callback(move |delta| {
    delta
      .and_then(|delta| delta.new_file().path().or(delta.old_file().path()))
      .map(|delta_path| delta_path == rel_path_for_delta.as_path())
      .unwrap_or(false)
  });
  apply_opts.hunk_callback(move |hunk| {
    let Some(hunk) = hunk else {
      return false;
    };
    let old_start = hunk.old_start().saturating_sub(1) as usize;
    let new_start = hunk.new_start().saturating_sub(1) as usize;
    let old_range = old_start..(old_start + hunk.old_lines() as usize);
    let new_range = new_start..(new_start + hunk.new_lines() as usize);
    let base = (hunk.old_lines() > 0).then(|| old_range.clone());
    let current = (hunk.new_lines() > 0).then(|| new_range.clone());
    targets
      .iter()
      .any(|target| target.base == base && target.current == current)
  });

  repo.apply(&diff, location, Some(&mut apply_opts))?;
  Ok(())
}

fn collect_diff_hunks(diff: &git2::Diff<'_>) -> Result<Vec<DiffHunkInfo>, git2::Error> {
  let hunks = std::cell::RefCell::new(Vec::new());
  diff.foreach(
    &mut |_delta, _progress| true,
    None,
    Some(&mut |_delta, hunk| {
      let old_start = hunk.old_start().saturating_sub(1) as usize;
      let new_start = hunk.new_start().saturating_sub(1) as usize;
      hunks.borrow_mut().push(DiffHunkInfo {
        old_start,
        old_lines: hunk.old_lines() as usize,
        new_start,
        new_lines: hunk.new_lines() as usize,
        old_changed: Vec::new(),
        new_changed: Vec::new(),
      });
      true
    }),
    Some(&mut |_delta, _hunk, line| {
      let mut hunks = hunks.borrow_mut();
      let Some(current) = hunks.last_mut() else {
        return true;
      };
      match line.origin() {
        '+' | '>' => {
          if let Some(line_no) = line.new_lineno() {
            let line_idx = line_no.saturating_sub(1) as usize;
            push_line_range(&mut current.new_changed, line_idx);
          }
        }
        '-' | '<' => {
          if let Some(line_no) = line.old_lineno() {
            let line_idx = line_no.saturating_sub(1) as usize;
            push_line_range(&mut current.old_changed, line_idx);
          }
        }
        _ => {}
      }
      true
    }),
  )?;
  Ok(hunks.into_inner())
}

pub fn diff_head_to_index_hunks(
  repo_root: &Path,
  path: &Path,
) -> Result<Vec<DiffHunkInfo>, git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let rel_path = repo_relative_path(&repo_root_path(&repo)?, path);
  let mut diff_opts = DiffOptions::new();
  diff_opts.pathspec(&rel_path);
  diff_opts.include_untracked(true);
  diff_opts.context_lines(0);
  diff_opts.interhunk_lines(0);
  diff_opts.patience(true);
  let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
  let diff = repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut diff_opts))?;
  collect_diff_hunks(&diff)
}

pub fn diff_index_to_workdir_hunks(
  repo_root: &Path,
  path: &Path,
) -> Result<Vec<DiffHunkInfo>, git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let rel_path = repo_relative_path(&repo_root_path(&repo)?, path);
  let mut diff_opts = DiffOptions::new();
  diff_opts.pathspec(&rel_path);
  diff_opts.include_untracked(true);
  diff_opts.context_lines(0);
  diff_opts.interhunk_lines(0);
  diff_opts.patience(true);
  let diff = repo.diff_index_to_workdir(None, Some(&mut diff_opts))?;
  collect_diff_hunks(&diff)
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
  let target_commit = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
  match target_commit {
    Some(commit) => repo.reset_default(Some(commit.as_object()), &[rel_path.as_path()])?,
    None => repo.reset_default(None, &[rel_path.as_path()])?,
  }
  Ok(())
}

fn hunk_matches(hunk: &DiffHunk<'_>, target: &HunkRange, reverse: bool) -> bool {
  let old_start = hunk.old_start().saturating_sub(1) as usize;
  let old_end = old_start + hunk.old_lines() as usize;
  let new_start = hunk.new_start().saturating_sub(1) as usize;
  let new_end = new_start + hunk.new_lines() as usize;
  let old_range = old_start..old_end;
  let new_range = new_start..new_end;

  let (base_range, current_range) = if reverse {
    (&target.current, &target.base)
  } else {
    (&target.base, &target.current)
  };

  let base_match = base_range
    .as_ref()
    .map(|base| base.start == old_range.start && base.end == old_range.end);
  let current_match = current_range
    .as_ref()
    .map(|current| current.start == new_range.start && current.end == new_range.end);

  match (base_match, current_match) {
    (Some(base), Some(current)) => base && current,
    (Some(base), None) => base,
    (None, Some(current)) => current,
    (None, None) => false,
  }
}

fn apply_hunk_with_diff(
  repo: &GitRepository,
  diff: &git2::Diff<'_>,
  rel_path: &Path,
  target: &HunkRange,
  location: ApplyLocation,
  reverse: bool,
) -> Result<(), git2::Error> {
  let rel_path_for_delta = rel_path.to_path_buf();
  let target_for_hunk = target.clone();
  let mut apply_opts = ApplyOptions::new();
  apply_opts.delta_callback(move |delta| {
    delta
      .and_then(|delta| delta.new_file().path().or(delta.old_file().path()))
      .map(|delta_path| delta_path == rel_path_for_delta.as_path())
      .unwrap_or(false)
  });
  apply_opts.hunk_callback(move |hunk| {
    let Some(hunk) = hunk else {
      return false;
    };
    hunk_matches(&hunk, &target_for_hunk, reverse)
  });

  repo.apply(diff, location, Some(&mut apply_opts))?;
  Ok(())
}

fn apply_hunk(
  repo_root: &Path,
  path: &Path,
  target: &HunkRange,
  location: ApplyLocation,
  reverse: bool,
) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let rel_path = repo_relative_path(&repo_root_path(&repo)?, path);
  let mut diff_opts = DiffOptions::new();
  diff_opts.pathspec(&rel_path);
  diff_opts.include_untracked(true);
  diff_opts.context_lines(0);
  diff_opts.interhunk_lines(0);
  if reverse {
    diff_opts.reverse(true);
  }
  let diff = repo.diff_index_to_workdir(None, Some(&mut diff_opts))?;

  apply_hunk_with_diff(&repo, &diff, &rel_path, target, location, reverse)
}

pub fn stage_hunk(repo_root: &Path, path: &Path, target: &HunkRange) -> Result<(), git2::Error> {
  apply_hunk(repo_root, path, target, ApplyLocation::Index, false)
}

pub fn restore_hunk(repo_root: &Path, path: &Path, target: &HunkRange) -> Result<(), git2::Error> {
  apply_hunk(repo_root, path, target, ApplyLocation::WorkDir, true)
}

pub fn unstage_hunk(repo_root: &Path, path: &Path, target: &HunkRange) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let rel_path = repo_relative_path(&repo_root_path(&repo)?, path);

  let mut diff_opts = DiffOptions::new();
  diff_opts.pathspec(&rel_path);
  diff_opts.include_untracked(true);
  diff_opts.context_lines(0);
  diff_opts.interhunk_lines(0);
  diff_opts.reverse(true);
  let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
  let diff = repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut diff_opts))?;

  let result = apply_hunk_with_diff(&repo, &diff, &rel_path, target, ApplyLocation::Index, true);

  result
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

pub fn restore_change(
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

pub fn commit_repository(repo_root: &Path, message: &str, amend: bool) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let message = message.trim();
  let has_message = !message.is_empty();
  if !amend && !has_message {
    return Ok(());
  }

  let mut index = repo.index()?;
  let tree_id = index.write_tree()?;
  index.write()?;
  let tree = repo.find_tree(tree_id)?;

  let signature = repo
    .signature()
    .or_else(|_| Signature::now("Reviu", "reviu@example.com"))?;

  let head_commit = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
  if amend {
    if let Some(commit) = head_commit.as_ref() {
      let message = if has_message { Some(message) } else { None };
      commit.amend(
        Some("HEAD"),
        None,
        Some(&signature),
        None,
        message,
        Some(&tree),
      )?;
    } else if has_message {
      repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])?;
    }
  } else if let Some(commit) = head_commit.as_ref() {
    repo.commit(
      Some("HEAD"),
      &signature,
      &signature,
      message,
      &tree,
      &[commit],
    )?;
  } else {
    repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])?;
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn diff_for(old: &str, new: &str) -> BufferDiff {
    let repo_root = Path::new("/repo");
    let path = Path::new("/repo/file.txt");
    diff_buffers_for_path(repo_root, path, old, new, 0).expect("diff should build")
  }

  #[test]
  fn diff_buffers_add_only() {
    let old = "a\nb\n";
    let new = "a\nb\n\n\n";
    let diff = diff_for(old, new);
    assert_eq!(diff.hunks.len(), 1);
    let hunk = &diff.hunks[0];
    assert_eq!(hunk.old_lines, 0);
    assert!(hunk.new_lines >= 1);
    assert!(!hunk.new_changed.is_empty());
  }

  #[test]
  fn diff_buffers_delete_only() {
    let old = "a\nb\nc\n";
    let new = "a\nc\n";
    let diff = diff_for(old, new);
    assert_eq!(diff.hunks.len(), 1);
    let hunk = &diff.hunks[0];
    assert!(hunk.old_lines >= 1);
    assert_eq!(hunk.new_lines, 0);
    assert!(!hunk.old_changed.is_empty());
  }

  #[test]
  fn diff_buffers_mixed_change() {
    let old = "a\nb\nc\n";
    let new = "a\nx\ny\nc\n";
    let diff = diff_for(old, new);
    assert_eq!(diff.hunks.len(), 1);
    let hunk = &diff.hunks[0];
    assert!(hunk.old_lines >= 1);
    assert!(hunk.new_lines >= 1);
    assert!(!hunk.old_changed.is_empty());
    assert!(!hunk.new_changed.is_empty());
  }
}

pub fn has_head_commit(repo_root: &Path) -> bool {
  let Ok(repo) = GitRepository::discover(repo_root) else {
    return false;
  };
  let Ok(head) = repo.head() else {
    return false;
  };
  head.peel_to_commit().is_ok()
}

pub fn can_undo_last_commit(repo_root: &Path) -> bool {
  let Ok(repo) = GitRepository::discover(repo_root) else {
    return false;
  };
  let Ok(head) = repo.head() else {
    return false;
  };
  if !head.is_branch() {
    return false;
  }
  let Some(branch_name) = head.shorthand() else {
    return false;
  };
  let Ok(branch) = repo.find_branch(branch_name, BranchType::Local) else {
    return false;
  };
  let Ok(upstream) = branch.upstream() else {
    return false;
  };
  let Ok(head_commit) = head.peel_to_commit() else {
    return false;
  };
  if head_commit.parent_count() == 0 {
    return false;
  }
  let Some(upstream_oid) = upstream.get().target() else {
    return false;
  };
  let Ok((ahead, _behind)) = repo.graph_ahead_behind(head_commit.id(), upstream_oid) else {
    return false;
  };
  ahead > 0
}

pub fn undo_last_commit(repo_root: &Path) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let head = repo.head()?;
  let head_commit = head.peel_to_commit()?;
  let parent = head_commit
    .parent(0)
    .map_err(|_| git2::Error::from_str("No parent commit to reset to"))?;
  repo.reset(parent.as_object(), ResetType::Soft, None)?;
  Ok(())
}

fn repo_root_path(repo: &GitRepository) -> Result<PathBuf, git2::Error> {
  repo
    .workdir()
    .map(|path| path.to_path_buf())
    .ok_or_else(|| git2::Error::from_str("Repository has no working directory"))
}

fn upstream_info(repo: &GitRepository) -> Result<UpstreamInfo, git2::Error> {
  let head = repo.head()?;
  if !head.is_branch() {
    return Err(git2::Error::from_str("HEAD is not a branch"));
  }
  let head_ref = head
    .name()
    .ok_or_else(|| git2::Error::from_str("HEAD has no reference name"))?
    .to_string();
  let head_oid = head.peel_to_commit()?.id();

  let upstream_ref = repo
    .branch_upstream_name(&head_ref)?
    .as_str()
    .ok_or_else(|| git2::Error::from_str("Upstream name is not valid UTF-8"))?
    .to_string();
  let upstream_oid = repo.find_reference(&upstream_ref)?.peel_to_commit()?.id();

  let remote_name = repo
    .branch_upstream_remote(&head_ref)?
    .as_str()
    .ok_or_else(|| git2::Error::from_str("Remote name is not valid UTF-8"))?
    .to_string();

  Ok(UpstreamInfo {
    head_ref,
    upstream_ref,
    remote_name,
    head_oid,
    upstream_oid,
  })
}

fn push_remote_ref(info: &UpstreamInfo) -> String {
  let prefix = format!("refs/remotes/{}/", info.remote_name);
  if let Some(branch) = info.upstream_ref.strip_prefix(&prefix) {
    format!("refs/heads/{}", branch)
  } else {
    info.head_ref.clone()
  }
}

fn build_remote_callbacks(repo: &GitRepository) -> RemoteCallbacks<'static> {
  let config = repo.config().ok();
  let mut callbacks = RemoteCallbacks::new();
  callbacks.credentials(move |url, username_from_url, allowed| {
    if allowed.is_ssh_key() || allowed.is_ssh_interactive() {
      let username = username_from_url.unwrap_or("git");
      return Cred::ssh_key_from_agent(username);
    }

    if allowed.is_user_pass_plaintext() {
      if let Some(config) = config.as_ref() {
        if let Ok(cred) = Cred::credential_helper(config, url, username_from_url) {
          return Ok(cred);
        }
      }
    }

    if allowed.is_default() {
      return Cred::default();
    }

    if allowed.is_username() {
      if let Some(username) = username_from_url {
        return Cred::username(username);
      }
    }

    Err(git2::Error::from_str("No supported authentication methods"))
  });
  callbacks
}

fn ensure_force_with_lease(repo: &GitRepository, info: &UpstreamInfo) -> Result<(), git2::Error> {
  let remote_ref = push_remote_ref(info);
  let mut remote = repo.find_remote(&info.remote_name)?;
  let callbacks = build_remote_callbacks(repo);
  let proxy = ProxyOptions::new();
  let connection = remote.connect_auth(Direction::Push, Some(callbacks), Some(proxy))?;
  let remote_oid = match connection.list() {
    Ok(remote_heads) => {
      let remote_head = remote_heads
        .iter()
        .find(|head| head.name() == remote_ref)
        .ok_or_else(|| git2::Error::from_str("Remote branch not found"))?;
      remote_head.oid()
    }
    Err(err) => return Err(err),
  };
  if remote_oid != info.upstream_oid {
    return Err(git2::Error::from_str(
      "Remote branch has moved; fetch before force pushing",
    ));
  }

  Ok(())
}

pub fn push_status(repo_root: &Path) -> Result<PushStatus, git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let info = upstream_info(&repo)?;
  let (ahead, behind) = repo.graph_ahead_behind(info.head_oid, info.upstream_oid)?;
  Ok(PushStatus { ahead, behind })
}

pub fn branch_status(repo_root: &Path) -> Result<BranchStatus, git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let head = match repo.head() {
    Ok(head) => head,
    Err(_) => {
      return Ok(BranchStatus {
        name: "No branch".to_string(),
        ahead: 0,
        behind: 0,
      });
    }
  };

  let name = match (head.is_branch(), head.shorthand()) {
    (true, Some(name)) => name.to_string(),
    _ => "Detached HEAD".to_string(),
  };

  let (ahead, behind) = match upstream_info(&repo) {
    Ok(info) => repo.graph_ahead_behind(info.head_oid, info.upstream_oid)?,
    Err(_) => (0, 0),
  };

  Ok(BranchStatus {
    name,
    ahead,
    behind,
  })
}

pub fn push_repository(repo_root: &Path, force: bool) -> Result<(), git2::Error> {
  let repo = GitRepository::discover(repo_root)?;
  let info = upstream_info(&repo)?;
  if force {
    ensure_force_with_lease(&repo, &info)?;
  }
  let remote_ref = push_remote_ref(&info);
  let refspec = if force {
    format!("+{}:{}", info.head_ref, remote_ref)
  } else {
    format!("{}:{}", info.head_ref, remote_ref)
  };

  let mut remote = repo.find_remote(&info.remote_name)?;
  let mut options = PushOptions::new();
  let callbacks = build_remote_callbacks(&repo);
  let proxy = ProxyOptions::new();
  options.remote_callbacks(callbacks);
  options.proxy_options(proxy);
  remote.push(&[&refspec], Some(&mut options))?;
  Ok(())
}
