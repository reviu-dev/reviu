use crate::highlighter::{HIGHLIGHT_NAMES, LanguageConfig};
use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

pub static XML_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  let language = tree_sitter_xml::LANGUAGE_XML.into();
  let query_source = include_str!("../tree-sitter-queries/xml-highlights.scm");

  let mut config = HighlightConfiguration::new(language, "xml", query_source, "", "")
    .expect("Failed to create XML highlight config");

  config.configure(HIGHLIGHT_NAMES);

  LanguageConfig {
    name: "xml",
    highlight_config: config,
  }
});
