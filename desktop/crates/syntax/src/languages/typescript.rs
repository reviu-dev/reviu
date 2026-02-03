use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static TYPESCRIPT_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
  let query_source = include_str!("../tree-sitter-queries/typescript-highlights.scm");

  let mut config = HighlightConfiguration::new(language, "typescript", query_source, "", "")
    .expect("Failed to create TypeScript highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "typescript",
    highlight_config: config,
  }
});
