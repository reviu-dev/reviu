use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static CPP_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  // tree-sitter-cpp only ships C++-specific highlight additions.
  // We need the base C query as well for normal standalone C++ files.
  build_language_config(
    "cpp",
    tree_sitter_cpp::LANGUAGE.into(),
    &[
      tree_sitter_c::HIGHLIGHT_QUERY,
      tree_sitter_cpp::HIGHLIGHT_QUERY,
    ],
    &[],
    &[],
  )
});
