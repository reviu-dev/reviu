use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static RUBY_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "ruby",
    tree_sitter_ruby::LANGUAGE.into(),
    &[tree_sitter_ruby::HIGHLIGHTS_QUERY],
    &[],
    &[tree_sitter_ruby::LOCALS_QUERY],
  )
});
