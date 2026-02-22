pub mod css;
pub mod dockerfile;
pub mod html;
pub mod json;
pub mod markdown;
pub mod python;
pub mod rust;
pub mod scss;
pub mod toml;
pub mod typescript;
pub mod vue;
pub mod xml;
pub mod yaml;

use crate::highlighter::LanguageConfig;

const EXTENSIONS_XML: &[&str] = &[
  "xml", "svg", "xhtml", "xht", "xsl", "xslt", "xsd", "wsdl", "ares", "axml", "ant", "mxml",
  "plist", "iml", "idea",
];
const EXTENSIONS_PYTHON: &[&str] = &["py", "pyi", "pyw"];
const EXTENSIONS_RUST: &[&str] = &["rs"];
const EXTENSIONS_TYPESCRIPT: &[&str] = &["ts", "cts", "mts", "tsx", "js", "cjs", "mjs", "jsx"];
const EXTENSIONS_YAML: &[&str] = &["yml", "yaml"];
const EXTENSIONS_JSON: &[&str] = &["json", "jsonc"];
const EXTENSIONS_CSS: &[&str] = &["css"];
const EXTENSIONS_SCSS: &[&str] = &["scss"];
const EXTENSIONS_TOML: &[&str] = &["toml"];
const EXTENSIONS_DOCKERFILE: &[&str] = &["dockerfile"];
const EXTENSIONS_HTML: &[&str] = &["html", "htm"];
const EXTENSIONS_MARKDOWN: &[&str] = &["md", "markdown", "mdx"];
const EXTENSIONS_VUE: &[&str] = &["vue"];

pub fn detect_language_config(extension: &str) -> Option<&'static LanguageConfig> {
  let extension = extension
    .trim()
    .trim_start_matches('.')
    .to_ascii_lowercase();
  let extension = extension.as_str();
  match extension {
    _ if EXTENSIONS_CSS.contains(&extension) => Some(&*css::CSS_CONFIG),
    _ if EXTENSIONS_SCSS.contains(&extension) => Some(&*scss::SCSS_CONFIG),
    _ if EXTENSIONS_TOML.contains(&extension) => Some(&*toml::TOML_CONFIG),
    _ if EXTENSIONS_DOCKERFILE.contains(&extension) => Some(&*dockerfile::DOCKERFILE_CONFIG),
    _ if EXTENSIONS_HTML.contains(&extension) => Some(&*html::HTML_CONFIG),
    _ if EXTENSIONS_JSON.contains(&extension) => Some(&*json::JSON_CONFIG),
    _ if EXTENSIONS_MARKDOWN.contains(&extension) => Some(&*markdown::MARKDOWN_CONFIG),
    _ if EXTENSIONS_PYTHON.contains(&extension) => Some(&*python::PYTHON_CONFIG),
    _ if EXTENSIONS_RUST.contains(&extension) => Some(&*rust::RUST_CONFIG),
    _ if EXTENSIONS_TYPESCRIPT.contains(&extension) => Some(&*typescript::TYPESCRIPT_CONFIG),
    _ if EXTENSIONS_VUE.contains(&extension) => Some(&*vue::VUE_CONFIG),
    _ if EXTENSIONS_XML.contains(&extension) => Some(&*xml::XML_CONFIG),
    _ if EXTENSIONS_YAML.contains(&extension) => Some(&*yaml::YAML_CONFIG),
    _ => None,
  }
}

pub fn language_config_for_name(name: &str) -> Option<&'static LanguageConfig> {
  let name = name
    .trim()
    .trim_matches(|c| c == '{' || c == '}')
    .trim_start_matches('.')
    .to_ascii_lowercase();
  match name.as_str() {
    "css" => Some(&*css::CSS_CONFIG),
    "scss" => Some(&*scss::SCSS_CONFIG),
    "toml" => Some(&*toml::TOML_CONFIG),
    "sass" | "less" | "postcss" => Some(&*css::CSS_CONFIG),
    "rust" | "rs" => Some(&*rust::RUST_CONFIG),
    "python" | "python3" | "py" => Some(&*python::PYTHON_CONFIG),
    "dockerfile" | "docker" => Some(&*dockerfile::DOCKERFILE_CONFIG),
    "yaml" | "yml" => Some(&*yaml::YAML_CONFIG),
    "xml" => Some(&*xml::XML_CONFIG),
    "markdown" | "md" | "mdx" => Some(&*markdown::MARKDOWN_CONFIG),
    "html" => Some(&*html::HTML_CONFIG),
    "javascript" | "js" | "typescript" | "ts" | "tsx" | "jsx" => {
      Some(&*typescript::TYPESCRIPT_CONFIG)
    }
    "json" => Some(&*json::JSON_CONFIG),
    "vue" => Some(&*vue::VUE_CONFIG),
    _ => detect_language_config(&name),
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
  fn test_detect_is_case_and_dot_insensitive() {
    assert!(detect_language_config("RS").is_some());
    assert!(detect_language_config(".tsx").is_some());
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
  fn test_detect_scss() {
    assert!(detect_language_config("scss").is_some());
  }

  #[test]
  fn test_detect_toml() {
    assert!(detect_language_config("toml").is_some());
  }

  #[test]
  fn test_detect_dockerfile() {
    assert!(detect_language_config("dockerfile").is_some());
  }

  #[test]
  fn test_detect_python() {
    assert!(detect_language_config("py").is_some());
    assert!(detect_language_config("pyi").is_some());
    assert!(detect_language_config("pyw").is_some());
  }

  #[test]
  fn test_detect_xml() {
    assert!(detect_language_config("xml").is_some());
  }

  #[test]
  fn test_detect_yaml() {
    assert!(detect_language_config("yaml").is_some());
    assert!(detect_language_config("yml").is_some());
  }

  #[test]
  fn test_detect_vue() {
    assert!(detect_language_config("vue").is_some());
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

  #[test]
  fn test_python_config_has_correct_name() {
    let config = detect_language_config("py").unwrap();
    assert_eq!(config.name, "python");
  }

  #[test]
  fn test_xml_config_has_correct_name() {
    let config = detect_language_config("xml").unwrap();
    assert_eq!(config.name, "xml");
  }

  #[test]
  fn test_vue_config_has_correct_name() {
    let config = detect_language_config("vue").unwrap();
    assert_eq!(config.name, "vue");
  }

  #[test]
  fn test_language_config_for_name_supports_code_fence_language_names() {
    let rust = language_config_for_name("rust").unwrap();
    assert_eq!(rust.name, "rust");

    let python = language_config_for_name("python").unwrap();
    assert_eq!(python.name, "python");

    let yaml = language_config_for_name("yml").unwrap();
    assert_eq!(yaml.name, "yaml");
  }
}
