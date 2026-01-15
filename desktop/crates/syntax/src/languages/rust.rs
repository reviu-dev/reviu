use crate::highlighter::LanguageConfig;
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static RUST_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_rust::LANGUAGE.into();
  let query_source = include_str!("../tree-sitter-queries/rust-highlights.scm");

  let highlight_names = vec![
    "keyword",
    "keyword.control",
    "function",
    "function.method",
    "function.macro",
    "type",
    "type.builtin",
    "string",
    "string.escape",
    "number",
    "comment",
    "variable",
    "property",
    "constant",
    "operator",
    "punctuation.bracket",
    "attribute",
    "lifetime",
  ];

  let mut config = HighlightConfiguration::new(
    language,
    "rust",
    query_source,
    "", // injections query
    "", // locals query
  )
  .expect("Failed to create Rust highlight config");

  config.configure(&highlight_names);

  LanguageConfig {
    name: "rust",
    highlight_config: config,
    extensions: &["rs"],
  }
});
