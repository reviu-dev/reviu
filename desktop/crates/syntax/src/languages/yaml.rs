use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;
use tree_sitter_yaml as _;

unsafe extern "C" {
  fn tree_sitter_yaml() -> tree_sitter::Language;
}

pub static YAML_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "yaml",
    unsafe { tree_sitter_yaml() },
    &[include_str!("../tree-sitter-queries/yaml-highlights.scm")],
    &[],
    &[],
  )
});
