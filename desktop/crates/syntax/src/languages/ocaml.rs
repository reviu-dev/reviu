use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static OCAML_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "ocaml",
    tree_sitter_ocaml::LANGUAGE_OCAML.into(),
    &[tree_sitter_ocaml::HIGHLIGHTS_QUERY],
    &[],
    &[tree_sitter_ocaml::LOCALS_QUERY],
  )
});

pub static OCAML_INTERFACE_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "ocaml",
    tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into(),
    &[include_str!(
      "../tree-sitter-queries/ocaml-interface-highlights.scm"
    )],
    &[],
    &[tree_sitter_ocaml::LOCALS_QUERY],
  )
});
