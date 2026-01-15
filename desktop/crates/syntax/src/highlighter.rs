use crate::theme::TokenType;
use std::ops::Range;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Highlight span with token type
#[derive(Clone, Debug)]
pub struct HighlightSpan {
  pub byte_range: Range<usize>,
  pub token_type: TokenType,
}

/// Language configuration
pub struct LanguageConfig {
  pub name: &'static str,
  pub highlight_config: HighlightConfiguration,
  pub extensions: &'static [&'static str],
}

/// Syntax highlighting manager
pub struct SyntaxHighlighter {
  highlighter: Highlighter,
  pub config: &'static LanguageConfig,
}

impl SyntaxHighlighter {
  pub fn new(config: &'static LanguageConfig) -> Self {
    Self {
      highlighter: Highlighter::new(),
      config,
    }
  }

  /// Highlight complete text
  /// Returns Ok(highlights) or Err if parsing fails
  pub fn highlight_text(&mut self, text: &str) -> Result<Vec<HighlightSpan>, String> {
    let mut highlights = Vec::new();
    self.highlight_text_stream(
      text,
      |_| true,
      |span| {
        highlights.push(span);
        true
      },
    )?;
    Ok(highlights)
  }

  /// Stream highlight events for incremental processing.
  /// Return `false` from callbacks to cancel early.
  pub fn highlight_text_stream<F, G>(
    &mut self,
    text: &str,
    mut on_source: F,
    mut on_span: G,
  ) -> Result<(), String>
  where
    F: FnMut(Range<usize>) -> bool,
    G: FnMut(HighlightSpan) -> bool,
  {
    let events = self
      .highlighter
      .highlight(&self.config.highlight_config, text.as_bytes(), None, |_| {
        None
      })
      .map_err(|e| format!("Highlight failed: {}", e))?;

    let mut highlight_stack = Vec::new();

    for event in events {
      match event.map_err(|e| format!("Event error: {}", e))? {
        HighlightEvent::Source { start, end } => {
          if let Some(&highlight_idx) = highlight_stack.last()
            && !on_span(HighlightSpan {
              byte_range: start..end,
              token_type: map_highlight_index_to_token_type(highlight_idx),
            })
          {
            return Ok(());
          }
          if !on_source(start..end) {
            return Ok(());
          }
        }
        HighlightEvent::HighlightStart(idx) => {
          highlight_stack.push(idx.0);
        }
        HighlightEvent::HighlightEnd => {
          highlight_stack.pop();
        }
      }
    }

    Ok(())
  }
}

/// Map highlight index to TokenType
fn map_highlight_index_to_token_type(idx: usize) -> TokenType {
  // Indices correspond to the order in highlight_names of HighlightConfiguration
  // See languages/rust.rs for the list of names
  match idx {
    0 => TokenType::Keyword,             // keyword
    1 => TokenType::KeywordControl,      // keyword.control
    2 => TokenType::Function,            // function
    3 => TokenType::FunctionMethod,      // function.method
    4 => TokenType::FunctionSpecial,     // function.macro
    5 => TokenType::Type,                // type
    6 => TokenType::TypeBuiltin,         // type.builtin
    7 => TokenType::String,              // string
    8 => TokenType::StringEscape,        // string.escape
    9 => TokenType::Number,              // number
    10 => TokenType::Comment,            // comment
    11 => TokenType::Variable,           // variable
    12 => TokenType::Property,           // property
    13 => TokenType::Constant,           // constant
    14 => TokenType::Operator,           // operator
    15 => TokenType::PunctuationBracket, // punctuation.bracket
    16 => TokenType::Attribute,          // attribute
    17 => TokenType::Lifetime,           // lifetime
    _ => TokenType::Variable,            // fallback
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::languages::rust::RUST_CONFIG;

  #[test]
  fn test_highlight_simple_rust() {
    let mut highlighter = SyntaxHighlighter::new(&RUST_CONFIG);
    let result = highlighter.highlight_text("fn main() {}");

    assert!(result.is_ok());
    let highlights = result.unwrap();
    assert!(!highlights.is_empty());
  }

  #[test]
  fn test_highlight_keyword() {
    let mut highlighter = SyntaxHighlighter::new(&RUST_CONFIG);
    let result = highlighter.highlight_text("fn");

    assert!(result.is_ok());
    let highlights = result.unwrap();

    // "fn" should be highlighted as keyword
    assert!(
      highlights
        .iter()
        .any(|h| matches!(h.token_type, TokenType::Keyword))
    );
  }

  #[test]
  fn test_highlight_string() {
    let mut highlighter = SyntaxHighlighter::new(&RUST_CONFIG);
    let result = highlighter.highlight_text(r#"let s = "hello";"#);

    assert!(result.is_ok());
    let highlights = result.unwrap();

    // Should have a String token
    assert!(highlights.iter().any(|h| h.token_type == TokenType::String));
  }

  #[test]
  fn test_highlight_comment() {
    let mut highlighter = SyntaxHighlighter::new(&RUST_CONFIG);
    let result = highlighter.highlight_text("// comment");

    assert!(result.is_ok());
    let highlights = result.unwrap();
    assert!(
      highlights
        .iter()
        .any(|h| h.token_type == TokenType::Comment)
    );
  }

  #[test]
  fn test_highlight_empty_text() {
    let mut highlighter = SyntaxHighlighter::new(&RUST_CONFIG);
    let result = highlighter.highlight_text("");

    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
  }

  #[test]
  fn test_highlight_invalid_syntax_doesnt_panic() {
    let mut highlighter = SyntaxHighlighter::new(&RUST_CONFIG);
    // Tree-sitter should handle invalid syntax gracefully
    let result = highlighter.highlight_text("fn {{{");

    // Should return a result (even with parse error)
    assert!(result.is_ok() || result.is_err());
  }

  #[test]
  fn test_map_highlight_indices() {
    // Verify that all indices map correctly
    assert_eq!(map_highlight_index_to_token_type(0), TokenType::Keyword);
    assert_eq!(map_highlight_index_to_token_type(2), TokenType::Function);
    assert_eq!(map_highlight_index_to_token_type(7), TokenType::String);
    assert_eq!(map_highlight_index_to_token_type(999), TokenType::Variable); // fallback
  }
}
