use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static TOML_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_toml_ng::LANGUAGE.into();
  let query_source = include_str!("../tree-sitter-queries/toml-highlights.scm");

  let mut config = HighlightConfiguration::new(language, "toml", query_source, "", "")
    .expect("Failed to create TOML highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "toml",
    highlight_config: config,
  }
});
