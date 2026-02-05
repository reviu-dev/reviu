use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static SCSS_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_scss::language().into();
  let query_source = include_str!("../tree-sitter-queries/scss-highlights.scm");

  let mut config = HighlightConfiguration::new(language, "scss", query_source, "", "")
    .expect("Failed to create SCSS highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "scss",
    highlight_config: config,
  }
});
