use std::path::Path;

use gpui::SharedString;
use gpui_component::IconNamed;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiIconName {
  GitBranch,
  GitMerge,
}

pub const FILE_ICON_SIZE_PX: f32 = 16.0;

impl IconNamed for UiIconName {
  fn path(self) -> SharedString {
    match self {
      UiIconName::GitBranch => "icons/git-branch.svg",
      UiIconName::GitMerge => "icons/git-merge.svg",
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

fn file_icon_path_for_name_str(file_name: &str) -> Option<&'static str> {
  let name = file_name.to_lowercase();

  if name == "dockerfile" {
    return Some("file-icons/docker.svg");
  }

  if name.starts_with("vite.config.") {
    return Some("file-icons/vite.svg");
  }

  if name.ends_with(".d.ts") {
    return Some("file-icons/typescript.svg");
  }

  let ext = Path::new(&name).extension()?.to_str()?;

  match ext {
    "vue" => Some("file-icons/vue.svg"),
    "ts" | "tsx" | "mts" | "cts" => Some("file-icons/typescript.svg"),
    "js" | "jsx" | "mjs" | "cjs" => Some("file-icons/javascript.svg"),
    "go" => Some("file-icons/go.svg"),
    "rs" => Some("file-icons/rust.svg"),
    "svelte" => Some("file-icons/svelte.svg"),
    "css" => Some("file-icons/css.svg"),
    "scss" | "sass" => Some("file-icons/sass.svg"),
    "html" | "htm" => Some("file-icons/html.svg"),
    "json" | "jsonc" => Some("file-icons/json.svg"),
    "md" | "mdx" => Some("file-icons/markdown.svg"),
    "yml" | "yaml" => Some("file-icons/yaml.svg"),
    "svg" => Some("file-icons/svg.svg"),
    "py" => Some("file-icons/python.svg"),
    "rb" => Some("file-icons/ruby.svg"),
    _ => None,
  }
}

pub fn file_icon_for_name(file_name: &str) -> Option<FileIcon> {
  file_icon_path_for_name_str(file_name).map(FileIcon::new)
}

pub fn file_icon_for_path(path: impl AsRef<Path>) -> Option<FileIcon> {
  let name = path.as_ref().file_name()?.to_str()?;
  file_icon_for_name(name)
}

pub fn file_icon_path_for_name(file_name: &str) -> Option<SharedString> {
  file_icon_path_for_name_str(file_name).map(SharedString::from)
}

pub fn file_icon_path_for_path(path: impl AsRef<Path>) -> Option<SharedString> {
  let name = path.as_ref().file_name()?.to_str()?;
  file_icon_path_for_name(name)
}
