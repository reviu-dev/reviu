use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static DART_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "dart",
    tree_sitter_dart_orchard::LANGUAGE.into(),
    &[tree_sitter_dart_orchard::HIGHLIGHTS_QUERY],
    &[],
    &[],
  )
});
