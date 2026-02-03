use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_dockerfile as _;
use tree_sitter_highlight::HighlightConfiguration;

unsafe extern "C" {
  fn tree_sitter_dockerfile() -> tree_sitter::Language;
}

pub static DOCKERFILE_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = unsafe { tree_sitter_dockerfile() };
  let query_source = include_str!("../tree-sitter-queries/dockerfile-highlights.scm");

  let mut config = HighlightConfiguration::new(language, "dockerfile", query_source, "", "")
    .expect("Failed to create Dockerfile highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "dockerfile",
    highlight_config: config,
  }
});
