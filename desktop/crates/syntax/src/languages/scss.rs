use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static SCSS_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "scss",
    tree_sitter_scss::language(),
    &[include_str!("../tree-sitter-queries/scss-highlights.scm")],
    &[],
    &[],
  )
});
