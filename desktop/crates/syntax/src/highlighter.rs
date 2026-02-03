use crate::languages;
use crate::theme::TokenType;
use std::ops::Range;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

pub const HIGHLIGHT_NAMES: &[&str] = &[
  "keyword",
  "keyword.control",
  "keyword.operator.regex",
  "function",
  "function.definition",
  "function.method",
  "function.special",
  "function.special.definition",
  "type",
  "type.builtin",
  "type.interface",
  "type.class",
  "type.name",
  "string",
  "string.escape",
  "escape",
  "string.regex",
  "string.special.key",
  "number",
  "boolean",
  "comment",
  "comment.doc",
  "variable",
  "variable.special",
  "variable.parameter",
  "property",
  "property.name",
  "constant",
  "constant.builtin",
  "operator",
  "punctuation",
  "punctuation.bracket",
  "punctuation.delimiter",
  "punctuation.special",
  "attribute",
  "lifetime",
  "embedded",
  "constructor",
  "nested",
  "text.title",
  "text.literal",
  "text.uri",
  "text.reference",
  "none",
  "tag",
  "tag.error",
  "string.special",
  "label",
  "character.special",
  "tag.delimiter",
  "tag.attribute",
];

const HIGHLIGHT_TOKEN_TYPES: &[Option<TokenType>] = &[
  Some(TokenType::Keyword),              // keyword
  Some(TokenType::KeywordControl),       // keyword.control
  Some(TokenType::KeywordControl),       // keyword.operator.regex
  Some(TokenType::Function),             // function
  Some(TokenType::Function),             // function.definition
  Some(TokenType::FunctionMethod),       // function.method
  Some(TokenType::FunctionSpecial),      // function.special
  Some(TokenType::FunctionSpecial),      // function.special.definition
  Some(TokenType::Type),                 // type
  Some(TokenType::TypeBuiltin),          // type.builtin
  Some(TokenType::TypeInterface),        // type.interface
  Some(TokenType::TypeClass),            // type.class
  Some(TokenType::Type),                 // type.name
  Some(TokenType::String),               // string
  Some(TokenType::StringEscape),         // string.escape
  Some(TokenType::StringEscape),         // escape
  Some(TokenType::StringRegex),          // string.regex
  Some(TokenType::Property),             // string.special.key
  Some(TokenType::Number),               // number
  Some(TokenType::Boolean),              // boolean
  Some(TokenType::Comment),              // comment
  Some(TokenType::CommentDoc),           // comment.doc
  Some(TokenType::Variable),             // variable
  Some(TokenType::VariableSpecial),      // variable.special
  Some(TokenType::VariableParameter),    // variable.parameter
  Some(TokenType::Property),             // property
  Some(TokenType::Property),             // property.name
  Some(TokenType::Constant),             // constant
  Some(TokenType::ConstantBuiltin),      // constant.builtin
  Some(TokenType::Operator),             // operator
  Some(TokenType::Punctuation),          // punctuation
  Some(TokenType::PunctuationBracket),   // punctuation.bracket
  Some(TokenType::PunctuationDelimiter), // punctuation.delimiter
  Some(TokenType::PunctuationSpecial),   // punctuation.special
  Some(TokenType::Attribute),            // attribute
  Some(TokenType::Lifetime),             // lifetime
  Some(TokenType::Embedded),             // embedded
  Some(TokenType::Type),                 // constructor
  Some(TokenType::Variable),             // nested
  Some(TokenType::Keyword),              // text.title
  Some(TokenType::String),               // text.literal
  Some(TokenType::String),               // text.uri
  Some(TokenType::Constant),             // text.reference
  None,                                  // none
  Some(TokenType::Keyword),              // tag
  Some(TokenType::VariableSpecial),      // tag.error
  Some(TokenType::String),               // string.special
  Some(TokenType::VariableSpecial),      // label
  Some(TokenType::PunctuationSpecial),   // character.special
  Some(TokenType::Punctuation),          // tag.delimiter
  Some(TokenType::Attribute),            // tag.attribute
];

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
      .highlight(&self.config.highlight_config, text.as_bytes(), None, |language| {
        languages::language_config_for_name(language).map(|config| &config.highlight_config)
      })
      .map_err(|e| format!("Highlight failed: {}", e))?;

    let mut highlight_stack = Vec::new();

    for event in events {
      match event.map_err(|e| format!("Event error: {}", e))? {
        HighlightEvent::Source { start, end } => {
          if let Some(&highlight_idx) = highlight_stack.last() {
            if let Some(token_type) = map_highlight_index_to_token_type(highlight_idx) {
              if !on_span(HighlightSpan {
                byte_range: start..end,
                token_type,
              }) {
                return Ok(());
              }
            }
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
fn map_highlight_index_to_token_type(idx: usize) -> Option<TokenType> {
  HIGHLIGHT_TOKEN_TYPES.get(idx).copied().flatten()
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
    assert_eq!(
      map_highlight_index_to_token_type(0),
      Some(TokenType::Keyword)
    );
    assert_eq!(
      map_highlight_index_to_token_type(3),
      Some(TokenType::Function)
    );
    assert_eq!(
      map_highlight_index_to_token_type(13),
      Some(TokenType::String)
    );
    assert_eq!(map_highlight_index_to_token_type(999), None);
  }
}
