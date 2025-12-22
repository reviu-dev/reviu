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

    // Load file contents for lazy line loading
    let old_content = if delta.status() != Delta::Added {
      self.load_file_content(old_file.id()).ok()
    } else {
      None
    };

    let new_content = if delta.status() != Delta::Deleted {
      // Try to read from working directory
      let workdir_path = self.repo.workdir().map(|wd| wd.join(path));
      if let Some(full_path) = workdir_path {
        std::fs::read_to_string(full_path).ok()
      } else {
        None
      }
    } else {
      None
    };

    Ok(Some(FileDiff {
      path: PathBuf::from(path),
      old_path,
      status,
      hunks: Vec::new(),
      old_content,
      new_content,
    }))
  }

  /// Load file content from a git blob
  fn load_file_content(&self, oid: git2::Oid) -> Result<String> {
    let blob = self.repo.find_blob(oid)?;
    let content = std::str::from_utf8(blob.content())
      .map_err(|e| Error::Unknown(format!("Invalid UTF-8 in blob: {}", e)))?;
    Ok(content.to_string())
  }

  /// Collect hunks for a specific file from the diff
  fn collect_hunks_for_file(&self, diff: &Diff, file_path: &std::path::Path) -> Result<Vec<Hunk>> {
    use std::cell::RefCell;
    use std::rc::Rc;

    let hunks = Rc::new(RefCell::new(Vec::new()));
    let is_target_file = Rc::new(RefCell::new(false));

    let hunks_clone = hunks.clone();
    let is_target_file_clone = is_target_file.clone();

    diff.print(git2::DiffFormat::Patch, move |delta, _hunk_opt, _line| {
      // Check if this is our target file
      let delta_path = delta.new_file().path().or_else(|| delta.old_file().path());
      let is_target = delta_path == Some(file_path);
      *is_target_file_clone.borrow_mut() = is_target;

      if !is_target {
        return true;
      }

      if _line.origin() == 'H' {
        // Hunk header - create new hunk
        if let Some(hunk_git) = _hunk_opt {
          if let Ok(parsed_hunk) = parse_hunk_from_git2(hunk_git) {
            let mut hunks_mut = hunks_clone.borrow_mut();
            hunks_mut.push(parsed_hunk);
          }
        }
      } else if let Some(line) = parse_line(_line) {
        // Add line to the current hunk
        let mut hunks_mut = hunks_clone.borrow_mut();
        if let Some(current_hunk) = hunks_mut.last_mut() {
          current_hunk.lines.push(line);
        }
      }

      true
    })?;

    let final_hunks = Rc::try_unwrap(hunks)
      .map_err(|_| Error::Unknown("Failed to unwrap hunks".into()))?
      .into_inner();

    Ok(final_hunks)
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
    old_byte_range: 0..0, // Will be calculated from file content
    new_byte_range: 0..0, // Will be calculated from file content
    lines: Vec::new(),    // Lines will be populated from git2 diff
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

  // git2 provides line numbers directly based on the origin:
  // - Context lines: both old_lineno and new_lineno are present
  // - Addition lines (+): only new_lineno is present
  // - Deletion lines (-): only old_lineno is present
  let old_lineno = line.old_lineno();
  let new_lineno = line.new_lineno();

  Some(Line {
    origin,
    content,
    old_lineno,
    new_lineno,
  })
}
