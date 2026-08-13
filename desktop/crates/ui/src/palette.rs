//! Shared shell for the palette dialogs: row geometry, keyboard-hint footer,
//! empty state, and the dialog chrome.

use gpui::{
  AnyElement, App, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div,
  prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, IndexPath, Sizable as _, Size, WindowExt as _, h_flex,
  list::{List, ListDelegate, ListItem, ListState},
  v_flex,
};

pub(crate) const PALETTE_WIDTH: f32 = 620.0;
pub(crate) const PALETTE_LIST_MAX_HEIGHT: f32 = 400.0;

pub(crate) fn palette_list_item(ix: IndexPath, selected_index: Option<IndexPath>) -> ListItem {
  ListItem::new(ix)
    .selected(Some(ix) == selected_index)
    .mx_1()
    .px_2()
    .py_1p5()
    .rounded_md()
}

pub(crate) fn palette_empty(cx: &App) -> AnyElement {
  v_flex()
    .items_center()
    .py_8()
    .text_sm()
    .text_color(cx.theme().muted_foreground)
    .child("No matching results")
    .into_any_element()
}

pub(crate) fn palette_search_list<D: ListDelegate>(
  list: &Entity<ListState<D>>,
  placeholder: &'static str,
) -> List<D> {
  List::new(list)
    .w_full()
    .max_h(px(PALETTE_LIST_MAX_HEIGHT))
    .with_size(Size::Large)
    .search_placeholder(placeholder)
}

pub(crate) fn palette_section_header(label: impl IntoElement, cx: &App) -> gpui::Div {
  div()
    .px_3()
    .pt_3()
    .pb_1()
    .text_xs()
    .text_color(cx.theme().muted_foreground)
    .child(label)
}

pub(crate) fn palette_footer(
  navigable: bool,
  enter_label: &'static str,
  cx: &App,
) -> impl IntoElement {
  let theme = cx.theme();
  let key = |label: &'static str| {
    div()
      .px_1()
      .rounded_sm()
      .bg(theme.muted)
      .text_color(theme.muted_foreground)
      .child(label)
  };

  h_flex()
    .px_3()
    .py_2()
    .gap_4()
    .border_t_1()
    .border_color(theme.border)
    .text_xs()
    .text_color(theme.muted_foreground)
    .when(navigable, |this| {
      this.child(
        h_flex()
          .gap_1()
          .child(key("↑"))
          .child(key("↓"))
          .child("navigate"),
      )
    })
    .child(h_flex().gap_1().child(key("↵")).child(enter_label))
    .child(h_flex().gap_1().child(key("esc")).child("close"))
}

pub(crate) fn update_selected_index<D: ListDelegate>(
  selected_index: &mut Option<IndexPath>,
  ix: Option<IndexPath>,
  cx: &mut Context<ListState<D>>,
) {
  *selected_index = ix;
  cx.notify();
}

pub fn open_palette_dialog<V: Render>(view: Entity<V>, window: &mut Window, cx: &mut App) {
  window.open_dialog(cx, move |dialog, _, _| {
    dialog
      .on_ok(|_, _, _| false)
      .w(px(PALETTE_WIDTH))
      .p_0()
      .gap_0()
      .min_h_0()
      .overlay_closable(true)
      .keyboard(true)
      .close_button(false)
      .child(view.clone())
  });
}
