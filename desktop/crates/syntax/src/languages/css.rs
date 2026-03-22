use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static CSS_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "css",
    tree_sitter_css::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/css-highlights.scm")],
    &[],
    &[],
  )
});
