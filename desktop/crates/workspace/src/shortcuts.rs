use std::{borrow::Cow, collections::HashMap};

use editor::{
  AltLeft, AltRight, Backspace, BackspaceAll, BackspaceWord, CloseFind, CmdDown, CmdLeft, CmdRight,
  CmdUp, Copy, Cut, Delete, Down, End, Enter, Find, Home, Left, Paste, Quit, Redo, Right, Save,
  SelectAll, SelectCmdDown, SelectCmdLeft, SelectCmdRight, SelectCmdUp, SelectDown, SelectLeft,
  SelectRight, SelectUp, SelectWordLeft, SelectWordRight, ShowCharacterPalette, Tab, Undo, Up,
};
use gpui::{Action, App, Global, KeyBinding, KeyContext, Keystroke, Window};
use ui::{COMMAND_PALETTE_CONTEXT, CommandPaletteCommand, CommandPaletteCommandId};

#[cfg(test)]
use gpui::Keymap;
#[cfg(test)]
use std::collections::HashSet;

use crate::config::ConfigStore;
use crate::{
  AcceptBothConflict, AddSelectionToAgent, CommentHunk, CommitChanges, ForcePushChanges,
  JumpToLatestMessage, NavigateBack, NewAgentSession, NewAgentWorktreeSession, NextAnnotation,
  OpenFilesSidebar, OpenGitChangesSidebar, OpenGitHistorySidebar, OpenPullRequestSidebar,
  OpenRepository, OpenReviewSidebar, OpenSessionPage, OpenSettingsPage, PreviousAnnotation,
  PullChanges, PushChanges, RestoreFile, RestoreHunk, ReturnFocusToEditor,
  SendReviewCommentsToAgent, ShowBranchSwitcher, ShowCommandPalette, ShowFileSearch,
  ToggleDiffView, ToggleFileStage, ToggleHideWhitespace, ToggleHunkStage, ToggleTerminalSidebar,
};

pub const SHOW_COMMAND_PALETTE_SHORTCUT: &str = "cmd-k";
const SHORTCUT_KEYMAP_GENERATION_CONTEXT_KEY: &str = "workspace_shortcuts_generation";
pub const WORKSPACE_SHORTCUT_RECORDING_CONTEXT: &str = "WorkspaceShortcutRecording";

pub const WORKSPACE_CONTEXT: &str = "Workspace";
/// The right dock, so escape can mean "give the keyboard back" only in there.
pub const DOCK_PANEL_CONTEXT: &str = "DockPanel";
pub const WORKSPACE_SESSION_CONTEXT: &str = "Workspace WorkspaceSession";

const FILE_SEARCH_CONTEXT: &str = "WorkspaceSession";
const OPEN_REPOSITORY_CONTEXT: &str = "WorkspaceSession";
const COMMIT_CHANGES_CONTEXT: &str = "WorkspaceSession";
const COMMIT_CHANGES_DESCENDANT_FOCUS: &str = "CommitInput";
const PULL_CHANGES_CONTEXT: &str = "WorkspaceSession";
const PUSH_CHANGES_CONTEXT: &str = "WorkspaceSession";
const FORCE_PUSH_CHANGES_CONTEXT: &str = "WorkspaceSession";
const OPEN_SETTINGS_CONTEXT: &str = "Workspace";
const NAVIGATE_BACK_CONTEXT: &str = "Workspace";
const OPEN_SESSION_PAGE_CONTEXT: &str = "Workspace";
const TOGGLE_TERMINAL_CONTEXT: &str = "WorkspaceSession";
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

const SESSION_ONLY_ACTIVE_CONTEXTS: [&str; 1] = [WORKSPACE_SESSION_CONTEXT];

const COMMENT_HUNK_ACTIVE_CONTEXTS: [&str; 1] = [WORKSPACE_SESSION_CONTEXT];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutId {
  ShowCommandPalette,
  NavigateBack,
  OpenSessionPage,
  ShowFileSearch,
  OpenRepository,
  CommitChanges,
  PullChanges,
  PushChanges,
  ForcePushChanges,
  OpenSettingsPage,
  ToggleTerminalSidebar,
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
      ShortcutId::NavigateBack => "navigate_back",
      ShortcutId::OpenSessionPage => "open_session_page",
      ShortcutId::ShowFileSearch => "show_file_search",
      ShortcutId::OpenRepository => "open_repository",
      ShortcutId::CommitChanges => "commit_changes",
      ShortcutId::PullChanges => "pull_changes",
      ShortcutId::PushChanges => "push_changes",
      ShortcutId::ForcePushChanges => "force_push_changes",
      ShortcutId::OpenSettingsPage => "open_settings_page",
      ShortcutId::ToggleTerminalSidebar => "toggle_terminal_sidebar",
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
      "navigate_back" => Some(ShortcutId::NavigateBack),
      "open_session_page" => Some(ShortcutId::OpenSessionPage),
      "show_file_search" => Some(ShortcutId::ShowFileSearch),
      "open_repository" => Some(ShortcutId::OpenRepository),
      "commit_changes" => Some(ShortcutId::CommitChanges),
      "pull_changes" => Some(ShortcutId::PullChanges),
      "push_changes" => Some(ShortcutId::PushChanges),
      "force_push_changes" => Some(ShortcutId::ForcePushChanges),
      "open_settings_page" => Some(ShortcutId::OpenSettingsPage),
      "toggle_terminal_sidebar" => Some(ShortcutId::ToggleTerminalSidebar),
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

const SHORTCUT_DEFINITIONS: [ShortcutDefinition; 32] = [
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
    id: ShortcutId::NavigateBack,
    title: "Back",
    description: "Go back in navigation history.",
    scope_label: "Workspace",
    category: ShortcutCategory::Core,
    keystroke: "cmd-[",
    context: NAVIGATE_BACK_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &ALL_WORKSPACE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenSessionPage,
    title: "Go to Sessions",
    description: "Focus the sessions workspace.",
    scope_label: "Workspace",
    category: ShortcutCategory::Core,
    keystroke: "cmd-1",
    context: OPEN_SESSION_PAGE_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &ALL_WORKSPACE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ShowFileSearch,
    title: "File Search",
    description: "Open file search where file navigation is available.",
    scope_label: "PR Changes and Sessions",
    category: ShortcutCategory::Core,
    keystroke: "cmd-p",
    context: FILE_SEARCH_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &FILE_SEARCH_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::PreviousAnnotation,
    title: "Previous Change",
    description: "Jump to the previous conflict or change in the diff.",
    scope_label: "Conflicts and changes, PR Changes, Sessions",
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
    scope_label: "Conflicts and changes, PR Changes, Sessions",
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
    scope_label: "PR Changes and Sessions",
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
    scope_label: "Sessions",
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
    scope_label: "Sessions",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-l",
    context: HUNK_ACTION_SESSION_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::NewAgentSession,
    title: "New Session",
    description: "Start a new agent session in the current repository.",
    scope_label: "Sessions",
    category: ShortcutCategory::Core,
    keystroke: "cmd-t",
    context: "WorkspaceSession",
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::NewAgentWorktreeSession,
    title: "New Worktree Session",
    description: "Start a new agent session in its own git worktree of the current repository.",
    scope_label: "Sessions",
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
    scope_label: "Sessions",
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
    scope_label: "Sessions",
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
    scope_label: "Sessions",
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
    scope_label: "Sessions",
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
    scope_label: "Sessions",
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
    scope_label: "Sessions",
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
    scope_label: "PR Changes and Sessions",
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
    scope_label: "PR Changes and Sessions",
    category: ShortcutCategory::Review,
    keystroke: "cmd-alt-/",
    context: TOGGLE_DIFF_VIEW_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &COMMENT_HUNK_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenRepository,
    title: "Open Repository",
    description: "Open a local repository.",
    scope_label: "Sessions",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-o",
    context: OPEN_REPOSITORY_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::CommitChanges,
    title: "Commit Changes",
    description: "Commit the staged changes.",
    scope_label: "Sessions",
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
    scope_label: "Sessions",
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
    scope_label: "Sessions",
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
    scope_label: "Sessions",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-y",
    context: FORCE_PUSH_CHANGES_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ToggleTerminalSidebar,
    title: "Toggle Terminal",
    description: "Show or hide the terminal.",
    scope_label: "Sessions",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-j",
    context: TOGGLE_TERMINAL_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ShowBranchSwitcher,
    title: "Switch Branch",
    description: "Switch branch.",
    scope_label: "Sessions",
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
    scope_label: "Sessions",
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
    scope_label: "Sessions",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-e",
    context: OPEN_GIT_CHANGES_SIDEBAR_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenFilesSidebar,
    title: "Show Files",
    description: "Switch the right panel to the repository file tree.",
    scope_label: "Sessions",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-f",
    context: OPEN_FILES_SIDEBAR_CONTEXT,
    display_context: WORKSPACE_SESSION_CONTEXT,
    active_contexts: &SESSION_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenReviewSidebar,
    title: "Show Review",
    description: "Switch the right panel to the review waiting to be sent.",
    scope_label: "Sessions",
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
    scope_label: "Sessions",
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

  fn set(&mut self, id: ShortcutId, keystroke: String) {
    self.entries.insert(id, keystroke);
  }

  fn remove(&mut self, id: ShortcutId) {
    self.entries.remove(&id);
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
      ShortcutId::NavigateBack => KeyBinding::new(keystroke, NavigateBack, Some(&context)),
      ShortcutId::OpenSessionPage => KeyBinding::new(keystroke, OpenSessionPage, Some(&context)),
      ShortcutId::ShowFileSearch => KeyBinding::new(keystroke, ShowFileSearch, Some(&context)),
      ShortcutId::OpenRepository => KeyBinding::new(keystroke, OpenRepository, Some(&context)),
      ShortcutId::CommitChanges => KeyBinding::new(keystroke, CommitChanges, Some(&context)),
      ShortcutId::PullChanges => KeyBinding::new(keystroke, PullChanges, Some(&context)),
      ShortcutId::PushChanges => KeyBinding::new(keystroke, PushChanges, Some(&context)),
      ShortcutId::ForcePushChanges => KeyBinding::new(keystroke, ForcePushChanges, Some(&context)),
      ShortcutId::OpenSettingsPage => KeyBinding::new(keystroke, OpenSettingsPage, Some(&context)),
      ShortcutId::ToggleTerminalSidebar => {
        KeyBinding::new(keystroke, ToggleTerminalSidebar, Some(&context))
      }
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
  let mut overrides = ShortcutOverrides::get(cx);

  if keystroke == shortcut_definition(id).keystroke {
    overrides.remove(id);
    ConfigStore::clear_shortcut_override(id);
  } else {
    overrides.set(id, keystroke.clone());
    ConfigStore::persist_shortcut_override(id, &keystroke);
  }

  cx.set_global(overrides);
}

pub fn clear_shortcut_override(cx: &mut App, id: ShortcutId) {
  let mut overrides = ShortcutOverrides::get(cx);
  overrides.remove(id);
  cx.set_global(overrides);
  ConfigStore::clear_shortcut_override(id);
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
    Command::OpenRepository => Some(ShortcutId::OpenRepository),
    Command::OpenSessionPage => Some(ShortcutId::OpenSessionPage),
    Command::OpenSettingsPage => Some(ShortcutId::OpenSettingsPage),
    Command::SendReview => Some(ShortcutId::SendReviewCommentsToAgent),
    // One key toggles either way, so both rows show it.
    Command::StageSelectedFile | Command::UnstageSelectedFile => Some(ShortcutId::ToggleFileStage),
    Command::ToggleTerminal => Some(ShortcutId::ToggleTerminalSidebar),
    Command::ShowChanges => Some(ShortcutId::OpenGitChangesSidebar),
    Command::ShowReview => Some(ShortcutId::OpenReviewSidebar),
    Command::ShowFiles => Some(ShortcutId::OpenFilesSidebar),
    Command::ShowHistory => Some(ShortcutId::OpenGitHistorySidebar),
    Command::ShowPullRequest => Some(ShortcutId::OpenPullRequestSidebar),
    Command::ShowFileSearch => Some(ShortcutId::ShowFileSearch),
    Command::ToggleDiffView => Some(ShortcutId::ToggleDiffView),
    Command::ToggleHideWhitespace => Some(ShortcutId::ToggleHideWhitespace),
    Command::SendSelectionToAgent => Some(ShortcutId::AddSelectionToAgent),
    Command::JumpToLatestMessage => Some(ShortcutId::JumpToLatestMessage),
    Command::NewAgentSession => Some(ShortcutId::NewAgentSession),
    Command::NewAgentWorktreeSession => Some(ShortcutId::NewAgentWorktreeSession),
    Command::SwitchRepository
    | Command::ForgetRepository
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
  Vec::new()
}

pub fn key_context_for_pathname(_pathname: &str) -> &'static str {
  WORKSPACE_SESSION_CONTEXT
}

pub fn current_key_context_for_pathname(pathname: &str, cx: &App) -> String {
  key_context_for_pathname_with_generation(pathname, ShortcutBindingState::current_generation(cx))
}

fn key_context_for_pathname_with_generation(pathname: &str, generation: u32) -> String {
  key_context_with_shortcut_generation(key_context_for_pathname(pathname), generation)
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
  vec![
    KeyBinding::new("enter", Enter, None),
    KeyBinding::new("tab", Tab, None),
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
    KeyBinding::new("escape", CloseFind, Some("Editor")),
    KeyBinding::new("escape", ReturnFocusToEditor, Some(DOCK_PANEL_CONTEXT)),
    // Deeper than the window's own Tab, so the shell gets the key instead of
    // losing the focus to the next widget.
    KeyBinding::new("tab", terminal::SendTab, Some(terminal::TERMINAL_CONTEXT)),
    KeyBinding::new(
      "shift-tab",
      terminal::SendBackTab,
      Some(terminal::TERMINAL_CONTEXT),
    ),
    KeyBinding::new("home", Home, None),
    KeyBinding::new("end", End, None),
    KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, None),
    KeyBinding::new("cmd-q", Quit, None),
  ]
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
    ShortcutId::NavigateBack => f(&NavigateBack),
    ShortcutId::OpenSessionPage => f(&OpenSessionPage),
    ShortcutId::ShowFileSearch => f(&ShowFileSearch),
    ShortcutId::OpenRepository => f(&OpenRepository),
    ShortcutId::CommitChanges => f(&CommitChanges),
    ShortcutId::PullChanges => f(&PullChanges),
    ShortcutId::PushChanges => f(&PushChanges),
    ShortcutId::ForcePushChanges => f(&ForcePushChanges),
    ShortcutId::OpenSettingsPage => f(&OpenSettingsPage),
    ShortcutId::ToggleTerminalSidebar => f(&ToggleTerminalSidebar),
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

    // The surfaces that only had a key now show it on their palette row.
    assert_eq!(
      palette_command_shortcut(Command::ToggleTerminal),
      Some(ShortcutId::ToggleTerminalSidebar)
    );
    assert_eq!(
      palette_command_shortcut(Command::ShowHistory),
      Some(ShortcutId::OpenGitHistorySidebar)
    );
    assert_eq!(
      palette_command_shortcut(Command::ShowFileSearch),
      Some(ShortcutId::ShowFileSearch)
    );
    assert_eq!(
      palette_command_shortcut(Command::SendSelectionToAgent),
      Some(ShortcutId::AddSelectionToAgent)
    );

    assert_eq!(palette_command_shortcut(Command::CherryPick), None);
    assert_eq!(palette_command_shortcut(Command::SignOut), None);
    assert_eq!(palette_command_shortcut(Command::SendFeedback), None);
  }

  fn context_stack(pathname: &str) -> Vec<KeyContext> {
    vec![KeyContext::parse(&key_context_for_pathname_with_generation(pathname, 0)).unwrap()]
  }

  fn context_stack_with_extra(pathname: &str, extra_contexts: &[&str]) -> Vec<KeyContext> {
    let mut contexts = context_stack(pathname);
    contexts.extend(
      extra_contexts
        .iter()
        .map(|context| KeyContext::parse(context).expect("valid extra key context")),
    );
    contexts
  }

  fn has_binding_with_bindings(pathname: &str, keystroke: &str, bindings: Vec<KeyBinding>) -> bool {
    has_binding_with_bindings_in_contexts(pathname, &[], keystroke, bindings)
  }

  fn has_binding_with_bindings_in_contexts(
    pathname: &str,
    extra_contexts: &[&str],
    keystroke: &str,
    bindings: Vec<KeyBinding>,
  ) -> bool {
    let mut keymap = Keymap::default();
    keymap.add_bindings(bindings);
    let input = [Keystroke::parse(keystroke).unwrap()];
    let (bindings, pending) =
      keymap.bindings_for_input(&input, &context_stack_with_extra(pathname, extra_contexts));
    !bindings.is_empty() && !pending
  }

  fn has_binding(pathname: &str, keystroke: &str) -> bool {
    has_binding_with_bindings(pathname, keystroke, workspace_key_bindings())
  }

  fn app_and_workspace_key_bindings() -> Vec<KeyBinding> {
    let mut bindings = default_app_key_bindings();
    bindings.extend(workspace_key_bindings());
    bindings
  }

  fn first_binding_action_name(
    pathname: &str,
    extra_contexts: &[&str],
    keystroke: &str,
    bindings: Vec<KeyBinding>,
  ) -> Option<&'static str> {
    let mut keymap = Keymap::default();
    keymap.add_bindings(bindings);
    let input = [Keystroke::parse(keystroke).unwrap()];
    let (bindings, pending) =
      keymap.bindings_for_input(&input, &context_stack_with_extra(pathname, extra_contexts));
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
  fn every_shortcut_is_scoped_to_a_page_the_workspace_still_has() {
    // A page that goes away takes its shortcuts with it: Settings lists them
    // all, so one left behind is a key the user presses for nothing.
    let reachable: HashSet<&str> = ALL_WORKSPACE_ACTIVE_CONTEXTS.into_iter().collect();

    for definition in shortcut_definitions() {
      assert!(
        reachable.contains(definition.display_context),
        "{} is displayed for a page the workspace cannot route to",
        definition.title
      );
      for context in definition.active_contexts {
        assert!(
          reachable.contains(context),
          "{} is active on a page the workspace cannot route to",
          definition.title
        );
      }
    }
  }

  #[test]
  fn the_reachable_contexts_are_the_ones_routing_can_produce() {
    let routed: HashSet<&str> = [
      "/session",
      "/git-config",
      "/settings",
      "/github/owner/repo/pull/42",
    ]
    .into_iter()
    .map(key_context_for_pathname)
    .collect();

    assert_eq!(
      routed,
      ALL_WORKSPACE_ACTIVE_CONTEXTS
        .into_iter()
        .collect::<HashSet<_>>()
    );
  }

  #[test]
  fn shortcut_definition_lookup_returns_expected_definition() {
    let definition = shortcut_definition(ShortcutId::CommitChanges);
    assert_eq!(definition.title, "Commit Changes");
    assert_eq!(definition.scope_label, "Sessions");
    assert_eq!(
      shortcut_keystroke(ShortcutId::CommitChanges),
      Keystroke::parse("cmd-enter").unwrap()
    );
  }

  #[test]
  fn key_context_for_pathname_matches_workspace_routes() {
    assert_eq!(
      key_context_for_pathname("/session"),
      WORKSPACE_SESSION_CONTEXT
    );
    assert_eq!(
      key_context_for_pathname("/github/owner/repo"),
      WORKSPACE_SESSION_CONTEXT
    );
    assert_eq!(
      key_context_for_pathname("/github/owner/repo/pull/42"),
      WORKSPACE_SESSION_CONTEXT
    );
    assert_eq!(
      key_context_for_pathname("/settings"),
      WORKSPACE_SESSION_CONTEXT
    );
  }

  #[test]
  fn current_key_context_for_pathname_appends_shortcut_generation() {
    assert_eq!(
      key_context_for_pathname_with_generation("/session", 0),
      format!("{WORKSPACE_SESSION_CONTEXT} {SHORTCUT_KEYMAP_GENERATION_CONTEXT_KEY}=0")
    );

    assert_eq!(
      key_context_for_pathname_with_generation("/session", 1),
      format!("{WORKSPACE_SESSION_CONTEXT} {SHORTCUT_KEYMAP_GENERATION_CONTEXT_KEY}=1")
    );
  }

  #[test]
  fn command_palette_binding_is_available_in_all_workspace_contexts() {
    assert!(has_binding("/session", SHOW_COMMAND_PALETTE_SHORTCUT));
    assert!(has_binding("/github", SHOW_COMMAND_PALETTE_SHORTCUT));
    assert!(has_binding("/settings", SHOW_COMMAND_PALETTE_SHORTCUT));
  }

  #[test]
  fn file_search_binding_is_available_on_old_links_that_land_in_the_shell() {
    assert!(has_binding("/session", "cmd-p"));
    assert!(has_binding("/settings", "cmd-p"));
  }

  #[test]
  fn session_creation_bindings_live_on_the_shell() {
    assert!(has_binding("/session", "cmd-t"));
    assert!(has_binding("/session", "cmd-shift-t"));
    assert!(has_binding("/settings", "cmd-t"));
    assert!(has_binding("/settings", "cmd-shift-t"));
  }

  #[test]
  fn git_shortcuts_are_scoped_to_the_repository_surfaces() {
    assert!(has_binding("/session", "cmd-o"));
    assert!(has_binding_with_bindings_in_contexts(
      "/session",
      &["List"],
      "cmd-enter",
      workspace_key_bindings(),
    ));
    assert!(has_binding_with_bindings_in_contexts(
      "/session",
      &["Editor"],
      "cmd-alt-enter",
      workspace_key_bindings(),
    ));
    assert!(has_binding_with_bindings_in_contexts(
      "/session",
      &["List"],
      "cmd-alt-enter",
      workspace_key_bindings(),
    ));
    assert!(has_binding("/session", "cmd-u"));
    assert!(has_binding("/session", "cmd-y"));
    assert!(has_binding("/session", "cmd-shift-y"));
    assert!(has_binding("/settings", "cmd-o"));
    assert!(has_binding_with_bindings_in_contexts(
      "/settings",
      &["List"],
      "cmd-enter",
      workspace_key_bindings(),
    ));
    assert!(has_binding("/settings", "cmd-u"));
    assert!(has_binding("/settings", "cmd-y"));
    assert!(has_binding("/settings", "cmd-shift-y"));
  }

  #[test]
  fn escape_no_longer_closes_workspace_pages() {
    assert!(!has_binding("/settings", "escape"));
    assert!(!has_binding("/git-config", "escape"));
    assert!(!has_binding("/session", "escape"));
    assert!(!has_binding("/github", "escape"));
    assert!(!has_binding("/github/owner/repo", "escape"));
    assert!(!has_binding("/settings", "cmd-w"));
    assert!(!has_binding("/git-config", "cmd-w"));
  }

  #[test]
  fn workspace_navigation_shortcuts_are_available_across_workspace_pages() {
    assert!(has_binding("/session", "cmd-,"));
    assert!(has_binding("/github", "cmd-,"));
    assert!(has_binding("/settings", "cmd-,"));
  }

  #[test]
  fn the_shell_is_the_first_navigation_shortcut() {
    assert_eq!(
      shortcut_keystroke(ShortcutId::OpenSessionPage),
      Keystroke::parse("cmd-1").expect("cmd-1 keystroke")
    );
    assert!(
      !SHORTCUT_DEFINITIONS
        .iter()
        .any(|definition| definition.keystroke == "cmd-2"),
      "there is no second page to switch to"
    );
  }

  #[test]
  fn core_navigation_shortcuts_are_available_across_workspace_pages() {
    for pathname in ["/session", "/github", "/settings"] {
      assert!(has_binding(pathname, "cmd-["));
      assert!(has_binding(pathname, "cmd-1"));
    }
  }

  #[test]
  fn git_keyboard_first_shortcuts_follow_old_links_to_the_shell() {
    for keystroke in [
      "cmd-j",
      "cmd-u",
      "cmd-y",
      "cmd-shift-y",
      "cmd-shift-b",
      "cmd-shift-h",
    ] {
      assert!(
        has_binding("/session", keystroke),
        "{keystroke} should be active in the shell"
      );
      assert!(
        has_binding("/settings", keystroke),
        "{keystroke} should stay active when an old link lands in the shell"
      );
    }
  }

  #[test]
  fn comment_hunk_shortcut_is_available_wherever_the_shell_shows_a_diff() {
    for descendant in ["List", "Editor", "Tree"] {
      assert!(has_binding_with_bindings_in_contexts(
        "/session",
        &[descendant],
        "cmd-alt-enter",
        workspace_key_bindings(),
      ));
    }
    assert!(has_binding_with_bindings_in_contexts(
      "/settings",
      &["Editor"],
      "cmd-alt-enter",
      workspace_key_bindings(),
    ));
  }

  #[test]
  fn every_dock_surface_has_a_key_of_its_own() {
    // Six surfaces in the right dock, six shortcuts, none of them shared.
    let dock = [
      (ShortcutId::OpenGitChangesSidebar, "cmd-shift-e"),
      (ShortcutId::OpenReviewSidebar, "cmd-shift-r"),
      (ShortcutId::OpenFilesSidebar, "cmd-shift-f"),
      (ShortcutId::OpenGitHistorySidebar, "cmd-shift-h"),
      (ShortcutId::OpenPullRequestSidebar, "cmd-shift-p"),
      (ShortcutId::ToggleTerminalSidebar, "cmd-j"),
    ];

    for (id, keystroke) in dock {
      assert_eq!(
        shortcut_keystroke(id),
        Keystroke::parse(keystroke).expect("dock keystroke")
      );
      assert!(
        has_binding("/session", keystroke),
        "{keystroke} should be active in the shell"
      );
      assert!(
        has_binding("/settings", keystroke),
        "{keystroke} stays active when an old link lands in the shell"
      );
    }

    let keystrokes: HashSet<&str> = dock.iter().map(|(_, keystroke)| *keystroke).collect();
    assert_eq!(keystrokes.len(), dock.len(), "two surfaces share a key");
  }

  #[test]
  fn review_shortcuts_follow_old_links_to_the_shell() {
    for keystroke in ["cmd-/", "cmd-alt-/"] {
      assert!(has_binding("/session", keystroke));
      assert!(has_binding("/settings", keystroke));
    }
  }

  #[test]
  fn annotation_shortcuts_follow_old_links_to_the_shell() {
    for keystroke in ["cmd-alt-up", "cmd-alt-down"] {
      assert!(has_binding("/session", keystroke));
      assert!(has_binding("/settings", keystroke));
    }
  }

  #[test]
  fn local_git_shortcuts_reach_the_shell_as_they_move_there() {
    // Hunk actions and the selection hand-off now work on both surfaces.
    let bound_in = |pathname: &str, keystroke: &str| {
      has_binding_with_bindings_in_contexts(
        pathname,
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
      assert!(bound_in("/session", keystroke), "{keystroke} in the shell");
      assert!(
        bound_in("/settings", keystroke),
        "{keystroke} stays active when an old link lands in the shell"
      );
    }

    // The repository and sync shortcuts followed their commands into the shell.
    for keystroke in [
      "cmd-o",
      "cmd-u",
      "cmd-y",
      "cmd-shift-y",
      "cmd-j",
      "cmd-shift-b",
      "cmd-shift-h",
      "cmd-shift-e",
    ] {
      assert!(
        has_binding("/session", keystroke),
        "{keystroke} followed its command into the shell"
      );
      assert!(
        has_binding("/settings", keystroke),
        "{keystroke} stays active when an old link lands in the shell"
      );
    }

    for keystroke in ["shift-enter", "shift-backspace", "cmd-shift-enter"] {
      assert!(has_binding_with_bindings_in_contexts(
        "/session",
        &["List"],
        keystroke,
        workspace_key_bindings(),
      ));
    }

    for keystroke in ["cmd-enter", "cmd-shift-backspace"] {
      assert!(!bound_in("/session", keystroke));
      assert!(has_binding_with_bindings_in_contexts(
        "/session",
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
        "/session",
        &["Editor"],
        "cmd-backspace",
        app_and_workspace_key_bindings(),
      ),
      Some(<BackspaceAll as Action>::name_for_type())
    );
    assert_eq!(
      first_binding_action_name(
        "/session",
        &["Editor"],
        "cmd-enter",
        app_and_workspace_key_bindings(),
      ),
      None
    );
    assert_eq!(
      first_binding_action_name(
        "/session",
        &["Editor"],
        "cmd-shift-backspace",
        app_and_workspace_key_bindings(),
      ),
      None
    );
    assert_eq!(
      first_binding_action_name(
        "/session",
        &["List"],
        "cmd-enter",
        app_and_workspace_key_bindings(),
      ),
      Some(<ToggleFileStage as Action>::name_for_type())
    );
    assert_eq!(
      first_binding_action_name(
        "/session",
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
      "/session",
      "cmd-shift-u",
      bindings.clone()
    ));
    assert!(has_binding_with_bindings(
      "/settings",
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

    let current_context =
      [KeyContext::parse(&key_context_for_pathname_with_generation("/session", 2)).unwrap()];
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
      "/session",
      &[COMMAND_PALETTE_CONTEXT],
      SHOW_COMMAND_PALETTE_SHORTCUT,
      bindings.clone(),
    ));
    assert!(!has_binding_with_bindings_in_contexts(
      "/session",
      &[COMMAND_PALETTE_CONTEXT],
      "cmd-p",
      bindings.clone(),
    ));
    assert!(!has_binding_with_bindings_in_contexts(
      "/settings",
      &[COMMAND_PALETTE_CONTEXT],
      "escape",
      bindings,
    ));
  }

  #[test]
  fn shortcut_recording_context_disables_workspace_shortcuts() {
    let bindings = workspace_key_bindings();

    assert!(!has_binding_with_bindings_in_contexts(
      "/settings",
      &[WORKSPACE_SHORTCUT_RECORDING_CONTEXT],
      SHOW_COMMAND_PALETTE_SHORTCUT,
      bindings.clone(),
    ));
    assert!(!has_binding_with_bindings_in_contexts(
      "/settings",
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
      ShortcutId::OpenRepository,
      &Keystroke::parse("cmd-w").unwrap(),
      &overrides,
    );

    assert!(result.is_ok());
  }
}
