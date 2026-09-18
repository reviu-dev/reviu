use std::{borrow::Cow, collections::HashMap};

use editor::{
  AltLeft, AltRight, Backspace, BackspaceAll, BackspaceWord, CloseFind, CmdDown, CmdLeft, CmdRight,
  CmdUp, Copy, Cut, Delete, Down, End, Enter, Find, FindNext, FindPrevious, Home, Left, Outdent,
  Paste, Quit, Redo, Right, Save, SelectAll, SelectCmdDown, SelectCmdLeft, SelectCmdRight,
  SelectCmdUp, SelectDown, SelectLeft, SelectRight, SelectUp, SelectWordLeft, SelectWordRight,
  ShowCharacterPalette, Tab, ToggleFindCaseSensitive, ToggleFindRegex, ToggleFindWholeWord, Undo,
  Up,
};
use gpui::{Action, App, Global, KeyBinding, KeyContext, Keystroke, Window};
use ui::{COMMAND_PALETTE_CONTEXT, CommandPaletteCommand, CommandPaletteCommandId};

#[cfg(test)]
use gpui::Keymap;
#[cfg(test)]
use std::collections::HashSet;

use crate::config::ConfigStore;
use crate::project_search_view::PROJECT_SEARCH_CONTEXT;
use crate::{
  AcceptBothConflict, AddSelectionToAgent, CloseCenterPane, CloseCenterTab, CommentHunk,
  CommitChanges, DeleteSelectedFileItem, ForcePushChanges, JumpToLatestMessage, NewAgentSession,
  NewAgentWorktreeSession, NewFile, NewFileInFilesPanel, NextAnnotation, NextCenterTab,
  OpenFilesSidebar, OpenGitChangesSidebar, OpenGitHistorySidebar, OpenProject,
  OpenPullRequestSidebar, OpenReviewSidebar, OpenSettingsPage, PreviousAnnotation,
  PreviousCenterTab, PullChanges, PushChanges, RenameSelectedFileItem, RestoreFile, RestoreHunk,
  ReturnFocusToEditor, SaveFileAs, SendReviewCommentsToAgent, ShowBranchSwitcher,
  ShowCommandPalette, ShowFileSearch, ShowGlobalSearch, ToggleDiffView, ToggleFileStage,
  ToggleHideWhitespace, ToggleHunkStage,
};

pub const SHOW_COMMAND_PALETTE_SHORTCUT: &str = "cmd-k";
const SHORTCUT_KEYMAP_GENERATION_CONTEXT_KEY: &str = "workspace_shortcuts_generation";
pub const WORKSPACE_SHORTCUT_RECORDING_CONTEXT: &str = "WorkspaceShortcutRecording";

pub const WORKSPACE_CONTEXT: &str = "Workspace";
/// The right dock, so escape can mean "give the keyboard back" only in there.
pub const DOCK_PANEL_CONTEXT: &str = "DockPanel";
pub const FILES_TREE_CONTEXT: &str = "Tree && !Input";
pub const WORKSPACE_SESSION_CONTEXT: &str = "Workspace WorkspaceSession";

const FILE_SEARCH_CONTEXT: &str = "WorkspaceSession";
const GLOBAL_SEARCH_CONTEXT: &str = "WorkspaceSession";
const OPEN_PROJECT_CONTEXT: &str = "WorkspaceSession";
const COMMIT_CHANGES_CONTEXT: &str = "WorkspaceSession";
const COMMIT_CHANGES_DESCENDANT_FOCUS: &str = "CommitInput";
const PULL_CHANGES_CONTEXT: &str = "WorkspaceSession";
const PUSH_CHANGES_CONTEXT: &str = "WorkspaceSession";
const FORCE_PUSH_CHANGES_CONTEXT: &str = "WorkspaceSession";
const OPEN_SETTINGS_CONTEXT: &str = "Workspace";
const CENTER_TAB_CONTEXT: &str = "WorkspaceSession";
const SHOW_BRANCH_SWITCHER_CONTEXT: &str = "WorkspaceSession";
const OPEN_GIT_HISTORY_SIDEBAR_CONTEXT: &str = "WorkspaceSession";
const OPEN_GIT_CHANGES_SIDEBAR_CONTEXT: &str = "WorkspaceSession";
const OPEN_FILES_SIDEBAR_CONTEXT: &str = "WorkspaceSession";
const OPEN_REVIEW_SIDEBAR_CONTEXT: &str = "WorkspaceSession";
const OPEN_PULL_REQUEST_SIDEBAR_CONTEXT: &str = "WorkspaceSession";
const TOGGLE_DIFF_VIEW_CONTEXT: &str = "WorkspaceSession";
const REVIEW_ANNOTATION_CONTEXT: &str = "WorkspaceSession";
const HUNK_ACTION_CONTEXT: &str = "WorkspaceSession";
const HUNK_ACTION_SESSION_CONTEXT: &str = "WorkspaceSession";
const HUNK_OR_CONFLICT_ACTION_FOCUS: &str = "List || Editor";
const FILE_ACTION_FOCUS: &str = "List";
const COMMENT_HUNK_CONTEXT: &str = "WorkspaceSession";
const COMMENT_HUNK_DESCENDANT_FOCUS: &str = "List || Editor || Tree";

const ALL_WORKSPACE_ACTIVE_CONTEXTS: [&str; 1] = [WORKSPACE_SESSION_CONTEXT];

const FILE_SEARCH_ACTIVE_CONTEXTS: [&str; 1] = [WORKSPACE_SESSION_CONTEXT];
const GLOBAL_SEARCH_ACTIVE_CONTEXTS: [&str; 1] = [WORKSPACE_SESSION_CONTEXT];

const SESSION_ONLY_ACTIVE_CONTEXTS: [&str; 1] = [WORKSPACE_SESSION_CONTEXT];

const COMMENT_HUNK_ACTIVE_CONTEXTS: [&str; 1] = [WORKSPACE_SESSION_CONTEXT];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutId {
  ShowCommandPalette,
  NextCenterTab,
  PreviousCenterTab,
  CloseCenterTab,
  NewFile,
  SaveFileAs,
  ShowFileSearch,
  ShowGlobalSearch,
  OpenProject,
  CommitChanges,
  PullChanges,
  PushChanges,
  ForcePushChanges,
  OpenSettingsPage,
  ShowBranchSwitcher,
  OpenGitHistorySidebar,
  OpenGitChangesSidebar,
  OpenFilesSidebar,
  OpenReviewSidebar,
  OpenPullRequestSidebar,
  ToggleDiffView,
  ToggleHideWhitespace,
  PreviousAnnotation,
  NextAnnotation,
  CommentHunk,
  SendReviewCommentsToAgent,
  AddSelectionToAgent,
  JumpToLatestMessage,
  NewAgentSession,
  NewAgentWorktreeSession,
  ToggleHunkStage,
  RestoreHunk,
  ToggleFileStage,
  RestoreFile,
  AcceptBothConflict,
}

impl ShortcutId {
  pub fn storage_key(self) -> &'static str {
    match self {
      ShortcutId::ShowCommandPalette => "show_command_palette",
      ShortcutId::NextCenterTab => "next_center_tab",
      ShortcutId::PreviousCenterTab => "previous_center_tab",
      ShortcutId::CloseCenterTab => "close_center_tab",
      ShortcutId::NewFile => "new_file",
      ShortcutId::SaveFileAs => "save_file_as",
      ShortcutId::ShowFileSearch => "show_file_search",
      ShortcutId::ShowGlobalSearch => "show_global_search",
      ShortcutId::OpenProject => "open_project",
      ShortcutId::CommitChanges => "commit_changes",
      ShortcutId::PullChanges => "pull_changes",
      ShortcutId::PushChanges => "push_changes",
      ShortcutId::ForcePushChanges => "force_push_changes",
      ShortcutId::OpenSettingsPage => "open_settings_page",
      ShortcutId::ShowBranchSwitcher => "show_branch_switcher",
      ShortcutId::OpenGitHistorySidebar => "open_git_history_sidebar",
      ShortcutId::OpenGitChangesSidebar => "open_git_changes_sidebar",
      ShortcutId::OpenFilesSidebar => "open_files_sidebar",
      ShortcutId::OpenReviewSidebar => "open_review_sidebar",
      ShortcutId::OpenPullRequestSidebar => "open_pull_request_sidebar",
      ShortcutId::ToggleDiffView => "toggle_diff_view",
      ShortcutId::ToggleHideWhitespace => "toggle_hide_whitespace",
      ShortcutId::PreviousAnnotation => "previous_annotation",
      ShortcutId::NextAnnotation => "next_annotation",
      ShortcutId::CommentHunk => "comment_hunk",
      ShortcutId::SendReviewCommentsToAgent => "send_review_comments_to_agent",
      ShortcutId::AddSelectionToAgent => "add_selection_to_agent",
      ShortcutId::JumpToLatestMessage => "jump_to_latest_message",
      ShortcutId::NewAgentSession => "new_agent_session",
      ShortcutId::NewAgentWorktreeSession => "new_agent_worktree_session",
      ShortcutId::ToggleHunkStage => "toggle_hunk_stage",
      ShortcutId::RestoreHunk => "restore_hunk",
      ShortcutId::ToggleFileStage => "toggle_file_stage",
      ShortcutId::RestoreFile => "restore_file",
      ShortcutId::AcceptBothConflict => "accept_both_conflict",
    }
  }

  pub fn from_storage_key(value: &str) -> Option<Self> {
    match value {
      "show_command_palette" => Some(ShortcutId::ShowCommandPalette),
      "next_center_tab" => Some(ShortcutId::NextCenterTab),
      "previous_center_tab" => Some(ShortcutId::PreviousCenterTab),
      "close_center_tab" => Some(ShortcutId::CloseCenterTab),
      "new_file" => Some(ShortcutId::NewFile),
      "save_file_as" => Some(ShortcutId::SaveFileAs),
      "show_file_search" => Some(ShortcutId::ShowFileSearch),
      "show_global_search" => Some(ShortcutId::ShowGlobalSearch),
      "open_project" => Some(ShortcutId::OpenProject),
      "commit_changes" => Some(ShortcutId::CommitChanges),
      "pull_changes" => Some(ShortcutId::PullChanges),
      "push_changes" => Some(ShortcutId::PushChanges),
      "force_push_changes" => Some(ShortcutId::ForcePushChanges),
      "open_settings_page" => Some(ShortcutId::OpenSettingsPage),
      "show_branch_switcher" => Some(ShortcutId::ShowBranchSwitcher),
      "open_git_history_sidebar" => Some(ShortcutId::OpenGitHistorySidebar),
      "open_git_changes_sidebar" => Some(ShortcutId::OpenGitChangesSidebar),
      "open_files_sidebar" => Some(ShortcutId::OpenFilesSidebar),
      "open_review_sidebar" => Some(ShortcutId::OpenReviewSidebar),
      "open_pull_request_sidebar" => Some(ShortcutId::OpenPullRequestSidebar),
      "toggle_diff_view" => Some(ShortcutId::ToggleDiffView),
      "toggle_hide_whitespace" => Some(ShortcutId::ToggleHideWhitespace),
      "previous_annotation" => Some(ShortcutId::PreviousAnnotation),
      "next_annotation" => Some(ShortcutId::NextAnnotation),
      "comment_hunk" => Some(ShortcutId::CommentHunk),
      "send_review_comments_to_agent" => Some(ShortcutId::SendReviewCommentsToAgent),
      "add_selection_to_agent" => Some(ShortcutId::AddSelectionToAgent),
      "jump_to_latest_message" => Some(ShortcutId::JumpToLatestMessage),
      "new_agent_session" => Some(ShortcutId::NewAgentSession),
      "new_agent_worktree_session" => Some(ShortcutId::NewAgentWorktreeSession),
      "toggle_hunk_stage" => Some(ShortcutId::ToggleHunkStage),
      "restore_hunk" => Some(ShortcutId::RestoreHunk),
      "toggle_file_stage" => Some(ShortcutId::ToggleFileStage),
      "restore_file" => Some(ShortcutId::RestoreFile),
      "accept_both_conflict" => Some(ShortcutId::AcceptBothConflict),
      _ => None,
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutCategory {
  Core,
  Review,
  LocalGit,
  App,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutDefinition {
  pub id: ShortcutId,
  pub title: &'static str,
  pub description: &'static str,
  pub scope_label: &'static str,
  pub category: ShortcutCategory,
  pub keystroke: &'static str,
  pub context: &'static str,
  pub display_context: &'static str,
  pub active_contexts: &'static [&'static str],
}

const SHORTCUT_DEFINITIONS: [ShortcutDefinition; 35] = [
  ShortcutDefinition {
    id: ShortcutId::ShowCommandPalette,
    title: "Command Palette",
    description: "Open the command palette for the workspace.",
    scope_label: "Workspace",
    category: ShortcutCategory::Core,
    keystroke: SHOW_COMMAND_PALETTE_SHORTCUT,
    context: WORKSPACE_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &ALL_WORKSPACE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::NextCenterTab,
    title: "Next Tab",
    description: "Activate the next center tab.",
    scope_label: "Workspace",
    category: ShortcutCategory::Core,
    keystroke: "cmd-shift-]",
    context: CENTER_TAB_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::PreviousCenterTab,
    title: "Previous Tab",
    description: "Activate the previous center tab.",
    scope_label: "Workspace",
    category: ShortcutCategory::Core,
    keystroke: "cmd-shift-[",
    context: CENTER_TAB_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::CloseCenterTab,
    title: "Close Tab",
    description: "Close the active center tab.",
    scope_label: "Workspace",
    category: ShortcutCategory::Core,
    keystroke: "cmd-w",
    context: CENTER_TAB_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::NewFile,
    title: "New File",
    description: "Open a new untitled editor in the current project.",
    scope_label: "Projects",
    category: ShortcutCategory::Core,
    keystroke: "cmd-n",
    context: CENTER_TAB_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::SaveFileAs,
    title: "Save File As",
    description: "Save the active editor under a new name.",
    scope_label: "Projects",
    category: ShortcutCategory::Core,
    keystroke: "cmd-shift-s",
    context: CENTER_TAB_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ShowFileSearch,
    title: "File Search",
    description: "Open file search where file navigation is available.",
    scope_label: "PR Changes and Projects",
    category: ShortcutCategory::Core,
    keystroke: "cmd-p",
    context: FILE_SEARCH_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &FILE_SEARCH_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ShowGlobalSearch,
    title: "Project Search",
    description: "Search across files in the current project.",
    scope_label: "Projects",
    category: ShortcutCategory::Core,
    keystroke: "cmd-shift-f",
    context: GLOBAL_SEARCH_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &GLOBAL_SEARCH_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::PreviousAnnotation,
    title: "Previous Change",
    description: "Jump to the previous conflict or change in the diff.",
    scope_label: "Conflicts and changes, PR Changes, Projects",
    category: ShortcutCategory::Review,
    keystroke: "cmd-alt-up",
    context: REVIEW_ANNOTATION_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &COMMENT_HUNK_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::NextAnnotation,
    title: "Next Change",
    description: "Jump to the next conflict or change in the diff.",
    scope_label: "Conflicts and changes, PR Changes, Projects",
    category: ShortcutCategory::Review,
    keystroke: "cmd-alt-down",
    context: REVIEW_ANNOTATION_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &COMMENT_HUNK_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::CommentHunk,
    title: "Comment Hunk",
    description: "Start a review comment on the focused hunk.",
    scope_label: "PR Changes and Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-alt-enter",
    context: COMMENT_HUNK_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &COMMENT_HUNK_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::SendReviewCommentsToAgent,
    title: "Send Review Comments To Agent",
    description: "Send all local review comments to the in-app agent.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-a",
    context: "WorkspaceSession",
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::AddSelectionToAgent,
    title: "Send Selection To Agent",
    description: "Attach the selected diff lines to the agent message as context.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-l",
    context: HUNK_ACTION_SESSION_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::NewAgentSession,
    title: "New Chat",
    description: "Start a new agent chat in the current project.",
    scope_label: "Projects",
    category: ShortcutCategory::Core,
    keystroke: "cmd-t",
    context: "WorkspaceSession",
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::NewAgentWorktreeSession,
    title: "New Worktree Chat",
    description: "Start a new agent chat in its own git worktree.",
    scope_label: "Projects",
    category: ShortcutCategory::Core,
    keystroke: "cmd-shift-t",
    context: "WorkspaceSession",
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::JumpToLatestMessage,
    title: "Jump to Latest Message",
    description: "Scroll the conversation to the newest message and keep following the reply.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-j",
    context: "WorkspaceSession",
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ToggleHunkStage,
    title: "Stage / Unstage Hunk · Accept Current",
    description: "Stage the focused hunk (or unstage it if staged). On a file with unresolved conflicts, accept the active conflict's current change instead.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "shift-enter",
    context: HUNK_ACTION_SESSION_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::RestoreHunk,
    title: "Restore Hunk · Accept Incoming",
    description: "Discard the focused hunk and restore the file. On a file with unresolved conflicts, accept the active conflict's incoming change instead.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "shift-backspace",
    context: HUNK_ACTION_SESSION_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ToggleFileStage,
    title: "Stage / Unstage File",
    description: "Stage the selected file, or unstage it if already staged.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-enter",
    context: HUNK_ACTION_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::RestoreFile,
    title: "Restore File",
    description: "Discard all changes in the selected file.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-backspace",
    context: HUNK_ACTION_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::AcceptBothConflict,
    title: "Accept Both Conflict Changes",
    description: "Keep the current and incoming changes in the active conflict.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-enter",
    context: HUNK_ACTION_SESSION_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ToggleDiffView,
    title: "Toggle Diff View",
    description: "Switch between inline and split diff view.",
    scope_label: "PR Changes and Projects",
    category: ShortcutCategory::Review,
    keystroke: "cmd-/",
    context: TOGGLE_DIFF_VIEW_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &COMMENT_HUNK_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ToggleHideWhitespace,
    title: "Toggle Hide Whitespace",
    description: "Show or hide whitespace-only changes in the diff.",
    scope_label: "PR Changes and Projects",
    category: ShortcutCategory::Review,
    keystroke: "cmd-alt-/",
    context: TOGGLE_DIFF_VIEW_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &COMMENT_HUNK_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenProject,
    title: "Add Project",
    description: "Add a local project.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-o",
    context: OPEN_PROJECT_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::CommitChanges,
    title: "Commit Changes",
    description: "Commit the staged changes.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-enter",
    context: COMMIT_CHANGES_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::PullChanges,
    title: "Pull Changes",
    description: "Pull the current branch from its upstream remote.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-u",
    context: PULL_CHANGES_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::PushChanges,
    title: "Push Changes",
    description: "Push the current branch to its upstream remote.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-y",
    context: PUSH_CHANGES_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ForcePushChanges,
    title: "Force Push Changes",
    description: "Force push the current branch to its upstream remote with lease.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-y",
    context: FORCE_PUSH_CHANGES_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ShowBranchSwitcher,
    title: "Switch Branch",
    description: "Switch branch.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-b",
    context: SHOW_BRANCH_SWITCHER_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenGitHistorySidebar,
    title: "Focus History Tree",
    description: "Switch the Git sidebar to History and focus the commit tree.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-h",
    context: OPEN_GIT_HISTORY_SIDEBAR_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenGitChangesSidebar,
    title: "Focus Changes List",
    description: "Switch the Git sidebar to Changes and focus the file list.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-c",
    context: OPEN_GIT_CHANGES_SIDEBAR_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenFilesSidebar,
    title: "Show Files",
    description: "Switch the right panel to the project file tree.",
    scope_label: "Projects",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-e",
    context: OPEN_FILES_SIDEBAR_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenReviewSidebar,
    title: "Show Review",
    description: "Switch the right panel to the review waiting to be sent.",
    scope_label: "Projects",
    category: ShortcutCategory::Review,
    keystroke: "cmd-shift-r",
    context: OPEN_REVIEW_SIDEBAR_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenPullRequestSidebar,
    title: "Show Pull Request",
    description: "Switch the right panel to the pull request of the current branch.",
    scope_label: "Projects",
    category: ShortcutCategory::Review,
    keystroke: "cmd-shift-p",
    context: OPEN_PULL_REQUEST_SIDEBAR_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenSettingsPage,
    title: "Open Settings",
    description: "Open settings over the workspace.",
    scope_label: "Workspace",
    category: ShortcutCategory::App,
    keystroke: "cmd-,",
    context: OPEN_SETTINGS_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &ALL_WORKSPACE_ACTIVE_CONTEXTS,
  },
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShortcutOverrides {
  entries: HashMap<ShortcutId, String>,
}

impl Global for ShortcutOverrides {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ShortcutBindingState {
  generation: u32,
  base_app_key_bindings_installed: bool,
}

impl Global for ShortcutBindingState {}

impl ShortcutOverrides {
  pub fn get(cx: &App) -> Self {
    cx.try_global::<Self>().cloned().unwrap_or_default()
  }

  fn keystroke_for(&self, id: ShortcutId) -> Option<&str> {
    self.entries.get(&id).map(String::as_str)
  }

  pub(crate) fn from_entries(entries: HashMap<ShortcutId, String>) -> Self {
    Self { entries }
  }

  pub fn contains(&self, id: ShortcutId) -> bool {
    self.entries.contains_key(&id)
  }
}

impl ShortcutBindingState {
  fn current_generation(cx: &App) -> u32 {
    cx.try_global::<Self>()
      .copied()
      .unwrap_or_default()
      .generation
  }

  fn advance_generation(cx: &mut App) -> u32 {
    let mut state = cx.try_global::<Self>().copied().unwrap_or_default();

    if !state.base_app_key_bindings_installed {
      cx.bind_keys(default_app_key_bindings());
      state.base_app_key_bindings_installed = true;
    }

    state.generation = state.generation.saturating_add(1);
    let generation = state.generation;
    cx.set_global(state);
    generation
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutOverrideError {
  MissingModifier,
  ReservedBinding { title: &'static str },
  ShortcutConflict { shortcut_id: ShortcutId },
}

impl ShortcutOverrideError {
  pub fn message(self) -> String {
    match self {
      ShortcutOverrideError::MissingModifier => {
        "Shortcut must include Command, Control, Alt, or a function key.".to_string()
      }
      ShortcutOverrideError::ReservedBinding { title } => {
        format!("Conflicts with the reserved app shortcut \"{}\".", title)
      }
      ShortcutOverrideError::ShortcutConflict { shortcut_id } => {
        format!(
          "Conflicts with \"{}\" in an overlapping context.",
          shortcut_definition(shortcut_id).title
        )
      }
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReservedAppBinding {
  title: &'static str,
  keystroke: &'static str,
}

const RESERVED_APP_BINDINGS: [ReservedAppBinding; 39] = [
  ReservedAppBinding {
    title: "Confirm",
    keystroke: "enter",
  },
  ReservedAppBinding {
    title: "Next Field",
    keystroke: "tab",
  },
  ReservedAppBinding {
    title: "Delete Backward",
    keystroke: "backspace",
  },
  ReservedAppBinding {
    title: "Delete Word Backward",
    keystroke: "alt-backspace",
  },
  ReservedAppBinding {
    title: "Delete to Start of Line",
    keystroke: "cmd-backspace",
  },
  ReservedAppBinding {
    title: "Delete Forward",
    keystroke: "delete",
  },
  ReservedAppBinding {
    title: "Move Up",
    keystroke: "up",
  },
  ReservedAppBinding {
    title: "Move Down",
    keystroke: "down",
  },
  ReservedAppBinding {
    title: "Move Left",
    keystroke: "left",
  },
  ReservedAppBinding {
    title: "Move Word Left",
    keystroke: "alt-left",
  },
  ReservedAppBinding {
    title: "Move to Line Start",
    keystroke: "cmd-left",
  },
  ReservedAppBinding {
    title: "Move Right",
    keystroke: "right",
  },
  ReservedAppBinding {
    title: "Move Word Right",
    keystroke: "alt-right",
  },
  ReservedAppBinding {
    title: "Move to Line End",
    keystroke: "cmd-right",
  },
  ReservedAppBinding {
    title: "Move to Document Start",
    keystroke: "cmd-up",
  },
  ReservedAppBinding {
    title: "Move to Document End",
    keystroke: "cmd-down",
  },
  ReservedAppBinding {
    title: "Select Up",
    keystroke: "shift-up",
  },
  ReservedAppBinding {
    title: "Select Down",
    keystroke: "shift-down",
  },
  ReservedAppBinding {
    title: "Select to Line Start",
    keystroke: "shift-cmd-left",
  },
  ReservedAppBinding {
    title: "Select to Line End",
    keystroke: "shift-cmd-right",
  },
  ReservedAppBinding {
    title: "Select to Document Start",
    keystroke: "shift-cmd-up",
  },
  ReservedAppBinding {
    title: "Select to Document End",
    keystroke: "shift-cmd-down",
  },
  ReservedAppBinding {
    title: "Select Left",
    keystroke: "shift-left",
  },
  ReservedAppBinding {
    title: "Select Word Left",
    keystroke: "shift-alt-left",
  },
  ReservedAppBinding {
    title: "Select Right",
    keystroke: "shift-right",
  },
  ReservedAppBinding {
    title: "Select Word Right",
    keystroke: "shift-alt-right",
  },
  ReservedAppBinding {
    title: "Select All",
    keystroke: "cmd-a",
  },
  ReservedAppBinding {
    title: "Paste",
    keystroke: "cmd-v",
  },
  ReservedAppBinding {
    title: "Copy",
    keystroke: "cmd-c",
  },
  ReservedAppBinding {
    title: "Cut",
    keystroke: "cmd-x",
  },
  ReservedAppBinding {
    title: "Undo",
    keystroke: "cmd-z",
  },
  ReservedAppBinding {
    title: "Redo",
    keystroke: "cmd-shift-z",
  },
  ReservedAppBinding {
    title: "Save",
    keystroke: "cmd-s",
  },
  ReservedAppBinding {
    title: "Find",
    keystroke: "cmd-f",
  },
  ReservedAppBinding {
    title: "Close Find",
    keystroke: "escape",
  },
  ReservedAppBinding {
    title: "Home",
    keystroke: "home",
  },
  ReservedAppBinding {
    title: "End",
    keystroke: "end",
  },
  ReservedAppBinding {
    title: "Character Palette",
    keystroke: "ctrl-cmd-space",
  },
  ReservedAppBinding {
    title: "Quit",
    keystroke: "cmd-q",
  },
];

impl ShortcutDefinition {
  fn key_binding_with_keystroke(self, keystroke: &str, generation: u32) -> KeyBinding {
    let base = shortcut_binding_context(&guarded_shortcut_context(self.context), generation);
    let context = if let Some(descendant_focus) = self.descendant_focus() {
      format!("({}) > ({})", base, descendant_focus)
    } else {
      base
    };

    match self.id {
      ShortcutId::ShowCommandPalette => {
        KeyBinding::new(keystroke, ShowCommandPalette, Some(&context))
      }
      ShortcutId::NextCenterTab => KeyBinding::new(keystroke, NextCenterTab, Some(&context)),
      ShortcutId::PreviousCenterTab => {
        KeyBinding::new(keystroke, PreviousCenterTab, Some(&context))
      }
      ShortcutId::CloseCenterTab => KeyBinding::new(keystroke, CloseCenterTab, Some(&context)),
      ShortcutId::NewFile => KeyBinding::new(keystroke, NewFile, Some(&context)),
      ShortcutId::SaveFileAs => KeyBinding::new(keystroke, SaveFileAs, Some(&context)),
      ShortcutId::ShowFileSearch => KeyBinding::new(keystroke, ShowFileSearch, Some(&context)),
      ShortcutId::ShowGlobalSearch => KeyBinding::new(keystroke, ShowGlobalSearch, Some(&context)),
      ShortcutId::OpenProject => KeyBinding::new(keystroke, OpenProject, Some(&context)),
      ShortcutId::CommitChanges => KeyBinding::new(keystroke, CommitChanges, Some(&context)),
      ShortcutId::PullChanges => KeyBinding::new(keystroke, PullChanges, Some(&context)),
      ShortcutId::PushChanges => KeyBinding::new(keystroke, PushChanges, Some(&context)),
      ShortcutId::ForcePushChanges => KeyBinding::new(keystroke, ForcePushChanges, Some(&context)),
      ShortcutId::OpenSettingsPage => KeyBinding::new(keystroke, OpenSettingsPage, Some(&context)),
      ShortcutId::ShowBranchSwitcher => {
        KeyBinding::new(keystroke, ShowBranchSwitcher, Some(&context))
      }
      ShortcutId::OpenGitHistorySidebar => {
        KeyBinding::new(keystroke, OpenGitHistorySidebar, Some(&context))
      }
      ShortcutId::OpenGitChangesSidebar => {
        KeyBinding::new(keystroke, OpenGitChangesSidebar, Some(&context))
      }
      ShortcutId::OpenFilesSidebar => KeyBinding::new(keystroke, OpenFilesSidebar, Some(&context)),
      ShortcutId::OpenReviewSidebar => {
        KeyBinding::new(keystroke, OpenReviewSidebar, Some(&context))
      }
      ShortcutId::OpenPullRequestSidebar => {
        KeyBinding::new(keystroke, OpenPullRequestSidebar, Some(&context))
      }
      ShortcutId::ToggleDiffView => KeyBinding::new(keystroke, ToggleDiffView, Some(&context)),
      ShortcutId::ToggleHideWhitespace => {
        KeyBinding::new(keystroke, ToggleHideWhitespace, Some(&context))
      }
      ShortcutId::PreviousAnnotation => {
        KeyBinding::new(keystroke, PreviousAnnotation, Some(&context))
      }
      ShortcutId::NextAnnotation => KeyBinding::new(keystroke, NextAnnotation, Some(&context)),
      ShortcutId::CommentHunk => KeyBinding::new(keystroke, CommentHunk, Some(&context)),
      ShortcutId::SendReviewCommentsToAgent => {
        KeyBinding::new(keystroke, SendReviewCommentsToAgent, Some(&context))
      }
      ShortcutId::AddSelectionToAgent => {
        KeyBinding::new(keystroke, AddSelectionToAgent, Some(&context))
      }
      ShortcutId::JumpToLatestMessage => {
        KeyBinding::new(keystroke, JumpToLatestMessage, Some(&context))
      }
      ShortcutId::NewAgentSession => KeyBinding::new(keystroke, NewAgentSession, Some(&context)),
      ShortcutId::NewAgentWorktreeSession => {
        KeyBinding::new(keystroke, NewAgentWorktreeSession, Some(&context))
      }
      ShortcutId::ToggleHunkStage => KeyBinding::new(keystroke, ToggleHunkStage, Some(&context)),
      ShortcutId::RestoreHunk => KeyBinding::new(keystroke, RestoreHunk, Some(&context)),
      ShortcutId::ToggleFileStage => KeyBinding::new(keystroke, ToggleFileStage, Some(&context)),
      ShortcutId::RestoreFile => KeyBinding::new(keystroke, RestoreFile, Some(&context)),
      ShortcutId::AcceptBothConflict => {
        KeyBinding::new(keystroke, AcceptBothConflict, Some(&context))
      }
    }
  }

  fn descendant_focus(self) -> Option<&'static str> {
    match self.id {
      ShortcutId::CommentHunk => Some(COMMENT_HUNK_DESCENDANT_FOCUS),
      ShortcutId::ToggleHunkStage | ShortcutId::RestoreHunk | ShortcutId::AcceptBothConflict => {
        Some(HUNK_OR_CONFLICT_ACTION_FOCUS)
      }
      ShortcutId::ToggleFileStage | ShortcutId::RestoreFile => Some(FILE_ACTION_FOCUS),
      ShortcutId::CommitChanges => Some(COMMIT_CHANGES_DESCENDANT_FOCUS),
      _ => None,
    }
  }

  fn default_keystroke(self) -> Keystroke {
    Keystroke::parse(self.keystroke).expect("valid shortcut definition keystroke")
  }
}

impl ShortcutCategory {
  pub fn title(self) -> &'static str {
    match self {
      ShortcutCategory::Core => "Core",
      ShortcutCategory::Review => "Review",
      ShortcutCategory::LocalGit => "Local Git",
      ShortcutCategory::App => "App",
    }
  }
}

pub fn shortcut_definitions() -> &'static [ShortcutDefinition] {
  &SHORTCUT_DEFINITIONS
}

pub fn shortcut_definition(id: ShortcutId) -> &'static ShortcutDefinition {
  shortcut_definitions()
    .iter()
    .find(|definition| definition.id == id)
    .expect("shortcut definition must exist")
}

pub fn load_shortcut_overrides() -> ShortcutOverrides {
  ShortcutOverrides {
    entries: ConfigStore::load_shortcut_overrides(),
  }
}

pub fn set_shortcut_override(cx: &mut App, id: ShortcutId, keystroke: &Keystroke) {
  let keystroke = serialize_keystroke(keystroke);
  let keystroke = (keystroke != shortcut_definition(id).keystroke).then_some(keystroke.as_str());
  save_shortcut_override(cx, id, keystroke);
}

pub fn clear_shortcut_override(cx: &mut App, id: ShortcutId) {
  save_shortcut_override(cx, id, None);
}

fn save_shortcut_override(cx: &mut App, id: ShortcutId, keystroke: Option<&str>) {
  let kind = crate::config_reload::ConfigKind::Keybindings;
  match crate::keybindings_file::update_override(id, keystroke) {
    Ok(text) => crate::config_reload::saved(kind, text, cx),
    Err(error) => crate::config_reload::save_failed(kind, error, cx),
  }
}

pub fn shortcut_is_customized(cx: &App, id: ShortcutId) -> bool {
  ShortcutOverrides::get(cx).contains(id)
}

pub fn shortcut_keystroke(id: ShortcutId) -> Keystroke {
  shortcut_definition(id).default_keystroke()
}

pub fn validate_shortcut_override(
  id: ShortcutId,
  keystroke: &Keystroke,
  overrides: &ShortcutOverrides,
) -> Result<(), ShortcutOverrideError> {
  if !has_shortcut_modifier(keystroke) {
    return Err(ShortcutOverrideError::MissingModifier);
  }

  let keystroke_text = serialize_keystroke(keystroke);
  if let Some(binding) = RESERVED_APP_BINDINGS
    .iter()
    .find(|binding| binding.keystroke == keystroke_text)
  {
    return Err(ShortcutOverrideError::ReservedBinding {
      title: binding.title,
    });
  }

  for definition in shortcut_definitions() {
    if definition.id == id {
      continue;
    }

    if effective_shortcut_keystroke_text(definition.id, overrides) == keystroke_text
      && active_contexts_overlap(shortcut_definition(id), definition)
    {
      return Err(ShortcutOverrideError::ShortcutConflict {
        shortcut_id: definition.id,
      });
    }
  }

  Ok(())
}

pub fn resolved_shortcut_keystroke_in(
  cx: &App,
  window: &Window,
  id: ShortcutId,
  context: &str,
) -> Keystroke {
  resolved_shortcut_keystroke_in_generation(
    window,
    id,
    context,
    ShortcutBindingState::current_generation(cx),
  )
}

fn resolved_shortcut_keystroke_in_generation(
  window: &Window,
  id: ShortcutId,
  context: &str,
  generation: u32,
) -> Keystroke {
  let context = KeyContext::parse(&key_context_with_shortcut_generation(context, generation)).ok();

  with_shortcut_action(id, |action| {
    context
      .and_then(|context| window.highest_precedence_binding_for_action_in_context(action, context))
      .or_else(|| window.highest_precedence_binding_for_action(action))
      .and_then(|binding| {
        binding
          .keystrokes()
          .first()
          .map(|keystroke| keystroke.inner().clone())
      })
      .unwrap_or_else(|| shortcut_keystroke(id))
  })
}

/// The shortcut that runs a palette command, when one exists. Exhaustive on
/// purpose: a new command has to say whether a key reaches it.
fn palette_command_shortcut(command: CommandPaletteCommandId) -> Option<ShortcutId> {
  use CommandPaletteCommandId as Command;
  match command {
    Command::Commit => Some(ShortcutId::CommitChanges),
    Command::Push => Some(ShortcutId::PushChanges),
    Command::ForcePush => Some(ShortcutId::ForcePushChanges),
    Command::Pull => Some(ShortcutId::PullChanges),
    Command::SwitchBranch => Some(ShortcutId::ShowBranchSwitcher),
    Command::OpenProject => Some(ShortcutId::OpenProject),
    Command::NewFile => Some(ShortcutId::NewFile),
    Command::SaveFileAs => Some(ShortcutId::SaveFileAs),
    Command::OpenSettingsPage => Some(ShortcutId::OpenSettingsPage),
    Command::SendReview => Some(ShortcutId::SendReviewCommentsToAgent),
    // One key toggles either way, so both rows show it.
    Command::StageSelectedFile | Command::UnstageSelectedFile => Some(ShortcutId::ToggleFileStage),
    Command::NewTerminal => None,
    Command::ShowChanges => Some(ShortcutId::OpenGitChangesSidebar),
    Command::ShowReview => Some(ShortcutId::OpenReviewSidebar),
    Command::ShowFiles => Some(ShortcutId::OpenFilesSidebar),
    Command::ShowHistory => Some(ShortcutId::OpenGitHistorySidebar),
    Command::ShowPullRequest => Some(ShortcutId::OpenPullRequestSidebar),
    Command::ShowFileSearch => Some(ShortcutId::ShowFileSearch),
    Command::ShowGlobalSearch => Some(ShortcutId::ShowGlobalSearch),
    Command::ToggleDiffView => Some(ShortcutId::ToggleDiffView),
    Command::ToggleHideWhitespace => Some(ShortcutId::ToggleHideWhitespace),
    Command::SendSelectionToAgent => Some(ShortcutId::AddSelectionToAgent),
    Command::JumpToLatestMessage => Some(ShortcutId::JumpToLatestMessage),
    Command::NewAgentSession => Some(ShortcutId::NewAgentSession),
    Command::NewAgentWorktreeSession => Some(ShortcutId::NewAgentWorktreeSession),
    Command::SwitchProject
    | Command::ForgetProject
    | Command::CheckoutDetached
    | Command::ContinueRebase
    | Command::SkipRebase
    | Command::UndoLastCommit
    | Command::Amend
    | Command::DiscardReview
    | Command::SubmitPullRequestReview
    | Command::DiscardPullRequestReview
    | Command::AcceptAllCurrentConflicts
    | Command::AcceptAllIncomingConflicts
    | Command::CreateBranch
    | Command::CreateBranchFrom
    | Command::DeleteBranch
    | Command::MergeBranch
    | Command::AbortMerge
    | Command::RebaseBranch
    | Command::InteractiveRebase
    | Command::InteractiveRebaseOntoBranch
    | Command::InteractiveRebaseEditBranch
    | Command::InteractiveRebaseHeadCount
    | Command::AbortRebase
    | Command::CreatePullRequest
    | Command::OpenPullRequest
    | Command::CherryPick
    | Command::StageAll
    | Command::UnstageAll
    | Command::RestoreAll
    | Command::Fetch
    | Command::Stash
    | Command::StashIncludeUntracked
    | Command::ApplyStash
    | Command::DropStash
    | Command::PopStash
    | Command::OpenGithubFromUrl
    | Command::OpenGitConfigPage
    | Command::OpenSettingsFile
    | Command::OpenKeybindingsFile
    | Command::OpenBillingPage
    | Command::OpenAboutPage
    | Command::OpenLogs
    | Command::RevealLogs
    | Command::SendFeedback
    | Command::SignIn
    | Command::SignOut
    | Command::OpenBrowserExtensions => None,
  }
}

/// Stamps each command with the key that runs it, so a palette row teaches the
/// shortcut instead of hiding it in Settings.
pub fn with_palette_keybindings(
  commands: Vec<CommandPaletteCommand>,
  window: &Window,
  cx: &App,
) -> Vec<CommandPaletteCommand> {
  commands
    .into_iter()
    .map(|command| match palette_command_shortcut(command.id) {
      Some(shortcut) => {
        command.keybinding(resolved_display_shortcut_keystroke_in(cx, window, shortcut))
      }
      None => command,
    })
    .collect()
}

pub fn resolved_display_shortcut_keystroke_in(
  cx: &App,
  window: &Window,
  id: ShortcutId,
) -> Keystroke {
  let definition = shortcut_definition(id);
  resolved_shortcut_keystroke_in(cx, window, id, definition.display_context)
}

pub fn install_workspace_shortcuts(cx: &mut App) {
  let overrides = ShortcutOverrides::get(cx);
  let generation = ShortcutBindingState::advance_generation(cx);
  cx.bind_keys(workspace_key_bindings_with_overrides_and_generation(
    &overrides, generation,
  ));
}

pub fn install_app_key_bindings(cx: &mut App) {
  install_workspace_shortcuts(cx);
}

#[cfg(test)]
pub fn workspace_key_bindings() -> Vec<KeyBinding> {
  workspace_key_bindings_with_overrides(&ShortcutOverrides::default())
}

#[cfg(test)]
pub fn workspace_key_bindings_with_overrides(overrides: &ShortcutOverrides) -> Vec<KeyBinding> {
  workspace_key_bindings_with_overrides_and_generation(overrides, 0)
}

fn workspace_key_bindings_with_overrides_and_generation(
  overrides: &ShortcutOverrides,
  generation: u32,
) -> Vec<KeyBinding> {
  let mut bindings: Vec<KeyBinding> = shortcut_definitions()
    .iter()
    .copied()
    .map(|definition| {
      definition.key_binding_with_keystroke(
        effective_shortcut_keystroke_text(definition.id, overrides).as_ref(),
        generation,
      )
    })
    .collect();
  bindings.extend(fixed_workspace_key_bindings());
  bindings
}

fn fixed_workspace_key_bindings() -> Vec<KeyBinding> {
  vec![KeyBinding::new(
    "escape",
    CloseCenterPane,
    Some(&guarded_shortcut_context(CENTER_TAB_CONTEXT)),
  )]
}

pub fn current_workspace_key_context(cx: &App) -> String {
  workspace_key_context_with_generation(ShortcutBindingState::current_generation(cx))
}

fn workspace_key_context_with_generation(generation: u32) -> String {
  key_context_with_shortcut_generation(WORKSPACE_SESSION_CONTEXT, generation)
}

fn key_context_with_shortcut_generation(context: &str, generation: u32) -> String {
  format!("{context} {SHORTCUT_KEYMAP_GENERATION_CONTEXT_KEY}={generation}")
}

fn shortcut_binding_context(context: &str, generation: u32) -> String {
  format!("({context}) && {SHORTCUT_KEYMAP_GENERATION_CONTEXT_KEY} == {generation}")
}

fn guarded_shortcut_context(context: &str) -> String {
  format!("({context}) && !{COMMAND_PALETTE_CONTEXT} && !{WORKSPACE_SHORTCUT_RECORDING_CONTEXT}")
}

fn default_app_key_bindings() -> Vec<KeyBinding> {
  let mut bindings = vec![
    KeyBinding::new("enter", Enter, None),
    KeyBinding::new("tab", Tab, None),
    KeyBinding::new("shift-tab", Outdent, Some("Editor && !Input")),
    KeyBinding::new("backspace", Backspace, None),
    KeyBinding::new("alt-backspace", BackspaceWord, None),
    KeyBinding::new("cmd-backspace", BackspaceAll, None),
    KeyBinding::new("delete", Delete, None),
    KeyBinding::new("up", Up, None),
    KeyBinding::new("down", Down, None),
    KeyBinding::new("left", Left, None),
    KeyBinding::new("alt-left", AltLeft, None),
    KeyBinding::new("cmd-left", CmdLeft, None),
    KeyBinding::new("right", Right, None),
    KeyBinding::new("alt-right", AltRight, None),
    KeyBinding::new("cmd-right", CmdRight, None),
    KeyBinding::new("cmd-up", CmdUp, None),
    KeyBinding::new("cmd-down", CmdDown, None),
    KeyBinding::new("shift-up", SelectUp, None),
    KeyBinding::new("shift-down", SelectDown, None),
    KeyBinding::new("shift-cmd-left", SelectCmdLeft, None),
    KeyBinding::new("shift-cmd-right", SelectCmdRight, None),
    KeyBinding::new("shift-cmd-up", SelectCmdUp, None),
    KeyBinding::new("shift-cmd-down", SelectCmdDown, None),
    KeyBinding::new("shift-left", SelectLeft, None),
    KeyBinding::new("shift-alt-left", SelectWordLeft, None),
    KeyBinding::new("shift-right", SelectRight, None),
    KeyBinding::new("shift-alt-right", SelectWordRight, None),
    KeyBinding::new("cmd-a", SelectAll, None),
    KeyBinding::new("cmd-v", Paste, None),
    KeyBinding::new("cmd-c", Copy, None),
    KeyBinding::new("cmd-x", Cut, None),
    KeyBinding::new("cmd-z", Undo, None),
    KeyBinding::new("cmd-shift-z", Redo, None),
    KeyBinding::new("cmd-s", Save, None),
    KeyBinding::new("cmd-f", Find, None),
    KeyBinding::new("cmd-g", FindNext, Some("Editor")),
    KeyBinding::new("cmd-shift-g", FindPrevious, Some("Editor")),
    KeyBinding::new("cmd-g", FindNext, Some(PROJECT_SEARCH_CONTEXT)),
    KeyBinding::new("cmd-shift-g", FindPrevious, Some(PROJECT_SEARCH_CONTEXT)),
    KeyBinding::new("alt-cmd-c", ToggleFindCaseSensitive, Some("Editor")),
    KeyBinding::new("alt-cmd-w", ToggleFindWholeWord, Some("Editor")),
    KeyBinding::new("alt-cmd-x", ToggleFindRegex, Some("Editor")),
    KeyBinding::new("escape", CloseFind, Some("Editor")),
    KeyBinding::new("escape", ReturnFocusToEditor, Some(DOCK_PANEL_CONTEXT)),
    KeyBinding::new("cmd-n", NewFileInFilesPanel, Some(FILES_TREE_CONTEXT)),
    KeyBinding::new("f2", RenameSelectedFileItem, Some(FILES_TREE_CONTEXT)),
    KeyBinding::new("delete", DeleteSelectedFileItem, Some(FILES_TREE_CONTEXT)),
    KeyBinding::new("home", Home, None),
    KeyBinding::new("end", End, None),
    KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, None),
    KeyBinding::new("cmd-q", Quit, None),
  ];
  bindings.extend(terminal_key_bindings());
  bindings
}

fn terminal_key_bindings() -> Vec<KeyBinding> {
  let mut bindings = [
    ("enter", "enter"),
    ("shift-enter", "shift-enter"),
    ("alt-enter", "alt-enter"),
    ("escape", "escape"),
    ("tab", "tab"),
    ("shift-tab", "shift-tab"),
    ("backspace", "backspace"),
    ("shift-backspace", "shift-backspace"),
    ("alt-backspace", "alt-backspace"),
    ("cmd-backspace", "cmd-backspace"),
    ("ctrl-backspace", "ctrl-w"),
    ("delete", "delete"),
    ("up", "up"),
    ("down", "down"),
    ("left", "left"),
    ("right", "right"),
    ("shift-left", "shift-left"),
    ("shift-right", "shift-right"),
    ("alt-left", "alt-b"),
    ("alt-right", "alt-f"),
    ("cmd-left", "ctrl-a"),
    ("cmd-right", "ctrl-e"),
    ("home", "home"),
    ("end", "end"),
    ("pageup", "pageup"),
    ("pagedown", "pagedown"),
  ]
  .into_iter()
  .map(|(keystroke, terminal_keystroke)| {
    KeyBinding::new(
      keystroke,
      terminal::SendKeystroke(terminal_keystroke.to_string()),
      Some(terminal::TERMINAL_CONTEXT),
    )
  })
  .collect::<Vec<_>>();

  let (find, next_match, previous_match) = if cfg!(target_os = "macos") {
    ("cmd-f", "cmd-g", "cmd-shift-g")
  } else {
    ("ctrl-f", "ctrl-g", "ctrl-shift-g")
  };
  bindings.extend([
    KeyBinding::new(
      "shift-up",
      terminal::ScrollLineUp,
      Some(terminal::TERMINAL_CONTEXT),
    ),
    KeyBinding::new(
      "shift-down",
      terminal::ScrollLineDown,
      Some(terminal::TERMINAL_CONTEXT),
    ),
    KeyBinding::new(
      "shift-pageup",
      terminal::ScrollPageUp,
      Some(terminal::TERMINAL_CONTEXT),
    ),
    KeyBinding::new(
      "shift-pagedown",
      terminal::ScrollPageDown,
      Some(terminal::TERMINAL_CONTEXT),
    ),
    KeyBinding::new(
      "shift-home",
      terminal::ScrollToTop,
      Some(terminal::TERMINAL_CONTEXT),
    ),
    KeyBinding::new(
      "shift-end",
      terminal::ScrollToBottom,
      Some(terminal::TERMINAL_CONTEXT),
    ),
    KeyBinding::new(find, terminal::OpenSearch, Some(terminal::TERMINAL_CONTEXT)),
    KeyBinding::new(
      find,
      terminal::OpenSearch,
      Some(terminal::TERMINAL_SEARCH_CONTEXT),
    ),
    KeyBinding::new(
      "escape",
      terminal::CloseSearch,
      Some(terminal::TERMINAL_SEARCH_CONTEXT),
    ),
    KeyBinding::new(
      next_match,
      terminal::SearchNext,
      Some(terminal::TERMINAL_SEARCH_CONTEXT),
    ),
    KeyBinding::new(
      previous_match,
      terminal::SearchPrevious,
      Some(terminal::TERMINAL_SEARCH_CONTEXT),
    ),
  ]);

  if cfg!(target_os = "macos") {
    bindings.extend([
      KeyBinding::new(
        "cmd-up",
        terminal::ScrollPageUp,
        Some(terminal::TERMINAL_CONTEXT),
      ),
      KeyBinding::new(
        "cmd-down",
        terminal::ScrollPageDown,
        Some(terminal::TERMINAL_CONTEXT),
      ),
      KeyBinding::new(
        "cmd-home",
        terminal::ScrollToTop,
        Some(terminal::TERMINAL_CONTEXT),
      ),
      KeyBinding::new(
        "cmd-end",
        terminal::ScrollToBottom,
        Some(terminal::TERMINAL_CONTEXT),
      ),
      KeyBinding::new(
        "cmd-c",
        terminal::SendKeystroke("cmd-c".to_string()),
        Some(terminal::TERMINAL_CONTEXT),
      ),
      KeyBinding::new(
        "cmd-v",
        terminal::SendKeystroke("cmd-v".to_string()),
        Some(terminal::TERMINAL_CONTEXT),
      ),
    ]);
  } else {
    bindings.extend([
      KeyBinding::new(
        "ctrl-shift-c",
        terminal::SendKeystroke("ctrl-shift-c".to_string()),
        Some(terminal::TERMINAL_CONTEXT),
      ),
      KeyBinding::new(
        "ctrl-insert",
        terminal::SendKeystroke("ctrl-shift-c".to_string()),
        Some(terminal::TERMINAL_CONTEXT),
      ),
      KeyBinding::new(
        "ctrl-shift-v",
        terminal::SendKeystroke("ctrl-shift-v".to_string()),
        Some(terminal::TERMINAL_CONTEXT),
      ),
      KeyBinding::new(
        "shift-insert",
        terminal::SendKeystroke("ctrl-shift-v".to_string()),
        Some(terminal::TERMINAL_CONTEXT),
      ),
    ]);
  }

  bindings
}

fn effective_shortcut_keystroke_text(
  id: ShortcutId,
  overrides: &ShortcutOverrides,
) -> Cow<'static, str> {
  overrides
    .keystroke_for(id)
    .map(|keystroke| Cow::Owned(keystroke.to_string()))
    .unwrap_or_else(|| Cow::Borrowed(shortcut_definition(id).keystroke))
}

fn has_shortcut_modifier(keystroke: &Keystroke) -> bool {
  keystroke.modifiers.platform
    || keystroke.modifiers.control
    || keystroke.modifiers.alt
    || is_function_key(&keystroke.key)
}

fn is_function_key(key: &str) -> bool {
  key
    .strip_prefix('f')
    .and_then(|suffix| suffix.parse::<u8>().ok())
    .is_some_and(|number| (1..=24).contains(&number))
}

fn serialize_keystroke(keystroke: &Keystroke) -> String {
  let mut parts = Vec::new();
  if keystroke.modifiers.control {
    parts.push("ctrl".to_string());
  }
  if keystroke.modifiers.alt {
    parts.push("alt".to_string());
  }
  if keystroke.modifiers.shift {
    parts.push("shift".to_string());
  }
  if keystroke.modifiers.platform {
    parts.push("cmd".to_string());
  }
  if keystroke.modifiers.function {
    parts.push("fn".to_string());
  }
  parts.push(keystroke.key.clone());
  parts.join("-")
}

fn active_contexts_overlap(a: &ShortcutDefinition, b: &ShortcutDefinition) -> bool {
  a.active_contexts
    .iter()
    .any(|context| b.active_contexts.contains(context))
}

fn with_shortcut_action<T>(id: ShortcutId, f: impl FnOnce(&dyn Action) -> T) -> T {
  match id {
    ShortcutId::ShowCommandPalette => f(&ShowCommandPalette),
    ShortcutId::NextCenterTab => f(&NextCenterTab),
    ShortcutId::PreviousCenterTab => f(&PreviousCenterTab),
    ShortcutId::CloseCenterTab => f(&CloseCenterTab),
    ShortcutId::NewFile => f(&NewFile),
    ShortcutId::SaveFileAs => f(&SaveFileAs),
    ShortcutId::ShowFileSearch => f(&ShowFileSearch),
    ShortcutId::ShowGlobalSearch => f(&ShowGlobalSearch),
    ShortcutId::OpenProject => f(&OpenProject),
    ShortcutId::CommitChanges => f(&CommitChanges),
    ShortcutId::PullChanges => f(&PullChanges),
    ShortcutId::PushChanges => f(&PushChanges),
    ShortcutId::ForcePushChanges => f(&ForcePushChanges),
    ShortcutId::OpenSettingsPage => f(&OpenSettingsPage),
    ShortcutId::ShowBranchSwitcher => f(&ShowBranchSwitcher),
    ShortcutId::OpenGitHistorySidebar => f(&OpenGitHistorySidebar),
    ShortcutId::OpenGitChangesSidebar => f(&OpenGitChangesSidebar),
    ShortcutId::OpenFilesSidebar => f(&OpenFilesSidebar),
    ShortcutId::OpenReviewSidebar => f(&OpenReviewSidebar),
    ShortcutId::OpenPullRequestSidebar => f(&OpenPullRequestSidebar),
    ShortcutId::ToggleDiffView => f(&ToggleDiffView),
    ShortcutId::ToggleHideWhitespace => f(&ToggleHideWhitespace),
    ShortcutId::PreviousAnnotation => f(&PreviousAnnotation),
    ShortcutId::NextAnnotation => f(&NextAnnotation),
    ShortcutId::CommentHunk => f(&CommentHunk),
    ShortcutId::SendReviewCommentsToAgent => f(&SendReviewCommentsToAgent),
    ShortcutId::AddSelectionToAgent => f(&AddSelectionToAgent),
    ShortcutId::JumpToLatestMessage => f(&JumpToLatestMessage),
    ShortcutId::NewAgentSession => f(&NewAgentSession),
    ShortcutId::NewAgentWorktreeSession => f(&NewAgentWorktreeSession),
    ShortcutId::ToggleHunkStage => f(&ToggleHunkStage),
    ShortcutId::RestoreHunk => f(&RestoreHunk),
    ShortcutId::ToggleFileStage => f(&ToggleFileStage),
    ShortcutId::RestoreFile => f(&RestoreFile),
    ShortcutId::AcceptBothConflict => f(&AcceptBothConflict),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_palette_command_points_at_the_shortcut_that_runs_it() {
    use CommandPaletteCommandId as Command;

    assert_eq!(
      palette_command_shortcut(Command::Commit),
      Some(ShortcutId::CommitChanges)
    );
    assert_eq!(
      palette_command_shortcut(Command::SwitchBranch),
      Some(ShortcutId::ShowBranchSwitcher)
    );
    assert_eq!(
      palette_command_shortcut(Command::SendReview),
      Some(ShortcutId::SendReviewCommentsToAgent)
    );
    // One key toggles either way.
    assert_eq!(
      palette_command_shortcut(Command::StageSelectedFile),
      palette_command_shortcut(Command::UnstageSelectedFile)
    );

    assert_eq!(palette_command_shortcut(Command::NewTerminal), None);

    // The dock surfaces that only had a key now show it on their palette row.
    assert_eq!(
      palette_command_shortcut(Command::ShowHistory),
      Some(ShortcutId::OpenGitHistorySidebar)
    );
    assert_eq!(
      palette_command_shortcut(Command::ShowFileSearch),
      Some(ShortcutId::ShowFileSearch)
    );
    assert_eq!(
      palette_command_shortcut(Command::ShowGlobalSearch),
      Some(ShortcutId::ShowGlobalSearch)
    );
    assert_eq!(
      palette_command_shortcut(Command::SendSelectionToAgent),
      Some(ShortcutId::AddSelectionToAgent)
    );

    assert_eq!(palette_command_shortcut(Command::CherryPick), None);
    assert_eq!(palette_command_shortcut(Command::SignOut), None);
    assert_eq!(palette_command_shortcut(Command::SendFeedback), None);
  }

  fn context_stack(_surface: &str) -> Vec<KeyContext> {
    vec![KeyContext::parse(&workspace_key_context_with_generation(0)).unwrap()]
  }

  fn context_stack_with_extra(surface: &str, extra_contexts: &[&str]) -> Vec<KeyContext> {
    let mut contexts = context_stack(surface);
    contexts.extend(
      extra_contexts
        .iter()
        .map(|context| KeyContext::parse(context).expect("valid extra key context")),
    );
    contexts
  }

  fn has_binding_with_bindings(surface: &str, keystroke: &str, bindings: Vec<KeyBinding>) -> bool {
    has_binding_with_bindings_in_contexts(surface, &[], keystroke, bindings)
  }

  fn has_binding_with_bindings_in_contexts(
    surface: &str,
    extra_contexts: &[&str],
    keystroke: &str,
    bindings: Vec<KeyBinding>,
  ) -> bool {
    let mut keymap = Keymap::default();
    keymap.add_bindings(bindings);
    let input = [Keystroke::parse(keystroke).unwrap()];
    let (bindings, pending) =
      keymap.bindings_for_input(&input, &context_stack_with_extra(surface, extra_contexts));
    !bindings.is_empty() && !pending
  }

  fn has_binding(surface: &str, keystroke: &str) -> bool {
    has_binding_with_bindings(surface, keystroke, workspace_key_bindings())
  }

  fn app_and_workspace_key_bindings() -> Vec<KeyBinding> {
    let mut bindings = default_app_key_bindings();
    bindings.extend(workspace_key_bindings());
    bindings
  }

  fn first_binding_action_name(
    surface: &str,
    extra_contexts: &[&str],
    keystroke: &str,
    bindings: Vec<KeyBinding>,
  ) -> Option<&'static str> {
    let mut keymap = Keymap::default();
    keymap.add_bindings(bindings);
    let input = [Keystroke::parse(keystroke).unwrap()];
    let (bindings, pending) =
      keymap.bindings_for_input(&input, &context_stack_with_extra(surface, extra_contexts));
    assert!(!pending);
    bindings.first().map(|binding| binding.action().name())
  }

  fn overrides(entries: &[(ShortcutId, &str)]) -> ShortcutOverrides {
    ShortcutOverrides {
      entries: entries
        .iter()
        .map(|(id, keystroke)| (*id, (*keystroke).to_string()))
        .collect(),
    }
  }

  #[test]
  fn outdent_is_scoped_to_editor_text_not_nested_inputs_or_other_surfaces() {
    assert_eq!(
      first_binding_action_name(
        "Changes",
        &["Editor"],
        "shift-tab",
        default_app_key_bindings()
      ),
      Some("editor::Outdent")
    );
    for contexts in [
      &[][..],
      &["Input"][..],
      &["Editor", "Input"][..],
      &["Terminal"][..],
    ] {
      assert_ne!(
        first_binding_action_name("Changes", contexts, "shift-tab", default_app_key_bindings()),
        Some("editor::Outdent")
      );
    }
  }

  #[test]
  fn shortcut_definitions_have_unique_ids() {
    let ids = shortcut_definitions()
      .iter()
      .map(|definition| definition.id)
      .collect::<HashSet<_>>();

    assert_eq!(ids.len(), shortcut_definitions().len());
  }

  #[test]
  fn shortcut_display_metadata_is_complete() {
    for definition in shortcut_definitions() {
      assert!(!definition.scope_label.is_empty());
      assert!(KeyContext::parse(definition.display_context).is_ok());
      assert!(!definition.active_contexts.is_empty());
    }
  }

  #[test]
  fn every_shortcut_uses_the_workspace_context() {
    let reachable: HashSet<&str> = ALL_WORKSPACE_ACTIVE_CONTEXTS.into_iter().collect();

    for definition in shortcut_definitions() {
      assert!(reachable.contains(definition.display_context));
      for context in definition.active_contexts {
        assert!(reachable.contains(context));
      }
    }
  }

  #[test]
  fn shortcut_definition_lookup_returns_expected_definition() {
    let definition = shortcut_definition(ShortcutId::CommitChanges);
    assert_eq!(definition.title, "Commit Changes");
    assert_eq!(definition.scope_label, "Projects");
    assert_eq!(
      shortcut_keystroke(ShortcutId::CommitChanges),
      Keystroke::parse("cmd-enter").unwrap()
    );
  }

  #[test]
  fn workspace_key_context_appends_shortcut_generation() {
    assert_eq!(
      workspace_key_context_with_generation(0),
      format!("{WORKSPACE_SESSION_CONTEXT} {SHORTCUT_KEYMAP_GENERATION_CONTEXT_KEY}=0")
    );

    assert_eq!(
      workspace_key_context_with_generation(1),
      format!("{WORKSPACE_SESSION_CONTEXT} {SHORTCUT_KEYMAP_GENERATION_CONTEXT_KEY}=1")
    );
  }

  #[test]
  fn command_palette_binding_is_available_in_the_workspace() {
    assert!(has_binding("workspace", SHOW_COMMAND_PALETTE_SHORTCUT));
  }

  #[test]
  fn file_search_binding_is_available_in_the_workspace() {
    assert!(has_binding("workspace", "cmd-p"));
  }

  #[test]
  fn global_search_binding_is_available_in_the_workspace() {
    assert!(has_binding("workspace", "cmd-shift-f"));
  }

  #[test]
  fn session_creation_bindings_live_in_the_workspace() {
    assert!(has_binding("workspace", "cmd-t"));
    assert!(has_binding("workspace", "cmd-shift-t"));
  }

  #[test]
  fn git_shortcuts_are_scoped_to_the_repository_surfaces() {
    assert!(has_binding("workspace", "cmd-o"));
    assert!(has_binding_with_bindings_in_contexts(
      "workspace",
      &["List"],
      "cmd-enter",
      workspace_key_bindings(),
    ));
    assert!(has_binding_with_bindings_in_contexts(
      "workspace",
      &["Editor"],
      "cmd-alt-enter",
      workspace_key_bindings(),
    ));
    assert!(has_binding_with_bindings_in_contexts(
      "workspace",
      &["List"],
      "cmd-alt-enter",
      workspace_key_bindings(),
    ));
    assert!(has_binding("workspace", "cmd-u"));
    assert!(has_binding("workspace", "cmd-y"));
    assert!(has_binding("workspace", "cmd-shift-y"));
  }

  #[test]
  fn cmd_w_closes_the_active_center_tab() {
    assert_eq!(
      first_binding_action_name("workspace", &[], "cmd-w", workspace_key_bindings(),),
      Some("workspace::CloseCenterTab")
    );
  }

  #[test]
  fn cmd_n_creates_inline_only_from_the_files_tree() {
    assert_eq!(
      first_binding_action_name("workspace", &[], "cmd-n", app_and_workspace_key_bindings()),
      Some("workspace::NewFile")
    );
    assert_eq!(
      first_binding_action_name(
        "workspace",
        &["Tree"],
        "cmd-n",
        app_and_workspace_key_bindings(),
      ),
      Some("workspace::NewFileInFilesPanel")
    );
  }

  #[test]
  fn terminal_keystrokes_override_global_and_workspace_actions() {
    let mut keystrokes = vec![
      "enter",
      "escape",
      "tab",
      "backspace",
      "delete",
      "up",
      "down",
      "left",
      "right",
      "shift-enter",
      "home",
      "end",
      "pageup",
      "pagedown",
    ];
    if cfg!(target_os = "macos") {
      keystrokes.extend(["cmd-c", "cmd-v"]);
    } else {
      keystrokes.extend(["ctrl-shift-c", "ctrl-shift-v"]);
    }

    for keystroke in keystrokes {
      assert_eq!(
        first_binding_action_name(
          "workspace",
          &[terminal::TERMINAL_CONTEXT],
          keystroke,
          app_and_workspace_key_bindings(),
        ),
        Some(<terminal::SendKeystroke as Action>::name_for_type()),
        "{keystroke} should reach the terminal",
      );
    }
  }

  #[test]
  fn terminal_scrollback_shortcuts_override_terminal_input() {
    let cases = [
      (
        "shift-up",
        <terminal::ScrollLineUp as Action>::name_for_type(),
      ),
      (
        "shift-down",
        <terminal::ScrollLineDown as Action>::name_for_type(),
      ),
      (
        "shift-pageup",
        <terminal::ScrollPageUp as Action>::name_for_type(),
      ),
      (
        "shift-pagedown",
        <terminal::ScrollPageDown as Action>::name_for_type(),
      ),
      (
        "shift-home",
        <terminal::ScrollToTop as Action>::name_for_type(),
      ),
      (
        "shift-end",
        <terminal::ScrollToBottom as Action>::name_for_type(),
      ),
    ];

    for (keystroke, expected_action) in cases {
      assert_eq!(
        first_binding_action_name(
          "workspace",
          &[terminal::TERMINAL_CONTEXT],
          keystroke,
          app_and_workspace_key_bindings(),
        ),
        Some(expected_action),
      );
    }
  }

  #[test]
  fn terminal_search_shortcuts_override_terminal_input() {
    let find = if cfg!(target_os = "macos") {
      "cmd-f"
    } else {
      "ctrl-f"
    };
    assert_eq!(
      first_binding_action_name(
        "workspace",
        &[terminal::TERMINAL_CONTEXT],
        find,
        app_and_workspace_key_bindings(),
      ),
      Some(<terminal::OpenSearch as Action>::name_for_type()),
    );
    assert_eq!(
      first_binding_action_name(
        "workspace",
        &[terminal::TERMINAL_SEARCH_CONTEXT],
        "escape",
        app_and_workspace_key_bindings(),
      ),
      Some(<terminal::CloseSearch as Action>::name_for_type()),
    );
  }

  #[test]
  fn escape_can_close_center_panes_from_the_session() {
    assert_eq!(
      first_binding_action_name("workspace", &[], "escape", workspace_key_bindings(),),
      Some("workspace::CloseCenterPane")
    );
  }

  #[test]
  fn settings_shortcut_is_available_in_the_workspace() {
    assert!(has_binding("workspace", "cmd-,"));
  }

  #[test]
  fn core_tab_shortcuts_are_available_in_the_workspace() {
    assert!(has_binding("workspace", "cmd-shift-]"));
    assert!(has_binding("workspace", "cmd-shift-["));
    assert!(has_binding("workspace", "cmd-w"));
  }

  #[test]
  fn git_keyboard_first_shortcuts_reach_the_workspace() {
    for keystroke in [
      "cmd-u",
      "cmd-y",
      "cmd-shift-y",
      "cmd-shift-b",
      "cmd-shift-h",
    ] {
      assert!(
        has_binding("workspace", keystroke),
        "{keystroke} should be active in the workspace"
      );
    }
  }

  #[test]
  fn comment_hunk_shortcut_is_available_wherever_the_workspace_shows_a_diff() {
    for descendant in ["List", "Editor", "Tree"] {
      assert!(has_binding_with_bindings_in_contexts(
        "workspace",
        &[descendant],
        "cmd-alt-enter",
        workspace_key_bindings(),
      ));
    }
  }

  #[test]
  fn every_dock_surface_has_a_key_of_its_own() {
    // Five surfaces in the right dock, five shortcuts, none of them shared.
    let dock = [
      (ShortcutId::OpenGitChangesSidebar, "cmd-shift-c"),
      (ShortcutId::OpenReviewSidebar, "cmd-shift-r"),
      (ShortcutId::OpenFilesSidebar, "cmd-shift-e"),
      (ShortcutId::OpenGitHistorySidebar, "cmd-shift-h"),
      (ShortcutId::OpenPullRequestSidebar, "cmd-shift-p"),
    ];

    for (id, keystroke) in dock {
      assert_eq!(
        shortcut_keystroke(id),
        Keystroke::parse(keystroke).expect("dock keystroke")
      );
      assert!(
        has_binding("workspace", keystroke),
        "{keystroke} should be active in the workspace"
      );
    }

    let keystrokes: HashSet<&str> = dock.iter().map(|(_, keystroke)| *keystroke).collect();
    assert_eq!(keystrokes.len(), dock.len(), "two surfaces share a key");
  }

  #[test]
  fn review_shortcuts_reach_the_workspace() {
    for keystroke in ["cmd-/", "cmd-alt-/"] {
      assert!(has_binding("workspace", keystroke));
    }
  }

  #[test]
  fn annotation_shortcuts_reach_the_workspace() {
    for keystroke in ["cmd-alt-up", "cmd-alt-down"] {
      assert!(has_binding("workspace", keystroke));
    }
  }

  #[test]
  fn local_git_shortcuts_reach_the_workspace() {
    let bound_in = |surface: &str, keystroke: &str| {
      has_binding_with_bindings_in_contexts(
        surface,
        &["Editor"],
        keystroke,
        workspace_key_bindings(),
      )
    };
    for keystroke in [
      "shift-enter",
      "shift-backspace",
      "cmd-shift-enter",
      "cmd-shift-l",
    ] {
      assert!(
        bound_in("workspace", keystroke),
        "{keystroke} in the workspace"
      );
    }

    for keystroke in [
      "cmd-o",
      "cmd-u",
      "cmd-y",
      "cmd-shift-y",
      "cmd-shift-b",
      "cmd-shift-h",
      "cmd-shift-c",
      "cmd-shift-e",
    ] {
      assert!(
        has_binding("workspace", keystroke),
        "{keystroke} reaches the workspace"
      );
    }

    for keystroke in ["shift-enter", "shift-backspace", "cmd-shift-enter"] {
      assert!(has_binding_with_bindings_in_contexts(
        "workspace",
        &["List"],
        keystroke,
        workspace_key_bindings(),
      ));
    }

    for keystroke in ["cmd-enter", "cmd-shift-backspace"] {
      assert!(!bound_in("workspace", keystroke));
      assert!(has_binding_with_bindings_in_contexts(
        "workspace",
        &["List"],
        keystroke,
        workspace_key_bindings(),
      ));
    }
  }

  #[test]
  fn file_level_git_shortcuts_stay_out_of_the_editor() {
    assert_eq!(
      first_binding_action_name(
        "workspace",
        &["Editor"],
        "cmd-backspace",
        app_and_workspace_key_bindings(),
      ),
      Some(<BackspaceAll as Action>::name_for_type())
    );
    assert_eq!(
      first_binding_action_name(
        "workspace",
        &["Editor"],
        "cmd-enter",
        app_and_workspace_key_bindings(),
      ),
      None
    );
    assert_eq!(
      first_binding_action_name(
        "workspace",
        &["Editor"],
        "cmd-shift-backspace",
        app_and_workspace_key_bindings(),
      ),
      None
    );
    assert_eq!(
      first_binding_action_name(
        "workspace",
        &["List"],
        "cmd-enter",
        app_and_workspace_key_bindings(),
      ),
      Some(<ToggleFileStage as Action>::name_for_type())
    );
    assert_eq!(
      first_binding_action_name(
        "workspace",
        &["List"],
        "cmd-shift-backspace",
        app_and_workspace_key_bindings(),
      ),
      Some(<RestoreFile as Action>::name_for_type())
    );
  }

  #[test]
  fn default_workspace_shortcuts_do_not_reuse_reserved_app_bindings() {
    for definition in shortcut_definitions() {
      assert!(
        RESERVED_APP_BINDINGS
          .iter()
          .all(|binding| binding.keystroke != definition.keystroke),
        "{} reuses the reserved {} shortcut",
        definition.title,
        definition.keystroke
      );
    }
  }

  #[test]
  fn workspace_key_bindings_apply_overrides() {
    let overrides = overrides(&[(ShortcutId::ShowFileSearch, "cmd-shift-u")]);
    let bindings = workspace_key_bindings_with_overrides(&overrides);

    assert!(has_binding_with_bindings(
      "workspace",
      "cmd-shift-u",
      bindings
    ));
  }

  #[test]
  fn newer_workspace_shortcut_generations_shadow_previous_bindings() {
    let mut keymap = Keymap::default();
    keymap.add_bindings(workspace_key_bindings_with_overrides_and_generation(
      &ShortcutOverrides::default(),
      1,
    ));
    keymap.add_bindings(workspace_key_bindings_with_overrides_and_generation(
      &overrides(&[(ShortcutId::ShowFileSearch, "cmd-shift-u")]),
      2,
    ));

    let current_context = [KeyContext::parse(&workspace_key_context_with_generation(2)).unwrap()];
    let old_input = [Keystroke::parse("cmd-p").unwrap()];
    let new_input = [Keystroke::parse("cmd-shift-u").unwrap()];

    let (old_bindings, old_pending) = keymap.bindings_for_input(&old_input, &current_context);
    assert!(old_bindings.is_empty());
    assert!(!old_pending);

    let (new_bindings, new_pending) = keymap.bindings_for_input(&new_input, &current_context);
    assert_eq!(new_bindings.len(), 1);
    assert!(!new_pending);
  }

  #[test]
  fn command_palette_context_disables_workspace_shortcuts() {
    let bindings = workspace_key_bindings();

    assert!(!has_binding_with_bindings_in_contexts(
      "workspace",
      &[COMMAND_PALETTE_CONTEXT],
      SHOW_COMMAND_PALETTE_SHORTCUT,
      bindings.clone(),
    ));
    assert!(!has_binding_with_bindings_in_contexts(
      "workspace",
      &[COMMAND_PALETTE_CONTEXT],
      "cmd-p",
      bindings.clone(),
    ));
    assert!(!has_binding_with_bindings_in_contexts(
      "workspace",
      &[COMMAND_PALETTE_CONTEXT],
      "escape",
      bindings,
    ));
  }

  #[test]
  fn shortcut_recording_context_disables_workspace_shortcuts() {
    let bindings = workspace_key_bindings();

    assert!(!has_binding_with_bindings_in_contexts(
      "workspace",
      &[WORKSPACE_SHORTCUT_RECORDING_CONTEXT],
      SHOW_COMMAND_PALETTE_SHORTCUT,
      bindings.clone(),
    ));
    assert!(!has_binding_with_bindings_in_contexts(
      "workspace",
      &[WORKSPACE_SHORTCUT_RECORDING_CONTEXT],
      "escape",
      bindings,
    ));
  }

  #[test]
  fn validate_shortcut_override_requires_modifier_or_function_key() {
    let overrides = ShortcutOverrides::default();
    let error = validate_shortcut_override(
      ShortcutId::ShowCommandPalette,
      &Keystroke::parse("k").unwrap(),
      &overrides,
    )
    .expect_err("plain letters should be rejected");

    assert_eq!(error, ShortcutOverrideError::MissingModifier);
  }

  #[test]
  fn validate_shortcut_override_rejects_reserved_bindings() {
    let overrides = ShortcutOverrides::default();
    let error = validate_shortcut_override(
      ShortcutId::ShowCommandPalette,
      &Keystroke::parse("cmd-c").unwrap(),
      &overrides,
    )
    .expect_err("reserved app shortcuts should be rejected");

    assert_eq!(
      error,
      ShortcutOverrideError::ReservedBinding { title: "Copy" }
    );
  }

  #[test]
  fn validate_shortcut_override_rejects_overlapping_product_conflicts() {
    let overrides = ShortcutOverrides::default();
    let error = validate_shortcut_override(
      ShortcutId::ShowFileSearch,
      &Keystroke::parse("cmd-enter").unwrap(),
      &overrides,
    )
    .expect_err("an overlap with a staging shortcut should be rejected");

    assert_eq!(
      error,
      ShortcutOverrideError::ShortcutConflict {
        shortcut_id: ShortcutId::ToggleFileStage,
      }
    );
  }

  #[test]
  fn validate_shortcut_override_allows_non_overlapping_shortcuts() {
    let overrides = ShortcutOverrides::default();
    let result = validate_shortcut_override(
      ShortcutId::OpenProject,
      &Keystroke::parse("cmd-shift-g").unwrap(),
      &overrides,
    );

    assert!(result.is_ok());
  }
}
