use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static SVELTE_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "svelte",
    tree_sitter_svelte_next::LANGUAGE.into(),
    &[tree_sitter_svelte_next::HIGHLIGHTS_QUERY],
    &[tree_sitter_svelte_next::INJECTIONS_QUERY],
    &[tree_sitter_svelte_next::LOCALS_QUERY],
  )
});
