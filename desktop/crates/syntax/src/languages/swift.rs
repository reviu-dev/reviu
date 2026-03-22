use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static SWIFT_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "swift",
    tree_sitter_swift::LANGUAGE.into(),
    &[tree_sitter_swift::HIGHLIGHTS_QUERY],
    &[tree_sitter_swift::INJECTIONS_QUERY],
    &[tree_sitter_swift::LOCALS_QUERY],
  )
});
