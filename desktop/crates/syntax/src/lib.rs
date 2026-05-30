mod highlighter;
pub mod languages;
pub mod runs;
mod theme;

pub use highlighter::{HighlightSpan, LanguageConfig, SyntaxHighlighter};
pub use runs::{
  clamp_to_char_boundary, compute_line_bounds, highlight_text_to_line_spans,
  highlights_to_text_runs, line_index_for_byte,
};
pub use theme::{SyntaxTheme, TokenType};
