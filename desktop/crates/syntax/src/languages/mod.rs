pub mod rust;
pub mod typescript;

use crate::highlighter::LanguageConfig;

pub fn detect_language_config(extension: &str) -> Option<&'static LanguageConfig> {
  match extension {
    "rs" => Some(&*rust::RUST_CONFIG),
    "ts" | "tsx" | "js" | "jsx" => Some(&*typescript::TYPESCRIPT_CONFIG),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_detect_rust() {
    assert!(detect_language_config("rs").is_some());
  }

  #[test]
  fn test_detect_typescript() {
    assert!(detect_language_config("ts").is_some());
    assert!(detect_language_config("tsx").is_some());
  }

  #[test]
  fn test_detect_javascript() {
    assert!(detect_language_config("js").is_some());
    assert!(detect_language_config("jsx").is_some());
  }

  #[test]
  fn test_detect_unknown() {
    assert!(detect_language_config("unknown").is_none());
    assert!(detect_language_config("").is_none());
  }

  #[test]
  fn test_rust_config_has_correct_name() {
    let config = detect_language_config("rs").unwrap();
    assert_eq!(config.name, "rust");
  }

  #[test]
  fn test_typescript_config_has_correct_name() {
    let config = detect_language_config("ts").unwrap();
    assert_eq!(config.name, "typescript");
  }
}
