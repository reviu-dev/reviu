use std::{
  collections::HashSet,
  fs::File,
  io::{BufRead, BufReader},
  ops::Range,
  path::{Path, PathBuf},
  rc::Rc,
  sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
  },
  time::Duration,
};

use editor::{FindNext, FindPrevious, SearchOptions};
use gpui::{
  AnyElement, App, Context, Entity, FocusHandle, Focusable, HighlightStyle, IntoElement,
  ParentElement, Render, ScrollStrategy, SharedString, Styled, StyledText, Subscription, Task,
  Window, div, img, prelude::*, px, size,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Icon, IconName, Selectable, Sizable as _,
  VirtualListScrollHandle,
  button::{Button, ButtonVariants as _},
  h_flex,
  input::{Input, InputEvent, InputState},
  list::ListItem,
  scroll::Scrollbar,
  spinner::Spinner,
  v_flex, v_virtual_list,
};
use ui::{FILE_ICON_SIZE_PX, UiIconName, file_icon_path_for_path_with_theme};

use crate::project_files::list_project_search_files_with_options;

const MAX_SEARCH_RESULTS: usize = 500;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const SEARCH_RESULT_BATCH_SIZE: usize = 32;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);
const FILE_HEADER_ROW_HEIGHT: f32 = 40.0;
const MATCH_ROW_HEIGHT: f32 = 36.0;
pub(crate) const PROJECT_SEARCH_CONTEXT: &str = "ProjectSearch";

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

enum ProjectSearchUpdate {
  Batch(Vec<ProjectSearchFileResults>),
  Finished { limit_reached: bool },
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
  include_input: Entity<InputState>,
  exclude_input: Entity<InputState>,
  _query_subscription: Subscription,
  _include_subscription: Subscription,
  _exclude_subscription: Subscription,
  results: Vec<ProjectSearchFileResults>,
  rows: Vec<ProjectSearchRow>,
  collapsed_files: HashSet<PathBuf>,
  options: SearchOptions,
  loading_files: bool,
  searching: bool,
  limit_reached: bool,
  filters_open: bool,
  active_match_index: Option<usize>,
  include_ignored: bool,
  include_hidden: bool,
  error: Option<SharedString>,
  on_open: ProjectSearchHandler,
  scroll_handle: VirtualListScrollHandle,
  search_generation: u64,
  latest_search_generation: Arc<AtomicU64>,
  _search_task: Task<()>,
  _files_task: Task<()>,
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
    let include_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Include: e.g. src/**/*.rs"));
    let exclude_input =
      cx.new(|cx| InputState::new(window, cx).placeholder("Exclude: e.g. vendor/*, *.lock"));
    let query_subscription = cx.subscribe_in(&query_input, window, Self::on_query_input_event);
    let include_subscription = cx.subscribe_in(&include_input, window, Self::on_filter_input_event);
    let exclude_subscription = cx.subscribe_in(&exclude_input, window, Self::on_filter_input_event);
    Self {
      focus_handle: cx.focus_handle(),
      checkout_root,
      files: Arc::new(files),
      query_input,
      include_input,
      exclude_input,
      _query_subscription: query_subscription,
      _include_subscription: include_subscription,
      _exclude_subscription: exclude_subscription,
      results: Vec::new(),
      rows: Vec::new(),
      collapsed_files: HashSet::new(),
      options: cx
        .try_global::<SearchOptions>()
        .copied()
        .unwrap_or_default(),
      loading_files,
      searching: false,
      limit_reached: false,
      filters_open: false,
      active_match_index: None,
      include_ignored: false,
      include_hidden: false,
      error: None,
      on_open,
      scroll_handle: VirtualListScrollHandle::new(),
      search_generation: 0,
      latest_search_generation: Arc::new(AtomicU64::new(0)),
      _search_task: Task::ready(()),
      _files_task: Task::ready(()),
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
        if let Some(request) = self
          .active_open_request()
          .or_else(|| self.first_open_request())
        {
          self.open_result(request, window, cx);
        }
      }
      _ => {}
    }
  }

  fn on_filter_input_event(
    &mut self,
    _input: &Entity<InputState>,
    event: &InputEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if matches!(event, InputEvent::Change) {
      self.refresh_results(cx);
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

  fn filter_texts(&self, cx: &App) -> (String, String) {
    (
      self.include_input.read(cx).value().to_string(),
      self.exclude_input.read(cx).value().to_string(),
    )
  }

  fn toggle_filters(&mut self, cx: &mut Context<Self>) {
    self.filters_open = !self.filters_open;
    cx.notify();
  }

  fn toggle_include_ignored(&mut self, cx: &mut Context<Self>) {
    self.include_ignored = !self.include_ignored;
    self.reload_files(cx);
  }

  fn toggle_include_hidden(&mut self, cx: &mut Context<Self>) {
    self.include_hidden = !self.include_hidden;
    self.reload_files(cx);
  }

  fn reload_files(&mut self, cx: &mut Context<Self>) {
    self.search_generation = self.search_generation.wrapping_add(1);
    self
      .latest_search_generation
      .store(self.search_generation, Ordering::Relaxed);
    self.loading_files = true;
    self.searching = false;
    self.limit_reached = false;
    self.results.clear();
    self.rows.clear();
    self.active_match_index = None;
    self.error = None;

    let checkout_root = self.checkout_root.clone();
    let include_ignored = self.include_ignored;
    let include_hidden = self.include_hidden;
    self._files_task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          list_project_search_files_with_options(&checkout_root, include_ignored, include_hidden)
        })
        .await;

      let _ = this.update(cx, |this, cx| match result {
        Ok(files) => {
          this.files = Arc::new(files);
          this.loading_files = false;
          this.refresh_results(cx);
        }
        Err(error) => {
          log::error!("load files for project search filters: {error:#}");
          this.loading_files = false;
          this.error = Some("Could not load project files".into());
          cx.notify();
        }
      });
    });
    cx.notify();
  }

  fn refresh_results(&mut self, cx: &mut Context<Self>) {
    let query = self.query_input.read(cx).value().to_string();
    self.search_generation = self.search_generation.wrapping_add(1);
    let generation = self.search_generation;
    self
      .latest_search_generation
      .store(generation, Ordering::Relaxed);

    if query.trim().is_empty() {
      self.searching = false;
      self.limit_reached = false;
      self.results.clear();
      self.rows.clear();
      self.active_match_index = None;
      self._search_task = Task::ready(());
      cx.notify();
      return;
    }

    self.searching = true;
    self.limit_reached = false;
    self.error = None;
    self.results.clear();
    self.rows.clear();
    self.active_match_index = None;
    let checkout_root = self.checkout_root.clone();
    let files = self.files.clone();
    let options = self.options;
    let (include_patterns, exclude_patterns) = self.filter_texts(cx);
    let latest_generation = self.latest_search_generation.clone();
    self._search_task = cx.spawn(async move |this, cx| {
      cx.background_executor().timer(SEARCH_DEBOUNCE).await;

      let (update_sender, update_receiver) = async_channel::bounded(8);
      let search = cx.background_spawn(async move {
        search_project_files_streaming(
          &checkout_root,
          files.as_ref(),
          &query,
          options,
          include_patterns,
          exclude_patterns,
          SearchCancellation {
            generation,
            latest_generation,
          },
          update_sender,
        );
      });

      while let Ok(update) = update_receiver.recv().await {
        let _ = this.update(cx, |this, cx| {
          if this.search_generation != generation {
            return;
          }
          match update {
            ProjectSearchUpdate::Batch(batch) => {
              this.results.extend(batch);
              this.collapsed_files.retain(|path| {
                this
                  .results
                  .iter()
                  .any(|result| result.path.as_path() == path.as_path())
              });
              this.sync_result_rows();
              this.ensure_active_match();
            }
            ProjectSearchUpdate::Finished { limit_reached } => {
              this.searching = false;
              this.limit_reached = limit_reached;
            }
          }
          cx.notify();
        });
      }

      search.await;
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

  fn ensure_active_match(&mut self) {
    let result_count = self.result_count();
    self.active_match_index = match (self.active_match_index, result_count) {
      (_, 0) => None,
      (Some(index), count) => Some(index.min(count.saturating_sub(1))),
      (None, _) => Some(0),
    };
  }

  fn active_open_request(&self) -> Option<ProjectSearchOpenRequest> {
    let (result_index, match_index) =
      match_indices_for_flat_index(&self.results, self.active_match_index?)?;
    let file = self.results.get(result_index)?;
    let found = file.matches.get(match_index)?;
    Some(ProjectSearchOpenRequest {
      path: file.path.clone(),
      line: Some(found.line_number),
      column: Some(found.column),
    })
  }

  fn select_match(&mut self, index: usize, cx: &mut Context<Self>) {
    let result_count = self.result_count();
    if result_count == 0 {
      self.active_match_index = None;
      cx.notify();
      return;
    }

    let index = index.min(result_count.saturating_sub(1));
    let Some((result_index, match_index)) = match_indices_for_flat_index(&self.results, index)
    else {
      self.active_match_index = None;
      cx.notify();
      return;
    };

    if let Some(file) = self.results.get(result_index) {
      let path = file.path.clone();
      if self.collapsed_files.remove(&path) {
        self.sync_result_rows();
      }
    }

    self.active_match_index = Some(index);
    if let Some(row_index) = self.row_index_for_match(result_index, match_index) {
      self
        .scroll_handle
        .scroll_to_item(row_index, ScrollStrategy::Center);
    }
    cx.notify();
  }

  fn select_next_match(&mut self, cx: &mut Context<Self>) {
    let result_count = self.result_count();
    if result_count == 0 {
      self.active_match_index = None;
      cx.notify();
      return;
    }

    let next = self
      .active_match_index
      .map(|index| (index + 1) % result_count)
      .unwrap_or(0);
    self.select_match(next, cx);
  }

  fn select_previous_match(&mut self, cx: &mut Context<Self>) {
    let result_count = self.result_count();
    if result_count == 0 {
      self.active_match_index = None;
      cx.notify();
      return;
    }

    let previous = self
      .active_match_index
      .map(|index| {
        if index == 0 {
          result_count.saturating_sub(1)
        } else {
          index.saturating_sub(1)
        }
      })
      .unwrap_or_else(|| result_count.saturating_sub(1));
    self.select_match(previous, cx);
  }

  fn row_index_for_match(&self, result_index: usize, match_index: usize) -> Option<usize> {
    self.rows.iter().position(|row| {
      matches!(
        row,
        ProjectSearchRow::Match {
          result_index: row_result_index,
          match_index: row_match_index,
        } if *row_result_index == result_index && *row_match_index == match_index
      )
    })
  }

  fn active_match_number(&self) -> usize {
    self
      .active_match_index
      .filter(|index| *index < self.result_count())
      .map(|index| index + 1)
      .unwrap_or(0)
  }

  fn find_next_action(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
    self.select_next_match(cx);
  }

  fn find_previous_action(&mut self, _: &FindPrevious, _: &mut Window, cx: &mut Context<Self>) {
    self.select_previous_match(cx);
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

  fn result_count_label(&self, result_count: usize) -> String {
    if result_count == 0 {
      if self.searching {
        "...".to_string()
      } else {
        "0/0".to_string()
      }
    } else {
      let suffix = if self.limit_reached {
        "+"
      } else if self.searching {
        "..."
      } else {
        ""
      };
      format!("{}/{}{}", self.active_match_number(), result_count, suffix)
    }
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
        let active =
          self.active_match_index == flat_index_for_match(&self.results, result_index, match_index);
        let flat_index = flat_index_for_match(&self.results, result_index, match_index);
        Some(render_match_row(
          file.path.clone(),
          found,
          flat_index,
          active,
          cx,
        ))
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
    let include_input = self.include_input.clone();
    let exclude_input = self.exclude_input.clone();
    let query = query_input.read(cx).value().to_string();
    let result_count = self.result_count();
    let result_count_label = self.result_count_label(result_count);
    let case_sensitive = self.options.case_sensitive;
    let whole_word = self.options.whole_word;
    let regex = self.options.regex;
    let filters_open = self.filters_open;
    let include_ignored = self.include_ignored;
    let include_hidden = self.include_hidden;
    let entity = cx.entity();

    v_flex()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .track_focus(&self.focus_handle)
      .key_context(PROJECT_SEARCH_CONTEXT)
      .on_action(cx.listener(Self::find_next_action))
      .on_action(cx.listener(Self::find_previous_action))
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
                          .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                              entity.update(cx, |view, cx| view.toggle_regex(cx));
                            }
                          }),
                      ),
                  ),
              )
              .child(
                h_flex()
                  .w(px(224.0))
                  .gap_2()
                  .items_center()
                  .child(
                    Button::new("project-search-filters")
                      .icon(IconName::Settings2)
                      .ghost()
                      .xsmall()
                      .compact()
                      .selected(filters_open)
                      .tooltip("Search filters")
                      .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                          entity.update(cx, |view, cx| view.toggle_filters(cx));
                        }
                      }),
                  )
                  .child(div().h(px(18.0)).w(px(1.0)).bg(theme.border))
                  .child(
                    Button::new("project-search-prev")
                      .icon(IconName::ChevronLeft)
                      .ghost()
                      .xsmall()
                      .compact()
                      .tooltip("Previous match")
                      .disabled(result_count == 0)
                      .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                          entity.update(cx, |view, cx| view.select_previous_match(cx));
                        }
                      }),
                  )
                  .child(
                    Button::new("project-search-next")
                      .icon(IconName::ChevronRight)
                      .ghost()
                      .xsmall()
                      .compact()
                      .tooltip("Next match")
                      .disabled(result_count == 0)
                      .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                          entity.update(cx, |view, cx| view.select_next_match(cx));
                        }
                      }),
                  )
                  .child(
                    h_flex()
                      .w(px(88.0))
                      .gap_1()
                      .items_center()
                      .text_xs()
                      .text_color(theme.muted_foreground)
                      .when(self.searching, |this| this.child(Spinner::new().small()))
                      .child(result_count_label),
                  ),
              ),
          )
          .when(filters_open, |this| {
            this.child(
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
                    .pl_1()
                    .pr_1()
                    .border_1()
                    .border_color(theme.border)
                    .rounded_md()
                    .bg(theme.background)
                    .child(
                      Input::new(&include_input)
                        .small()
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false),
                    ),
                )
                .child(
                  div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(32.0))
                    .flex()
                    .items_center()
                    .pl_1()
                    .pr_1()
                    .border_1()
                    .border_color(theme.border)
                    .rounded_md()
                    .bg(theme.background)
                    .child(
                      Input::new(&exclude_input)
                        .small()
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false),
                    ),
                )
                .child(
                  h_flex()
                    .w(px(224.0))
                    .gap_2()
                    .items_center()
                    .child(
                      Button::new("project-search-include-ignored")
                        .icon(UiIconName::FileX)
                        .ghost()
                        .xsmall()
                        .compact()
                        .selected(include_ignored)
                        .tooltip("Include ignored files")
                        .on_click({
                          let entity = cx.entity();
                          move |_, _, cx| {
                            entity.update(cx, |view, cx| view.toggle_include_ignored(cx));
                          }
                        }),
                    )
                    .child(
                      Button::new("project-search-include-hidden")
                        .icon(UiIconName::Eye)
                        .ghost()
                        .xsmall()
                        .compact()
                        .selected(include_hidden)
                        .tooltip("Include hidden files")
                        .on_click({
                          let entity = cx.entity();
                          move |_, _, cx| {
                            entity.update(cx, |view, cx| view.toggle_include_hidden(cx));
                          }
                        }),
                    ),
                ),
            )
          }),
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
    .h(px(FILE_HEADER_ROW_HEIGHT))
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
  flat_index: Option<usize>,
  active: bool,
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
  .h(px(MATCH_ROW_HEIGHT))
  .py_1p5()
  .selected(active)
  .on_click(move |_, window, cx| {
    let request = request.clone();
    entity.update(cx, |view, cx| {
      view.active_match_index = flat_index;
      view.open_result(request, window, cx);
    });
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

fn match_indices_for_flat_index(
  results: &[ProjectSearchFileResults],
  target_index: usize,
) -> Option<(usize, usize)> {
  let mut offset = 0;
  for (result_index, file) in results.iter().enumerate() {
    let next_offset = offset + file.matches.len();
    if target_index < next_offset {
      return Some((result_index, target_index - offset));
    }
    offset = next_offset;
  }
  None
}

fn flat_index_for_match(
  results: &[ProjectSearchFileResults],
  result_index: usize,
  match_index: usize,
) -> Option<usize> {
  let file = results.get(result_index)?;
  if match_index >= file.matches.len() {
    return None;
  }

  Some(
    results
      .iter()
      .take(result_index)
      .map(|file| file.matches.len())
      .sum::<usize>()
      + match_index,
  )
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

struct SearchCancellation {
  generation: u64,
  latest_generation: Arc<AtomicU64>,
}

#[derive(Default)]
struct ProjectSearchCompletion {
  limit_reached: bool,
}

impl SearchCancellation {
  fn is_cancelled(&self) -> bool {
    self.latest_generation.load(Ordering::Relaxed) != self.generation
  }
}

#[cfg(test)]
fn search_project_files(
  checkout_root: &Path,
  files: &[PathBuf],
  query: &str,
  options: SearchOptions,
) -> Vec<ProjectSearchFileResults> {
  let mut results = Vec::new();
  let _ = search_project_files_batched(
    checkout_root,
    files,
    query,
    options,
    "",
    "",
    None,
    |batch| {
      results.extend(batch);
      true
    },
  );
  results
}

fn search_project_files_streaming(
  checkout_root: &Path,
  files: &[PathBuf],
  query: &str,
  options: SearchOptions,
  include_patterns: String,
  exclude_patterns: String,
  cancellation: SearchCancellation,
  update_sender: async_channel::Sender<ProjectSearchUpdate>,
) {
  let completion = search_project_files_batched(
    checkout_root,
    files,
    query,
    options,
    &include_patterns,
    &exclude_patterns,
    Some(&cancellation),
    |batch| {
      update_sender
        .send_blocking(ProjectSearchUpdate::Batch(batch))
        .is_ok()
    },
  );
  let _ = update_sender.send_blocking(ProjectSearchUpdate::Finished {
    limit_reached: completion.limit_reached,
  });
}

fn search_project_files_batched(
  checkout_root: &Path,
  files: &[PathBuf],
  query: &str,
  options: SearchOptions,
  include_patterns: &str,
  exclude_patterns: &str,
  cancellation: Option<&SearchCancellation>,
  mut on_batch: impl FnMut(Vec<ProjectSearchFileResults>) -> bool,
) -> ProjectSearchCompletion {
  let query = query.trim();
  if query.is_empty() {
    return ProjectSearchCompletion::default();
  }

  let matcher = match SearchMatcher::new(query, options) {
    Ok(matcher) => matcher,
    Err(_) => return ProjectSearchCompletion::default(),
  };
  let path_filter = SearchPathFilter::new(include_patterns, exclude_patterns);
  let mut total_results = 0;
  let mut batch_results = 0;
  let mut batch = Vec::new();

  for path in files {
    if total_results >= MAX_SEARCH_RESULTS
      || cancellation.is_some_and(SearchCancellation::is_cancelled)
    {
      break;
    }

    if !path_filter.matches(path) {
      continue;
    }

    let absolute_path = checkout_root.join(path);
    if std::fs::metadata(&absolute_path).map_or(true, |metadata| {
      !metadata.is_file() || metadata.len() > MAX_FILE_BYTES
    }) {
      continue;
    }

    let Ok(file) = File::open(&absolute_path) else {
      continue;
    };

    let mut matches = Vec::new();
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_number = 0u32;
    loop {
      if total_results >= MAX_SEARCH_RESULTS
        || cancellation.is_some_and(SearchCancellation::is_cancelled)
      {
        break;
      }
      line.clear();
      let Ok(bytes_read) = reader.read_line(&mut line) else {
        break;
      };
      if bytes_read == 0 {
        break;
      }
      line_number = line_number.saturating_add(1);
      let line = line.trim_end_matches(['\r', '\n']);
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
        line_number,
        column: line[..start].chars().count().saturating_add(1) as u32,
        preview: preview.into(),
        match_range,
      });
      total_results += 1;
    }

    if !matches.is_empty() {
      batch_results += matches.len();
      batch.push(ProjectSearchFileResults {
        path: path.clone(),
        matches,
      });
      if batch_results >= SEARCH_RESULT_BATCH_SIZE {
        if !on_batch(std::mem::take(&mut batch)) {
          return ProjectSearchCompletion {
            limit_reached: total_results >= MAX_SEARCH_RESULTS,
          };
        }
        batch_results = 0;
      }
    }
  }

  if !batch.is_empty() {
    let _ = on_batch(batch);
  }

  ProjectSearchCompletion {
    limit_reached: total_results >= MAX_SEARCH_RESULTS,
  }
}

struct SearchPathFilter {
  includes: Vec<regex::Regex>,
  excludes: Vec<regex::Regex>,
}

impl SearchPathFilter {
  fn new(include_patterns: &str, exclude_patterns: &str) -> Self {
    Self {
      includes: compile_path_patterns(include_patterns),
      excludes: compile_path_patterns(exclude_patterns),
    }
  }

  fn matches(&self, path: &Path) -> bool {
    let path = path
      .to_string_lossy()
      .replace(std::path::MAIN_SEPARATOR, "/");
    let included =
      self.includes.is_empty() || self.includes.iter().any(|regex| regex.is_match(&path));
    let excluded = self.excludes.iter().any(|regex| regex.is_match(&path));
    included && !excluded
  }
}

fn compile_path_patterns(patterns: &str) -> Vec<regex::Regex> {
  patterns
    .split([',', '\n'])
    .map(str::trim)
    .filter(|pattern| !pattern.is_empty())
    .filter_map(|pattern| regex::Regex::new(&glob_pattern_to_regex(pattern)).ok())
    .collect()
}

fn glob_pattern_to_regex(pattern: &str) -> String {
  let pattern = pattern.replace(std::path::MAIN_SEPARATOR, "/");
  let mut regex = String::from("^");
  let mut chars = pattern.chars().peekable();
  while let Some(ch) = chars.next() {
    match ch {
      '*' if chars.peek() == Some(&'*') => {
        chars.next();
        if chars.peek() == Some(&'/') {
          chars.next();
          regex.push_str("(?:.*/)?");
        } else {
          regex.push_str(".*");
        }
      }
      '*' => regex.push_str("[^/]*"),
      '?' => regex.push_str("[^/]"),
      _ => regex.push_str(&regex::escape(&ch.to_string())),
    }
  }
  regex.push('$');
  regex
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

  #[test]
  fn project_search_batches_results_progressively() {
    let temp = TempDir::new("project-search-batches");
    let files = (0..(SEARCH_RESULT_BATCH_SIZE + 1))
      .map(|index| {
        let path = PathBuf::from(format!("file-{index}.txt"));
        std::fs::write(temp.path.join(&path), "needle\n").expect("write file");
        path
      })
      .collect::<Vec<_>>();
    let mut batch_sizes = Vec::new();

    let completion = search_project_files_batched(
      &temp.path,
      &files,
      "needle",
      SearchOptions::default(),
      "",
      "",
      None,
      |batch| {
        batch_sizes.push(batch.len());
        true
      },
    );

    assert_eq!(batch_sizes, vec![SEARCH_RESULT_BATCH_SIZE, 1]);
    assert!(!completion.limit_reached);
  }

  #[test]
  fn project_search_filters_paths() {
    let temp = TempDir::new("project-search-filters");
    std::fs::create_dir_all(temp.path.join("src/bin")).expect("create src");
    std::fs::create_dir_all(temp.path.join("vendor")).expect("create vendor");
    std::fs::write(temp.path.join("src/lib.rs"), "needle\n").expect("write lib");
    std::fs::write(temp.path.join("src/bin/main.rs"), "needle\n").expect("write main");
    std::fs::write(temp.path.join("vendor/lib.rs"), "needle\n").expect("write vendor");
    std::fs::write(temp.path.join("README.md"), "needle\n").expect("write readme");
    let files = vec![
      PathBuf::from("src/lib.rs"),
      PathBuf::from("src/bin/main.rs"),
      PathBuf::from("vendor/lib.rs"),
      PathBuf::from("README.md"),
    ];
    let mut paths = Vec::new();

    let completion = search_project_files_batched(
      &temp.path,
      &files,
      "needle",
      SearchOptions::default(),
      "src/**/*.rs",
      "src/bin/*",
      None,
      |batch| {
        paths.extend(batch.into_iter().map(|file| file.path));
        true
      },
    );

    assert_eq!(paths, vec![PathBuf::from("src/lib.rs")]);
    assert!(!completion.limit_reached);
  }

  #[test]
  fn project_search_maps_flat_match_indices() {
    let results = vec![
      ProjectSearchFileResults {
        path: PathBuf::from("src/lib.rs"),
        matches: vec![
          ProjectSearchMatch {
            line_number: 1,
            column: 1,
            preview: "needle".into(),
            match_range: Some(0..6),
          },
          ProjectSearchMatch {
            line_number: 2,
            column: 3,
            preview: "needle".into(),
            match_range: Some(0..6),
          },
        ],
      },
      ProjectSearchFileResults {
        path: PathBuf::from("README.md"),
        matches: vec![ProjectSearchMatch {
          line_number: 1,
          column: 1,
          preview: "needle".into(),
          match_range: Some(0..6),
        }],
      },
    ];

    assert_eq!(match_indices_for_flat_index(&results, 0), Some((0, 0)));
    assert_eq!(match_indices_for_flat_index(&results, 1), Some((0, 1)));
    assert_eq!(match_indices_for_flat_index(&results, 2), Some((1, 0)));
    assert_eq!(match_indices_for_flat_index(&results, 3), None);
    assert_eq!(flat_index_for_match(&results, 1, 0), Some(2));
    assert_eq!(flat_index_for_match(&results, 1, 1), None);
  }

  #[test]
  fn project_search_reports_when_the_result_limit_is_reached() {
    let temp = TempDir::new("project-search-limit");
    let files = (0..=MAX_SEARCH_RESULTS)
      .map(|index| {
        let path = PathBuf::from(format!("file-{index}.txt"));
        std::fs::write(temp.path.join(&path), "needle\n").expect("write file");
        path
      })
      .collect::<Vec<_>>();
    let mut result_count = 0;

    let completion = search_project_files_batched(
      &temp.path,
      &files,
      "needle",
      SearchOptions::default(),
      "",
      "",
      None,
      |batch| {
        result_count += batch.iter().map(|file| file.matches.len()).sum::<usize>();
        true
      },
    );

    assert_eq!(result_count, MAX_SEARCH_RESULTS);
    assert!(completion.limit_reached);
  }
}
