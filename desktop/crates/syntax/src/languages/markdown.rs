use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static MARKDOWN_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_md::LANGUAGE.into();
  let query_source = include_str!("../tree-sitter-queries/markdown-highlights.scm");

  let mut config = HighlightConfiguration::new(language, "markdown", query_source, "", "")
    .expect("Failed to create Markdown highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "markdown",
    highlight_config: config,
  }
});
