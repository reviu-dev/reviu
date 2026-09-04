//! Application actions the workspace and its pages react to.

use gpui::actions;

actions!(
  workspace,
  [
    OpenProject,
    SaveFile,
    ShowCommandPalette,
    ShowFileSearch,
    CommitChanges
  ]
);
