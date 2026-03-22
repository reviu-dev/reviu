use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static KOTLIN_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "kotlin",
    tree_sitter_kotlin_sg::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/kotlin-highlights.scm")],
    &[],
    &[],
  )
});
