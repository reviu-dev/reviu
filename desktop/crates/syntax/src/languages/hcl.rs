use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static HCL_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "hcl",
    tree_sitter_hcl::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/hcl-highlights.scm")],
    &[],
    &[],
  )
});
