use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static PHP_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "php",
    tree_sitter_php::LANGUAGE_PHP.into(),
    &[tree_sitter_php::HIGHLIGHTS_QUERY],
    &[include_str!("../tree-sitter-queries/php-injections.scm")],
    &[],
  )
});
