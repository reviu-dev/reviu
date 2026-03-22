use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static ASTRO_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "astro",
    tree_sitter_astro_next::LANGUAGE.into(),
    &[tree_sitter_astro_next::HIGHLIGHTS_QUERY],
    &[tree_sitter_astro_next::INJECTIONS_QUERY],
    &[],
  )
});
