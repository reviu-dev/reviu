use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
  AnyElement, App, AppContext as _, Context, Corner, Entity, Focusable, IntoElement, ParentElement,
  Pixels, RenderOnce, SharedString, Styled, Task, WeakEntity, Window, div, px, relative,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, IndexPath,
  button::{Button, ButtonCustomVariant, ButtonVariants as _},
  h_flex,
  list::{List, ListDelegate, ListItem, ListState},
  popover::{Popover, PopoverState},
};

type DropdownSelectHandler<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

pub trait DropdownSelectItem: Clone {
  type Value: Clone + PartialEq + 'static;

  fn value(&self) -> &Self::Value;

  fn selected(&self) -> bool;

  fn disabled(&self) -> bool {
    false
  }

  fn matches(&self, query: &str) -> bool;

  fn render_item(&self, window: &mut Window, cx: &mut App) -> AnyElement;

  fn render_selected(&self, window: &mut Window, cx: &mut App) -> AnyElement {
    self.render_item(window, cx)
  }
}

#[derive(Clone)]
pub struct DropdownSelectOption<T: Clone + 'static> {
  pub value: T,
  pub label: SharedString,
  pub prefix: Option<SharedString>,
  pub selected: bool,
  pub disabled: bool,
}

impl<T: Clone + 'static> DropdownSelectOption<T> {
  pub fn new(value: T, label: impl Into<SharedString>) -> Self {
    Self {
      value,
      label: label.into(),
      prefix: None,
      selected: false,
      disabled: false,
    }
  }

  pub fn prefix(mut self, prefix: impl Into<SharedString>) -> Self {
    self.prefix = Some(prefix.into());
    self
  }

  pub fn selected(mut self, selected: bool) -> Self {
    self.selected = selected;
    self
  }

  pub fn disabled(mut self, disabled: bool) -> Self {
    self.disabled = disabled;
    self
  }
}

impl<T: Clone + PartialEq + 'static> DropdownSelectItem for DropdownSelectOption<T> {
  type Value = T;

  fn value(&self) -> &Self::Value {
    &self.value
  }

  fn selected(&self) -> bool {
    self.selected
  }

  fn disabled(&self) -> bool {
    self.disabled
  }

  fn matches(&self, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
      return true;
    }

    let lowered_query = query.to_lowercase();
    if self.label.to_lowercase().contains(&lowered_query) {
      return true;
    }

    self
      .prefix
      .as_ref()
      .is_some_and(|prefix| prefix.to_lowercase().contains(&lowered_query))
  }

  fn render_item(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
    let prefix = self.prefix.clone().filter(|prefix| !prefix.is_empty());

    h_flex()
      .min_w_0()
      .flex_1()
      .items_center()
      .gap_0()
      .when_some(prefix, |this, prefix| {
        this.child(
          div()
            .min_w_0()
            .flex_1()
            .overflow_hidden()
            .text_ellipsis_start()
            .text_color(cx.theme().muted_foreground)
            .child(prefix),
        )
      })
      .child(
        div()
          .min_w_0()
          .overflow_hidden()
          .text_ellipsis()
          .child(self.label.clone()),
      )
      .into_any_element()
  }

  fn render_selected(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
    let prefix = self.prefix.clone().filter(|prefix| !prefix.is_empty());

    h_flex()
      .min_w_0()
      .items_center()
      .gap_0()
      .when_some(prefix, |this, prefix| {
        this.child(
          div()
            .min_w_0()
            .overflow_hidden()
            .text_ellipsis_start()
            .text_color(cx.theme().muted_foreground)
            .text_sm()
            .child(prefix),
        )
      })
      .child(div().flex_shrink_0().text_sm().child(self.label.clone()))
      .into_any_element()
  }
}

pub struct DropdownSelectConfig<I: DropdownSelectItem + 'static> {
  pub id: SharedString,
  pub placeholder: SharedString,
  pub trigger_label: Option<SharedString>,
  pub trigger_height: Option<Pixels>,
  pub search_placeholder: SharedString,
  pub options: Vec<I>,
  pub disabled: bool,
  pub searchable: bool,
  pub width: Pixels,
  pub menu_width: Pixels,
  pub anchor: Corner,
  pub on_select: Option<DropdownSelectHandler<I::Value>>,
}

impl<I: DropdownSelectItem + 'static> DropdownSelectConfig<I> {
  pub fn new(id: impl Into<SharedString>) -> Self {
    Self {
      id: id.into(),
      placeholder: "Select...".into(),
      trigger_label: None,
      trigger_height: None,
      search_placeholder: "Search...".into(),
      options: Vec::new(),
      disabled: false,
      searchable: true,
      width: px(240.),
      menu_width: px(240.),
      anchor: Corner::TopLeft,
      on_select: None,
    }
  }

  pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
    self.placeholder = placeholder.into();
    self
  }

  pub fn trigger_label(mut self, trigger_label: impl Into<SharedString>) -> Self {
    self.trigger_label = Some(trigger_label.into());
    self
  }

  pub fn trigger_height(mut self, trigger_height: Pixels) -> Self {
    self.trigger_height = Some(trigger_height);
    self
  }

  pub fn search_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
    self.search_placeholder = placeholder.into();
    self
  }

  pub fn options(mut self, options: Vec<I>) -> Self {
    self.options = options;
    self
  }

  pub fn disabled(mut self, disabled: bool) -> Self {
    self.disabled = disabled;
    self
  }

  pub fn searchable(mut self, searchable: bool) -> Self {
    self.searchable = searchable;
    self
  }

  pub fn width(mut self, width: Pixels) -> Self {
    self.width = width;
    self
  }

  pub fn menu_width(mut self, width: Pixels) -> Self {
    self.menu_width = width;
    self
  }

  pub fn anchor(mut self, anchor: Corner) -> Self {
    self.anchor = anchor;
    self
  }

  pub fn on_select(mut self, handler: DropdownSelectHandler<I::Value>) -> Self {
    self.on_select = Some(handler);
    self
  }
}

#[derive(IntoElement)]
pub struct DropdownSelect<I: DropdownSelectItem + 'static> {
  config: DropdownSelectConfig<I>,
}

pub fn dropdown_select<I: DropdownSelectItem + 'static>(
  config: DropdownSelectConfig<I>,
) -> DropdownSelect<I> {
  DropdownSelect { config }
}

struct DropdownSelectRuntime<I: DropdownSelectItem + 'static> {
  list: Option<Entity<ListState<DropdownSelectListDelegate<I>>>>,
}

impl<I: DropdownSelectItem + 'static> Default for DropdownSelectRuntime<I> {
  fn default() -> Self {
    Self { list: None }
  }
}

struct DropdownSelectListDelegate<I: DropdownSelectItem + 'static> {
  options: Vec<I>,
  filtered_options: Vec<I>,
  selected_index: Option<IndexPath>,
  search_query: String,
  on_select: Option<DropdownSelectHandler<I::Value>>,
  popover: Option<WeakEntity<PopoverState>>,
}

impl<I: DropdownSelectItem + 'static> DropdownSelectListDelegate<I> {
  fn new(options: Vec<I>, on_select: Option<DropdownSelectHandler<I::Value>>) -> Self {
    let filtered_options = Self::filter_options(&options, "");
    let selected_index = Self::selected_index_for(&filtered_options);
    Self {
      filtered_options,
      options,
      selected_index,
      search_query: String::new(),
      on_select,
      popover: None,
    }
  }

  fn set_options(&mut self, options: Vec<I>, on_select: Option<DropdownSelectHandler<I::Value>>) {
    let previous_selected_value = self.selected_value();
    self.options = options;
    self.on_select = on_select;
    let query = self.search_query.clone();
    self.apply_search_query_with_preferred_selection(&query, previous_selected_value.as_ref());
  }

  fn set_popover(&mut self, popover: WeakEntity<PopoverState>) {
    self.popover = Some(popover);
  }

  fn filter_options(options: &[I], query: &str) -> Vec<I> {
    let query = query.trim();
    options
      .iter()
      .filter(|option| option.matches(query))
      .cloned()
      .collect()
  }

  fn apply_search_query(&mut self, query: &str) {
    let previous_selected_value = self.selected_value();
    self.apply_search_query_with_preferred_selection(query, previous_selected_value.as_ref());
  }

  fn apply_search_query_with_preferred_selection(
    &mut self,
    query: &str,
    preferred: Option<&I::Value>,
  ) {
    self.search_query = query.trim().to_string();
    self.filtered_options = Self::filter_options(&self.options, &self.search_query);
    self.selected_index = preferred
      .and_then(|value| Self::index_for_value(&self.filtered_options, value))
      .or_else(|| Self::selected_index_for(&self.filtered_options));
  }

  fn selected_index_for(options: &[I]) -> Option<IndexPath> {
    options
      .iter()
      .position(|option| option.selected())
      .map(IndexPath::new)
  }

  fn index_for_value(options: &[I], value: &I::Value) -> Option<IndexPath> {
    options
      .iter()
      .position(|option| option.value() == value)
      .map(IndexPath::new)
  }

  fn selected_value(&self) -> Option<I::Value> {
    self
      .selected_index
      .and_then(|ix| self.filtered_options.get(ix.row))
      .map(|option| option.value().clone())
  }

  fn dismiss_popover(&self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
    let Some(popover) = self.popover.as_ref().and_then(|popover| popover.upgrade()) else {
      return;
    };

    let _ = popover.update(cx, |popover, cx| {
      popover.dismiss(window, cx);
    });
  }
}

impl<I: DropdownSelectItem + 'static> ListDelegate for DropdownSelectListDelegate<I> {
  type Item = ListItem;

  fn perform_search(
    &mut self,
    query: &str,
    _: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Task<()> {
    self.apply_search_query(query);
    cx.notify();
    Task::ready(())
  }

  fn items_count(&self, _: usize, _: &App) -> usize {
    self.filtered_options.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let option = self.filtered_options.get(ix.row)?.clone();

    Some(
      ListItem::new(ix.row)
        .selected(self.selected_index == Some(ix))
        .disabled(option.disabled())
        .check_icon(IconName::Check)
        .confirmed(option.selected())
        .child(option.render_item(window, cx)),
    )
  }

  fn render_empty(
    &mut self,
    _: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    h_flex()
      .w_full()
      .justify_center()
      .py_6()
      .text_color(cx.theme().muted_foreground)
      .child("No options")
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    self.selected_index = ix;
    cx.notify();
  }

  fn confirm(&mut self, _: bool, window: &mut Window, cx: &mut Context<ListState<Self>>) {
    if let Some(ix) = self.selected_index {
      if let Some(option) = self.filtered_options.get(ix.row).cloned() {
        if !option.disabled() {
          if let Some(handler) = self.on_select.clone() {
            handler(option.value().clone(), window, cx);
          }
        }
      }
    }

    self.dismiss_popover(window, cx);
  }

  fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
    self.dismiss_popover(window, cx);
  }
}

impl<I: DropdownSelectItem + 'static> RenderOnce for DropdownSelect<I> {
  fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
    let runtime = window.use_keyed_state(
      SharedString::from(format!("dropdown-select-runtime:{}", self.config.id)),
      cx,
      |_, _| DropdownSelectRuntime::<I>::default(),
    );

    let theme = cx.theme().clone();
    let selected_option = self
      .config
      .options
      .iter()
      .find(|option| option.selected())
      .cloned();
    let selected_title = selected_option
      .as_ref()
      .map(|option| option.render_selected(window, cx));
    let has_selected_title = selected_title.is_some();
    let options = self.config.options.clone();
    let on_select = self.config.on_select.clone();
    let searchable = self.config.searchable;
    let menu_width = self.config.menu_width;
    let anchor = self.config.anchor;
    let trigger_label = self.config.trigger_label.clone();
    let trigger_height = self.config.trigger_height;
    let search_placeholder = self.config.search_placeholder.clone();
    let placeholder = self.config.placeholder.clone();

    let trigger = Button::new(self.config.id.clone())
      .custom(
        ButtonCustomVariant::new(cx)
          .color(theme.background)
          .foreground(theme.foreground)
          .border(theme.transparent)
          .hover(theme.secondary_hover)
          .active(theme.secondary),
      )
      // .compact()
      .rounded_none()
      .border_0()
      .w_full()
      .when_some(trigger_height, |this, height| this.h(height))
      .when(trigger_height.is_none(), |this| this.h_full())
      .disabled(self.config.disabled)
      .child(
        h_flex()
          .w_full()
          .min_w_0()
          .items_center()
          .justify_between()
          .gap_2()
          .child(
            div()
              .min_w_0()
              .flex_1()
              .overflow_hidden()
              .flex()
              .flex_col()
              .justify_center()
              .when_some(trigger_label.clone(), |this, label| {
                this.child(
                  div()
                    .text_xs()
                    .line_height(relative(1.0))
                    .text_color(theme.muted_foreground)
                    .child(label),
                )
              })
              .child(
                div()
                  .min_w_0()
                  .text_sm()
                  .overflow_hidden()
                  .when_some(selected_title, |this, title| this.child(title))
                  .when(!has_selected_title, |this| {
                    this.text_color(theme.muted_foreground).child(placeholder)
                  }),
              ),
          )
          .child(
            Icon::new(IconName::ChevronDown)
              .size_3()
              .text_color(theme.muted_foreground),
          ),
      );

    if self.config.disabled {
      return div().h_full().w(self.config.width).child(trigger);
    }

    let options_for_state = options.clone();
    let on_select_for_state = on_select.clone();
    let list_state: Entity<ListState<DropdownSelectListDelegate<I>>> =
      runtime.update(cx, |runtime: &mut DropdownSelectRuntime<I>, cx| {
        if runtime.list.is_none() {
          runtime.list = Some(cx.new(|cx| {
            ListState::new(
              DropdownSelectListDelegate::new(
                options_for_state.clone(),
                on_select_for_state.clone(),
              ),
              window,
              cx,
            )
            .searchable(searchable)
          }));
        }

        runtime.list.clone().expect("dropdown list state")
      });

    let options_for_delegate = options.clone();
    let on_select_for_delegate = on_select.clone();
    let _ = list_state.update(
      cx,
      |state: &mut ListState<DropdownSelectListDelegate<I>>, cx| {
        state.set_searchable(searchable, cx);
        let selected_index = {
          let delegate = state.delegate_mut();
          delegate.set_options(options_for_delegate, on_select_for_delegate);
          delegate.selected_index
        };
        state.set_selected_index(selected_index, window, cx);
      },
    );

    let list_focus = list_state.read(cx).focus_handle(cx);
    let popover = Popover::new(SharedString::from(format!(
      "dropdown-select-popover:{}",
      self.config.id
    )))
    .anchor(anchor)
    .appearance(false)
    .overlay_closable(true)
    .track_focus(&list_focus)
    .trigger(trigger)
    .content(move |_, window, cx| {
      let popover_weak = cx.entity().downgrade();
      let _ = list_state.update(
        cx,
        |state: &mut ListState<DropdownSelectListDelegate<I>>, cx| {
          state.delegate_mut().set_popover(popover_weak);
          state.focus(window, cx);
        },
      );

      div()
        .w(menu_width)
        .bg(cx.theme().background)
        .border_1()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius)
        .shadow_md()
        .overflow_hidden()
        .child(
          List::new(&list_state)
            .search_placeholder(search_placeholder.clone())
            .max_h(px(320.)),
        )
    });

    div().h_full().w(self.config.width).child(popover)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn apply_search_query_filters_label_and_prefix() {
    let options = vec![
      DropdownSelectOption::new("a", "reviu").prefix("/Users/joris/workspace/"),
      DropdownSelectOption::new("b", "git-playground").prefix("/Users/joris/workspace/"),
      DropdownSelectOption::new("c", "other").prefix("/tmp/"),
    ];

    let mut delegate = DropdownSelectListDelegate::new(options, None);
    delegate.apply_search_query("play");

    assert_eq!(delegate.filtered_options.len(), 1);
    assert_eq!(
      delegate.filtered_options[0].label.as_ref(),
      "git-playground"
    );

    delegate.apply_search_query("workspace");
    assert_eq!(delegate.filtered_options.len(), 2);
  }

  #[test]
  fn set_options_reapplies_active_search_query() {
    let options = vec![
      DropdownSelectOption::new("repo-a", "reviu").selected(true),
      DropdownSelectOption::new("repo-b", "git-playground"),
    ];

    let mut delegate = DropdownSelectListDelegate::new(options.clone(), None);
    delegate.apply_search_query("play");
    assert_eq!(delegate.filtered_options.len(), 1);
    assert_eq!(
      delegate.filtered_options[0].label.as_ref(),
      "git-playground"
    );

    delegate.set_options(options, None);
    assert_eq!(delegate.filtered_options.len(), 1);
    assert_eq!(
      delegate.filtered_options[0].label.as_ref(),
      "git-playground"
    );
    assert_eq!(delegate.selected_index, None);
  }

  #[test]
  fn set_options_preserves_active_row_selection() {
    let options = vec![
      DropdownSelectOption::new("repo-a", "reviu").selected(true),
      DropdownSelectOption::new("repo-b", "git-playground"),
      DropdownSelectOption::new("repo-c", "zed"),
    ];

    let mut delegate = DropdownSelectListDelegate::new(options.clone(), None);
    delegate.selected_index = Some(IndexPath::new(1));

    delegate.set_options(options, None);

    assert_eq!(delegate.selected_index, Some(IndexPath::new(1)));
    assert_eq!(delegate.selected_value(), Some("repo-b"));
  }
}
