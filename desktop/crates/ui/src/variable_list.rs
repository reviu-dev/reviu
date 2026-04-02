use std::{ops::Range, rc::Rc};

use gpui::{
  Action, AnyElement, App, Context, DefiniteLength, EdgesRefinement, Entity, EventEmitter,
  FocusHandle, Focusable, IntoElement, KeyBinding, Length, ListSizingBehavior, Pixels, Render,
  RenderOnce, ScrollStrategy, SharedString, StyleRefinement, Styled, Task, Window, actions, div,
  prelude::*,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, Selectable, Sizable as _, Size as ComponentSize,
  StyledExt as _, VirtualListScrollHandle, h_flex,
  input::{InputEvent, InputState},
  scroll::Scrollbar,
  spinner::Spinner,
  v_flex, v_virtual_list,
};
use serde::Deserialize;

use crate::Input;

const VARIABLE_LIST_CONTEXT: &str = "VariableList";

#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = ui_variable_list, no_json)]
struct VariableListConfirm {
  secondary: bool,
}

actions!(
  ui_variable_list,
  [
    VariableListCancel,
    VariableListSelectDown,
    VariableListSelectUp
  ]
);

pub(crate) fn init(cx: &mut App) {
  cx.bind_keys([
    KeyBinding::new("escape", VariableListCancel, Some(VARIABLE_LIST_CONTEXT)),
    KeyBinding::new(
      "enter",
      VariableListConfirm { secondary: false },
      Some(VARIABLE_LIST_CONTEXT),
    ),
    KeyBinding::new(
      "secondary-enter",
      VariableListConfirm { secondary: true },
      Some(VARIABLE_LIST_CONTEXT),
    ),
    KeyBinding::new("up", VariableListSelectUp, Some(VARIABLE_LIST_CONTEXT)),
    KeyBinding::new("down", VariableListSelectDown, Some(VARIABLE_LIST_CONTEXT)),
  ]);
}

#[derive(Clone)]
pub enum VariableListEvent {
  Select(usize),
  Confirm(usize),
  Cancel,
}

struct VariableListOptions {
  size: ComponentSize,
  scrollbar_visible: bool,
  search_placeholder: Option<SharedString>,
  max_height: Option<Length>,
  paddings: EdgesRefinement<DefiniteLength>,
}

impl Default for VariableListOptions {
  fn default() -> Self {
    Self {
      size: ComponentSize::default(),
      scrollbar_visible: true,
      search_placeholder: None,
      max_height: None,
      paddings: EdgesRefinement::default(),
    }
  }
}

pub trait VariableListDelegate: Sized + 'static {
  type Item: Selectable + IntoElement;

  fn perform_search(
    &mut self,
    _query: &str,
    _window: &mut Window,
    _cx: &mut Context<VariableListState<Self>>,
  ) -> Task<()> {
    Task::ready(())
  }

  fn items_count(&self, cx: &App) -> usize;

  fn item_size(&self, ix: usize, cx: &App) -> gpui::Size<Pixels>;

  fn render_item(
    &mut self,
    ix: usize,
    window: &mut Window,
    cx: &mut Context<VariableListState<Self>>,
  ) -> Option<Self::Item>;

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<VariableListState<Self>>,
  ) -> impl IntoElement {
    h_flex()
      .size_full()
      .justify_center()
      .text_color(cx.theme().muted_foreground.opacity(0.6))
      .child(Icon::new(IconName::Inbox).size_12())
      .into_any_element()
  }

  fn render_initial(
    &mut self,
    _window: &mut Window,
    _cx: &mut Context<VariableListState<Self>>,
  ) -> Option<AnyElement> {
    None
  }

  fn loading(&self, _cx: &App) -> bool {
    false
  }

  fn render_loading(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<VariableListState<Self>>,
  ) -> impl IntoElement {
    v_flex()
      .size_full()
      .items_center()
      .justify_center()
      .child(Spinner::new().small().color(cx.theme().muted_foreground))
  }

  fn set_selected_index(
    &mut self,
    ix: Option<usize>,
    window: &mut Window,
    cx: &mut Context<VariableListState<Self>>,
  );

  fn confirm(
    &mut self,
    _secondary: bool,
    _window: &mut Window,
    _cx: &mut Context<VariableListState<Self>>,
  ) {
  }

  fn cancel(&mut self, _window: &mut Window, _cx: &mut Context<VariableListState<Self>>) {}
}

pub struct VariableListState<D: VariableListDelegate> {
  focus_handle: FocusHandle,
  query_input: Entity<InputState>,
  options: VariableListOptions,
  delegate: D,
  last_query: Option<String>,
  scroll_handle: VirtualListScrollHandle,
  selected_index: Option<usize>,
  deferred_scroll_to_index: Option<(usize, ScrollStrategy)>,
  reset_on_cancel: bool,
  searchable: bool,
  selectable: bool,
  _search_task: Task<()>,
  _query_input_subscription: gpui::Subscription,
}

impl<D> VariableListState<D>
where
  D: VariableListDelegate,
{
  pub fn new(delegate: D, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let query_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
    let _query_input_subscription =
      cx.subscribe_in(&query_input, window, Self::on_query_input_event);

    Self {
      focus_handle: cx.focus_handle(),
      query_input,
      options: VariableListOptions::default(),
      delegate,
      last_query: None,
      scroll_handle: VirtualListScrollHandle::new(),
      selected_index: None,
      deferred_scroll_to_index: None,
      reset_on_cancel: true,
      searchable: false,
      selectable: true,
      _search_task: Task::ready(()),
      _query_input_subscription,
    }
  }

  pub fn searchable(mut self, searchable: bool) -> Self {
    self.searchable = searchable;
    self
  }

  pub fn delegate(&self) -> &D {
    &self.delegate
  }

  pub fn delegate_mut(&mut self) -> &mut D {
    &mut self.delegate
  }

  pub fn focus(&mut self, window: &mut Window, cx: &mut App) {
    self.focus_handle(cx).focus(window, cx);
  }

  fn items_count(&self, cx: &App) -> usize {
    self.delegate.items_count(cx)
  }

  fn set_searching(&mut self, searching: bool, window: &mut Window, cx: &mut Context<Self>) {
    self
      .query_input
      .update(cx, |input, cx| input.set_loading(searching, window, cx));
  }

  fn set_selected_index_internal(
    &mut self,
    ix: Option<usize>,
    window: &mut Window,
    cx: &mut Context<Self>,
    scroll: bool,
  ) {
    if !self.selectable {
      return;
    }

    self.selected_index = ix;
    self.delegate.set_selected_index(ix, window, cx);
    if scroll {
      self.scroll_to_selected_item(cx);
    }
  }

  fn scroll_to_selected_item(&mut self, cx: &mut Context<Self>) {
    if let Some(ix) = self.selected_index {
      self.deferred_scroll_to_index = Some((ix, ScrollStrategy::Top));
      cx.notify();
    }
  }

  fn sync_selected_index(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let items_count = self.items_count(cx);
    let next_selected_index = match (self.selected_index, items_count) {
      (_, 0) => None,
      (Some(ix), count) if ix < count => Some(ix),
      (Some(_), count) => Some(count.saturating_sub(1)),
      (None, _) => None,
    };

    if next_selected_index != self.selected_index {
      self.set_selected_index_internal(next_selected_index, window, cx, false);
    }
  }

  fn on_query_input_event(
    &mut self,
    state: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match event {
      InputEvent::Change => {
        let text = state.read(cx).value();
        let text = text.trim().to_string();
        if Some(&text) == self.last_query.as_ref() {
          return;
        }

        self.set_searching(true, window, cx);
        let search = self.delegate.perform_search(&text, window, cx);
        let next_selected_index = if self.items_count(cx) > 0 {
          Some(0)
        } else {
          None
        };
        self.set_selected_index_internal(next_selected_index, window, cx, false);

        self._search_task = cx.spawn_in(window, async move |this, window| {
          search.await;

          let _ = this.update_in(window, |this, window, cx| {
            this.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
            this.last_query = Some(text);
            this.sync_selected_index(window, cx);
            this.set_searching(false, window, cx);
          });
        });
      }
      _ => {}
    }
  }

  fn on_action_cancel(
    &mut self,
    _: &VariableListCancel,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    cx.propagate();
    if self.reset_on_cancel {
      self.set_selected_index_internal(None, window, cx, false);
    }

    self.delegate.cancel(window, cx);
    cx.emit(VariableListEvent::Cancel);
    cx.notify();
  }

  fn on_action_confirm(
    &mut self,
    action: &VariableListConfirm,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.items_count(cx) == 0 {
      return;
    }

    let Some(ix) = self.selected_index else {
      return;
    };

    self
      .delegate
      .set_selected_index(self.selected_index, window, cx);
    self.delegate.confirm(action.secondary, window, cx);
    cx.emit(VariableListEvent::Confirm(ix));
    cx.notify();
  }

  fn select_item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
    self.set_selected_index_internal(Some(ix), window, cx, true);
    cx.emit(VariableListEvent::Select(ix));
    cx.notify();
  }

  fn previous_index(&self, items_count: usize) -> usize {
    match self.selected_index {
      Some(0) | None => items_count.saturating_sub(1),
      Some(ix) => ix.saturating_sub(1),
    }
  }

  fn next_index(&self, items_count: usize) -> usize {
    match self.selected_index {
      None => 0,
      Some(ix) if ix + 1 >= items_count => 0,
      Some(ix) => ix + 1,
    }
  }

  fn on_action_select_prev(
    &mut self,
    _: &VariableListSelectUp,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let items_count = self.items_count(cx);
    if items_count == 0 {
      return;
    }

    self.select_item(self.previous_index(items_count), window, cx);
  }

  fn on_action_select_next(
    &mut self,
    _: &VariableListSelectDown,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let items_count = self.items_count(cx);
    if items_count == 0 {
      return;
    }

    self.select_item(self.next_index(items_count), window, cx);
  }

  fn render_list_item(
    &mut self,
    ix: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let selectable = self.selectable;
    let selected = self.selected_index == Some(ix);
    let id = SharedString::from(format!("variable-list-item-{ix}"));

    div()
      .id(id)
      .w_full()
      .relative()
      .overflow_hidden()
      .children(
        self
          .delegate
          .render_item(ix, window, cx)
          .map(|item| item.selected(selected)),
      )
      .when(selectable, |this| {
        this.on_click(
          cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
            this.selected_index = Some(ix);
            this.on_action_confirm(
              &VariableListConfirm {
                secondary: event.modifiers().secondary(),
              },
              window,
              cx,
            );
          }),
        )
      })
  }

  fn render_items(
    &mut self,
    items_count: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let item_sizes = Rc::new(
      (0..items_count)
        .map(|ix| self.delegate.item_size(ix, cx))
        .collect::<Vec<_>>(),
    );
    let scrollbar_visible = self.options.scrollbar_visible;
    let scroll_handle = self.scroll_handle.clone();

    v_flex()
      .flex_grow()
      .relative()
      .size_full()
      .when_some(self.options.max_height, |this, max_height| {
        this.max_h(max_height)
      })
      .overflow_hidden()
      .when(items_count == 0, |this| {
        this.child(self.delegate.render_empty(window, cx))
      })
      .when(items_count > 0, |this| {
        this.child(
          v_virtual_list(
            cx.entity(),
            "variable-list-virtual-list",
            item_sizes,
            move |list, visible_range: Range<usize>, window, cx| {
              visible_range
                .map(|ix| list.render_list_item(ix, window, cx).into_any_element())
                .collect::<Vec<_>>()
            },
          )
          .paddings(self.options.paddings.clone())
          .when(self.options.max_height.is_some(), |this| {
            this.with_sizing_behavior(ListSizingBehavior::Infer)
          })
          .track_scroll(&scroll_handle)
          .into_any_element(),
        )
      })
      .when(scrollbar_visible, |this| {
        this.child(Scrollbar::vertical(&scroll_handle))
      })
  }
}

impl<D> Focusable for VariableListState<D>
where
  D: VariableListDelegate,
{
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    if self.searchable {
      self.query_input.focus_handle(cx)
    } else {
      self.focus_handle.clone()
    }
  }
}

impl<D> EventEmitter<VariableListEvent> for VariableListState<D> where D: VariableListDelegate {}

impl<D> Render for VariableListState<D>
where
  D: VariableListDelegate,
{
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.sync_selected_index(window, cx);

    if let Some((ix, strategy)) = self.deferred_scroll_to_index.take() {
      self.scroll_handle.scroll_to_item(ix, strategy);
    }

    let loading = self.delegate.loading(cx);
    let query_input = if self.searchable {
      if let Some(placeholder) = &self.options.search_placeholder {
        self.query_input.update(cx, |input, cx| {
          input.set_placeholder(placeholder.clone(), window, cx);
        });
      }
      Some(self.query_input.clone())
    } else {
      None
    };

    let loading_view = if loading {
      Some(self.delegate.render_loading(window, cx).into_any_element())
    } else {
      None
    };
    let initial_view = if let Some(input) = &query_input {
      if input.read(cx).value().is_empty() {
        self.delegate.render_initial(window, cx)
      } else {
        None
      }
    } else {
      self.delegate.render_initial(window, cx)
    };
    let items_count = self.items_count(cx);

    v_flex()
      .key_context(VARIABLE_LIST_CONTEXT)
      .id("variable-list-state")
      .track_focus(&self.focus_handle)
      .size_full()
      .relative()
      .overflow_hidden()
      .when_some(query_input, |this, input| {
        this.child(
          div()
            .map(|this| match self.options.size {
              ComponentSize::Small => this.px_1p5(),
              _ => this.px_2(),
            })
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
              Input::new(&input)
                .with_size(self.options.size)
                .prefix(Icon::new(IconName::Search).text_color(cx.theme().muted_foreground))
                .cleanable(true)
                .p_0()
                .appearance(false),
            ),
        )
      })
      .when(!loading, |this| {
        this
          .on_action(cx.listener(Self::on_action_cancel))
          .on_action(cx.listener(Self::on_action_confirm))
          .on_action(cx.listener(Self::on_action_select_next))
          .on_action(cx.listener(Self::on_action_select_prev))
          .map(|this| {
            if let Some(view) = initial_view {
              this.child(view)
            } else {
              this.child(self.render_items(items_count, window, cx))
            }
          })
      })
      .children(loading_view)
  }
}

#[derive(IntoElement)]
pub struct VariableList<D: VariableListDelegate + 'static> {
  state: Entity<VariableListState<D>>,
  style: StyleRefinement,
  options: VariableListOptions,
}

impl<D> VariableList<D>
where
  D: VariableListDelegate + 'static,
{
  pub fn new(state: &Entity<VariableListState<D>>) -> Self {
    Self {
      state: state.clone(),
      style: StyleRefinement::default(),
      options: VariableListOptions::default(),
    }
  }

  pub fn scrollbar_visible(mut self, visible: bool) -> Self {
    self.options.scrollbar_visible = visible;
    self
  }

  pub fn search_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
    self.options.search_placeholder = Some(placeholder.into());
    self
  }
}

impl<D> Styled for VariableList<D>
where
  D: VariableListDelegate + 'static,
{
  fn style(&mut self) -> &mut StyleRefinement {
    &mut self.style
  }
}

impl<D> gpui_component::Sizable for VariableList<D>
where
  D: VariableListDelegate + 'static,
{
  fn with_size(mut self, size: impl Into<ComponentSize>) -> Self {
    self.options.size = size.into();
    self
  }
}

impl<D> RenderOnce for VariableList<D>
where
  D: VariableListDelegate + 'static,
{
  fn render(mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
    self.options.paddings = self.style.padding.clone();
    self.options.max_height = self.style.max_size.height;
    self.style.padding = EdgesRefinement::default();
    self.style.max_size.height = None;

    self.state.update(cx, |state, _| {
      state.options = self.options;
    });

    div()
      .id("variable-list")
      .size_full()
      .refine_style(&self.style)
      .child(self.state.clone())
  }
}
