use std::path::Path;

pub fn is_markdown_path(path: &Path) -> bool {
  matches!(
    path
      .extension()
      .and_then(|ext| ext.to_str())
      .map(|ext| ext.to_ascii_lowercase())
      .as_deref(),
    Some("md" | "markdown" | "mdx")
  )
}

pub fn is_svg_path(path: &Path) -> bool {
  matches!(
    path
      .extension()
      .and_then(|ext| ext.to_str())
      .map(|ext| ext.to_ascii_lowercase())
      .as_deref(),
    Some("svg")
  )
}

pub fn is_previewable_path(path: &Path) -> bool {
  is_markdown_path(path) || is_svg_path(path)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn markdown_path_detection_is_case_insensitive_and_extension_based() {
    assert!(is_markdown_path(Path::new("README.md")));
    assert!(is_markdown_path(Path::new("docs/GUIDE.MD")));
    assert!(is_markdown_path(Path::new("notes.markdown")));
    assert!(is_markdown_path(Path::new("post.MdX")));
    assert!(!is_markdown_path(Path::new("README")));
    assert!(!is_markdown_path(Path::new("icon.svg")));
    assert!(!is_markdown_path(Path::new("note.md.txt")));
  }

  #[test]
  fn svg_path_detection_is_case_insensitive_and_extension_based() {
    assert!(is_svg_path(Path::new("icon.svg")));
    assert!(is_svg_path(Path::new("ICON.SVG")));
    assert!(!is_svg_path(Path::new("icon.svgz")));
    assert!(!is_svg_path(Path::new("README.md")));
    assert!(!is_svg_path(Path::new("icon")));
  }

  #[test]
  fn previewable_path_detection_accepts_markdown_and_svg_only() {
    assert!(is_previewable_path(Path::new("README.md")));
    assert!(is_previewable_path(Path::new("icon.svg")));
    assert!(!is_previewable_path(Path::new("script.ts")));
  }
}
