mod highlighter;
pub mod languages;
mod theme;

pub use highlighter::{HighlightSpan, LanguageConfig, SyntaxHighlighter};
pub use theme::{SyntaxTheme, Theme, TokenType};
