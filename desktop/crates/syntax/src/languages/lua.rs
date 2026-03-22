use crate::highlighter::{LanguageConfig, build_language_config};
use once_cell::sync::Lazy;

pub static LUA_CONFIG: Lazy<LanguageConfig> = Lazy::new(|| {
  build_language_config(
    "lua",
    tree_sitter_lua::LANGUAGE.into(),
    &[include_str!("../tree-sitter-queries/lua-highlights.scm")],
    &[tree_sitter_lua::INJECTIONS_QUERY],
    &[tree_sitter_lua::LOCALS_QUERY],
  )
});
