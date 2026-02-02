use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;
use tree_sitter_yaml as _;

unsafe extern "C" {
  fn tree_sitter_yaml() -> tree_sitter::Language;
}

pub static YAML_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = unsafe { tree_sitter_yaml() };
  let query_source = include_str!("../tree-sitter-queries/yaml-highlights.scm");

  let mut config =
    HighlightConfiguration::new(language, "yaml", query_source, "", "")
      .expect("Failed to create YAML highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "yaml",
    highlight_config: config,
    extensions: &["yml", "yaml"],
  }
});
