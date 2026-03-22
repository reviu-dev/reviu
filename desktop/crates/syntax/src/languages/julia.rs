use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static JULIA_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "julia",
    tree_sitter_julia::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/julia-highlights.scm")],
    &[],
    &[],
  )
});
