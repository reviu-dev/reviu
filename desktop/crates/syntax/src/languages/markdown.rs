use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static MARKDOWN_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "markdown",
    tree_sitter_md::LANGUAGE.into(),
    &[include_str!(
      "../tree-sitter-queries/markdown-highlights.scm"
    )],
    &[],
    &[],
  )
});
