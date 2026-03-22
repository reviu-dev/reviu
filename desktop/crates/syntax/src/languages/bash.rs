use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static BASH_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_bash::LANGUAGE.into();
  let query_source = include_str!("../tree-sitter-queries/bash-highlights.scm");

  let mut config = HighlightConfiguration::new(language, "bash", query_source, "", "")
    .expect("Failed to create Bash highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "bash",
    highlight_config: config,
  }
});
