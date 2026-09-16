use std::{
  ops::Range,
  path::{Path, PathBuf},
  sync::Arc,
};

use gpui::{
  AnyElement, App, Context, Entity, FocusHandle, Focusable, HighlightStyle, IntoElement,
  ParentElement, Render, SharedString, Styled, StyledText, Subscription, Task, Window, div, img,
  prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _, Size, WindowExt as _, h_flex,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  skeleton::Skeleton,
  spinner::Spinner,
  v_flex,
};
use ui::{FILE_ICON_SIZE_PX, file_icon_path_for_path_with_theme};

const PALETTE_LIST_MAX_HEIGHT: f32 = 420.0;
const MAX_SEARCH_RESULTS: usize = 200;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GlobalSearchOpenRequest {
  pub path: PathBuf,
  pub line: Option<u32>,
  pub column: Option<u32>,
}

pub(crate) type GlobalSearchHandler =
  Arc<dyn Fn(GlobalSearchOpenRequest, &mut Window, &mut App) -> Result<(), SharedString>>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct GlobalSearchResult {
  path: PathBuf,
  line_number: u32,
  column: u32,
  preview: SharedString,
  match_range: Option<Range<usize>>,
}

struct GlobalSearchListDelegate {
  checkout_root: PathBuf,
  files: Arc<Vec<PathBuf>>,
  results: Vec<GlobalSearchResult>,
  selected_index: Option<IndexPath>,
  query: String,
  search_generation: u64,
  loading_files: bool,
}

impl GlobalSearchListDelegate {
  fn replace_files(&mut self, files: Vec<PathBuf>, loading: bool) {
    self.files = Arc::new(files);
    self.loading_files = loading;
    self.search_generation = self.search_generation.wrapping_add(1);
  }

  fn request_at(&self, ix: IndexPath) -> Option<GlobalSearchOpenRequest> {
    let result = self.results.get(ix.row)?;
    Some(GlobalSearchOpenRequest {
      path: result.path.clone(),
      line: Some(result.line_number),
      column: Some(result.column),
    })
  }
}

fn palette_list_item(ix: IndexPath, selected_index: Option<IndexPath>) -> ListItem {
  ListItem::new(ix)
    .selected(Some(ix) == selected_index)
    .mx_1()
    .px_2()
    .py_1p5()
    .rounded_md()
}

fn empty_state(label: impl Into<SharedString>, cx: &App) -> AnyElement {
  v_flex()
    .items_center()
    .py_8()
    .text_sm()
    .text_color(cx.theme().muted_foreground)
    .child(label.into())
    .into_any_element()
}

fn loading_row(index: usize) -> AnyElement {
  palette_list_item(IndexPath::new(index), None)
    .child(
      v_flex()
        .gap_1()
        .child(
          h_flex()
            .items_center()
            .gap_2()
            .child(Skeleton::new().size(px(16.0)).rounded(px(4.0)))
            .child(
              Skeleton::new()
                .h(px(16.0))
                .w(px(match index % 3 {
                  0 => 112.0,
                  1 => 156.0,
                  _ => 92.0,
                }))
                .rounded(px(999.0)),
            ),
        )
        .child(
          Skeleton::new()
            .secondary()
            .h(px(14.0))
            .w(px(match index % 3 {
              0 => 360.0,
              1 => 280.0,
              _ => 420.0,
            }))
            .rounded(px(999.0)),
        ),
    )
    .into_any_element()
}

fn highlighted_preview(result: &GlobalSearchResult, color: gpui::Hsla) -> StyledText {
  let highlights = result
    .match_range
    .clone()
    .map(|range| {
      vec![(
        range,
        HighlightStyle {
          color: Some(color),
          ..Default::default()
        },
      )]
    })
    .unwrap_or_default();
  StyledText::new(result.preview.clone()).with_highlights(highlights)
}

impl ListDelegate for GlobalSearchListDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.results.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let result = self.results.get(ix.row)?.clone();
    let theme = cx.theme().clone();
    let file_icon: AnyElement = file_icon_path_for_path_with_theme(&result.path, &theme)
      .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
      .unwrap_or_else(|| {
        Icon::new(IconName::File)
          .size_3()
          .text_color(theme.muted_foreground)
          .into_any_element()
      });
    let file_name = result
      .path
      .file_name()
      .and_then(|name| name.to_str())
      .map(ToString::to_string)
      .unwrap_or_else(|| result.path.to_string_lossy().to_string());
    let directory = result
      .path
      .parent()
      .and_then(|parent| parent.to_str())
      .unwrap_or_default()
      .to_string();

    Some(
      palette_list_item(ix, self.selected_index).child(
        v_flex()
          .gap_1()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .w_full()
              .child(file_icon)
              .child(
                div()
                  .text_sm()
                  .overflow_hidden()
                  .whitespace_nowrap()
                  .text_ellipsis()
                  .child(file_name),
              )
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(format!(":{}", result.line_number)),
              )
              .when(!directory.is_empty(), |this| {
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
                    .child(directory),
                )
              }),
          )
          .child(
            div()
              .pl_6()
              .text_xs()
              .line_height(px(17.0))
              .text_color(theme.muted_foreground)
              .overflow_hidden()
              .whitespace_nowrap()
              .text_ellipsis()
              .child(highlighted_preview(&result, theme.blue)),
          ),
      ),
    )
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    if self.loading_files {
      return v_flex()
        .debug_selector(|| "global-search-loading".to_string())
        .w_full()
        .gap_1()
        .py_2()
        .children((0..7).map(loading_row))
        .into_any_element();
    }
    if self.query.trim().is_empty() {
      return empty_state("Type to search across project files", cx).into_any_element();
    }
    if self.files.is_empty() {
      return empty_state("No project files to search", cx).into_any_element();
    }
    empty_state("No matching results", cx).into_any_element()
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

  fn perform_search(
    &mut self,
    query: &str,
    window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Task<()> {
    self.query = query.trim().to_string();
    self.search_generation = self.search_generation.wrapping_add(1);
    let generation = self.search_generation;

    if self.query.is_empty() || self.loading_files {
      self.results.clear();
      self.selected_index = None;
      cx.notify();
      return Task::ready(());
    }

    let checkout_root = self.checkout_root.clone();
    let files = self.files.clone();
    let query = self.query.clone();
    let search =
      cx.background_spawn(async move { search_project_files(&checkout_root, &files, &query) });
    cx.spawn_in(window, async move |this, cx| {
      let results = search.await;
      let _ = this.update_in(cx, |state, window, cx| {
        if state.delegate().search_generation != generation {
          return;
        }
        state.delegate_mut().results = results;
        let selected = (!state.delegate().results.is_empty()).then(IndexPath::default);
        state.set_selected_index(selected, window, cx);
        cx.notify();
      });
    })
  }
}

pub(crate) struct GlobalSearchPalette {
  focus_handle: FocusHandle,
  results_list: Entity<ListState<GlobalSearchListDelegate>>,
  loading_files: bool,
  error: Option<SharedString>,
  on_open: Option<GlobalSearchHandler>,
  _subscriptions: Vec<Subscription>,
}

impl GlobalSearchPalette {
  pub(crate) fn new(
    window: &mut Window,
    cx: &mut Context<Self>,
    checkout_root: PathBuf,
    files: Vec<PathBuf>,
    loading_files: bool,
    on_open: GlobalSearchHandler,
  ) -> Self {
    let delegate = GlobalSearchListDelegate {
      checkout_root,
      files: Arc::new(files),
      results: Vec::new(),
      selected_index: None,
      query: String::new(),
      search_generation: 0,
      loading_files,
    };
    let results_list = cx.new(|cx| ListState::new(delegate, window, cx).searchable(true));

    let _subscriptions = vec![cx.subscribe_in(
      &results_list,
      window,
      |palette, list_state, event: &ListEvent, window, cx| {
        if let ListEvent::Confirm(index) = event {
          let request = list_state.read(cx).delegate().request_at(*index);
          if let Some(request) = request {
            palette.open_result(request, window, cx);
          }
        }
      },
    )];

    cx.on_next_frame(window, |this, window, cx| this.focus_list(window, cx));

    Self {
      focus_handle: cx.focus_handle(),
      results_list,
      loading_files,
      error: None,
      on_open: Some(on_open),
      _subscriptions,
    }
  }

  pub(crate) fn replace_files(
    &mut self,
    files: Vec<PathBuf>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.loading_files = false;
    self.error = None;
    self.results_list.update(cx, |state, cx| {
      let query = state.delegate().query.clone();
      state.delegate_mut().replace_files(files, false);
      state.set_query(&query, window, cx);
    });
    cx.notify();
  }

  pub(crate) fn set_loading_error(
    &mut self,
    error: impl Into<SharedString>,
    cx: &mut Context<Self>,
  ) {
    self.loading_files = false;
    self.error = Some(error.into());
    self.results_list.update(cx, |state, cx| {
      state.delegate_mut().loading_files = false;
      cx.notify();
    });
    cx.notify();
  }

  fn focus_list(&self, window: &mut Window, cx: &mut Context<Self>) {
    self.results_list.update(cx, |state, cx| {
      state.focus(window, cx);
    });
  }

  fn open_result(
    &mut self,
    request: GlobalSearchOpenRequest,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(handler) = self.on_open.as_ref() else {
      return;
    };

    match handler(request, window, cx) {
      Ok(()) => window.close_dialog(cx),
      Err(error) => {
        self.error = Some(error);
        cx.notify();
      }
    }
  }
}

impl Focusable for GlobalSearchPalette {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for GlobalSearchPalette {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let key = |label: &'static str| {
      div()
        .px_1()
        .rounded_sm()
        .bg(theme.muted)
        .text_color(theme.muted_foreground)
        .child(label)
    };

    v_flex()
      .track_focus(&self.focus_handle)
      .child(
        List::new(&self.results_list)
          .w_full()
          .max_h(px(PALETTE_LIST_MAX_HEIGHT))
          .with_size(Size::Large)
          .search_placeholder("Search project..."),
      )
      .when(self.loading_files, |parent| {
        parent.child(
          h_flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(Spinner::new().small())
            .child("Loading project files..."),
        )
      })
      .when_some(self.error.clone(), |parent, error| {
        parent.child(
          div()
            .px_3()
            .py_2()
            .text_sm()
            .text_color(theme.red)
            .child(error),
        )
      })
      .child(
        h_flex()
          .px_3()
          .py_2()
          .gap_4()
          .border_t_1()
          .border_color(theme.border)
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(
            h_flex()
              .gap_1()
              .child(key("↑"))
              .child(key("↓"))
              .child("navigate"),
          )
          .child(h_flex().gap_1().child(key("↵")).child("open"))
          .child(h_flex().gap_1().child(key("esc")).child("close")),
      )
  }
}

pub(crate) fn open_global_search_palette<T: 'static>(
  window: &mut Window,
  cx: &mut Context<T>,
  checkout_root: PathBuf,
  files: Vec<PathBuf>,
  handler: GlobalSearchHandler,
  loading_files: bool,
) -> Entity<GlobalSearchPalette> {
  let palette =
    cx.new(|cx| GlobalSearchPalette::new(window, cx, checkout_root, files, loading_files, handler));
  ui::open_palette_dialog(palette.clone(), window, cx);
  palette
}

fn search_project_files(
  checkout_root: &Path,
  files: &[PathBuf],
  query: &str,
) -> Vec<GlobalSearchResult> {
  let query = query.trim();
  if query.is_empty() {
    return Vec::new();
  }

  let query_lower = query.to_lowercase();
  let mut results = Vec::new();

  for path in files {
    if results.len() >= MAX_SEARCH_RESULTS {
      break;
    }

    let absolute_path = checkout_root.join(path);
    if std::fs::metadata(&absolute_path).map_or(true, |metadata| {
      !metadata.is_file() || metadata.len() > MAX_FILE_BYTES
    }) {
      continue;
    }

    let Ok(contents) = std::fs::read_to_string(&absolute_path) else {
      continue;
    };

    for (line_index, line) in contents.lines().enumerate() {
      if results.len() >= MAX_SEARCH_RESULTS {
        break;
      }

      let Some((start, end)) = find_case_insensitive(line, query, &query_lower) else {
        continue;
      };
      let preview = line.trim().to_string();
      let leading_trimmed = line.len().saturating_sub(line.trim_start().len());
      let match_range = start
        .checked_sub(leading_trimmed)
        .and_then(|preview_start| {
          end
            .checked_sub(leading_trimmed)
            .map(|preview_end| preview_start..preview_end)
        })
        .filter(|range| {
          range.end <= preview.len()
            && preview.is_char_boundary(range.start)
            && preview.is_char_boundary(range.end)
        });
      let column = line[..start].chars().count().saturating_add(1) as u32;
      results.push(GlobalSearchResult {
        path: path.clone(),
        line_number: line_index.saturating_add(1) as u32,
        column,
        preview: preview.into(),
        match_range,
      });
    }
  }

  results
}

fn find_case_insensitive(line: &str, query: &str, query_lower: &str) -> Option<(usize, usize)> {
  line
    .find(query)
    .map(|start| (start, start + query.len()))
    .or_else(|| {
      let line_lower = line.to_lowercase();
      let start = line_lower.find(query_lower)?;
      let end = start + query_lower.len();
      (line.is_char_boundary(start) && line.is_char_boundary(end)).then_some((start, end))
    })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::TempDir;
  use std::path::Path;

  #[test]
  fn global_search_finds_line_and_column() {
    let temp = TempDir::new("global-search");
    std::fs::create_dir_all(temp.path.join("src")).expect("create src");
    std::fs::write(temp.path.join("src/lib.rs"), "first\nlet needle = true;\n")
      .expect("write file");

    let results = search_project_files(
      &temp.path,
      &[Path::new("src/lib.rs").to_path_buf()],
      "needle",
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path, PathBuf::from("src/lib.rs"));
    assert_eq!(results[0].line_number, 2);
    assert_eq!(results[0].column, 5);
  }

  #[test]
  fn global_search_is_case_insensitive() {
    let temp = TempDir::new("global-search-case");
    std::fs::write(temp.path.join("README.md"), "Hello Reviu\n").expect("write file");

    let results = search_project_files(&temp.path, &[PathBuf::from("README.md")], "reviu");

    assert_eq!(results.len(), 1);
  }
}
