use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

unsafe extern "C" {
  fn tree_sitter_dart_orchard() -> tree_sitter::Language;
}

pub static DART_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "dart",
    unsafe { tree_sitter_dart_orchard() },
    &[include_str!("../tree-sitter-queries/dart-highlights.scm")],
    &[],
    &[],
  )
});
