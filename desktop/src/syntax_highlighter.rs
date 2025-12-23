//! Syntax highlighting using tree-sitter
//! Provides language detection and syntax highlighting for diff views

use gpui::{Hsla, TextRun};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedLanguage {
  Rust,
  TypeScript,
  JavaScript,
  Python,
  Go,
}

impl SupportedLanguage {
  /// Detect language from file extension
  pub fn from_path(path: &Path) -> Option<Self> {
    let extension = path.extension()?.to_str()?;
    match extension {
      "rs" => Some(Self::Rust),
      "ts" => Some(Self::TypeScript),
      "tsx" => Some(Self::TypeScript),
      "js" => Some(Self::JavaScript),
      "jsx" => Some(Self::JavaScript),
      "py" => Some(Self::Python),
      "go" => Some(Self::Go),
      _ => None,
    }
  }

  /// Get the tree-sitter language for this language
  fn tree_sitter_language(&self) -> Language {
    match self {
      Self::Rust => tree_sitter_rust::LANGUAGE.into(),
      Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
      Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
      Self::Python => tree_sitter_python::LANGUAGE.into(),
      Self::Go => tree_sitter_go::LANGUAGE.into(),
    }
  }

  /// Get the highlight query for this language
  fn highlight_query(&self) -> &'static str {
    match self {
      Self::Rust => include_str!("../tree_sitter_queries/rust_highlights.scm"),
      Self::TypeScript => include_str!("../tree_sitter_queries/typescript_highlights.scm"),
      Self::JavaScript => include_str!("../tree_sitter_queries/javascript_highlights.scm"),
      Self::Python => include_str!("../tree_sitter_queries/python_highlights.scm"),
      Self::Go => include_str!("../tree_sitter_queries/go_highlights.scm"),
    }
  }
}

/// Syntax highlighter for code in diffs
pub struct SyntaxHighlighter {
  _language: SupportedLanguage,
  parser: Parser,
  query: Query,
}

impl SyntaxHighlighter {
  /// Create a new syntax highlighter for the given language
  pub fn new(language: SupportedLanguage) -> anyhow::Result<Self> {
    let ts_language = language.tree_sitter_language();
    let mut parser = Parser::new();
    parser.set_language(&ts_language)?;

    let query = Query::new(&ts_language, language.highlight_query())?;

    Ok(Self {
      _language: language,
      parser,
      query,
    })
  }

  /// Highlight a single line of code and return text runs
  /// Returns owned Vec of TextRun to avoid lifetime issues
  pub fn highlight_line(&mut self, line: &str, default_color: Hsla) -> Vec<TextRun> {
    // Parse the line
    let tree = match self.parser.parse(line, None) {
      Some(tree) => tree,
      None => {
        // Parsing failed, return default
        return vec![TextRun {
          len: line.len(),
          color: default_color,
          ..Default::default()
        }];
      }
    };

    let root_node = tree.root_node();
    let mut cursor = QueryCursor::new();

    // Collect all captures with their byte ranges
    let mut highlights: Vec<(usize, usize, String)> = Vec::new();

    let mut captures = cursor.captures(&self.query, root_node, line.as_bytes());
    while let Some((match_, _capture_idx)) = captures.next() {
      for capture in match_.captures {
        let start = capture.node.start_byte();
        let end = capture.node.end_byte();
        let capture_name = self.query.capture_names()[capture.index as usize].to_string();
        highlights.push((start, end, capture_name));
      }
    }

    // Sort by start position
    highlights.sort_by_key(|(start, _, _)| *start);

    if highlights.is_empty() {
      // No highlights found, return default
      return vec![TextRun {
        len: line.len(),
        color: default_color,
        ..Default::default()
      }];
    }

    // Build text runs from highlights
    let mut runs = Vec::new();
    let mut current_pos = 0;

    for (start, end, capture_name) in &highlights {
      // Skip overlapping highlights
      if *start < current_pos {
        continue;
      }

      // Add unhighlighted text before this capture
      if *start > current_pos {
        runs.push(TextRun {
          len: start - current_pos,
          color: default_color,
          ..Default::default()
        });
      }

      // Add highlighted text
      let color = Self::color_for_capture(capture_name, default_color);
      runs.push(TextRun {
        len: end - start,
        color,
        ..Default::default()
      });

      current_pos = *end;
    }

    // Add remaining unhighlighted text
    if current_pos < line.len() {
      runs.push(TextRun {
        len: line.len() - current_pos,
        color: default_color,
        ..Default::default()
      });
    }

    runs
  }

  /// Map tree-sitter capture names to colors
  /// Based on common naming conventions used in tree-sitter queries
  fn color_for_capture(capture_name: &str, default_color: Hsla) -> Hsla {
    match capture_name {
      // Keywords
      "keyword" | "keyword.control" | "keyword.function" | "keyword.return"
      | "keyword.operator" | "keyword.import" | "keyword.storage" => Hsla {
        h: 280.0 / 360.0,
        s: 0.6,
        l: 0.65,
        a: 1.0,
      },

      // Strings
      "string" | "string.special" | "string.escape" => Hsla {
        h: 120.0 / 360.0,
        s: 0.5,
        l: 0.55,
        a: 1.0,
      },

      // Comments
      "comment" | "comment.line" | "comment.block" => Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.45,
        a: 1.0,
      },

      // Functions
      "function" | "function.method" | "function.call" | "function.builtin" | "method"
      | "method.call" => Hsla {
        h: 210.0 / 360.0,
        s: 0.7,
        l: 0.6,
        a: 1.0,
      },

      // Types
      "type" | "type.builtin" | "type.definition" | "class" | "struct" | "enum" | "interface" => {
        Hsla {
          h: 180.0 / 360.0,
          s: 0.6,
          l: 0.6,
          a: 1.0,
        }
      }

      // Variables
      "variable" | "variable.parameter" | "variable.builtin" | "parameter" => Hsla {
        h: 200.0 / 360.0,
        s: 0.4,
        l: 0.7,
        a: 1.0,
      },

      // Constants
      "constant" | "constant.builtin" | "boolean" | "number" => Hsla {
        h: 30.0 / 360.0,
        s: 0.7,
        l: 0.6,
        a: 1.0,
      },

      // Properties
      "property" | "attribute" | "field" => Hsla {
        h: 340.0 / 360.0,
        s: 0.6,
        l: 0.65,
        a: 1.0,
      },

      // Operators
      "operator" | "punctuation" | "punctuation.bracket" | "punctuation.delimiter" => default_color,

      // Fallback
      _ => default_color,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_language_detection() {
    assert_eq!(
      SupportedLanguage::from_path(Path::new("main.rs")),
      Some(SupportedLanguage::Rust)
    );
    assert_eq!(
      SupportedLanguage::from_path(Path::new("app.ts")),
      Some(SupportedLanguage::TypeScript)
    );
    assert_eq!(
      SupportedLanguage::from_path(Path::new("script.py")),
      Some(SupportedLanguage::Python)
    );
    assert_eq!(SupportedLanguage::from_path(Path::new("unknown.txt")), None);
  }

  #[test]
  fn test_highlighter_creation() {
    let highlighter = SyntaxHighlighter::new(SupportedLanguage::Rust);
    assert!(highlighter.is_ok());
  }

  #[test]
  fn test_highlighting() {
    let mut highlighter = SyntaxHighlighter::new(SupportedLanguage::Rust).unwrap();
    let default_color = Hsla {
      h: 0.0,
      s: 0.0,
      l: 1.0,
      a: 1.0,
    };

    // Test highlighting a simple Rust line
    let runs = highlighter.highlight_line("fn main() {", default_color);

    // Should have multiple runs (fn keyword, main identifier, etc.)
    assert!(runs.len() > 1);
  }
}
