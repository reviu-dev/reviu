use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static PYTHON_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_python::LANGUAGE.into();
  let query_source = include_str!("../tree-sitter-queries/python-highlights.scm");

  let mut config =
    HighlightConfiguration::new(language, "python", query_source, "", "")
      .expect("Failed to create Python highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "python",
    highlight_config: config,
    extensions: &["py", "pyi", "pyw"],
  }
});
