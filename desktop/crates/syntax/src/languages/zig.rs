use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static ZIG_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "zig",
    tree_sitter_zig::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/zig-highlights.scm")],
    &[include_str!("../tree-sitter-queries/zig-injections.scm")],
    &[],
  )
});
