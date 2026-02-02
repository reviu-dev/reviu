use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static RUST_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_rust::LANGUAGE.into();
  let query_source = include_str!("../tree-sitter-queries/rust-highlights.scm");

  let mut config = HighlightConfiguration::new(
    language,
    "rust",
    query_source,
    "", // injections query
    "", // locals query
  )
  .expect("Failed to create Rust highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "rust",
    highlight_config: config,
    extensions: &["rs"],
  }
});
