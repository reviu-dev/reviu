pub mod astro;
pub mod bash;
pub mod c;
pub mod cmake;
pub mod cpp;
pub mod csharp;
pub mod css;
pub mod dart;
pub mod dockerfile;
pub mod elixir;
pub mod go;
pub mod hcl;
pub mod html;
pub mod java;
pub mod json;
pub mod julia;
pub mod kotlin;
pub mod lua;
pub mod make;
pub mod markdown;
pub mod php;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod scala;
pub mod scss;
pub mod sql;
pub mod svelte;
pub mod swift;
pub mod toml;
pub mod typescript;
pub mod vue;
pub mod xml;
pub mod yaml;
pub mod zig;

use crate::highlighter::LanguageConfig;
use std::path::Path;

struct LanguageRegistration {
  load: fn() -> &'static LanguageConfig,
  aliases: &'static [&'static str],
  extensions: &'static [&'static str],
  file_names: &'static [&'static str],
  file_name_prefixes: &'static [&'static str],
}

const LANGUAGE_REGISTRATIONS: &[LanguageRegistration] = &[
  LanguageRegistration {
    load: astro_config,
    aliases: &["astro"],
    extensions: &["astro"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: bash_config,
    aliases: &["bash", "sh", "shell", "zsh"],
    extensions: &["sh", "bash", "zsh"],
    file_names: &[
      ".bashrc",
      ".bash_profile",
      ".bash_login",
      ".bash_logout",
      ".profile",
      ".zshrc",
      ".zprofile",
      ".zshenv",
      ".zlogin",
      ".zlogout",
      ".envrc",
    ],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: c_config,
    aliases: &["c"],
    extensions: &["c", "h"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: cmake_config,
    aliases: &["cmake"],
    extensions: &["cmake"],
    file_names: &["cmakelists.txt"],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: cpp_config,
    aliases: &["cpp", "c++", "cplusplus"],
    extensions: &[
      "cpp", "cc", "cxx", "c++", "cp", "hpp", "hh", "hxx", "h++", "ipp", "inl", "tpp", "ixx",
      "cppm", "ccm", "cxxm",
    ],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: csharp_config,
    aliases: &["csharp", "c#", "cs"],
    extensions: &["cs", "csx"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: css_config,
    aliases: &["css", "sass", "less", "postcss"],
    extensions: &["css"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: dart_config,
    aliases: &["dart"],
    extensions: &["dart"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: elixir_config,
    aliases: &["elixir", "ex", "exs"],
    extensions: &["ex", "exs"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: go_config,
    aliases: &["go"],
    extensions: &["go"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: hcl_config,
    aliases: &["hcl", "terraform", "tf", "tfvars"],
    extensions: &["hcl", "tf", "tfvars"],
    file_names: &[".terraformrc", "terraform.rc"],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: scss_config,
    aliases: &["scss"],
    extensions: &["scss"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: sql_config,
    aliases: &["sql"],
    extensions: &["sql"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: svelte_config,
    aliases: &["svelte"],
    extensions: &["svelte"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: swift_config,
    aliases: &["swift"],
    extensions: &["swift"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: toml_config,
    aliases: &["toml"],
    extensions: &["toml"],
    file_names: &["cargo.lock", "poetry.lock", "uv.lock"],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: dockerfile_config,
    aliases: &["dockerfile", "docker"],
    extensions: &["dockerfile"],
    file_names: &[],
    file_name_prefixes: &["dockerfile"],
  },
  LanguageRegistration {
    load: html_config,
    aliases: &["html"],
    extensions: &["html", "htm"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: java_config,
    aliases: &["java"],
    extensions: &["java"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: julia_config,
    aliases: &["julia", "jl"],
    extensions: &["jl"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: kotlin_config,
    aliases: &["kotlin", "kt", "kts"],
    extensions: &["kt", "kts"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: lua_config,
    aliases: &["lua"],
    extensions: &["lua"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: make_config,
    aliases: &["make", "makefile"],
    extensions: &["mk", "mak"],
    file_names: &["makefile", "gnumakefile"],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: json_config,
    aliases: &["json", "jsonc"],
    extensions: &["json", "jsonc"],
    file_names: &["composer.lock", "pipfile.lock"],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: markdown_config,
    aliases: &["markdown", "md", "mdx"],
    extensions: &["md", "markdown", "mdx"],
    file_names: &["readme", "changelog"],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: php_config,
    aliases: &["php", "php3", "php4", "php5", "phtml"],
    extensions: &["php", "php3", "php4", "php5", "phtml"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: python_config,
    aliases: &["python", "python3", "py"],
    extensions: &["py", "pyi", "pyw"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: ruby_config,
    aliases: &["ruby", "rb"],
    extensions: &["rb"],
    file_names: &[
      ".irbrc",
      ".pryrc",
      ".ruby-version",
      "appraisals",
      "berksfile",
      "brewfile",
      "capfile",
      "cheffile",
      "dangerfile",
      "fastfile",
      "gemfile",
      "guardfile",
      "podfile",
      "rakefile",
      "thorfile",
      "vagrantfile",
    ],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: scala_config,
    aliases: &["scala"],
    extensions: &["scala", "sbt", "sc"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: rust_config,
    aliases: &["rust", "rs"],
    extensions: &["rs"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: typescript_config,
    aliases: &["javascript", "js", "typescript", "ts", "tsx", "jsx"],
    extensions: &["ts", "cts", "mts", "tsx", "js", "cjs", "mjs", "jsx"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: vue_config,
    aliases: &["vue"],
    extensions: &["vue"],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: xml_config,
    aliases: &["xml"],
    extensions: &[
      "xml", "svg", "xhtml", "xht", "xsl", "xslt", "xsd", "wsdl", "ares", "axml", "ant", "mxml",
      "plist", "iml", "idea", "csproj", "fsproj", "vbproj", "props", "targets", "resx",
    ],
    file_names: &[],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: yaml_config,
    aliases: &["yaml", "yml"],
    extensions: &["yml", "yaml"],
    file_names: &["pubspec.lock"],
    file_name_prefixes: &[],
  },
  LanguageRegistration {
    load: zig_config,
    aliases: &["zig"],
    extensions: &["zig"],
    file_names: &["build.zig.zon"],
    file_name_prefixes: &[],
  },
];

pub fn detect_language_config(identifier: &str) -> Option<&'static LanguageConfig> {
  let raw_identifier = normalize_identifier(identifier)?;
  find_by_file_name_or_extension(&raw_identifier).or_else(|| {
    let stripped_identifier = raw_identifier.trim_start_matches('.');
    find_by_alias_or_extension(stripped_identifier)
  })
}

pub fn detect_language_config_for_path(path: &Path) -> Option<&'static LanguageConfig> {
  if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
    let normalized_file_name = file_name.trim().to_ascii_lowercase();
    if let Some(config) = find_by_file_name_or_extension(&normalized_file_name) {
      return Some(config);
    }
  }

  path
    .extension()
    .and_then(|ext| ext.to_str())
    .and_then(find_by_alias_or_extension)
}

pub fn detect_language_name_for_path(path: &Path) -> Option<&'static str> {
  detect_language_config_for_path(path).map(|config| config.name)
}

pub fn language_config_for_name(name: &str) -> Option<&'static LanguageConfig> {
  let normalized_name = normalize_identifier(name)?;
  let stripped_name = normalized_name.trim_start_matches('.');
  find_by_alias_or_extension(stripped_name).or_else(|| find_by_file_name(&normalized_name))
}

fn find_by_file_name(file_name: &str) -> Option<&'static LanguageConfig> {
  LANGUAGE_REGISTRATIONS.iter().find_map(|registration| {
    if registration.file_names.contains(&file_name)
      || registration
        .file_name_prefixes
        .iter()
        .any(|prefix| file_name.starts_with(prefix))
    {
      Some((registration.load)())
    } else {
      None
    }
  })
}

fn find_by_file_name_or_extension(file_name: &str) -> Option<&'static LanguageConfig> {
  let normalized_file_name = Path::new(file_name)
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or(file_name);

  find_by_file_name(normalized_file_name).or_else(|| {
    Path::new(normalized_file_name)
      .extension()
      .and_then(|ext| ext.to_str())
      .and_then(find_by_alias_or_extension)
  })
}

fn find_by_alias_or_extension(identifier: &str) -> Option<&'static LanguageConfig> {
  if identifier.is_empty() {
    return None;
  }

  let normalized_identifier = identifier.to_ascii_lowercase();
  LANGUAGE_REGISTRATIONS.iter().find_map(|registration| {
    if registration
      .aliases
      .contains(&normalized_identifier.as_str())
      || registration
        .extensions
        .contains(&normalized_identifier.as_str())
    {
      Some((registration.load)())
    } else {
      None
    }
  })
}

fn normalize_identifier(identifier: &str) -> Option<String> {
  let normalized = identifier
    .trim()
    .trim_matches(|c| c == '{' || c == '}')
    .trim()
    .to_ascii_lowercase();

  if normalized.is_empty() {
    None
  } else {
    Some(normalized)
  }
}

fn bash_config() -> &'static LanguageConfig {
  &bash::BASH_CONFIG
}

fn c_config() -> &'static LanguageConfig {
  &c::C_CONFIG
}

fn cmake_config() -> &'static LanguageConfig {
  &cmake::CMAKE_CONFIG
}

fn cpp_config() -> &'static LanguageConfig {
  &cpp::CPP_CONFIG
}

fn csharp_config() -> &'static LanguageConfig {
  &csharp::CSHARP_CONFIG
}

fn astro_config() -> &'static LanguageConfig {
  &astro::ASTRO_CONFIG
}

fn css_config() -> &'static LanguageConfig {
  &css::CSS_CONFIG
}

fn dart_config() -> &'static LanguageConfig {
  &dart::DART_CONFIG
}

fn elixir_config() -> &'static LanguageConfig {
  &elixir::ELIXIR_CONFIG
}

fn go_config() -> &'static LanguageConfig {
  &go::GO_CONFIG
}

fn hcl_config() -> &'static LanguageConfig {
  &hcl::HCL_CONFIG
}

fn java_config() -> &'static LanguageConfig {
  &java::JAVA_CONFIG
}

fn julia_config() -> &'static LanguageConfig {
  &julia::JULIA_CONFIG
}

fn kotlin_config() -> &'static LanguageConfig {
  &kotlin::KOTLIN_CONFIG
}

fn lua_config() -> &'static LanguageConfig {
  &lua::LUA_CONFIG
}

fn make_config() -> &'static LanguageConfig {
  &make::MAKE_CONFIG
}

fn scss_config() -> &'static LanguageConfig {
  &scss::SCSS_CONFIG
}

fn sql_config() -> &'static LanguageConfig {
  &sql::SQL_CONFIG
}

fn svelte_config() -> &'static LanguageConfig {
  &svelte::SVELTE_CONFIG
}

fn swift_config() -> &'static LanguageConfig {
  &swift::SWIFT_CONFIG
}

fn toml_config() -> &'static LanguageConfig {
  &toml::TOML_CONFIG
}

fn dockerfile_config() -> &'static LanguageConfig {
  &dockerfile::DOCKERFILE_CONFIG
}

fn html_config() -> &'static LanguageConfig {
  &html::HTML_CONFIG
}

fn json_config() -> &'static LanguageConfig {
  &json::JSON_CONFIG
}

fn markdown_config() -> &'static LanguageConfig {
  &markdown::MARKDOWN_CONFIG
}

fn php_config() -> &'static LanguageConfig {
  &php::PHP_CONFIG
}

fn python_config() -> &'static LanguageConfig {
  &python::PYTHON_CONFIG
}

fn ruby_config() -> &'static LanguageConfig {
  &ruby::RUBY_CONFIG
}

fn scala_config() -> &'static LanguageConfig {
  &scala::SCALA_CONFIG
}

fn rust_config() -> &'static LanguageConfig {
  &rust::RUST_CONFIG
}

fn typescript_config() -> &'static LanguageConfig {
  &typescript::TYPESCRIPT_CONFIG
}

fn vue_config() -> &'static LanguageConfig {
  &vue::VUE_CONFIG
}

fn xml_config() -> &'static LanguageConfig {
  &xml::XML_CONFIG
}

fn yaml_config() -> &'static LanguageConfig {
  &yaml::YAML_CONFIG
}

fn zig_config() -> &'static LanguageConfig {
  &zig::ZIG_CONFIG
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
  fn test_detect_json_lock_files() {
    assert_eq!(
      detect_language_config("composer.lock").unwrap().name,
      "json"
    );
    assert_eq!(detect_language_config("Pipfile.lock").unwrap().name, "json");
  }

  #[test]
  fn test_detect_markdown() {
    assert!(detect_language_config("md").is_some());
    assert!(detect_language_config("markdown").is_some());
  }

  #[test]
  fn test_detect_php() {
    assert!(detect_language_config("php").is_some());
    assert!(detect_language_config("phtml").is_some());
  }

  #[test]
  fn test_detect_html() {
    assert!(detect_language_config("html").is_some());
    assert!(detect_language_config("htm").is_some());
  }

  #[test]
  fn test_detect_java() {
    assert!(detect_language_config("java").is_some());
  }

  #[test]
  fn test_detect_julia() {
    assert!(detect_language_config("julia").is_some());
    assert!(detect_language_config("jl").is_some());
  }

  #[test]
  fn test_detect_kotlin() {
    assert!(detect_language_config("kotlin").is_some());
    assert!(detect_language_config("kt").is_some());
    assert!(detect_language_config("kts").is_some());
  }

  #[test]
  fn test_detect_lua() {
    assert!(detect_language_config("lua").is_some());
  }

  #[test]
  fn test_detect_make() {
    assert!(detect_language_config("make").is_some());
    assert!(detect_language_config("makefile").is_some());
    assert!(detect_language_config("mk").is_some());
  }

  #[test]
  fn test_detect_css() {
    assert!(detect_language_config("css").is_some());
  }

  #[test]
  fn test_detect_dart() {
    assert!(detect_language_config("dart").is_some());
  }

  #[test]
  fn test_detect_elixir() {
    assert!(detect_language_config("elixir").is_some());
    assert!(detect_language_config("ex").is_some());
    assert!(detect_language_config("exs").is_some());
  }

  #[test]
  fn test_detect_go() {
    assert!(detect_language_config("go").is_some());
  }

  #[test]
  fn test_detect_dart_file_names() {
    assert_eq!(detect_language_config("pubspec.yaml").unwrap().name, "yaml");
    assert_eq!(detect_language_config("pubspec.lock").unwrap().name, "yaml");
    assert_eq!(
      detect_language_config("analysis_options.yaml")
        .unwrap()
        .name,
      "yaml"
    );
  }

  #[test]
  fn test_detect_language_config_treats_filename_like_identifiers_as_files() {
    assert_eq!(
      detect_language_config("README.md").unwrap().name,
      "markdown"
    );
    assert_eq!(
      detect_language_config("/tmp/pubspec.yaml").unwrap().name,
      "yaml"
    );
    assert_eq!(
      detect_language_config("/tmp/analysis_options.yaml")
        .unwrap()
        .name,
      "yaml"
    );
    assert_eq!(
      detect_language_config("/tmp/build.zig").unwrap().name,
      "zig"
    );
  }

  #[test]
  fn test_detect_hcl() {
    assert!(detect_language_config("hcl").is_some());
    assert!(detect_language_config("terraform").is_some());
    assert!(detect_language_config("tf").is_some());
    assert!(detect_language_config("tfvars").is_some());
  }

  #[test]
  fn test_detect_scss() {
    assert!(detect_language_config("scss").is_some());
  }

  #[test]
  fn test_detect_sql() {
    assert!(detect_language_config("sql").is_some());
  }

  #[test]
  fn test_detect_swift() {
    assert!(detect_language_config("swift").is_some());
  }

  #[test]
  fn test_detect_toml() {
    assert!(detect_language_config("toml").is_some());
  }

  #[test]
  fn test_detect_toml_lock_files() {
    assert_eq!(detect_language_config("Cargo.lock").unwrap().name, "toml");
    assert_eq!(detect_language_config("poetry.lock").unwrap().name, "toml");
    assert_eq!(detect_language_config("uv.lock").unwrap().name, "toml");
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
  fn test_detect_ruby() {
    assert!(detect_language_config("rb").is_some());
    assert!(detect_language_config("ruby").is_some());
  }

  #[test]
  fn test_detect_scala() {
    assert!(detect_language_config("scala").is_some());
    assert!(detect_language_config("sbt").is_some());
    assert!(detect_language_config("sc").is_some());
  }

  #[test]
  fn test_detect_bash() {
    assert!(detect_language_config("sh").is_some());
    assert!(detect_language_config("bash").is_some());
    assert!(detect_language_config("zsh").is_some());
  }

  #[test]
  fn test_detect_c() {
    assert!(detect_language_config("c").is_some());
    assert!(detect_language_config("h").is_some());
  }

  #[test]
  fn test_detect_cmake() {
    assert!(detect_language_config("cmake").is_some());
  }

  #[test]
  fn test_detect_cpp() {
    assert!(detect_language_config("cpp").is_some());
    assert!(detect_language_config("c++").is_some());
    assert!(detect_language_config("hpp").is_some());
    assert!(detect_language_config("cc").is_some());
  }

  #[test]
  fn test_detect_csharp() {
    assert!(detect_language_config("csharp").is_some());
    assert!(detect_language_config("c#").is_some());
    assert!(detect_language_config("cs").is_some());
    assert!(detect_language_config("csx").is_some());
  }

  #[test]
  fn test_detect_astro() {
    assert!(detect_language_config("astro").is_some());
  }

  #[test]
  fn test_detect_svelte() {
    assert!(detect_language_config("svelte").is_some());
  }

  #[test]
  fn test_detect_zig() {
    assert!(detect_language_config("zig").is_some());
  }

  #[test]
  fn test_detect_bash_dotfiles() {
    assert!(detect_language_config(".bashrc").is_some());
    assert!(detect_language_config(".zprofile").is_some());
    assert!(detect_language_config(".envrc").is_some());
  }

  #[test]
  fn test_detect_ruby_file_names() {
    assert_eq!(detect_language_config("Gemfile").unwrap().name, "ruby");
    assert_eq!(detect_language_config("Rakefile").unwrap().name, "ruby");
    assert_eq!(
      detect_language_config(".ruby-version").unwrap().name,
      "ruby"
    );
  }

  #[test]
  fn test_detect_make_file_names() {
    assert_eq!(detect_language_config("Makefile").unwrap().name, "make");
    assert_eq!(detect_language_config("GNUmakefile").unwrap().name, "make");
  }

  #[test]
  fn test_detect_cmake_file_names() {
    assert_eq!(
      detect_language_config("CMakeLists.txt").unwrap().name,
      "cmake"
    );
  }

  #[test]
  fn test_detect_zig_file_names() {
    assert_eq!(detect_language_config("build.zig").unwrap().name, "zig");
    assert_eq!(detect_language_config("build.zig.zon").unwrap().name, "zig");
  }

  #[test]
  fn test_detect_hcl_file_names() {
    assert_eq!(
      detect_language_config(".terraform.lock.hcl").unwrap().name,
      "hcl"
    );
    assert_eq!(detect_language_config(".terraformrc").unwrap().name, "hcl");
    assert_eq!(detect_language_config("terraform.rc").unwrap().name, "hcl");
  }

  #[test]
  fn test_detect_xml() {
    assert!(detect_language_config("xml").is_some());
    assert!(detect_language_config("csproj").is_some());
    assert!(detect_language_config("props").is_some());
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
  fn test_detect_readme_by_file_name() {
    assert_eq!(
      detect_language_config("README.md").unwrap().name,
      "markdown"
    );
    assert_eq!(
      detect_language_config("CHANGELOG").unwrap().name,
      "markdown"
    );
  }

  #[test]
  fn test_detect_dockerfile_variants_by_path() {
    let config = detect_language_config_for_path(Path::new("/tmp/Dockerfile.dev")).unwrap();
    assert_eq!(config.name, "dockerfile");
  }

  #[test]
  fn test_detect_bash_dotfiles_by_path() {
    let config = detect_language_config_for_path(Path::new("/tmp/.bashrc")).unwrap();
    assert_eq!(config.name, "bash");
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
  fn test_c_config_has_correct_name() {
    let config = detect_language_config("c").unwrap();
    assert_eq!(config.name, "c");
  }

  #[test]
  fn test_cmake_config_has_correct_name() {
    let config = detect_language_config("cmake").unwrap();
    assert_eq!(config.name, "cmake");
  }

  #[test]
  fn test_cpp_config_has_correct_name() {
    let config = detect_language_config("cpp").unwrap();
    assert_eq!(config.name, "cpp");
  }

  #[test]
  fn test_csharp_config_has_correct_name() {
    let config = detect_language_config("csharp").unwrap();
    assert_eq!(config.name, "csharp");
  }

  #[test]
  fn test_hcl_config_has_correct_name() {
    let config = detect_language_config("terraform").unwrap();
    assert_eq!(config.name, "hcl");
  }

  #[test]
  fn test_dart_config_has_correct_name() {
    let config = detect_language_config("dart").unwrap();
    assert_eq!(config.name, "dart");
  }

  #[test]
  fn test_elixir_config_has_correct_name() {
    let config = detect_language_config("elixir").unwrap();
    assert_eq!(config.name, "elixir");
  }

  #[test]
  fn test_java_config_has_correct_name() {
    let config = detect_language_config("java").unwrap();
    assert_eq!(config.name, "java");
  }

  #[test]
  fn test_julia_config_has_correct_name() {
    let config = detect_language_config("julia").unwrap();
    assert_eq!(config.name, "julia");
  }

  #[test]
  fn test_kotlin_config_has_correct_name() {
    let config = detect_language_config("kotlin").unwrap();
    assert_eq!(config.name, "kotlin");
  }

  #[test]
  fn test_lua_config_has_correct_name() {
    let config = detect_language_config("lua").unwrap();
    assert_eq!(config.name, "lua");
  }

  #[test]
  fn test_make_config_has_correct_name() {
    let config = detect_language_config("make").unwrap();
    assert_eq!(config.name, "make");
  }

  #[test]
  fn test_sql_config_has_correct_name() {
    let config = detect_language_config("sql").unwrap();
    assert_eq!(config.name, "sql");
  }

  #[test]
  fn test_swift_config_has_correct_name() {
    let config = detect_language_config("swift").unwrap();
    assert_eq!(config.name, "swift");
  }

  #[test]
  fn test_php_config_has_correct_name() {
    let config = detect_language_config("php").unwrap();
    assert_eq!(config.name, "php");
  }

  #[test]
  fn test_ruby_config_has_correct_name() {
    let config = detect_language_config("rb").unwrap();
    assert_eq!(config.name, "ruby");
  }

  #[test]
  fn test_scala_config_has_correct_name() {
    let config = detect_language_config("scala").unwrap();
    assert_eq!(config.name, "scala");
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
  fn test_bash_config_has_correct_name() {
    let config = detect_language_config("sh").unwrap();
    assert_eq!(config.name, "bash");
  }

  #[test]
  fn test_zig_config_has_correct_name() {
    let config = detect_language_config("zig").unwrap();
    assert_eq!(config.name, "zig");
  }

  #[test]
  fn test_language_config_for_name_supports_code_fence_language_names() {
    let rust = language_config_for_name("rust").unwrap();
    assert_eq!(rust.name, "rust");

    let python = language_config_for_name("python").unwrap();
    assert_eq!(python.name, "python");

    let ruby = language_config_for_name("ruby").unwrap();
    assert_eq!(ruby.name, "ruby");

    let scala = language_config_for_name("scala").unwrap();
    assert_eq!(scala.name, "scala");

    let make = language_config_for_name("make").unwrap();
    assert_eq!(make.name, "make");

    let php = language_config_for_name("php").unwrap();
    assert_eq!(php.name, "php");

    let yaml = language_config_for_name("yml").unwrap();
    assert_eq!(yaml.name, "yaml");

    let terraform = language_config_for_name("terraform").unwrap();
    assert_eq!(terraform.name, "hcl");

    let java = language_config_for_name("java").unwrap();
    assert_eq!(java.name, "java");

    let julia = language_config_for_name("julia").unwrap();
    assert_eq!(julia.name, "julia");

    let kotlin = language_config_for_name("kotlin").unwrap();
    assert_eq!(kotlin.name, "kotlin");

    let lua = language_config_for_name("lua").unwrap();
    assert_eq!(lua.name, "lua");

    let cmake = language_config_for_name("cmake").unwrap();
    assert_eq!(cmake.name, "cmake");

    let c = language_config_for_name("c").unwrap();
    assert_eq!(c.name, "c");

    let cpp = language_config_for_name("c++").unwrap();
    assert_eq!(cpp.name, "cpp");

    let csharp = language_config_for_name("c#").unwrap();
    assert_eq!(csharp.name, "csharp");

    let dart = language_config_for_name("dart").unwrap();
    assert_eq!(dart.name, "dart");

    let elixir = language_config_for_name("elixir").unwrap();
    assert_eq!(elixir.name, "elixir");

    let bash = language_config_for_name("{.bash}").unwrap();
    assert_eq!(bash.name, "bash");

    let shell = language_config_for_name("shell").unwrap();
    assert_eq!(shell.name, "bash");

    let sql = language_config_for_name("sql").unwrap();
    assert_eq!(sql.name, "sql");

    let swift = language_config_for_name("swift").unwrap();
    assert_eq!(swift.name, "swift");
  }

  #[test]
  fn test_detect_language_name_for_path_returns_canonical_name() {
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/script.sh")),
      Some("bash")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/page.astro")),
      Some("astro")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/main.go")),
      Some("go")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/main.c")),
      Some("c")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/CMakeLists.txt")),
      Some("cmake")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/main.dart")),
      Some("dart")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/main.exs")),
      Some("elixir")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/main.cpp")),
      Some("cpp")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/pubspec.yaml")),
      Some("yaml")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/pubspec.lock")),
      Some("yaml")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/analysis_options.yaml")),
      Some("yaml")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/Program.cs")),
      Some("csharp")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/App.csproj")),
      Some("xml")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/Cargo.lock")),
      Some("toml")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/composer.lock")),
      Some("json")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/Pipfile.lock")),
      Some("json")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/poetry.lock")),
      Some("toml")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/Main.java")),
      Some("java")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/main.jl")),
      Some("julia")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/query.sql")),
      Some("sql")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/Main.kt")),
      Some("kotlin")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/build.gradle.kts")),
      Some("kotlin")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/main.lua")),
      Some("lua")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/Makefile")),
      Some("make")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/build.mk")),
      Some("make")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/main.swift")),
      Some("swift")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/main.tf")),
      Some("hcl")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/Gemfile")),
      Some("ruby")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/Main.scala")),
      Some("scala")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/build.sbt")),
      Some("scala")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/index.php")),
      Some("php")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/build.zig")),
      Some("zig")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/Component.svelte")),
      Some("svelte")
    );
    assert_eq!(
      detect_language_name_for_path(Path::new("/tmp/component.vue")),
      Some("vue")
    );
  }
}
