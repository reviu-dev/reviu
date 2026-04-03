use std::{
  cell::RefCell,
  fs,
  path::{Path, PathBuf},
  sync::Arc,
};

use anyhow::{Context, Result, bail};
use git2::{ApplyLocation, Diff, DiffLineType, DiffOptions, Patch, Repository};

use crate::GitFileBases;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
  Uncommitted,
  Unstaged,
  Staged,
}

#[derive(Clone, Debug)]
pub struct DiffSet {
  pub uncommitted: FileDiff,
  pub unstaged: FileDiff,
  pub staged: FileDiff,
}

#[derive(Clone, Debug)]
pub struct FileDiff {
  pub kind: DiffKind,
  pub hunks: Vec<DiffHunk>,
}

impl FileDiff {
  pub fn empty(kind: DiffKind) -> Self {
    Self {
      kind,
      hunks: Vec::new(),
    }
  }

  pub fn clone_with_kind(&self, kind: DiffKind) -> Self {
    Self {
      kind,
      hunks: self.hunks.clone(),
    }
  }
}

#[derive(Clone, Debug)]
pub struct DiffHunk {
  pub id: String,
  pub old_start: usize,
  pub old_lines: usize,
  pub new_start: usize,
  pub new_lines: usize,
  pub lines: Vec<DiffLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
  Context,
  Add,
  Remove,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
  pub kind: DiffLineKind,
  pub content: Arc<str>,
  pub no_newline: bool,
}

#[derive(Clone, Debug)]
pub struct RepoFile {
  pub repo_root: PathBuf,
  pub file_path: PathBuf,
}

impl RepoFile {
  pub fn new(repo_root: impl Into<PathBuf>, file_path: impl Into<PathBuf>) -> Result<Self> {
    let repo_root = repo_root.into();
    let file_path = file_path.into();

    let repo_root = repo_root
      .canonicalize()
      .with_context(|| format!("canonicalize repo root {:?}", repo_root))?;
    let file_path = file_path
      .canonicalize()
      .with_context(|| format!("canonicalize file path {:?}", file_path))?;

    if !file_path.starts_with(&repo_root) {
      bail!(
        "file path {:?} is not inside repo root {:?}",
        file_path,
        repo_root
      );
    }

    Ok(Self {
      repo_root,
      file_path,
    })
  }

  pub fn relative_path(&self) -> Result<PathBuf> {
    self
      .file_path
      .strip_prefix(&self.repo_root)
      .map(Path::to_path_buf)
      .with_context(|| {
        format!(
          "file path {:?} is not inside repo root {:?}",
          self.file_path, self.repo_root
        )
      })
  }
}

pub fn compute_file_diffs(repo_file: &RepoFile) -> Result<DiffSet> {
  let repo = Repository::open(&repo_file.repo_root)
    .with_context(|| format!("open repo at {:?}", repo_file.repo_root))?;
  let rel_path = repo_file.relative_path()?;

  let uncommitted = compute_diff(&repo, &rel_path, DiffKind::Uncommitted)?;
  let unstaged = compute_diff(&repo, &rel_path, DiffKind::Unstaged)?;
  let staged = compute_diff(&repo, &rel_path, DiffKind::Staged)?;

  Ok(DiffSet {
    uncommitted,
    unstaged,
    staged,
  })
}

pub fn compute_buffer_diffs(
  bases: &GitFileBases,
  buffer_text: &str,
  rel_path: &Path,
) -> Result<DiffSet> {
  let head = bases.head.as_deref();
  let index = bases.index.as_deref();
  let uncommitted = compute_buffer_diff(DiffKind::Uncommitted, head, buffer_text, rel_path)?;
  let (unstaged, staged) = if head == index {
    (
      uncommitted.clone_with_kind(DiffKind::Unstaged),
      FileDiff::empty(DiffKind::Staged),
    )
  } else {
    (
      compute_buffer_diff(DiffKind::Unstaged, index, buffer_text, rel_path)?,
      compute_buffer_diff(DiffKind::Staged, head, index.unwrap_or(""), rel_path)?,
    )
  };

  Ok(DiffSet {
    uncommitted,
    unstaged,
    staged,
  })
}

pub fn apply_hunk(
  repo_file: &RepoFile,
  hunk: &DiffHunk,
  reverse: bool,
  location: ApplyLocation,
) -> Result<()> {
  let repo = Repository::open(&repo_file.repo_root)
    .with_context(|| format!("open repo at {:?}", repo_file.repo_root))?;
  let rel_path = repo_file.relative_path()?;
  let patch = build_hunk_patch(rel_path, hunk, reverse);
  let diff = Diff::from_buffer(patch.as_bytes())?;
  repo.apply(&diff, location, None)?;
  Ok(())
}

pub fn apply_hunk_to_text(base_text: &str, hunk: &DiffHunk, reverse: bool) -> Result<String> {
  let (base_lines, mut trailing_newline) = split_lines(base_text);
  let mut output = Vec::new();

  let base_start = if reverse {
    hunk.new_start
  } else {
    hunk.old_start
  };
  let mut old_idx = if base_start == 0 {
    0
  } else {
    base_start.saturating_sub(1)
  };
  if old_idx > base_lines.len() {
    bail!(
      "hunk start {} beyond base length {}",
      base_start,
      base_lines.len()
    );
  }

  output.extend_from_slice(&base_lines[..old_idx]);

  for line in &hunk.lines {
    let mut kind = line.kind;
    if reverse {
      kind = match kind {
        DiffLineKind::Add => DiffLineKind::Remove,
        DiffLineKind::Remove => DiffLineKind::Add,
        DiffLineKind::Context => DiffLineKind::Context,
      };
    }

    if line.no_newline {
      match kind {
        DiffLineKind::Add => trailing_newline = false,
        DiffLineKind::Remove => trailing_newline = true,
        DiffLineKind::Context => {}
      }
    }

    match kind {
      DiffLineKind::Context => {
        let Some(existing) = base_lines.get(old_idx) else {
          bail!("context line out of range at {}", old_idx + 1);
        };
        if existing.as_str() != line.content.as_ref() {
          bail!(
            "context mismatch at {}: expected {:?}, found {:?}",
            old_idx + 1,
            line.content,
            existing
          );
        }
        output.push(existing.clone());
        old_idx += 1;
      }
      DiffLineKind::Remove => {
        let Some(existing) = base_lines.get(old_idx) else {
          bail!("remove line out of range at {}", old_idx + 1);
        };
        if existing.as_str() != line.content.as_ref() {
          bail!(
            "remove mismatch at {}: expected {:?}, found {:?}",
            old_idx + 1,
            line.content,
            existing
          );
        }
        old_idx += 1;
      }
      DiffLineKind::Add => {
        output.push(line.content.to_string());
      }
    }
  }

  output.extend_from_slice(&base_lines[old_idx..]);
  let mut result = output.join("\n");
  if trailing_newline {
    result.push('\n');
  }
  Ok(result)
}

pub fn write_index_content(repo_file: &RepoFile, content: &str) -> Result<()> {
  let repo = Repository::open(&repo_file.repo_root)
    .with_context(|| format!("open repo at {:?}", repo_file.repo_root))?;
  let mut index = repo.index()?;
  let rel_path = repo_file.relative_path()?;
  let mut entry = git2::IndexEntry {
    ctime: git2::IndexTime::new(0, 0),
    mtime: git2::IndexTime::new(0, 0),
    dev: 0,
    ino: 0,
    mode: 0o100644,
    uid: 0,
    gid: 0,
    file_size: content.len() as u32,
    id: git2::Oid::from_bytes(&[0; 20]).unwrap(),
    flags: 0,
    flags_extended: 0,
    path: rel_path.to_string_lossy().replace('\\', "/").into_bytes(),
  };
  if let Some(existing) = index.get_path(&rel_path, 0) {
    entry.mode = existing.mode;
    entry.uid = existing.uid;
    entry.gid = existing.gid;
  }
  index.add_frombuffer(&entry, content.as_bytes())?;
  index.write()?;
  Ok(())
}

fn compute_diff(repo: &Repository, rel_path: &Path, kind: DiffKind) -> Result<FileDiff> {
  let trailing_change = trailing_newline_change_for_diff(kind, repo, rel_path)?;
  let mut opts = DiffOptions::new();
  opts
    .pathspec(rel_path)
    .context_lines(3)
    .patience(true)
    .indent_heuristic(true)
    .include_untracked(true)
    .show_untracked_content(true);

  let diff = match kind {
    DiffKind::Uncommitted => {
      let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
      repo.diff_tree_to_workdir(head_tree.as_ref(), Some(&mut opts))?
    }
    DiffKind::Unstaged => {
      let index = repo.index()?;
      repo.diff_index_to_workdir(Some(&index), Some(&mut opts))?
    }
    DiffKind::Staged => {
      let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
      let index = repo.index()?;
      repo.diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut opts))?
    }
  };

  let hunks: RefCell<Vec<DiffHunk>> = RefCell::new(Vec::new());
  let current: RefCell<Option<DiffHunk>> = RefCell::new(None);

  diff.foreach(
    &mut |_file, _progress| true,
    None,
    Some(&mut |_file, hunk| {
      if let Some(mut hunk) = current.borrow_mut().take() {
        normalize_no_newline_hunk(&mut hunk, trailing_change);
        hunk.id = compute_hunk_id(&hunk);
        hunks.borrow_mut().push(hunk);
      }

      *current.borrow_mut() = Some(DiffHunk {
        id: String::new(),
        old_start: hunk.old_start() as usize,
        old_lines: hunk.old_lines() as usize,
        new_start: hunk.new_start() as usize,
        new_lines: hunk.new_lines() as usize,
        lines: Vec::new(),
      });

      true
    }),
    Some(&mut |_file, _hunk, line| {
      if let Some(hunk) = current.borrow_mut().as_mut() {
        if apply_no_newline_marker(&line, hunk) {
          return true;
        }
        if let Some(diff_line) = DiffLine::from_git_line(line) {
          hunk.lines.push(diff_line);
        }
      }

      true
    }),
  )?;

  let mut current = current.into_inner();
  if let Some(mut hunk) = current.take() {
    normalize_no_newline_hunk(&mut hunk, trailing_change);
    hunk.id = compute_hunk_id(&hunk);
    hunks.borrow_mut().push(hunk);
  }

  Ok(FileDiff {
    kind,
    hunks: hunks.into_inner(),
  })
}

fn split_lines(text: &str) -> (Vec<String>, bool) {
  if text.is_empty() {
    return (Vec::new(), false);
  }
  let trailing_newline = text.ends_with('\n');
  let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
  if trailing_newline {
    lines.pop();
  }
  if lines.len() == 1 && lines[0].is_empty() && !trailing_newline {
    lines.clear();
  }
  (lines, trailing_newline)
}

pub fn compute_buffer_diff(
  kind: DiffKind,
  base: Option<&str>,
  buffer_text: &str,
  rel_path: &Path,
) -> Result<FileDiff> {
  let mut opts = DiffOptions::new();
  opts.context_lines(3).patience(true).indent_heuristic(true);

  let base_text = base.unwrap_or("");
  let trailing_change = trailing_newline_change(base_text, buffer_text);
  let patch = Patch::from_buffers(
    base_text.as_bytes(),
    Some(rel_path),
    buffer_text.as_bytes(),
    Some(rel_path),
    Some(&mut opts),
  )?;

  let mut hunks = Vec::new();
  for hunk_idx in 0..patch.num_hunks() {
    let (hunk, line_count) = patch.hunk(hunk_idx)?;
    let mut diff_hunk = DiffHunk {
      id: String::new(),
      old_start: hunk.old_start() as usize,
      old_lines: hunk.old_lines() as usize,
      new_start: hunk.new_start() as usize,
      new_lines: hunk.new_lines() as usize,
      lines: Vec::new(),
    };

    for line_idx in 0..line_count {
      let line = patch.line_in_hunk(hunk_idx, line_idx)?;
      if apply_no_newline_marker(&line, &mut diff_hunk) {
        continue;
      }
      if let Some(diff_line) = DiffLine::from_git_line(line) {
        diff_hunk.lines.push(diff_line);
      }
    }

    normalize_no_newline_hunk(&mut diff_hunk, trailing_change);
    diff_hunk.id = compute_hunk_id(&diff_hunk);
    hunks.push(diff_hunk);
  }

  Ok(FileDiff { kind, hunks })
}

/// Build a diff set from a unified patch for a single file.
pub fn diff_set_from_patch(patch: &str) -> Result<DiffSet> {
  let uncommitted = file_diff_from_patch(patch, DiffKind::Uncommitted)?;
  Ok(DiffSet {
    uncommitted,
    unstaged: FileDiff {
      kind: DiffKind::Unstaged,
      hunks: Vec::new(),
    },
    staged: FileDiff {
      kind: DiffKind::Staged,
      hunks: Vec::new(),
    },
  })
}

fn file_diff_from_patch(patch: &str, kind: DiffKind) -> Result<FileDiff> {
  let diff = Diff::from_buffer(patch.as_bytes())?;
  let hunks: RefCell<Vec<DiffHunk>> = RefCell::new(Vec::new());
  let current: RefCell<Option<DiffHunk>> = RefCell::new(None);

  diff.foreach(
    &mut |_file, _progress| true,
    None,
    Some(&mut |_file, hunk| {
      if let Some(mut hunk) = current.borrow_mut().take() {
        normalize_no_newline_hunk(&mut hunk, None);
        hunk.id = compute_hunk_id(&hunk);
        hunks.borrow_mut().push(hunk);
      }

      *current.borrow_mut() = Some(DiffHunk {
        id: String::new(),
        old_start: hunk.old_start() as usize,
        old_lines: hunk.old_lines() as usize,
        new_start: hunk.new_start() as usize,
        new_lines: hunk.new_lines() as usize,
        lines: Vec::new(),
      });

      true
    }),
    Some(&mut |_file, _hunk, line| {
      if let Some(hunk) = current.borrow_mut().as_mut() {
        if apply_no_newline_marker(&line, hunk) {
          return true;
        }
        if let Some(diff_line) = DiffLine::from_git_line(line) {
          hunk.lines.push(diff_line);
        }
      }

      true
    }),
  )?;

  let mut current = current.into_inner();
  if let Some(mut hunk) = current.take() {
    normalize_no_newline_hunk(&mut hunk, None);
    hunk.id = compute_hunk_id(&hunk);
    hunks.borrow_mut().push(hunk);
  }

  Ok(FileDiff {
    kind,
    hunks: hunks.into_inner(),
  })
}

impl DiffLine {
  fn from_git_line(line: git2::DiffLine) -> Option<Self> {
    let (kind, no_newline) = match line.origin_value() {
      DiffLineType::Context => (DiffLineKind::Context, false),
      DiffLineType::Addition => (DiffLineKind::Add, false),
      DiffLineType::Deletion => (DiffLineKind::Remove, false),
      DiffLineType::ContextEOFNL | DiffLineType::AddEOFNL | DiffLineType::DeleteEOFNL => {
        return None;
      }
      _ => return None,
    };

    let mut text = String::from_utf8_lossy(line.content()).to_string();
    if text.ends_with('\n') || text.ends_with('\r') {
      text = text.trim_end_matches(['\r', '\n']).to_string();
    }

    Some(DiffLine {
      kind,
      content: Arc::from(text),
      no_newline,
    })
  }
}

fn apply_no_newline_marker(line: &git2::DiffLine, hunk: &mut DiffHunk) -> bool {
  match line.origin_value() {
    DiffLineType::ContextEOFNL | DiffLineType::AddEOFNL | DiffLineType::DeleteEOFNL => {
      if let Some(last) = hunk.lines.last_mut() {
        last.no_newline = true;
      }
      true
    }
    _ => false,
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrailingNewlineChange {
  Added,
  Removed,
}

fn trailing_newline_change(base_text: &str, buffer_text: &str) -> Option<TrailingNewlineChange> {
  let base_has_newline = base_text.ends_with('\n');
  let buffer_has_newline = buffer_text.ends_with('\n');
  match (base_has_newline, buffer_has_newline) {
    (false, true) => Some(TrailingNewlineChange::Added),
    (true, false) => Some(TrailingNewlineChange::Removed),
    _ => None,
  }
}

fn normalize_no_newline_hunk(hunk: &mut DiffHunk, trailing_change: Option<TrailingNewlineChange>) {
  let has_change = hunk
    .lines
    .iter()
    .any(|line| !matches!(line.kind, DiffLineKind::Context));
  if !has_change {
    if let Some(change) = trailing_change {
      let mut normalized = hunk.lines.clone();
      normalized.push(DiffLine {
        kind: match change {
          TrailingNewlineChange::Added => DiffLineKind::Add,
          TrailingNewlineChange::Removed => DiffLineKind::Remove,
        },
        content: Arc::from(""),
        no_newline: false,
      });
      hunk.lines = normalized;
      let (old_lines, new_lines) = count_hunk_line_counts(hunk);
      hunk.old_lines = old_lines;
      hunk.new_lines = new_lines;
    }
    return;
  }

  let mut normalized = Vec::new();
  let mut idx = 0;
  let mut changed = false;
  let last_change_idx = hunk
    .lines
    .iter()
    .rposition(|line| !matches!(line.kind, DiffLineKind::Context));
  while idx < hunk.lines.len() {
    if idx + 1 < hunk.lines.len() {
      let remove = &hunk.lines[idx];
      let add = &hunk.lines[idx + 1];
      let pair_is_last_change = last_change_idx == Some(idx + 1);
      let content_matches = remove.content == add.content;
      let no_newline_diff = remove.no_newline != add.no_newline;
      let should_normalize = remove.kind == DiffLineKind::Remove
        && add.kind == DiffLineKind::Add
        && content_matches
        && (no_newline_diff || (pair_is_last_change && trailing_change.is_some()));
      if should_normalize {
        changed = true;
        normalized.push(DiffLine {
          kind: DiffLineKind::Context,
          content: remove.content.clone(),
          no_newline: false,
        });

        let change = if no_newline_diff {
          if remove.no_newline && !add.no_newline {
            Some(TrailingNewlineChange::Added)
          } else {
            Some(TrailingNewlineChange::Removed)
          }
        } else {
          trailing_change
        };

        if let Some(change) = change {
          match change {
            TrailingNewlineChange::Added => {
              normalized.push(DiffLine {
                kind: DiffLineKind::Add,
                content: Arc::from(""),
                no_newline: false,
              });
            }
            TrailingNewlineChange::Removed => {
              normalized.push(DiffLine {
                kind: DiffLineKind::Remove,
                content: Arc::from(""),
                no_newline: false,
              });
            }
          }
        }

        idx += 2;
        continue;
      }
    }

    normalized.push(hunk.lines[idx].clone());
    idx += 1;
  }

  if changed {
    hunk.lines = normalized;
  }

  let (old_lines, new_lines) = count_hunk_line_counts(hunk);
  hunk.old_lines = old_lines;
  hunk.new_lines = new_lines;
}

fn trailing_newline_change_for_diff(
  kind: DiffKind,
  repo: &Repository,
  rel_path: &Path,
) -> Result<Option<TrailingNewlineChange>> {
  let (old_text, new_text) = match kind {
    DiffKind::Uncommitted => (
      read_head_content(repo, rel_path)?,
      read_workdir_content(repo, rel_path)?,
    ),
    DiffKind::Unstaged => (
      read_index_content(repo, rel_path)?,
      read_workdir_content(repo, rel_path)?,
    ),
    DiffKind::Staged => (
      read_head_content(repo, rel_path)?,
      read_index_content(repo, rel_path)?,
    ),
  };

  let (Some(old_text), Some(new_text)) = (old_text, new_text) else {
    return Ok(None);
  };
  Ok(trailing_newline_change(&old_text, &new_text))
}

fn read_head_content(repo: &Repository, rel_path: &Path) -> Result<Option<String>> {
  let head = match repo.head() {
    Ok(head) => head,
    Err(_) => return Ok(None),
  };
  let tree = match head.peel_to_tree() {
    Ok(tree) => tree,
    Err(_) => return Ok(None),
  };
  read_tree_content(repo, &tree, rel_path)
}

fn read_index_content(repo: &Repository, rel_path: &Path) -> Result<Option<String>> {
  let index = repo.index()?;
  let entry = match index.get_path(rel_path, 0) {
    Some(entry) => entry,
    None => return Ok(None),
  };
  let blob = repo.find_blob(entry.id)?;
  Ok(Some(String::from_utf8_lossy(blob.content()).into_owned()))
}

fn read_workdir_content(repo: &Repository, rel_path: &Path) -> Result<Option<String>> {
  let Some(workdir) = repo.workdir() else {
    return Ok(None);
  };
  let path = workdir.join(rel_path);
  match fs::read(&path) {
    Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
    Err(err) => Err(err.into()),
  }
}

fn read_tree_content(
  repo: &Repository,
  tree: &git2::Tree,
  rel_path: &Path,
) -> Result<Option<String>> {
  let entry = match tree.get_path(rel_path) {
    Ok(entry) => entry,
    Err(_) => return Ok(None),
  };
  let blob = repo.find_blob(entry.id())?;
  Ok(Some(String::from_utf8_lossy(blob.content()).into_owned()))
}

fn count_hunk_line_counts(hunk: &DiffHunk) -> (usize, usize) {
  let mut old_lines: usize = 0;
  let mut new_lines: usize = 0;
  for line in &hunk.lines {
    match line.kind {
      DiffLineKind::Context => {
        old_lines = old_lines.saturating_add(1);
        new_lines = new_lines.saturating_add(1);
      }
      DiffLineKind::Add => {
        new_lines = new_lines.saturating_add(1);
      }
      DiffLineKind::Remove => {
        old_lines = old_lines.saturating_add(1);
      }
    }
  }
  (old_lines, new_lines)
}

fn compute_hunk_id(hunk: &DiffHunk) -> String {
  let mut hasher = blake3::Hasher::new();
  hasher.update(
    format!(
      "{},{},{},{}\n",
      hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
    )
    .as_bytes(),
  );

  for line in &hunk.lines {
    let prefix = match line.kind {
      DiffLineKind::Context => b' ',
      DiffLineKind::Add => b'+',
      DiffLineKind::Remove => b'-',
    };
    hasher.update(&[prefix]);
    hasher.update(line.content.as_bytes());
    if line.no_newline {
      hasher.update(b"\\ No newline at end of file");
    }
    hasher.update(b"\n");
  }

  hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::Path;

  fn diff_kinds(lines: &[DiffLine]) -> Vec<(DiffLineKind, String)> {
    lines
      .iter()
      .map(|line| (line.kind, line.content.to_string()))
      .collect()
  }

  #[test]
  fn newline_added_at_eof_is_empty_add_line() {
    let base = "line";
    let buffer = "line\n";
    let diff = compute_buffer_diff(
      DiffKind::Uncommitted,
      Some(base),
      buffer,
      Path::new("test.txt"),
    )
    .expect("diff");

    assert_eq!(diff.hunks.len(), 1);
    let lines = diff_kinds(&diff.hunks[0].lines);
    assert_eq!(
      lines,
      vec![
        (DiffLineKind::Context, "line".to_string()),
        (DiffLineKind::Add, String::new()),
      ]
    );
  }

  #[test]
  fn newline_removed_at_eof_is_empty_remove_line() {
    let base = "line\n";
    let buffer = "line";
    let diff = compute_buffer_diff(
      DiffKind::Uncommitted,
      Some(base),
      buffer,
      Path::new("test.txt"),
    )
    .expect("diff");

    assert_eq!(diff.hunks.len(), 1);
    let lines = diff_kinds(&diff.hunks[0].lines);
    assert_eq!(
      lines,
      vec![
        (DiffLineKind::Context, "line".to_string()),
        (DiffLineKind::Remove, String::new()),
      ]
    );
  }

  #[test]
  fn diff_set_from_patch_parses_single_hunk() {
    let patch = r#"diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 line1
-line2
+line2b
 line3
"#;

    let diff_set = diff_set_from_patch(patch).expect("diff set");
    assert_eq!(diff_set.unstaged.hunks.len(), 0);
    assert_eq!(diff_set.staged.hunks.len(), 0);
    assert_eq!(diff_set.uncommitted.hunks.len(), 1);

    let hunk = &diff_set.uncommitted.hunks[0];
    assert!(!hunk.id.is_empty());
    let lines = diff_kinds(&hunk.lines);
    assert_eq!(
      lines,
      vec![
        (DiffLineKind::Context, "line1".to_string()),
        (DiffLineKind::Remove, "line2".to_string()),
        (DiffLineKind::Add, "line2b".to_string()),
        (DiffLineKind::Context, "line3".to_string()),
      ]
    );
  }

  #[test]
  fn compute_buffer_diffs_reuses_uncommitted_when_head_matches_index() {
    let base = "line1\nline2\nline3\n";
    let buffer = "line1\nline2 changed\nline3\n";
    let bases = GitFileBases {
      head: Some(base.to_string()),
      index: Some(base.to_string()),
    };

    let diffs = compute_buffer_diffs(&bases, buffer, Path::new("test.txt")).expect("diffs");

    assert_eq!(diffs.uncommitted.hunks.len(), 1);
    assert_eq!(diffs.unstaged.hunks.len(), diffs.uncommitted.hunks.len());
    assert_eq!(
      diffs.unstaged.hunks[0].old_start,
      diffs.uncommitted.hunks[0].old_start
    );
    assert_eq!(
      diffs.unstaged.hunks[0].new_start,
      diffs.uncommitted.hunks[0].new_start
    );
    assert_eq!(
      diff_kinds(&diffs.unstaged.hunks[0].lines),
      diff_kinds(&diffs.uncommitted.hunks[0].lines)
    );
    assert_eq!(diffs.unstaged.kind, DiffKind::Unstaged);
    assert!(diffs.staged.hunks.is_empty());
  }

  #[test]
  fn file_diff_from_patch_parses_multiple_hunks() {
    let patch = r#"diff --git a/file.txt b/file.txt
index 1111111..2222222 100644
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
 line1
-line2
+line2b
 line3
@@ -5,2 +5,3 @@
 line5
 line6
+line7
"#;

    let diff = file_diff_from_patch(patch, DiffKind::Uncommitted).expect("diff");
    assert_eq!(diff.hunks.len(), 2);
    assert!(!diff.hunks[0].id.is_empty());
    assert!(!diff.hunks[1].id.is_empty());
  }

  #[test]
  fn file_diff_from_patch_marks_no_newline() {
    let patch = r#"diff --git a/file.txt b/file.txt
index 1111111..2222222 100644
--- a/file.txt
+++ b/file.txt
@@ -1 +1 @@
-line1
+line1 modified
\ No newline at end of file
"#;

    let diff = file_diff_from_patch(patch, DiffKind::Uncommitted).expect("diff");
    let has_no_newline = diff
      .hunks
      .iter()
      .flat_map(|hunk| hunk.lines.iter())
      .any(|line| line.no_newline);
    assert!(has_no_newline);
  }

  #[test]
  fn diff_set_from_patch_rejects_invalid_input() {
    let err = diff_set_from_patch("not a diff").err();
    assert!(err.is_some());
  }

  #[test]
  fn diff_set_from_patch_parses_multiple_files() {
    let patch = r#"diff --git a/packages/server/src/router/github.ts b/packages/server/src/router/github.ts
index f33cea64..15f46e24 100644
--- a/packages/server/src/router/github.ts
+++ b/packages/server/src/router/github.ts
@@ -1,6 +1,13 @@
-import { request } from '@octokit/request'
-import { authProcedure, router } from '../lib/trpc.js'
-
-export const githubRouter = router(
-  {
-    getPullRequests: authProcedure.query(async ({ ctx: { session } }) => {
+import type { PullRequest } from '../types/github.js'
+import { request } from '@octokit/request'
+import z from 'zod'
+import { authProcedure, router } from '../lib/trpc.js'
+
+const getPullRequestsSchema = z.object({
+  owner: z.string(),
+  repo: z.string(),
+})
+
+export const githubRouter = router(
+  {
+    getPullRequests: authProcedure.input(getPullRequestsSchema).query(async ({ ctx: { ghAccessToken }, input }) => {
diff --git a/packages/server/src/types/github.ts b/packages/server/src/types/github.ts
new file mode 100644
index 00000000..17ef03df
--- /dev/null
+++ b/packages/server/src/types/github.ts
@@ -0,0 +1,4 @@
+import type { Endpoints } from '@octokit/types'
+
+export type PullRequest = Endpoints['GET /repos/{owner}/{repo}/pulls']['response']['data'][number]
+export type PullRequestDetails = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}']['response']['data']
diff --git a/packages/server/src/types/index.ts b/packages/server/src/types/index.ts
new file mode 100644
index 00000000..eac8680b
--- /dev/null
+++ b/packages/server/src/types/index.ts
@@ -0,0 +1 @@
+export * from './github.js'
"#;

    let diff_set = diff_set_from_patch(patch).expect("diff set");
    assert_eq!(diff_set.uncommitted.hunks.len(), 3);
    assert!(
      diff_set
        .uncommitted
        .hunks
        .iter()
        .all(|hunk| !hunk.id.is_empty())
    );
  }

  #[test]
  fn file_diff_from_patch_parses_deleted_file() {
    let patch = r#"diff --git a/old.txt b/old.txt
deleted file mode 100644
index 1111111..0000000
--- a/old.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-line1
-line2
"#;

    let diff = file_diff_from_patch(patch, DiffKind::Uncommitted).expect("diff");
    assert_eq!(diff.hunks.len(), 1);
    let lines = diff_kinds(&diff.hunks[0].lines);
    assert_eq!(
      lines,
      vec![
        (DiffLineKind::Remove, "line1".to_string()),
        (DiffLineKind::Remove, "line2".to_string()),
      ]
    );
  }
}

fn build_hunk_patch(rel_path: PathBuf, hunk: &DiffHunk, reverse: bool) -> String {
  let old_start = if reverse {
    hunk.new_start
  } else {
    hunk.old_start
  };
  let old_lines = if reverse {
    hunk.new_lines
  } else {
    hunk.old_lines
  };
  let new_start = if reverse {
    hunk.old_start
  } else {
    hunk.new_start
  };
  let new_lines = if reverse {
    hunk.old_lines
  } else {
    hunk.new_lines
  };

  let old_range = format_hunk_range(old_start, old_lines);
  let new_range = format_hunk_range(new_start, new_lines);

  let rel_path = rel_path.to_string_lossy();
  let mut patch = String::new();
  patch.push_str(&format!("diff --git a/{path} b/{path}\n", path = rel_path));
  patch.push_str(&format!("--- a/{path}\n", path = rel_path));
  patch.push_str(&format!("+++ b/{path}\n", path = rel_path));
  patch.push_str(&format!("@@ -{} +{} @@\n", old_range, new_range));

  for line in &hunk.lines {
    let mut kind = line.kind;
    if reverse {
      kind = match kind {
        DiffLineKind::Add => DiffLineKind::Remove,
        DiffLineKind::Remove => DiffLineKind::Add,
        DiffLineKind::Context => DiffLineKind::Context,
      };
    }

    let prefix = match kind {
      DiffLineKind::Context => ' ',
      DiffLineKind::Add => '+',
      DiffLineKind::Remove => '-',
    };

    patch.push(prefix);
    patch.push_str(line.content.as_ref());
    patch.push('\n');

    if line.no_newline {
      patch.push_str("\\ No newline at end of file\n");
    }
  }

  patch
}

fn format_hunk_range(start: usize, lines: usize) -> String {
  if lines == 1 {
    start.to_string()
  } else {
    format!("{},{}", start, lines)
  }
}
