use std::{rc::Rc, sync::Arc};

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

#[derive(Clone, Debug)]
pub enum CommandPaletteAction {
  SwitchBranch(CommandPaletteBranch),
  CreateBranch {
    name: String,
  },
  CreateBranchFrom {
    name: String,
    base: CommandPaletteBranch,
  },
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

    let base_item = list_base_item(ix, total_items, self.selected_index.clone(), &theme);

    self.matched_branches.get(ix.row).map(|branch| {
      base_item
        .child(
          h_flex()
            .items_center()
            .gap_2()
            .child(Icon::new(IconName::File))
            .child(Label::new(branch.name.clone())),
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

    let base_item = list_base_item(ix, total_items, self.selected_index.clone(), &theme);

    self
      .matched_branches_and_commands
      .get(ix.row)
      .map(|branch| match branch.as_ref() {
        BranchListWithCommands::CommandPaletteCommand(command) => base_item.child(
          h_flex()
            .items_center()
            .gap_2()
            .child(Icon::new(IconName::Plus))
            .child(Label::new(command.name.clone())),
        ),
        BranchListWithCommands::SwitchBranch(branch) => base_item
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(Icon::new(IconName::File))
              .child(Label::new(branch.name.clone())),
          )
          .suffix(|_, _| {
            Button::new("action")
              .ghost()
              .small()
              .icon(IconName::ArrowRight)
          }),
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
      list_base_item(ix, total_items, self.selected_index.clone(), &theme)
        .child(
          h_flex()
            .items_center()
            .gap_2()
            .child(Icon::new(IconName::File))
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
  SwitchBranch,
  CreateBranch,
  CreateBranchFrom,
  MergeBranch,
}

impl CommandPaletteCommand {
  pub fn switch_branch() -> Self {
    Self {
      id: CommandPaletteCommandId::SwitchBranch,
      name: "Switch branch".into(),
      description: Some("Checkout or create branches".into()),
    }
  }

  pub fn merge_branch() -> Self {
    Self {
      id: CommandPaletteCommandId::MergeBranch,
      name: "Merge branch".into(),
      description: Some("Merge a branch into the current branch".into()),
    }
  }

  fn matches(&self, query: &str) -> bool {
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
  pub commands: Vec<CommandPaletteCommand>,
  pub on_action: CommandPaletteHandler,
}

impl CommandPaletteConfig {
  pub fn new(branches: Vec<CommandPaletteBranch>, on_action: CommandPaletteHandler) -> Self {
    Self {
      branches,
      commands: vec![
        CommandPaletteCommand::switch_branch(),
        CommandPaletteCommand::merge_branch(),
      ],
      on_action,
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandPaletteScreen {
  Root,
  SwitchBranch,
  CreateBranch,
  CreateBranchFrom,
  MergeBranch,
}

pub struct CommandPalette {
  focus_handle: FocusHandle,
  screen: CommandPaletteScreen,
  commands_list: Entity<ListState<CommandListDelegate>>,
  branches_list: Entity<ListState<BranchesListDelegate>>,
  branches_with_commands_list: Entity<ListState<BranchesListWithCommandsDelegate>>,
  create_branch_input: Entity<InputState>,
  create_branch_base: Option<Rc<CommandPaletteBranch>>,
  error: Option<SharedString>,
  on_action: Option<CommandPaletteHandler>,
  _subscriptions: Vec<Subscription>,
}

impl CommandPalette {
  pub fn new(window: &mut Window, cx: &mut Context<Self>, config: CommandPaletteConfig) -> Self {
    let create_branch_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Enter branch name..."));

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

    let mut branches_with_actions: Vec<Rc<BranchListWithCommands>> = default_branches
      .into_iter()
      .map(|b| Rc::new(BranchListWithCommands::SwitchBranch(b)))
      .collect();

    branches_with_actions.insert(
      0,
      Rc::new(BranchListWithCommands::CommandPaletteCommand(
        CommandPaletteCommand {
          id: CommandPaletteCommandId::CreateBranch,
          name: "Create branch".into(),
          description: Some("Create a new branch".into()),
        },
      )),
    );
    branches_with_actions.insert(
      1,
      Rc::new(BranchListWithCommands::CommandPaletteCommand(
        CommandPaletteCommand {
          id: CommandPaletteCommandId::CreateBranchFrom,
          name: "Create branch from...".into(),
          description: Some("Create a new branch from an existing branch".into()),
        },
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
        |command_palette, list_state, ev: &ListEvent, window, cx| match ev {
          ListEvent::Confirm(ix) => {
            if let Some(command) = list_state.read(cx).delegate().matched_commands.get(ix.row) {
              command_palette.select_command(command.id, cx, window);
            }
          }
          _ => {}
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
            let branch = {
              let list = list_state.read(cx);
              list.delegate().matched_branches.get(ix.row).cloned()
            };

            command_palette.create_branch_base = branch.clone();

            command_palette.select_command(CommandPaletteCommandId::CreateBranch, cx, window);
          }
        },
      ),
      cx.subscribe_in(&create_branch_input, window, Self::on_input_event),
    ];

    cx.on_next_frame(window, |this, window, cx| {
      this.focus_screen_input(window, cx)
    });

    Self {
      focus_handle: cx.focus_handle(),
      create_branch_input,
      create_branch_base: None,
      screen: CommandPaletteScreen::Root,
      commands_list,
      branches_list,
      branches_with_commands_list,
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
    match event {
      InputEvent::PressEnter { secondary: _ } => {
        let branch_name = state.read(cx).value().to_string();
        if branch_name.is_empty() {
          self.error = Some("Branch name cannot be empty".into());
          cx.notify();
          return;
        }

        match self.screen {
          CommandPaletteScreen::CreateBranch => {
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
          _ => {}
        }
      }
      _ => {}
    };
  }

  pub fn focus_screen_input(&self, window: &mut Window, cx: &mut Context<Self>) {
    match self.screen {
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
      CommandPaletteScreen::CreateBranchFrom => {
        self.branches_list.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      }
      _ => {}
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
      CommandPaletteCommandId::SwitchBranch => {
        self.set_screen(CommandPaletteScreen::SwitchBranch, cx, window);
      }
      CommandPaletteCommandId::MergeBranch => {
        self.set_screen(CommandPaletteScreen::MergeBranch, cx, window);
      }
      CommandPaletteCommandId::CreateBranch => {
        self.set_screen(CommandPaletteScreen::CreateBranch, cx, window);
      }
      CommandPaletteCommandId::CreateBranchFrom => {
        self.set_screen(CommandPaletteScreen::CreateBranchFrom, cx, window);
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

  fn render_merge_branch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    div()
      .gap_3()
      .child("TODO: Merge branch UI")
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
      .gap_3()
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
      CommandPaletteScreen::SwitchBranch => self.render_switch_branch(cx).into_any_element(),
      CommandPaletteScreen::CreateBranch => self.render_create_branch(cx).into_any_element(),
      CommandPaletteScreen::CreateBranchFrom => {
        self.render_create_branch_from(cx).into_any_element()
      }
      CommandPaletteScreen::MergeBranch => self.render_merge_branch(cx).into_any_element(),
    };

    div()
      .track_focus(&self.focus_handle)
      .child(content)
      .h_full()
      .text_color(theme.foreground)
  }
}
