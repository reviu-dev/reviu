use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static SCALA_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "scala",
    tree_sitter_scala::LANGUAGE.into(),
    &[tree_sitter_scala::HIGHLIGHTS_QUERY],
    &[],
    &[tree_sitter_scala::LOCALS_QUERY],
  )
});
