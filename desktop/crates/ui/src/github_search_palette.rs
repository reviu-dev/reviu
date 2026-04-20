use std::{rc::Rc, sync::Arc, time::Duration};

use crate::{SelectableRowStyle, UiIconName, selectable_list_item};
use gpui::{
  AnyElement, App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable, IntoElement,
  ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _, WindowExt,
  avatar::Avatar,
  h_flex,
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  v_flex,
};

const LIST_INPUT_HEIGHT: f32 = 35.0;
const LIST_ITEM_HEIGHT: f32 = 32.0;
const SEARCH_DEBOUNCE_MS: u64 = 280;
const PLACEHOLDER_ROW_SPAN: usize = 4;

#[derive(Clone, Debug)]
pub struct GithubSearchRepoEntry {
  pub owner: SharedString,
  pub repo: SharedString,
  pub full_name: SharedString,
  pub description: Option<SharedString>,
  pub stars: u64,
  pub private: bool,
  pub owner_avatar_url: Option<SharedString>,
}

pub type GithubRepoSearchFn =
  Arc<dyn Fn(String) -> anyhow::Result<Vec<GithubSearchRepoEntry>> + Send + Sync>;

pub type GithubRepoSelectFn = Arc<
  dyn Fn(GithubSearchRepoEntry, &mut Window, &mut App) -> Result<(), SharedString> + Send + Sync,
>;

pub struct GithubSearchPaletteConfig {
  pub search_fn: GithubRepoSearchFn,
  pub on_select: GithubRepoSelectFn,
}

impl GithubSearchPaletteConfig {
  pub fn new(search_fn: GithubRepoSearchFn, on_select: GithubRepoSelectFn) -> Self {
    Self {
      search_fn,
      on_select,
    }
  }
}

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

struct GithubSearchListDelegate {
  matched_repositories: Vec<Rc<GithubSearchRepoEntry>>,
  selected_index: Option<IndexPath>,
  query: SharedString,
  search_fn: GithubRepoSearchFn,
  generation: u64,
  loading: bool,
}

impl ListDelegate for GithubSearchListDelegate {
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

    self.matched_repositories.get(ix.row).map(|entry| {
      let primary: SharedString = entry.full_name.clone();
      let description = entry.description.clone();
      let stars_label: Option<SharedString> = if entry.stars > 0 {
        Some(format_stars(entry.stars).into())
      } else {
        None
      };
      let avatar = Avatar::new()
        .name(entry.full_name.clone())
        .when_some(entry.owner_avatar_url.clone(), |this, url| this.src(url))
        .xsmall();

      base_item.child(
        h_flex()
          .items_center()
          .gap_2()
          .w_full()
          .child(avatar)
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .min_w_0()
              .flex_shrink()
              .child(Label::new(primary).truncate())
              .when(entry.private, |this| {
                this.child(
                  Icon::new(UiIconName::Lock)
                    .size_3()
                    .text_color(theme.muted_foreground),
                )
              }),
          )
          .when_some(description, |this, description| {
            this.child(
              div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(description),
            )
          })
          .when_some(stars_label, |this, stars| {
            this.child(
              h_flex()
                .items_center()
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(Icon::new(UiIconName::Star).size_3())
                .child(stars),
            )
          }),
      )
    })
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    self.selected_index = ix;
    cx.notify();
  }

  fn loading(&self, _cx: &App) -> bool {
    self.loading
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    render_placeholder(cx, IconName::Search, "No repositories match your search.")
  }

  fn render_initial(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<AnyElement> {
    Some(
      render_placeholder(cx, IconName::Search, "Type to search GitHub repositories.")
        .into_any_element(),
    )
  }

  fn perform_search(
    &mut self,
    query: &str,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Task<()> {
    self.query = query.to_string().into();
    self.generation = self.generation.wrapping_add(1);
    let generation = self.generation;
    let trimmed = query.trim().to_string();

    if trimmed.is_empty() {
      self.matched_repositories.clear();
      self.selected_index = None;
      self.loading = false;
      return Task::ready(());
    }

    self.loading = self.matched_repositories.is_empty();

    let search_fn = self.search_fn.clone();
    cx.spawn(async move |list, cx| {
      cx.background_executor()
        .timer(Duration::from_millis(SEARCH_DEBOUNCE_MS))
        .await;

      let stale = list
        .read_with(cx, |list, _| list.delegate().generation != generation)
        .unwrap_or(true);
      if stale {
        return;
      }

      let query_for_search = trimmed.clone();
      let search_task = cx.background_spawn(async move { search_fn(query_for_search) });
      let result = search_task.await;

      let _ = list.update(cx, |list, cx| {
        let delegate = list.delegate_mut();
        if delegate.generation != generation {
          return;
        }
        delegate.loading = false;
        match result {
          Ok(repositories) => {
            delegate.matched_repositories = repositories.into_iter().map(Rc::new).collect();
            delegate.selected_index = if delegate.matched_repositories.is_empty() {
              None
            } else {
              Some(IndexPath::default())
            };
          }
          Err(_) => {
            delegate.matched_repositories.clear();
            delegate.selected_index = None;
          }
        }
        cx.notify();
      });
    })
  }
}

fn render_placeholder(cx: &App, icon: IconName, message: &'static str) -> impl IntoElement {
  v_flex()
    .size_full()
    .items_center()
    .justify_center()
    .gap_2()
    .p_6()
    .text_color(cx.theme().muted_foreground)
    .child(Icon::new(icon).size_6())
    .child(Label::new(message))
}

fn format_stars(stars: u64) -> String {
  if stars >= 1_000_000 {
    format!("{:.1}M", stars as f64 / 1_000_000.0)
  } else if stars >= 1_000 {
    format!("{:.1}k", stars as f64 / 1_000.0)
  } else {
    stars.to_string()
  }
}

pub struct GithubSearchPalette {
  focus_handle: FocusHandle,
  results_list: Entity<ListState<GithubSearchListDelegate>>,
  error: Option<SharedString>,
  on_select: Option<GithubRepoSelectFn>,
  _subscriptions: Vec<Subscription>,
}

impl GithubSearchPalette {
  pub fn new(
    window: &mut Window,
    cx: &mut Context<Self>,
    config: GithubSearchPaletteConfig,
  ) -> Self {
    let delegate = GithubSearchListDelegate {
      matched_repositories: Vec::new(),
      selected_index: None,
      query: "".into(),
      search_fn: config.search_fn,
      generation: 0,
      loading: false,
    };
    let results_list = cx.new(|cx| ListState::new(delegate, window, cx).searchable(true));

    let _subscriptions = vec![cx.subscribe_in(
      &results_list,
      window,
      |palette, list_state, ev: &ListEvent, window, cx| {
        if let ListEvent::Confirm(ix) = ev {
          let entry = list_state
            .read(cx)
            .delegate()
            .matched_repositories
            .get(ix.row)
            .cloned();

          if let Some(entry) = entry {
            palette.select_repo((*entry).clone(), window, cx);
          }
        }
      },
    )];

    cx.on_next_frame(window, |this, window, cx| this.focus_list(window, cx));

    Self {
      focus_handle: cx.focus_handle(),
      results_list,
      error: None,
      on_select: Some(config.on_select),
      _subscriptions,
    }
  }

  fn focus_list(&self, window: &mut Window, cx: &mut Context<Self>) {
    self.results_list.update(cx, |state, cx| {
      state.focus(window, cx);
    });
  }

  fn select_repo(
    &mut self,
    entry: GithubSearchRepoEntry,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(handler) = self.on_select.as_ref() else {
      return;
    };

    match handler(entry, window, cx) {
      Ok(()) => window.close_dialog(cx),
      Err(err) => {
        self.error = Some(err);
        cx.notify();
      }
    }
  }

  fn render_error(&self, theme: &gpui_component::Theme, error: &SharedString) -> Div {
    div()
      .px_3()
      .py_2()
      .text_sm()
      .text_color(theme.red)
      .child(error.clone())
  }
}

impl Focusable for GithubSearchPalette {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for GithubSearchPalette {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let count = self
      .results_list
      .read(cx)
      .delegate()
      .matched_repositories
      .len();
    let rows_for_height = count.max(PLACEHOLDER_ROW_SPAN);

    v_flex()
      .track_focus(&self.focus_handle)
      .max_h_128()
      .child(
        List::new(&self.results_list)
          .w_full()
          .h(px(
            LIST_ITEM_HEIGHT * rows_for_height as f32 + LIST_INPUT_HEIGHT,
          ))
          .border_1()
          .search_placeholder("Search GitHub repositories...")
          .border_color(theme.border)
          .rounded(theme.radius),
      )
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }
}

#[cfg(test)]
mod tests {
  use super::format_stars;

  #[test]
  fn format_stars_formats_small_counts_as_is() {
    assert_eq!(format_stars(0), "0");
    assert_eq!(format_stars(7), "7");
    assert_eq!(format_stars(999), "999");
  }

  #[test]
  fn format_stars_uses_k_suffix_for_thousands() {
    assert_eq!(format_stars(1_000), "1.0k");
    assert_eq!(format_stars(12_300), "12.3k");
  }

  #[test]
  fn format_stars_uses_m_suffix_for_millions() {
    assert_eq!(format_stars(1_500_000), "1.5M");
  }
}
