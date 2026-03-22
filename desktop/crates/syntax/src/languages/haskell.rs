use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static HASKELL_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "haskell",
    tree_sitter_haskell::LANGUAGE.into(),
    &[tree_sitter_haskell::HIGHLIGHTS_QUERY],
    &[tree_sitter_haskell::INJECTIONS_QUERY],
    &[tree_sitter_haskell::LOCALS_QUERY],
  )
});
