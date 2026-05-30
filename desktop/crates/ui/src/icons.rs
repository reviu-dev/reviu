use std::path::Path;

use gpui::SharedString;
use gpui_component::{IconNamed, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiIconName {
  GitBranch,
  GitMerge,
  GitCommitVertical,
  GitCommitHorizontal,
  GitPullRequestDraft,
  GitPullRequestClosed,
  GitPullRequest,
  GitPullRequestArrow,
  GitFork,
  ArrowUpFromLine,
  ArrowDownFromLine,
  MessageCircle,
  History,
  FileCode,
  EllipsisVertical,
  SquarePen,
  MessageCircleReply,
  MessageCirclePlus,
  RefreshCw,
  CreditCard,
  Download,
  Info,
  CircleDot,
  CircleCheck,
  Check,
  CircleSlash,
  Lock,
  ScanEye,
  Eye,
  FileDiff,
  Pin,
  SquareTerminal,
  X,
  SlidersHorizontal,
  Star,
  StarFilled,
  SmilePlus,
  Trash,
  FoldVertical,
  UnfoldVertical,
  Puzzle,
  Sparkles,
  // Brands
  BrandX,
  GoogleChrome,
  FirefoxBrowser,
  Claude,
  OpenAi,
}

pub const FILE_ICON_SIZE_PX: f32 = 16.0;

impl IconNamed for UiIconName {
  fn path(self) -> SharedString {
    match self {
      UiIconName::GitBranch => "icons/git-branch.svg",
      UiIconName::GitMerge => "icons/git-merge.svg",
      UiIconName::GitCommitVertical => "icons/git-commit-vertical.svg",
      UiIconName::GitCommitHorizontal => "icons/git-commit-horizontal.svg",
      UiIconName::GitPullRequestDraft => "icons/git-pull-request-draft.svg",
      UiIconName::GitPullRequestClosed => "icons/git-pull-request-closed.svg",
      UiIconName::GitPullRequest => "icons/git-pull-request.svg",
      UiIconName::GitPullRequestArrow => "icons/git-pull-request-arrow.svg",
      UiIconName::GitFork => "icons/git-fork.svg",
      UiIconName::ArrowUpFromLine => "icons/arrow-up-from-line.svg",
      UiIconName::ArrowDownFromLine => "icons/arrow-down-from-line.svg",
      UiIconName::MessageCircle => "icons/message-circle.svg",
      UiIconName::History => "icons/history.svg",
      UiIconName::FileCode => "icons/file-code.svg",
      UiIconName::EllipsisVertical => "icons/ellipsis-vertical.svg",
      UiIconName::SquarePen => "icons/square-pen.svg",
      UiIconName::MessageCircleReply => "icons/message-circle-reply.svg",
      UiIconName::MessageCirclePlus => "icons/message-circle-plus.svg",
      UiIconName::RefreshCw => "icons/refresh-cw.svg",
      UiIconName::CreditCard => "icons/credit-card.svg",
      UiIconName::Download => "icons/download.svg",
      UiIconName::Info => "icons/info.svg",
      UiIconName::CircleDot => "icons/circle-dot.svg",
      UiIconName::CircleCheck => "icons/circle-check.svg",
      UiIconName::Check => "icons/check.svg",
      UiIconName::CircleSlash => "icons/circle-slash.svg",
      UiIconName::Lock => "icons/lock.svg",
      UiIconName::ScanEye => "icons/scan-eye.svg",
      UiIconName::FileDiff => "icons/file-diff.svg",
      UiIconName::Pin => "icons/pin.svg",
      UiIconName::SquareTerminal => "icons/square-terminal.svg",
      UiIconName::X => "icons/x.svg",
      UiIconName::Eye => "icons/eye.svg",
      UiIconName::SlidersHorizontal => "icons/sliders-horizontal.svg",
      UiIconName::Star => "icons/star.svg",
      UiIconName::StarFilled => "icons/star-filled.svg",
      UiIconName::SmilePlus => "icons/smile-plus.svg",
      UiIconName::Trash => "icons/trash.svg",
      UiIconName::FoldVertical => "icons/fold-vertical.svg",
      UiIconName::UnfoldVertical => "icons/unfold-vertical.svg",
      UiIconName::Puzzle => "icons/puzzle.svg",
      UiIconName::Sparkles => "icons/sparkles.svg",
      // Brands
      UiIconName::BrandX => "icons/brands/x.svg",
      UiIconName::GoogleChrome => "icons/brands/googlechrome.svg",
      UiIconName::FirefoxBrowser => "icons/brands/firefoxbrowser.svg",
      UiIconName::Claude => "icons/brands/claude.svg",
      UiIconName::OpenAi => "icons/brands/openai.svg",
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
    ("makefile", "file-icons/makefile.svg"),
    ("tiltfile", "file-icons/tilt.svg"),
    ("cmakelists.txt", "file-icons/cmake.svg"),
    (".clang-format", "file-icons/clangd.svg"),
    (".clang-tidy", "file-icons/clangd.svg"),
    ("compile_commands.json", "file-icons/clangd.svg"),
    ("meson.build", "file-icons/meson.svg"),
    ("meson_options.txt", "file-icons/meson.svg"),
    ("xmake.lua", "file-icons/xmake.svg"),
    ("sconstruct", "file-icons/scons.svg"),
    ("sconscript", "file-icons/scons.svg"),
    ("pom.xml", "file-icons/maven.svg"),
    ("build.gradle", "file-icons/gradle.svg"),
    ("build.gradle.kts", "file-icons/gradle.svg"),
    ("settings.gradle", "file-icons/gradle.svg"),
    ("settings.gradle.kts", "file-icons/gradle.svg"),
    ("gradle.properties", "file-icons/gradle.svg"),
    ("pubspec.yaml", "file-icons/yaml.svg"),
    ("pubspec.lock", "file-icons/yaml.svg"),
    ("analysis_options.yaml", "file-icons/yaml.svg"),
    ("nest-cli.json", "file-icons/nest.svg"),
    ("nuget.config", "file-icons/nuget.svg"),
    ("packages.config", "file-icons/nuget.svg"),
    ("packages.lock.json", "file-icons/nuget.svg"),
    ("package.json", "file-icons/nodejs.svg"),
    ("knip.json", "file-icons/knip.svg"),
    ("knip.jsonc", "file-icons/knip.svg"),
    (".editorconfig", "file-icons/editorconfig.svg"),
    ("jsconfig.json", "file-icons/jsconfig.svg"),
    (".npmrc", "file-icons/npm.svg"),
    ("package-lock.json", "file-icons/npm.svg"),
    ("npm-shrinkwrap.json", "file-icons/npm.svg"),
    (".babelrc", "file-icons/babel.svg"),
    (".swcrc", "file-icons/swc.svg"),
    (".browserslistrc", "file-icons/browserlist.svg"),
    ("browserslist", "file-icons/browserlist.svg"),
    ("lerna.json", "file-icons/lerna.svg"),
    ("typedoc.json", "file-icons/typedoc.svg"),
    ("tsdoc.json", "file-icons/tsdoc.svg"),
    ("renovate.json", "file-icons/renovate.svg"),
    ("renovate.json5", "file-icons/renovate.svg"),
    (".renovaterc", "file-icons/renovate.svg"),
    (".renovaterc.json", "file-icons/renovate.svg"),
    (".renovaterc.json5", "file-icons/renovate.svg"),
    (".yarnrc", "file-icons/yarn.svg"),
    (".yarnrc.yml", "file-icons/yarn.svg"),
    ("yarn.lock", "file-icons/yarn.svg"),
    ("pnpm-lock.yaml", "file-icons/pnpm.svg"),
    ("pnpm-workspace.yaml", "file-icons/pnpm.svg"),
    (".pnpmfile.cjs", "file-icons/pnpm.svg"),
    (".pnpmfile.js", "file-icons/pnpm.svg"),
    (".pnpmfile.mjs", "file-icons/pnpm.svg"),
    ("go.mod", "file-icons/go-mod.svg"),
    ("go.sum", "file-icons/go-mod.svg"),
    ("go.work", "file-icons/go-mod.svg"),
    ("go.work.sum", "file-icons/go-mod.svg"),
    ("claude.md", "file-icons/claude.svg"),
    ("claude.local.md", "file-icons/claude.svg"),
    ("cargo.lock", "file-icons/lock.svg"),
    ("pipfile.lock", "file-icons/lock.svg"),
    ("poetry.lock", "file-icons/poetry.svg"),
    ("uv.lock", "file-icons/uv.svg"),
    ("uv.toml", "file-icons/uv.svg"),
    ("rust-toolchain", "file-icons/rust.svg"),
    (".rprofile", "file-icons/r.svg"),
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
    (".env", "file-icons/tune.svg"),
    (".vimrc", "file-icons/vim.svg"),
    ("_vimrc", "file-icons/vim.svg"),
    (".gvimrc", "file-icons/vim.svg"),
    ("_gvimrc", "file-icons/vim.svg"),
    (".ideavimrc", "file-icons/vim.svg"),
    (".eslintrc", "file-icons/eslint.svg"),
    (".eslintignore", "file-icons/eslint.svg"),
    (".ruff.toml", "file-icons/ruff.svg"),
    ("ruff.toml", "file-icons/ruff.svg"),
    (".prettierrc", "file-icons/prettier.svg"),
    (".prettierignore", "file-icons/prettier.svg"),
    ("biome.json", "file-icons/biome.svg"),
    ("biome.jsonc", "file-icons/biome.svg"),
    ("composer.json", "file-icons/php.svg"),
    ("composer.lock", "file-icons/php.svg"),
    ("phpunit.xml", "file-icons/phpunit.svg"),
    ("phpunit.xml.dist", "file-icons/phpunit.svg"),
    ("phpstan.neon", "file-icons/phpstan.svg"),
    ("phpstan.neon.dist", "file-icons/phpstan.svg"),
    (".php-cs-fixer.php", "file-icons/php-cs-fixer.svg"),
    (".php-cs-fixer.dist.php", "file-icons/php-cs-fixer.svg"),
    ("gemfile.lock", "file-icons/gemfile.svg"),
    (".rubocop.yml", "file-icons/rubocop.svg"),
    (".rubocop_todo.yml", "file-icons/rubocop.svg"),
    (".rspec", "file-icons/rspec.svg"),
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

  if name == "jsr.json" {
    return Some(if is_dark {
      "file-icons/jsr/dark.svg"
    } else {
      "file-icons/jsr/light.svg"
    });
  }

  if name == "copilot-instructions.md" {
    return Some(if is_dark {
      "file-icons/copilot/dark.svg"
    } else {
      "file-icons/copilot/light.svg"
    });
  }

  if name.starts_with("next.config.") {
    return Some(if is_dark {
      "file-icons/next/dark.svg"
    } else {
      "file-icons/next/light.svg"
    });
  }

  if name.starts_with("tsconfig.") {
    return Some("file-icons/tsconfig.svg");
  }

  if name.starts_with(".eslintrc.") || name.starts_with("eslint.config.") {
    return Some("file-icons/eslint.svg");
  }

  if name.starts_with(".env.") {
    return Some("file-icons/tune.svg");
  }

  if name.starts_with(".prettierrc.") || name.starts_with("prettier.config.") {
    return Some("file-icons/prettier.svg");
  }

  if name.starts_with("babel.config.") {
    return Some("file-icons/babel.svg");
  }

  if name.starts_with(".postcssrc.") || name.starts_with("postcss.config.") {
    return Some("file-icons/postcss.svg");
  }

  if name.starts_with("tailwind.config.") {
    return Some("file-icons/tailwindcss.svg");
  }

  if name.starts_with("knip.") || name.starts_with("knip.config.") {
    return Some("file-icons/knip.svg");
  }

  if name == "netlify.toml" {
    return Some(if is_dark {
      "file-icons/netlify/dark.svg"
    } else {
      "file-icons/netlify/light.svg"
    });
  }

  if name == "bun.lock" || name == "bun.lockb" || name == "bunfig.toml" {
    return Some(if is_dark {
      "file-icons/bun/dark.svg"
    } else {
      "file-icons/bun/light.svg"
    });
  }

  if name == "nginx.conf" || name == ".nginx.conf" || name.starts_with("nginx.") {
    return Some("file-icons/nginx.svg");
  }

  if name.starts_with("uno.config.") || name.starts_with("unocss.config.") {
    return Some("file-icons/unocss.svg");
  }

  if name.starts_with(".stylelintrc.") || name.starts_with("stylelint.config.") {
    return Some("file-icons/stylelint.svg");
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

  if name.starts_with("playwright.config.") {
    return Some("file-icons/playwright.svg");
  }

  if name.starts_with("cypress.config.") {
    return Some("file-icons/cypress.svg");
  }

  if name.starts_with("rollup.config.") {
    return Some("file-icons/rollup.svg");
  }

  if name.starts_with("commitlint.config.") {
    return Some("file-icons/commitlint.svg");
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
    "sln" => Some("file-icons/visualstudio.svg"),
    "java" => Some("file-icons/java.svg"),
    "jl" => Some("file-icons/julia.svg"),
    "kt" | "kts" => Some("file-icons/kotlin.svg"),
    "lua" => Some("file-icons/lua.svg"),
    "clj" | "cljs" | "cljc" | "edn" => Some("file-icons/clojure.svg"),
    "dart" => Some("file-icons/dart.svg"),
    "ex" | "exs" => Some("file-icons/elixir.svg"),
    "go" => Some("file-icons/go.svg"),
    "hs" | "lhs" => Some("file-icons/haskell.svg"),
    "r" => Some("file-icons/r.svg"),
    "rs" => Some("file-icons/rust.svg"),
    "svelte" => Some("file-icons/svelte.svg"),
    "css" => Some("file-icons/css.svg"),
    "scss" | "sass" => Some("file-icons/sass.svg"),
    "html" | "htm" => Some("file-icons/html.svg"),
    "json" | "jsonc" => Some("file-icons/json.svg"),
    "md" | "mdx" => Some("file-icons/markdown.svg"),
    "yml" | "yaml" => Some("file-icons/yaml.svg"),
    "xml" => Some("file-icons/xml.svg"),
    "scm" => Some("file-icons/scheme.svg"),
    "xaml" => Some("file-icons/xaml.svg"),
    "svg" => Some("file-icons/svg.svg"),
    "jar" => Some("file-icons/jar.svg"),
    "cmake" => Some("file-icons/cmake.svg"),
    "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp" | "ico" | "tif" | "tiff" | "heic"
    | "heif" => Some("file-icons/image.svg"),
    "pdf" => Some("file-icons/pdf.svg"),
    "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" => Some("file-icons/zip.svg"),
    "woff" | "woff2" | "ttf" | "otf" | "eot" | "fon" => Some("file-icons/font.svg"),
    "log" => Some("file-icons/log.svg"),
    "sh" | "bash" | "zsh" | "fish" => Some("file-icons/console.svg"),
    "py" => Some("file-icons/python.svg"),
    "sql" => Some("file-icons/sql.svg"),
    "rb" => Some("file-icons/ruby.svg"),
    "scala" | "sbt" | "sc" => Some("file-icons/scala.svg"),
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
      file_icon_path_for_name_str("Pipfile.lock", false),
      Some("file-icons/lock.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("poetry.lock", false),
      Some("file-icons/poetry.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("uv.lock", false),
      Some("file-icons/uv.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("uv.toml", false),
      Some("file-icons/uv.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("bunfig.toml", false),
      Some("file-icons/bun/light.svg")
    );
  }

  #[test]
  fn resolves_shell_and_media_icons() {
    assert_eq!(
      file_icon_path_for_name_str(".env", false),
      Some("file-icons/tune.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".env.example", false),
      Some("file-icons/tune.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".zshrc", false),
      Some("file-icons/console.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".envrc", false),
      Some("file-icons/console.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".vimrc", false),
      Some("file-icons/vim.svg")
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
    assert_eq!(
      file_icon_path_for_name_str("debug.log", false),
      Some("file-icons/log.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("inter.woff2", false),
      Some("file-icons/font.svg")
    );
  }

  #[test]
  fn resolves_tooling_and_framework_icons() {
    assert_eq!(
      file_icon_path_for_name_str("astro.config.mjs", false),
      Some("file-icons/astro-config.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("next.config.ts", false),
      Some("file-icons/next/light.svg")
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
      file_icon_path_for_name_str("babel.config.mjs", false),
      Some("file-icons/babel.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("tailwind.config.ts", false),
      Some("file-icons/tailwindcss.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("stylelint.config.js", false),
      Some("file-icons/stylelint.svg")
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
      file_icon_path_for_name_str("nest-cli.json", false),
      Some("file-icons/nest.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".ruff.toml", false),
      Some("file-icons/ruff.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("angular.json", false),
      Some("file-icons/angular.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("playwright.config.ts", false),
      Some("file-icons/playwright.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("cypress.config.ts", false),
      Some("file-icons/cypress.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("rollup.config.mjs", false),
      Some("file-icons/rollup.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("commitlint.config.js", false),
      Some("file-icons/commitlint.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("jsr.json", false),
      Some("file-icons/jsr/light.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("knip.json", false),
      Some("file-icons/knip.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("knip.ts", false),
      Some("file-icons/knip.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("renovate.json", false),
      Some("file-icons/renovate.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".renovaterc.json5", false),
      Some("file-icons/renovate.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("uno.config.ts", false),
      Some("file-icons/unocss.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("unocss.config.ts", false),
      Some("file-icons/unocss.svg")
    );
  }

  #[test]
  fn resolves_project_config_and_language_specific_icons() {
    assert_eq!(
      file_icon_path_for_name_str(".editorconfig", false),
      Some("file-icons/editorconfig.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("CLAUDE.md", false),
      Some("file-icons/claude.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("CLAUDE.local.md", false),
      Some("file-icons/claude.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("netlify.toml", false),
      Some("file-icons/netlify/light.svg")
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
      file_icon_path_for_name_str("App.sln", false),
      Some("file-icons/visualstudio.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("App.xaml", false),
      Some("file-icons/xaml.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("nuget.config", false),
      Some("file-icons/nuget.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("packages.lock.json", false),
      Some("file-icons/nuget.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("Main.java", false),
      Some("file-icons/java.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("main.jl", false),
      Some("file-icons/julia.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("main.exs", false),
      Some("file-icons/elixir.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("main.dart", false),
      Some("file-icons/dart.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("pubspec.yaml", false),
      Some("file-icons/yaml.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("pubspec.lock", false),
      Some("file-icons/yaml.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("analysis_options.yaml", false),
      Some("file-icons/yaml.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("Main.kt", false),
      Some("file-icons/kotlin.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("main.lua", false),
      Some("file-icons/lua.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("core.clj", false),
      Some("file-icons/clojure.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("query.sql", false),
      Some("file-icons/sql.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("main.hs", false),
      Some("file-icons/haskell.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("analysis.R", false),
      Some("file-icons/r.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".Rprofile", false),
      Some("file-icons/r.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("main.swift", false),
      Some("file-icons/swift.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("Main.scala", false),
      Some("file-icons/scala.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("build.sbt", false),
      Some("file-icons/scala.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("build.gradle.kts", false),
      Some("file-icons/gradle.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("pom.xml", false),
      Some("file-icons/maven.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("build.gradle.kts", false),
      Some("file-icons/gradle.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("app.jar", false),
      Some("file-icons/jar.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("CMakeLists.txt", false),
      Some("file-icons/cmake.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("toolchain.cmake", false),
      Some("file-icons/cmake.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("compile_commands.json", false),
      Some("file-icons/clangd.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("meson.build", false),
      Some("file-icons/meson.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("xmake.lua", false),
      Some("file-icons/xmake.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("SConstruct", false),
      Some("file-icons/scons.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("go.mod", false),
      Some("file-icons/go-mod.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("Cargo.lock", false),
      Some("file-icons/lock.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("Gemfile.lock", false),
      Some("file-icons/gemfile.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".rubocop.yml", false),
      Some("file-icons/rubocop.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".rspec", false),
      Some("file-icons/rspec.svg")
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
      file_icon_path_for_name_str("query.scm", false),
      Some("file-icons/scheme.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("nginx.conf", false),
      Some("file-icons/nginx.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".terraformrc", false),
      Some("file-icons/terraform.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("package.json", false),
      Some("file-icons/nodejs.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("jsconfig.json", false),
      Some("file-icons/jsconfig.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("go.work.sum", false),
      Some("file-icons/go-mod.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("composer.json", false),
      Some("file-icons/php.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("phpunit.xml.dist", false),
      Some("file-icons/phpunit.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("phpstan.neon", false),
      Some("file-icons/phpstan.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str(".php-cs-fixer.php", false),
      Some("file-icons/php-cs-fixer.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("browserslist", false),
      Some("file-icons/browserlist.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("typedoc.json", false),
      Some("file-icons/typedoc.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("tsdoc.json", false),
      Some("file-icons/tsdoc.svg")
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
  fn resolves_next_and_netlify_icons_with_theme() {
    assert_eq!(
      file_icon_path_for_name_str("next.config.ts", true),
      Some("file-icons/next/dark.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("next.config.ts", false),
      Some("file-icons/next/light.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("netlify.toml", true),
      Some("file-icons/netlify/dark.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("netlify.toml", false),
      Some("file-icons/netlify/light.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("bun.lock", true),
      Some("file-icons/bun/dark.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("bun.lock", false),
      Some("file-icons/bun/light.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("copilot-instructions.md", true),
      Some("file-icons/copilot/dark.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("copilot-instructions.md", false),
      Some("file-icons/copilot/light.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("jsr.json", true),
      Some("file-icons/jsr/dark.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("jsr.json", false),
      Some("file-icons/jsr/light.svg")
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
    assert_eq!(
      file_icon_path_for_name_str("Cargo.lock", true),
      Some("file-icons/lock.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("rust-toolchain", true),
      Some("file-icons/rust.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("rust-toolchain.toml", true),
      Some("file-icons/toml/dark.svg")
    );
    assert_eq!(
      file_icon_path_for_name_str("config.toml", true),
      Some("file-icons/toml/dark.svg")
    );
  }
}
