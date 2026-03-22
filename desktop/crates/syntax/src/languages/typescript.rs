use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static TYPESCRIPT_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "typescript",
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    &[include_str!(
      "../tree-sitter-queries/typescript-highlights.scm"
    )],
    &[],
    &[],
  )
});
