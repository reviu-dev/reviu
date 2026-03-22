use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static CSHARP_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "csharp",
    tree_sitter_c_sharp::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/csharp-highlights.scm")],
    &[],
    &[],
  )
});
