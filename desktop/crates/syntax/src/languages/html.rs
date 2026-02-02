use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static HTML_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_html::LANGUAGE.into();
  let query_source = include_str!("../tree-sitter-queries/html-highlights.scm");

  let mut config =
    HighlightConfiguration::new(language, "html", query_source, "", "")
      .expect("Failed to create HTML highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "html",
    highlight_config: config,
    extensions: &["html", "htm"],
  }
});
