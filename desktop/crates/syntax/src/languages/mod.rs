pub mod css;
pub mod dockerfile;
pub mod html;
pub mod json;
pub mod markdown;
pub mod rust;
pub mod typescript;
pub mod yaml;

use crate::highlighter::LanguageConfig;

pub fn detect_language_config(extension: &str) -> Option<&'static LanguageConfig> {
  match extension {
    "css" => Some(&*css::CSS_CONFIG),
    "dockerfile" => Some(&*dockerfile::DOCKERFILE_CONFIG),
    "html" | "htm" => Some(&*html::HTML_CONFIG),
    "json" => Some(&*json::JSON_CONFIG),
    "md" | "markdown" => Some(&*markdown::MARKDOWN_CONFIG),
    "rs" => Some(&*rust::RUST_CONFIG),
    "ts" | "tsx" | "js" | "jsx" => Some(&*typescript::TYPESCRIPT_CONFIG),
    "yml" | "yaml" => Some(&*yaml::YAML_CONFIG),
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
  fn test_detect_json() {
    assert!(detect_language_config("json").is_some());
  }

  #[test]
  fn test_detect_markdown() {
    assert!(detect_language_config("md").is_some());
    assert!(detect_language_config("markdown").is_some());
  }

  #[test]
  fn test_detect_html() {
    assert!(detect_language_config("html").is_some());
    assert!(detect_language_config("htm").is_some());
  }

  #[test]
  fn test_detect_css() {
    assert!(detect_language_config("css").is_some());
  }

  #[test]
  fn test_detect_dockerfile() {
    assert!(detect_language_config("dockerfile").is_some());
  }

  #[test]
  fn test_detect_yaml() {
    assert!(detect_language_config("yaml").is_some());
    assert!(detect_language_config("yml").is_some());
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
