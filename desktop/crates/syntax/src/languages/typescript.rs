use crate::highlighter::LanguageConfig;
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static TYPESCRIPT_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
  let query_source = include_str!("../tree-sitter-queries/typescript-highlights.scm");

  let highlight_names = vec![
    "keyword",
    "keyword.control",
    "function",
    "function.method",
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
  ];

  let mut config = HighlightConfiguration::new(language, "typescript", query_source, "", "")
    .expect("Failed to create TypeScript highlight config");

  config.configure(&highlight_names);

  LanguageConfig {
    name: "typescript",
    highlight_config: config,
    extensions: &["ts", "tsx", "js", "jsx"],
  }
});
