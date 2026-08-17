use std::{collections::BTreeMap, rc::Rc, sync::Arc};

use crate::github_url::parse_github_pull_request_url_action;
use crate::palette::{
  palette_empty, palette_footer, palette_list_item, palette_search_list, palette_section_header,
  update_selected_index,
};
use crate::{UiIconName, file_icon_path_for_name};
use gpui::{
  App, Context, Div, Entity, FocusHandle, Focusable, Global, InteractiveElement, IntoElement,
  ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div, prelude::*,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, IndexPath, Sizable, WindowExt, h_flex,
  input::{Input, InputEvent, InputState},
  label::Label,
  list::{ListDelegate, ListEvent, ListItem, ListState},
  notification::Notification,
  v_flex,
};

pub const COMMAND_PALETTE_CONTEXT: &str = "CommandPalette";

pub type CommandPaletteUsageRecorder = fn(CommandPaletteCommandId, &App);

pub struct CommandPaletteUsageRecorderGlobal(pub CommandPaletteUsageRecorder);

impl Global for CommandPaletteUsageRecorderGlobal {}

pub type CommandPaletteUsageScorer = fn(&App, CommandPaletteCommandId, i64) -> f64;

pub struct CommandPaletteUsageScorerGlobal(pub CommandPaletteUsageScorer);

impl Global for CommandPaletteUsageScorerGlobal {}

fn palette_row(
  icon: Icon,
  label: SharedString,
  hint: Option<SharedString>,
  theme: &gpui_component::Theme,
) -> impl IntoElement {
  h_flex()
    .w_full()
    .items_center()
    .justify_between()
    .gap_3()
    .child(
      h_flex()
        .min_w_0()
        .flex_1()
        .items_center()
        .gap_2()
        .child(icon.small().text_color(theme.muted_foreground))
        .child(
          div()
            .min_w_0()
            .flex_1()
            .text_sm()
            .overflow_hidden()
            .text_ellipsis()
            .child(Label::new(label)),
        ),
    )
    .when_some(hint, |row, hint| {
      row.child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .whitespace_nowrap()
          .text_ellipsis()
          .overflow_hidden()
          .child(hint),
      )
    })
}

#[derive(Clone, Debug)]
pub struct CommandPaletteCommand {
  pub id: CommandPaletteCommandId,
  pub name: SharedString,
  pub description: Option<SharedString>,
  pub disabled_reason: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandPalettePage {
  Session,
  Git,
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
  OpenPullRequest,
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
  RestoreAll,
  Pull,
  Fetch,
  Stash {
    include_untracked: bool,
    message: Option<String>,
  },
  ApplyStash(CommandPaletteStash),
  DropStash(CommandPaletteStash),
  PopStash(CommandPaletteStash),
  OpenRepository,
  OpenSessionPage,
  OpenGitPage,
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
  OpenPrMergePopover,
  OpenPrReviewPopover,
  TogglePrCommitByCommit,
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
    let theme = cx.theme().clone();

    self.matched_branches.get(ix.row).map(|branch| {
      palette_list_item(ix, self.selected_index).child(palette_row(
        Icon::new(UiIconName::GitBranch),
        branch.name.clone(),
        None,
        &theme,
      ))
    })
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    palette_empty(cx)
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
    let theme = cx.theme().clone();

    self.matched_repositories.get(ix.row).map(|repository| {
      palette_list_item(ix, self.selected_index).child(palette_row(
        Icon::new(IconName::FolderOpen),
        repository.path.clone(),
        None,
        &theme,
      ))
    })
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    palette_empty(cx)
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
    let theme = cx.theme().clone();

    self.matched_stashes.get(ix.row).map(|stash| {
      let label: SharedString = format!("#{} {}", stash.index, stash.name.as_ref()).into();
      let oid: SharedString = stash.oid.chars().take(7).collect::<String>().into();
      palette_list_item(ix, self.selected_index).child(palette_row(
        Icon::new(IconName::Inbox),
        label,
        Some(oid),
        &theme,
      ))
    })
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    palette_empty(cx)
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
    let theme = cx.theme().clone();
    let base_item = palette_list_item(ix, self.selected_index);

    self
      .matched_branches_and_commands
      .get(ix.row)
      .map(|branch| match branch.as_ref() {
        BranchListWithCommands::CommandPaletteCommand(command) => base_item.child(palette_row(
          command.icon(),
          command.name.clone(),
          None,
          &theme,
        )),
        BranchListWithCommands::SwitchBranch(branch) => base_item.child(palette_row(
          Icon::new(UiIconName::GitBranch),
          branch.name.clone(),
          None,
          &theme,
        )),
      })
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    palette_empty(cx)
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
    .filter(|c| !c.is_disabled())
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
    let theme = cx.theme().clone();

    self.item_at(ix).map(|command| {
      palette_list_item(ix, self.selected_index)
        .when(command.is_disabled(), |item| item.opacity(0.55))
        .child(palette_row(
          command.icon(),
          command.name.clone(),
          command.disabled_reason.clone(),
          &theme,
        ))
    })
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    palette_empty(cx)
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
    Some(palette_section_header(group.label(), cx))
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
  SwitchRepository,
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
  OpenPullRequest,
  CherryPick,
  StageAll,
  UnstageAll,
  RestoreAll,
  Pull,
  Fetch,
  Stash,
  StashIncludeUntracked,
  ApplyStash,
  DropStash,
  PopStash,
  OpenRepository,
  OpenSessionPage,
  OpenGitPage,
  OpenGithubFromUrl,
  SwitchToPrBranch,
  CopyPrBranch,
  ToggleUnchangedFiles,
  OpenPrMergePopover,
  OpenPrReviewPopover,
  TogglePrCommitByCommit,
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
      Self::OpenPullRequest => "open_pull_request",
      Self::CherryPick => "cherry_pick",
      Self::StageAll => "stage_all",
      Self::UnstageAll => "unstage_all",
      Self::RestoreAll => "restore_all",
      Self::Pull => "pull",
      Self::Fetch => "fetch",
      Self::Stash => "stash",
      Self::StashIncludeUntracked => "stash_include_untracked",
      Self::ApplyStash => "apply_stash",
      Self::DropStash => "drop_stash",
      Self::PopStash => "pop_stash",
      Self::OpenRepository => "open_repository",
      Self::OpenSessionPage => "open_session_page",
      Self::OpenGitPage => "open_git_page",
      Self::OpenGithubFromUrl => "open_github_from_url",
      Self::SwitchToPrBranch => "switch_to_pr_branch",
      Self::CopyPrBranch => "copy_pr_branch",
      Self::ToggleUnchangedFiles => "toggle_unchanged_files",
      Self::OpenPrMergePopover => "open_pr_merge_popover",
      Self::OpenPrReviewPopover => "open_pr_review_popover",
      Self::TogglePrCommitByCommit => "toggle_pr_commit_by_commit",
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
      "open_pull_request" => Some(Self::OpenPullRequest),
      "cherry_pick" => Some(Self::CherryPick),
      "stage_all" => Some(Self::StageAll),
      "restore_all" => Some(Self::RestoreAll),
      "unstage_all" => Some(Self::UnstageAll),
      "pull" => Some(Self::Pull),
      "fetch" => Some(Self::Fetch),
      "stash" => Some(Self::Stash),
      "stash_include_untracked" => Some(Self::StashIncludeUntracked),
      "apply_stash" => Some(Self::ApplyStash),
      "drop_stash" => Some(Self::DropStash),
      "pop_stash" => Some(Self::PopStash),
      "open_repository" => Some(Self::OpenRepository),
      "open_session_page" => Some(Self::OpenSessionPage),
      "open_git_page" => Some(Self::OpenGitPage),
      "open_github_from_url" => Some(Self::OpenGithubFromUrl),
      "switch_to_pr_branch" => Some(Self::SwitchToPrBranch),
      "copy_pr_branch" => Some(Self::CopyPrBranch),
      "toggle_unchanged_files" => Some(Self::ToggleUnchangedFiles),
      "open_pr_merge_popover" => Some(Self::OpenPrMergePopover),
      "open_pr_review_popover" => Some(Self::OpenPrReviewPopover),
      "toggle_pr_commit_by_commit" => Some(Self::TogglePrCommitByCommit),
      "open_git_config_page" => Some(Self::OpenGitConfigPage),
      "open_settings_page" => Some(Self::OpenSettingsPage),
      "open_billing_page" => Some(Self::OpenBillingPage),
      "open_about_page" => Some(Self::OpenAboutPage),
      "send_feedback" => Some(Self::SendFeedback),
      _ => None,
    }
  }

  /// Parent command id whose recents score should also be boosted when this
  /// command is triggered. Lets sub-menu variants surface their parent in
  /// the Recent section even though only the parent lives in the root list.
  pub fn parent_for_recents(self) -> Option<Self> {
    match self {
      Self::InteractiveRebaseEditBranch
      | Self::InteractiveRebaseOntoBranch
      | Self::InteractiveRebaseHeadCount => Some(Self::InteractiveRebase),
      Self::CreateBranch | Self::CreateBranchFrom => Some(Self::SwitchBranch),
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

  fn new(
    id: CommandPaletteCommandId,
    name: impl Into<SharedString>,
    description: impl Into<SharedString>,
  ) -> Self {
    Self {
      id,
      name: name.into(),
      description: Some(description.into()),
      disabled_reason: None,
    }
  }

  pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
    self.disabled_reason = Some(reason.into());
    self
  }

  pub fn is_disabled(&self) -> bool {
    self.disabled_reason.is_some()
  }

  pub fn switch_repository() -> Self {
    Self::new(
      CommandPaletteCommandId::SwitchRepository,
      "Switch repository",
      "Switch to another recent repository",
    )
  }

  pub fn forget_repository() -> Self {
    Self::new(
      CommandPaletteCommandId::ForgetRepository,
      "Forget repository",
      "Remove a repository from the recent list",
    )
  }

  pub fn switch_branch() -> Self {
    Self::new(
      CommandPaletteCommandId::SwitchBranch,
      "Switch branch",
      "Checkout or create branches",
    )
  }

  pub fn commit() -> Self {
    Self::new(
      CommandPaletteCommandId::Commit,
      "Commit",
      "Create a commit (stages all changes if needed)",
    )
  }

  pub fn checkout_detached() -> Self {
    Self::new(
      CommandPaletteCommandId::CheckoutDetached,
      "Git checkout detached",
      "Detach HEAD at a commit hash or tag",
    )
  }

  pub fn continue_rebase() -> Self {
    Self::new(
      CommandPaletteCommandId::ContinueRebase,
      "Rebase continue",
      "Continue the current rebase",
    )
  }

  pub fn skip_rebase() -> Self {
    Self::new(
      CommandPaletteCommandId::SkipRebase,
      "Rebase skip",
      "Skip the current rebase commit",
    )
  }

  pub fn push(label: impl Into<SharedString>) -> Self {
    Self::new(
      CommandPaletteCommandId::Push,
      label,
      "Push local commits to the remote branch",
    )
  }

  pub fn force_push() -> Self {
    Self::new(
      CommandPaletteCommandId::ForcePush,
      "Force push (with lease)",
      "Force push local commits to the remote branch",
    )
  }

  pub fn undo_last_commit() -> Self {
    Self::new(
      CommandPaletteCommandId::UndoLastCommit,
      "Undo last commit",
      "Undo the most recent local commit",
    )
  }

  pub fn amend() -> Self {
    Self::new(
      CommandPaletteCommandId::Amend,
      "Amend",
      "Amend the most recent commit",
    )
  }

  pub fn stage_selected_file() -> Self {
    Self::new(
      CommandPaletteCommandId::StageSelectedFile,
      "Stage file",
      "Stage the selected file",
    )
  }

  pub fn unstage_selected_file() -> Self {
    Self::new(
      CommandPaletteCommandId::UnstageSelectedFile,
      "Unstage file",
      "Unstage the selected file",
    )
  }

  pub fn accept_all_current_conflicts() -> Self {
    Self::new(
      CommandPaletteCommandId::AcceptAllCurrentConflicts,
      "Accept all current conflicts",
      "Resolve all conflict regions by keeping current changes",
    )
  }

  pub fn accept_all_incoming_conflicts() -> Self {
    Self::new(
      CommandPaletteCommandId::AcceptAllIncomingConflicts,
      "Accept all incoming conflicts",
      "Resolve all conflict regions by keeping incoming changes",
    )
  }

  pub fn merge_branch() -> Self {
    Self::new(
      CommandPaletteCommandId::MergeBranch,
      "Merge branch",
      "Merge a branch into the current branch",
    )
  }

  pub fn rebase_branch() -> Self {
    Self::new(
      CommandPaletteCommandId::RebaseBranch,
      "Rebase branch",
      "Rebase the current branch onto another branch",
    )
  }

  pub fn interactive_rebase() -> Self {
    Self::new(
      CommandPaletteCommandId::InteractiveRebase,
      "Rebase interactive",
      "Interactively edit and reorder commits before rebasing",
    )
  }

  pub fn interactive_rebase_onto_branch() -> Self {
    Self::new(
      CommandPaletteCommandId::InteractiveRebaseOntoBranch,
      "Onto branch",
      "Start interactive rebase onto another branch",
    )
  }

  pub fn interactive_rebase_edit_branch() -> Self {
    Self::new(
      CommandPaletteCommandId::InteractiveRebaseEditBranch,
      "Edit commits since branch",
      "Reorder, squash, or edit commits without incorporating upstream changes",
    )
  }

  pub fn interactive_rebase_head_count() -> Self {
    Self::new(
      CommandPaletteCommandId::InteractiveRebaseHeadCount,
      "Last N commits (HEAD~n)",
      "Start interactive rebase for the last N commits",
    )
  }

  pub fn abort_merge() -> Self {
    Self::new(
      CommandPaletteCommandId::AbortMerge,
      "Abort merge",
      "Abort the current merge operation",
    )
  }

  pub fn abort_rebase() -> Self {
    Self::new(
      CommandPaletteCommandId::AbortRebase,
      "Abort rebase",
      "Abort the current rebase operation",
    )
  }

  pub fn create_branch() -> Self {
    Self::new(
      CommandPaletteCommandId::CreateBranch,
      "Create branch",
      "Create a new branch",
    )
  }

  pub fn create_pull_request() -> Self {
    Self::new(
      CommandPaletteCommandId::CreatePullRequest,
      "Create pull request",
      "Create a pull request for the current branch",
    )
  }

  pub fn open_pull_request(number: u64) -> Self {
    Self::new(
      CommandPaletteCommandId::OpenPullRequest,
      format!("Open PR #{number}"),
      "Open the pull request for the current branch",
    )
  }

  pub fn create_branch_from() -> Self {
    Self::new(
      CommandPaletteCommandId::CreateBranchFrom,
      "Create branch from...",
      "Create a new branch from an existing branch",
    )
  }

  pub fn delete_branch() -> Self {
    Self::new(
      CommandPaletteCommandId::DeleteBranch,
      "Delete branch",
      "Force delete a local branch, or delete a remote branch",
    )
  }

  pub fn cherry_pick() -> Self {
    Self::new(
      CommandPaletteCommandId::CherryPick,
      "Cherry pick",
      "Apply one or more commits to the current branch",
    )
  }

  pub fn stage_all() -> Self {
    Self::new(
      CommandPaletteCommandId::StageAll,
      "Stage all",
      "Stage all changed files",
    )
  }

  pub fn unstage_all() -> Self {
    Self::new(
      CommandPaletteCommandId::UnstageAll,
      "Unstage all",
      "Unstage all staged files",
    )
  }

  pub fn restore_all() -> Self {
    Self::new(
      CommandPaletteCommandId::RestoreAll,
      "Restore all",
      "Discard every change in the working tree",
    )
  }

  pub fn pull() -> Self {
    Self::new(
      CommandPaletteCommandId::Pull,
      "Pull",
      "Pull changes from the remote branch",
    )
  }

  pub fn fetch() -> Self {
    Self::new(
      CommandPaletteCommandId::Fetch,
      "Fetch",
      "Fetch updates from remote repositories",
    )
  }

  pub fn stash() -> Self {
    Self::new(
      CommandPaletteCommandId::Stash,
      "Stash",
      "Stash tracked changes",
    )
  }

  pub fn stash_with_untracked() -> Self {
    Self::new(
      CommandPaletteCommandId::StashIncludeUntracked,
      "Stash with untracked",
      "Stash tracked and untracked changes",
    )
  }

  pub fn apply_stash() -> Self {
    Self::new(
      CommandPaletteCommandId::ApplyStash,
      "Apply stash",
      "Apply a stash entry without dropping it",
    )
  }

  pub fn drop_stash() -> Self {
    Self::new(
      CommandPaletteCommandId::DropStash,
      "Drop stash",
      "Delete a stash entry",
    )
  }

  pub fn pop_stash() -> Self {
    Self::new(
      CommandPaletteCommandId::PopStash,
      "Pop stash",
      "Apply and delete a stash entry",
    )
  }

  pub fn open_repository() -> Self {
    Self::new(
      CommandPaletteCommandId::OpenRepository,
      "Open repository",
      "Pick and open a local repository",
    )
  }

  pub fn open_session_page() -> Self {
    Self::new(
      CommandPaletteCommandId::OpenSessionPage,
      "Go to Sessions",
      "Navigate to the sessions workspace",
    )
  }

  pub fn open_git_page() -> Self {
    Self::new(
      CommandPaletteCommandId::OpenGitPage,
      "Go to Git",
      "Navigate to the Git page",
    )
  }

  pub fn open_github_from_url() -> Self {
    Self::new(
      CommandPaletteCommandId::OpenGithubFromUrl,
      "Open pull request from URL",
      "Open a GitHub pull request in Reviu from its URL",
    )
  }

  pub fn switch_to_pr_branch() -> Self {
    Self::new(
      CommandPaletteCommandId::SwitchToPrBranch,
      "Switch to PR branch",
      "Switch the local repository to the current pull request branch",
    )
  }

  pub fn copy_pr_branch() -> Self {
    Self::new(
      CommandPaletteCommandId::CopyPrBranch,
      "Copy PR branch name",
      "Copy the source branch name of the current pull request",
    )
  }

  pub fn open_pr_merge_popover() -> Self {
    Self::new(
      CommandPaletteCommandId::OpenPrMergePopover,
      "Merge pull request",
      "Open the merge popover for the current pull request",
    )
  }

  pub fn open_pr_review_popover() -> Self {
    Self::new(
      CommandPaletteCommandId::OpenPrReviewPopover,
      "Review pull request",
      "Open the review popover for the current pull request",
    )
  }

  pub fn toggle_pr_commit_by_commit(in_commit_by_commit_mode: bool) -> Self {
    if in_commit_by_commit_mode {
      Self::new(
        CommandPaletteCommandId::TogglePrCommitByCommit,
        "Show all changes",
        "Exit commit-by-commit review and show all pull request changes",
      )
    } else {
      Self::new(
        CommandPaletteCommandId::TogglePrCommitByCommit,
        "Review commit by commit",
        "Step through pull request changes one commit at a time",
      )
    }
  }

  pub fn toggle_unchanged_files(currently_shown: bool) -> Self {
    if currently_shown {
      Self::new(
        CommandPaletteCommandId::ToggleUnchangedFiles,
        "Hide unchanged files",
        "Show only files changed in this pull request",
      )
    } else {
      Self::new(
        CommandPaletteCommandId::ToggleUnchangedFiles,
        "Show unchanged files",
        "Show all project files alongside changed files",
      )
    }
  }

  pub fn open_settings_page() -> Self {
    Self::new(
      CommandPaletteCommandId::OpenSettingsPage,
      "Go to Settings",
      "Navigate to Settings",
    )
  }

  pub fn open_billing_page() -> Self {
    Self::new(
      CommandPaletteCommandId::OpenBillingPage,
      "Go to Billing",
      "Navigate to Billing",
    )
  }

  pub fn open_about_page() -> Self {
    Self::new(
      CommandPaletteCommandId::OpenAboutPage,
      "Go to About",
      "Navigate to About",
    )
  }

  pub fn send_feedback() -> Self {
    Self::new(
      CommandPaletteCommandId::SendFeedback,
      "Send Feedback",
      "Report a bug or suggest a feature",
    )
  }

  pub fn open_git_config_page() -> Self {
    Self::new(
      CommandPaletteCommandId::OpenGitConfigPage,
      "Go to Git Config",
      "Edit ~/.gitconfig",
    )
  }

  pub fn default_global_commands(
    current_page: CommandPalettePage,
    include_github: bool,
  ) -> Vec<Self> {
    let mut commands = Vec::new();

    if current_page != CommandPalettePage::Session {
      commands.push(Self::open_session_page());
    }

    if current_page != CommandPalettePage::Git {
      commands.push(Self::open_git_page());
    }

    if include_github {
      commands.push(Self::open_github_from_url());
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
      | CommandPaletteCommandId::RestoreAll
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
      | CommandPaletteCommandId::OpenPullRequest
      | CommandPaletteCommandId::SwitchToPrBranch
      | CommandPaletteCommandId::CopyPrBranch
      | CommandPaletteCommandId::ToggleUnchangedFiles
      | CommandPaletteCommandId::OpenPrMergePopover
      | CommandPaletteCommandId::OpenPrReviewPopover
      | CommandPaletteCommandId::TogglePrCommitByCommit => CommandPaletteGroup::PullRequest,

      CommandPaletteCommandId::SwitchRepository
      | CommandPaletteCommandId::ForgetRepository
      | CommandPaletteCommandId::OpenRepository => CommandPaletteGroup::Repository,

      CommandPaletteCommandId::OpenGithubFromUrl => CommandPaletteGroup::Github,

      CommandPaletteCommandId::OpenSessionPage
      | CommandPaletteCommandId::OpenGitPage
      | CommandPaletteCommandId::OpenGitConfigPage
      | CommandPaletteCommandId::OpenSettingsPage
      | CommandPaletteCommandId::OpenBillingPage
      | CommandPaletteCommandId::OpenAboutPage => CommandPaletteGroup::Navigation,

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
      CommandPaletteCommandId::RestoreAll => Icon::new(IconName::Undo),
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
      CommandPaletteCommandId::OpenRepository => Icon::new(IconName::FolderOpen),
      CommandPaletteCommandId::CreateBranch | CommandPaletteCommandId::CreateBranchFrom => {
        Icon::new(IconName::Plus)
      }
      CommandPaletteCommandId::CreatePullRequest | CommandPaletteCommandId::OpenPullRequest => {
        Icon::new(UiIconName::GitPullRequestArrow)
      }
      CommandPaletteCommandId::OpenSessionPage => Icon::new(UiIconName::MessageCircle),
      CommandPaletteCommandId::OpenGitPage => Icon::new(UiIconName::GitBranch),
      CommandPaletteCommandId::OpenGithubFromUrl => Icon::new(IconName::Github),
      CommandPaletteCommandId::SwitchToPrBranch => Icon::new(UiIconName::GitBranch),
      CommandPaletteCommandId::CopyPrBranch => Icon::new(IconName::Copy),
      CommandPaletteCommandId::ToggleUnchangedFiles => Icon::new(UiIconName::ScanEye),
      CommandPaletteCommandId::OpenPrMergePopover => Icon::new(UiIconName::GitMerge),
      CommandPaletteCommandId::OpenPrReviewPopover => Icon::new(UiIconName::Eye),
      CommandPaletteCommandId::TogglePrCommitByCommit => Icon::new(UiIconName::GitCommitHorizontal),
      CommandPaletteCommandId::OpenGitConfigPage => Self::git_config_icon(),
      CommandPaletteCommandId::OpenSettingsPage => Icon::new(IconName::Settings2),
      CommandPaletteCommandId::OpenBillingPage => Icon::new(UiIconName::CreditCard),
      CommandPaletteCommandId::OpenAboutPage => Icon::new(UiIconName::Info),
      CommandPaletteCommandId::SendFeedback => Icon::new(UiIconName::MessageCircle),
    }
  }

  fn matches(&self, query: &str) -> bool {
    if self.id == CommandPaletteCommandId::OpenGithubFromUrl
      && parse_github_pull_request_url_action(query).is_some()
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
      || self
        .disabled_reason
        .as_ref()
        .map(|text| text.as_ref().to_lowercase().contains(&query))
        .unwrap_or(false)
  }
}

pub struct CommandPaletteConfig {
  pub branches: Vec<CommandPaletteBranch>,
  pub rebase_branches: Vec<CommandPaletteBranch>,
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
      rebase_branches: Vec::new(),
      delete_branches: Vec::new(),
      stashes: Vec::new(),
      default_stash_message: None,
      repositories: Vec::new(),
      commands,
      initial_screen: CommandPaletteInitialScreen::Root,
      on_action,
    }
  }

  pub fn with_rebase_branches(mut self, rebase_branches: Vec<CommandPaletteBranch>) -> Self {
    self.rebase_branches = rebase_branches;
    self
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
      CommandPaletteInitialScreen::SwitchRepository => CommandPaletteScreen::SwitchRepository,
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
  rebase_branches_list: Entity<ListState<BranchesListDelegate>>,
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
  fn git_shell_arg(value: &str) -> String {
    if value
      .chars()
      .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/'))
    {
      return value.to_string();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
  }

  fn branch_git_command(
    screen: CommandPaletteScreen,
    branch: &CommandPaletteBranch,
  ) -> Option<String> {
    let branch_name = Self::git_shell_arg(branch.name.as_ref());
    match screen {
      CommandPaletteScreen::MergeBranch => Some(format!("git merge {branch_name}")),
      CommandPaletteScreen::RebaseBranch => Some(format!("git rebase {branch_name}")),
      CommandPaletteScreen::InteractiveRebaseBranch => Some(format!("git rebase -i {branch_name}")),
      CommandPaletteScreen::InteractiveRebaseEditBranch => Some(format!(
        "git rebase -i --onto $(git merge-base HEAD {branch_name}) {branch_name}"
      )),
      _ => None,
    }
  }

  fn interactive_rebase_head_count_git_command(value: &str) -> String {
    match Self::parse_interactive_rebase_head_count(value) {
      Some(count) => format!("git rebase -i HEAD~{count}"),
      None => "git rebase -i HEAD~n".to_string(),
    }
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
      cx.new(|cx| InputState::new(window, cx).placeholder("Paste a pull request URL..."));
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

    let default_rebase_branches: Vec<Rc<CommandPaletteBranch>> = config
      .rebase_branches
      .iter()
      .cloned()
      .map(Rc::new)
      .collect();

    let rebase_branches_list_delegate = BranchesListDelegate {
      _branches: default_rebase_branches.clone(),
      matched_branches: default_rebase_branches.clone(),
      selected_index: None,
      query: "".into(),
    };

    let rebase_branches_list =
      cx.new(|cx| ListState::new(rebase_branches_list_delegate, window, cx).searchable(true));

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
            command_palette.select_command_entry(command.as_ref(), cx, window);
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
            command_palette.select_command_entry(command.as_ref(), cx, window);
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
                  command_palette.select_command_entry(command, cx, window);
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
              CommandPaletteScreen::MergeBranch => {
                let branch = {
                  let list = list_state.read(cx);
                  list.delegate().matched_branches.get(ix.row).cloned()
                };

                if let Some(branch) = branch {
                  command_palette.trigger_action(
                    CommandPaletteCommandId::MergeBranch,
                    CommandPaletteAction::MergeBranch {
                      name: (*branch).clone(),
                    },
                    window,
                    cx,
                  );
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
        &rebase_branches_list,
        window,
        |command_palette, list_state, ev: &ListEvent, window, cx| {
          if let ListEvent::Confirm(ix) = ev {
            let branch = {
              let list = list_state.read(cx);
              list.delegate().matched_branches.get(ix.row).cloned()
            };

            if let Some(branch) = branch {
              let (id, action) = match command_palette.screen {
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
                _ => return,
              };
              command_palette.trigger_action(id, action, window, cx);
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
      rebase_branches_list,
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
    if let InputEvent::PressEnter { .. } = event {
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

    let Some(action) = parse_github_pull_request_url_action(&url) else {
      window.push_notification(Notification::error("Invalid pull request URL"), cx);
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
      CommandPaletteScreen::MergeBranch => {
        self.branches_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      CommandPaletteScreen::RebaseBranch
      | CommandPaletteScreen::InteractiveRebaseBranch
      | CommandPaletteScreen::InteractiveRebaseEditBranch => {
        self.rebase_branches_list.update(cx, |state, cx| {
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

  fn select_command_entry(
    &mut self,
    command: &CommandPaletteCommand,
    cx: &mut Context<Self>,
    window: &mut Window,
  ) {
    if let Some(reason) = command.disabled_reason.as_ref() {
      window.push_notification(Notification::info(reason.clone()), cx);
      return;
    }

    self.select_command(command.id, cx, window);
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
      CommandPaletteCommandId::OpenPullRequest => {
        self.trigger_action(command, CommandPaletteAction::OpenPullRequest, window, cx);
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
      CommandPaletteCommandId::RestoreAll => {
        self.trigger_action(command, CommandPaletteAction::RestoreAll, window, cx);
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
      CommandPaletteCommandId::OpenSessionPage => {
        self.trigger_action(command, CommandPaletteAction::OpenSessionPage, window, cx);
      }
      CommandPaletteCommandId::OpenGitPage => {
        self.trigger_action(command, CommandPaletteAction::OpenGitPage, window, cx);
      }
      CommandPaletteCommandId::OpenGithubFromUrl => {
        let query = self.commands_list.read(cx).delegate().query.to_string();
        if let Some(action) = parse_github_pull_request_url_action(&query) {
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
      CommandPaletteCommandId::OpenPrMergePopover => {
        self.trigger_action(
          command,
          CommandPaletteAction::OpenPrMergePopover,
          window,
          cx,
        );
      }
      CommandPaletteCommandId::OpenPrReviewPopover => {
        self.trigger_action(
          command,
          CommandPaletteAction::OpenPrReviewPopover,
          window,
          cx,
        );
      }
      CommandPaletteCommandId::TogglePrCommitByCommit => {
        self.trigger_action(
          command,
          CommandPaletteAction::TogglePrCommitByCommit,
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
          if let Some(parent) = id.parent_for_recents() {
            recorder(parent, cx);
          }
        }
        window.close_dialog(cx);
      }
      Err(err) => {
        window.push_notification(Notification::error(err), cx);
      }
    }
  }

  fn render_search_list<D: ListDelegate>(
    &self,
    list: &Entity<ListState<D>>,
    placeholder: &'static str,
  ) -> impl IntoElement {
    palette_search_list(list, placeholder)
  }

  fn render_git_command_preview(
    &self,
    command: impl Into<SharedString>,
    cx: &Context<Self>,
  ) -> Div {
    let theme = cx.theme();
    h_flex()
      .items_center()
      .gap_1()
      .px_3()
      .py_2()
      .border_t_1()
      .border_color(theme.border)
      .bg(theme.muted)
      .text_xs()
      .text_color(theme.muted_foreground)
      .child(
        div()
          .min_w_0()
          .flex_1()
          .overflow_hidden()
          .whitespace_nowrap()
          .text_ellipsis()
          .text_color(theme.foreground)
          .child(command.into()),
      )
  }

  fn selected_branch_git_command(
    &self,
    list: &Entity<ListState<BranchesListDelegate>>,
    cx: &Context<Self>,
  ) -> Option<String> {
    let delegate = list.read(cx);
    let delegate = delegate.delegate();
    let branch = delegate
      .selected_index
      .and_then(|ix| delegate.matched_branches.get(ix.row))
      .or_else(|| delegate.matched_branches.first())?;
    Self::branch_git_command(self.screen, branch.as_ref())
  }

  fn render_input_screen(&self, input: &Entity<InputState>, cx: &Context<Self>) -> Div {
    v_flex()
      .p_2()
      .child(Input::new(input).border_color(cx.theme().border))
  }

  fn render_root(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
    self.render_search_list(&self.commands_list, "Search commands...")
  }

  fn render_switch_repository(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
    self.render_search_list(&self.repositories_list, "Search repositories...")
  }

  fn render_forget_repository(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
    self.render_search_list(&self.repositories_list, "Select repository to forget...")
  }

  fn render_switch_branch(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
    self.render_search_list(&self.branches_with_commands_list, "Search branches...")
  }

  fn render_create_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_input_screen(&self.create_branch_input, cx)
  }

  fn render_checkout_detached(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_input_screen(&self.checkout_detached_input, cx)
  }

  fn render_cherry_pick(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_input_screen(&self.cherry_pick_input, cx)
  }

  fn render_stash(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_input_screen(&self.stash_input, cx)
  }

  fn render_stash_include_untracked(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_stash(cx)
  }

  fn render_select_stash(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
    self.render_search_list(&self.stashes_list, "Search stashes...")
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

  fn render_delete_branch(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
    self.render_search_list(&self.delete_branches_list, "Select branch to delete...")
  }

  fn render_merge_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let command = self.selected_branch_git_command(&self.branches_list, cx);

    v_flex()
      .child(self.render_search_list(&self.branches_list, "Search branches..."))
      .when_some(command, |this, command| {
        this.child(self.render_git_command_preview(command, cx))
      })
  }

  fn render_interactive_rebase_mode(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
    self.render_search_list(
      &self.interactive_rebase_mode_list,
      "Select interactive rebase mode...",
    )
  }

  fn render_interactive_rebase_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_rebase_branch(cx)
  }

  fn render_interactive_rebase_head_count(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let command = self
      .interactive_rebase_head_count_input
      .read(cx)
      .value()
      .to_string();
    let command = Self::interactive_rebase_head_count_git_command(&command);

    v_flex()
      .child(self.render_input_screen(&self.interactive_rebase_head_count_input, cx))
      .child(self.render_git_command_preview(command, cx))
  }

  fn render_rebase_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let command = self.selected_branch_git_command(&self.rebase_branches_list, cx);

    v_flex()
      .child(self.render_search_list(&self.rebase_branches_list, "Search base branches..."))
      .when_some(command, |this, command| {
        this.child(self.render_git_command_preview(command, cx))
      })
  }

  fn render_open_github_from_url(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_input_screen(&self.open_github_url_input, cx)
  }

  fn render_create_branch_from(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
    self.render_search_list(&self.branches_list, "Search branches...")
  }

  fn footer_key_hints(screen: CommandPaletteScreen) -> (bool, &'static str) {
    match screen {
      CommandPaletteScreen::Root => (true, "run"),
      CommandPaletteScreen::SwitchRepository
      | CommandPaletteScreen::ForgetRepository
      | CommandPaletteScreen::SwitchBranch
      | CommandPaletteScreen::DeleteBranch
      | CommandPaletteScreen::MergeBranch
      | CommandPaletteScreen::RebaseBranch
      | CommandPaletteScreen::InteractiveRebaseMode
      | CommandPaletteScreen::InteractiveRebaseBranch
      | CommandPaletteScreen::InteractiveRebaseEditBranch
      | CommandPaletteScreen::ApplyStash
      | CommandPaletteScreen::DropStash
      | CommandPaletteScreen::PopStash
      | CommandPaletteScreen::CreateBranchFrom => (true, "select"),
      CommandPaletteScreen::CheckoutDetached
      | CommandPaletteScreen::CreateBranch
      | CommandPaletteScreen::InteractiveRebaseHeadCount
      | CommandPaletteScreen::CherryPick
      | CommandPaletteScreen::Stash
      | CommandPaletteScreen::StashIncludeUntracked
      | CommandPaletteScreen::OpenGithubFromUrl => (false, "confirm"),
    }
  }

  fn render_footer(&self, cx: &Context<Self>) -> impl IntoElement {
    let (navigable, enter_label) = Self::footer_key_hints(self.screen);
    palette_footer(navigable, enter_label, cx)
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

    v_flex()
      .key_context(COMMAND_PALETTE_CONTEXT)
      .track_focus(&self.focus_handle)
      .text_color(theme.foreground)
      .child(content)
      .child(self.render_footer(cx))
  }
}

#[cfg(test)]
mod tests {
  use super::{
    CommandPalette, CommandPaletteBranch, CommandPaletteBranchKind, CommandPaletteCommand,
    CommandPaletteCommandId, CommandPaletteConfig, CommandPaletteGroup, CommandPaletteHandler,
    CommandPaletteInitialScreen, CommandPaletteScreen,
  };
  use std::rc::Rc;
  use std::sync::Arc;

  #[test]
  fn footer_hints_match_screen_kind() {
    assert_eq!(
      CommandPalette::footer_key_hints(CommandPaletteScreen::Root),
      (true, "run")
    );
    assert_eq!(
      CommandPalette::footer_key_hints(CommandPaletteScreen::SwitchBranch),
      (true, "select")
    );
    assert_eq!(
      CommandPalette::footer_key_hints(CommandPaletteScreen::MergeBranch),
      (true, "select")
    );
    assert_eq!(
      CommandPalette::footer_key_hints(CommandPaletteScreen::CreateBranch),
      (false, "confirm")
    );
    assert_eq!(
      CommandPalette::footer_key_hints(CommandPaletteScreen::Stash),
      (false, "confirm")
    );
  }

  #[test]
  fn interactive_rebase_variants_have_interactive_rebase_as_recents_parent() {
    assert_eq!(
      CommandPaletteCommandId::InteractiveRebaseEditBranch.parent_for_recents(),
      Some(CommandPaletteCommandId::InteractiveRebase)
    );
    assert_eq!(
      CommandPaletteCommandId::InteractiveRebaseOntoBranch.parent_for_recents(),
      Some(CommandPaletteCommandId::InteractiveRebase)
    );
    assert_eq!(
      CommandPaletteCommandId::InteractiveRebaseHeadCount.parent_for_recents(),
      Some(CommandPaletteCommandId::InteractiveRebase)
    );
    assert_eq!(
      CommandPaletteCommandId::InteractiveRebase.parent_for_recents(),
      None
    );
    assert_eq!(CommandPaletteCommandId::Commit.parent_for_recents(), None);
  }

  #[test]
  fn create_branch_variants_have_switch_branch_as_recents_parent() {
    assert_eq!(
      CommandPaletteCommandId::CreateBranch.parent_for_recents(),
      Some(CommandPaletteCommandId::SwitchBranch)
    );
    assert_eq!(
      CommandPaletteCommandId::CreateBranchFrom.parent_for_recents(),
      Some(CommandPaletteCommandId::SwitchBranch)
    );
    assert_eq!(
      CommandPaletteCommandId::SwitchBranch.parent_for_recents(),
      None
    );
  }

  #[test]
  fn open_github_from_url_command_matches_pull_request_urls_only() {
    let command = CommandPaletteCommand::open_github_from_url();

    assert!(command.matches("https://github.com/joris-gallot/guit/pull/4"));
    assert!(!command.matches("https://github.com/joris-gallot/guit"));
    assert!(!command.matches("https://github.com/joris-gallot/guit/pulls?q=is%3Apr"));
    assert!(!command.matches("https://github.com/joris-gallot/guit/issues/23"));
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
  fn branch_git_command_describes_merge_and_rebase_commands() {
    let branch = CommandPaletteBranch {
      name: "origin/main".into(),
      kind: CommandPaletteBranchKind::Remote,
    };

    assert_eq!(
      CommandPalette::branch_git_command(CommandPaletteScreen::MergeBranch, &branch),
      Some("git merge origin/main".to_string())
    );
    assert_eq!(
      CommandPalette::branch_git_command(CommandPaletteScreen::RebaseBranch, &branch),
      Some("git rebase origin/main".to_string())
    );
    assert_eq!(
      CommandPalette::branch_git_command(CommandPaletteScreen::InteractiveRebaseBranch, &branch),
      Some("git rebase -i origin/main".to_string())
    );
    assert_eq!(
      CommandPalette::branch_git_command(
        CommandPaletteScreen::InteractiveRebaseEditBranch,
        &branch
      ),
      Some("git rebase -i --onto $(git merge-base HEAD origin/main) origin/main".to_string())
    );
  }

  #[test]
  fn branch_git_command_shell_quotes_unusual_branch_names() {
    let branch = CommandPaletteBranch {
      name: "feature/needs quote".into(),
      kind: CommandPaletteBranchKind::Local,
    };

    assert_eq!(
      CommandPalette::branch_git_command(CommandPaletteScreen::RebaseBranch, &branch),
      Some("git rebase 'feature/needs quote'".to_string())
    );
  }

  #[test]
  fn interactive_rebase_head_count_git_command_uses_valid_count_or_placeholder() {
    assert_eq!(
      CommandPalette::interactive_rebase_head_count_git_command(" 4 "),
      "git rebase -i HEAD~4"
    );
    assert_eq!(
      CommandPalette::interactive_rebase_head_count_git_command("1"),
      "git rebase -i HEAD~n"
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
      CommandPaletteCommand::open_pull_request(42).group(),
      CommandPaletteGroup::PullRequest
    );
    assert_eq!(
      CommandPaletteCommand::switch_repository().group(),
      CommandPaletteGroup::Repository
    );
    assert_eq!(
      CommandPaletteCommand::open_github_from_url().group(),
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
    // Ord is derived from declaration order, make the contract explicit.
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
  fn default_global_commands_include_github_commands_when_github_is_enabled() {
    let commands = CommandPaletteCommand::default_global_commands(
      super::CommandPalettePage::Git,
      /* include_github */ true,
    );
    assert!(
      commands
        .iter()
        .any(|c| c.id == CommandPaletteCommandId::OpenGithubFromUrl)
    );
  }

  #[test]
  fn default_global_commands_omit_github_commands_when_github_is_disabled() {
    let commands = CommandPaletteCommand::default_global_commands(
      super::CommandPalettePage::Git,
      /* include_github */ false,
    );
    assert!(
      !commands
        .iter()
        .any(|c| c.id == CommandPaletteCommandId::OpenGithubFromUrl)
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
  fn open_pull_request_command_is_available_with_expected_metadata() {
    let command = CommandPaletteCommand::open_pull_request(42);
    assert_eq!(command.id, CommandPaletteCommandId::OpenPullRequest);
    assert_eq!(command.name.as_ref(), "Open PR #42");
    assert!(command.matches("pull request for the current branch"));
  }

  #[test]
  fn disabled_command_keeps_reason_searchable() {
    let command = CommandPaletteCommand::interactive_rebase()
      .disabled("Commit or stash worktree changes first");
    assert!(command.is_disabled());
    assert!(command.matches("stash worktree"));
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
      "Edit commits since branch"
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
      CommandPaletteCommandId::OpenPullRequest,
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
      CommandPaletteCommandId::OpenRepository,
      CommandPaletteCommandId::OpenSessionPage,
      CommandPaletteCommandId::OpenGitPage,
      CommandPaletteCommandId::OpenGithubFromUrl,
      CommandPaletteCommandId::SwitchToPrBranch,
      CommandPaletteCommandId::CopyPrBranch,
      CommandPaletteCommandId::ToggleUnchangedFiles,
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
