use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static ELIXIR_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "elixir",
    tree_sitter_elixir::LANGUAGE.into(),
    &[tree_sitter_elixir::HIGHLIGHTS_QUERY],
    &[tree_sitter_elixir::INJECTIONS_QUERY],
    &[],
  )
});
