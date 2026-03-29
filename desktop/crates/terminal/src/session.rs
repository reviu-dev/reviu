use std::{
  path::{Path, PathBuf},
  sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::{Receiver, Sender, channel},
  },
};

use alacritty_terminal::{
  event::{Event, EventListener, WindowSize},
  event_loop::{EventLoop, EventLoopSender, Msg},
  grid::{Dimensions, Scroll},
  index::{Column, Point},
  selection::{Selection, SelectionType},
  sync::FairMutex,
  term::{
    Config, Term, TermMode, cell::Flags, color::Colors, point_to_viewport, viewport_to_point,
  },
  tty,
  vte::ansi::{Color, CursorShape},
};
use anyhow::{Context as _, Result};
use gpui::{Modifiers, MouseButton};
use parking_lot::Mutex;

use crate::input;

const MIN_COLUMNS: u16 = 12;
const MIN_LINES: u16 = 4;
const DEFAULT_CELL_WIDTH_PX: u16 = 8;
const DEFAULT_CELL_HEIGHT_PX: u16 = 16;

static NEXT_WINDOW_ID: AtomicU64 = AtomicU64::new(1);

pub type ClipboardLoadFormatter = Arc<dyn Fn(&str) -> String + Sync + Send + 'static>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalBounds {
  pub columns: u16,
  pub lines: u16,
  pub cell_width: u16,
  pub cell_height: u16,
}

impl Default for TerminalBounds {
  fn default() -> Self {
    Self {
      columns: 120,
      lines: 32,
      cell_width: DEFAULT_CELL_WIDTH_PX,
      cell_height: DEFAULT_CELL_HEIGHT_PX,
    }
  }
}

impl TerminalBounds {
  pub fn from_viewport(width_px: f32, height_px: f32) -> Self {
    Self::from_size(
      width_px,
      height_px,
      DEFAULT_CELL_WIDTH_PX,
      DEFAULT_CELL_HEIGHT_PX,
    )
  }

  pub fn from_size(width_px: f32, height_px: f32, cell_width: u16, cell_height: u16) -> Self {
    let cell_width = cell_width.max(1);
    let cell_height = cell_height.max(1);
    let usable_width = width_px
      .max(0.0)
      .max(f32::from(MIN_COLUMNS) * f32::from(cell_width));
    let usable_height = height_px
      .max(0.0)
      .max(f32::from(MIN_LINES) * f32::from(cell_height));

    let columns =
      ((usable_width / f32::from(cell_width)).next_up().floor() as u16).max(MIN_COLUMNS);
    let lines = ((usable_height / f32::from(cell_height)).next_up().floor() as u16).max(MIN_LINES);

    Self {
      columns,
      lines,
      cell_width,
      cell_height,
    }
  }

  pub fn window_size(self) -> WindowSize {
    WindowSize {
      num_lines: self.lines,
      num_cols: self.columns,
      cell_width: self.cell_width,
      cell_height: self.cell_height,
    }
  }
}

impl Dimensions for TerminalBounds {
  fn total_lines(&self) -> usize {
    usize::from(self.lines)
  }

  fn screen_lines(&self) -> usize {
    usize::from(self.lines)
  }

  fn columns(&self) -> usize {
    usize::from(self.columns)
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportPoint {
  pub row: usize,
  pub col: usize,
}

impl ViewportPoint {
  fn clamped(self, rows: usize, cols: usize) -> Self {
    Self {
      row: self.row.min(rows.saturating_sub(1)),
      col: self.col.min(cols.saturating_sub(1)),
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportSelectionRange {
  pub start: ViewportPoint,
  pub end: ViewportPoint,
}

impl ViewportSelectionRange {
  pub fn normalized(self) -> Self {
    if (self.start.row, self.start.col) <= (self.end.row, self.end.col) {
      self
    } else {
      Self {
        start: self.end,
        end: self.start,
      }
    }
  }

  pub fn contains(self, point: ViewportPoint) -> bool {
    let normalized = self.normalized();
    (point.row, point.col) >= (normalized.start.row, normalized.start.col)
      && (point.row, point.col) <= (normalized.end.row, normalized.end.col)
  }

  pub fn is_collapsed(self) -> bool {
    self.start == self.end
  }

  fn clamped(self, rows: usize, cols: usize) -> Self {
    Self {
      start: self.start.clamped(rows, cols),
      end: self.end.clamped(rows, cols),
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalSelectionMode {
  Simple,
  Semantic,
  Lines,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCellSnapshot {
  pub row: usize,
  pub col: usize,
  pub c: char,
  pub fg: Color,
  pub bg: Color,
  pub flags: Flags,
  pub underline_color: Option<Color>,
  pub hyperlink_uri: Option<Arc<str>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalCursorSnapshot {
  pub point: ViewportPoint,
  pub shape: CursorShape,
}

#[derive(Clone)]
pub struct ScreenSnapshot {
  pub rows: usize,
  pub cols: usize,
  pub display_offset: usize,
  pub colors: Colors,
  pub cells: Vec<TerminalCellSnapshot>,
  pub cursor: Option<TerminalCursorSnapshot>,
  pub title: Option<String>,
  pub mode: TermMode,
  pub exit_status: Option<String>,
}

impl Default for ScreenSnapshot {
  fn default() -> Self {
    Self {
      rows: 0,
      cols: 0,
      display_offset: 0,
      colors: Colors::default(),
      cells: Vec::new(),
      cursor: None,
      title: None,
      mode: TermMode::default(),
      exit_status: None,
    }
  }
}

#[derive(Default)]
pub struct SessionPollResult {
  pub changed: bool,
  pub clipboard_store: Vec<String>,
  pub clipboard_load_requests: Vec<ClipboardLoadFormatter>,
}

#[derive(Clone)]
struct TerminalListener {
  event_tx: Sender<Event>,
  pty_tx: Arc<Mutex<Option<EventLoopSender>>>,
  window_size: Arc<Mutex<WindowSize>>,
}

impl TerminalListener {
  fn new(event_tx: Sender<Event>, window_size: WindowSize) -> Self {
    Self {
      event_tx,
      pty_tx: Arc::new(Mutex::new(None)),
      window_size: Arc::new(Mutex::new(window_size)),
    }
  }

  fn attach_sender(&self, sender: EventLoopSender) {
    *self.pty_tx.lock() = Some(sender);
  }

  fn set_window_size(&self, window_size: WindowSize) {
    *self.window_size.lock() = window_size;
  }

  fn write_to_pty(&self, bytes: Vec<u8>) -> bool {
    let Some(sender) = self.pty_tx.lock().clone() else {
      return false;
    };
    sender.send(Msg::Input(bytes.into())).is_ok()
  }
}

impl EventListener for TerminalListener {
  fn send_event(&self, event: Event) {
    match &event {
      Event::PtyWrite(text) => {
        if self.write_to_pty(text.as_bytes().to_vec()) {
          return;
        }
      }
      Event::TextAreaSizeRequest(formatter) => {
        let response = formatter(*self.window_size.lock());
        if self.write_to_pty(response.into_bytes()) {
          return;
        }
      }
      _ => {}
    }

    let _ = self.event_tx.send(event);
  }
}

pub struct TerminalSession {
  bounds: TerminalBounds,
  term: Arc<FairMutex<Term<TerminalListener>>>,
  event_rx: Receiver<Event>,
  pty_tx: EventLoopSender,
  listener: TerminalListener,
  working_directory: PathBuf,
  title: Option<String>,
  exit_status: Option<String>,
}

impl TerminalSession {
  pub fn spawn(working_directory: PathBuf, bounds: TerminalBounds) -> Result<Self> {
    if !working_directory.exists() {
      anyhow::bail!(
        "Working directory does not exist: {}",
        working_directory.display()
      );
    }

    let config = Config {
      scrolling_history: 20_000,
      ..Config::default()
    };
    let window_size = bounds.window_size();
    let (event_tx, event_rx) = channel();
    let listener = TerminalListener::new(event_tx, window_size);
    let term = Arc::new(FairMutex::new(Term::new(config, &bounds, listener.clone())));
    let window_id = NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed);
    let pty = tty::new(&tty_options(&working_directory), window_size, window_id)
      .with_context(|| format!("Failed to create PTY in {}", working_directory.display()))?;

    let event_loop = EventLoop::new(term.clone(), listener.clone(), pty, false, false)
      .context("Failed to create terminal event loop")?;
    let pty_tx = event_loop.channel();
    listener.attach_sender(pty_tx.clone());
    let _io_thread = event_loop.spawn();

    Ok(Self {
      bounds,
      term,
      event_rx,
      pty_tx,
      listener,
      working_directory,
      title: None,
      exit_status: None,
    })
  }

  pub fn working_directory(&self) -> &Path {
    &self.working_directory
  }

  pub fn bounds(&self) -> TerminalBounds {
    self.bounds
  }

  pub fn resize(&mut self, bounds: TerminalBounds) {
    if self.bounds == bounds {
      return;
    }

    self.bounds = bounds;
    let window_size = bounds.window_size();
    self.listener.set_window_size(window_size);
    let _ = self.pty_tx.send(Msg::Resize(window_size));
    self.term.lock().resize(bounds);
  }

  pub fn snapshot(&self) -> ScreenSnapshot {
    snapshot_from_term(
      &self.term.lock(),
      self.title.clone(),
      self.exit_status.clone(),
    )
  }

  pub fn mode(&self) -> TermMode {
    *self.term.lock().mode()
  }

  pub fn mouse_mode_enabled(&self) -> bool {
    input::mouse_mode_enabled(self.mode())
  }

  pub fn can_report_mouse_move(&self, pressed_button: Option<MouseButton>) -> bool {
    input::can_report_mouse_move(self.mode(), pressed_button)
  }

  pub fn poll(&mut self) -> SessionPollResult {
    let mut result = SessionPollResult::default();

    while let Ok(event) = self.event_rx.try_recv() {
      match event {
        Event::Wakeup => {
          result.changed = true;
        }
        Event::Title(title) => {
          self.title = Some(title);
          result.changed = true;
        }
        Event::ResetTitle => {
          self.title = None;
          result.changed = true;
        }
        Event::ClipboardStore(_, text) => {
          result.clipboard_store.push(text);
        }
        Event::ClipboardLoad(_, formatter) => {
          result.clipboard_load_requests.push(formatter);
        }
        Event::PtyWrite(text) => {
          self.send_text(text);
        }
        Event::TextAreaSizeRequest(formatter) => {
          self.send_text(formatter(self.bounds.window_size()));
        }
        Event::ChildExit(status) => {
          self.exit_status = Some(match status.code() {
            Some(code) => format!("Shell exited with code {code}."),
            None => "Shell exited.".to_string(),
          });
          result.changed = true;
        }
        Event::Exit => {
          self.exit_status = Some("Terminal requested shutdown.".to_string());
          result.changed = true;
        }
        Event::MouseCursorDirty
        | Event::CursorBlinkingChange
        | Event::Bell
        | Event::ColorRequest(_, _) => {}
      }
    }

    result
  }

  pub fn send_key_down(&mut self, event: &gpui::KeyDownEvent) -> bool {
    let Some(text) = input::encode_key_down(event, self.mode()) else {
      return false;
    };

    self.send_text(text);
    true
  }

  pub fn paste(&mut self, text: &str) {
    self.send_text(input::encode_paste(text, self.mode()));
  }

  pub fn scroll_display(&mut self, delta_lines: i32) {
    if delta_lines == 0 {
      return;
    }

    self.term.lock().scroll_display(Scroll::Delta(delta_lines));
  }

  pub fn send_mouse_press(
    &mut self,
    button: MouseButton,
    point: ViewportPoint,
    modifiers: Modifiers,
  ) -> bool {
    let Some(sequence) =
      input::encode_mouse_press(button, point.row, point.col, modifiers, self.mode())
    else {
      return false;
    };
    self.send_text(sequence);
    true
  }

  pub fn send_mouse_release(
    &mut self,
    button: MouseButton,
    point: ViewportPoint,
    modifiers: Modifiers,
  ) -> bool {
    let Some(sequence) =
      input::encode_mouse_release(button, point.row, point.col, modifiers, self.mode())
    else {
      return false;
    };
    self.send_text(sequence);
    true
  }

  pub fn send_mouse_move(
    &mut self,
    point: ViewportPoint,
    pressed_button: Option<MouseButton>,
    modifiers: Modifiers,
  ) -> bool {
    let Some(sequence) =
      input::encode_mouse_move(point.row, point.col, pressed_button, modifiers, self.mode())
    else {
      return false;
    };
    self.send_text(sequence);
    true
  }

  pub fn send_scroll(
    &mut self,
    delta_lines: i32,
    point: ViewportPoint,
    modifiers: Modifiers,
  ) -> bool {
    let Some(sequence) =
      input::encode_scroll(delta_lines, point.row, point.col, modifiers, self.mode())
    else {
      return false;
    };
    self.send_text(sequence);
    true
  }

  pub fn selection_text(&self, range: ViewportSelectionRange) -> Option<String> {
    selection_text_for_term(&self.term.lock(), range)
  }

  pub fn selection_range_for_mode(
    &self,
    start: ViewportPoint,
    end: ViewportPoint,
    mode: TerminalSelectionMode,
  ) -> Option<ViewportSelectionRange> {
    selection_range_for_term(&self.term.lock(), start, end, mode)
  }

  fn send_text(&self, text: String) {
    if text.is_empty() {
      return;
    }
    let _ = self.pty_tx.send(Msg::Input(text.into_bytes().into()));
  }
}

impl Drop for TerminalSession {
  fn drop(&mut self) {
    let _ = self.pty_tx.send(Msg::Shutdown);
  }
}

fn tty_options(working_directory: &Path) -> tty::Options {
  let mut options = tty::Options {
    working_directory: Some(working_directory.to_path_buf()),
    ..tty::Options::default()
  };
  options
    .env
    .insert("TERM".to_string(), "xterm-256color".to_string());
  options
    .env
    .insert("COLORTERM".to_string(), "truecolor".to_string());
  options
    .env
    .insert("TERM_PROGRAM".to_string(), "Reviu".to_string());
  options
}

fn snapshot_from_term<T: EventListener>(
  term: &Term<T>,
  title: Option<String>,
  exit_status: Option<String>,
) -> ScreenSnapshot {
  let rows = term.screen_lines();
  let cols = term.columns();
  let renderable = term.renderable_content();
  let display_offset = renderable.display_offset;
  let colors = *renderable.colors;
  let mode = renderable.mode;
  let cursor = point_to_viewport(display_offset, renderable.cursor.point).map(|point| {
    TerminalCursorSnapshot {
      point: ViewportPoint {
        row: point.line,
        col: point.column.0,
      },
      shape: renderable.cursor.shape,
    }
  });

  let mut cells = Vec::new();
  for cell in renderable.display_iter {
    let Some(point) = point_to_viewport(display_offset, cell.point) else {
      continue;
    };
    if point.line >= rows
      || cell
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
      continue;
    }

    cells.push(TerminalCellSnapshot {
      row: point.line,
      col: point.column.0,
      c: match cell.c {
        '\t' => ' ',
        ch => ch,
      },
      fg: cell.fg,
      bg: cell.bg,
      flags: cell.flags,
      underline_color: cell.underline_color(),
      hyperlink_uri: cell
        .hyperlink()
        .map(|hyperlink| Arc::<str>::from(hyperlink.uri().to_owned())),
    });
  }

  ScreenSnapshot {
    rows,
    cols,
    display_offset,
    colors,
    cells,
    cursor,
    title,
    mode,
    exit_status,
  }
}

fn selection_text_for_term<T: EventListener>(
  term: &Term<T>,
  range: ViewportSelectionRange,
) -> Option<String> {
  let rows = term.screen_lines();
  let cols = term.columns();
  if rows == 0 || cols == 0 {
    return None;
  }

  let range = range.normalized().clamped(rows, cols);
  let display_offset = term.grid().display_offset();
  let start = viewport_to_point(
    display_offset,
    Point::new(range.start.row, Column(range.start.col)),
  );
  let end = viewport_to_point(
    display_offset,
    Point::new(range.end.row, Column(range.end.col)),
  );
  Some(term.bounds_to_string(start, end))
}

fn selection_range_for_term<T: EventListener>(
  term: &Term<T>,
  start: ViewportPoint,
  end: ViewportPoint,
  mode: TerminalSelectionMode,
) -> Option<ViewportSelectionRange> {
  let rows = term.screen_lines();
  let cols = term.columns();
  if rows == 0 || cols == 0 {
    return None;
  }

  let start = start.clamped(rows, cols);
  let end = end.clamped(rows, cols);
  let display_offset = term.grid().display_offset();
  let start = viewport_to_point(display_offset, Point::new(start.row, Column(start.col)));
  let end = viewport_to_point(display_offset, Point::new(end.row, Column(end.col)));

  let mut selection = Selection::new(
    selection_type_for_mode(mode),
    start,
    alacritty_terminal::index::Side::Left,
  );
  selection.update(end, alacritty_terminal::index::Side::Right);
  let range = selection.to_range(term)?;
  let start = point_to_viewport(display_offset, range.start)?;
  let end = point_to_viewport(display_offset, range.end)?;

  Some(
    ViewportSelectionRange {
      start: ViewportPoint {
        row: start.line,
        col: start.column.0,
      },
      end: ViewportPoint {
        row: end.line,
        col: end.column.0,
      },
    }
    .clamped(rows, cols)
    .normalized(),
  )
}

fn selection_type_for_mode(mode: TerminalSelectionMode) -> SelectionType {
  match mode {
    TerminalSelectionMode::Simple => SelectionType::Simple,
    TerminalSelectionMode::Semantic => SelectionType::Semantic,
    TerminalSelectionMode::Lines => SelectionType::Lines,
  }
}

#[cfg(test)]
mod tests {
  use super::{
    TerminalBounds, TerminalSelectionMode, ViewportPoint, ViewportSelectionRange,
    selection_range_for_term, selection_text_for_term, snapshot_from_term,
  };
  use alacritty_terminal::{
    Term,
    event::VoidListener,
    term::{Config, cell::Flags},
    vte::ansi::{Processor, Rgb},
  };

  #[test]
  fn terminal_bounds_clamp_to_minimum_size() {
    let bounds = TerminalBounds::from_viewport(40.0, 40.0);

    assert_eq!(bounds.columns, 12);
    assert_eq!(bounds.lines, 4);
  }

  #[test]
  fn terminal_bounds_scale_with_viewport() {
    let bounds = TerminalBounds::from_viewport(1440.0, 960.0);

    assert!(bounds.columns >= 180);
    assert!(bounds.lines >= 60);
  }

  #[test]
  fn terminal_bounds_match_narrow_sidebar_dimensions() {
    let bounds = TerminalBounds::from_size(240.0, 640.0, 8, 16);

    assert_eq!(bounds.columns, 30);
    assert_eq!(bounds.lines, 40);
  }

  #[test]
  fn snapshot_captures_cells_and_cursor() {
    let bounds = TerminalBounds {
      columns: 8,
      lines: 3,
      ..TerminalBounds::default()
    };
    let mut term = Term::new(Config::default(), &bounds, VoidListener);
    let mut processor: Processor = Processor::new();

    processor.advance(&mut term, b"hi");

    let snapshot = snapshot_from_term(&term, Some("shell".to_string()), None);

    assert_eq!(snapshot.rows, 3);
    assert_eq!(snapshot.cols, 8);
    assert_eq!(snapshot.title.as_deref(), Some("shell"));
    assert_eq!(
      snapshot.cursor.map(|cursor| cursor.point),
      Some(ViewportPoint { row: 0, col: 2 })
    );
    assert!(
      snapshot
        .cells
        .iter()
        .any(|cell| cell.row == 0 && cell.col == 0 && cell.c == 'h')
    );
    assert!(
      snapshot
        .cells
        .iter()
        .any(|cell| cell.row == 0 && cell.col == 1 && cell.c == 'i')
    );
  }

  #[test]
  fn snapshot_captures_hyperlinks_and_underline_colors() {
    let bounds = TerminalBounds {
      columns: 8,
      lines: 3,
      ..TerminalBounds::default()
    };
    let mut term = Term::new(Config::default(), &bounds, VoidListener);
    let mut processor: Processor = Processor::new();

    processor.advance(
      &mut term,
      b"\x1b]8;;https://example.com\x07l\x1b]8;;\x07\x1b[58;2;255;0;255m\x1b[4:1mu\x1b[59m\x1b[24m",
    );

    let snapshot = snapshot_from_term(&term, None, None);
    let link_cell = snapshot
      .cells
      .iter()
      .find(|cell| cell.row == 0 && cell.col == 0)
      .expect("hyperlink cell should exist");
    let underline_cell = snapshot
      .cells
      .iter()
      .find(|cell| cell.row == 0 && cell.col == 1)
      .expect("underline cell should exist");

    assert_eq!(
      link_cell.hyperlink_uri.as_deref(),
      Some("https://example.com")
    );
    assert_eq!(link_cell.underline_color, None);
    assert_eq!(
      underline_cell.underline_color,
      Some(alacritty_terminal::vte::ansi::Color::Spec(Rgb {
        r: 255,
        g: 0,
        b: 255,
      }))
    );
    assert!(underline_cell.flags.contains(Flags::UNDERLINE));
  }

  #[test]
  fn selection_text_uses_viewport_coordinates() {
    let bounds = TerminalBounds {
      columns: 8,
      lines: 3,
      ..TerminalBounds::default()
    };
    let mut term = Term::new(Config::default(), &bounds, VoidListener);
    let mut processor: Processor = Processor::new();

    processor.advance(&mut term, b"hello");

    assert_eq!(
      selection_text_for_term(
        &term,
        ViewportSelectionRange {
          start: ViewportPoint { row: 0, col: 1 },
          end: ViewportPoint { row: 0, col: 3 },
        },
      )
      .as_deref(),
      Some("ell")
    );
  }

  #[test]
  fn viewport_selection_range_contains_points_inclusive() {
    let selection = ViewportSelectionRange {
      start: ViewportPoint { row: 3, col: 8 },
      end: ViewportPoint { row: 1, col: 4 },
    };

    assert!(selection.contains(ViewportPoint { row: 1, col: 4 }));
    assert!(selection.contains(ViewportPoint { row: 2, col: 0 }));
    assert!(selection.contains(ViewportPoint { row: 3, col: 8 }));
    assert!(!selection.contains(ViewportPoint { row: 1, col: 3 }));
    assert!(!selection.contains(ViewportPoint { row: 4, col: 0 }));
  }

  #[test]
  fn semantic_selection_range_expands_to_word_boundaries() {
    let bounds = TerminalBounds {
      columns: 16,
      lines: 3,
      ..TerminalBounds::default()
    };
    let mut term = Term::new(Config::default(), &bounds, VoidListener);
    let mut processor: Processor = Processor::new();

    processor.advance(&mut term, b"git status");

    assert_eq!(
      selection_range_for_term(
        &term,
        ViewportPoint { row: 0, col: 5 },
        ViewportPoint { row: 0, col: 5 },
        TerminalSelectionMode::Semantic,
      ),
      Some(ViewportSelectionRange {
        start: ViewportPoint { row: 0, col: 4 },
        end: ViewportPoint { row: 0, col: 9 },
      })
    );
  }

  #[test]
  fn line_selection_range_expands_to_entire_line() {
    let bounds = TerminalBounds {
      columns: 16,
      lines: 3,
      ..TerminalBounds::default()
    };
    let mut term = Term::new(Config::default(), &bounds, VoidListener);
    let mut processor: Processor = Processor::new();

    processor.advance(&mut term, b"git status\r\nnext");

    assert_eq!(
      selection_range_for_term(
        &term,
        ViewportPoint { row: 0, col: 2 },
        ViewportPoint { row: 0, col: 2 },
        TerminalSelectionMode::Lines,
      ),
      Some(ViewportSelectionRange {
        start: ViewportPoint { row: 0, col: 0 },
        end: ViewportPoint { row: 0, col: 15 },
      })
    );
  }
}
