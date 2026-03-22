use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static TOML_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "toml",
    tree_sitter_toml_ng::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/toml-highlights.scm")],
    &[],
    &[],
  )
});
