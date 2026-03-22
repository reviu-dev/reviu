use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static VUE_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "vue",
    tree_sitter_vue::LANGUAGE.into(),
    &[
      include_str!("../tree-sitter-queries/html-highlights.scm"),
      include_str!("../tree-sitter-queries/vue/highlights.scm"),
    ],
    &[
      include_str!("../tree-sitter-queries/vue/html_tags/injections.scm"),
      include_str!("../tree-sitter-queries/vue/injections.scm"),
    ],
    &[],
  )
});
