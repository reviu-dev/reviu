use std::{path::PathBuf, rc::Rc, sync::Arc};

use crate::{
  FILE_ICON_SIZE_PX, SelectableRowStyle, file_icon_path_for_path_with_theme, selectable_list_item,
};
use gpui::{
  AnyElement, App, Context, Div, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
  Render, SharedString, Styled, Subscription, Task, Window, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, IndexPath, WindowExt, h_flex,
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  v_flex,
};

const LIST_INPUT_HEIGHT: f32 = 35.0;
const LIST_ITEM_HEIGHT: f32 = 32.0;

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
pub struct SearchFileEntry {
  pub path: PathBuf,
  pub label: SharedString,
}

impl SearchFileEntry {
  pub fn new(path: PathBuf, label: impl Into<SharedString>) -> Self {
    Self {
      path,
      label: label.into(),
    }
  }

  fn matches(&self, query: &str) -> bool {
    if query.is_empty() {
      return true;
    }
    self.label.as_ref().to_lowercase().contains(query)
  }
}

pub type SearchFileHandler =
  Arc<dyn Fn(PathBuf, &mut Window, &mut App) -> Result<(), SharedString> + Send + Sync>;

pub struct SearchFilePaletteConfig {
  pub entries: Vec<SearchFileEntry>,
  pub on_open: SearchFileHandler,
}

impl SearchFilePaletteConfig {
  pub fn new(entries: Vec<SearchFileEntry>, on_open: SearchFileHandler) -> Self {
    Self { entries, on_open }
  }
}

struct SearchFileListDelegate {
  _files: Vec<Rc<SearchFileEntry>>,
  matched_files: Vec<Rc<SearchFileEntry>>,
  selected_index: Option<IndexPath>,
  query: SharedString,
}

impl SearchFileListDelegate {
  fn prepare(&mut self, query: impl Into<SharedString>) {
    self.query = query.into();

    let q = self.query.as_ref().to_lowercase();
    let files: Vec<Rc<SearchFileEntry>> = self
      ._files
      .iter()
      .filter(|entry| entry.matches(&q))
      .cloned()
      .collect();

    self.matched_files = files;
  }
}

impl ListDelegate for SearchFileListDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.matched_files.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let total_items = self.matched_files.len();
    let theme = cx.theme().clone();

    let base_item = list_base_item(ix, total_items, self.selected_index, &theme);

    self.matched_files.get(ix.row).map(|entry| {
      let file_icon: AnyElement = file_icon_path_for_path_with_theme(&entry.path, &theme)
        .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
        .unwrap_or_else(|| {
          Icon::new(IconName::File)
            .size_3()
            .text_color(theme.muted_foreground)
            .into_any_element()
        });
      let file_name: SharedString = entry
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(entry.label.as_ref())
        .to_string()
        .into();
      let dir_label = entry
        .path
        .parent()
        .and_then(|path| path.to_str())
        .map(str::to_string)
        .filter(|path| !path.is_empty() && path != ".");

      base_item.child(
        h_flex()
          .items_center()
          .gap_2()
          .w_full()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .min_w_0()
              .flex_shrink()
              .child(file_icon)
              .child(Label::new(file_name).truncate()),
          )
          .when_some(dir_label, |this, dir_label| {
            this.child(
              div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .text_right()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis_start()
                .child(dir_label),
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

pub struct SearchFilePalette {
  focus_handle: FocusHandle,
  files_list: Entity<ListState<SearchFileListDelegate>>,
  error: Option<SharedString>,
  on_open: Option<SearchFileHandler>,
  _subscriptions: Vec<Subscription>,
}

impl SearchFilePalette {
  pub fn new(window: &mut Window, cx: &mut Context<Self>, config: SearchFilePaletteConfig) -> Self {
    let files: Vec<Rc<SearchFileEntry>> = config.entries.into_iter().map(Rc::new).collect();
    let delegate = SearchFileListDelegate {
      _files: files.clone(),
      matched_files: files.clone(),
      selected_index: None,
      query: "".into(),
    };
    let files_list = cx.new(|cx| ListState::new(delegate, window, cx).searchable(true));

    let _subscriptions = vec![cx.subscribe_in(
      &files_list,
      window,
      |palette, list_state, ev: &ListEvent, window, cx| {
        if let ListEvent::Confirm(ix) = ev {
          let entry = {
            let list = list_state.read(cx);
            list.delegate().matched_files.get(ix.row).cloned()
          };

          if let Some(entry) = entry {
            palette.open_file(entry.path.clone(), window, cx);
          }
        }
      },
    )];

    cx.on_next_frame(window, |this, window, cx| this.focus_list(window, cx));

    Self {
      focus_handle: cx.focus_handle(),
      files_list,
      error: None,
      on_open: Some(config.on_open),
      _subscriptions,
    }
  }

  fn focus_list(&self, window: &mut Window, cx: &mut Context<Self>) {
    self.files_list.update(cx, |state, cx| {
      state.focus(window, cx);
    });
  }

  fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
    let Some(handler) = self.on_open.as_ref() else {
      return;
    };

    match handler(path, window, cx) {
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

  fn render_error(&self, theme: &gpui_component::Theme, error: &SharedString) -> Div {
    div().text_sm().text_color(theme.red).child(error.clone())
  }
}

impl Focusable for SearchFilePalette {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for SearchFilePalette {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let count = self.files_list.read(cx).delegate().matched_files.len();

    v_flex()
      .track_focus(&self.focus_handle)
      .max_h_128()
      .child(self.render_search_list(&self.files_list, count, "Search files...", cx))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }
}
