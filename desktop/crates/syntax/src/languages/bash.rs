use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static BASH_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "bash",
    tree_sitter_bash::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/bash-highlights.scm")],
    &[],
    &[],
  )
});
