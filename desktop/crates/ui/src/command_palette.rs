use std::{collections::BTreeMap, rc::Rc, sync::Arc};

use crate::github_url::parse_github_url_action;
use crate::{SelectableRowStyle, UiIconName, file_icon_path_for_name, selectable_list_item};
use gpui::{
  App, Context, Entity, FocusHandle, Focusable, Global, InteractiveElement, IntoElement,
  ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, IndexPath, Sizable, WindowExt,
  button::{Button, ButtonVariants},
  h_flex,
  input::{Input, InputEvent, InputState},
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  notification::Notification,
  v_flex,
};

const LIST_INPUT_HEIGHT: f32 = 35.0;
const LIST_ITEM_HEIGHT: f32 = 32.0; // Height of each list item in pixels (h_8)
const SECTION_HEADER_HEIGHT: f32 = 28.0;
pub const COMMAND_PALETTE_CONTEXT: &str = "CommandPalette";

pub type CommandPaletteUsageRecorder = fn(CommandPaletteCommandId, &App);

pub struct CommandPaletteUsageRecorderGlobal(pub CommandPaletteUsageRecorder);

impl Global for CommandPaletteUsageRecorderGlobal {}

pub type CommandPaletteUsageScorer = fn(&App, CommandPaletteCommandId, i64) -> f64;

pub struct CommandPaletteUsageScorerGlobal(pub CommandPaletteUsageScorer);

impl Global for CommandPaletteUsageScorerGlobal {}

fn list_base_item(
  ix: IndexPath,
  total_items: usize,
  selected_index: Option<IndexPath>,
  theme: &gpui_component::Theme,
) -> ListItem {
  let is_last_item = ix.row + 1 == total_items;

  selectable_list_item(
    ix,
    Some(ix) == selected_index,
    SelectableRowStyle::Flush,
    theme,
  )
  .h_8()
  .when(is_last_item, |item| item.rounded_b(theme.radius))
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
  GithubRepo,
  GithubPrDetails,
  GitConfig,
  Settings,
  Billing,
  About,
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
  ForgetRepository(CommandPaletteRepository),
  SwitchBranch(CommandPaletteBranch),
  CheckoutDetached {
    target: String,
  },
  Commit,
  ContinueRebase,
  SkipRebase,
  Push,
  ForcePush,
  UndoLastCommit,
  Amend,
  StageSelectedFile,
  UnstageSelectedFile,
  AcceptAllCurrentConflicts,
  AcceptAllIncomingConflicts,
  CreateBranch {
    name: String,
  },
  CreatePullRequest,
  CreateBranchFrom {
    name: String,
    base: CommandPaletteBranch,
  },
  DeleteBranch(CommandPaletteBranch),
  MergeBranch {
    name: CommandPaletteBranch,
  },
  AbortMerge,
  RebaseBranch {
    name: CommandPaletteBranch,
  },
  InteractiveRebaseBranch {
    name: CommandPaletteBranch,
  },
  InteractiveRebaseEditBranch {
    name: CommandPaletteBranch,
  },
  InteractiveRebaseHeadCount {
    count: usize,
  },
  AbortRebase,
  CherryPick {
    commit_hashes: Vec<String>,
  },
  StageAll,
  UnstageAll,
  Pull,
  Fetch,
  Stash {
    include_untracked: bool,
    message: Option<String>,
  },
  ApplyStash(CommandPaletteStash),
  DropStash(CommandPaletteStash),
  PopStash(CommandPaletteStash),
  CreateGithubRepository,
  SearchGithubRepository,
  OpenRepository,
  OpenGitPage,
  OpenGithubPage,
  OpenGithubRepoDetails {
    owner: String,
    repo: String,
    tab: Option<CommandPaletteGithubRepoTab>,
    issue_number: Option<u64>,
    issue_comment_id: Option<u64>,
  },
  OpenGithubProfile {
    login: String,
  },
  OpenGithubPrDetails {
    owner: String,
    repo: String,
    number: u64,
    open_changes_tab: bool,
    review_comment_id: Option<u64>,
  },
  OpenGithubCommitDetails {
    owner: String,
    repo: String,
    sha: String,
  },
  SwitchToPrBranch,
  CopyPrBranch,
  ToggleUnchangedFiles,
  OpenGitHistorySidebar,
  OpenGitChangesSidebar,
  OpenGitConfigPage,
  OpenSettingsPage,
  OpenBillingPage,
  OpenAboutPage,
  SendFeedback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandPaletteGithubRepoTab {
  Overview,
  PullRequests,
  Issues,
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
  recent_commands: Vec<Rc<CommandPaletteCommand>>,
  show_recent: bool,
  matched_sections: Vec<(CommandPaletteGroup, Vec<Rc<CommandPaletteCommand>>)>,
  selected_index: Option<IndexPath>,
  query: SharedString,
}

fn bucketize_commands(
  commands: &[Rc<CommandPaletteCommand>],
) -> Vec<(CommandPaletteGroup, Vec<Rc<CommandPaletteCommand>>)> {
  let mut buckets: BTreeMap<CommandPaletteGroup, Vec<Rc<CommandPaletteCommand>>> = BTreeMap::new();
  for command in commands {
    buckets
      .entry(command.group())
      .or_default()
      .push(command.clone());
  }
  buckets.into_iter().collect()
}

fn build_matched_sections(
  filtered: &[Rc<CommandPaletteCommand>],
  recent: &[Rc<CommandPaletteCommand>],
  query: &str,
  show_recent: bool,
) -> Vec<(CommandPaletteGroup, Vec<Rc<CommandPaletteCommand>>)> {
  let mut sections = bucketize_commands(filtered);
  if show_recent && query.is_empty() && !recent.is_empty() {
    sections.insert(0, (CommandPaletteGroup::Recent, recent.to_vec()));
  }
  sections
}

fn compute_recent_commands(
  commands: &[Rc<CommandPaletteCommand>],
  cx: &App,
  top_n: usize,
) -> Vec<Rc<CommandPaletteCommand>> {
  let Some(scorer) = cx
    .try_global::<CommandPaletteUsageScorerGlobal>()
    .map(|g| g.0)
  else {
    return Vec::new();
  };
  let now_secs = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0);

  let mut scored: Vec<(Rc<CommandPaletteCommand>, f64)> = commands
    .iter()
    .map(|c| {
      let score = scorer(cx, c.id, now_secs);
      (c.clone(), score)
    })
    .filter(|(_, s)| *s > 0.0)
    .collect();
  scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
  scored.into_iter().take(top_n).map(|(c, _)| c).collect()
}

impl CommandListDelegate {
  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();
    let filtered: Vec<Rc<CommandPaletteCommand>> = self
      ._commands
      .iter()
      .filter(|c| c.matches(&self.query))
      .cloned()
      .collect();
    self.matched_sections = build_matched_sections(
      &filtered,
      &self.recent_commands,
      self.query.as_ref(),
      self.show_recent,
    );
  }

  fn matched_total_count(&self) -> usize {
    self.matched_sections.iter().map(|(_, v)| v.len()).sum()
  }

  fn visible_sections_count(&self) -> usize {
    self.matched_sections.len()
  }

  fn item_at(&self, ix: IndexPath) -> Option<Rc<CommandPaletteCommand>> {
    self
      .matched_sections
      .get(ix.section)
      .and_then(|(_, items)| items.get(ix.row).cloned())
  }
}

impl ListDelegate for CommandListDelegate {
  type Item = ListItem;

  fn sections_count(&self, _cx: &App) -> usize {
    self.matched_sections.len()
  }

  fn items_count(&self, section: usize, _cx: &App) -> usize {
    self
      .matched_sections
      .get(section)
      .map(|(_, items)| items.len())
      .unwrap_or(0)
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let total_in_section = self
      .matched_sections
      .get(ix.section)
      .map(|(_, items)| items.len())
      .unwrap_or(0);
    let theme = cx.theme().clone();

    self.item_at(ix).map(|command| {
      list_base_item(ix, total_in_section, self.selected_index, &theme)
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

  fn render_section_header(
    &mut self,
    section: usize,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<impl IntoElement> {
    if self.matched_sections.len() <= 1 {
      return None;
    }
    let (group, _) = self.matched_sections.get(section)?;
    Some(
      h_flex()
        .px_3()
        .pt_2()
        .pb_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(group.label()),
    )
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
pub enum CommandPaletteInitialScreen {
  Root,
  SwitchBranch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandPaletteCommandId {
  SwitchRepository,
  ForgetRepository,
  SwitchBranch,
  CheckoutDetached,
  Commit,
  ContinueRebase,
  SkipRebase,
  Push,
  ForcePush,
  UndoLastCommit,
  Amend,
  StageSelectedFile,
  UnstageSelectedFile,
  AcceptAllCurrentConflicts,
  AcceptAllIncomingConflicts,
  CreateBranch,
  CreateBranchFrom,
  DeleteBranch,
  MergeBranch,
  AbortMerge,
  RebaseBranch,
  InteractiveRebase,
  InteractiveRebaseOntoBranch,
  InteractiveRebaseEditBranch,
  InteractiveRebaseHeadCount,
  AbortRebase,
  CreatePullRequest,
  CherryPick,
  StageAll,
  UnstageAll,
  Pull,
  Fetch,
  Stash,
  StashIncludeUntracked,
  ApplyStash,
  DropStash,
  PopStash,
  CreateGithubRepository,
  SearchGithubRepository,
  OpenRepository,
  OpenGitPage,
  OpenGithubPage,
  OpenGithubFromUrl,
  SwitchToPrBranch,
  CopyPrBranch,
  ToggleUnchangedFiles,
  OpenGitHistorySidebar,
  OpenGitChangesSidebar,
  OpenGitConfigPage,
  OpenSettingsPage,
  OpenBillingPage,
  OpenAboutPage,
  SendFeedback,
}

impl CommandPaletteCommandId {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::SwitchRepository => "switch_repository",
      Self::ForgetRepository => "forget_repository",
      Self::SwitchBranch => "switch_branch",
      Self::CheckoutDetached => "checkout_detached",
      Self::Commit => "commit",
      Self::ContinueRebase => "continue_rebase",
      Self::SkipRebase => "skip_rebase",
      Self::Push => "push",
      Self::ForcePush => "force_push",
      Self::UndoLastCommit => "undo_last_commit",
      Self::Amend => "amend",
      Self::StageSelectedFile => "stage_selected_file",
      Self::UnstageSelectedFile => "unstage_selected_file",
      Self::AcceptAllCurrentConflicts => "accept_all_current_conflicts",
      Self::AcceptAllIncomingConflicts => "accept_all_incoming_conflicts",
      Self::CreateBranch => "create_branch",
      Self::CreateBranchFrom => "create_branch_from",
      Self::DeleteBranch => "delete_branch",
      Self::MergeBranch => "merge_branch",
      Self::AbortMerge => "abort_merge",
      Self::RebaseBranch => "rebase_branch",
      Self::InteractiveRebase => "interactive_rebase",
      Self::InteractiveRebaseOntoBranch => "interactive_rebase_onto_branch",
      Self::InteractiveRebaseEditBranch => "interactive_rebase_edit_branch",
      Self::InteractiveRebaseHeadCount => "interactive_rebase_head_count",
      Self::AbortRebase => "abort_rebase",
      Self::CreatePullRequest => "create_pull_request",
      Self::CherryPick => "cherry_pick",
      Self::StageAll => "stage_all",
      Self::UnstageAll => "unstage_all",
      Self::Pull => "pull",
      Self::Fetch => "fetch",
      Self::Stash => "stash",
      Self::StashIncludeUntracked => "stash_include_untracked",
      Self::ApplyStash => "apply_stash",
      Self::DropStash => "drop_stash",
      Self::PopStash => "pop_stash",
      Self::CreateGithubRepository => "create_github_repository",
      Self::SearchGithubRepository => "search_github_repository",
      Self::OpenRepository => "open_repository",
      Self::OpenGitPage => "open_git_page",
      Self::OpenGithubPage => "open_github_page",
      Self::OpenGithubFromUrl => "open_github_from_url",
      Self::SwitchToPrBranch => "switch_to_pr_branch",
      Self::CopyPrBranch => "copy_pr_branch",
      Self::ToggleUnchangedFiles => "toggle_unchanged_files",
      Self::OpenGitHistorySidebar => "open_git_history_sidebar",
      Self::OpenGitChangesSidebar => "open_git_changes_sidebar",
      Self::OpenGitConfigPage => "open_git_config_page",
      Self::OpenSettingsPage => "open_settings_page",
      Self::OpenBillingPage => "open_billing_page",
      Self::OpenAboutPage => "open_about_page",
      Self::SendFeedback => "send_feedback",
    }
  }

  pub fn from_str(value: &str) -> Option<Self> {
    match value {
      "switch_repository" => Some(Self::SwitchRepository),
      "forget_repository" => Some(Self::ForgetRepository),
      "switch_branch" => Some(Self::SwitchBranch),
      "checkout_detached" => Some(Self::CheckoutDetached),
      "commit" => Some(Self::Commit),
      "continue_rebase" => Some(Self::ContinueRebase),
      "skip_rebase" => Some(Self::SkipRebase),
      "push" => Some(Self::Push),
      "force_push" => Some(Self::ForcePush),
      "undo_last_commit" => Some(Self::UndoLastCommit),
      "amend" => Some(Self::Amend),
      "stage_selected_file" => Some(Self::StageSelectedFile),
      "unstage_selected_file" => Some(Self::UnstageSelectedFile),
      "accept_all_current_conflicts" => Some(Self::AcceptAllCurrentConflicts),
      "accept_all_incoming_conflicts" => Some(Self::AcceptAllIncomingConflicts),
      "create_branch" => Some(Self::CreateBranch),
      "create_branch_from" => Some(Self::CreateBranchFrom),
      "delete_branch" => Some(Self::DeleteBranch),
      "merge_branch" => Some(Self::MergeBranch),
      "abort_merge" => Some(Self::AbortMerge),
      "rebase_branch" => Some(Self::RebaseBranch),
      "interactive_rebase" => Some(Self::InteractiveRebase),
      "interactive_rebase_onto_branch" => Some(Self::InteractiveRebaseOntoBranch),
      "interactive_rebase_edit_branch" => Some(Self::InteractiveRebaseEditBranch),
      "interactive_rebase_head_count" => Some(Self::InteractiveRebaseHeadCount),
      "abort_rebase" => Some(Self::AbortRebase),
      "create_pull_request" => Some(Self::CreatePullRequest),
      "cherry_pick" => Some(Self::CherryPick),
      "stage_all" => Some(Self::StageAll),
      "unstage_all" => Some(Self::UnstageAll),
      "pull" => Some(Self::Pull),
      "fetch" => Some(Self::Fetch),
      "stash" => Some(Self::Stash),
      "stash_include_untracked" => Some(Self::StashIncludeUntracked),
      "apply_stash" => Some(Self::ApplyStash),
      "drop_stash" => Some(Self::DropStash),
      "pop_stash" => Some(Self::PopStash),
      "create_github_repository" => Some(Self::CreateGithubRepository),
      "search_github_repository" => Some(Self::SearchGithubRepository),
      "open_repository" => Some(Self::OpenRepository),
      "open_git_page" => Some(Self::OpenGitPage),
      "open_github_page" => Some(Self::OpenGithubPage),
      "open_github_from_url" => Some(Self::OpenGithubFromUrl),
      "switch_to_pr_branch" => Some(Self::SwitchToPrBranch),
      "copy_pr_branch" => Some(Self::CopyPrBranch),
      "toggle_unchanged_files" => Some(Self::ToggleUnchangedFiles),
      "open_git_history_sidebar" => Some(Self::OpenGitHistorySidebar),
      "open_git_changes_sidebar" => Some(Self::OpenGitChangesSidebar),
      "open_git_config_page" => Some(Self::OpenGitConfigPage),
      "open_settings_page" => Some(Self::OpenSettingsPage),
      "open_billing_page" => Some(Self::OpenBillingPage),
      "open_about_page" => Some(Self::OpenAboutPage),
      "send_feedback" => Some(Self::SendFeedback),
      _ => None,
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CommandPaletteGroup {
  Recent,
  Changes,
  Sync,
  Branches,
  RebaseMergeProgress,
  Stash,
  PullRequest,
  Repository,
  Github,
  Navigation,
  Feedback,
}

impl CommandPaletteGroup {
  pub fn label(&self) -> &'static str {
    match self {
      Self::Recent => "Recent",
      Self::Changes => "Changes",
      Self::Sync => "Sync",
      Self::Branches => "Branches",
      Self::RebaseMergeProgress => "In progress",
      Self::Stash => "Stash",
      Self::PullRequest => "Pull request",
      Self::Repository => "Repository",
      Self::Github => "GitHub",
      Self::Navigation => "Navigation",
      Self::Feedback => "Feedback",
    }
  }
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
      name: "Switch repository".into(),
      description: Some("Switch to another recent repository".into()),
    }
  }

  pub fn forget_repository() -> Self {
    Self {
      id: CommandPaletteCommandId::ForgetRepository,
      name: "Forget repository".into(),
      description: Some("Remove a repository from the recent list".into()),
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

  pub fn checkout_detached() -> Self {
    Self {
      id: CommandPaletteCommandId::CheckoutDetached,
      name: "Git checkout detached".into(),
      description: Some("Detach HEAD at a commit hash or tag".into()),
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

  pub fn stage_selected_file() -> Self {
    Self {
      id: CommandPaletteCommandId::StageSelectedFile,
      name: "Stage file".into(),
      description: Some("Stage the selected file".into()),
    }
  }

  pub fn unstage_selected_file() -> Self {
    Self {
      id: CommandPaletteCommandId::UnstageSelectedFile,
      name: "Unstage file".into(),
      description: Some("Unstage the selected file".into()),
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

  pub fn interactive_rebase() -> Self {
    Self {
      id: CommandPaletteCommandId::InteractiveRebase,
      name: "Rebase interactive".into(),
      description: Some("Interactively edit and reorder commits before rebasing".into()),
    }
  }

  pub fn interactive_rebase_onto_branch() -> Self {
    Self {
      id: CommandPaletteCommandId::InteractiveRebaseOntoBranch,
      name: "Onto branch".into(),
      description: Some("Start interactive rebase onto another branch".into()),
    }
  }

  pub fn interactive_rebase_edit_branch() -> Self {
    Self {
      id: CommandPaletteCommandId::InteractiveRebaseEditBranch,
      name: "Edit branch commits".into(),
      description: Some(
        "Reorder, squash, or edit commits without incorporating upstream changes".into(),
      ),
    }
  }

  pub fn interactive_rebase_head_count() -> Self {
    Self {
      id: CommandPaletteCommandId::InteractiveRebaseHeadCount,
      name: "Last N commits (HEAD~n)".into(),
      description: Some("Start interactive rebase for the last N commits".into()),
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

  pub fn create_pull_request() -> Self {
    Self {
      id: CommandPaletteCommandId::CreatePullRequest,
      name: "Create pull request".into(),
      description: Some("Create a pull request for the current branch".into()),
    }
  }

  pub fn create_branch_from() -> Self {
    Self {
      id: CommandPaletteCommandId::CreateBranchFrom,
      name: "Create branch from...".into(),
      description: Some("Create a new branch from an existing branch".into()),
    }
  }

  pub fn delete_branch() -> Self {
    Self {
      id: CommandPaletteCommandId::DeleteBranch,
      name: "Delete branch".into(),
      description: Some("Force delete a local branch, or delete a remote branch".into()),
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

  pub fn pull() -> Self {
    Self {
      id: CommandPaletteCommandId::Pull,
      name: "Pull".into(),
      description: Some("Pull changes from the remote branch".into()),
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
      name: "Go to GitHub".into(),
      description: Some("Navigate to the GitHub page".into()),
    }
  }

  pub fn create_github_repository() -> Self {
    Self {
      id: CommandPaletteCommandId::CreateGithubRepository,
      name: "Create GitHub repository".into(),
      description: Some("Create a new repository under your account or an organization".into()),
    }
  }

  pub fn search_github_repository() -> Self {
    Self {
      id: CommandPaletteCommandId::SearchGithubRepository,
      name: "Search GitHub repository".into(),
      description: Some("Find a repository on GitHub by name or owner".into()),
    }
  }

  pub fn open_git_page() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenGitPage,
      name: "Go to Git".into(),
      description: Some("Navigate to the Git page".into()),
    }
  }

  pub fn open_github_from_url() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenGithubFromUrl,
      name: "Open from GitHub URL".into(),
      description: Some("Open a supported GitHub page from a GitHub URL".into()),
    }
  }

  pub fn switch_to_pr_branch() -> Self {
    Self {
      id: CommandPaletteCommandId::SwitchToPrBranch,
      name: "Switch to PR branch".into(),
      description: Some("Switch the local repository to the current pull request branch".into()),
    }
  }

  pub fn copy_pr_branch() -> Self {
    Self {
      id: CommandPaletteCommandId::CopyPrBranch,
      name: "Copy PR branch name".into(),
      description: Some("Copy the source branch name of the current pull request".into()),
    }
  }

  pub fn toggle_unchanged_files(currently_shown: bool) -> Self {
    if currently_shown {
      Self {
        id: CommandPaletteCommandId::ToggleUnchangedFiles,
        name: "Hide unchanged files".into(),
        description: Some("Show only files changed in this pull request".into()),
      }
    } else {
      Self {
        id: CommandPaletteCommandId::ToggleUnchangedFiles,
        name: "Show unchanged files".into(),
        description: Some("Show all project files alongside changed files".into()),
      }
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
      name: "Go to Settings".into(),
      description: Some("Navigate to Settings".into()),
    }
  }

  pub fn open_billing_page() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenBillingPage,
      name: "Go to Billing".into(),
      description: Some("Navigate to Billing".into()),
    }
  }

  pub fn open_about_page() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenAboutPage,
      name: "Go to About".into(),
      description: Some("Navigate to About".into()),
    }
  }

  pub fn send_feedback() -> Self {
    Self {
      id: CommandPaletteCommandId::SendFeedback,
      name: "Send Feedback".into(),
      description: Some("Report a bug or suggest a feature".into()),
    }
  }

  pub fn open_git_config_page() -> Self {
    Self {
      id: CommandPaletteCommandId::OpenGitConfigPage,
      name: "Go to Git Config".into(),
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
      commands.push(Self::open_github_from_url());
      commands.push(Self::search_github_repository());
      commands.push(Self::create_github_repository());
    }

    if current_page != CommandPalettePage::GitConfig {
      commands.push(Self::open_git_config_page());
    }

    if current_page != CommandPalettePage::Settings {
      commands.push(Self::open_settings_page());
    }

    if current_page != CommandPalettePage::Billing {
      commands.push(Self::open_billing_page());
    }

    if current_page != CommandPalettePage::About {
      commands.push(Self::open_about_page());
    }

    if current_page == CommandPalettePage::Git {
      commands.push(Self::open_git_history_sidebar());
      commands.push(Self::open_git_changes_sidebar());
    }

    commands.push(Self::send_feedback());

    commands
  }

  pub fn group(&self) -> CommandPaletteGroup {
    match self.id {
      CommandPaletteCommandId::Commit
      | CommandPaletteCommandId::Amend
      | CommandPaletteCommandId::UndoLastCommit
      | CommandPaletteCommandId::StageSelectedFile
      | CommandPaletteCommandId::UnstageSelectedFile
      | CommandPaletteCommandId::StageAll
      | CommandPaletteCommandId::UnstageAll
      | CommandPaletteCommandId::AcceptAllCurrentConflicts
      | CommandPaletteCommandId::AcceptAllIncomingConflicts
      | CommandPaletteCommandId::CherryPick
      | CommandPaletteCommandId::CheckoutDetached => CommandPaletteGroup::Changes,

      CommandPaletteCommandId::Pull
      | CommandPaletteCommandId::Fetch
      | CommandPaletteCommandId::Push
      | CommandPaletteCommandId::ForcePush => CommandPaletteGroup::Sync,

      CommandPaletteCommandId::SwitchBranch
      | CommandPaletteCommandId::CreateBranch
      | CommandPaletteCommandId::CreateBranchFrom
      | CommandPaletteCommandId::DeleteBranch
      | CommandPaletteCommandId::MergeBranch
      | CommandPaletteCommandId::RebaseBranch
      | CommandPaletteCommandId::InteractiveRebase
      | CommandPaletteCommandId::InteractiveRebaseOntoBranch
      | CommandPaletteCommandId::InteractiveRebaseEditBranch
      | CommandPaletteCommandId::InteractiveRebaseHeadCount => CommandPaletteGroup::Branches,

      CommandPaletteCommandId::ContinueRebase
      | CommandPaletteCommandId::SkipRebase
      | CommandPaletteCommandId::AbortRebase
      | CommandPaletteCommandId::AbortMerge => CommandPaletteGroup::RebaseMergeProgress,

      CommandPaletteCommandId::Stash
      | CommandPaletteCommandId::StashIncludeUntracked
      | CommandPaletteCommandId::ApplyStash
      | CommandPaletteCommandId::DropStash
      | CommandPaletteCommandId::PopStash => CommandPaletteGroup::Stash,

      CommandPaletteCommandId::CreatePullRequest
      | CommandPaletteCommandId::SwitchToPrBranch
      | CommandPaletteCommandId::CopyPrBranch
      | CommandPaletteCommandId::ToggleUnchangedFiles => CommandPaletteGroup::PullRequest,

      CommandPaletteCommandId::SwitchRepository
      | CommandPaletteCommandId::ForgetRepository
      | CommandPaletteCommandId::OpenRepository => CommandPaletteGroup::Repository,

      CommandPaletteCommandId::SearchGithubRepository
      | CommandPaletteCommandId::CreateGithubRepository
      | CommandPaletteCommandId::OpenGithubFromUrl
      | CommandPaletteCommandId::OpenGithubPage => CommandPaletteGroup::Github,

      CommandPaletteCommandId::OpenGitPage
      | CommandPaletteCommandId::OpenGitConfigPage
      | CommandPaletteCommandId::OpenSettingsPage
      | CommandPaletteCommandId::OpenBillingPage
      | CommandPaletteCommandId::OpenAboutPage
      | CommandPaletteCommandId::OpenGitHistorySidebar
      | CommandPaletteCommandId::OpenGitChangesSidebar => CommandPaletteGroup::Navigation,

      CommandPaletteCommandId::SendFeedback => CommandPaletteGroup::Feedback,
    }
  }

  fn icon(&self) -> Icon {
    match self.id {
      CommandPaletteCommandId::SwitchRepository => Icon::new(IconName::FolderOpen),
      CommandPaletteCommandId::ForgetRepository => Icon::new(UiIconName::Trash),
      CommandPaletteCommandId::SwitchBranch => Icon::new(UiIconName::GitBranch),
      CommandPaletteCommandId::CheckoutDetached => Icon::new(UiIconName::GitBranch),
      CommandPaletteCommandId::Commit => Icon::new(IconName::Check),
      CommandPaletteCommandId::ContinueRebase => Icon::new(IconName::Check),
      CommandPaletteCommandId::SkipRebase => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::Push => Icon::new(IconName::ArrowUp),
      CommandPaletteCommandId::ForcePush => Icon::new(IconName::ArrowUp),
      CommandPaletteCommandId::UndoLastCommit => Icon::new(IconName::Undo),
      CommandPaletteCommandId::Amend => Icon::new(IconName::Replace),
      CommandPaletteCommandId::StageSelectedFile => Icon::new(IconName::Plus),
      CommandPaletteCommandId::UnstageSelectedFile => Icon::new(IconName::Minus),
      CommandPaletteCommandId::AcceptAllCurrentConflicts => Icon::new(IconName::Replace),
      CommandPaletteCommandId::AcceptAllIncomingConflicts => Icon::new(IconName::Replace),
      CommandPaletteCommandId::MergeBranch => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::AbortMerge => Icon::new(IconName::Undo),
      CommandPaletteCommandId::RebaseBranch => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::InteractiveRebase => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::InteractiveRebaseOntoBranch => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::InteractiveRebaseEditBranch => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::InteractiveRebaseHeadCount => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::AbortRebase => Icon::new(IconName::Undo),
      CommandPaletteCommandId::CherryPick => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::StageAll => Icon::new(IconName::Plus),
      CommandPaletteCommandId::UnstageAll => Icon::new(UiIconName::ArrowUpFromLine),
      CommandPaletteCommandId::Pull => Icon::new(IconName::ArrowDown),
      CommandPaletteCommandId::Fetch => Icon::new(UiIconName::RefreshCw),
      CommandPaletteCommandId::Stash | CommandPaletteCommandId::StashIncludeUntracked => {
        Icon::new(UiIconName::ArrowDownFromLine)
      }
      CommandPaletteCommandId::ApplyStash | CommandPaletteCommandId::PopStash => {
        Icon::new(UiIconName::ArrowUpFromLine)
      }
      CommandPaletteCommandId::DropStash => Icon::new(UiIconName::Trash),
      CommandPaletteCommandId::DeleteBranch => Icon::new(UiIconName::Trash),
      CommandPaletteCommandId::CreateGithubRepository => Icon::new(IconName::Plus),
      CommandPaletteCommandId::SearchGithubRepository => Icon::new(IconName::Search),
      CommandPaletteCommandId::OpenRepository => Icon::new(IconName::FolderOpen),
      CommandPaletteCommandId::CreateBranch | CommandPaletteCommandId::CreateBranchFrom => {
        Icon::new(IconName::Plus)
      }
      CommandPaletteCommandId::CreatePullRequest => Icon::new(UiIconName::GitPullRequestArrow),
      CommandPaletteCommandId::OpenGitPage => Icon::new(UiIconName::GitBranch),
      CommandPaletteCommandId::OpenGithubPage => Icon::new(IconName::Github),
      CommandPaletteCommandId::OpenGithubFromUrl => Icon::new(IconName::Github),
      CommandPaletteCommandId::SwitchToPrBranch => Icon::new(UiIconName::GitBranch),
      CommandPaletteCommandId::CopyPrBranch => Icon::new(IconName::Copy),
      CommandPaletteCommandId::ToggleUnchangedFiles => Icon::new(UiIconName::ScanEye),
      CommandPaletteCommandId::OpenGitHistorySidebar => Icon::new(UiIconName::History),
      CommandPaletteCommandId::OpenGitChangesSidebar => Icon::new(UiIconName::FileCode),
      CommandPaletteCommandId::OpenGitConfigPage => Self::git_config_icon(),
      CommandPaletteCommandId::OpenSettingsPage => Icon::new(IconName::Settings2),
      CommandPaletteCommandId::OpenBillingPage => Icon::new(UiIconName::CreditCard),
      CommandPaletteCommandId::OpenAboutPage => Icon::new(UiIconName::Info),
      CommandPaletteCommandId::SendFeedback => Icon::new(UiIconName::MessageCircle),
    }
  }

  fn matches(&self, query: &str) -> bool {
    if self.id == CommandPaletteCommandId::OpenGithubFromUrl
      && parse_github_url_action(query).is_some()
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
  pub delete_branches: Vec<CommandPaletteBranch>,
  pub stashes: Vec<CommandPaletteStash>,
  pub default_stash_message: Option<SharedString>,
  pub repositories: Vec<CommandPaletteRepository>,
  pub commands: Vec<CommandPaletteCommand>,
  pub initial_screen: CommandPaletteInitialScreen,
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
      delete_branches: Vec::new(),
      stashes: Vec::new(),
      default_stash_message: None,
      repositories: Vec::new(),
      commands,
      initial_screen: CommandPaletteInitialScreen::Root,
      on_action,
    }
  }

  pub fn with_repositories(mut self, repositories: Vec<CommandPaletteRepository>) -> Self {
    self.repositories = repositories;
    self
  }

  pub fn with_delete_branches(mut self, delete_branches: Vec<CommandPaletteBranch>) -> Self {
    self.delete_branches = delete_branches;
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

  pub fn with_initial_screen(mut self, initial_screen: CommandPaletteInitialScreen) -> Self {
    self.initial_screen = initial_screen;
    self
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandPaletteScreen {
  Root,
  SwitchRepository,
  ForgetRepository,
  SwitchBranch,
  CheckoutDetached,
  CreateBranch,
  CreateBranchFrom,
  DeleteBranch,
  MergeBranch,
  RebaseBranch,
  InteractiveRebaseMode,
  InteractiveRebaseBranch,
  InteractiveRebaseEditBranch,
  InteractiveRebaseHeadCount,
  CherryPick,
  Stash,
  StashIncludeUntracked,
  ApplyStash,
  DropStash,
  PopStash,
  OpenGithubFromUrl,
}

impl From<CommandPaletteInitialScreen> for CommandPaletteScreen {
  fn from(value: CommandPaletteInitialScreen) -> Self {
    match value {
      CommandPaletteInitialScreen::Root => CommandPaletteScreen::Root,
      CommandPaletteInitialScreen::SwitchBranch => CommandPaletteScreen::SwitchBranch,
    }
  }
}

pub struct CommandPalette {
  focus_handle: FocusHandle,
  screen: CommandPaletteScreen,
  commands_list: Entity<ListState<CommandListDelegate>>,
  interactive_rebase_mode_list: Entity<ListState<CommandListDelegate>>,
  repositories_list: Entity<ListState<RepositoriesListDelegate>>,
  branches_list: Entity<ListState<BranchesListDelegate>>,
  delete_branches_list: Entity<ListState<BranchesListDelegate>>,
  stashes_list: Entity<ListState<StashesListDelegate>>,
  branches_with_commands_list: Entity<ListState<BranchesListWithCommandsDelegate>>,
  create_branch_input: Entity<InputState>,
  checkout_detached_input: Entity<InputState>,
  cherry_pick_input: Entity<InputState>,
  interactive_rebase_head_count_input: Entity<InputState>,
  stash_input: Entity<InputState>,
  open_github_url_input: Entity<InputState>,
  default_stash_message: SharedString,
  create_branch_base: Option<Rc<CommandPaletteBranch>>,
  on_action: Option<CommandPaletteHandler>,
  _subscriptions: Vec<Subscription>,
}

impl CommandPalette {
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

  fn parse_checkout_detached_target(value: &str) -> Option<String> {
    let target = value.trim();
    if target.is_empty() {
      None
    } else {
      Some(target.to_string())
    }
  }

  fn parse_interactive_rebase_head_count(value: &str) -> Option<usize> {
    let count: usize = value.trim().parse().ok()?;
    if count < 2 {
      return None;
    }
    Some(count)
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>, config: CommandPaletteConfig) -> Self {
    let create_branch_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Enter branch name..."));
    let checkout_detached_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Enter commit hash or tag..."));
    let cherry_pick_input = cx.new(|cx| {
      InputState::new(window, cx)
        .placeholder("Enter one or more commit hashes (space-separated)...")
    });
    let interactive_rebase_head_count_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Enter commit count (n >= 2)..."));
    let stash_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Enter stash message..."));
    let open_github_url_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Paste GitHub URL..."));
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

    let default_delete_branches: Vec<Rc<CommandPaletteBranch>> = config
      .delete_branches
      .iter()
      .cloned()
      .map(Rc::new)
      .collect();

    let delete_branches_list_delegate = BranchesListDelegate {
      _branches: default_delete_branches.clone(),
      matched_branches: default_delete_branches.clone(),
      selected_index: None,
      query: "".into(),
    };

    let delete_branches_list =
      cx.new(|cx| ListState::new(delete_branches_list_delegate, window, cx).searchable(true));

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

    let recent_commands = compute_recent_commands(&default_commands, cx, 5);
    let initial_commands_sections =
      build_matched_sections(&default_commands, &recent_commands, "", true);

    let commands_list_delegate = CommandListDelegate {
      matched_sections: initial_commands_sections,
      _commands: default_commands.clone(),
      recent_commands,
      show_recent: true,
      selected_index: None,
      query: "".into(),
    };

    let commands_list =
      cx.new(|cx| ListState::new(commands_list_delegate, window, cx).searchable(true));

    let interactive_rebase_mode_commands = vec![
      CommandPaletteCommand::interactive_rebase_edit_branch(),
      CommandPaletteCommand::interactive_rebase_onto_branch(),
      CommandPaletteCommand::interactive_rebase_head_count(),
    ];
    let interactive_rebase_mode_commands = interactive_rebase_mode_commands
      .into_iter()
      .map(Rc::new)
      .collect::<Vec<_>>();
    let interactive_rebase_mode_list_delegate = CommandListDelegate {
      matched_sections: bucketize_commands(&interactive_rebase_mode_commands),
      _commands: interactive_rebase_mode_commands.clone(),
      recent_commands: Vec::new(),
      show_recent: false,
      selected_index: None,
      query: "".into(),
    };
    let interactive_rebase_mode_list = cx
      .new(|cx| ListState::new(interactive_rebase_mode_list_delegate, window, cx).searchable(true));

    let _subscriptions = vec![
      cx.subscribe_in(
        &commands_list,
        window,
        |command_palette, list_state, ev: &ListEvent, window, cx| {
          if let ListEvent::Confirm(ix) = ev
            && let Some(command) = list_state.read(cx).delegate().item_at(*ix)
          {
            command_palette.select_command(command.id, cx, window);
          }
        },
      ),
      cx.subscribe_in(
        &interactive_rebase_mode_list,
        window,
        |command_palette, list_state, ev: &ListEvent, window, cx| {
          if let ListEvent::Confirm(ix) = ev
            && let Some(command) = list_state.read(cx).delegate().item_at(*ix)
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
              let (id, action) = match command_palette.screen {
                CommandPaletteScreen::ForgetRepository => (
                  CommandPaletteCommandId::ForgetRepository,
                  CommandPaletteAction::ForgetRepository((*repository).clone()),
                ),
                _ => (
                  CommandPaletteCommandId::SwitchRepository,
                  CommandPaletteAction::SwitchRepository((*repository).clone()),
                ),
              };
              command_palette.trigger_action(id, action, window, cx);
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
                    CommandPaletteCommandId::SwitchBranch,
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
              CommandPaletteScreen::MergeBranch
              | CommandPaletteScreen::RebaseBranch
              | CommandPaletteScreen::InteractiveRebaseBranch
              | CommandPaletteScreen::InteractiveRebaseEditBranch => {
                let branch = {
                  let list = list_state.read(cx);
                  list.delegate().matched_branches.get(ix.row).cloned()
                };

                if let Some(branch) = branch {
                  let (id, action) = match command_palette.screen {
                    CommandPaletteScreen::MergeBranch => (
                      CommandPaletteCommandId::MergeBranch,
                      CommandPaletteAction::MergeBranch {
                        name: (*branch).clone(),
                      },
                    ),
                    CommandPaletteScreen::RebaseBranch => (
                      CommandPaletteCommandId::RebaseBranch,
                      CommandPaletteAction::RebaseBranch {
                        name: (*branch).clone(),
                      },
                    ),
                    CommandPaletteScreen::InteractiveRebaseBranch => (
                      CommandPaletteCommandId::InteractiveRebaseOntoBranch,
                      CommandPaletteAction::InteractiveRebaseBranch {
                        name: (*branch).clone(),
                      },
                    ),
                    CommandPaletteScreen::InteractiveRebaseEditBranch => (
                      CommandPaletteCommandId::InteractiveRebaseEditBranch,
                      CommandPaletteAction::InteractiveRebaseEditBranch {
                        name: (*branch).clone(),
                      },
                    ),
                    _ => unreachable!(),
                  };
                  command_palette.trigger_action(id, action, window, cx);
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
        &delete_branches_list,
        window,
        |command_palette, list_state, ev: &ListEvent, window, cx| {
          if let ListEvent::Confirm(ix) = ev
            && command_palette.screen == CommandPaletteScreen::DeleteBranch
          {
            let branch = {
              let list = list_state.read(cx);
              list.delegate().matched_branches.get(ix.row).cloned()
            };

            if let Some(branch) = branch {
              command_palette.trigger_action(
                CommandPaletteCommandId::DeleteBranch,
                CommandPaletteAction::DeleteBranch((*branch).clone()),
                window,
                cx,
              );
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
              let (id, action) = match command_palette.screen {
                CommandPaletteScreen::ApplyStash => (
                  CommandPaletteCommandId::ApplyStash,
                  CommandPaletteAction::ApplyStash(stash.as_ref().clone()),
                ),
                CommandPaletteScreen::DropStash => (
                  CommandPaletteCommandId::DropStash,
                  CommandPaletteAction::DropStash(stash.as_ref().clone()),
                ),
                CommandPaletteScreen::PopStash => (
                  CommandPaletteCommandId::PopStash,
                  CommandPaletteAction::PopStash(stash.as_ref().clone()),
                ),
                _ => return,
              };
              command_palette.trigger_action(id, action, window, cx);
            }
          }
        },
      ),
      cx.subscribe_in(&create_branch_input, window, Self::on_input_event),
      cx.subscribe_in(
        &checkout_detached_input,
        window,
        Self::on_checkout_detached_input_event,
      ),
      cx.subscribe_in(&cherry_pick_input, window, Self::on_cherry_pick_input_event),
      cx.subscribe_in(
        &interactive_rebase_head_count_input,
        window,
        Self::on_interactive_rebase_head_count_input_event,
      ),
      cx.subscribe_in(&stash_input, window, Self::on_stash_input_event),
      cx.subscribe_in(
        &open_github_url_input,
        window,
        Self::on_open_github_url_input_event,
      ),
    ];

    cx.on_next_frame(window, |this, window, cx| {
      this.focus_screen_input(window, cx)
    });

    Self {
      focus_handle: cx.focus_handle(),
      create_branch_input,
      checkout_detached_input,
      cherry_pick_input,
      stash_input,
      default_stash_message,
      create_branch_base: None,
      screen: config.initial_screen.into(),
      commands_list,
      interactive_rebase_mode_list,
      repositories_list,
      branches_list,
      delete_branches_list,
      stashes_list,
      branches_with_commands_list,
      open_github_url_input,
      interactive_rebase_head_count_input,
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
        return;
      }

      if self.screen == CommandPaletteScreen::CreateBranch {
        if let Some(base_branch) = self.create_branch_base.as_ref() {
          let base = base_branch.as_ref().clone();

          self.trigger_action(
            CommandPaletteCommandId::CreateBranchFrom,
            CommandPaletteAction::CreateBranchFrom {
              name: branch_name,
              base,
            },
            window,
            cx,
          );
        } else {
          self.trigger_action(
            CommandPaletteCommandId::CreateBranch,
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

  fn on_open_github_url_input_event(
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
      return;
    }

    let Some(action) = parse_github_url_action(&url) else {
      window.push_notification(Notification::error("Invalid GitHub URL"), cx);
      return;
    };

    self.trigger_action(
      CommandPaletteCommandId::OpenGithubFromUrl,
      action,
      window,
      cx,
    );
  }

  fn on_checkout_detached_input_event(
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
    let Some(target) = Self::parse_checkout_detached_target(&input) else {
      return;
    };

    self.trigger_action(
      CommandPaletteCommandId::CheckoutDetached,
      CommandPaletteAction::CheckoutDetached { target },
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
      return;
    };

    self.trigger_action(
      CommandPaletteCommandId::CherryPick,
      CommandPaletteAction::CherryPick { commit_hashes },
      window,
      cx,
    );
  }

  fn on_interactive_rebase_head_count_input_event(
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
    let Some(count) = Self::parse_interactive_rebase_head_count(&input) else {
      window.push_notification(
        Notification::error("Commit count must be an integer >= 2"),
        cx,
      );
      return;
    };

    self.trigger_action(
      CommandPaletteCommandId::InteractiveRebaseHeadCount,
      CommandPaletteAction::InteractiveRebaseHeadCount { count },
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
        CommandPaletteCommandId::Stash,
        CommandPaletteAction::Stash {
          include_untracked: false,
          message,
        },
        window,
        cx,
      ),
      CommandPaletteScreen::StashIncludeUntracked => self.trigger_action(
        CommandPaletteCommandId::StashIncludeUntracked,
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
      CommandPaletteScreen::SwitchRepository | CommandPaletteScreen::ForgetRepository => {
        self.repositories_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::InteractiveRebaseMode => {
        self.interactive_rebase_mode_list.update(cx, |state, cx| {
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
      CommandPaletteScreen::DeleteBranch => {
        self.delete_branches_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::CheckoutDetached => {
        self.checkout_detached_input.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::CherryPick => {
        self.cherry_pick_input.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::InteractiveRebaseHeadCount => {
        self
          .interactive_rebase_head_count_input
          .update(cx, |state, cx| {
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
      CommandPaletteScreen::OpenGithubFromUrl => {
        self.open_github_url_input.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::CreateBranchFrom => {
        self.branches_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::MergeBranch
      | CommandPaletteScreen::RebaseBranch
      | CommandPaletteScreen::InteractiveRebaseBranch
      | CommandPaletteScreen::InteractiveRebaseEditBranch => {
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
      CommandPaletteCommandId::ForgetRepository => {
        self.set_screen(CommandPaletteScreen::ForgetRepository, cx, window);
      }
      CommandPaletteCommandId::SwitchBranch => {
        self.set_screen(CommandPaletteScreen::SwitchBranch, cx, window);
      }
      CommandPaletteCommandId::CheckoutDetached => {
        self.checkout_detached_input.update(cx, |input, cx| {
          input.set_value("", window, cx);
        });
        self.set_screen(CommandPaletteScreen::CheckoutDetached, cx, window);
      }
      CommandPaletteCommandId::Commit => {
        self.trigger_action(command, CommandPaletteAction::Commit, window, cx);
      }
      CommandPaletteCommandId::ContinueRebase => {
        self.trigger_action(command, CommandPaletteAction::ContinueRebase, window, cx);
      }
      CommandPaletteCommandId::SkipRebase => {
        self.trigger_action(command, CommandPaletteAction::SkipRebase, window, cx);
      }
      CommandPaletteCommandId::Push => {
        self.trigger_action(command, CommandPaletteAction::Push, window, cx);
      }
      CommandPaletteCommandId::ForcePush => {
        self.trigger_action(command, CommandPaletteAction::ForcePush, window, cx);
      }
      CommandPaletteCommandId::UndoLastCommit => {
        self.trigger_action(command, CommandPaletteAction::UndoLastCommit, window, cx);
      }
      CommandPaletteCommandId::Amend => {
        self.trigger_action(command, CommandPaletteAction::Amend, window, cx);
      }
      CommandPaletteCommandId::StageSelectedFile => {
        self.trigger_action(command, CommandPaletteAction::StageSelectedFile, window, cx);
      }
      CommandPaletteCommandId::UnstageSelectedFile => {
        self.trigger_action(
          command,
          CommandPaletteAction::UnstageSelectedFile,
          window,
          cx,
        );
      }
      CommandPaletteCommandId::AcceptAllCurrentConflicts => {
        self.trigger_action(
          command,
          CommandPaletteAction::AcceptAllCurrentConflicts,
          window,
          cx,
        );
      }
      CommandPaletteCommandId::AcceptAllIncomingConflicts => {
        self.trigger_action(
          command,
          CommandPaletteAction::AcceptAllIncomingConflicts,
          window,
          cx,
        );
      }
      CommandPaletteCommandId::MergeBranch => {
        self.set_screen(CommandPaletteScreen::MergeBranch, cx, window);
      }
      CommandPaletteCommandId::AbortMerge => {
        self.trigger_action(command, CommandPaletteAction::AbortMerge, window, cx);
      }
      CommandPaletteCommandId::RebaseBranch => {
        self.set_screen(CommandPaletteScreen::RebaseBranch, cx, window);
      }
      CommandPaletteCommandId::InteractiveRebase => {
        self.set_screen(CommandPaletteScreen::InteractiveRebaseMode, cx, window);
      }
      CommandPaletteCommandId::InteractiveRebaseOntoBranch => {
        self.set_screen(CommandPaletteScreen::InteractiveRebaseBranch, cx, window);
      }
      CommandPaletteCommandId::InteractiveRebaseEditBranch => {
        self.set_screen(
          CommandPaletteScreen::InteractiveRebaseEditBranch,
          cx,
          window,
        );
      }
      CommandPaletteCommandId::InteractiveRebaseHeadCount => {
        self
          .interactive_rebase_head_count_input
          .update(cx, |input, cx| {
            input.set_value("", window, cx);
          });
        self.set_screen(CommandPaletteScreen::InteractiveRebaseHeadCount, cx, window);
      }
      CommandPaletteCommandId::AbortRebase => {
        self.trigger_action(command, CommandPaletteAction::AbortRebase, window, cx);
      }
      CommandPaletteCommandId::CreateBranch => {
        self.set_screen(CommandPaletteScreen::CreateBranch, cx, window);
      }
      CommandPaletteCommandId::DeleteBranch => {
        self.set_screen(CommandPaletteScreen::DeleteBranch, cx, window);
      }
      CommandPaletteCommandId::CreatePullRequest => {
        self.trigger_action(command, CommandPaletteAction::CreatePullRequest, window, cx);
      }
      CommandPaletteCommandId::CherryPick => {
        self.cherry_pick_input.update(cx, |input, cx| {
          input.set_value("", window, cx);
        });
        self.set_screen(CommandPaletteScreen::CherryPick, cx, window);
      }
      CommandPaletteCommandId::StageAll => {
        self.trigger_action(command, CommandPaletteAction::StageAll, window, cx);
      }
      CommandPaletteCommandId::UnstageAll => {
        self.trigger_action(command, CommandPaletteAction::UnstageAll, window, cx);
      }
      CommandPaletteCommandId::Pull => {
        self.trigger_action(command, CommandPaletteAction::Pull, window, cx);
      }
      CommandPaletteCommandId::Fetch => {
        self.trigger_action(command, CommandPaletteAction::Fetch, window, cx);
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
        self.trigger_action(command, CommandPaletteAction::OpenRepository, window, cx);
      }
      CommandPaletteCommandId::OpenGitPage => {
        self.trigger_action(command, CommandPaletteAction::OpenGitPage, window, cx);
      }
      CommandPaletteCommandId::OpenGithubPage => {
        self.trigger_action(command, CommandPaletteAction::OpenGithubPage, window, cx);
      }
      CommandPaletteCommandId::CreateGithubRepository => {
        self.trigger_action(
          command,
          CommandPaletteAction::CreateGithubRepository,
          window,
          cx,
        );
      }
      CommandPaletteCommandId::SearchGithubRepository => {
        self.trigger_action(
          command,
          CommandPaletteAction::SearchGithubRepository,
          window,
          cx,
        );
      }
      CommandPaletteCommandId::OpenGithubFromUrl => {
        let query = self.commands_list.read(cx).delegate().query.to_string();
        if let Some(action) = parse_github_url_action(&query) {
          self.trigger_action(command, action, window, cx);
        } else {
          self.open_github_url_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
          });
          self.set_screen(CommandPaletteScreen::OpenGithubFromUrl, cx, window);
        }
      }
      CommandPaletteCommandId::SwitchToPrBranch => {
        self.trigger_action(command, CommandPaletteAction::SwitchToPrBranch, window, cx);
      }
      CommandPaletteCommandId::CopyPrBranch => {
        self.trigger_action(command, CommandPaletteAction::CopyPrBranch, window, cx);
      }
      CommandPaletteCommandId::ToggleUnchangedFiles => {
        self.trigger_action(
          command,
          CommandPaletteAction::ToggleUnchangedFiles,
          window,
          cx,
        );
      }
      CommandPaletteCommandId::OpenGitConfigPage => {
        self.trigger_action(command, CommandPaletteAction::OpenGitConfigPage, window, cx);
      }
      CommandPaletteCommandId::OpenSettingsPage => {
        self.trigger_action(command, CommandPaletteAction::OpenSettingsPage, window, cx);
      }
      CommandPaletteCommandId::OpenBillingPage => {
        self.trigger_action(command, CommandPaletteAction::OpenBillingPage, window, cx);
      }
      CommandPaletteCommandId::OpenAboutPage => {
        self.trigger_action(command, CommandPaletteAction::OpenAboutPage, window, cx);
      }
      CommandPaletteCommandId::SendFeedback => {
        self.trigger_action(command, CommandPaletteAction::SendFeedback, window, cx);
      }
      CommandPaletteCommandId::OpenGitHistorySidebar => {
        self.trigger_action(
          command,
          CommandPaletteAction::OpenGitHistorySidebar,
          window,
          cx,
        );
      }
      CommandPaletteCommandId::OpenGitChangesSidebar => {
        self.trigger_action(
          command,
          CommandPaletteAction::OpenGitChangesSidebar,
          window,
          cx,
        );
      }
    }
  }

  fn trigger_action(
    &mut self,
    id: CommandPaletteCommandId,
    action: CommandPaletteAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(handler) = self.on_action.as_ref() else {
      return;
    };

    match handler(action, window, cx) {
      Ok(()) => {
        let recorder = cx
          .try_global::<CommandPaletteUsageRecorderGlobal>()
          .map(|g| g.0);
        if let Some(recorder) = recorder {
          recorder(id, cx);
        }
        window.close_dialog(cx);
      }
      Err(err) => {
        window.push_notification(Notification::error(err), cx);
        window.close_dialog(cx);
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
    self.render_search_list_with_sections(list, count, 0, placeholder, cx)
  }

  fn render_search_list_with_sections<D: ListDelegate>(
    &self,
    list: &Entity<ListState<D>>,
    item_count: usize,
    visible_headers: usize,
    placeholder: &'static str,
    cx: &Context<Self>,
  ) -> impl IntoElement {
    let height_px = LIST_ITEM_HEIGHT * item_count as f32
      + SECTION_HEADER_HEIGHT * visible_headers as f32
      + LIST_INPUT_HEIGHT;
    List::new(list)
      .w_full()
      .h(px(height_px))
      .border_1()
      .search_placeholder(placeholder)
      .border_color(cx.theme().border)
      .rounded(cx.theme().radius)
  }

  fn render_root(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let commands_delegate = self.commands_list.read(cx).delegate();
    let count_commands = commands_delegate.matched_total_count();
    let visible_headers = {
      let sections = commands_delegate.visible_sections_count();
      if sections > 1 { sections } else { 0 }
    };

    v_flex()
      .h_full()
      .child(self.render_search_list_with_sections(
        &self.commands_list,
        count_commands,
        visible_headers,
        "Search commands...",
        cx,
      ))
  }

  fn render_switch_repository(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let count_items = self
      .repositories_list
      .read(cx)
      .delegate()
      .matched_repositories
      .len();

    v_flex().h_full().child(self.render_search_list(
      &self.repositories_list,
      count_items,
      "Search repositories...",
      cx,
    ))
  }

  fn render_forget_repository(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let count_items = self
      .repositories_list
      .read(cx)
      .delegate()
      .matched_repositories
      .len();

    v_flex().h_full().child(self.render_search_list(
      &self.repositories_list,
      count_items,
      "Select repository to forget...",
      cx,
    ))
  }

  fn render_switch_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let count_items = self
      .branches_with_commands_list
      .read(cx)
      .delegate()
      .matched_branches_and_commands
      .len();

    v_flex().h_full().child(self.render_search_list(
      &self.branches_with_commands_list,
      count_items,
      "Search branches...",
      cx,
    ))
  }

  fn render_create_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    v_flex()
      .gap_3()
      .child(Input::new(&self.create_branch_input).border_color(cx.theme().border))
  }

  fn render_checkout_detached(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    v_flex()
      .gap_3()
      .child(Input::new(&self.checkout_detached_input).border_color(cx.theme().border))
  }

  fn render_cherry_pick(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    v_flex()
      .gap_3()
      .child(Input::new(&self.cherry_pick_input).border_color(cx.theme().border))
  }

  fn render_stash(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    v_flex()
      .gap_3()
      .child(Input::new(&self.stash_input).border_color(cx.theme().border))
  }

  fn render_stash_include_untracked(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_stash(cx)
  }

  fn render_select_stash(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let count_stashes = self.stashes_list.read(cx).delegate().matched_stashes.len();

    v_flex().h_full().child(self.render_search_list(
      &self.stashes_list,
      count_stashes,
      "Search stashes...",
      cx,
    ))
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

  fn render_delete_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let count_branches = self
      .delete_branches_list
      .read(cx)
      .delegate()
      .matched_branches
      .len();

    v_flex().h_full().child(self.render_search_list(
      &self.delete_branches_list,
      count_branches,
      "Select branch to delete...",
      cx,
    ))
  }

  fn render_merge_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let count_branches = self
      .branches_list
      .read(cx)
      .delegate()
      .matched_branches
      .len();

    v_flex().h_full().child(self.render_search_list(
      &self.branches_list,
      count_branches,
      "Search branches...",
      cx,
    ))
  }

  fn render_interactive_rebase_mode(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let count_modes = self
      .interactive_rebase_mode_list
      .read(cx)
      .delegate()
      .matched_total_count();

    v_flex().h_full().child(self.render_search_list(
      &self.interactive_rebase_mode_list,
      count_modes,
      "Select interactive rebase mode...",
      cx,
    ))
  }

  fn render_interactive_rebase_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_merge_branch(cx)
  }

  fn render_interactive_rebase_head_count(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    v_flex()
      .gap_3()
      .child(Input::new(&self.interactive_rebase_head_count_input).border_color(cx.theme().border))
  }

  fn render_rebase_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_merge_branch(cx)
  }

  fn render_open_github_from_url(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    v_flex()
      .gap_3()
      .child(Input::new(&self.open_github_url_input).border_color(cx.theme().border))
  }

  fn render_create_branch_from(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let count_branches = self
      .branches_list
      .read(cx)
      .delegate()
      .matched_branches
      .len();

    v_flex().h_full().child(self.render_search_list(
      &self.branches_list,
      count_branches,
      "Search branches...",
      cx,
    ))
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
      CommandPaletteScreen::ForgetRepository => {
        self.render_forget_repository(cx).into_any_element()
      }
      CommandPaletteScreen::SwitchBranch => self.render_switch_branch(cx).into_any_element(),
      CommandPaletteScreen::CheckoutDetached => {
        self.render_checkout_detached(cx).into_any_element()
      }
      CommandPaletteScreen::CreateBranch => self.render_create_branch(cx).into_any_element(),
      CommandPaletteScreen::DeleteBranch => self.render_delete_branch(cx).into_any_element(),
      CommandPaletteScreen::CherryPick => self.render_cherry_pick(cx).into_any_element(),
      CommandPaletteScreen::Stash => self.render_stash(cx).into_any_element(),
      CommandPaletteScreen::StashIncludeUntracked => {
        self.render_stash_include_untracked(cx).into_any_element()
      }
      CommandPaletteScreen::ApplyStash => self.render_apply_stash(cx).into_any_element(),
      CommandPaletteScreen::DropStash => self.render_drop_stash(cx).into_any_element(),
      CommandPaletteScreen::PopStash => self.render_pop_stash(cx).into_any_element(),
      CommandPaletteScreen::OpenGithubFromUrl => {
        self.render_open_github_from_url(cx).into_any_element()
      }
      CommandPaletteScreen::CreateBranchFrom => {
        self.render_create_branch_from(cx).into_any_element()
      }
      CommandPaletteScreen::MergeBranch => self.render_merge_branch(cx).into_any_element(),
      CommandPaletteScreen::RebaseBranch => self.render_rebase_branch(cx).into_any_element(),
      CommandPaletteScreen::InteractiveRebaseMode => {
        self.render_interactive_rebase_mode(cx).into_any_element()
      }
      CommandPaletteScreen::InteractiveRebaseBranch
      | CommandPaletteScreen::InteractiveRebaseEditBranch => {
        self.render_interactive_rebase_branch(cx).into_any_element()
      }
      CommandPaletteScreen::InteractiveRebaseHeadCount => self
        .render_interactive_rebase_head_count(cx)
        .into_any_element(),
    };

    div()
      .max_h_128()
      .key_context(COMMAND_PALETTE_CONTEXT)
      .track_focus(&self.focus_handle)
      .child(content)
      .h_full()
      .text_color(theme.foreground)
  }
}

#[cfg(test)]
mod tests {
  use super::{
    CommandPalette, CommandPaletteCommand, CommandPaletteCommandId, CommandPaletteConfig,
    CommandPaletteGroup, CommandPaletteHandler, CommandPaletteInitialScreen,
  };
  use std::rc::Rc;
  use std::sync::Arc;

  #[test]
  fn open_github_from_url_command_matches_pull_and_repo_urls() {
    let command = CommandPaletteCommand::open_github_from_url();

    assert!(command.matches("https://github.com/joris-gallot/guit"));
    assert!(command.matches("https://github.com/joris-gallot/guit/pull/4"));
    assert!(command.matches("https://github.com/joris-gallot/guit/pulls?q=is%3Apr"));
    assert!(command.matches("https://github.com/joris-gallot/guit/issues?q=is%3Aissue"));
    assert!(command.matches("https://github.com/joris-gallot/guit/issues/23"));
    assert!(!command.matches("https://gitlab.com/acme/widget"));
  }

  #[test]
  fn command_palette_config_can_start_on_branch_switcher() {
    let handler: CommandPaletteHandler = Arc::new(|_, _, _| Ok(()));
    let config = CommandPaletteConfig::new(Vec::new(), Vec::new(), handler)
      .with_initial_screen(CommandPaletteInitialScreen::SwitchBranch);

    assert_eq!(
      config.initial_screen,
      CommandPaletteInitialScreen::SwitchBranch
    );
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
  fn parse_checkout_detached_target_trims_and_rejects_empty() {
    assert_eq!(
      CommandPalette::parse_checkout_detached_target("  v1.2.3 "),
      Some("v1.2.3".to_string())
    );
    assert_eq!(CommandPalette::parse_checkout_detached_target("   "), None);
  }

  #[test]
  fn parse_interactive_rebase_head_count_accepts_integer_greater_than_one() {
    let parsed = CommandPalette::parse_interactive_rebase_head_count(" 5 ");
    assert_eq!(parsed, Some(5));
  }

  #[test]
  fn parse_interactive_rebase_head_count_rejects_non_integer_or_small_value() {
    assert_eq!(
      CommandPalette::parse_interactive_rebase_head_count("abc"),
      None
    );
    assert_eq!(
      CommandPalette::parse_interactive_rebase_head_count("1"),
      None
    );
    assert_eq!(
      CommandPalette::parse_interactive_rebase_head_count("0"),
      None
    );
  }

  #[test]
  fn open_repository_command_is_available_with_expected_metadata() {
    let command = CommandPaletteCommand::open_repository();
    assert_eq!(command.id, CommandPaletteCommandId::OpenRepository);
    assert_eq!(command.name.as_ref(), "Open repository");
    assert!(command.matches("open repo"));
  }

  #[test]
  fn switch_repository_command_is_available_with_expected_metadata() {
    let command = CommandPaletteCommand::switch_repository();
    assert_eq!(command.id, CommandPaletteCommandId::SwitchRepository);
    assert_eq!(command.name.as_ref(), "Switch repository");
    assert!(command.matches("recent repository"));
  }

  #[test]
  fn forget_repository_command_is_available_with_expected_metadata() {
    use super::CommandPaletteGroup;
    let command = CommandPaletteCommand::forget_repository();
    assert_eq!(command.id, CommandPaletteCommandId::ForgetRepository);
    assert_eq!(command.name.as_ref(), "Forget repository");
    assert!(command.matches("forget"));
    assert!(command.matches("recent list"));
    assert_eq!(command.group(), CommandPaletteGroup::Repository);
  }

  #[test]
  fn search_github_repository_command_is_available_with_expected_metadata() {
    let command = CommandPaletteCommand::search_github_repository();
    assert_eq!(command.id, CommandPaletteCommandId::SearchGithubRepository);
    assert_eq!(command.name.as_ref(), "Search GitHub repository");
    assert!(command.matches("search github"));
    assert!(command.matches("repository on github"));
  }

  #[test]
  fn commands_are_bucketed_into_the_expected_groups() {
    use super::CommandPaletteGroup;

    assert_eq!(
      CommandPaletteCommand::commit().group(),
      CommandPaletteGroup::Changes
    );
    assert_eq!(
      CommandPaletteCommand::push("Push").group(),
      CommandPaletteGroup::Sync
    );
    assert_eq!(
      CommandPaletteCommand::switch_branch().group(),
      CommandPaletteGroup::Branches
    );
    assert_eq!(
      CommandPaletteCommand::continue_rebase().group(),
      CommandPaletteGroup::RebaseMergeProgress
    );
    assert_eq!(
      CommandPaletteCommand::stash().group(),
      CommandPaletteGroup::Stash
    );
    assert_eq!(
      CommandPaletteCommand::create_pull_request().group(),
      CommandPaletteGroup::PullRequest
    );
    assert_eq!(
      CommandPaletteCommand::switch_repository().group(),
      CommandPaletteGroup::Repository
    );
    assert_eq!(
      CommandPaletteCommand::search_github_repository().group(),
      CommandPaletteGroup::Github
    );
    assert_eq!(
      CommandPaletteCommand::open_settings_page().group(),
      CommandPaletteGroup::Navigation
    );
    assert_eq!(
      CommandPaletteCommand::send_feedback().group(),
      CommandPaletteGroup::Feedback
    );
  }

  #[test]
  fn command_palette_group_order_is_stable_and_total() {
    use super::CommandPaletteGroup;
    // Ord is derived from declaration order — make the contract explicit.
    let mut groups = [
      CommandPaletteGroup::Feedback,
      CommandPaletteGroup::Navigation,
      CommandPaletteGroup::Github,
      CommandPaletteGroup::Repository,
      CommandPaletteGroup::PullRequest,
      CommandPaletteGroup::Stash,
      CommandPaletteGroup::RebaseMergeProgress,
      CommandPaletteGroup::Branches,
      CommandPaletteGroup::Sync,
      CommandPaletteGroup::Changes,
    ];
    groups.sort();
    assert_eq!(
      groups,
      [
        CommandPaletteGroup::Changes,
        CommandPaletteGroup::Sync,
        CommandPaletteGroup::Branches,
        CommandPaletteGroup::RebaseMergeProgress,
        CommandPaletteGroup::Stash,
        CommandPaletteGroup::PullRequest,
        CommandPaletteGroup::Repository,
        CommandPaletteGroup::Github,
        CommandPaletteGroup::Navigation,
        CommandPaletteGroup::Feedback,
      ]
    );
  }

  #[test]
  fn default_global_commands_include_search_github_repository_when_github_is_enabled() {
    let commands = CommandPaletteCommand::default_global_commands(
      super::CommandPalettePage::Git,
      /* include_github */ true,
    );
    assert!(
      commands
        .iter()
        .any(|c| c.id == CommandPaletteCommandId::SearchGithubRepository)
    );
  }

  #[test]
  fn default_global_commands_omit_search_github_repository_when_github_is_disabled() {
    let commands = CommandPaletteCommand::default_global_commands(
      super::CommandPalettePage::Git,
      /* include_github */ false,
    );
    assert!(
      !commands
        .iter()
        .any(|c| c.id == CommandPaletteCommandId::SearchGithubRepository)
    );
  }

  #[test]
  fn switch_to_pr_branch_command_is_available_with_expected_metadata() {
    let command = CommandPaletteCommand::switch_to_pr_branch();
    assert_eq!(command.id, CommandPaletteCommandId::SwitchToPrBranch);
    assert_eq!(command.name.as_ref(), "Switch to PR branch");
    assert!(command.matches("current pull request branch"));
  }

  #[test]
  fn create_pull_request_command_is_available_with_expected_metadata() {
    let command = CommandPaletteCommand::create_pull_request();
    assert_eq!(command.id, CommandPaletteCommandId::CreatePullRequest);
    assert_eq!(command.name.as_ref(), "Create pull request");
    assert!(command.matches("current branch"));
  }

  #[test]
  fn delete_branch_command_is_available_with_expected_metadata() {
    let command = CommandPaletteCommand::delete_branch();
    assert_eq!(command.id, CommandPaletteCommandId::DeleteBranch);
    assert_eq!(command.name.as_ref(), "Delete branch");
    assert!(command.matches("remote branch"));
  }

  #[test]
  fn billing_and_about_commands_are_available_with_expected_metadata() {
    let billing = CommandPaletteCommand::open_billing_page();
    let about = CommandPaletteCommand::open_about_page();

    assert_eq!(billing.id, CommandPaletteCommandId::OpenBillingPage);
    assert_eq!(billing.name.as_ref(), "Go to Billing");
    assert!(billing.matches("billing"));

    assert_eq!(about.id, CommandPaletteCommandId::OpenAboutPage);
    assert_eq!(about.name.as_ref(), "Go to About");
    assert!(about.matches("about"));
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
    let checkout_detached = CommandPaletteCommand::checkout_detached();
    let commit = CommandPaletteCommand::commit();
    let continue_rebase = CommandPaletteCommand::continue_rebase();
    let skip_rebase = CommandPaletteCommand::skip_rebase();
    let interactive_rebase = CommandPaletteCommand::interactive_rebase();
    let interactive_rebase_onto_branch = CommandPaletteCommand::interactive_rebase_onto_branch();
    let interactive_rebase_edit_branch = CommandPaletteCommand::interactive_rebase_edit_branch();
    let interactive_rebase_head_count = CommandPaletteCommand::interactive_rebase_head_count();
    let push = CommandPaletteCommand::push("Push");
    let force_push = CommandPaletteCommand::force_push();
    let undo_last_commit = CommandPaletteCommand::undo_last_commit();
    let amend = CommandPaletteCommand::amend();
    let stage_selected_file = CommandPaletteCommand::stage_selected_file();
    let unstage_selected_file = CommandPaletteCommand::unstage_selected_file();
    let accept_all_current_conflicts = CommandPaletteCommand::accept_all_current_conflicts();
    let accept_all_incoming_conflicts = CommandPaletteCommand::accept_all_incoming_conflicts();

    assert_eq!(
      checkout_detached.id,
      CommandPaletteCommandId::CheckoutDetached
    );
    assert_eq!(checkout_detached.name.as_ref(), "Git checkout detached");
    assert!(checkout_detached.matches("commit hash"));

    assert_eq!(commit.id, CommandPaletteCommandId::Commit);
    assert_eq!(commit.name.as_ref(), "Commit");
    assert!(commit.matches("stages all changes"));

    assert_eq!(continue_rebase.id, CommandPaletteCommandId::ContinueRebase);
    assert_eq!(continue_rebase.name.as_ref(), "Rebase continue");
    assert!(continue_rebase.matches("current rebase"));

    assert_eq!(skip_rebase.id, CommandPaletteCommandId::SkipRebase);
    assert_eq!(skip_rebase.name.as_ref(), "Rebase skip");
    assert!(skip_rebase.matches("rebase commit"));

    assert_eq!(
      interactive_rebase.id,
      CommandPaletteCommandId::InteractiveRebase
    );
    assert_eq!(interactive_rebase.name.as_ref(), "Rebase interactive");
    assert!(interactive_rebase.matches("reorder commits"));

    assert_eq!(
      interactive_rebase_onto_branch.id,
      CommandPaletteCommandId::InteractiveRebaseOntoBranch
    );
    assert_eq!(interactive_rebase_onto_branch.name.as_ref(), "Onto branch");
    assert!(interactive_rebase_onto_branch.matches("another branch"));

    assert_eq!(
      interactive_rebase_edit_branch.id,
      CommandPaletteCommandId::InteractiveRebaseEditBranch
    );
    assert_eq!(
      interactive_rebase_edit_branch.name.as_ref(),
      "Edit branch commits"
    );
    assert!(interactive_rebase_edit_branch.matches("without incorporating upstream"));

    assert_eq!(
      interactive_rebase_head_count.id,
      CommandPaletteCommandId::InteractiveRebaseHeadCount
    );
    assert_eq!(
      interactive_rebase_head_count.name.as_ref(),
      "Last N commits (HEAD~n)"
    );
    assert!(interactive_rebase_head_count.matches("last n commits"));

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
      stage_selected_file.id,
      CommandPaletteCommandId::StageSelectedFile
    );
    assert_eq!(stage_selected_file.name.as_ref(), "Stage file");
    assert!(stage_selected_file.matches("selected file"));

    assert_eq!(
      unstage_selected_file.id,
      CommandPaletteCommandId::UnstageSelectedFile
    );
    assert_eq!(unstage_selected_file.name.as_ref(), "Unstage file");
    assert!(unstage_selected_file.matches("selected file"));

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

  #[test]
  fn command_palette_command_id_string_round_trip() {
    let all_ids = [
      CommandPaletteCommandId::SwitchRepository,
      CommandPaletteCommandId::ForgetRepository,
      CommandPaletteCommandId::SwitchBranch,
      CommandPaletteCommandId::CheckoutDetached,
      CommandPaletteCommandId::Commit,
      CommandPaletteCommandId::ContinueRebase,
      CommandPaletteCommandId::SkipRebase,
      CommandPaletteCommandId::Push,
      CommandPaletteCommandId::ForcePush,
      CommandPaletteCommandId::UndoLastCommit,
      CommandPaletteCommandId::Amend,
      CommandPaletteCommandId::StageSelectedFile,
      CommandPaletteCommandId::UnstageSelectedFile,
      CommandPaletteCommandId::AcceptAllCurrentConflicts,
      CommandPaletteCommandId::AcceptAllIncomingConflicts,
      CommandPaletteCommandId::CreateBranch,
      CommandPaletteCommandId::CreateBranchFrom,
      CommandPaletteCommandId::DeleteBranch,
      CommandPaletteCommandId::MergeBranch,
      CommandPaletteCommandId::AbortMerge,
      CommandPaletteCommandId::RebaseBranch,
      CommandPaletteCommandId::InteractiveRebase,
      CommandPaletteCommandId::InteractiveRebaseOntoBranch,
      CommandPaletteCommandId::InteractiveRebaseEditBranch,
      CommandPaletteCommandId::InteractiveRebaseHeadCount,
      CommandPaletteCommandId::AbortRebase,
      CommandPaletteCommandId::CreatePullRequest,
      CommandPaletteCommandId::CherryPick,
      CommandPaletteCommandId::StageAll,
      CommandPaletteCommandId::UnstageAll,
      CommandPaletteCommandId::Pull,
      CommandPaletteCommandId::Fetch,
      CommandPaletteCommandId::Stash,
      CommandPaletteCommandId::StashIncludeUntracked,
      CommandPaletteCommandId::ApplyStash,
      CommandPaletteCommandId::DropStash,
      CommandPaletteCommandId::PopStash,
      CommandPaletteCommandId::CreateGithubRepository,
      CommandPaletteCommandId::SearchGithubRepository,
      CommandPaletteCommandId::OpenRepository,
      CommandPaletteCommandId::OpenGitPage,
      CommandPaletteCommandId::OpenGithubPage,
      CommandPaletteCommandId::OpenGithubFromUrl,
      CommandPaletteCommandId::SwitchToPrBranch,
      CommandPaletteCommandId::CopyPrBranch,
      CommandPaletteCommandId::ToggleUnchangedFiles,
      CommandPaletteCommandId::OpenGitHistorySidebar,
      CommandPaletteCommandId::OpenGitChangesSidebar,
      CommandPaletteCommandId::OpenGitConfigPage,
      CommandPaletteCommandId::OpenSettingsPage,
      CommandPaletteCommandId::OpenBillingPage,
      CommandPaletteCommandId::OpenAboutPage,
      CommandPaletteCommandId::SendFeedback,
    ];

    for id in all_ids {
      let key = id.as_str();
      assert_eq!(
        CommandPaletteCommandId::from_str(key),
        Some(id),
        "round-trip failed for {:?}",
        id
      );
    }

    let mut keys = all_ids.iter().map(|id| id.as_str()).collect::<Vec<_>>();
    keys.sort();
    let len_before_dedup = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), len_before_dedup, "duplicate as_str keys");

    assert_eq!(CommandPaletteCommandId::from_str("nonexistent"), None);
  }

  #[test]
  fn build_matched_sections_prepends_recent_on_empty_query() {
    let commit = Rc::new(CommandPaletteCommand::commit());
    let fetch = Rc::new(CommandPaletteCommand::fetch());
    let filtered = vec![commit.clone(), fetch];
    let recent = vec![commit];

    let sections = super::build_matched_sections(&filtered, &recent, "", true);
    assert_eq!(
      sections.first().map(|(g, _)| *g),
      Some(CommandPaletteGroup::Recent)
    );
    assert_eq!(
      sections.first().map(|(_, items)| items.len()),
      Some(1),
      "Recent section should contain the one recent command"
    );
  }

  #[test]
  fn build_matched_sections_skips_recent_when_query_non_empty() {
    let commit = Rc::new(CommandPaletteCommand::commit());
    let filtered = vec![commit.clone()];
    let recent = vec![commit];

    let sections = super::build_matched_sections(&filtered, &recent, "commit", true);
    assert!(
      !sections
        .iter()
        .any(|(g, _)| *g == CommandPaletteGroup::Recent)
    );
  }

  #[test]
  fn build_matched_sections_skips_recent_when_disabled() {
    let commit = Rc::new(CommandPaletteCommand::commit());
    let filtered = vec![commit.clone()];
    let recent = vec![commit];

    let sections = super::build_matched_sections(&filtered, &recent, "", false);
    assert!(
      !sections
        .iter()
        .any(|(g, _)| *g == CommandPaletteGroup::Recent)
    );
  }

  #[test]
  fn build_matched_sections_skips_recent_when_empty() {
    let commit = Rc::new(CommandPaletteCommand::commit());
    let filtered = vec![commit];
    let recent: Vec<Rc<CommandPaletteCommand>> = Vec::new();

    let sections = super::build_matched_sections(&filtered, &recent, "", true);
    assert!(
      !sections
        .iter()
        .any(|(g, _)| *g == CommandPaletteGroup::Recent)
    );
  }
}
