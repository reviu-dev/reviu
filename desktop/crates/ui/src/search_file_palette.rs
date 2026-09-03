use std::{ops::Range, path::PathBuf, sync::Arc};

use crate::palette::{
  palette_empty, palette_footer, palette_list_item, palette_search_list, palette_section_header,
  update_selected_index,
};
use crate::{FILE_ICON_SIZE_PX, file_icon_path_for_path_with_theme};
use gpui::{
  AnyElement, App, Context, Div, Entity, FocusHandle, Focusable, HighlightStyle, IntoElement,
  ParentElement, Render, SharedString, Styled, StyledText, Subscription, Task, Window, div, img,
  prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _, WindowExt, h_flex,
  list::{ListDelegate, ListEvent, ListItem, ListState},
  spinner::Spinner,
  v_flex,
};

const MAX_SEARCH_RESULTS: usize = 100;
const INITIAL_PROJECT_RESULTS_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchFileGroup {
  Changed,
  Recent,
  #[default]
  Repository,
}

impl SearchFileGroup {
  fn label(self) -> &'static str {
    match self {
      Self::Changed => "Changed",
      Self::Recent => "Recent",
      Self::Repository => "Project",
    }
  }

  fn score_bonus(self) -> i64 {
    match self {
      Self::Changed => 250,
      Self::Recent => 100,
      Self::Repository => 0,
    }
  }
}

#[derive(Clone, Debug)]
pub struct SearchFileEntry {
  pub path: PathBuf,
  pub label: SharedString,
  pub group: SearchFileGroup,
}

impl SearchFileEntry {
  pub fn new(path: PathBuf, label: impl Into<SharedString>) -> Self {
    Self {
      path,
      label: label.into(),
      group: SearchFileGroup::Repository,
    }
  }

  pub fn in_group(mut self, group: SearchFileGroup) -> Self {
    self.group = group;
    self
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchFileOpenRequest {
  pub path: PathBuf,
  pub line: Option<u32>,
  pub column: Option<u32>,
}

pub type SearchFileHandler = Arc<
  dyn Fn(SearchFileOpenRequest, &mut Window, &mut App) -> Result<(), SharedString> + Send + Sync,
>;

pub struct SearchFilePaletteConfig {
  pub entries: Vec<SearchFileEntry>,
  pub on_open: SearchFileHandler,
  pub loading: bool,
}

impl SearchFilePaletteConfig {
  pub fn new(entries: Vec<SearchFileEntry>, on_open: SearchFileHandler) -> Self {
    Self {
      entries,
      on_open,
      loading: false,
    }
  }

  pub fn loading(mut self, loading: bool) -> Self {
    self.loading = loading;
    self
  }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SearchFileQuery {
  raw: String,
  path: String,
  line: Option<u32>,
  column: Option<u32>,
}

impl SearchFileQuery {
  fn parse(raw: &str) -> Self {
    let raw = raw.trim().to_string();
    let mut path = raw.as_str();
    let mut line = None;
    let mut column = None;

    if let Some((prefix, value)) = split_numeric_suffix(path) {
      path = prefix;
      line = Some(value);
      if let Some((prefix, value)) = split_numeric_suffix(path) {
        path = prefix;
        column = line;
        line = Some(value);
      }
    }

    let path = path.to_string();
    Self {
      raw,
      path,
      line,
      column,
    }
  }
}

fn split_numeric_suffix(value: &str) -> Option<(&str, u32)> {
  let (prefix, suffix) = value.rsplit_once(':')?;
  if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
    return None;
  }
  Some((prefix, suffix.parse().ok()?))
}

#[derive(Clone)]
struct SearchFileMatch {
  entry: Arc<SearchFileEntry>,
  positions: Vec<usize>,
}

struct SearchFileListDelegate {
  files: Arc<Vec<Arc<SearchFileEntry>>>,
  matched_sections: Vec<SearchFileSection>,
  selected_index: Option<IndexPath>,
  query: SearchFileQuery,
  search_generation: u64,
  loading: bool,
}

struct SearchFileSection {
  label: Option<SharedString>,
  files: Vec<SearchFileMatch>,
}

impl SearchFileListDelegate {
  fn replace_files(&mut self, entries: Vec<SearchFileEntry>, loading: bool) {
    self.files = Arc::new(entries.into_iter().map(Arc::new).collect());
    self.loading = loading;
    self.search_generation = self.search_generation.wrapping_add(1);
  }

  fn prepare_empty_query(&mut self) {
    self.matched_sections = build_initial_sections(self.files.as_ref());
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

  fn item_at(&self, ix: IndexPath) -> Option<&SearchFileMatch> {
    self
      .matched_sections
      .get(ix.section)
      .and_then(|section| section.files.get(ix.row))
  }

  fn open_request_at(&self, ix: IndexPath) -> Option<SearchFileOpenRequest> {
    let entry = self.item_at(ix)?.entry.clone();
    Some(SearchFileOpenRequest {
      path: entry.path.clone(),
      line: self.query.line,
      column: self.query.column,
    })
  }
}

fn build_initial_sections(files: &[Arc<SearchFileEntry>]) -> Vec<SearchFileSection> {
  let groups = [
    SearchFileGroup::Changed,
    SearchFileGroup::Recent,
    SearchFileGroup::Repository,
  ];

  groups
    .into_iter()
    .filter_map(|group| {
      let limit = match group {
        SearchFileGroup::Repository => INITIAL_PROJECT_RESULTS_LIMIT,
        SearchFileGroup::Changed | SearchFileGroup::Recent => usize::MAX,
      };
      let files = files
        .iter()
        .filter(|entry| entry.group == group)
        .take(limit)
        .cloned()
        .map(|entry| SearchFileMatch {
          entry,
          positions: Vec::new(),
        })
        .collect::<Vec<_>>();
      (!files.is_empty()).then(|| SearchFileSection {
        label: Some(group.label().into()),
        files,
      })
    })
    .collect()
}

#[derive(Debug)]
struct RankedFileMatch {
  entry: Arc<SearchFileEntry>,
  positions: Vec<usize>,
  score: i64,
  sort_key: String,
}

fn rank_file_matches(files: &[Arc<SearchFileEntry>], query: &str) -> Vec<SearchFileMatch> {
  let mut matches = files
    .iter()
    .filter_map(|entry| score_file_match(entry.clone(), query))
    .collect::<Vec<_>>();

  matches.sort_by(|left, right| {
    right
      .score
      .cmp(&left.score)
      .then_with(|| left.sort_key.cmp(&right.sort_key))
  });
  matches.truncate(MAX_SEARCH_RESULTS);
  matches
    .into_iter()
    .map(|matched| SearchFileMatch {
      entry: matched.entry,
      positions: matched.positions,
    })
    .collect()
}

fn score_file_match(entry: Arc<SearchFileEntry>, query: &str) -> Option<RankedFileMatch> {
  let label = entry.label.as_ref();
  let file_name = entry
    .path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or(label);
  let file_name_start = label.rfind(file_name).unwrap_or(0);
  let query_lower = query.to_lowercase();
  let file_name_lower = file_name.to_lowercase();
  let file_stem_lower = entry
    .path
    .file_stem()
    .and_then(|stem| stem.to_str())
    .unwrap_or(file_name)
    .to_lowercase();
  let label_lower = label.to_lowercase();

  let filename_match = fuzzy_subsequence(file_name, query).map(|(positions, quality)| {
    (
      positions
        .into_iter()
        .map(|position| position + file_name_start)
        .collect::<Vec<_>>(),
      quality,
    )
  });
  let path_match = fuzzy_subsequence(label, query);

  let (positions, base_score) = if file_name_lower == query_lower {
    (filename_match?.0, 20_000)
  } else if file_stem_lower == query_lower {
    (filename_match?.0, 18_000)
  } else if file_name_lower.starts_with(&query_lower) {
    (filename_match?.0, 14_000)
  } else if file_name_lower.contains(&query_lower) {
    (filename_match?.0, 12_000)
  } else if let Some((positions, quality)) = filename_match {
    (positions, 9_000 + quality)
  } else if label_lower == query_lower {
    (path_match?.0, 8_000)
  } else if label_lower.contains(&query_lower) {
    (path_match?.0, 6_000)
  } else {
    let (positions, quality) = path_match?;
    (positions, 3_000 + quality)
  };

  Some(RankedFileMatch {
    score: base_score + entry.group.score_bonus(),
    entry,
    positions,
    sort_key: label_lower,
  })
}

fn fuzzy_subsequence(candidate: &str, query: &str) -> Option<(Vec<usize>, i64)> {
  if query.is_empty() {
    return Some((Vec::new(), 0));
  }

  let mut positions = Vec::new();
  let mut candidate_chars = candidate.char_indices().peekable();
  for query_character in query.chars() {
    let position = loop {
      let (position, candidate_character) = candidate_chars.next()?;
      if characters_equal(candidate_character, query_character) {
        break position;
      }
    };
    positions.push(position);
  }

  let contiguous = positions
    .windows(2)
    .filter(|pair| {
      candidate[pair[0]..]
        .chars()
        .next()
        .is_some_and(|character| pair[0] + character.len_utf8() == pair[1])
    })
    .count() as i64;
  let boundaries = positions
    .iter()
    .filter(|position| {
      **position == 0
        || candidate[..**position]
          .chars()
          .next_back()
          .is_some_and(|character| matches!(character, '/' | '\\' | '_' | '-' | '.' | ' '))
    })
    .count() as i64;
  let span = positions
    .last()
    .zip(positions.first())
    .map_or(0, |(last, first)| last.saturating_sub(*first)) as i64;
  let start = positions.first().copied().unwrap_or_default() as i64;
  let quality = contiguous * 40 + boundaries * 25 - span - start;
  Some((positions, quality))
}

fn characters_equal(left: char, right: char) -> bool {
  if left.is_ascii() && right.is_ascii() {
    return left.eq_ignore_ascii_case(&right);
  }
  left.to_lowercase().eq(right.to_lowercase())
}

fn highlight_ranges(text: &str, positions: &[usize], source_offset: usize) -> Vec<Range<usize>> {
  let mut ranges = Vec::<Range<usize>>::new();
  for position in positions.iter().copied() {
    let Some(relative) = position.checked_sub(source_offset) else {
      continue;
    };
    if relative >= text.len() || !text.is_char_boundary(relative) {
      continue;
    }
    let Some(character) = text[relative..].chars().next() else {
      continue;
    };
    let range = relative..relative + character.len_utf8();
    if let Some(previous) = ranges.last_mut()
      && previous.end == range.start
    {
      previous.end = range.end;
    } else {
      ranges.push(range);
    }
  }
  ranges
}

fn highlighted_text(
  text: SharedString,
  positions: &[usize],
  source_offset: usize,
  color: gpui::Hsla,
) -> StyledText {
  let highlights: Vec<(Range<usize>, HighlightStyle)> =
    highlight_ranges(text.as_ref(), positions, source_offset)
      .into_iter()
      .map(|range| {
        (
          range,
          HighlightStyle {
            color: Some(color),
            ..Default::default()
          },
        )
      })
      .collect();
  StyledText::new(text).with_highlights(highlights)
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
    let theme = cx.theme().clone();
    let matched = self.item_at(ix)?.clone();
    let entry = matched.entry;
    let label = entry.label.as_ref();
    let file_icon: AnyElement = file_icon_path_for_path_with_theme(&entry.path, &theme)
      .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
      .unwrap_or_else(|| {
        Icon::new(IconName::File)
          .size_3()
          .text_color(theme.muted_foreground)
          .into_any_element()
      });
    let file_name = entry
      .path
      .file_name()
      .and_then(|name| name.to_str())
      .unwrap_or(label);
    let file_name_start = label.rfind(file_name).unwrap_or(0);
    let file_name: SharedString = file_name.to_string().into();
    let directory = label[..file_name_start]
      .trim_end_matches(['/', '\\'])
      .to_string();

    Some(
      palette_list_item(ix, self.selected_index).child(
        h_flex()
          .items_center()
          .gap_2()
          .w_full()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .min_w_0()
              .flex_shrink(1.)
              .child(file_icon)
              .child(
                div()
                  .text_sm()
                  .overflow_hidden()
                  .whitespace_nowrap()
                  .text_ellipsis()
                  .child(highlighted_text(
                    file_name,
                    &matched.positions,
                    file_name_start,
                    theme.blue,
                  )),
              ),
          )
          .when(!directory.is_empty(), |this| {
            let directory: SharedString = directory.into();
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
                .child(highlighted_text(
                  directory,
                  &matched.positions,
                  0,
                  theme.blue,
                )),
            )
          }),
      ),
    )
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
    Some(palette_section_header(label, cx))
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    if self.loading {
      return h_flex()
        .justify_center()
        .items_center()
        .gap_2()
        .py_8()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(Spinner::new().small())
        .child("Loading project files...")
        .into_any_element();
    }
    palette_empty(cx)
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
    window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Task<()> {
    self.query = SearchFileQuery::parse(query);
    self.search_generation = self.search_generation.wrapping_add(1);
    let generation = self.search_generation;

    if self.query.path.is_empty() {
      self.prepare_empty_query();
      return Task::ready(());
    }

    let files = self.files.clone();
    let query = self.query.path.clone();
    let search = cx.background_spawn(async move { rank_file_matches(files.as_ref(), &query) });
    cx.spawn_in(window, async move |this, cx| {
      let matches = search.await;
      let _ = this.update_in(cx, |state, window, cx| {
        if state.delegate().search_generation != generation {
          return;
        }
        state.delegate_mut().matched_sections = if matches.is_empty() {
          Vec::new()
        } else {
          vec![SearchFileSection {
            label: None,
            files: matches,
          }]
        };
        let selected = (state.delegate().matched_total_count() > 0).then(IndexPath::default);
        state.set_selected_index(selected, window, cx);
        cx.notify();
      });
    })
  }
}

pub struct SearchFilePalette {
  focus_handle: FocusHandle,
  files_list: Entity<ListState<SearchFileListDelegate>>,
  loading: bool,
  error: Option<SharedString>,
  on_open: Option<SearchFileHandler>,
  _subscriptions: Vec<Subscription>,
}

impl SearchFilePalette {
  pub fn new(window: &mut Window, cx: &mut Context<Self>, config: SearchFilePaletteConfig) -> Self {
    let files = Arc::new(config.entries.into_iter().map(Arc::new).collect::<Vec<_>>());
    let mut delegate = SearchFileListDelegate {
      files,
      matched_sections: Vec::new(),
      selected_index: None,
      query: SearchFileQuery::default(),
      search_generation: 0,
      loading: config.loading,
    };
    delegate.prepare_empty_query();
    let files_list = cx.new(|cx| ListState::new(delegate, window, cx).searchable(true));

    let _subscriptions = vec![cx.subscribe_in(
      &files_list,
      window,
      |palette, list_state, event: &ListEvent, window, cx| {
        if let ListEvent::Confirm(index) = event {
          let request = list_state.read(cx).delegate().open_request_at(*index);
          if let Some(request) = request {
            palette.open_file(request, window, cx);
          }
        }
      },
    )];

    cx.on_next_frame(window, |this, window, cx| this.focus_list(window, cx));

    Self {
      focus_handle: cx.focus_handle(),
      files_list,
      loading: config.loading,
      error: None,
      on_open: Some(config.on_open),
      _subscriptions,
    }
  }

  pub fn replace_entries(
    &mut self,
    entries: Vec<SearchFileEntry>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.loading = false;
    self.error = None;
    self.files_list.update(cx, |state, cx| {
      let query = state.delegate().query.raw.clone();
      state.delegate_mut().replace_files(entries, false);
      state.set_query(&query, window, cx);
    });
    cx.notify();
  }

  pub fn set_loading_error(&mut self, error: impl Into<SharedString>, cx: &mut Context<Self>) {
    self.loading = false;
    self.error = Some(error.into());
    self.files_list.update(cx, |state, cx| {
      state.delegate_mut().loading = false;
      cx.notify();
    });
    cx.notify();
  }

  fn focus_list(&self, window: &mut Window, cx: &mut Context<Self>) {
    self.files_list.update(cx, |state, cx| {
      state.focus(window, cx);
    });
  }

  fn open_file(
    &mut self,
    request: SearchFileOpenRequest,
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

  fn render_error(&self, theme: &gpui_component::Theme, error: &SharedString) -> Div {
    div()
      .px_3()
      .py_2()
      .text_sm()
      .text_color(theme.red)
      .child(error.clone())
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

    v_flex()
      .track_focus(&self.focus_handle)
      .child(palette_search_list(&self.files_list, "Search files..."))
      .when(
        self.loading && !self.files_list.read(cx).delegate().files.is_empty(),
        |parent| {
          parent.child(
            h_flex()
              .items_center()
              .gap_2()
              .px_3()
              .py_2()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(Spinner::new().small())
              .child("Refreshing project files..."),
          )
        },
      )
      .when_some(self.error.clone(), |parent, error| {
        parent.child(self.render_error(&theme, &error))
      })
      .child(palette_footer(true, "open", cx))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::Mutex;

  fn entry(path: &str, group: SearchFileGroup) -> Arc<SearchFileEntry> {
    Arc::new(SearchFileEntry::new(PathBuf::from(path), path).in_group(group))
  }

  #[test]
  fn initial_sections_follow_review_priority() {
    let files = vec![
      entry("README.md", SearchFileGroup::Repository),
      entry("src/lib.rs", SearchFileGroup::Changed),
      entry("src/main.rs", SearchFileGroup::Recent),
    ];

    let sections = build_initial_sections(&files);

    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0].label.as_deref(), Some("Changed"));
    assert_eq!(sections[1].label.as_deref(), Some("Recent"));
    assert_eq!(sections[2].label.as_deref(), Some("Project"));
  }

  #[test]
  fn exact_file_stem_beats_directory_match() {
    let files = vec![
      entry(".agents/skills/gpui/SKILL.md", SearchFileGroup::Changed),
      entry("AGENTS.md", SearchFileGroup::Repository),
      entry(".agents/references/action.md", SearchFileGroup::Repository),
    ];

    let matches = rank_file_matches(&files, "AGENTS");

    assert_eq!(matches[0].entry.path, PathBuf::from("AGENTS.md"));
  }

  #[test]
  fn changed_file_wins_between_comparable_matches() {
    let files = vec![
      entry("src/file_search.rs", SearchFileGroup::Repository),
      entry("tests/file_search.rs", SearchFileGroup::Changed),
    ];

    let matches = rank_file_matches(&files, "file_search");

    assert_eq!(matches[0].entry.path, PathBuf::from("tests/file_search.rs"));
  }

  #[test]
  fn fuzzy_search_matches_non_contiguous_characters() {
    let files = vec![entry(
      "desktop/crates/workspace/src/session_page.rs",
      SearchFileGroup::Repository,
    )];

    let matches = rank_file_matches(&files, "sspg");

    assert_eq!(matches.len(), 1);
    assert!(!matches[0].positions.is_empty());
  }

  #[test]
  fn search_and_initial_repository_results_are_limited() {
    let files = (0..150)
      .map(|index| entry(&format!("src/file_{index}.rs"), SearchFileGroup::Repository))
      .collect::<Vec<_>>();

    assert_eq!(rank_file_matches(&files, "file").len(), MAX_SEARCH_RESULTS);
    assert_eq!(
      build_initial_sections(&files)[0].files.len(),
      INITIAL_PROJECT_RESULTS_LIMIT
    );
  }

  #[test]
  fn query_parses_line_and_column() {
    assert_eq!(
      SearchFileQuery::parse("src/main.rs:42:7"),
      SearchFileQuery {
        raw: "src/main.rs:42:7".to_string(),
        path: "src/main.rs".to_string(),
        line: Some(42),
        column: Some(7),
      }
    );
    assert_eq!(SearchFileQuery::parse("src/main.rs:42").line, Some(42));
  }

  #[test]
  fn highlight_ranges_merge_contiguous_characters() {
    assert_eq!(
      highlight_ranges("AGENTS.md", &[0, 1, 2, 3, 4, 5], 0),
      vec![0..6]
    );
  }

  fn open_test_palette(
    cx: &mut gpui::TestAppContext,
    config: SearchFilePaletteConfig,
  ) -> (
    gpui::Entity<SearchFilePalette>,
    &mut gpui::VisualTestContext,
  ) {
    cx.update(gpui_component::init);
    let mut palette = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let view = cx.new(|cx| SearchFilePalette::new(window, cx, config));
      palette = Some(view.clone());
      gpui_component::Root::new(view, window, cx)
    });
    let palette = palette.expect("palette");
    cx.run_until_parked();
    palette.update_in(cx, |palette, window, cx| {
      palette.focus_list(window, cx);
    });
    cx.run_until_parked();
    (palette, cx)
  }

  #[gpui::test]
  async fn loading_and_error_states_are_replaced_by_loaded_entries(cx: &mut gpui::TestAppContext) {
    let handler: SearchFileHandler = Arc::new(|_, _, _| Ok(()));
    let (palette, cx) = open_test_palette(
      cx,
      SearchFilePaletteConfig::new(Vec::new(), handler).loading(true),
    );

    palette.read_with(cx, |palette, cx| {
      assert!(palette.loading);
      assert!(palette.files_list.read(cx).delegate().loading);
    });
    palette.update(cx, |palette, cx| {
      palette.set_loading_error("failed", cx);
    });
    palette.read_with(cx, |palette, cx| {
      assert!(!palette.loading);
      assert_eq!(palette.error.as_deref(), Some("failed"));
      assert!(!palette.files_list.read(cx).delegate().loading);
    });
    palette.update_in(cx, |palette, window, cx| {
      palette.replace_entries(
        vec![SearchFileEntry::new(
          PathBuf::from("src/main.rs"),
          "src/main.rs",
        )],
        window,
        cx,
      );
    });
    cx.run_until_parked();
    palette.read_with(cx, |palette, cx| {
      assert!(palette.error.is_none());
      assert_eq!(
        palette.files_list.read(cx).delegate().matched_total_count(),
        1
      );
    });
  }

  #[gpui::test]
  async fn a_query_started_while_loading_uses_entries_when_they_arrive(
    cx: &mut gpui::TestAppContext,
  ) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let handler: SearchFileHandler = Arc::new(move |request, _, _| {
      recorded.lock().expect("record request").push(request);
      Err("keep open".into())
    });
    let (palette, cx) = open_test_palette(
      cx,
      SearchFilePaletteConfig::new(Vec::new(), handler).loading(true),
    );

    cx.simulate_input("AGENTS");
    cx.run_until_parked();
    palette.update_in(cx, |palette, window, cx| {
      palette.replace_entries(
        vec![SearchFileEntry::new(
          PathBuf::from("AGENTS.md"),
          "AGENTS.md",
        )],
        window,
        cx,
      );
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert_eq!(
      requests.lock().expect("read requests").first(),
      Some(&SearchFileOpenRequest {
        path: PathBuf::from("AGENTS.md"),
        line: None,
        column: None,
      })
    );
  }

  #[gpui::test]
  async fn confirming_a_position_query_passes_the_line_to_the_handler(
    cx: &mut gpui::TestAppContext,
  ) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let handler: SearchFileHandler = Arc::new(move |request, _, _| {
      recorded.lock().expect("record request").push(request);
      Err("keep open".into())
    });
    let (_palette, cx) = open_test_palette(
      cx,
      SearchFilePaletteConfig::new(
        vec![SearchFileEntry::new(
          PathBuf::from("src/main.rs"),
          "src/main.rs",
        )],
        handler,
      ),
    );

    cx.simulate_input("main.rs:42:7");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert_eq!(
      requests.lock().expect("read requests").as_slice(),
      &[SearchFileOpenRequest {
        path: PathBuf::from("src/main.rs"),
        line: Some(42),
        column: Some(7),
      }]
    );
  }
}
