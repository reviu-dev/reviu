use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static C_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "c",
    tree_sitter_c::LANGUAGE.into(),
    &[tree_sitter_c::HIGHLIGHT_QUERY],
    &[],
    &[],
  )
});
