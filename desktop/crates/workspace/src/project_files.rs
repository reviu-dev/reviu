use anyhow::Result;
use std::path::{Path, PathBuf};

const PROJECT_FILE_SEARCH_LIMIT: usize = 20_000;

pub(crate) fn list_project_files(project_root: &Path) -> Result<Vec<PathBuf>> {
  let mut files = Vec::new();
  let mut dirs = vec![PathBuf::new()];
  while let Some(rel_dir) = dirs.pop() {
    for entry in std::fs::read_dir(project_root.join(&rel_dir))? {
      let entry = entry?;
      let name = entry.file_name();
      let name = name.to_string_lossy();
      if matches!(name.as_ref(), ".git" | "node_modules" | "target") {
        continue;
      }
      let rel_path = rel_dir.join(name.as_ref());
      let file_type = entry.file_type()?;
      if file_type.is_dir() {
        dirs.push(rel_path);
      } else if file_type.is_file() {
        files.push(rel_path);
        if files.len() >= PROJECT_FILE_SEARCH_LIMIT {
          files.sort();
          return Ok(files);
        }
      }
    }
  }
  files.sort();
  Ok(files)
}
