use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;
use tree_sitter_dockerfile as _;

unsafe extern "C" {
  fn tree_sitter_dockerfile() -> tree_sitter::Language;
}

pub static DOCKERFILE_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "dockerfile",
    unsafe { tree_sitter_dockerfile() },
    &[include_str!(
      "../tree-sitter-queries/dockerfile-highlights.scm"
    )],
    &[],
    &[],
  )
});
