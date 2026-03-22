use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static JAVA_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "java",
    tree_sitter_java::LANGUAGE.into(),
    &[tree_sitter_java::HIGHLIGHTS_QUERY],
    &[],
    &[],
  )
});
