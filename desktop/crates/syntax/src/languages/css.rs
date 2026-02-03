use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static CSS_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_css::LANGUAGE.into();
  let query_source = include_str!("../tree-sitter-queries/css-highlights.scm");

  let mut config = HighlightConfiguration::new(language, "css", query_source, "", "")
    .expect("Failed to create CSS highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "css",
    highlight_config: config,
  }
});
