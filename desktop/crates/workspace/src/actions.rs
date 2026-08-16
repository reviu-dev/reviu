//! Application actions the workspace and its pages react to.

use gpui::actions;

actions!(
  workspace,
  [
    OpenRepository,
    SaveFile,
    ShowCommandPalette,
    ShowFileSearch,
    CommitChanges
  ]
);
