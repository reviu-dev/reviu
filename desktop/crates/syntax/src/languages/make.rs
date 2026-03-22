use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static MAKE_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "make",
    tree_sitter_make::LANGUAGE.into(),
    &[tree_sitter_make::HIGHLIGHTS_QUERY],
    &[],
    &[],
  )
});
