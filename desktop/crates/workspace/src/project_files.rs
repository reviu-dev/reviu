use anyhow::Result;
use git2::Repository;
use std::path::{Path, PathBuf};

const PROJECT_FILE_SEARCH_LIMIT: usize = 20_000;
const DEFAULT_FILE_SCAN_EXCLUDED_NAMES: &[&str] = &[
  ".git",
  ".svn",
  ".hg",
  ".jj",
  ".sl",
  ".repo",
  "CVS",
  ".DS_Store",
  "Thumbs.db",
  ".classpath",
  ".settings",
  "node_modules",
  "target",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProjectEntryKind {
  File,
  Directory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectEntry {
  pub path: PathBuf,
  pub kind: ProjectEntryKind,
  pub is_gitignored: bool,
  pub is_hidden: bool,
}

impl ProjectEntry {
  pub fn file(path: PathBuf) -> Self {
    Self::new(path, ProjectEntryKind::File, false, false)
  }

  fn new(path: PathBuf, kind: ProjectEntryKind, is_gitignored: bool, is_hidden: bool) -> Self {
    Self {
      path,
      kind,
      is_gitignored,
      is_hidden,
    }
  }

  pub fn is_file(&self) -> bool {
    self.kind == ProjectEntryKind::File
  }

  pub fn path_id(&self) -> String {
    self
      .path
      .to_string_lossy()
      .replace(std::path::MAIN_SEPARATOR, "/")
  }
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectScanOptions {
  pub include_gitignored: bool,
  pub include_hidden: bool,
  pub max_files: usize,
}

impl Default for ProjectScanOptions {
  fn default() -> Self {
    Self {
      include_gitignored: true,
      include_hidden: true,
      max_files: PROJECT_FILE_SEARCH_LIMIT,
    }
  }
}

pub(crate) fn list_project_entries(project_root: &Path) -> Result<Vec<ProjectEntry>> {
  list_project_entries_with_options(project_root, &ProjectScanOptions::default())
}

pub(crate) fn list_project_entries_with_options(
  project_root: &Path,
  options: &ProjectScanOptions,
) -> Result<Vec<ProjectEntry>> {
  let gitignore = GitIgnoreChecker::new(project_root);
  let mut entries = Vec::new();
  let mut file_count = 0;
  let mut dirs = vec![PathBuf::new()];

  while let Some(rel_dir) = dirs.pop() {
    for entry in std::fs::read_dir(project_root.join(&rel_dir))? {
      let entry = entry?;
      let name = entry.file_name();
      let name = name.to_string_lossy();
      if is_default_file_scan_excluded_name(name.as_ref()) {
        continue;
      }

      let rel_path = rel_dir.join(name.as_ref());
      let file_type = entry.file_type()?;
      let kind = if file_type.is_dir() {
        ProjectEntryKind::Directory
      } else if file_type.is_file() {
        ProjectEntryKind::File
      } else {
        continue;
      };
      let is_hidden = is_hidden_path(&rel_path);
      if is_hidden && !options.include_hidden {
        if kind == ProjectEntryKind::Directory {
          continue;
        }
        continue;
      }

      let is_gitignored = gitignore
        .as_ref()
        .is_some_and(|gitignore| gitignore.is_ignored(project_root, &rel_path));
      if is_gitignored && !options.include_gitignored {
        if kind == ProjectEntryKind::Directory {
          continue;
        }
        continue;
      }

      if kind == ProjectEntryKind::Directory {
        dirs.push(rel_path.clone());
      } else {
        file_count += 1;
      }

      entries.push(ProjectEntry::new(rel_path, kind, is_gitignored, is_hidden));
      if file_count >= options.max_files {
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        return Ok(entries);
      }
    }
  }

  entries.sort_by(|a, b| a.path.cmp(&b.path));
  Ok(entries)
}

pub(crate) fn list_project_files(project_root: &Path) -> Result<Vec<PathBuf>> {
  Ok(
    list_project_entries(project_root)?
      .into_iter()
      .filter(ProjectEntry::is_file)
      .map(|entry| entry.path)
      .collect(),
  )
}

pub(crate) fn list_project_search_files(project_root: &Path) -> Result<Vec<PathBuf>> {
  Ok(
    list_project_entries_with_options(
      project_root,
      &ProjectScanOptions {
        include_gitignored: false,
        include_hidden: false,
        ..ProjectScanOptions::default()
      },
    )?
    .into_iter()
    .filter(ProjectEntry::is_file)
    .map(|entry| entry.path)
    .collect(),
  )
}

pub(crate) fn is_default_file_scan_excluded_name(name: &str) -> bool {
  DEFAULT_FILE_SCAN_EXCLUDED_NAMES.contains(&name)
}

fn is_hidden_path(path: &Path) -> bool {
  path
    .components()
    .any(|component| component.as_os_str().to_string_lossy().starts_with('.'))
}

struct GitIgnoreChecker {
  repo: Repository,
  workdir: PathBuf,
}

impl GitIgnoreChecker {
  fn new(project_root: &Path) -> Option<Self> {
    let repo = Repository::discover(project_root).ok()?;
    let workdir = repo.workdir()?.to_path_buf();
    Some(Self { repo, workdir })
  }

  fn is_ignored(&self, project_root: &Path, rel_path: &Path) -> bool {
    let absolute_path = project_root.join(rel_path);
    let repo_path = absolute_path
      .strip_prefix(&self.workdir)
      .unwrap_or(rel_path);
    self.repo.status_should_ignore(repo_path).unwrap_or(false)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::{TempDir, TempRepo, commit_text_file};

  #[test]
  fn project_entries_include_gitignored_files_but_skip_hard_excludes() {
    let repo = TempRepo::init("project-files-ignored");
    commit_text_file(&repo.path, Path::new(".gitignore"), "ignored/\n", "ignore");
    std::fs::create_dir_all(repo.path.join("ignored")).expect("create ignored dir");
    std::fs::write(repo.path.join("ignored/secret.txt"), "secret\n").expect("write ignored");
    std::fs::create_dir_all(repo.path.join("node_modules/pkg")).expect("create node_modules");
    std::fs::write(repo.path.join("node_modules/pkg/index.js"), "module\n").expect("write module");

    let entries = list_project_entries(&repo.path).expect("list entries");
    let paths = entries
      .iter()
      .map(|entry| entry.path.as_path())
      .collect::<Vec<_>>();

    assert!(paths.contains(&Path::new("ignored")));
    assert!(paths.contains(&Path::new("ignored/secret.txt")));
    assert!(!paths.contains(&Path::new("node_modules")));
    assert!(!paths.contains(&Path::new("node_modules/pkg/index.js")));

    let ignored = entries
      .iter()
      .find(|entry| entry.path == Path::new("ignored/secret.txt"))
      .expect("ignored file");
    assert!(ignored.is_gitignored);
  }

  #[test]
  fn project_entries_can_hide_gitignored_files() {
    let repo = TempRepo::init("project-files-hide-ignored");
    commit_text_file(&repo.path, Path::new(".gitignore"), "ignored/\n", "ignore");
    std::fs::create_dir_all(repo.path.join("ignored")).expect("create ignored dir");
    std::fs::write(repo.path.join("ignored/secret.txt"), "secret\n").expect("write ignored");
    std::fs::write(repo.path.join("visible.txt"), "visible\n").expect("write visible");

    let entries = list_project_entries_with_options(
      &repo.path,
      &ProjectScanOptions {
        include_gitignored: false,
        ..ProjectScanOptions::default()
      },
    )
    .expect("list entries");
    let paths = entries
      .iter()
      .map(|entry| entry.path.as_path())
      .collect::<Vec<_>>();

    assert!(paths.contains(&Path::new("visible.txt")));
    assert!(!paths.contains(&Path::new("ignored")));
    assert!(!paths.contains(&Path::new("ignored/secret.txt")));
  }

  #[test]
  fn project_entries_can_hide_hidden_files() {
    let dir = TempDir::new("project-files-hidden");
    std::fs::write(dir.path.join("visible.txt"), "visible\n").expect("write visible");
    std::fs::create_dir_all(dir.path.join(".hidden")).expect("create hidden dir");
    std::fs::write(dir.path.join(".hidden/secret.txt"), "hidden\n").expect("write hidden");

    let entries = list_project_entries_with_options(
      &dir.path,
      &ProjectScanOptions {
        include_hidden: false,
        ..ProjectScanOptions::default()
      },
    )
    .expect("list entries");
    let paths = entries
      .iter()
      .map(|entry| entry.path.as_path())
      .collect::<Vec<_>>();

    assert!(paths.contains(&Path::new("visible.txt")));
    assert!(!paths.contains(&Path::new(".hidden")));
    assert!(!paths.contains(&Path::new(".hidden/secret.txt")));
  }

  #[test]
  fn project_search_files_skip_ignored_and_hidden_files() {
    let repo = TempRepo::init("project-search-files");
    commit_text_file(&repo.path, Path::new(".gitignore"), "ignored/\n", "ignore");
    std::fs::write(repo.path.join("visible.txt"), "visible\n").expect("write visible");
    std::fs::create_dir_all(repo.path.join("ignored")).expect("create ignored dir");
    std::fs::write(repo.path.join("ignored/secret.txt"), "secret\n").expect("write ignored");
    std::fs::create_dir_all(repo.path.join(".hidden")).expect("create hidden dir");
    std::fs::write(repo.path.join(".hidden/secret.txt"), "hidden\n").expect("write hidden");

    let paths = list_project_search_files(&repo.path).expect("list search files");

    assert!(paths.contains(&PathBuf::from("visible.txt")));
    assert!(!paths.contains(&PathBuf::from("ignored/secret.txt")));
    assert!(!paths.contains(&PathBuf::from(".hidden/secret.txt")));
  }
}
