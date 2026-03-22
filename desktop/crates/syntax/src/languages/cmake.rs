use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static CMAKE_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "cmake",
    tree_sitter_cmake::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/cmake-highlights.scm")],
    &[include_str!("../tree-sitter-queries/cmake-injections.scm")],
    &[],
  )
});
