use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static HTML_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "html",
    tree_sitter_html::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/html-highlights.scm")],
    &[],
    &[],
  )
});
