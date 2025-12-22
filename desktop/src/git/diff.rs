use crate::error::{Error, Result};
use crate::state::{FileDiff, FileStatusKind, Hunk, HunkId, Line, LineOrigin};
use git2::{Delta, Diff, DiffOptions, Repository};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// Diff engine for calculating and managing diffs
pub struct DiffEngine<'repo> {
  repo: &'repo Repository,
}

impl<'repo> DiffEngine<'repo> {
  /// Create a new diff engine
  pub fn new(repo: &'repo Repository) -> Self {
    Self { repo }
  }

  /// Get diff for working directory vs index (unstaged changes)
  pub fn diff_workdir_to_index(&self) -> Result<Vec<FileDiff>> {
    let mut opts = DiffOptions::new();
    opts.context_lines(0); // Minimal context by default
    opts.interhunk_lines(0);

    let diff = self.repo.diff_index_to_workdir(None, Some(&mut opts))?;
    self.parse_diff(&diff)
  }

  /// Get diff for index vs HEAD (staged changes)
  pub fn diff_index_to_head(&self) -> Result<Vec<FileDiff>> {
    let mut opts = DiffOptions::new();
    opts.context_lines(0);
    opts.interhunk_lines(0);

    let head = self.repo.head()?.peel_to_tree()?;
    let diff = self
      .repo
      .diff_tree_to_index(Some(&head), None, Some(&mut opts))?;
    self.parse_diff(&diff)
  }

  /// Get diff for a specific file with custom context lines
  pub fn diff_file_with_context(
    &self,
    path: &std::path::Path,
    context_lines: u32,
    staged: bool,
  ) -> Result<Option<FileDiff>> {
    let mut opts = DiffOptions::new();
    opts.context_lines(context_lines);
    opts.interhunk_lines(0);
    opts.pathspec(path);

    let diff = if staged {
      let head = self.repo.head()?.peel_to_tree()?;
      self
        .repo
        .diff_tree_to_index(Some(&head), None, Some(&mut opts))?
    } else {
      self.repo.diff_index_to_workdir(None, Some(&mut opts))?
    };

    let mut file_diffs = self.parse_diff(&diff)?;
    Ok(file_diffs.pop())
  }

  /// Parse a git2::Diff into our FileDiff structure
  fn parse_diff(&self, diff: &Diff) -> Result<Vec<FileDiff>> {
    let mut file_diffs = Vec::new();

    diff.foreach(
      &mut |delta, _progress| {
        if let Some(file_diff) = self.parse_delta(&delta).ok().flatten() {
          file_diffs.push(file_diff);
        }
        true
      },
      None,
      None,
      None,
    )?;

    // Now collect hunks for each file
    for file_diff in &mut file_diffs {
      let hunks = self.collect_hunks_for_file(diff, &file_diff.path)?;
      file_diff.hunks = hunks;
    }

    Ok(file_diffs)
  }

  /// Parse a delta into a FileDiff (without hunks yet)
  fn parse_delta(&self, delta: &git2::DiffDelta) -> Result<Option<FileDiff>> {
    let new_file = delta.new_file();
    let old_file = delta.old_file();

    let path = new_file
      .path()
      .or_else(|| old_file.path())
      .ok_or_else(|| Error::Unknown("No path in delta".into()))?;

    let old_path = if delta.status() == Delta::Renamed {
      old_file.path().map(PathBuf::from)
    } else {
      None
    };

    let status = match delta.status() {
      Delta::Added => FileStatusKind::Added,
      Delta::Deleted => FileStatusKind::Deleted,
      Delta::Modified => FileStatusKind::Modified,
      Delta::Renamed => FileStatusKind::Renamed {
        from: old_path.clone(),
      },
      Delta::Copied => FileStatusKind::Copied {
        from: old_path.clone(),
      },
      Delta::Untracked => FileStatusKind::Untracked,
      _ => return Ok(None),
    };

    Ok(Some(FileDiff {
      path: PathBuf::from(path),
      old_path,
      status,
      hunks: Vec::new(),
    }))
  }

  /// Collect hunks for a specific file from the diff
  fn collect_hunks_for_file(&self, diff: &Diff, file_path: &std::path::Path) -> Result<Vec<Hunk>> {
    use std::cell::RefCell;
    use std::rc::Rc;

    let hunks = Rc::new(RefCell::new(Vec::new()));
    let current_hunk_idx = Rc::new(RefCell::new(None::<usize>));
    let is_target_file = Rc::new(RefCell::new(false));

    let hunks_clone = hunks.clone();
    let current_hunk_idx_clone = current_hunk_idx.clone();
    let is_target_file_clone = is_target_file.clone();

    diff.print(git2::DiffFormat::Patch, move |delta, _hunk_opt, line| {
      // Check if this is our target file
      let delta_path = delta.new_file().path().or_else(|| delta.old_file().path());
      let is_target = delta_path == Some(file_path);
      *is_target_file_clone.borrow_mut() = is_target;

      if !is_target {
        return true;
      }

      match line.origin() {
        'H' => {
          // Hunk header - create new hunk
          if let Some(hunk_git) = _hunk_opt {
            if let Ok(parsed_hunk) = parse_hunk_from_git2(hunk_git) {
              let mut hunks_mut = hunks_clone.borrow_mut();
              hunks_mut.push(parsed_hunk);
              *current_hunk_idx_clone.borrow_mut() = Some(hunks_mut.len() - 1);
            }
          }
        }
        ' ' | '+' | '-' => {
          // Diff line - add to current hunk
          if let Some(idx) = *current_hunk_idx_clone.borrow() {
            if let Some(parsed_line) = parse_line(line) {
              let mut hunks_mut = hunks_clone.borrow_mut();
              if let Some(hunk) = hunks_mut.get_mut(idx) {
                hunk.lines.push(parsed_line);
              }
            }
          }
        }
        _ => {}
      }

      true
    })?;

    let final_hunks = Rc::try_unwrap(hunks)
      .map_err(|_| Error::Unknown("Failed to unwrap hunks".into()))?
      .into_inner();

    Ok(final_hunks)
  }

  /// Parse a git2::DiffHunk into our Hunk structure (without lines)
  fn parse_hunk(&self, hunk: git2::DiffHunk) -> Result<Hunk> {
    parse_hunk_from_git2(hunk)
  }

  /// Expand context for a specific hunk
  pub fn expand_hunk_context(
    &self,
    file_path: &std::path::Path,
    hunk_id: HunkId,
    context_lines: u32,
    staged: bool,
  ) -> Result<Option<Hunk>> {
    let file_diff = self.diff_file_with_context(file_path, context_lines, staged)?;

    if let Some(file_diff) = file_diff {
      for mut hunk in file_diff.hunks {
        if hunk.id == hunk_id {
          hunk.context_expanded = true;
          return Ok(Some(hunk));
        }
      }
    }

    Ok(None)
  }
}

/// Parse a git2::DiffHunk into our Hunk structure (without lines)
fn parse_hunk_from_git2(hunk: git2::DiffHunk) -> Result<Hunk> {
  let header = String::from_utf8_lossy(hunk.header()).to_string();

  // Generate a unique ID for this hunk based on its position and header
  let mut hasher = DefaultHasher::new();
  hunk.old_start().hash(&mut hasher);
  hunk.new_start().hash(&mut hasher);
  header.hash(&mut hasher);
  let id = HunkId(hasher.finish());

  Ok(Hunk {
    id,
    old_start: hunk.old_start(),
    old_lines: hunk.old_lines(),
    new_start: hunk.new_start(),
    new_lines: hunk.new_lines(),
    header,
    lines: Vec::new(),
    context_expanded: false,
  })
}

/// Parse a git2::DiffLine into our Line structure
fn parse_line(line: git2::DiffLine) -> Option<Line> {
  let origin = match line.origin() {
    ' ' | '=' => LineOrigin::Context,
    '+' => LineOrigin::Addition,
    '-' => LineOrigin::Deletion,
    _ => return None,
  };

  let content = String::from_utf8_lossy(line.content()).to_string();

  // Get line numbers (they're -1 if not applicable)
  let old_lineno = if line.old_lineno().is_some() {
    line.old_lineno()
  } else {
    None
  };

  let new_lineno = if line.new_lineno().is_some() {
    line.new_lineno()
  } else {
    None
  };

  Some(Line {
    origin,
    content,
    old_lineno,
    new_lineno,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use git2::Signature;
  use std::fs;
  use tempfile::tempdir;

  fn setup_repo_with_changes() -> (tempfile::TempDir, Repository) {
    let dir = tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();

    // Configure user
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Test User").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();

    // Create initial commit
    let file_path = dir.path().join("test.txt");
    fs::write(&file_path, "line 1\nline 2\nline 3\n").unwrap();

    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("test.txt")).unwrap();
    index.write().unwrap();

    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = Signature::now("Test User", "test@example.com").unwrap();

    repo
      .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
      .unwrap();

    // Modify the file
    fs::write(&file_path, "line 1\nmodified line 2\nline 3\nnew line 4\n").unwrap();

    (dir, repo)
  }

  #[test]
  fn test_diff_engine_creation() {
    let dir = tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let engine = DiffEngine::new(&repo);
    // Just verify it compiles and creates
  }

  #[test]
  fn test_diff_workdir_to_index() {
    let (_dir, repo) = setup_repo_with_changes();
    let engine = DiffEngine::new(&repo);

    let diffs = engine.diff_workdir_to_index().unwrap();
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].path.to_str().unwrap(), "test.txt");
    assert!(matches!(diffs[0].status, FileStatusKind::Modified));
  }

  #[test]
  fn test_diff_with_hunks() {
    let (_dir, repo) = setup_repo_with_changes();
    let engine = DiffEngine::new(&repo);

    let diffs = engine.diff_workdir_to_index().unwrap();
    assert!(!diffs.is_empty());

    let file_diff = &diffs[0];
    assert!(!file_diff.hunks.is_empty());

    let hunk = &file_diff.hunks[0];
    assert!(!hunk.lines.is_empty());
  }

  #[test]
  fn test_staged_diff() {
    let (dir, repo) = setup_repo_with_changes();

    // Stage the changes
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("test.txt")).unwrap();
    index.write().unwrap();

    let engine = DiffEngine::new(&repo);
    let diffs = engine.diff_index_to_head().unwrap();

    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].path.to_str().unwrap(), "test.txt");
  }

  #[test]
  fn test_line_origins() {
    let (_dir, repo) = setup_repo_with_changes();
    let engine = DiffEngine::new(&repo);

    let diffs = engine.diff_workdir_to_index().unwrap();
    let file_diff = &diffs[0];
    let hunk = &file_diff.hunks[0];

    // Check that we have different line origins
    let has_addition = hunk
      .lines
      .iter()
      .any(|l| matches!(l.origin, LineOrigin::Addition));
    let has_deletion = hunk
      .lines
      .iter()
      .any(|l| matches!(l.origin, LineOrigin::Deletion));

    assert!(has_addition || has_deletion);
  }
}
