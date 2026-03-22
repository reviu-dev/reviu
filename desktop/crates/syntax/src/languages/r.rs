use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static R_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "r",
    tree_sitter_r::LANGUAGE.into(),
    &[tree_sitter_r::HIGHLIGHTS_QUERY],
    &[],
    &[tree_sitter_r::LOCALS_QUERY],
  )
});
