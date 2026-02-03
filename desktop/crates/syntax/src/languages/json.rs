use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static JSON_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_json::LANGUAGE.into();
  let query_source = include_str!("../tree-sitter-queries/json-highlights.scm");

  let mut config = HighlightConfiguration::new(language, "json", query_source, "", "")
    .expect("Failed to create JSON highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "json",
    highlight_config: config,
  }
});
