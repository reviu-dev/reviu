use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static SQL_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "sql",
    tree_sitter_sequel::LANGUAGE.into(),
    &[tree_sitter_sequel::HIGHLIGHTS_QUERY],
    &[],
    &[],
  )
});
