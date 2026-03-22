use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static GO_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "go",
    tree_sitter_go::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/go-highlights.scm")],
    &[],
    &[],
  )
});
