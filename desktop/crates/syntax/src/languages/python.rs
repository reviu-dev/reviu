use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static PYTHON_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "python",
    tree_sitter_python::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/python-highlights.scm")],
    &[],
    &[],
  )
});
