use std::{borrow::Cow, collections::HashMap};

use editor::{
  AltLeft, AltRight, Backspace, BackspaceAll, BackspaceWord, CloseFind, CmdDown, CmdLeft, CmdRight,
  CmdUp, Copy, Cut, Delete, Down, End, Enter, Find, Home, Left, Paste, Quit, Redo, Right, Save,
  SelectAll, SelectCmdDown, SelectCmdLeft, SelectCmdRight, SelectCmdUp, SelectDown, SelectLeft,
  SelectRight, SelectUp, SelectWordLeft, SelectWordRight, ShowCharacterPalette, Tab, Undo, Up,
};
use gpui::{Action, App, Global, KeyBinding, KeyContext, Keystroke, Window};
use ui::COMMAND_PALETTE_CONTEXT;

#[cfg(test)]
use gpui::Keymap;
#[cfg(test)]
use std::collections::HashSet;

use crate::config::ConfigStore;
use crate::{
  CloseWorkspacePage, CommitChanges, ForcePushChanges, NavigateBack, NextAnnotation, NextPageTab,
  OpenGitChangesSidebar, OpenGitHistorySidebar, OpenGitPage, OpenGithubPage, OpenRepository,
  OpenSettingsPage, PreviousAnnotation, PreviousPageTab, PullChanges, PushChanges,
  RefreshCurrentPage, ShowBranchSwitcher, ShowCommandPalette, ShowFileSearch, SwitchToPrBranch,
  ToggleDiffView, ToggleTerminalSidebar,
};

pub const SHOW_COMMAND_PALETTE_SHORTCUT: &str = "cmd-k";
const SHORTCUT_KEYMAP_GENERATION_CONTEXT_KEY: &str = "workspace_shortcuts_generation";
pub const WORKSPACE_SHORTCUT_RECORDING_CONTEXT: &str = "WorkspaceShortcutRecording";
pub const GIT_REPO_SELECT_CONTEXT: &str = "GitRepoSelect";
pub const GIT_BRANCH_SELECT_CONTEXT: &str = "GitBranchSelect";
pub const GIT_HISTORY_TREE_CONTEXT: &str = "GitHistoryTree";
pub const GITHUB_PR_CHANGES_TREE_CONTEXT: &str = "GithubPrChangesTree";

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const WORKSPACE_GIT_CONTEXT: &str = "Workspace WorkspaceGit";
pub const WORKSPACE_GITHUB_HOME_CONTEXT: &str = "Workspace WorkspaceGithubHome";
pub const WORKSPACE_GITHUB_REPO_CONTEXT: &str = "Workspace WorkspaceGithubRepo";
pub const WORKSPACE_GITHUB_REPO_CODE_CONTEXT: &str =
  "Workspace WorkspaceGithubRepo WorkspaceGithubRepoCode";
pub const WORKSPACE_GITHUB_PR_CONTEXT: &str = "Workspace WorkspaceGithubPr";
pub const WORKSPACE_GITHUB_PR_CHANGES_CONTEXT: &str =
  "Workspace WorkspaceGithubPr WorkspaceGithubPrChanges";
pub const WORKSPACE_BILLING_CONTEXT: &str = "Workspace WorkspaceBilling";
pub const WORKSPACE_GIT_CONFIG_CONTEXT: &str = "Workspace WorkspaceGitConfig";
pub const WORKSPACE_SETTINGS_CONTEXT: &str = "Workspace WorkspaceSettings";
pub const WORKSPACE_ABOUT_CONTEXT: &str = "Workspace WorkspaceAbout";

const FILE_SEARCH_CONTEXT: &str =
  "WorkspaceGit || WorkspaceGithubRepoCode || WorkspaceGithubPrChanges";
const OPEN_REPOSITORY_CONTEXT: &str = "WorkspaceGit";
const COMMIT_CHANGES_CONTEXT: &str = "WorkspaceGit";
const PULL_CHANGES_CONTEXT: &str = "WorkspaceGit";
const PUSH_CHANGES_CONTEXT: &str = "WorkspaceGit";
const FORCE_PUSH_CHANGES_CONTEXT: &str = "WorkspaceGit";
const CLOSE_WORKSPACE_PAGE_CONTEXT: &str =
  "WorkspaceBilling || WorkspaceGitConfig || WorkspaceSettings || WorkspaceAbout";
const OPEN_SETTINGS_CONTEXT: &str = "Workspace";
const NAVIGATE_BACK_CONTEXT: &str = "Workspace";
const OPEN_GIT_PAGE_CONTEXT: &str = "Workspace";
const OPEN_GITHUB_PAGE_CONTEXT: &str = "Workspace";
const REFRESH_CURRENT_PAGE_CONTEXT: &str = "WorkspaceGit || WorkspaceGithubHome || WorkspaceGithubRepo || WorkspaceGithubRepoCode || WorkspaceGithubPr || WorkspaceGithubPrChanges";
const TOGGLE_TERMINAL_CONTEXT: &str = "WorkspaceGit";
const SHOW_BRANCH_SWITCHER_CONTEXT: &str = "WorkspaceGit";
const OPEN_GIT_HISTORY_SIDEBAR_CONTEXT: &str = "WorkspaceGit";
const OPEN_GIT_CHANGES_SIDEBAR_CONTEXT: &str = "WorkspaceGit";
const TOGGLE_DIFF_VIEW_CONTEXT: &str = "WorkspaceGit || WorkspaceGithubPrChanges";
const SWITCH_TO_PR_BRANCH_CONTEXT: &str = "WorkspaceGithubPr || WorkspaceGithubPrChanges";
const REVIEW_ANNOTATION_CONTEXT: &str = "WorkspaceGit || WorkspaceGithubPrChanges";
const PAGE_TAB_CONTEXT: &str =
  "WorkspaceGithubRepo || WorkspaceGithubRepoCode || WorkspaceGithubPr || WorkspaceGithubPrChanges";

const ALL_WORKSPACE_ACTIVE_CONTEXTS: [&str; 10] = [
  WORKSPACE_GIT_CONTEXT,
  WORKSPACE_GITHUB_HOME_CONTEXT,
  WORKSPACE_GITHUB_REPO_CONTEXT,
  WORKSPACE_GITHUB_REPO_CODE_CONTEXT,
  WORKSPACE_GITHUB_PR_CONTEXT,
  WORKSPACE_GITHUB_PR_CHANGES_CONTEXT,
  WORKSPACE_BILLING_CONTEXT,
  WORKSPACE_GIT_CONFIG_CONTEXT,
  WORKSPACE_SETTINGS_CONTEXT,
  WORKSPACE_ABOUT_CONTEXT,
];

const FILE_SEARCH_ACTIVE_CONTEXTS: [&str; 3] = [
  WORKSPACE_GIT_CONTEXT,
  WORKSPACE_GITHUB_REPO_CODE_CONTEXT,
  WORKSPACE_GITHUB_PR_CHANGES_CONTEXT,
];

const REFRESHABLE_PAGE_ACTIVE_CONTEXTS: [&str; 6] = [
  WORKSPACE_GIT_CONTEXT,
  WORKSPACE_GITHUB_HOME_CONTEXT,
  WORKSPACE_GITHUB_REPO_CONTEXT,
  WORKSPACE_GITHUB_REPO_CODE_CONTEXT,
  WORKSPACE_GITHUB_PR_CONTEXT,
  WORKSPACE_GITHUB_PR_CHANGES_CONTEXT,
];

const GIT_ONLY_ACTIVE_CONTEXTS: [&str; 1] = [WORKSPACE_GIT_CONTEXT];

const SECONDARY_PAGE_ACTIVE_CONTEXTS: [&str; 4] = [
  WORKSPACE_BILLING_CONTEXT,
  WORKSPACE_GIT_CONFIG_CONTEXT,
  WORKSPACE_SETTINGS_CONTEXT,
  WORKSPACE_ABOUT_CONTEXT,
];

const GIT_AND_PR_CHANGES_ACTIVE_CONTEXTS: [&str; 2] =
  [WORKSPACE_GIT_CONTEXT, WORKSPACE_GITHUB_PR_CHANGES_CONTEXT];

const PR_PAGE_ACTIVE_CONTEXTS: [&str; 2] = [
  WORKSPACE_GITHUB_PR_CONTEXT,
  WORKSPACE_GITHUB_PR_CHANGES_CONTEXT,
];

const REPO_AND_PR_PAGE_ACTIVE_CONTEXTS: [&str; 4] = [
  WORKSPACE_GITHUB_REPO_CONTEXT,
  WORKSPACE_GITHUB_REPO_CODE_CONTEXT,
  WORKSPACE_GITHUB_PR_CONTEXT,
  WORKSPACE_GITHUB_PR_CHANGES_CONTEXT,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutId {
  ShowCommandPalette,
  NavigateBack,
  OpenGitPage,
  OpenGithubPage,
  RefreshCurrentPage,
  ShowFileSearch,
  OpenRepository,
  CommitChanges,
  PullChanges,
  PushChanges,
  ForcePushChanges,
  CloseWorkspacePage,
  OpenSettingsPage,
  ToggleTerminalSidebar,
  ShowBranchSwitcher,
  OpenGitHistorySidebar,
  OpenGitChangesSidebar,
  ToggleDiffView,
  SwitchToPrBranch,
  PreviousAnnotation,
  NextAnnotation,
  PreviousPageTab,
  NextPageTab,
}

impl ShortcutId {
  pub fn storage_key(self) -> &'static str {
    match self {
      ShortcutId::ShowCommandPalette => "show_command_palette",
      ShortcutId::NavigateBack => "navigate_back",
      ShortcutId::OpenGitPage => "open_git_page",
      ShortcutId::OpenGithubPage => "open_github_page",
      ShortcutId::RefreshCurrentPage => "refresh_current_page",
      ShortcutId::ShowFileSearch => "show_file_search",
      ShortcutId::OpenRepository => "open_repository",
      ShortcutId::CommitChanges => "commit_changes",
      ShortcutId::PullChanges => "pull_changes",
      ShortcutId::PushChanges => "push_changes",
      ShortcutId::ForcePushChanges => "force_push_changes",
      ShortcutId::CloseWorkspacePage => "close_workspace_page",
      ShortcutId::OpenSettingsPage => "open_settings_page",
      ShortcutId::ToggleTerminalSidebar => "toggle_terminal_sidebar",
      ShortcutId::ShowBranchSwitcher => "show_branch_switcher",
      ShortcutId::OpenGitHistorySidebar => "open_git_history_sidebar",
      ShortcutId::OpenGitChangesSidebar => "open_git_changes_sidebar",
      ShortcutId::ToggleDiffView => "toggle_diff_view",
      ShortcutId::SwitchToPrBranch => "switch_to_pr_branch",
      ShortcutId::PreviousAnnotation => "previous_annotation",
      ShortcutId::NextAnnotation => "next_annotation",
      ShortcutId::PreviousPageTab => "previous_page_tab",
      ShortcutId::NextPageTab => "next_page_tab",
    }
  }

  pub fn from_storage_key(value: &str) -> Option<Self> {
    match value {
      "show_command_palette" => Some(ShortcutId::ShowCommandPalette),
      "navigate_back" => Some(ShortcutId::NavigateBack),
      "open_git_page" => Some(ShortcutId::OpenGitPage),
      "open_github_page" => Some(ShortcutId::OpenGithubPage),
      "refresh_current_page" => Some(ShortcutId::RefreshCurrentPage),
      "show_file_search" => Some(ShortcutId::ShowFileSearch),
      "open_repository" => Some(ShortcutId::OpenRepository),
      "commit_changes" => Some(ShortcutId::CommitChanges),
      "pull_changes" => Some(ShortcutId::PullChanges),
      "push_changes" => Some(ShortcutId::PushChanges),
      "force_push_changes" => Some(ShortcutId::ForcePushChanges),
      "close_workspace_page" => Some(ShortcutId::CloseWorkspacePage),
      "open_settings_page" => Some(ShortcutId::OpenSettingsPage),
      "toggle_terminal_sidebar" => Some(ShortcutId::ToggleTerminalSidebar),
      "show_branch_switcher" => Some(ShortcutId::ShowBranchSwitcher),
      "open_git_history_sidebar" => Some(ShortcutId::OpenGitHistorySidebar),
      "open_git_changes_sidebar" => Some(ShortcutId::OpenGitChangesSidebar),
      "toggle_diff_view" => Some(ShortcutId::ToggleDiffView),
      "switch_to_pr_branch" => Some(ShortcutId::SwitchToPrBranch),
      "previous_annotation" => Some(ShortcutId::PreviousAnnotation),
      "next_annotation" => Some(ShortcutId::NextAnnotation),
      "previous_page_tab" => Some(ShortcutId::PreviousPageTab),
      "next_page_tab" => Some(ShortcutId::NextPageTab),
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

const SHORTCUT_DEFINITIONS: [ShortcutDefinition; 23] = [
  ShortcutDefinition {
    id: ShortcutId::ShowCommandPalette,
    title: "Command Palette",
    description: "Open the command palette for the current page.",
    scope_label: "All workspace pages",
    category: ShortcutCategory::Core,
    keystroke: SHOW_COMMAND_PALETTE_SHORTCUT,
    context: WORKSPACE_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &ALL_WORKSPACE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::NavigateBack,
    title: "Back",
    description: "Go back to the previous page in navigation history.",
    scope_label: "All workspace pages",
    category: ShortcutCategory::Core,
    keystroke: "cmd-[",
    context: NAVIGATE_BACK_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &ALL_WORKSPACE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenGitPage,
    title: "Go to Git",
    description: "Switch to the Git page.",
    scope_label: "All workspace pages",
    category: ShortcutCategory::Core,
    keystroke: "cmd-1",
    context: OPEN_GIT_PAGE_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &ALL_WORKSPACE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenGithubPage,
    title: "Go to GitHub",
    description: "Switch to the GitHub page.",
    scope_label: "All workspace pages",
    category: ShortcutCategory::Core,
    keystroke: "cmd-2",
    context: OPEN_GITHUB_PAGE_CONTEXT,
    display_context: WORKSPACE_GITHUB_HOME_CONTEXT,
    active_contexts: &ALL_WORKSPACE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::RefreshCurrentPage,
    title: "Refresh Current Page",
    description: "Refresh the current Git or GitHub page.",
    scope_label: "Git and GitHub pages",
    category: ShortcutCategory::Core,
    keystroke: "cmd-r",
    context: REFRESH_CURRENT_PAGE_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &REFRESHABLE_PAGE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ShowFileSearch,
    title: "File Search",
    description: "Open file search where file navigation is available.",
    scope_label: "Git, Repo Code, and PR Changes pages",
    category: ShortcutCategory::Core,
    keystroke: "cmd-p",
    context: FILE_SEARCH_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &FILE_SEARCH_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::PreviousAnnotation,
    title: "Previous Annotation",
    description: "Jump to the previous conflict or review comment.",
    scope_label: "Git conflicts and PR Changes comments",
    category: ShortcutCategory::Review,
    keystroke: "cmd-alt-up",
    context: REVIEW_ANNOTATION_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_AND_PR_CHANGES_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::NextAnnotation,
    title: "Next Annotation",
    description: "Jump to the next conflict or review comment.",
    scope_label: "Git conflicts and PR Changes comments",
    category: ShortcutCategory::Review,
    keystroke: "cmd-alt-down",
    context: REVIEW_ANNOTATION_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_AND_PR_CHANGES_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::PreviousPageTab,
    title: "Previous Tab",
    description: "Move to the previous repository or pull request tab.",
    scope_label: "Repository and pull request pages",
    category: ShortcutCategory::Review,
    keystroke: "cmd-shift-[",
    context: PAGE_TAB_CONTEXT,
    display_context: WORKSPACE_GITHUB_REPO_CONTEXT,
    active_contexts: &REPO_AND_PR_PAGE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::NextPageTab,
    title: "Next Tab",
    description: "Move to the next repository or pull request tab.",
    scope_label: "Repository and pull request pages",
    category: ShortcutCategory::Review,
    keystroke: "cmd-shift-]",
    context: PAGE_TAB_CONTEXT,
    display_context: WORKSPACE_GITHUB_REPO_CONTEXT,
    active_contexts: &REPO_AND_PR_PAGE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ToggleDiffView,
    title: "Toggle Diff View",
    description: "Switch between inline and split diff view.",
    scope_label: "Git page and PR Changes page",
    category: ShortcutCategory::Review,
    keystroke: "cmd-/",
    context: TOGGLE_DIFF_VIEW_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_AND_PR_CHANGES_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::SwitchToPrBranch,
    title: "Switch To PR Branch",
    description: "Switch the local repo to the current pull request branch.",
    scope_label: "Pull request pages",
    category: ShortcutCategory::Review,
    keystroke: "cmd-.",
    context: SWITCH_TO_PR_BRANCH_CONTEXT,
    display_context: WORKSPACE_GITHUB_PR_CONTEXT,
    active_contexts: &PR_PAGE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenRepository,
    title: "Open Repository",
    description: "Open a local repository from the Git page.",
    scope_label: "Git page",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-o",
    context: OPEN_REPOSITORY_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::CommitChanges,
    title: "Commit Changes",
    description: "Commit changes from the Git page.",
    scope_label: "Git page",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-enter",
    context: COMMIT_CHANGES_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::PullChanges,
    title: "Pull Changes",
    description: "Pull the current branch from its upstream remote.",
    scope_label: "Git page",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-u",
    context: PULL_CHANGES_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::PushChanges,
    title: "Push Changes",
    description: "Push the current branch to its upstream remote.",
    scope_label: "Git page",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-y",
    context: PUSH_CHANGES_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ForcePushChanges,
    title: "Force Push Changes",
    description: "Force push the current branch to its upstream remote.",
    scope_label: "Git page",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-y",
    context: FORCE_PUSH_CHANGES_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ToggleTerminalSidebar,
    title: "Toggle Terminal",
    description: "Show or hide the terminal sidebar on the Git page.",
    scope_label: "Git page",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-j",
    context: TOGGLE_TERMINAL_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::ShowBranchSwitcher,
    title: "Switch Branch",
    description: "Open branch switching on the Git page.",
    scope_label: "Git page",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-b",
    context: SHOW_BRANCH_SWITCHER_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenGitHistorySidebar,
    title: "Open History Sidebar",
    description: "Open the Git history sidebar.",
    scope_label: "Git page",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-h",
    context: OPEN_GIT_HISTORY_SIDEBAR_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenGitChangesSidebar,
    title: "Open Changes Sidebar",
    description: "Open the Git changes sidebar.",
    scope_label: "Git page",
    category: ShortcutCategory::LocalGit,
    keystroke: "cmd-shift-e",
    context: OPEN_GIT_CHANGES_SIDEBAR_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &GIT_ONLY_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::OpenSettingsPage,
    title: "Go to Settings",
    description: "Open settings from anywhere in the workspace.",
    scope_label: "All workspace pages",
    category: ShortcutCategory::App,
    keystroke: "cmd-,",
    context: OPEN_SETTINGS_CONTEXT,
    display_context: WORKSPACE_GIT_CONTEXT,
    active_contexts: &ALL_WORKSPACE_ACTIVE_CONTEXTS,
  },
  ShortcutDefinition {
    id: ShortcutId::CloseWorkspacePage,
    title: "Close Page",
    description: "Close the current secondary workspace page.",
    scope_label: "Settings, Billing, About, and Git Config pages",
    category: ShortcutCategory::App,
    keystroke: "cmd-w",
    context: CLOSE_WORKSPACE_PAGE_CONTEXT,
    display_context: WORKSPACE_SETTINGS_CONTEXT,
    active_contexts: &SECONDARY_PAGE_ACTIVE_CONTEXTS,
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
    let context = shortcut_binding_context(&guarded_shortcut_context(self.context), generation);

    match self.id {
      ShortcutId::ShowCommandPalette => {
        KeyBinding::new(keystroke, ShowCommandPalette, Some(&context))
      }
      ShortcutId::NavigateBack => KeyBinding::new(keystroke, NavigateBack, Some(&context)),
      ShortcutId::OpenGitPage => KeyBinding::new(keystroke, OpenGitPage, Some(&context)),
      ShortcutId::OpenGithubPage => KeyBinding::new(keystroke, OpenGithubPage, Some(&context)),
      ShortcutId::RefreshCurrentPage => {
        KeyBinding::new(keystroke, RefreshCurrentPage, Some(&context))
      }
      ShortcutId::ShowFileSearch => KeyBinding::new(keystroke, ShowFileSearch, Some(&context)),
      ShortcutId::OpenRepository => KeyBinding::new(keystroke, OpenRepository, Some(&context)),
      ShortcutId::CommitChanges => KeyBinding::new(keystroke, CommitChanges, Some(&context)),
      ShortcutId::PullChanges => KeyBinding::new(keystroke, PullChanges, Some(&context)),
      ShortcutId::PushChanges => KeyBinding::new(keystroke, PushChanges, Some(&context)),
      ShortcutId::ForcePushChanges => KeyBinding::new(keystroke, ForcePushChanges, Some(&context)),
      ShortcutId::CloseWorkspacePage => {
        KeyBinding::new(keystroke, CloseWorkspacePage, Some(&context))
      }
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
      ShortcutId::ToggleDiffView => KeyBinding::new(keystroke, ToggleDiffView, Some(&context)),
      ShortcutId::SwitchToPrBranch => KeyBinding::new(keystroke, SwitchToPrBranch, Some(&context)),
      ShortcutId::PreviousAnnotation => {
        KeyBinding::new(keystroke, PreviousAnnotation, Some(&context))
      }
      ShortcutId::NextAnnotation => KeyBinding::new(keystroke, NextAnnotation, Some(&context)),
      ShortcutId::PreviousPageTab => KeyBinding::new(keystroke, PreviousPageTab, Some(&context)),
      ShortcutId::NextPageTab => KeyBinding::new(keystroke, NextPageTab, Some(&context)),
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
  shortcut_definitions()
    .iter()
    .copied()
    .map(|definition| {
      definition.key_binding_with_keystroke(
        effective_shortcut_keystroke_text(definition.id, overrides).as_ref(),
        generation,
      )
    })
    .collect()
}

pub fn key_context_for_pathname(pathname: &str) -> &'static str {
  if pathname.starts_with("/github/") {
    let segments: Vec<&str> = pathname.trim_start_matches('/').split('/').collect();

    if segments.len() >= 6 && segments[3] == "pull" && segments[5] == "changes" {
      return WORKSPACE_GITHUB_PR_CHANGES_CONTEXT;
    }

    if segments.len() >= 5 && segments[3] == "pull" {
      return WORKSPACE_GITHUB_PR_CONTEXT;
    }

    if segments.len() >= 4 && segments[3] == "code" && !segments.contains(&"pull") {
      return WORKSPACE_GITHUB_REPO_CODE_CONTEXT;
    }

    if segments.len() >= 3 {
      return WORKSPACE_GITHUB_REPO_CONTEXT;
    }
  }

  match pathname {
    "/git" => WORKSPACE_GIT_CONTEXT,
    "/github" => WORKSPACE_GITHUB_HOME_CONTEXT,
    "/billing" => WORKSPACE_BILLING_CONTEXT,
    "/git-config" => WORKSPACE_GIT_CONFIG_CONTEXT,
    "/settings" => WORKSPACE_SETTINGS_CONTEXT,
    "/about" => WORKSPACE_ABOUT_CONTEXT,
    _ => WORKSPACE_GIT_CONTEXT,
  }
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
  format!(
    "({context}) && !{COMMAND_PALETTE_CONTEXT} && !{WORKSPACE_SHORTCUT_RECORDING_CONTEXT} && !{GIT_REPO_SELECT_CONTEXT} && !{GIT_BRANCH_SELECT_CONTEXT}"
  )
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
    ShortcutId::OpenGitPage => f(&OpenGitPage),
    ShortcutId::OpenGithubPage => f(&OpenGithubPage),
    ShortcutId::RefreshCurrentPage => f(&RefreshCurrentPage),
    ShortcutId::ShowFileSearch => f(&ShowFileSearch),
    ShortcutId::OpenRepository => f(&OpenRepository),
    ShortcutId::CommitChanges => f(&CommitChanges),
    ShortcutId::PullChanges => f(&PullChanges),
    ShortcutId::PushChanges => f(&PushChanges),
    ShortcutId::ForcePushChanges => f(&ForcePushChanges),
    ShortcutId::CloseWorkspacePage => f(&CloseWorkspacePage),
    ShortcutId::OpenSettingsPage => f(&OpenSettingsPage),
    ShortcutId::ToggleTerminalSidebar => f(&ToggleTerminalSidebar),
    ShortcutId::ShowBranchSwitcher => f(&ShowBranchSwitcher),
    ShortcutId::OpenGitHistorySidebar => f(&OpenGitHistorySidebar),
    ShortcutId::OpenGitChangesSidebar => f(&OpenGitChangesSidebar),
    ShortcutId::ToggleDiffView => f(&ToggleDiffView),
    ShortcutId::SwitchToPrBranch => f(&SwitchToPrBranch),
    ShortcutId::PreviousAnnotation => f(&PreviousAnnotation),
    ShortcutId::NextAnnotation => f(&NextAnnotation),
    ShortcutId::PreviousPageTab => f(&PreviousPageTab),
    ShortcutId::NextPageTab => f(&NextPageTab),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

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
  fn shortcut_definition_lookup_returns_expected_definition() {
    let definition = shortcut_definition(ShortcutId::CommitChanges);
    assert_eq!(definition.title, "Commit Changes");
    assert_eq!(definition.scope_label, "Git page");
    assert_eq!(
      shortcut_keystroke(ShortcutId::CommitChanges),
      Keystroke::parse("cmd-enter").unwrap()
    );
  }

  #[test]
  fn refresh_current_page_binding_is_limited_to_refreshable_workspace_routes() {
    assert!(has_binding("/git", "cmd-r"));
    assert!(has_binding("/github", "cmd-r"));
    assert!(has_binding("/github/owner/repo", "cmd-r"));
    assert!(has_binding("/github/owner/repo/code", "cmd-r"));
    assert!(has_binding("/github/owner/repo/pull/42", "cmd-r"));
    assert!(has_binding("/github/owner/repo/pull/42/changes", "cmd-r"));
    assert!(!has_binding("/settings", "cmd-r"));
    assert!(!has_binding("/billing", "cmd-r"));
  }

  #[test]
  fn key_context_for_pathname_matches_workspace_routes() {
    assert_eq!(key_context_for_pathname("/git"), WORKSPACE_GIT_CONTEXT);
    assert_eq!(
      key_context_for_pathname("/github"),
      WORKSPACE_GITHUB_HOME_CONTEXT
    );
    assert_eq!(
      key_context_for_pathname("/github/owner/repo"),
      WORKSPACE_GITHUB_REPO_CONTEXT
    );
    assert_eq!(
      key_context_for_pathname("/github/owner/repo/code"),
      WORKSPACE_GITHUB_REPO_CODE_CONTEXT
    );
    assert_eq!(
      key_context_for_pathname("/github/owner/repo/commit/abc123"),
      WORKSPACE_GITHUB_REPO_CONTEXT
    );
    assert_eq!(
      key_context_for_pathname("/github/owner/repo/pull/42"),
      WORKSPACE_GITHUB_PR_CONTEXT
    );
    assert_eq!(
      key_context_for_pathname("/github/owner/repo/pull/42/changes"),
      WORKSPACE_GITHUB_PR_CHANGES_CONTEXT
    );
    assert_eq!(
      key_context_for_pathname("/settings"),
      WORKSPACE_SETTINGS_CONTEXT
    );
  }

  #[test]
  fn current_key_context_for_pathname_appends_shortcut_generation() {
    assert_eq!(
      key_context_for_pathname_with_generation("/git", 0),
      format!("{WORKSPACE_GIT_CONTEXT} {SHORTCUT_KEYMAP_GENERATION_CONTEXT_KEY}=0")
    );

    assert_eq!(
      key_context_for_pathname_with_generation("/git", 1),
      format!("{WORKSPACE_GIT_CONTEXT} {SHORTCUT_KEYMAP_GENERATION_CONTEXT_KEY}=1")
    );
  }

  #[test]
  fn command_palette_binding_is_available_in_all_workspace_contexts() {
    assert!(has_binding("/git", SHOW_COMMAND_PALETTE_SHORTCUT));
    assert!(has_binding("/github", SHOW_COMMAND_PALETTE_SHORTCUT));
    assert!(has_binding(
      "/github/owner/repo",
      SHOW_COMMAND_PALETTE_SHORTCUT
    ));
    assert!(has_binding("/settings", SHOW_COMMAND_PALETTE_SHORTCUT));
  }

  #[test]
  fn file_search_binding_is_limited_to_supported_routes() {
    assert!(has_binding("/git", "cmd-p"));
    assert!(has_binding("/github/owner/repo/code", "cmd-p"));
    assert!(has_binding("/github/owner/repo/pull/42/changes", "cmd-p"));
    assert!(!has_binding("/github", "cmd-p"));
    assert!(!has_binding("/github/owner/repo", "cmd-p"));
    assert!(!has_binding("/github/owner/repo/pull/42", "cmd-p"));
    assert!(!has_binding("/settings", "cmd-p"));
  }

  #[test]
  fn git_only_shortcuts_are_scoped_to_the_git_page() {
    assert!(has_binding("/git", "cmd-o"));
    assert!(has_binding("/git", "cmd-enter"));
    assert!(has_binding("/git", "cmd-u"));
    assert!(has_binding("/git", "cmd-y"));
    assert!(has_binding("/git", "cmd-shift-y"));
    assert!(!has_binding("/github", "cmd-o"));
    assert!(!has_binding("/github/owner/repo/code", "cmd-enter"));
    assert!(!has_binding("/github", "cmd-u"));
    assert!(!has_binding("/github", "cmd-y"));
    assert!(!has_binding("/github", "cmd-shift-y"));
    assert!(!has_binding("/settings", "cmd-o"));
    assert!(!has_binding("/settings", "cmd-u"));
    assert!(!has_binding("/settings", "cmd-y"));
    assert!(!has_binding("/settings", "cmd-shift-y"));
  }

  #[test]
  fn close_page_binding_is_limited_to_secondary_workspace_pages() {
    assert!(has_binding("/settings", "cmd-w"));
    assert!(has_binding("/billing", "cmd-w"));
    assert!(has_binding("/git-config", "cmd-w"));
    assert!(has_binding("/about", "cmd-w"));
    assert!(!has_binding("/git", "cmd-w"));
    assert!(!has_binding("/github", "cmd-w"));
    assert!(!has_binding("/github/owner/repo", "cmd-w"));
  }

  #[test]
  fn workspace_navigation_shortcuts_are_available_across_workspace_pages() {
    assert!(has_binding("/git", "cmd-,"));
    assert!(has_binding("/github", "cmd-,"));
    assert!(has_binding("/github/owner/repo/pull/42", "cmd-,"));
    assert!(has_binding("/settings", "cmd-,"));
  }

  #[test]
  fn core_navigation_shortcuts_are_available_across_workspace_pages() {
    for pathname in ["/git", "/github", "/github/owner/repo/pull/42", "/settings"] {
      assert!(has_binding(pathname, "cmd-["));
      assert!(has_binding(pathname, "cmd-1"));
      assert!(has_binding(pathname, "cmd-2"));
    }
  }

  #[test]
  fn git_keyboard_first_shortcuts_are_scoped_to_git_page() {
    for keystroke in [
      "cmd-j",
      "cmd-u",
      "cmd-y",
      "cmd-shift-y",
      "cmd-shift-b",
      "cmd-shift-h",
      "cmd-shift-e",
    ] {
      assert!(
        has_binding("/git", keystroke),
        "{keystroke} should be active on /git"
      );
      assert!(
        !has_binding("/github", keystroke),
        "{keystroke} should not be active on /github"
      );
      assert!(
        !has_binding("/github/owner/repo/pull/42/changes", keystroke),
        "{keystroke} should not be active on PR changes"
      );
    }
  }

  #[test]
  fn review_shortcuts_are_scoped_to_pr_pages_and_changes() {
    assert!(has_binding("/git", "cmd-/"));
    assert!(has_binding("/github/owner/repo/pull/42/changes", "cmd-/"));
    assert!(!has_binding("/github/owner/repo/pull/42", "cmd-/"));

    assert!(has_binding("/github/owner/repo/pull/42", "cmd-."));
    assert!(has_binding("/github/owner/repo/pull/42/changes", "cmd-."));
    assert!(!has_binding("/git", "cmd-."));
    assert!(!has_binding("/github/owner/repo/code", "cmd-."));
  }

  #[test]
  fn annotation_shortcuts_are_scoped_to_git_and_pr_changes() {
    for keystroke in ["cmd-alt-up", "cmd-alt-down"] {
      assert!(has_binding("/git", keystroke));
      assert!(has_binding("/github/owner/repo/pull/42/changes", keystroke));
      assert!(!has_binding("/github", keystroke));
      assert!(!has_binding("/github/owner/repo", keystroke));
      assert!(!has_binding("/github/owner/repo/pull/42", keystroke));
    }
  }

  #[test]
  fn page_tab_shortcuts_are_scoped_to_repo_and_pr_pages() {
    for keystroke in ["cmd-shift-[", "cmd-shift-]"] {
      assert!(!has_binding("/git", keystroke));
      assert!(!has_binding("/github", keystroke));
      assert!(has_binding("/github/owner/repo", keystroke));
      assert!(has_binding("/github/owner/repo/code", keystroke));
      assert!(has_binding("/github/owner/repo/pull/42", keystroke));
      assert!(has_binding("/github/owner/repo/pull/42/changes", keystroke));
    }
  }

  #[test]
  fn workspace_key_bindings_apply_overrides() {
    let overrides = overrides(&[(ShortcutId::ShowFileSearch, "cmd-shift-p")]);
    let bindings = workspace_key_bindings_with_overrides(&overrides);

    assert!(has_binding_with_bindings(
      "/git",
      "cmd-shift-p",
      bindings.clone()
    ));
    assert!(!has_binding_with_bindings("/git", "cmd-p", bindings));
  }

  #[test]
  fn newer_workspace_shortcut_generations_shadow_previous_bindings() {
    let mut keymap = Keymap::default();
    keymap.add_bindings(workspace_key_bindings_with_overrides_and_generation(
      &ShortcutOverrides::default(),
      1,
    ));
    keymap.add_bindings(workspace_key_bindings_with_overrides_and_generation(
      &overrides(&[(ShortcutId::ShowFileSearch, "cmd-shift-p")]),
      2,
    ));

    let current_context =
      [KeyContext::parse(&key_context_for_pathname_with_generation("/git", 2)).unwrap()];
    let old_input = [Keystroke::parse("cmd-p").unwrap()];
    let new_input = [Keystroke::parse("cmd-shift-p").unwrap()];

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
      "/git",
      &[COMMAND_PALETTE_CONTEXT],
      SHOW_COMMAND_PALETTE_SHORTCUT,
      bindings.clone(),
    ));
    assert!(!has_binding_with_bindings_in_contexts(
      "/git",
      &[COMMAND_PALETTE_CONTEXT],
      "cmd-p",
      bindings,
    ));
  }

  #[test]
  fn repo_and_branch_select_contexts_disable_git_page_shortcuts() {
    let bindings = workspace_key_bindings();

    for context in [GIT_REPO_SELECT_CONTEXT, GIT_BRANCH_SELECT_CONTEXT] {
      assert!(!has_binding_with_bindings_in_contexts(
        "/git",
        &[context],
        SHOW_COMMAND_PALETTE_SHORTCUT,
        bindings.clone(),
      ));
      assert!(!has_binding_with_bindings_in_contexts(
        "/git",
        &[context],
        "cmd-p",
        bindings.clone(),
      ));
      assert!(!has_binding_with_bindings_in_contexts(
        "/git",
        &[context],
        "cmd-o",
        bindings.clone(),
      ));
      assert!(!has_binding_with_bindings_in_contexts(
        "/git",
        &[context],
        "cmd-enter",
        bindings.clone(),
      ));
    }
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
      "cmd-w",
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
    .expect_err("git page overlap should be rejected");

    assert_eq!(
      error,
      ShortcutOverrideError::ShortcutConflict {
        shortcut_id: ShortcutId::CommitChanges,
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
