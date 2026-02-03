use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static VUE_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_vue::LANGUAGE.into();
  let html_highlights = include_str!("../tree-sitter-queries/html-highlights.scm");
  let vue_highlights = include_str!("../tree-sitter-queries/vue/highlights.scm");
  let highlights_query = format!("{}\n{}", html_highlights, vue_highlights);

  let html_tags_injections = include_str!("../tree-sitter-queries/vue/html_tags/injections.scm");
  let vue_injections = include_str!("../tree-sitter-queries/vue/injections.scm");
  let injections_query = format!("{}\n{}", html_tags_injections, vue_injections);

  let mut config = HighlightConfiguration::new(
    language,
    "vue",
    &highlights_query,
    &injections_query,
    "",
  )
  .expect("Failed to create Vue highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "vue",
    highlight_config: config,
  }
});
