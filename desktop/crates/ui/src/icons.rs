use gpui::SharedString;
use gpui_component::IconNamed;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiIconName {
  GitBranch,
  GitMerge,
}

impl IconNamed for UiIconName {
  fn path(self) -> SharedString {
    match self {
      UiIconName::GitBranch => "icons/git-branch.svg",
      UiIconName::GitMerge => "icons/git-merge.svg",
    }
    .into()
  }
}
