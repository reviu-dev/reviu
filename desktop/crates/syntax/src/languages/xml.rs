use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static XML_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "xml",
    tree_sitter_xml::LANGUAGE_XML.into(),
    &[include_str!("../tree-sitter-queries/xml-highlights.scm")],
    &[],
    &[],
  )
});
