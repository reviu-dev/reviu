use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static JSON_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "json",
    tree_sitter_json::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/json-highlights.scm")],
    &[],
    &[],
  )
});
