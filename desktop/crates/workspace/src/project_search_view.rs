use std::{
  collections::HashSet,
  ops::Range,
  path::{Path, PathBuf},
  rc::Rc,
  sync::Arc,
  time::Duration,
};

use editor::SearchOptions;
use gpui::{
  AnyElement, App, Context, Entity, FocusHandle, Focusable, HighlightStyle, IntoElement,
  ParentElement, Render, SharedString, Styled, StyledText, Subscription, Task, Window, div, img,
  prelude::*, px, size,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, Selectable, Sizable as _, VirtualListScrollHandle,
  button::{Button, ButtonVariants as _},
  h_flex,
  input::{Input, InputEvent, InputState},
  list::ListItem,
  scroll::Scrollbar,
  spinner::Spinner,
  v_flex, v_virtual_list,
};
use ui::{FILE_ICON_SIZE_PX, file_icon_path_for_path_with_theme};

const MAX_SEARCH_RESULTS: usize = 500;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);
const FILE_HEADER_ROW_HEIGHT: f32 = 40.0;
const MATCH_ROW_HEIGHT: f32 = 32.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectSearchOpenRequest {
  pub path: PathBuf,
  pub line: Option<u32>,
  pub column: Option<u32>,
}

pub(crate) type ProjectSearchHandler =
  Arc<dyn Fn(ProjectSearchOpenRequest, &mut Window, &mut App) -> Result<(), SharedString>>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProjectSearchMatch {
  line_number: u32,
  column: u32,
  preview: SharedString,
  match_range: Option<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProjectSearchFileResults {
  path: PathBuf,
  matches: Vec<ProjectSearchMatch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectSearchRow {
  File {
    result_index: usize,
  },
  Match {
    result_index: usize,
    match_index: usize,
  },
}

impl ProjectSearchRow {
  fn height(self) -> f32 {
    match self {
      Self::File { .. } => FILE_HEADER_ROW_HEIGHT,
      Self::Match { .. } => MATCH_ROW_HEIGHT,
    }
  }
}

pub(crate) struct ProjectSearchView {
  focus_handle: FocusHandle,
  checkout_root: PathBuf,
  files: Arc<Vec<PathBuf>>,
  query_input: Entity<InputState>,
  _query_subscription: Subscription,
  results: Vec<ProjectSearchFileResults>,
  rows: Vec<ProjectSearchRow>,
  collapsed_files: HashSet<PathBuf>,
  options: SearchOptions,
  loading_files: bool,
  searching: bool,
  error: Option<SharedString>,
  on_open: ProjectSearchHandler,
  scroll_handle: VirtualListScrollHandle,
  search_generation: u64,
  _search_task: Task<()>,
}

impl ProjectSearchView {
  pub(crate) fn new(
    window: &mut Window,
    cx: &mut Context<Self>,
    checkout_root: PathBuf,
    files: Vec<PathBuf>,
    loading_files: bool,
    on_open: ProjectSearchHandler,
  ) -> Self {
    let query_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search..."));
    let query_subscription = cx.subscribe_in(&query_input, window, Self::on_query_input_event);
    Self {
      focus_handle: cx.focus_handle(),
      checkout_root,
      files: Arc::new(files),
      query_input,
      _query_subscription: query_subscription,
      results: Vec::new(),
      rows: Vec::new(),
      collapsed_files: HashSet::new(),
      options: cx
        .try_global::<SearchOptions>()
        .copied()
        .unwrap_or_default(),
      loading_files,
      searching: false,
      error: None,
      on_open,
      scroll_handle: VirtualListScrollHandle::new(),
      search_generation: 0,
      _search_task: Task::ready(()),
    }
  }

  pub(crate) fn checkout_root(&self) -> &Path {
    &self.checkout_root
  }

  pub(crate) fn replace_files(&mut self, files: Vec<PathBuf>, cx: &mut Context<Self>) {
    self.files = Arc::new(files);
    self.loading_files = false;
    self.error = None;
    self.refresh_results(cx);
  }

  pub(crate) fn set_loading_error(
    &mut self,
    error: impl Into<SharedString>,
    cx: &mut Context<Self>,
  ) {
    self.loading_files = false;
    self.error = Some(error.into());
    cx.notify();
  }

  pub(crate) fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
    self
      .query_input
      .update(cx, |input, cx| input.focus(window, cx));
  }

  #[cfg(test)]
  pub(crate) fn set_query_for_test(
    &mut self,
    query: impl Into<String>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self
      .query_input
      .update(cx, |input, cx| input.set_value(query.into(), window, cx));
  }

  #[cfg(test)]
  pub(crate) fn query_for_test(&self, cx: &App) -> String {
    self.query_input.read(cx).value().to_string()
  }

  fn on_query_input_event(
    &mut self,
    _input: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match event {
      InputEvent::Change => self.refresh_results(cx),
      InputEvent::PressEnter { .. } => {
        if let Some(request) = self.first_open_request() {
          self.open_result(request, window, cx);
        }
      }
      _ => {}
    }
  }

  fn first_open_request(&self) -> Option<ProjectSearchOpenRequest> {
    let file = self.results.first()?;
    let found = file.matches.first()?;
    Some(ProjectSearchOpenRequest {
      path: file.path.clone(),
      line: Some(found.line_number),
      column: Some(found.column),
    })
  }

  fn open_result(
    &mut self,
    request: ProjectSearchOpenRequest,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match (self.on_open)(request, window, cx) {
      Ok(()) => {}
      Err(error) => {
        self.error = Some(error);
        cx.notify();
      }
    }
  }

  fn refresh_results(&mut self, cx: &mut Context<Self>) {
    let query = self.query_input.read(cx).value().to_string();
    self.search_generation = self.search_generation.wrapping_add(1);
    let generation = self.search_generation;

    if query.trim().is_empty() {
      self.searching = false;
      self.results.clear();
      self.rows.clear();
      self._search_task = Task::ready(());
      cx.notify();
      return;
    }

    self.searching = true;
    self.error = None;
    self.results.clear();
    self.rows.clear();
    let checkout_root = self.checkout_root.clone();
    let files = self.files.clone();
    let options = self.options;
    self._search_task = cx.spawn(async move |this, cx| {
      cx.background_executor().timer(SEARCH_DEBOUNCE).await;
      let results = cx
        .background_spawn(async move {
          search_project_files(&checkout_root, files.as_ref(), &query, options)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        if this.search_generation != generation {
          return;
        }
        this.searching = false;
        this.results = results;
        this.collapsed_files.retain(|path| {
          this
            .results
            .iter()
            .any(|result| result.path.as_path() == path.as_path())
        });
        this.sync_result_rows();
        cx.notify();
      });
    });
    cx.notify();
  }

  fn sync_result_rows(&mut self) {
    self.rows.clear();
    for (result_index, file) in self.results.iter().enumerate() {
      self.rows.push(ProjectSearchRow::File { result_index });
      if !self.collapsed_files.contains(&file.path) {
        self.rows.extend(
          (0..file.matches.len()).map(|match_index| ProjectSearchRow::Match {
            result_index,
            match_index,
          }),
        );
      }
    }
  }

  fn toggle_file_collapsed(&mut self, path: &Path, cx: &mut Context<Self>) {
    if !self.collapsed_files.remove(path) {
      self.collapsed_files.insert(path.to_path_buf());
    }
    self.sync_result_rows();
    cx.notify();
  }

  fn toggle_case_sensitive(&mut self, cx: &mut Context<Self>) {
    self.options.case_sensitive = !self.options.case_sensitive;
    self.refresh_results(cx);
  }

  fn toggle_whole_word(&mut self, cx: &mut Context<Self>) {
    self.options.whole_word = !self.options.whole_word;
    self.refresh_results(cx);
  }

  fn toggle_regex(&mut self, cx: &mut Context<Self>) {
    self.options.regex = !self.options.regex;
    self.refresh_results(cx);
  }

  fn result_count(&self) -> usize {
    self.results.iter().map(|file| file.matches.len()).sum()
  }

  fn render_result_row(&mut self, row_index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
    match *self.rows.get(row_index)? {
      ProjectSearchRow::File { result_index } => {
        let file = self.results.get(result_index)?;
        Some(render_file_header(
          file,
          self.collapsed_files.contains(&file.path),
          cx,
        ))
      }
      ProjectSearchRow::Match {
        result_index,
        match_index,
      } => {
        let file = self.results.get(result_index)?;
        let found = file.matches.get(match_index)?;
        Some(render_match_row(file.path.clone(), found, cx))
      }
    }
  }

  fn render_results(&mut self, query_empty: bool, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let row_sizes = Rc::new(
      self
        .rows
        .iter()
        .map(|row| size(px(0.0), px(row.height())))
        .collect::<Vec<_>>(),
    );
    let scroll_handle = self.scroll_handle.clone();
    let scrollbar_handle = self.scroll_handle.clone();
    let has_rows = !self.rows.is_empty();

    div()
      .flex_1()
      .min_h_0()
      .relative()
      .overflow_hidden()
      .when(self.loading_files, |this| {
        this.child(
          h_flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(Spinner::new().small())
            .child("Loading project files..."),
        )
      })
      .when_some(self.error.clone(), |this, error| {
        this.child(
          div()
            .px_4()
            .py_3()
            .text_sm()
            .text_color(theme.red)
            .child(error),
        )
      })
      .when(!self.loading_files && query_empty, |this| {
        this.child(empty_state("Search All Files", &theme))
      })
      .when(
        !self.loading_files && !query_empty && self.searching && !has_rows,
        |this| this.child(empty_state("Searching...", &theme)),
      )
      .when(
        !self.loading_files && !query_empty && !self.searching && !has_rows,
        |this| this.child(empty_state("No Results", &theme)),
      )
      .when(has_rows, |this| {
        this
          .child(
            v_virtual_list(
              cx.entity(),
              "project-search-results",
              row_sizes,
              move |view, visible_range, _window, cx| {
                visible_range
                  .filter_map(|row_index| view.render_result_row(row_index, cx))
                  .collect::<Vec<_>>()
              },
            )
            .track_scroll(&scroll_handle),
          )
          .child(Scrollbar::vertical(&scrollbar_handle))
      })
      .into_any_element()
  }
}

impl Focusable for ProjectSearchView {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for ProjectSearchView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let query_input = self.query_input.clone();
    let query = query_input.read(cx).value().to_string();
    let result_count = self.result_count();
    let case_sensitive = self.options.case_sensitive;
    let whole_word = self.options.whole_word;
    let regex = self.options.regex;
    let entity = cx.entity();

    v_flex()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .track_focus(&self.focus_handle)
      .bg(theme.background)
      .child(
        v_flex()
          .gap_1()
          .px_2()
          .py_1p5()
          .border_b_1()
          .border_color(theme.border)
          .child(
            h_flex()
              .gap_2()
              .items_center()
              .child(
                div()
                  .flex_1()
                  .min_w(px(0.0))
                  .h(px(32.0))
                  .flex()
                  .items_center()
                  .gap_1()
                  .pl_1()
                  .pr_1()
                  .border_1()
                  .border_color(theme.border)
                  .rounded_md()
                  .bg(theme.background)
                  .child(
                    div().flex_1().min_w(px(0.0)).child(
                      Input::new(&query_input)
                        .small()
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false),
                    ),
                  )
                  .child(
                    h_flex()
                      .flex_none()
                      .gap_1()
                      .child(
                        Button::new("project-search-case")
                          .label("Aa")
                          .ghost()
                          .xsmall()
                          .compact()
                          .selected(case_sensitive)
                          .tooltip("Match case")
                          .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                              entity.update(cx, |view, cx| view.toggle_case_sensitive(cx));
                            }
                          }),
                      )
                      .child(
                        Button::new("project-search-word")
                          .label("wd")
                          .ghost()
                          .xsmall()
                          .compact()
                          .selected(whole_word)
                          .tooltip("Whole word")
                          .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                              entity.update(cx, |view, cx| view.toggle_whole_word(cx));
                            }
                          }),
                      )
                      .child(
                        Button::new("project-search-regex")
                          .label(".*")
                          .ghost()
                          .xsmall()
                          .compact()
                          .selected(regex)
                          .tooltip("Use regex")
                          .on_click(move |_, _, cx| {
                            entity.update(cx, |view, cx| view.toggle_regex(cx));
                          }),
                      ),
                  ),
              )
              .child(
                h_flex()
                  .w(px(88.0))
                  .gap_1()
                  .items_center()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .when(self.searching, |this| this.child(Spinner::new().small()))
                  .child(format!("{}", result_count)),
              ),
          ),
      )
      .child(self.render_results(query.trim().is_empty(), cx))
  }
}

fn empty_state(label: &'static str, theme: &gpui_component::Theme) -> AnyElement {
  v_flex()
    .size_full()
    .items_center()
    .justify_center()
    .py_10()
    .text_sm()
    .text_color(theme.muted_foreground)
    .child(label)
    .into_any_element()
}

fn render_file_header(
  file: &ProjectSearchFileResults,
  collapsed: bool,
  cx: &mut Context<ProjectSearchView>,
) -> AnyElement {
  let theme = cx.theme().clone();
  let path = file.path.clone();
  let file_icon: AnyElement = file_icon_path_for_path_with_theme(&path, &theme)
    .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
    .unwrap_or_else(|| {
      Icon::new(IconName::File)
        .size_3()
        .text_color(theme.muted_foreground)
        .into_any_element()
    });
  let file_name = path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or_default()
    .to_string();
  let directory = path
    .parent()
    .map(|parent| parent.to_string_lossy().to_string())
    .unwrap_or_default();
  let entity = cx.entity();
  let toggle_path = path.clone();
  let collapse_button_id = format!("project-search-file-collapse-{}", path.display());
  let open_button_id = format!("project-search-open-file-{}", path.display());
  let open_request = ProjectSearchOpenRequest {
    path: path.clone(),
    line: None,
    column: None,
  };

  h_flex()
    .w_full()
    .items_center()
    .gap_2()
    .px_2()
    .py_2()
    .border_b_1()
    .border_color(theme.border)
    .bg(theme.muted.opacity(0.35))
    .child(
      Button::new(collapse_button_id)
        .icon(if collapsed {
          IconName::ChevronRight
        } else {
          IconName::ChevronDown
        })
        .ghost()
        .xsmall()
        .compact()
        .tooltip(if collapsed {
          "Expand file"
        } else {
          "Collapse file"
        })
        .on_click({
          let entity = entity.clone();
          move |_, _, cx| {
            entity.update(cx, |view, cx| view.toggle_file_collapsed(&toggle_path, cx));
          }
        }),
    )
    .child(file_icon)
    .child(
      div()
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(file_name),
    )
    .when(!directory.is_empty(), |this| {
      this.child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(directory),
      )
    })
    .child(div().flex_1())
    .child(
      div()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(file.matches.len().to_string()),
    )
    .child(
      Button::new(open_button_id)
        .label("Open File")
        .ghost()
        .xsmall()
        .compact()
        .on_click(move |_, window, cx| {
          let request = open_request.clone();
          entity.update(cx, |view, cx| view.open_result(request, window, cx));
        }),
    )
    .into_any_element()
}

fn render_match_row(
  path: PathBuf,
  found: &ProjectSearchMatch,
  cx: &mut Context<ProjectSearchView>,
) -> AnyElement {
  let theme = cx.theme().clone();
  let line_number = found.line_number;
  let column = found.column;
  let request = ProjectSearchOpenRequest {
    path,
    line: Some(line_number),
    column: Some(column),
  };
  let entity = cx.entity();

  ListItem::new(format!(
    "project-search-match-{}-{}-{}",
    request.path.display(),
    line_number,
    column
  ))
  .w_full()
  .py_1p5()
  .on_click(move |_, window, cx| {
    let request = request.clone();
    entity.update(cx, |view, cx| view.open_result(request, window, cx));
  })
  .child(
    h_flex()
      .w_full()
      .items_center()
      .gap_3()
      .child(
        div()
          .w(px(56.0))
          .text_xs()
          .text_right()
          .text_color(theme.muted_foreground)
          .child(format!("{}:{}", line_number, column)),
      )
      .child(
        div()
          .flex_1()
          .min_w_0()
          .text_sm()
          .overflow_hidden()
          .whitespace_nowrap()
          .text_ellipsis()
          .child(highlighted_preview(
            found,
            gpui::Hsla {
              h: 44.0 / 360.0,
              s: 0.95,
              l: 0.62,
              a: 0.45,
            },
          )),
      ),
  )
  .into_any_element()
}

fn highlighted_preview(found: &ProjectSearchMatch, color: gpui::Hsla) -> StyledText {
  let highlights = found
    .match_range
    .clone()
    .map(|range| {
      vec![(
        range,
        HighlightStyle {
          background_color: Some(color),
          ..Default::default()
        },
      )]
    })
    .unwrap_or_default();
  StyledText::new(found.preview.clone()).with_highlights(highlights)
}

fn search_project_files(
  checkout_root: &Path,
  files: &[PathBuf],
  query: &str,
  options: SearchOptions,
) -> Vec<ProjectSearchFileResults> {
  let query = query.trim();
  if query.is_empty() {
    return Vec::new();
  }

  let matcher = match SearchMatcher::new(query, options) {
    Ok(matcher) => matcher,
    Err(_) => return Vec::new(),
  };
  let mut grouped = Vec::new();
  let mut total_results = 0;

  for path in files {
    if total_results >= MAX_SEARCH_RESULTS {
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

    let mut matches = Vec::new();
    for (line_index, line) in contents.lines().enumerate() {
      if total_results >= MAX_SEARCH_RESULTS {
        break;
      }
      let Some((start, end)) = matcher.find(line) else {
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
      matches.push(ProjectSearchMatch {
        line_number: line_index.saturating_add(1) as u32,
        column: line[..start].chars().count().saturating_add(1) as u32,
        preview: preview.into(),
        match_range,
      });
      total_results += 1;
    }

    if !matches.is_empty() {
      grouped.push(ProjectSearchFileResults {
        path: path.clone(),
        matches,
      });
    }
  }

  grouped
}

struct SearchMatcher {
  regex: regex::Regex,
  options: SearchOptions,
}

impl SearchMatcher {
  fn new(query: &str, options: SearchOptions) -> Result<Self, String> {
    let pattern = if options.regex {
      query.to_string()
    } else {
      regex::escape(query)
    };
    let regex = regex::RegexBuilder::new(&pattern)
      .case_insensitive(!options.effective_case_sensitive(query))
      .build()
      .map_err(|error| error.to_string())?;
    Ok(Self { regex, options })
  }

  fn find(&self, line: &str) -> Option<(usize, usize)> {
    self.regex.find_iter(line).find_map(|found| {
      let start = found.start();
      let end = found.end();
      if start == end {
        return None;
      }
      if self.options.whole_word && !is_whole_word_match(line, start, end) {
        return None;
      }
      (line.is_char_boundary(start) && line.is_char_boundary(end)).then_some((start, end))
    })
  }
}

fn is_whole_word_match(line_text: &str, byte_start: usize, byte_end: usize) -> bool {
  let previous_is_word = line_text[..byte_start]
    .chars()
    .next_back()
    .is_some_and(is_find_word_char);
  let next_is_word = line_text[byte_end..]
    .chars()
    .next()
    .is_some_and(is_find_word_char);
  !previous_is_word && !next_is_word
}

fn is_find_word_char(ch: char) -> bool {
  ch.is_alphanumeric() || ch == '_'
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::TempDir;

  #[test]
  fn project_search_groups_matches_by_file() {
    let temp = TempDir::new("project-search");
    std::fs::create_dir_all(temp.path.join("src")).expect("create src");
    std::fs::write(temp.path.join("src/lib.rs"), "first\nlet needle = true;\n").expect("write lib");
    std::fs::write(temp.path.join("README.md"), "needle\n").expect("write readme");

    let results = search_project_files(
      &temp.path,
      &[PathBuf::from("src/lib.rs"), PathBuf::from("README.md")],
      "needle",
      SearchOptions::default(),
    );

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].path, PathBuf::from("src/lib.rs"));
    assert_eq!(results[0].matches[0].line_number, 2);
    assert_eq!(results[0].matches[0].column, 5);
    assert_eq!(results[1].path, PathBuf::from("README.md"));
  }
}
