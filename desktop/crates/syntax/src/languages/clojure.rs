use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static CLOJURE_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "clojure",
    tree_sitter_clojure_orchard::LANGUAGE.into(),
    &[include_str!(
      "../tree-sitter-queries/clojure-highlights.scm"
    )],
    &[],
    &[],
  )
});
