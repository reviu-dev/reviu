use std::path::Path;

use gpui::SharedString;
use gpui_component::{IconNamed, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiIconName {
  GitBranch,
  GitMerge,
  ArrowUpFromLine,
  ArrowDownFromLine,
  MessageCircle,
  History,
  FileCode,
  EllipsisVertical,
  SquarePen,
  MessageCircleReply,
  RefreshCcw,
  CreditCard,
  Download,
  Info,
  CircleDot,
  CircleCheck,
  CircleSlash,
}

pub const FILE_ICON_SIZE_PX: f32 = 16.0;

impl IconNamed for UiIconName {
  fn path(self) -> SharedString {
    match self {
      UiIconName::GitBranch => "icons/git-branch.svg",
      UiIconName::GitMerge => "icons/git-merge.svg",
      UiIconName::ArrowUpFromLine => "icons/arrow-up-from-line.svg",
      UiIconName::ArrowDownFromLine => "icons/arrow-down-from-line.svg",
      UiIconName::MessageCircle => "icons/message-circle.svg",
      UiIconName::History => "icons/history.svg",
      UiIconName::FileCode => "icons/file-code.svg",
      UiIconName::EllipsisVertical => "icons/ellipsis-vertical.svg",
      UiIconName::SquarePen => "icons/square-pen.svg",
      UiIconName::MessageCircleReply => "icons/message-circle-reply.svg",
      UiIconName::RefreshCcw => "icons/refresh-ccw.svg",
      UiIconName::CreditCard => "icons/credit-card.svg",
      UiIconName::Download => "icons/download.svg",
      UiIconName::Info => "icons/info.svg",
      UiIconName::CircleDot => "icons/circle-dot.svg",
      UiIconName::CircleCheck => "icons/circle-check.svg",
      UiIconName::CircleSlash => "icons/circle-slash.svg",
    }
    .into()
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileIcon {
  path: SharedString,
}

impl FileIcon {
  fn new(path: impl Into<SharedString>) -> Self {
    Self { path: path.into() }
  }
}

impl IconNamed for FileIcon {
  fn path(self) -> SharedString {
    self.path
  }
}

fn file_icon_path_for_name_str(file_name: &str, is_dark: bool) -> Option<&'static str> {
  let name = file_name.to_lowercase();

  const GIT_FILES: &[&str] = &[
    ".git",
    ".gitattributes",
    ".gitignore",
    ".gitmodules",
    ".gitkeep",
    ".gitconfig",
  ];

  const EXACT_ICON_FILES: &[(&str, &str)] = &[
    ("dockerfile", "file-icons/docker.svg"),
    ("tiltfile", "file-icons/tilt.svg"),
    (".editorconfig", "file-icons/editorconfig.svg"),
    (".npmrc", "file-icons/npm.svg"),
    ("package-lock.json", "file-icons/npm.svg"),
    ("npm-shrinkwrap.json", "file-icons/npm.svg"),
    (".yarnrc", "file-icons/yarn.svg"),
    (".yarnrc.yml", "file-icons/yarn.svg"),
    ("yarn.lock", "file-icons/yarn.svg"),
    ("pnpm-lock.yaml", "file-icons/pnpm.svg"),
    ("pnpm-workspace.yaml", "file-icons/pnpm.svg"),
    (".pnpmfile.cjs", "file-icons/pnpm.svg"),
    (".pnpmfile.js", "file-icons/pnpm.svg"),
    (".pnpmfile.mjs", "file-icons/pnpm.svg"),
    ("bun.lock", "file-icons/bun.svg"),
    ("bun.lockb", "file-icons/bun.svg"),
    ("bunfig.toml", "file-icons/bun.svg"),
    (".nvmrc", "file-icons/nodejs.svg"),
    (".node-version", "file-icons/nodejs.svg"),
    (".bashrc", "file-icons/console.svg"),
    (".bash_profile", "file-icons/console.svg"),
    (".bash_logout", "file-icons/console.svg"),
    (".zshrc", "file-icons/console.svg"),
    (".zprofile", "file-icons/console.svg"),
    (".zshenv", "file-icons/console.svg"),
    (".zlogin", "file-icons/console.svg"),
    (".profile", "file-icons/console.svg"),
    (".envrc", "file-icons/console.svg"),
    (".eslintrc", "file-icons/eslint.svg"),
    (".eslintignore", "file-icons/eslint.svg"),
    (".prettierrc", "file-icons/prettier.svg"),
    (".prettierignore", "file-icons/prettier.svg"),
    ("biome.json", "file-icons/biome.svg"),
    ("biome.jsonc", "file-icons/biome.svg"),
    (".postcssrc", "file-icons/postcss.svg"),
    ("dependabot.yml", "file-icons/dependabot.svg"),
    ("dependabot.yaml", "file-icons/dependabot.svg"),
    ("nx.json", "file-icons/nx.svg"),
    ("angular.json", "file-icons/angular.svg"),
    ("nodemon.json", "file-icons/nodemon.svg"),
    (".nodemonrc", "file-icons/nodemon.svg"),
    ("ecosystem.json", "file-icons/pm2-ecosystem.svg"),
    ("ecosystem.yml", "file-icons/pm2-ecosystem.svg"),
    ("ecosystem.yaml", "file-icons/pm2-ecosystem.svg"),
    ("schema.prisma", "file-icons/prisma.svg"),
    (".oxlintrc.json", "file-icons/oxc.svg"),
    (".oxfmtrc.json", "file-icons/oxc.svg"),
    (".oxfmtrc.jsonc", "file-icons/oxc.svg"),
    (".terraformrc", "file-icons/terraform.svg"),
    ("terraform.rc", "file-icons/terraform.svg"),
    ("license", "file-icons/license.svg"),
    ("copying", "file-icons/license.svg"),
    ("unlicense", "file-icons/license.svg"),
    ("licence", "file-icons/license.svg"),
    ("build.zig", "file-icons/zig.svg"),
    ("build.zig.zon", "file-icons/zig.svg"),
  ];

  if let Some((_, icon)) = EXACT_ICON_FILES
    .iter()
    .find(|(exact_name, _)| name == *exact_name)
  {
    return Some(*icon);
  }

  if GIT_FILES.contains(&name.as_str()) {
    return Some("file-icons/git.svg");
  }

  if name.starts_with("vite.config.") {
    return Some("file-icons/vite.svg");
  }

  if name.starts_with("astro.config.") {
    return Some("file-icons/astro-config.svg");
  }

  if name.starts_with("tsconfig.") {
    return Some("file-icons/tsconfig.svg");
  }

  if name.starts_with(".eslintrc.") || name.starts_with("eslint.config.") {
    return Some("file-icons/eslint.svg");
  }

  if name.starts_with(".prettierrc.") || name.starts_with("prettier.config.") {
    return Some("file-icons/prettier.svg");
  }

  if name.starts_with(".postcssrc.") || name.starts_with("postcss.config.") {
    return Some("file-icons/postcss.svg");
  }

  if name.starts_with("webpack.config.")
    || (name.starts_with("webpack.") && name.contains(".config."))
  {
    return Some("file-icons/webpack.svg");
  }

  if name.starts_with("vitest.config.") || name.starts_with("vitest.workspace.") {
    return Some("file-icons/vitest.svg");
  }

  if name.starts_with("jest.config.")
    || name.starts_with("jest.setup.")
    || name.starts_with("jest.preset.")
  {
    return Some("file-icons/jest.svg");
  }

  if name.starts_with("ecosystem.config.") {
    return Some("file-icons/pm2-ecosystem.svg");
  }

  if name.starts_with("nuxt.config.") {
    return Some("file-icons/nuxt.svg");
  }

  if name.starts_with("drizzle.config.") {
    return Some("file-icons/drizzle.svg");
  }

  if name.starts_with("prisma.config.") {
    return Some("file-icons/prisma.svg");
  }

  if name.starts_with(".oxlintrc.")
    || name.starts_with("oxlint.config.")
    || name.starts_with(".oxfmtrc.")
    || name.starts_with("oxfmt.config.")
  {
    return Some("file-icons/oxc.svg");
  }

  if name.starts_with("license.") || name.starts_with("copying.") || name.starts_with("licence.") {
    return Some("file-icons/license.svg");
  }

  if name.ends_with(".d.ts") {
    return Some("file-icons/typescript-def.svg");
  }

  if name.ends_with(".zig.zon") {
    return Some("file-icons/zig.svg");
  }

  if name == ".terraform.lock.hcl" || ext_eq(&name, "hcl") {
    return Some(if is_dark {
      "file-icons/hcl/dark.svg"
    } else {
      "file-icons/hcl/light.svg"
    });
  }

  let ext = Path::new(&name).extension()?.to_str()?;

  match ext {
    "vue" => Some("file-icons/vue.svg"),
    "astro" => Some("file-icons/astro.svg"),
    "ts" | "tsx" | "mts" | "cts" => Some("file-icons/typescript.svg"),
    "js" | "jsx" | "mjs" | "cjs" => Some("file-icons/javascript.svg"),
    "c" | "h" => Some("file-icons/c.svg"),
    "cpp" | "cc" | "cxx" | "c++" | "cp" | "hpp" | "hh" | "hxx" | "h++" | "ipp" | "inl" | "tpp"
    | "ixx" | "cppm" | "ccm" | "cxxm" => Some("file-icons/cpp.svg"),
    "cs" | "csx" | "csproj" => Some("file-icons/csharp.svg"),
    "go" => Some("file-icons/go.svg"),
    "rs" => Some("file-icons/rust.svg"),
    "svelte" => Some("file-icons/svelte.svg"),
    "css" => Some("file-icons/css.svg"),
    "scss" | "sass" => Some("file-icons/sass.svg"),
    "html" | "htm" => Some("file-icons/html.svg"),
    "json" | "jsonc" => Some("file-icons/json.svg"),
    "md" | "mdx" => Some("file-icons/markdown.svg"),
    "yml" | "yaml" => Some("file-icons/yaml.svg"),
    "xml" => Some("file-icons/xml.svg"),
    "svg" => Some("file-icons/svg.svg"),
    "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp" | "ico" | "tif" | "tiff" | "heic"
    | "heif" => Some("file-icons/image.svg"),
    "pdf" => Some("file-icons/pdf.svg"),
    "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" => Some("file-icons/zip.svg"),
    "sh" | "bash" | "zsh" | "fish" => Some("file-icons/console.svg"),
    "py" => Some("file-icons/python.svg"),
    "sql" => Some("file-icons/sql.svg"),
    "rb" => Some("file-icons/ruby.svg"),
    "php" | "phtml" => Some("file-icons/php.svg"),
    "tf" | "tfvars" => Some("file-icons/terraform.svg"),
    "swift" => Some("file-icons/swift.svg"),
    "zig" => Some("file-icons/zig.svg"),
    "ml" | "mli" | "mll" | "mly" | "opam" => Some("file-icons/ocaml.svg"),
    "toml" => Some(if is_dark {
      "file-icons/toml/dark.svg"
    } else {
      "file-icons/toml/light.svg"
    }),
    _ => None,
  }
}

fn ext_eq(file_name: &str, ext: &str) -> bool {
  Path::new(file_name)
    .extension()
    .and_then(|value| value.to_str())
    == Some(ext)
}

pub fn file_icon_for_name(file_name: &str) -> Option<FileIcon> {
  file_icon_path_for_name_str(file_name, false).map(FileIcon::new)
}

pub fn file_icon_for_path(path: impl AsRef<Path>) -> Option<FileIcon> {
  let name = path.as_ref().file_name()?.to_str()?;
  file_icon_for_name(name)
}

pub fn file_icon_path_for_name(file_name: &str) -> Option<SharedString> {
  file_icon_path_for_name_str(file_name, false).map(SharedString::from)
}

pub fn file_icon_path_for_path(path: impl AsRef<Path>) -> Option<SharedString> {
  let name = path.as_ref().file_name()?.to_str()?;
  file_icon_path_for_name(name)
}

pub fn file_icon_path_for_name_with_theme(file_name: &str, theme: &Theme) -> Option<SharedString> {
  file_icon_path_for_name_str(file_name, theme.mode.is_dark()).map(SharedString::from)
}

pub fn file_icon_path_for_path_with_theme(
  path: impl AsRef<Path>,
  theme: &Theme,
) -> Option<SharedString> {
  let name = path.as_ref().file_name()?.to_str()?;
  file_icon_path_for_name_with_theme(name, theme)
}

#[cfg(test)]
mod tests {
  use super::file_icon_path_for_name_str;

  #[test]
  fn resolves_runtime_and_package_manager_icons() {
    assert_eq!(
      file_icon_path_for_name_str(".nvmrc", false),
      Some("file-icons/nodejs.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".node-version", false),
      Some("file-icons/nodejs.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("package-lock.json", false),
      Some("file-icons/npm.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("yarn.lock", false),
      Some("file-icons/yarn.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("pnpm-workspace.yaml", false),
      Some("file-icons/pnpm.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("bunfig.toml", false),
      Some("file-icons/bun.svg")
    );
  }

  #[test]
  fn resolves_shell_and_media_icons() {
    assert_eq!(
      file_icon_path_for_name_str(".zshrc", false),
      Some("file-icons/console.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("deploy.sh", false),
      Some("file-icons/console.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("photo.jpeg", false),
      Some("file-icons/image.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("slides.pdf", false),
      Some("file-icons/pdf.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("archive.tgz", false),
      Some("file-icons/zip.svg")
    );
  }

  #[test]
  fn resolves_tooling_and_framework_icons() {
    assert_eq!(
      file_icon_path_for_name_str("astro.config.mjs", false),
      Some("file-icons/astro-config.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("eslint.config.js", false),
      Some("file-icons/eslint.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".prettierrc.yaml", false),
      Some("file-icons/prettier.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("postcss.config.cjs", false),
      Some("file-icons/postcss.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("webpack.dev.config.ts", false),
      Some("file-icons/webpack.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("vitest.workspace.ts", false),
      Some("file-icons/vitest.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("jest.setup.ts", false),
      Some("file-icons/jest.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("nuxt.config.ts", false),
      Some("file-icons/nuxt.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("angular.json", false),
      Some("file-icons/angular.svg")
    );
  }

  #[test]
  fn resolves_project_config_and_language_specific_icons() {
    assert_eq!(
      file_icon_path_for_name_str(".editorconfig", false),
      Some("file-icons/editorconfig.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("nx.json", false),
      Some("file-icons/nx.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("schema.prisma", false),
      Some("file-icons/prisma.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("drizzle.config.ts", false),
      Some("file-icons/drizzle.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("index.php", false),
      Some("file-icons/php.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("Package.swift", false),
      Some("file-icons/swift.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("main.c", false),
      Some("file-icons/c.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("main.h", false),
      Some("file-icons/c.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("main.cpp", false),
      Some("file-icons/cpp.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("vector.hpp", false),
      Some("file-icons/cpp.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("Program.cs", false),
      Some("file-icons/csharp.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("script.csx", false),
      Some("file-icons/csharp.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("App.csproj", false),
      Some("file-icons/csharp.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("build.zig.zon", false),
      Some("file-icons/zig.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("dune.mli", false),
      Some("file-icons/ocaml.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("main.tf", false),
      Some("file-icons/terraform.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("terraform.auto.tfvars", false),
      Some("file-icons/terraform.svg")
    );
  }

  #[test]
  fn resolves_oxc_and_license_icons_without_breaking_generic_files() {
    assert_eq!(
      file_icon_path_for_name_str(".oxlintrc.json", false),
      Some("file-icons/oxc.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("oxfmt.config.ts", false),
      Some("file-icons/oxc.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("LICENSE.md", false),
      Some("file-icons/license.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("settings.json", false),
      Some("file-icons/json.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("deployment.yaml", false),
      Some("file-icons/yaml.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("feed.xml", false),
      Some("file-icons/xml.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".terraformrc", false),
      Some("file-icons/terraform.svg")
    );
  }

  #[test]
  fn resolves_hcl_icons_with_theme() {
    assert_eq!(
      file_icon_path_for_name_str("main.hcl", true),
      Some("file-icons/hcl/dark.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("main.hcl", false),
      Some("file-icons/hcl/light.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".terraform.lock.hcl", true),
      Some("file-icons/hcl/dark.svg")
    );
  }

  #[test]
  fn keeps_existing_special_cases() {
    assert_eq!(
      file_icon_path_for_name_str("vite.config.ts", false),
      Some("file-icons/vite.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("tsconfig.base.json", false),
      Some("file-icons/tsconfig.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("types.d.ts", false),
      Some("file-icons/typescript-def.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("Cargo.toml", true),
      Some("file-icons/toml/dark.svg")
    );
  }
}
