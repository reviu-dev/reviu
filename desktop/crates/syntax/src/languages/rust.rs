use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static RUST_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "rust",
    tree_sitter_rust::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/rust-highlights.scm")],
    &[],
    &[],
  )
});
