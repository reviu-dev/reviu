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
const SECTION_HEADER_HEIGHT: f32 = 28.0;

fn list_base_item(
  ix: IndexPath,
  is_last_overall: bool,
  selected_index: Option<IndexPath>,
  theme: &gpui_component::Theme,
) -> ListItem {
  selectable_list_item(
    ix,
    Some(ix) == selected_index,
    SelectableRowStyle::Flush,
    theme,
  )
  .h_8()
  .when(is_last_overall, |item| item.rounded_b(theme.radius))
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
  pub group: Option<SharedString>,
}

impl SearchFileEntry {
  pub fn new(path: PathBuf, label: impl Into<SharedString>) -> Self {
    Self {
      path,
      label: label.into(),
      group: None,
    }
  }

  pub fn grouped(mut self, group: impl Into<SharedString>) -> Self {
    self.group = Some(group.into());
    self
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
  matched_sections: Vec<SearchFileSection>,
  selected_index: Option<IndexPath>,
  query: SharedString,
}

struct SearchFileSection {
  label: Option<SharedString>,
  files: Vec<Rc<SearchFileEntry>>,
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

    self.matched_sections = build_file_sections(files);
  }

  fn matched_total_count(&self) -> usize {
    self
      .matched_sections
      .iter()
      .map(|section| section.files.len())
      .sum()
  }

  fn visible_sections_count(&self) -> usize {
    self
      .matched_sections
      .iter()
      .filter(|section| section.label.is_some())
      .count()
  }

  fn item_at(&self, ix: IndexPath) -> Option<Rc<SearchFileEntry>> {
    self
      .matched_sections
      .get(ix.section)
      .and_then(|section| section.files.get(ix.row))
      .cloned()
  }
}

fn build_file_sections(files: Vec<Rc<SearchFileEntry>>) -> Vec<SearchFileSection> {
  let mut sections: Vec<SearchFileSection> = Vec::new();

  for file in files {
    if let Some(section) = sections
      .iter_mut()
      .find(|section| section.label == file.group)
    {
      section.files.push(file);
      continue;
    }

    sections.push(SearchFileSection {
      label: file.group.clone(),
      files: vec![file],
    });
  }

  sections
}

impl ListDelegate for SearchFileListDelegate {
  type Item = ListItem;

  fn sections_count(&self, _cx: &App) -> usize {
    self.matched_sections.len()
  }

  fn items_count(&self, section: usize, _cx: &App) -> usize {
    self
      .matched_sections
      .get(section)
      .map_or(0, |section| section.files.len())
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let last_section_ix = self.matched_sections.len().saturating_sub(1);
    let total_in_last_section = self
      .matched_sections
      .last()
      .map_or(0, |section| section.files.len());
    let is_last_overall = ix.section == last_section_ix && ix.row + 1 == total_in_last_section;
    let theme = cx.theme().clone();

    let base_item = list_base_item(ix, is_last_overall, self.selected_index, &theme);

    self.item_at(ix).map(|entry| {
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

  fn render_section_header(
    &mut self,
    section: usize,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<impl IntoElement> {
    if self.visible_sections_count() <= 1 {
      return None;
    }
    let label = self.matched_sections.get(section)?.label.clone()?;

    Some(
      h_flex()
        .px_3()
        .pt_2()
        .pb_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(label),
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
      matched_sections: build_file_sections(files.clone()),
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
            list.delegate().item_at(*ix)
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
    visible_headers: usize,
    placeholder: &'static str,
    cx: &Context<Self>,
  ) -> impl IntoElement {
    List::new(list)
      .w_full()
      .h(px(
        LIST_ITEM_HEIGHT * count as f32
          + SECTION_HEADER_HEIGHT * visible_headers as f32
          + LIST_INPUT_HEIGHT,
      ))
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
    let delegate = self.files_list.read(cx);
    let count = delegate.delegate().matched_total_count();
    let visible_headers = match delegate.delegate().visible_sections_count() {
      0 | 1 => 0,
      count => count,
    };

    v_flex()
      .track_focus(&self.focus_handle)
      .max_h_128()
      .child(self.render_search_list(
        &self.files_list,
        count,
        visible_headers,
        "Search files...",
        cx,
      ))
      .when(self.error.is_some(), |parent| {
        parent.child(self.render_error(&theme, &self.error.clone().unwrap_or_default()))
      })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn file_sections_preserve_group_order_and_items() {
    let files = vec![
      Rc::new(SearchFileEntry::new(PathBuf::from("src/main.rs"), "src/main.rs").grouped("Changed")),
      Rc::new(SearchFileEntry::new(PathBuf::from("README.md"), "README.md").grouped("Unchanged")),
      Rc::new(SearchFileEntry::new(PathBuf::from("src/lib.rs"), "src/lib.rs").grouped("Changed")),
    ];

    let sections = build_file_sections(files);

    assert_eq!(sections.len(), 2);
    assert_eq!(
      sections[0].label.as_ref().map(|label| label.as_ref()),
      Some("Changed")
    );
    assert_eq!(sections[0].files.len(), 2);
    assert_eq!(
      sections[1].label.as_ref().map(|label| label.as_ref()),
      Some("Unchanged")
    );
    assert_eq!(sections[1].files.len(), 1);
  }

  #[test]
  fn file_delegate_search_keeps_matching_group_sections() {
    let files = vec![
      Rc::new(SearchFileEntry::new(PathBuf::from("src/main.rs"), "src/main.rs").grouped("Changed")),
      Rc::new(SearchFileEntry::new(PathBuf::from("README.md"), "README.md").grouped("Unchanged")),
    ];
    let mut delegate = SearchFileListDelegate {
      _files: files.clone(),
      matched_sections: build_file_sections(files),
      selected_index: None,
      query: "".into(),
    };

    delegate.prepare("readme");

    assert_eq!(delegate.matched_total_count(), 1);
    assert_eq!(
      delegate
        .matched_sections
        .first()
        .and_then(|section| section.label.as_ref())
        .map(|label| label.as_ref()),
      Some("Unchanged")
    );
  }
}
