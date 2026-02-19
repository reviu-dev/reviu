use std::{rc::Rc, sync::Arc};

use crate::{UiIconName, file_icon_path_for_name};
use gpui::{
  App, Context, Div, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
  ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, IndexPath, Sizable, WindowExt,
  button::{Button, ButtonVariants},
  h_flex,
  input::{Input, InputEvent, InputState},
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  v_flex,
};

const LIST_INPUT_HEIGHT: f32 = 35.0;
const LIST_ITEM_HEIGHT: f32 = 32.0; // Height of each list item in pixels (h_8)

fn list_base_item(
  ix: IndexPath,
  total_items: usize,
  selected_index: Option<IndexPath>,
  theme: &gpui_component::Theme,
) -> ListItem {
  let is_last_item = ix.row + 1 == total_items;

  ListItem::new(ix)
    .h_8()
    .when(is_last_item, |item| item.rounded_b(theme.radius))
    .selected(Some(ix) == selected_index)
}

fn update_selected_index<D: ListDelegate>(
  selected_index: &mut Option<IndexPath>,
  ix: Option<IndexPath>,
  cx: &mut Context<ListState<D>>,
) {
  *selected_index = ix;
  cx.notify();
}

#[derive(Clone, Debug)]
pub struct CommandPaletteCommand {
  pub id: CommandPaletteCommandId,
  pub name: SharedString,
  pub description: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandPalettePage {
  Git,
  Github,
  GithubPrDetails,
  GitConfig,
  Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandPaletteBranchKind {
  Local,
  Remote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandPaletteBranch {
  pub name: SharedString,
  pub kind: CommandPaletteBranchKind,
}

impl CommandPaletteBranch {
  fn matches(&self, query: &str) -> bool {
    if query.is_empty() {
      return true;
    }
    self.name.as_ref().to_lowercase().contains(query)
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandPaletteRepository {
  pub path: SharedString,
}

impl CommandPaletteRepository {
  fn matches(&self, query: &str) -> bool {
    if query.is_empty() {
      return true;
    }
    self.path.as_ref().to_lowercase().contains(query)
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandPaletteStash {
  pub index: usize,
  pub name: SharedString,
  pub oid: SharedString,
}

impl CommandPaletteStash {
  fn matches(&self, query: &str) -> bool {
    if query.is_empty() {
      return true;
    }

    let stash_index = format!("#{}", self.index);
    self.name.as_ref().to_lowercase().contains(query)
      || self.oid.as_ref().to_lowercase().contains(query)
      || stash_index.contains(query)
      || self.index.to_string().contains(query)
  }
}

#[derive(Clone, Debug)]
pub enum CommandPaletteAction {
  SwitchRepository(CommandPaletteRepository),
  SwitchBranch(CommandPaletteBranch),
  Commit,
  ContinueRebase,
  SkipRebase,
  Push,
  ForcePush,
  UndoLastCommit,
  Amend,
  AcceptAllCurrentConflicts,
  AcceptAllIncomingConflicts,
  CreateBranch {
    name: String,
  },
  CreateBranchFrom {
    name: String,
    base: CommandPaletteBranch,
  },
  MergeBranch {
    name: CommandPaletteBranch,
  },
  AbortMerge,
  RebaseBranch {
    name: CommandPaletteBranch,
  },
  AbortRebase,
  CherryPick {
    commit_hashes: Vec<String>,
  },
  StageAll,
  UnstageAll,
  Fetch,
  Stash {
    include_untracked: bool,
    message: Option<String>,
  },
  ApplyStash(CommandPaletteStash),
  DropStash(CommandPaletteStash),
  PopStash(CommandPaletteStash),
  OpenRepository,
  OpenGitPage,
  OpenGithubPage,
  OpenGithubPrDetails {
    owner: String,
    repo: String,
    number: u64,
  },
  OpenGitHistorySidebar,
  OpenGitChangesSidebar,
  OpenGitConfigPage,
  OpenSettingsPage,
}

struct BranchesListDelegate {
  _branches: Vec<Rc<CommandPaletteBranch>>,
  matched_branches: Vec<Rc<CommandPaletteBranch>>,
  selected_index: Option<IndexPath>,
  query: SharedString,
}

impl BranchesListDelegate {
  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();

    let q = self.query.as_ref().to_lowercase(); // String

    let branches: Vec<Rc<CommandPaletteBranch>> = self
      ._branches
      .iter()
      .filter(|branch| branch.matches(&q))
      .cloned()
      .collect();

    self.matched_branches = branches;
  }
}

impl ListDelegate for BranchesListDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.matched_branches.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let total_items = self.matched_branches.len();
    let theme = cx.theme().clone();

    let base_item = list_base_item(ix, total_items, self.selected_index, &theme);

    self.matched_branches.get(ix.row).map(|branch| {
      base_item.child(
        h_flex()
          .items_center()
          .gap_2()
          .child(Icon::new(UiIconName::GitBranch))
          .child(
            div()
              .flex_1()
              .overflow_hidden()
              .text_ellipsis()
              .child(Label::new(branch.name.clone())),
          ),
      )
    })
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    update_selected_index(&mut self.selected_index, ix, cx);
  }

  fn perform_search(
    &mut self,
    query: &str,
    _: &mut Window,
    _: &mut Context<ListState<Self>>,
  ) -> Task<()> {
    self.prepare(query.to_owned());
    Task::ready(())
  }
}

struct RepositoriesListDelegate {
  _repositories: Vec<Rc<CommandPaletteRepository>>,
  matched_repositories: Vec<Rc<CommandPaletteRepository>>,
  selected_index: Option<IndexPath>,
  query: SharedString,
}

impl RepositoriesListDelegate {
  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();

    let q = self.query.as_ref().to_lowercase();

    let repositories: Vec<Rc<CommandPaletteRepository>> = self
      ._repositories
      .iter()
      .filter(|repository| repository.matches(&q))
      .cloned()
      .collect();

    self.matched_repositories = repositories;
  }
}

impl ListDelegate for RepositoriesListDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.matched_repositories.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let total_items = self.matched_repositories.len();
    let theme = cx.theme().clone();

    let base_item = list_base_item(ix, total_items, self.selected_index, &theme);

    self.matched_repositories.get(ix.row).map(|repository| {
      base_item.child(
        h_flex()
          .items_center()
          .gap_2()
          .child(Icon::new(IconName::FolderOpen))
          .child(
            div()
              .flex_1()
              .overflow_hidden()
              .text_ellipsis()
              .child(Label::new(repository.path.clone())),
          ),
      )
    })
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    update_selected_index(&mut self.selected_index, ix, cx);
  }

  fn perform_search(
    &mut self,
    query: &str,
    _: &mut Window,
    _: &mut Context<ListState<Self>>,
  ) -> Task<()> {
    self.prepare(query.to_owned());
    Task::ready(())
  }
}

struct StashesListDelegate {
  _stashes: Vec<Rc<CommandPaletteStash>>,
  matched_stashes: Vec<Rc<CommandPaletteStash>>,
  selected_index: Option<IndexPath>,
  query: SharedString,
}

impl StashesListDelegate {
  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();

    let q = self.query.as_ref().to_lowercase();

    let stashes = self
      ._stashes
      .iter()
      .filter(|stash| stash.matches(&q))
      .cloned()
      .collect();

    self.matched_stashes = stashes;
  }
}

impl ListDelegate for StashesListDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.matched_stashes.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let total_items = self.matched_stashes.len();
    let theme = cx.theme().clone();
    let base_item = list_base_item(ix, total_items, self.selected_index, &theme);

    self.matched_stashes.get(ix.row).map(|stash| {
      let label: SharedString = format!("#{} {}", stash.index, stash.name.as_ref()).into();
      base_item.child(
        h_flex()
          .items_center()
          .gap_2()
          .child(Icon::new(IconName::Inbox))
          .child(
            div()
              .flex_1()
              .overflow_hidden()
              .text_ellipsis()
              .child(Label::new(label)),
          ),
      )
    })
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    update_selected_index(&mut self.selected_index, ix, cx);
  }

  fn perform_search(
    &mut self,
    query: &str,
    _: &mut Window,
    _: &mut Context<ListState<Self>>,
  ) -> Task<()> {
    self.prepare(query.to_owned());
    Task::ready(())
  }
}

#[derive(Clone, Debug)]
pub enum BranchListWithCommands {
  SwitchBranch(Rc<CommandPaletteBranch>),
  CommandPaletteCommand(CommandPaletteCommand),
}

struct BranchesListWithCommandsDelegate {
  _branches_with_commands: Vec<Rc<BranchListWithCommands>>,
  matched_branches_and_commands: Vec<Rc<BranchListWithCommands>>,
  selected_index: Option<IndexPath>,
  query: SharedString,
}

impl BranchesListWithCommandsDelegate {
  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();

    let q = self.query.as_ref().to_lowercase(); // String

    let branches: Vec<Rc<BranchListWithCommands>> = self
      ._branches_with_commands
      .iter()
      .filter(|branch| match branch.as_ref() {
        BranchListWithCommands::SwitchBranch(branch) => branch.matches(&q),
        BranchListWithCommands::CommandPaletteCommand(command) => command.matches(&q),
      })
      .cloned()
      .collect();

    self.matched_branches_and_commands = branches;
  }
}

impl ListDelegate for BranchesListWithCommandsDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.matched_branches_and_commands.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let total_items = self.matched_branches_and_commands.len();
    let theme = cx.theme().clone();

    let base_item = list_base_item(ix, total_items, self.selected_index, &theme);

    self
      .matched_branches_and_commands
      .get(ix.row)
      .map(|branch| match branch.as_ref() {
        BranchListWithCommands::CommandPaletteCommand(command) => base_item.child(
          h_flex()
            .items_center()
            .gap_2()
            .child(command.icon())
            .child(Label::new(command.name.clone())),
        ),
        BranchListWithCommands::SwitchBranch(branch) => base_item.child(
          h_flex()
            .items_center()
            .gap_2()
            .child(Icon::new(UiIconName::GitBranch))
            .child(
              div()
                .flex_1()
                .overflow_hidden()
                .text_ellipsis()
                .child(Label::new(branch.name.clone())),
            ),
        ),
      })
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    update_selected_index(&mut self.selected_index, ix, cx);
  }

  fn perform_search(
    &mut self,
    query: &str,
    _: &mut Window,
    _: &mut Context<ListState<Self>>,
  ) -> Task<()> {
    self.prepare(query.to_owned());
    Task::ready(())
  }
}

struct CommandListDelegate {
  _commands: Vec<Rc<CommandPaletteCommand>>,
  matched_commands: Vec<Rc<CommandPaletteCommand>>,
  selected_index: Option<IndexPath>,
  query: SharedString,
}

impl CommandListDelegate {
  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();

    let commands: Vec<Rc<CommandPaletteCommand>> = self
      ._commands
      .iter()
      .filter(|command| command.matches(&self.query))
      .cloned()
      .collect();

    self.matched_commands = commands;
  }
}

impl ListDelegate for CommandListDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.matched_commands.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let total_items = self.matched_commands.len();
    let theme = cx.theme().clone();

    self.matched_commands.get(ix.row).map(|command| {
      list_base_item(ix, total_items, self.selected_index, &theme)
        .child(
          h_flex()
            .items_center()
            .gap_2()
            .child(command.icon())
            .child(Label::new(command.name.clone())),
        )
        .suffix(|_, _| {
          Button::new("action")
            .ghost()
            .small()
            .icon(IconName::ArrowRight)
        })
    })
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    update_selected_index(&mut self.selected_index, ix, cx);
  }

  fn perform_search(
    &mut self,
    query: &str,
    _: &mut Window,
    _: &mut Context<ListState<Self>>,
  ) -> Task<()> {
    self.prepare(query.to_owned());
    Task::ready(())
  }
}

pub type CommandPaletteHandler = Arc<
  dyn Fn(CommandPaletteAction, &mut Window, &mut App) -> Result<(), SharedString> + Send + Sync,
>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandPaletteCommandId {
  SwitchRepository,
  SwitchBranch,
  Commit,
  ContinueRebase,
  SkipRebase,
  Push,
  ForcePush,
  UndoLastCommit,
  Amend,
  AcceptAllCurrentConflicts,
  AcceptAllIncomingConflicts,
  CreateBranch,
  CreateBranchFrom,
  MergeBranch,
  AbortMerge,
  RebaseBranch,
  AbortRebase,
  CherryPick,
  StageAll,
  UnstageAll,
  Fetch,
  Stash,
  StashIncludeUntracked,
  ApplyStash,
  DropStash,
  PopStash,
  OpenRepository,
  OpenGitPage,
  OpenGithubPage,
  OpenGithubPrFromUrl,
  OpenGitHistorySidebar,
  OpenGitChangesSidebar,
  OpenGitConfigPage,
  OpenSettingsPage,
}

impl CommandPaletteCommand {
  fn git_config_icon() -> Icon {
    file_icon_path_for_name(".gitconfig").map_or_else(
      || Icon::new(IconName::File),
      |path| Icon::empty().path(path),
    )
  }

  pub fn switch_repository() -> Self {
    Self {
      id: CommandPaletteCommandId::SwitchRepository,
      name: "Switch repo".into(),
      description: Some("Switch to another recent repository".into()),
    }
  }

  pub fn switch_branch() -> Self {
    Self {
      id: CommandPaletteCommandId::SwitchBranch,
      name: "Switch branch".into(),
      description: Some("Checkout or create branches".into()),
    }
  }

  pub fn commit() -> Self {
    Self {
      id: CommandPaletteCommandId::Commit,
      name: "Commit".into(),
      description: Some("Create a commit (stages all changes if needed)".into()),
    }
  }

  pub fn continue_rebase() -> Self {
    Self {
      id: CommandPaletteCommandId::ContinueRebase,
      name: "Rebase continue".into(),
      description: Some("Continue the current rebase".into()),
    }
  }

  pub fn skip_rebase() -> Self {
    Self {
      id: CommandPaletteCommandId::SkipRebase,
      name: "Rebase skip".into(),
      description: Some("Skip the current rebase commit".into()),
    }
  }

  pub fn push(label: impl Into<SharedString>) -> Self {
    Self {
      id: CommandPaletteCommandId::Push,
      name: label.into(),
      description: Some("Push local commits to the remote branch".into()),
    }
  }

  pub fn force_push() -> Self {
    Self {
      id: CommandPaletteCommandId::ForcePush,
      name: "Force push (with lease)".into(),
      description: Some("Force push local commits to the remote branch".into()),
    }
  }

  pub fn undo_last_commit() -> Self {
    Self {
      id: CommandPaletteCommandId::UndoLastCommit,
      name: "Undo last commit".into(),
      description: Some("Undo the most recent local commit".into()),
    }
  }

  pub fn amend() -> Self {
    Self {
      id: CommandPaletteCommandId::Amend,
      name: "Amend".into(),
      description: Some("Amend the most recent commit".into()),
    }
  }

  pub fn accept_all_current_conflicts() -> Self {
    Self {
      id: CommandPaletteCommandId::AcceptAllCurrentConflicts,
      name: "Accept all current conflicts".into(),
      description: Some("Resolve all conflict regions by keeping current changes".into()),
    }
  }

  pub fn accept_all_incoming_conflicts() -> Self {
    Self {
      id: CommandPaletteCommandId::AcceptAllIncomingConflicts,
      name: "Accept all incoming conflicts".into(),
      description: Some("Resolve all conflict regions by keeping incoming changes".into()),
    }
  }

  pub fn merge_branch() -> Self {
    Self {
      id: CommandPaletteCommandId::MergeBranch,
      name: "Merge branch".into(),
      description: Some("Merge a branch into the current branch".into()),
    }
  }

  pub fn rebase_branch() -> Self {
    Self {
      id: CommandPaletteCommandId::RebaseBranch,
      name: "Rebase branch".into(),
      description: Some("Rebase the current branch onto another branch".into()),
    }
  }

  pub fn abort_merge() -> Self {
    Self {
      id: CommandPaletteCommandId::AbortMerge,
      name: "Abort merge".into(),
      description: Some("Abort the current merge operation".into()),
    }
  }

  pub fn abort_rebase() -> Self {
    Self {
      id: CommandPaletteCommandId::AbortRebase,
      name: "Abort rebase".into(),
      description: Some("Abort the current rebase operation".into()),
    }
  }

  pub fn create_branch() -> Self {
    Self {
      id: CommandPaletteCommandId::CreateBranch,
      name: "Create branch".into(),
      description: Some("Create a new branch".into()),
    }
  }

  pub fn create_branch_from() -> Self {
    Self {
      id: CommandPaletteCommandId::CreateBranchFrom,
      name: "Create branch from...".into(),
      description: Some("Create a new branch from an existing branch".into()),
    }
  }

  pub fn cherry_pick() -> Self {
    Self {
      id: CommandPaletteCommandId::CherryPick,
      name: "Cherry pick".into(),
      description: Some("Apply one or more commits to the current branch".into()),
    }
  }

  pub fn stage_all() -> Self {
    Self {
      id: CommandPaletteCommandId::StageAll,
      name: "Stage all".into(),
      description: Some("Stage all changed files".into()),
    }
  }

  pub fn unstage_all() -> Self {
    Self {
      id: CommandPaletteCommandId::UnstageAll,
      name: "Unstage all".into(),
      description: Some("Unstage all staged files".into()),
    }
  }

  pub fn fetch() -> Self {
    Self {
      id: CommandPaletteCommandId::Fetch,
      name: "Fetch".into(),
      description: Some("Fetch updates from remote repositories".into()),
    }
  }

  pub fn stash() -> Self {
    Self {
      id: CommandPaletteCommandId::Stash,
      name: "Stash".into(),
      description: Some("Stash tracked changes".into()),
    }
  }

  pub fn stash_with_untracked() -> Self {
    Self {
      id: CommandPaletteCommandId::StashIncludeUntracked,
      name: "Stash with untracked".into(),
      description: Some("Stash tracked and untracked changes".into()),
    }
  }

  pub fn apply_stash() -> Self {
    Self {
      id: CommandPaletteCommandId::ApplyStash,
      name: "Apply stash".into(),
      description: Some("Apply a stash entry without dropping it".into()),
    }
  }

  pub fn drop_stash() -> Self {
    Self {
      id: CommandPaletteCommandId::DropStash,
      name: "Drop stash".into(),
      description: Some("Delete a stash entry".into()),
    }
  }

  pub fn pop_stash() -> Self {
    Self {
      id: CommandPaletteCommandId::PopStash,
      name: "Pop stash".into(),
      description: Some("Apply and delete a stash entry".into()),
    }
  }

  pub fn open_repository() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenRepository,
      name: "Open repository".into(),
      description: Some("Pick and open a local repository".into()),
    }
  }

  pub fn open_github_page() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenGithubPage,
      name: "Open GitHub page".into(),
      description: Some("Go to the GitHub page".into()),
    }
  }

  pub fn open_git_page() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenGitPage,
      name: "Open Git page".into(),
      description: Some("Go to the Git page".into()),
    }
  }

  pub fn open_github_pr_from_url() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenGithubPrFromUrl,
      name: "Open GitHub PR from URL".into(),
      description: Some("Open a pull request details page from a GitHub URL".into()),
    }
  }

  pub fn open_git_history_sidebar() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenGitHistorySidebar,
      name: "Open History in sidebar".into(),
      description: Some("Switch Git sidebar to History".into()),
    }
  }

  pub fn open_git_changes_sidebar() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenGitChangesSidebar,
      name: "Open Changes in sidebar".into(),
      description: Some("Switch Git sidebar to Changes".into()),
    }
  }

  pub fn open_settings_page() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenSettingsPage,
      name: "Open Settings".into(),
      description: Some("Go to Settings".into()),
    }
  }

  pub fn open_git_config_page() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenGitConfigPage,
      name: "Open Git Config".into(),
      description: Some("Edit ~/.gitconfig".into()),
    }
  }

  pub fn default_global_commands(
    current_page: CommandPalettePage,
    include_github: bool,
  ) -> Vec<Self> {
    let mut commands = Vec::new();

    if current_page != CommandPalettePage::Git {
      commands.push(Self::open_git_page());
    }

    if include_github && current_page != CommandPalettePage::Github {
      commands.push(Self::open_github_page());
    }

    if include_github {
      commands.push(Self::open_github_pr_from_url());
    }

    if current_page != CommandPalettePage::GitConfig {
      commands.push(Self::open_git_config_page());
    }

    if current_page != CommandPalettePage::Settings {
      commands.push(Self::open_settings_page());
    }

    if current_page == CommandPalettePage::Git {
      commands.push(Self::open_git_history_sidebar());
      commands.push(Self::open_git_changes_sidebar());
    }

    commands
  }

  fn icon(&self) -> Icon {
    match self.id {
      CommandPaletteCommandId::SwitchRepository => Icon::new(IconName::FolderOpen),
      CommandPaletteCommandId::SwitchBranch => Icon::new(UiIconName::GitBranch),
      CommandPaletteCommandId::Commit => Icon::new(IconName::Check),
      CommandPaletteCommandId::ContinueRebase => Icon::new(IconName::Check),
      CommandPaletteCommandId::SkipRebase => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::Push => Icon::new(IconName::ArrowUp),
      CommandPaletteCommandId::ForcePush => Icon::new(IconName::TriangleAlert),
      CommandPaletteCommandId::UndoLastCommit => Icon::new(IconName::Undo),
      CommandPaletteCommandId::Amend => Icon::new(IconName::Replace),
      CommandPaletteCommandId::AcceptAllCurrentConflicts => Icon::new(IconName::Replace),
      CommandPaletteCommandId::AcceptAllIncomingConflicts => Icon::new(IconName::Replace),
      CommandPaletteCommandId::MergeBranch => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::AbortMerge => Icon::new(IconName::Undo),
      CommandPaletteCommandId::RebaseBranch => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::AbortRebase => Icon::new(IconName::Undo),
      CommandPaletteCommandId::CherryPick => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::StageAll => Icon::new(IconName::Plus),
      CommandPaletteCommandId::UnstageAll => Icon::new(UiIconName::ArrowUpFromLine),
      CommandPaletteCommandId::Fetch => Icon::new(UiIconName::RefreshCcw),
      CommandPaletteCommandId::Stash | CommandPaletteCommandId::StashIncludeUntracked => {
        Icon::new(UiIconName::ArrowDownFromLine)
      }
      CommandPaletteCommandId::ApplyStash | CommandPaletteCommandId::PopStash => {
        Icon::new(UiIconName::ArrowUpFromLine)
      }
      CommandPaletteCommandId::DropStash => Icon::new(IconName::Delete),
      CommandPaletteCommandId::OpenRepository => Icon::new(IconName::FolderOpen),
      CommandPaletteCommandId::CreateBranch | CommandPaletteCommandId::CreateBranchFrom => {
        Icon::new(IconName::Plus)
      }
      CommandPaletteCommandId::OpenGitPage => Icon::new(UiIconName::GitBranch),
      CommandPaletteCommandId::OpenGithubPage => Icon::new(IconName::GitHub),
      CommandPaletteCommandId::OpenGithubPrFromUrl => Icon::new(IconName::GitHub),
      CommandPaletteCommandId::OpenGitHistorySidebar => Icon::new(UiIconName::History),
      CommandPaletteCommandId::OpenGitChangesSidebar => Icon::new(UiIconName::FileCode),
      CommandPaletteCommandId::OpenGitConfigPage => Self::git_config_icon(),
      CommandPaletteCommandId::OpenSettingsPage => Icon::new(IconName::Settings2),
    }
  }

  fn matches(&self, query: &str) -> bool {
    if self.id == CommandPaletteCommandId::OpenGithubPrFromUrl
      && CommandPalette::parse_github_pull_request_url(query).is_some()
    {
      return true;
    }

    if query.is_empty() {
      return true;
    }
    let query = query.to_lowercase();
    if self.name.as_ref().to_lowercase().contains(&query) {
      return true;
    }
    self
      .description
      .as_ref()
      .map(|text| text.as_ref().to_lowercase().contains(&query))
      .unwrap_or(false)
  }
}

pub struct CommandPaletteConfig {
  pub branches: Vec<CommandPaletteBranch>,
  pub stashes: Vec<CommandPaletteStash>,
  pub default_stash_message: Option<SharedString>,
  pub repositories: Vec<CommandPaletteRepository>,
  pub commands: Vec<CommandPaletteCommand>,
  pub on_action: CommandPaletteHandler,
}

impl CommandPaletteConfig {
  pub fn new(
    branches: Vec<CommandPaletteBranch>,
    commands: Vec<CommandPaletteCommand>,
    on_action: CommandPaletteHandler,
  ) -> Self {
    Self {
      branches,
      stashes: Vec::new(),
      default_stash_message: None,
      repositories: Vec::new(),
      commands,
      on_action,
    }
  }

  pub fn with_repositories(mut self, repositories: Vec<CommandPaletteRepository>) -> Self {
    self.repositories = repositories;
    self
  }

  pub fn with_stashes(mut self, stashes: Vec<CommandPaletteStash>) -> Self {
    self.stashes = stashes;
    self
  }

  pub fn with_default_stash_message(
    mut self,
    default_stash_message: impl Into<SharedString>,
  ) -> Self {
    self.default_stash_message = Some(default_stash_message.into());
    self
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandPaletteScreen {
  Root,
  SwitchRepository,
  SwitchBranch,
  CreateBranch,
  CreateBranchFrom,
  MergeBranch,
  RebaseBranch,
  CherryPick,
  Stash,
  StashIncludeUntracked,
  ApplyStash,
  DropStash,
  PopStash,
  OpenGithubPrFromUrl,
}

pub struct CommandPalette {
  focus_handle: FocusHandle,
  screen: CommandPaletteScreen,
  commands_list: Entity<ListState<CommandListDelegate>>,
  repositories_list: Entity<ListState<RepositoriesListDelegate>>,
  branches_list: Entity<ListState<BranchesListDelegate>>,
  stashes_list: Entity<ListState<StashesListDelegate>>,
  branches_with_commands_list: Entity<ListState<BranchesListWithCommandsDelegate>>,
  create_branch_input: Entity<InputState>,
  cherry_pick_input: Entity<InputState>,
  stash_input: Entity<InputState>,
  open_github_pr_input: Entity<InputState>,
  default_stash_message: SharedString,
  create_branch_base: Option<Rc<CommandPaletteBranch>>,
  error: Option<SharedString>,
  on_action: Option<CommandPaletteHandler>,
  _subscriptions: Vec<Subscription>,
}

impl CommandPalette {
  fn parse_github_pull_request_url(url: &str) -> Option<(String, String, u64)> {
    let url = url.trim();
    let tail = url
      .strip_prefix("https://github.com/")
      .or_else(|| url.strip_prefix("http://github.com/"))
      .or_else(|| url.strip_prefix("github.com/"))?;

    let mut parts = tail.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() {
      return None;
    }
    if parts.next()? != "pull" {
      return None;
    }

    let number_part = parts.next()?;
    let number_part = number_part
      .split('#')
      .next()
      .unwrap_or(number_part)
      .split('?')
      .next()
      .unwrap_or(number_part);
    let number: u64 = number_part.parse().ok()?;

    Some((owner.to_string(), repo.to_string(), number))
  }

  fn parse_cherry_pick_commit_hashes(value: &str) -> Option<Vec<String>> {
    let commits = value
      .split_whitespace()
      .map(ToString::to_string)
      .collect::<Vec<_>>();

    if commits.is_empty() {
      None
    } else {
      Some(commits)
    }
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>, config: CommandPaletteConfig) -> Self {
    let create_branch_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Enter branch name..."));
    let cherry_pick_input = cx.new(|cx| {
      InputState::new(window, cx)
        .placeholder("Enter one or more commit hashes (space-separated)...")
    });
    let stash_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Enter stash message..."));
    let open_github_pr_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Paste GitHub pull request URL..."));
    let default_stash_message = config.default_stash_message.clone().unwrap_or_default();

    let default_repositories: Vec<Rc<CommandPaletteRepository>> =
      config.repositories.iter().cloned().map(Rc::new).collect();

    let repositories_list_delegate = RepositoriesListDelegate {
      _repositories: default_repositories.clone(),
      matched_repositories: default_repositories.clone(),
      selected_index: None,
      query: "".into(),
    };

    let repositories_list =
      cx.new(|cx| ListState::new(repositories_list_delegate, window, cx).searchable(true));

    let default_branches: Vec<Rc<CommandPaletteBranch>> =
      config.branches.iter().cloned().map(Rc::new).collect();

    let branches_list_delegate = BranchesListDelegate {
      _branches: default_branches.clone(),
      matched_branches: default_branches.clone(),
      selected_index: None,
      query: "".into(),
    };

    let branches_list =
      cx.new(|cx| ListState::new(branches_list_delegate, window, cx).searchable(true));

    let default_stashes: Vec<Rc<CommandPaletteStash>> =
      config.stashes.iter().cloned().map(Rc::new).collect();

    let stashes_list_delegate = StashesListDelegate {
      _stashes: default_stashes.clone(),
      matched_stashes: default_stashes.clone(),
      selected_index: None,
      query: "".into(),
    };

    let stashes_list =
      cx.new(|cx| ListState::new(stashes_list_delegate, window, cx).searchable(true));

    let mut branches_with_actions: Vec<Rc<BranchListWithCommands>> = default_branches
      .into_iter()
      .map(|b| Rc::new(BranchListWithCommands::SwitchBranch(b)))
      .collect();

    branches_with_actions.insert(
      0,
      Rc::new(BranchListWithCommands::CommandPaletteCommand(
        CommandPaletteCommand::create_branch(),
      )),
    );
    branches_with_actions.insert(
      1,
      Rc::new(BranchListWithCommands::CommandPaletteCommand(
        CommandPaletteCommand::create_branch_from(),
      )),
    );

    let branches_list_with_commands_delegate = BranchesListWithCommandsDelegate {
      _branches_with_commands: branches_with_actions.clone(),
      matched_branches_and_commands: branches_with_actions.clone(),
      selected_index: None,
      query: "".into(),
    };

    let branches_with_commands_list = cx
      .new(|cx| ListState::new(branches_list_with_commands_delegate, window, cx).searchable(true));

    let default_commands: Vec<Rc<CommandPaletteCommand>> = config
      .commands
      .iter()
      .map(|command| Rc::new(command.clone()))
      .collect();

    let commands_list_delegate = CommandListDelegate {
      _commands: default_commands.clone(),
      matched_commands: default_commands.clone(),
      selected_index: None,
      query: "".into(),
    };

    let commands_list =
      cx.new(|cx| ListState::new(commands_list_delegate, window, cx).searchable(true));

    let _subscriptions = vec![
      cx.subscribe_in(
        &commands_list,
        window,
        |command_palette, list_state, ev: &ListEvent, window, cx| {
          if let ListEvent::Confirm(ix) = ev
            && let Some(command) = list_state.read(cx).delegate().matched_commands.get(ix.row)
          {
            command_palette.select_command(command.id, cx, window);
          }
        },
      ),
      cx.subscribe_in(
        &repositories_list,
        window,
        |command_palette, list_state, ev: &ListEvent, window, cx| {
          if let ListEvent::Confirm(ix) = ev {
            let repository = {
              let list = list_state.read(cx);
              list.delegate().matched_repositories.get(ix.row).cloned()
            };

            if let Some(repository) = repository {
              command_palette.trigger_action(
                CommandPaletteAction::SwitchRepository((*repository).clone()),
                window,
                cx,
              );
            }
          }
        },
      ),
      cx.subscribe_in(
        &branches_with_commands_list,
        window,
        |command_palette, list_state, ev: &ListEvent, window, cx| {
          if let ListEvent::Confirm(ix) = ev {
            let branch_action = {
              let list = list_state.read(cx);
              list
                .delegate()
                .matched_branches_and_commands
                .get(ix.row)
                .cloned()
            };

            if let Some(branch_action) = branch_action {
              match branch_action.as_ref() {
                BranchListWithCommands::SwitchBranch(branch) => {
                  command_palette.trigger_action(
                    CommandPaletteAction::SwitchBranch((**branch).clone()),
                    window,
                    cx,
                  );
                }
                BranchListWithCommands::CommandPaletteCommand(command) => {
                  command_palette.select_command(command.id, cx, window);
                }
              }
            }
          }
        },
      ),
      cx.subscribe_in(
        &branches_list,
        window,
        |command_palette, list_state, ev: &ListEvent, window, cx| {
          if let ListEvent::Confirm(ix) = ev {
            match command_palette.screen {
              CommandPaletteScreen::MergeBranch | CommandPaletteScreen::RebaseBranch => {
                let branch = {
                  let list = list_state.read(cx);
                  list.delegate().matched_branches.get(ix.row).cloned()
                };

                if let Some(branch) = branch {
                  let action = match command_palette.screen {
                    CommandPaletteScreen::MergeBranch => CommandPaletteAction::MergeBranch {
                      name: (*branch).clone(),
                    },
                    CommandPaletteScreen::RebaseBranch => CommandPaletteAction::RebaseBranch {
                      name: (*branch).clone(),
                    },
                    _ => unreachable!(),
                  };
                  command_palette.trigger_action(action, window, cx);
                }
              }
              CommandPaletteScreen::CreateBranchFrom => {
                let branch = {
                  let list = list_state.read(cx);
                  list.delegate().matched_branches.get(ix.row).cloned()
                };

                command_palette.create_branch_base = branch.clone();

                command_palette.select_command(CommandPaletteCommandId::CreateBranch, cx, window);
              }
              _ => {}
            }
          }
        },
      ),
      cx.subscribe_in(
        &stashes_list,
        window,
        |command_palette, list_state, ev: &ListEvent, window, cx| {
          if let ListEvent::Confirm(ix) = ev {
            let stash = {
              let list = list_state.read(cx);
              list.delegate().matched_stashes.get(ix.row).cloned()
            };

            if let Some(stash) = stash {
              let action = match command_palette.screen {
                CommandPaletteScreen::ApplyStash => {
                  CommandPaletteAction::ApplyStash(stash.as_ref().clone())
                }
                CommandPaletteScreen::DropStash => {
                  CommandPaletteAction::DropStash(stash.as_ref().clone())
                }
                CommandPaletteScreen::PopStash => {
                  CommandPaletteAction::PopStash(stash.as_ref().clone())
                }
                _ => return,
              };
              command_palette.trigger_action(action, window, cx);
            }
          }
        },
      ),
      cx.subscribe_in(&create_branch_input, window, Self::on_input_event),
      cx.subscribe_in(&cherry_pick_input, window, Self::on_cherry_pick_input_event),
      cx.subscribe_in(&stash_input, window, Self::on_stash_input_event),
      cx.subscribe_in(
        &open_github_pr_input,
        window,
        Self::on_open_github_pr_input_event,
      ),
    ];

    cx.on_next_frame(window, |this, window, cx| {
      this.focus_screen_input(window, cx)
    });

    Self {
      focus_handle: cx.focus_handle(),
      create_branch_input,
      cherry_pick_input,
      stash_input,
      default_stash_message,
      create_branch_base: None,
      screen: CommandPaletteScreen::Root,
      commands_list,
      repositories_list,
      branches_list,
      stashes_list,
      branches_with_commands_list,
      open_github_pr_input,
      error: None,
      on_action: Some(config.on_action),
      _subscriptions,
    }
  }

  fn on_input_event(
    &mut self,
    state: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if let InputEvent::PressEnter { secondary: _ } = event {
      let branch_name = state.read(cx).value().to_string();
      if branch_name.is_empty() {
        self.error = Some("Branch name cannot be empty".into());
        cx.notify();
        return;
      }

      if self.screen == CommandPaletteScreen::CreateBranch {
        if let Some(base_branch) = self.create_branch_base.as_ref() {
          let base = base_branch.as_ref().clone();

          self.trigger_action(
            CommandPaletteAction::CreateBranchFrom {
              name: branch_name,
              base,
            },
            window,
            cx,
          );
        } else {
          self.trigger_action(
            CommandPaletteAction::CreateBranch {
              name: branch_name.clone(),
            },
            window,
            cx,
          );
        }
      }
    }
  }

  fn on_open_github_pr_input_event(
    &mut self,
    state: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !matches!(event, InputEvent::PressEnter { .. }) {
      return;
    }

    let url = state.read(cx).value().to_string();
    if url.trim().is_empty() {
      self.error = Some("GitHub pull request URL cannot be empty".into());
      cx.notify();
      return;
    }

    let Some((owner, repo, number)) = Self::parse_github_pull_request_url(&url) else {
      self.error = Some("Invalid GitHub pull request URL".into());
      cx.notify();
      return;
    };

    self.trigger_action(
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
      },
      window,
      cx,
    );
  }

  fn on_cherry_pick_input_event(
    &mut self,
    state: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !matches!(event, InputEvent::PressEnter { .. }) {
      return;
    }

    let input = state.read(cx).value().to_string();
    let Some(commit_hashes) = Self::parse_cherry_pick_commit_hashes(&input) else {
      self.error = Some("Commit hash list cannot be empty".into());
      cx.notify();
      return;
    };

    self.trigger_action(
      CommandPaletteAction::CherryPick { commit_hashes },
      window,
      cx,
    );
  }

  fn on_stash_input_event(
    &mut self,
    state: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !matches!(event, InputEvent::PressEnter { .. }) {
      return;
    }

    let message = state.read(cx).value().trim().to_string();
    let message = if message.is_empty() {
      None
    } else {
      Some(message)
    };

    match self.screen {
      CommandPaletteScreen::Stash => self.trigger_action(
        CommandPaletteAction::Stash {
          include_untracked: false,
          message,
        },
        window,
        cx,
      ),
      CommandPaletteScreen::StashIncludeUntracked => self.trigger_action(
        CommandPaletteAction::Stash {
          include_untracked: true,
          message,
        },
        window,
        cx,
      ),
      _ => {}
    }
  }

  fn prepare_stash_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let default_stash_message = self.default_stash_message.clone();
    self.stash_input.update(cx, move |input, cx| {
      input.set_value(default_stash_message.clone(), window, cx);
    });
  }

  pub fn focus_screen_input(&self, window: &mut Window, cx: &mut Context<Self>) {
    match self.screen {
      CommandPaletteScreen::SwitchRepository => {
        self.repositories_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::SwitchBranch => {
        self.branches_with_commands_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::Root => {
        self.commands_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::CreateBranch => {
        self.create_branch_input.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::CherryPick => {
        self.cherry_pick_input.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::Stash | CommandPaletteScreen::StashIncludeUntracked => {
        self.stash_input.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::ApplyStash
      | CommandPaletteScreen::DropStash
      | CommandPaletteScreen::PopStash => {
        self.stashes_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::OpenGithubPrFromUrl => {
        self.open_github_pr_input.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::CreateBranchFrom => {
        self.branches_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::MergeBranch | CommandPaletteScreen::RebaseBranch => {
        self.branches_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
    }
  }

  fn set_screen(
    &mut self,
    screen: CommandPaletteScreen,
    cx: &mut Context<Self>,
    window: &mut Window,
  ) {
    if self.screen == screen {
      return;
    }

    self.screen = screen;
    self.error = None;
    cx.notify();

    cx.on_next_frame(window, |this, window, cx| {
      this.focus_screen_input(window, cx)
    });
  }

  fn select_command(
    &mut self,
    command: CommandPaletteCommandId,
    cx: &mut Context<Self>,
    window: &mut Window,
  ) {
    match command {
      CommandPaletteCommandId::SwitchRepository => {
        self.set_screen(CommandPaletteScreen::SwitchRepository, cx, window);
      }
      CommandPaletteCommandId::SwitchBranch => {
        self.set_screen(CommandPaletteScreen::SwitchBranch, cx, window);
      }
      CommandPaletteCommandId::Commit => {
        self.trigger_action(CommandPaletteAction::Commit, window, cx);
      }
      CommandPaletteCommandId::ContinueRebase => {
        self.trigger_action(CommandPaletteAction::ContinueRebase, window, cx);
      }
      CommandPaletteCommandId::SkipRebase => {
        self.trigger_action(CommandPaletteAction::SkipRebase, window, cx);
      }
      CommandPaletteCommandId::Push => {
        self.trigger_action(CommandPaletteAction::Push, window, cx);
      }
      CommandPaletteCommandId::ForcePush => {
        self.trigger_action(CommandPaletteAction::ForcePush, window, cx);
      }
      CommandPaletteCommandId::UndoLastCommit => {
        self.trigger_action(CommandPaletteAction::UndoLastCommit, window, cx);
      }
      CommandPaletteCommandId::Amend => {
        self.trigger_action(CommandPaletteAction::Amend, window, cx);
      }
      CommandPaletteCommandId::AcceptAllCurrentConflicts => {
        self.trigger_action(CommandPaletteAction::AcceptAllCurrentConflicts, window, cx);
      }
      CommandPaletteCommandId::AcceptAllIncomingConflicts => {
        self.trigger_action(CommandPaletteAction::AcceptAllIncomingConflicts, window, cx);
      }
      CommandPaletteCommandId::MergeBranch => {
        self.set_screen(CommandPaletteScreen::MergeBranch, cx, window);
      }
      CommandPaletteCommandId::AbortMerge => {
        self.trigger_action(CommandPaletteAction::AbortMerge, window, cx);
      }
      CommandPaletteCommandId::RebaseBranch => {
        self.set_screen(CommandPaletteScreen::RebaseBranch, cx, window);
      }
      CommandPaletteCommandId::AbortRebase => {
        self.trigger_action(CommandPaletteAction::AbortRebase, window, cx);
      }
      CommandPaletteCommandId::CreateBranch => {
        self.set_screen(CommandPaletteScreen::CreateBranch, cx, window);
      }
      CommandPaletteCommandId::CherryPick => {
        self.cherry_pick_input.update(cx, |input, cx| {
          input.set_value("", window, cx);
        });
        self.set_screen(CommandPaletteScreen::CherryPick, cx, window);
      }
      CommandPaletteCommandId::StageAll => {
        self.trigger_action(CommandPaletteAction::StageAll, window, cx);
      }
      CommandPaletteCommandId::UnstageAll => {
        self.trigger_action(CommandPaletteAction::UnstageAll, window, cx);
      }
      CommandPaletteCommandId::Fetch => {
        self.trigger_action(CommandPaletteAction::Fetch, window, cx);
      }
      CommandPaletteCommandId::Stash => {
        self.prepare_stash_input(window, cx);
        self.set_screen(CommandPaletteScreen::Stash, cx, window);
      }
      CommandPaletteCommandId::StashIncludeUntracked => {
        self.prepare_stash_input(window, cx);
        self.set_screen(CommandPaletteScreen::StashIncludeUntracked, cx, window);
      }
      CommandPaletteCommandId::ApplyStash => {
        self.set_screen(CommandPaletteScreen::ApplyStash, cx, window);
      }
      CommandPaletteCommandId::DropStash => {
        self.set_screen(CommandPaletteScreen::DropStash, cx, window);
      }
      CommandPaletteCommandId::PopStash => {
        self.set_screen(CommandPaletteScreen::PopStash, cx, window);
      }
      CommandPaletteCommandId::CreateBranchFrom => {
        self.set_screen(CommandPaletteScreen::CreateBranchFrom, cx, window);
      }
      CommandPaletteCommandId::OpenRepository => {
        self.trigger_action(CommandPaletteAction::OpenRepository, window, cx);
      }
      CommandPaletteCommandId::OpenGitPage => {
        self.trigger_action(CommandPaletteAction::OpenGitPage, window, cx);
      }
      CommandPaletteCommandId::OpenGithubPage => {
        self.trigger_action(CommandPaletteAction::OpenGithubPage, window, cx);
      }
      CommandPaletteCommandId::OpenGithubPrFromUrl => {
        let query = self.commands_list.read(cx).delegate().query.to_string();
        if let Some((owner, repo, number)) = Self::parse_github_pull_request_url(&query) {
          self.trigger_action(
            CommandPaletteAction::OpenGithubPrDetails {
              owner,
              repo,
              number,
            },
            window,
            cx,
          );
        } else {
          self.open_github_pr_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
          });
          self.set_screen(CommandPaletteScreen::OpenGithubPrFromUrl, cx, window);
        }
      }
      CommandPaletteCommandId::OpenGitConfigPage => {
        self.trigger_action(CommandPaletteAction::OpenGitConfigPage, window, cx);
      }
      CommandPaletteCommandId::OpenSettingsPage => {
        self.trigger_action(CommandPaletteAction::OpenSettingsPage, window, cx);
      }
      CommandPaletteCommandId::OpenGitHistorySidebar => {
        self.trigger_action(CommandPaletteAction::OpenGitHistorySidebar, window, cx);
      }
      CommandPaletteCommandId::OpenGitChangesSidebar => {
        self.trigger_action(CommandPaletteAction::OpenGitChangesSidebar, window, cx);
      }
    }
  }

  fn trigger_action(
    &mut self,
    action: CommandPaletteAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(handler) = self.on_action.as_ref() else {
      return;
    };

    match handler(action, window, cx) {
      Ok(()) => window.close_dialog(cx),
      Err(err) => {
        self.error = Some(err);
        cx.notify();
      }
    }
  }

  fn render_search_list<D: ListDelegate>(
    &self,
    list: &Entity<ListState<D>>,
    count: usize,
    placeholder: &'static str,
    cx: &Context<Self>,
  ) -> impl IntoElement {
    List::new(list)
      .w_full()
      .h(px(LIST_ITEM_HEIGHT * count as f32 + LIST_INPUT_HEIGHT))
      .border_1()
      .search_placeholder(placeholder)
      .border_color(cx.theme().border)
      .rounded(cx.theme().radius)
  }

  fn render_root(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let count_commands = self
      .commands_list
      .read(cx)
      .delegate()
      .matched_commands
      .len();

    v_flex()
      .h_full()
      .child(self.render_search_list(
        &self.commands_list,
        count_commands,
        "Search commands...",
        cx,
      ))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }

  fn render_switch_repository(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let count_items = self
      .repositories_list
      .read(cx)
      .delegate()
      .matched_repositories
      .len();

    v_flex()
      .h_full()
      .child(self.render_search_list(
        &self.repositories_list,
        count_items,
        "Search repositories...",
        cx,
      ))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }

  fn render_switch_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let count_items = self
      .branches_with_commands_list
      .read(cx)
      .delegate()
      .matched_branches_and_commands
      .len();

    v_flex()
      .h_full()
      .child(self.render_search_list(
        &self.branches_with_commands_list,
        count_items,
        "Search branches...",
        cx,
      ))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }

  fn render_create_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    v_flex()
      .gap_3()
      .child(Input::new(&self.create_branch_input).border_color(theme.border))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }

  fn render_cherry_pick(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    v_flex()
      .gap_3()
      .child(Input::new(&self.cherry_pick_input).border_color(theme.border))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }

  fn render_stash(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    v_flex()
      .gap_3()
      .child(Input::new(&self.stash_input).border_color(theme.border))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }

  fn render_stash_include_untracked(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_stash(cx)
  }

  fn render_select_stash(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let count_stashes = self.stashes_list.read(cx).delegate().matched_stashes.len();

    v_flex()
      .h_full()
      .child(self.render_search_list(&self.stashes_list, count_stashes, "Search stashes...", cx))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }

  fn render_apply_stash(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_select_stash(cx)
  }

  fn render_drop_stash(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_select_stash(cx)
  }

  fn render_pop_stash(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_select_stash(cx)
  }

  fn render_merge_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let count_branches = self
      .branches_list
      .read(cx)
      .delegate()
      .matched_branches
      .len();

    v_flex()
      .h_full()
      .child(self.render_search_list(
        &self.branches_list,
        count_branches,
        "Search branches...",
        cx,
      ))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }

  fn render_rebase_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_merge_branch(cx)
  }

  fn render_open_github_pr_from_url(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    v_flex()
      .gap_3()
      .child(Input::new(&self.open_github_pr_input).border_color(theme.border))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }

  fn render_create_branch_from(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let count_branches = self
      .branches_list
      .read(cx)
      .delegate()
      .matched_branches
      .len();

    v_flex()
      .h_full()
      .child(self.render_search_list(
        &self.branches_list,
        count_branches,
        "Search branches...",
        cx,
      ))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }

  fn render_error(&self, theme: &gpui_component::Theme, error: &SharedString) -> Div {
    div().text_sm().text_color(theme.red).child(error.clone())
  }
}

impl Focusable for CommandPalette {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for CommandPalette {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let content = match self.screen {
      CommandPaletteScreen::Root => self.render_root(cx).into_any_element(),
      CommandPaletteScreen::SwitchRepository => {
        self.render_switch_repository(cx).into_any_element()
      }
      CommandPaletteScreen::SwitchBranch => self.render_switch_branch(cx).into_any_element(),
      CommandPaletteScreen::CreateBranch => self.render_create_branch(cx).into_any_element(),
      CommandPaletteScreen::CherryPick => self.render_cherry_pick(cx).into_any_element(),
      CommandPaletteScreen::Stash => self.render_stash(cx).into_any_element(),
      CommandPaletteScreen::StashIncludeUntracked => {
        self.render_stash_include_untracked(cx).into_any_element()
      }
      CommandPaletteScreen::ApplyStash => self.render_apply_stash(cx).into_any_element(),
      CommandPaletteScreen::DropStash => self.render_drop_stash(cx).into_any_element(),
      CommandPaletteScreen::PopStash => self.render_pop_stash(cx).into_any_element(),
      CommandPaletteScreen::OpenGithubPrFromUrl => {
        self.render_open_github_pr_from_url(cx).into_any_element()
      }
      CommandPaletteScreen::CreateBranchFrom => {
        self.render_create_branch_from(cx).into_any_element()
      }
      CommandPaletteScreen::MergeBranch => self.render_merge_branch(cx).into_any_element(),
      CommandPaletteScreen::RebaseBranch => self.render_rebase_branch(cx).into_any_element(),
    };

    div()
      .max_h_128()
      .track_focus(&self.focus_handle)
      .child(content)
      .h_full()
      .text_color(theme.foreground)
  }
}

#[cfg(test)]
mod tests {
  use super::{CommandPalette, CommandPaletteCommand, CommandPaletteCommandId};

  #[test]
  fn parse_github_pull_request_url_accepts_standard_url() {
    let parsed =
      CommandPalette::parse_github_pull_request_url("https://github.com/joris-gallot/guit/pull/23");
    assert_eq!(parsed, Some(("joris-gallot".into(), "guit".into(), 23)));
  }

  #[test]
  fn parse_github_pull_request_url_rejects_non_pull_url() {
    let parsed = CommandPalette::parse_github_pull_request_url(
      "https://github.com/joris-gallot/guit/issues/23",
    );
    assert_eq!(parsed, None);
  }

  #[test]
  fn parse_github_pull_request_url_accepts_changes_fragment_url() {
    let parsed = CommandPalette::parse_github_pull_request_url(
      "https://github.com/joris-gallot/guit/pull/4/changes#diff-914ffa9e8939125aa8bba06dbe2ac48755c94e58e2e6c24aa81d52cfafea0709",
    );
    assert_eq!(parsed, Some(("joris-gallot".into(), "guit".into(), 4)));
  }

  #[test]
  fn parse_github_pull_request_url_accepts_query_params() {
    let parsed = CommandPalette::parse_github_pull_request_url(
      "https://github.com/joris-gallot/guit/pull/4?notification_referrer_id=NT_kwDOAAABBBCCC",
    );
    assert_eq!(parsed, Some(("joris-gallot".into(), "guit".into(), 4)));
  }

  #[test]
  fn parse_cherry_pick_commit_hashes_accepts_single_hash() {
    let parsed = CommandPalette::parse_cherry_pick_commit_hashes("abc1234");
    assert_eq!(parsed, Some(vec!["abc1234".to_string()]));
  }

  #[test]
  fn parse_cherry_pick_commit_hashes_accepts_multiple_hashes() {
    let parsed = CommandPalette::parse_cherry_pick_commit_hashes(" abc1234   def5678\t1234abcd ");
    assert_eq!(
      parsed,
      Some(vec![
        "abc1234".to_string(),
        "def5678".to_string(),
        "1234abcd".to_string()
      ])
    );
  }

  #[test]
  fn parse_cherry_pick_commit_hashes_rejects_empty_input() {
    let parsed = CommandPalette::parse_cherry_pick_commit_hashes("   \n\t  ");
    assert_eq!(parsed, None);
  }

  #[test]
  fn open_repository_command_is_available_with_expected_metadata() {
    let command = CommandPaletteCommand::open_repository();
    assert_eq!(command.id, CommandPaletteCommandId::OpenRepository);
    assert_eq!(command.name.as_ref(), "Open repository");
    assert!(command.matches("open repo"));
  }

  #[test]
  fn fetch_command_is_available_with_expected_metadata() {
    let command = CommandPaletteCommand::fetch();
    assert_eq!(command.id, CommandPaletteCommandId::Fetch);
    assert_eq!(command.name.as_ref(), "Fetch");
    assert!(command.matches("fetch updates"));
  }

  #[test]
  fn stage_all_command_is_available_with_expected_metadata() {
    let command = CommandPaletteCommand::stage_all();
    assert_eq!(command.id, CommandPaletteCommandId::StageAll);
    assert_eq!(command.name.as_ref(), "Stage all");
    assert!(command.matches("changed files"));
  }

  #[test]
  fn unstage_all_command_is_available_with_expected_metadata() {
    let command = CommandPaletteCommand::unstage_all();
    assert_eq!(command.id, CommandPaletteCommandId::UnstageAll);
    assert_eq!(command.name.as_ref(), "Unstage all");
    assert!(command.matches("staged files"));
  }

  #[test]
  fn commit_and_rebase_progress_commands_are_available_with_expected_metadata() {
    let commit = CommandPaletteCommand::commit();
    let continue_rebase = CommandPaletteCommand::continue_rebase();
    let skip_rebase = CommandPaletteCommand::skip_rebase();
    let push = CommandPaletteCommand::push("Push");
    let force_push = CommandPaletteCommand::force_push();
    let undo_last_commit = CommandPaletteCommand::undo_last_commit();
    let amend = CommandPaletteCommand::amend();
    let accept_all_current_conflicts = CommandPaletteCommand::accept_all_current_conflicts();
    let accept_all_incoming_conflicts = CommandPaletteCommand::accept_all_incoming_conflicts();

    assert_eq!(commit.id, CommandPaletteCommandId::Commit);
    assert_eq!(commit.name.as_ref(), "Commit");
    assert!(commit.matches("stages all changes"));

    assert_eq!(continue_rebase.id, CommandPaletteCommandId::ContinueRebase);
    assert_eq!(continue_rebase.name.as_ref(), "Rebase continue");
    assert!(continue_rebase.matches("current rebase"));

    assert_eq!(skip_rebase.id, CommandPaletteCommandId::SkipRebase);
    assert_eq!(skip_rebase.name.as_ref(), "Rebase skip");
    assert!(skip_rebase.matches("rebase commit"));

    assert_eq!(push.id, CommandPaletteCommandId::Push);
    assert_eq!(push.name.as_ref(), "Push");
    assert!(push.matches("remote branch"));

    assert_eq!(force_push.id, CommandPaletteCommandId::ForcePush);
    assert_eq!(force_push.name.as_ref(), "Force push (with lease)");
    assert!(force_push.matches("force push local commits"));

    assert_eq!(undo_last_commit.id, CommandPaletteCommandId::UndoLastCommit);
    assert_eq!(undo_last_commit.name.as_ref(), "Undo last commit");
    assert!(undo_last_commit.matches("recent local commit"));

    assert_eq!(amend.id, CommandPaletteCommandId::Amend);
    assert_eq!(amend.name.as_ref(), "Amend");
    assert!(amend.matches("amend the most recent"));

    assert_eq!(
      accept_all_current_conflicts.id,
      CommandPaletteCommandId::AcceptAllCurrentConflicts
    );
    assert_eq!(
      accept_all_current_conflicts.name.as_ref(),
      "Accept all current conflicts"
    );
    assert!(accept_all_current_conflicts.matches("keeping current changes"));

    assert_eq!(
      accept_all_incoming_conflicts.id,
      CommandPaletteCommandId::AcceptAllIncomingConflicts
    );
    assert_eq!(
      accept_all_incoming_conflicts.name.as_ref(),
      "Accept all incoming conflicts"
    );
    assert!(accept_all_incoming_conflicts.matches("incoming changes"));
  }

  #[test]
  fn stash_commands_are_available_with_expected_metadata() {
    let stash = CommandPaletteCommand::stash();
    let stash_untracked = CommandPaletteCommand::stash_with_untracked();
    let apply_stash = CommandPaletteCommand::apply_stash();
    let drop_stash = CommandPaletteCommand::drop_stash();
    let pop_stash = CommandPaletteCommand::pop_stash();

    assert_eq!(stash.id, CommandPaletteCommandId::Stash);
    assert_eq!(
      stash_untracked.id,
      CommandPaletteCommandId::StashIncludeUntracked
    );
    assert_eq!(apply_stash.id, CommandPaletteCommandId::ApplyStash);
    assert_eq!(drop_stash.id, CommandPaletteCommandId::DropStash);
    assert_eq!(pop_stash.id, CommandPaletteCommandId::PopStash);
    assert!(stash.matches("tracked changes"));
    assert!(apply_stash.matches("without dropping"));
  }
}
