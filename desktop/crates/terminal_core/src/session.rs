use std::{
  ffi::OsString,
  path::{Path, PathBuf},
  sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
  },
};

use alacritty_terminal::{
  event::{Event, EventListener, WindowSize},
  event_loop::{EventLoop, EventLoopSender, Msg},
  grid::{Dimensions, Grid, Scroll},
  index::{Column, Direction, Line, Point},
  selection::{Selection, SelectionType},
  sync::FairMutex,
  term::{
    Config, Term, TermMode,
    cell::{Cell, Flags},
    color::Colors,
    point_to_viewport,
    search::{RegexIter, RegexSearch},
    viewport_to_point,
  },
  tty,
  vte::ansi::{Color, CursorShape, NamedColor},
};
use anyhow::{Context as _, Result};
use async_channel::{Receiver, Sender, unbounded};
use parking_lot::{Mutex, RwLock};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

const MIN_COLUMNS: u16 = 12;
const MIN_LINES: u16 = 4;
const DEFAULT_CELL_WIDTH_PX: u16 = 8;
const DEFAULT_CELL_HEIGHT_PX: u16 = 16;

const DEFAULT_SCROLLBACK_LINES: usize = 20_000;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSearchMatch {
  start: Point,
  end: Point,
}

pub struct TerminalSearchHandle {
  term: Arc<FairMutex<Term<TerminalListener>>>,
}

impl TerminalSearchHandle {
  pub fn find_matches(&self, query: &str) -> Vec<TerminalSearchMatch> {
    let term = self.term.lock();
    search_matches_for_term(&term, query)
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCellSnapshot {
  pub row: usize,
  pub col: usize,
  pub c: char,
  pub zerowidth: Arc<[char]>,
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

#[derive(Clone, Default)]
pub struct ScreenSnapshot {
  pub rows: usize,
  pub cols: usize,
  pub total_lines: usize,
  pub display_offset: usize,
  pub colors: Colors,
  pub cells: Vec<TerminalCellSnapshot>,
  pub cursor: Option<TerminalCursorSnapshot>,
  pub title: Option<String>,
  pub mode: TermMode,
  pub exit_status: Option<String>,
}

#[derive(Default)]
pub struct SessionEventResult {
  pub changed: bool,
  pub wakeup: bool,
  /// The process is gone and its last output is in the grid.
  pub exited: bool,
  pub clipboard_store: Vec<String>,
  pub clipboard_load_requests: Vec<ClipboardLoadFormatter>,
}

#[derive(Clone)]
struct TerminalListener {
  event_tx: Sender<Event>,
  pty_tx: Arc<Mutex<Option<EventLoopSender>>>,
  window_size: Arc<Mutex<WindowSize>>,
  wakeup_pending: Arc<AtomicBool>,
}

impl TerminalListener {
  fn new(event_tx: Sender<Event>, window_size: WindowSize) -> Self {
    Self {
      event_tx,
      pty_tx: Arc::new(Mutex::new(None)),
      window_size: Arc::new(Mutex::new(window_size)),
      wakeup_pending: Arc::new(AtomicBool::new(false)),
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

  fn acknowledge_wakeup(&self) {
    self.wakeup_pending.store(false, Ordering::Release);
  }
}

impl EventListener for TerminalListener {
  fn send_event(&self, event: Event) {
    if matches!(event, Event::Wakeup) && self.wakeup_pending.swap(true, Ordering::AcqRel) {
      return;
    }

    match &event {
      Event::PtyWrite(text) if self.write_to_pty(text.as_bytes().to_vec()) => {
        return;
      }
      Event::TextAreaSizeRequest(formatter) => {
        let response = formatter(*self.window_size.lock());
        if self.write_to_pty(response.into_bytes()) {
          return;
        }
      }
      _ => {}
    }

    let _ = self.event_tx.try_send(event);
  }
}

pub struct WorkingDirectoryTracker {
  process_id: Pid,
  tracked_process_id: Mutex<Option<Pid>>,
  system: Mutex<System>,
  current: RwLock<PathBuf>,
  /// Our own duplicate of the PTY master, to ask which process group owns
  /// the terminal; the event loop owns and closes the original.
  #[cfg(unix)]
  pty_file: Option<std::fs::File>,
  running_command: RwLock<Option<String>>,
  refresh_state: AtomicU8,
}

impl WorkingDirectoryTracker {
  fn new(process_id: u32, working_directory: PathBuf, pty_file: Option<std::fs::File>) -> Self {
    #[cfg(not(unix))]
    let _ = pty_file;

    Self {
      process_id: Pid::from_u32(process_id),
      tracked_process_id: Mutex::new(None),
      system: Mutex::new(System::new()),
      current: RwLock::new(working_directory),
      #[cfg(unix)]
      pty_file,
      running_command: RwLock::new(None),
      refresh_state: AtomicU8::new(0),
    }
  }

  pub fn begin_refresh(&self) -> bool {
    loop {
      match self.refresh_state.load(Ordering::Acquire) {
        0 => {
          if self
            .refresh_state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
          {
            return true;
          }
        }
        1 => {
          match self
            .refresh_state
            .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
          {
            Ok(_) => return false,
            Err(0) => continue,
            Err(_) => return false,
          }
        }
        _ => return false,
      }
    }
  }

  pub fn refresh(&self) -> Option<PathBuf> {
    let mut latest = None;
    loop {
      let next = self.refresh_working_directory();
      let running_command = self.refresh_running_command();
      *self.running_command.write() = running_command;
      if let Some(next) = next {
        *self.current.write() = next.clone();
        latest = Some(next);
      }

      match self
        .refresh_state
        .compare_exchange(1, 0, Ordering::AcqRel, Ordering::Acquire)
      {
        Ok(_) => return latest,
        Err(2) => {
          if self
            .refresh_state
            .compare_exchange(2, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
          {
            continue;
          }
        }
        Err(_) => return latest,
      }
    }
  }

  fn refresh_working_directory(&self) -> Option<PathBuf> {
    let mut system = self.system.lock();
    let mut tracked_process_id = self.tracked_process_id.lock();

    if let Some(process_id) = *tracked_process_id {
      system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[process_id]),
        true,
        ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always),
      );
      if let Some(path) = process_working_directory(&system, process_id) {
        return Some(path);
      }
      *tracked_process_id = None;
    }

    system.refresh_processes_specifics(
      ProcessesToUpdate::All,
      true,
      ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always),
    );
    if let Some(root_process) = system.process(self.process_id)
      && root_process.name() != "login"
      && let Some(path) = process_working_directory(&system, self.process_id)
    {
      *tracked_process_id = Some(self.process_id);
      return Some(path);
    }

    if let Some((process_id, path)) = closest_descendant_working_directory(&system, self.process_id)
    {
      *tracked_process_id = Some(process_id);
      return Some(path);
    }

    None
  }

  pub fn current(&self) -> PathBuf {
    self.current.read().clone()
  }

  /// The command the user started from the shell, `None` while the shell
  /// itself waits at its prompt.
  pub fn running_command(&self) -> Option<String> {
    self.running_command.read().clone()
  }

  fn refresh_running_command(&self) -> Option<String> {
    let group = self.foreground_process_group()?;
    let mut system = self.system.lock();
    system.refresh_processes_specifics(
      ProcessesToUpdate::Some(&[group, self.process_id]),
      true,
      ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    let process = system.process(group)?;
    // On macOS the PTY runs `login`, which starts the shell as its child.
    let shell_is_child_of_login = process.parent() == Some(self.process_id)
      && system
        .process(self.process_id)
        .is_some_and(|root| root.name() == "login");
    if group == self.process_id || shell_is_child_of_login {
      return None;
    }
    command_label(process.cmd(), process.name())
  }

  #[cfg(unix)]
  fn foreground_process_group(&self) -> Option<Pid> {
    use std::os::fd::AsRawFd as _;
    let file = self.pty_file.as_ref()?;
    // SAFETY: `tcgetpgrp` only reads the terminal's state, and the descriptor
    // stays open as long as `file`, which this tracker owns.
    let group = unsafe { libc::tcgetpgrp(file.as_raw_fd()) };
    u32::try_from(group)
      .ok()
      .filter(|group| *group > 0)
      .map(Pid::from_u32)
  }

  #[cfg(not(unix))]
  fn foreground_process_group(&self) -> Option<Pid> {
    None
  }
}

const COMMAND_LABEL_MAX_CHARS: usize = 40;

/// Script runners hide the tool behind them: `node /usr/local/bin/npm run
/// dev` reads better as `npm run dev`.
const SCRIPT_RUNNERS: &[&str] = &["node", "python", "python3", "ruby", "perl", "bun", "deno"];

fn command_label(arguments: &[std::ffi::OsString], name: &std::ffi::OsStr) -> Option<String> {
  let arguments = arguments
    .iter()
    .map(|argument| argument.to_string_lossy().into_owned())
    .collect::<Vec<_>>();
  let program_name = |argument: &str| {
    Path::new(argument)
      .file_name()
      .map(|name| name.to_string_lossy().into_owned())
      .unwrap_or_else(|| argument.to_string())
  };
  let mut words = match arguments.split_first() {
    None => vec![name.to_string_lossy().into_owned()],
    Some((program, rest)) => {
      let program = program_name(program);
      match rest.split_first() {
        Some((script, rest))
          if SCRIPT_RUNNERS.contains(&program.as_str()) && !script.starts_with('-') =>
        {
          std::iter::once(program_name(script))
            .chain(rest.iter().cloned())
            .collect()
        }
        _ => std::iter::once(program)
          .chain(rest.iter().cloned())
          .collect(),
      }
    }
  };
  words.retain(|word| !word.is_empty());
  let label = words.join(" ");
  if label.is_empty() {
    return None;
  }
  if label.chars().count() <= COMMAND_LABEL_MAX_CHARS {
    return Some(label);
  }
  let mut truncated = label
    .chars()
    .take(COMMAND_LABEL_MAX_CHARS - 1)
    .collect::<String>();
  truncated.push('…');
  Some(truncated)
}

fn process_working_directory(system: &System, process_id: Pid) -> Option<PathBuf> {
  system
    .process(process_id)
    .and_then(|process| process.cwd())
    .filter(|path| path.is_dir())
    .map(Path::to_path_buf)
}

fn closest_descendant_working_directory(system: &System, root: Pid) -> Option<(Pid, PathBuf)> {
  system
    .processes()
    .iter()
    .filter_map(|(process_id, process)| {
      let depth = process_depth(system, *process_id, root)?;
      if depth == 0 {
        return None;
      }
      let path = process.cwd()?.to_path_buf();
      path.is_dir().then_some((*process_id, path, depth))
    })
    .min_by_key(|(_, _, depth)| *depth)
    .map(|(process_id, path, _)| (process_id, path))
}

fn process_depth(system: &System, process_id: Pid, root: Pid) -> Option<usize> {
  if process_id == root {
    return Some(0);
  }

  let mut current = process_id;
  for depth in 1..=64 {
    let parent = system.process(current)?.parent()?;
    if parent == root {
      return Some(depth);
    }
    if parent == current {
      return None;
    }
    current = parent;
  }
  None
}

pub struct TerminalSession {
  bounds: TerminalBounds,
  term: Arc<FairMutex<Term<TerminalListener>>>,
  event_rx: Receiver<Event>,
  pty_tx: EventLoopSender,
  listener: TerminalListener,
  working_directory: Arc<WorkingDirectoryTracker>,
  #[cfg(unix)]
  child_process_id: u32,
  scrollback_lines: usize,
  title: Option<String>,
  exit_status: Option<String>,
  child_exit: Option<std::process::ExitStatus>,
}

/// A program to run in its own PTY instead of the user's login shell.
pub struct ProgramOptions {
  pub program: String,
  pub args: Vec<String>,
  pub env: std::collections::HashMap<String, String>,
  pub scrollback_lines: usize,
}

/// How much of the output's end `tail_text` returns.
#[derive(Clone, Copy, Debug)]
pub enum TailBudget {
  Lines(usize),
  Bytes(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TailText {
  pub text: String,
  /// Earlier output exists that `text` leaves out.
  pub clipped: bool,
}

#[cfg(not(windows))]
fn shell_process_id(pty: &tty::Pty) -> u32 {
  pty.child().id()
}

#[cfg(unix)]
fn pty_file_for_tracking(pty: &tty::Pty) -> Result<Option<std::fs::File>> {
  let file = pty
    .file()
    .try_clone()
    .context("Failed to duplicate the PTY descriptor")?;
  Ok(Some(file))
}

#[cfg(windows)]
fn shell_process_id(pty: &tty::Pty) -> u32 {
  pty
    .child_watcher()
    .pid()
    .map(std::num::NonZeroU32::get)
    .unwrap_or_default()
}

impl TerminalSession {
  pub fn spawn(working_directory: PathBuf, bounds: TerminalBounds) -> Result<Self> {
    Self::spawn_with(working_directory, bounds, None)
  }

  /// Runs `program` directly in a PTY: it sees a terminal, so tools keep
  /// their colors and progress output, but nothing reads a login profile.
  pub fn spawn_program(
    working_directory: PathBuf,
    bounds: TerminalBounds,
    program: ProgramOptions,
  ) -> Result<Self> {
    Self::spawn_with(working_directory, bounds, Some(program))
  }

  fn spawn_with(
    working_directory: PathBuf,
    bounds: TerminalBounds,
    program: Option<ProgramOptions>,
  ) -> Result<Self> {
    if !working_directory.exists() {
      anyhow::bail!(
        "Working directory does not exist: {}",
        working_directory.display()
      );
    }

    let scrollback_lines = program
      .as_ref()
      .map_or(DEFAULT_SCROLLBACK_LINES, |program| program.scrollback_lines);
    let config = Config {
      scrolling_history: scrollback_lines,
      ..Config::default()
    };
    let window_size = bounds.window_size();
    let (event_tx, event_rx) = unbounded();
    let listener = TerminalListener::new(event_tx, window_size);
    let term = Arc::new(FairMutex::new(Term::new(config, &bounds, listener.clone())));
    let window_id = NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed);
    let mut options = tty_options(&working_directory, std::env::var_os("LANG"));
    let drain_on_exit = program.is_some();
    if let Some(program) = program {
      options.shell = Some(tty::Shell::new(program.program, program.args));
      options.env.extend(program.env);
      options.drain_on_exit = true;
    }
    let pty = tty::new(&options, window_size, window_id)
      .with_context(|| format!("Failed to create PTY in {}", working_directory.display()))?;
    let child_process_id = shell_process_id(&pty);
    #[cfg(unix)]
    let working_directory = Arc::new(WorkingDirectoryTracker::new(
      child_process_id,
      working_directory,
      pty_file_for_tracking(&pty)?,
    ));
    #[cfg(not(unix))]
    let working_directory = Arc::new(WorkingDirectoryTracker::new(
      child_process_id,
      working_directory,
      None,
    ));

    let event_loop = EventLoop::new(term.clone(), listener.clone(), pty, drain_on_exit, false)
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
      #[cfg(unix)]
      child_process_id,
      scrollback_lines,
      title: None,
      exit_status: None,
      child_exit: None,
    })
  }

  /// How the process ended, once it has.
  pub fn child_exit(&self) -> Option<std::process::ExitStatus> {
    self.child_exit
  }

  /// Kills the process and everything it started: it leads its own group.
  #[cfg(unix)]
  pub fn kill(&self) -> bool {
    let Ok(group) = i32::try_from(self.child_process_id) else {
      return false;
    };
    if group <= 0 {
      return false;
    }
    // SAFETY: `killpg` only sends a signal; the group is the child's own
    // session, created by the PTY spawn, so it never reaches Reviu itself.
    unsafe { libc::killpg(group, libc::SIGKILL) == 0 }
  }

  #[cfg(not(unix))]
  pub fn kill(&self) -> bool {
    self.pty_tx.send(Msg::Shutdown).is_ok()
  }

  /// The end of the output, oldest line first. Lines the terminal wrapped
  /// come back whole, and `styled` keeps colors as SGR sequences.
  pub fn tail_text(&self, budget: TailBudget, styled: bool) -> TailText {
    let term = self.term.lock();
    let mut tail = tail_text_for_term(&term, budget, styled);
    if term.grid().history_size() >= self.scrollback_lines && self.scrollback_lines > 0 {
      tail.clipped = true;
    }
    tail
  }

  /// Starts the shell off the calling thread. Call it from the main thread:
  /// the shell inherits the signal mask of the thread that forks it, and the
  /// executor's pool threads block signals, which would leave Ctrl-C dead.
  pub fn spawn_on_thread(
    working_directory: PathBuf,
    bounds: TerminalBounds,
  ) -> Result<Receiver<Result<Self>>> {
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::Builder::new()
      .name("terminal-spawn".to_string())
      .spawn(move || {
        // A closed channel means the view went away before the shell was
        // ready; dropping the session shuts the shell down.
        sender
          .send_blocking(Self::spawn(working_directory, bounds))
          .ok();
      })
      .context("Failed to start the terminal")?;
    Ok(receiver)
  }

  pub fn working_directory(&self) -> PathBuf {
    self.working_directory.current()
  }

  pub fn working_directory_tracker(&self) -> Arc<WorkingDirectoryTracker> {
    Arc::clone(&self.working_directory)
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

  pub fn event_receiver(&self) -> Receiver<Event> {
    self.event_rx.clone()
  }

  pub fn search_handle(&self) -> TerminalSearchHandle {
    TerminalSearchHandle {
      term: Arc::clone(&self.term),
    }
  }

  pub fn scroll_to_search_match(&mut self, found: TerminalSearchMatch) {
    self.term.lock().scroll_to_point(found.start);
  }

  pub fn visible_search_ranges(
    &self,
    matches: &[TerminalSearchMatch],
  ) -> Vec<ViewportSelectionRange> {
    let term = self.term.lock();
    matches
      .iter()
      .filter_map(|found| search_match_to_viewport(&term, *found))
      .collect()
  }

  pub fn process_events(&mut self, events: impl IntoIterator<Item = Event>) -> SessionEventResult {
    let mut result = SessionEventResult::default();

    for event in events {
      match event {
        Event::Wakeup => {
          self.listener.acknowledge_wakeup();
          result.changed = true;
          result.wakeup = true;
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
          self.child_exit = Some(status);
          self.exit_status = Some(match status.code() {
            Some(code) => format!("Shell exited with code {code}."),
            None => "Shell exited.".to_string(),
          });
          result.changed = true;
        }
        Event::Exit => {
          // The PTY sends this after the child's exit, which says more.
          if self.exit_status.is_none() {
            self.exit_status = Some("Terminal requested shutdown.".to_string());
          }
          result.changed = true;
          result.exited = true;
        }
        Event::MouseCursorDirty
        | Event::CursorBlinkingChange
        | Event::Bell
        | Event::ColorRequest(_, _) => {}
      }
    }

    result
  }

  pub fn input(&mut self, text: &str) {
    self.send_text(text.to_string());
  }

  pub fn scroll_display(&mut self, delta_lines: i32) {
    if delta_lines == 0 {
      return;
    }

    self.term.lock().scroll_display(Scroll::Delta(delta_lines));
  }

  pub fn set_display_offset(&mut self, display_offset: usize) {
    let mut term = self.term.lock();
    let current_offset = term.grid().display_offset();
    let maximum_offset = term.total_lines().saturating_sub(term.screen_lines());
    let target_offset = display_offset.min(maximum_offset);
    let delta = target_offset as i32 - current_offset as i32;
    if delta != 0 {
      term.scroll_display(Scroll::Delta(delta));
    }
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

fn search_matches_for_term<T>(term: &Term<T>, query: &str) -> Vec<TerminalSearchMatch> {
  if query.is_empty() {
    return Vec::new();
  }

  let pattern = format!("(?i:{})", regex::escape(query));
  let Ok(mut search) = RegexSearch::new(&pattern) else {
    return Vec::new();
  };
  let start = Point::new(term.grid().topmost_line(), Column(0));
  let end = Point::new(term.grid().bottommost_line(), term.grid().last_column());

  RegexIter::new(start, end, Direction::Right, term, &mut search)
    .map(|found| TerminalSearchMatch {
      start: *found.start(),
      end: *found.end(),
    })
    .collect()
}

fn search_match_to_viewport<T>(
  term: &Term<T>,
  found: TerminalSearchMatch,
) -> Option<ViewportSelectionRange> {
  let rows = term.screen_lines();
  let cols = term.columns();
  if rows == 0 || cols == 0 {
    return None;
  }

  let display_offset = term.grid().display_offset();
  let viewport_top = -(display_offset as i32);
  let viewport_bottom = viewport_top + rows as i32 - 1;
  if found.end.line.0 < viewport_top || found.start.line.0 > viewport_bottom {
    return None;
  }

  let start_line = found.start.line.0.max(viewport_top);
  let end_line = found.end.line.0.min(viewport_bottom);
  let start_col = if found.start.line.0 < viewport_top {
    0
  } else {
    found.start.column.0.min(cols - 1)
  };
  let end_col = if found.end.line.0 > viewport_bottom {
    cols - 1
  } else {
    found.end.column.0.min(cols - 1)
  };

  Some(ViewportSelectionRange {
    start: ViewportPoint {
      row: (start_line - viewport_top) as usize,
      col: start_col,
    },
    end: ViewportPoint {
      row: (end_line - viewport_top) as usize,
      col: end_col,
    },
  })
}

fn tty_options(working_directory: &Path, inherited_lang: Option<OsString>) -> tty::Options {
  let mut options = tty::Options {
    working_directory: Some(working_directory.to_path_buf()),
    ..tty::Options::default()
  };
  // An app opened from the Finder or the Dock gets no locale, and the shell
  // would fall back to ASCII: accents and non-ASCII paths come out garbled.
  if inherited_lang.is_none() {
    options
      .env
      .insert("LANG".to_string(), "en_US.UTF-8".to_string());
  }
  // Launched from another shell, Reviu inherits its level; the shell adds one
  // on start, so this puts it back at 1 like a standalone terminal.
  options.env.insert("SHLVL".to_string(), "0".to_string());
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

fn tail_text_for_term<T>(term: &Term<T>, budget: TailBudget, styled: bool) -> TailText {
  let grid = term.grid();
  let columns = grid.columns();
  let top = grid.topmost_line();
  let mut line = grid.bottommost_line();
  let row_is_blank = |line: Line| {
    (0..columns).all(|column| {
      let cell = &grid[line][Column(column)];
      cell.c == ' ' && cell.zerowidth().is_none()
    })
  };
  while line > top && row_is_blank(line) {
    line = Line(line.0 - 1);
  }
  if line == top && row_is_blank(line) {
    return TailText::default();
  }

  // Walk up from the last written row, gathering whole logical lines: a row
  // whose last cell carries WRAPLINE continues on the next one.
  let mut logical_lines: Vec<String> = Vec::new();
  let mut rows_of_current: Vec<Line> = Vec::new();
  let mut bytes = 0;
  let mut clipped = false;
  loop {
    rows_of_current.push(line);
    let continues_previous = line > top
      && grid[Line(line.0 - 1)][Column(columns.saturating_sub(1))]
        .flags
        .contains(Flags::WRAPLINE);
    if !continues_previous {
      rows_of_current.reverse();
      let text = logical_line_text(grid, &rows_of_current, columns, styled);
      bytes += text.len() + 1;
      logical_lines.push(text);
      rows_of_current.clear();
      let full = match budget {
        TailBudget::Lines(limit) => logical_lines.len() >= limit,
        TailBudget::Bytes(limit) => bytes >= limit,
      };
      if full {
        clipped = line > top;
        break;
      }
    }
    if line == top {
      break;
    }
    line = Line(line.0 - 1);
  }
  logical_lines.reverse();
  let mut text = logical_lines.join("\n");
  text.push('\n');
  if let TailBudget::Bytes(limit) = budget
    && text.len() > limit
  {
    let mut cut = text.len() - limit;
    while cut < text.len() && !text.is_char_boundary(cut) {
      cut += 1;
    }
    text.drain(..cut);
    clipped = true;
  }
  TailText { text, clipped }
}

fn logical_line_text(grid: &Grid<Cell>, rows: &[Line], columns: usize, styled: bool) -> String {
  let mut text = String::new();
  let mut style = SgrStyle::default();
  for line in rows {
    for column in 0..columns {
      let cell = &grid[*line][Column(column)];
      if cell
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
      {
        continue;
      }
      if styled {
        let next = SgrStyle::of(cell);
        if next != style {
          next.write_transition(&mut text);
          style = next;
        }
      }
      text.push(cell.c);
      if let Some(zerowidth) = cell.zerowidth() {
        text.extend(zerowidth.iter());
      }
    }
  }
  let trimmed = text.trim_end_matches(' ').len();
  text.truncate(trimmed);
  if styled && style != SgrStyle::default() {
    text.push_str("\u{1b}[0m");
  }
  text
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SgrStyle {
  foreground: Option<Color>,
  background: Option<Color>,
  bold: bool,
  dim: bool,
  italic: bool,
  underline: bool,
}

impl SgrStyle {
  fn of(cell: &Cell) -> Self {
    let color = |color: Color, default: NamedColor| match color {
      Color::Named(named) if named == default => None,
      color => Some(color),
    };
    Self {
      foreground: color(cell.fg, NamedColor::Foreground),
      background: color(cell.bg, NamedColor::Background),
      bold: cell.flags.contains(Flags::BOLD),
      dim: cell.flags.contains(Flags::DIM),
      italic: cell.flags.contains(Flags::ITALIC),
      underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
    }
  }

  fn write_transition(&self, text: &mut String) {
    let mut codes = vec!["0".to_string()];
    if self.bold {
      codes.push("1".to_string());
    }
    if self.dim {
      codes.push("2".to_string());
    }
    if self.italic {
      codes.push("3".to_string());
    }
    if self.underline {
      codes.push("4".to_string());
    }
    if let Some(code) = self
      .foreground
      .and_then(|color| sgr_color(color, 30, 90, 38))
    {
      codes.push(code);
    }
    if let Some(code) = self
      .background
      .and_then(|color| sgr_color(color, 40, 100, 48))
    {
      codes.push(code);
    }
    text.push_str("\u{1b}[");
    text.push_str(&codes.join(";"));
    text.push('m');
  }
}

fn sgr_color(color: Color, normal: u16, bright: u16, extended: u16) -> Option<String> {
  match color {
    Color::Spec(rgb) => Some(format!("{extended};2;{};{};{}", rgb.r, rgb.g, rgb.b)),
    Color::Indexed(index) => Some(format!("{extended};5;{index}")),
    Color::Named(named) => {
      let index = named as usize;
      // Dim variants follow the bright ones in `NamedColor`; fold them back.
      let dim_black = NamedColor::DimBlack as usize;
      let index = if (dim_black..dim_black + 8).contains(&index) {
        index - dim_black
      } else {
        index
      };
      match index {
        0..=7 => Some((normal + index as u16).to_string()),
        8..=15 => Some((bright + index as u16 - 8).to_string()),
        _ => None,
      }
    }
  }
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
    if point.line >= rows {
      continue;
    }

    cells.push(TerminalCellSnapshot {
      row: point.line,
      col: point.column.0,
      c: match cell.c {
        '\t' => ' ',
        ch => ch,
      },
      zerowidth: cell.zerowidth().unwrap_or_default().into(),
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
    total_lines: term.total_lines(),
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
    TerminalBounds, TerminalListener, TerminalSelectionMode, ViewportPoint, ViewportSelectionRange,
    search_match_to_viewport, search_matches_for_term, selection_range_for_term,
    selection_text_for_term, snapshot_from_term,
  };
  #[cfg(not(windows))]
  use super::{TerminalSession, WorkingDirectoryTracker};
  use alacritty_terminal::{
    Term,
    event::{EventListener, VoidListener},
    term::{Config, cell::Flags},
    vte::ansi::{Processor, Rgb},
  };

  #[test]
  fn listener_coalesces_wakeups_until_the_previous_one_is_acknowledged() {
    let (sender, receiver) = async_channel::unbounded();
    let listener = TerminalListener::new(sender, TerminalBounds::default().window_size());

    for _ in 0..4 {
      listener.send_event(alacritty_terminal::event::Event::Wakeup);
    }
    assert_eq!(receiver.len(), 1);

    listener.acknowledge_wakeup();
    listener.send_event(alacritty_terminal::event::Event::Wakeup);
    assert_eq!(receiver.len(), 2);
  }

  #[cfg(not(windows))]
  #[test]
  fn working_directory_tracker_follows_the_shell_process() {
    let root =
      std::env::temp_dir().join(format!("reviu-terminal-cwd-tracker-{}", std::process::id()));
    let target = root.join("nested");
    if root.exists() {
      std::fs::remove_dir_all(&root).expect("stale tracker fixture should be removed");
    }
    std::fs::create_dir_all(&target).expect("tracker fixture should be created");
    let target = target
      .canonicalize()
      .expect("tracker fixture should resolve");

    let mut child = std::process::Command::new("sh")
      .arg("-c")
      .arg("cd -- \"$1\" && sleep 5")
      .arg("reviu-terminal-cwd-test")
      .arg(&target)
      .spawn()
      .expect("tracker shell should spawn");
    let tracker = WorkingDirectoryTracker::new(child.id(), root.clone(), None);
    assert!(tracker.begin_refresh());
    assert!(!tracker.begin_refresh());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut observed = tracker.refresh();
    while observed.as_deref() != Some(target.as_path()) && std::time::Instant::now() < deadline {
      assert!(tracker.begin_refresh());
      observed = tracker.refresh();
      std::thread::sleep(std::time::Duration::from_millis(20));
    }

    child.kill().expect("tracker shell should stop");
    child.wait().expect("tracker shell should be reaped");
    std::fs::remove_dir_all(root).expect("tracker fixture should be removed");
    assert_eq!(observed.as_deref(), Some(target.as_path()));
    assert_eq!(tracker.current(), target);
  }

  #[cfg(not(windows))]
  #[test]
  fn terminal_session_tracks_cd_commands() {
    let root =
      std::env::temp_dir().join(format!("reviu-terminal-session-cwd-{}", std::process::id()));
    let target = root.join("nested");
    if root.exists() {
      std::fs::remove_dir_all(&root).expect("stale session fixture should be removed");
    }
    std::fs::create_dir_all(&target).expect("session fixture should be created");
    let target = target
      .canonicalize()
      .expect("session fixture should resolve");
    let mut session = TerminalSession::spawn(root.clone(), TerminalBounds::default())
      .expect("terminal session should spawn");

    session.input(&format!("cd \"{}\"\r", target.display()));
    let tracker = session.working_directory_tracker();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut observed = None;
    while observed.as_deref() != Some(target.as_path()) && std::time::Instant::now() < deadline {
      if tracker.begin_refresh() {
        observed = tracker.refresh();
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }

    drop(session);
    std::fs::remove_dir_all(root).expect("session fixture should be removed");
    assert_eq!(observed.as_deref(), Some(target.as_path()));
  }

  #[test]
  fn terminal_search_is_literal_case_insensitive_and_reaches_history() {
    let bounds = TerminalBounds {
      columns: 16,
      lines: 2,
      ..TerminalBounds::default()
    };
    let config = Config {
      scrolling_history: 20,
      ..Config::default()
    };
    let mut term = Term::new(config, &bounds, VoidListener);
    let mut processor: Processor = Processor::new();
    processor.advance(&mut term, b"Alpha [one]\r\nbeta\r\nALPHA [one]");

    let matches = search_matches_for_term(&term, "alpha [one]");

    assert_eq!(matches.len(), 2);
    assert!(search_match_to_viewport(&term, matches[0]).is_none());
    assert!(search_match_to_viewport(&term, matches[1]).is_some());
    term.scroll_to_point(matches[0].start);
    assert!(search_match_to_viewport(&term, matches[0]).is_some());
  }

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
    assert_eq!(snapshot.total_lines, 3);
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
  fn snapshot_preserves_wide_cells_and_combining_marks() {
    let bounds = TerminalBounds {
      columns: 8,
      lines: 3,
      ..TerminalBounds::default()
    };
    let mut term = Term::new(Config::default(), &bounds, VoidListener);
    let mut processor: Processor = Processor::new();

    processor.advance(&mut term, "日Ae\u{301}👩\u{200d}💻".as_bytes());

    let snapshot = snapshot_from_term(&term, None, None);
    let wide = snapshot
      .cells
      .iter()
      .find(|cell| cell.row == 0 && cell.col == 0)
      .expect("wide cell should exist");
    let spacer = snapshot
      .cells
      .iter()
      .find(|cell| cell.row == 0 && cell.col == 1)
      .expect("wide spacer should exist");
    let combined = snapshot
      .cells
      .iter()
      .find(|cell| cell.row == 0 && cell.col == 3)
      .expect("combined cell should exist");

    let emoji = snapshot
      .cells
      .iter()
      .find(|cell| cell.row == 0 && cell.col == 4)
      .expect("emoji cell should exist");

    assert_eq!(wide.c, '日');
    assert!(wide.flags.contains(Flags::WIDE_CHAR));
    assert!(spacer.flags.contains(Flags::WIDE_CHAR_SPACER));
    assert_eq!(combined.c, 'e');
    assert_eq!(&*combined.zerowidth, &['\u{301}']);
    assert_eq!(emoji.c, '👩');
    assert_eq!(&*emoji.zerowidth, &['\u{200d}']);
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

  #[test]
  fn the_shell_gets_a_utf8_locale_only_when_none_is_inherited() {
    let directory = std::env::temp_dir();

    let options = super::tty_options(&directory, None);
    assert_eq!(
      options.env.get("LANG").map(String::as_str),
      Some("en_US.UTF-8")
    );

    let options = super::tty_options(&directory, Some("fr_FR.UTF-8".into()));
    assert_eq!(
      options.env.get("LANG"),
      None,
      "the user's locale is inherited untouched"
    );
    assert_eq!(options.env.get("SHLVL").map(String::as_str), Some("0"));
  }

  #[cfg(not(windows))]
  #[test]
  fn ctrl_c_interrupts_a_command_in_a_shell_started_off_thread() {
    let receiver =
      TerminalSession::spawn_on_thread(std::env::temp_dir(), TerminalBounds::default())
        .expect("the spawn thread starts");
    let mut session = receiver
      .recv_blocking()
      .expect("the spawn thread answers")
      .expect("the shell starts");
    let events = session.event_receiver();
    let screen_text = |session: &TerminalSession| -> String {
      session.snapshot().cells.iter().map(|cell| cell.c).collect()
    };

    session.input("sleep 30\r");
    std::thread::sleep(std::time::Duration::from_millis(500));
    session.input("\u{3}");
    session.input("echo interrupted-$((40 + 2))\r");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
      while let Ok(event) = events.try_recv() {
        session.process_events([event]);
      }
      if screen_text(&session).contains("interrupted-42") {
        return;
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!(
      "Ctrl-C did not interrupt sleep, screen:\n{}",
      screen_text(&session)
    );
  }

  #[test]
  fn command_labels_name_the_tool_the_user_ran() {
    let label = |arguments: &[&str]| {
      let arguments = arguments
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
      super::command_label(&arguments, std::ffi::OsStr::new("fallback"))
    };

    assert_eq!(
      label(&["/usr/bin/cargo", "test"]).as_deref(),
      Some("cargo test")
    );
    assert_eq!(
      label(&["node", "/usr/local/bin/npm", "run", "dev"]).as_deref(),
      Some("npm run dev")
    );
    assert_eq!(
      label(&["python3", "-m", "http.server"]).as_deref(),
      Some("python3 -m http.server"),
      "a flag is not a script"
    );
    assert_eq!(label(&[]).as_deref(), Some("fallback"));

    let long = label(&[
      "cargo",
      "test",
      "--workspace",
      "--all-features",
      "--",
      "--nocapture",
    ])
    .expect("a label");
    assert_eq!(long.chars().count(), super::COMMAND_LABEL_MAX_CHARS);
    assert!(long.ends_with('…'));
  }

  #[cfg(not(windows))]
  #[test]
  fn the_running_command_follows_the_foreground_process() {
    let mut session = TerminalSession::spawn(std::env::temp_dir(), TerminalBounds::default())
      .expect("the shell starts");
    let tracker = session.working_directory_tracker();
    let running_command_until = |expected: Option<&str>| {
      let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
      while std::time::Instant::now() < deadline {
        if tracker.begin_refresh() {
          tracker.refresh();
        }
        if tracker.running_command().as_deref() == expected {
          return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
      }
      panic!(
        "expected {expected:?}, the tracker saw {:?}",
        tracker.running_command()
      );
    };

    running_command_until(None);
    session.input("sleep 30\r");
    running_command_until(Some("sleep 30"));
    session.input("\u{3}");
    running_command_until(None);
  }

  #[cfg(not(windows))]
  fn run_program(script: &str) -> TerminalSession {
    let mut session = TerminalSession::spawn_program(
      std::env::temp_dir(),
      TerminalBounds {
        columns: 40,
        ..TerminalBounds::default()
      },
      super::ProgramOptions {
        program: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), script.to_string()],
        env: Default::default(),
        scrollback_lines: 1_000,
      },
    )
    .expect("the program starts");
    let events = session.event_receiver();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
      let Ok(event) = events.recv_blocking() else {
        break;
      };
      if session.process_events([event]).exited {
        return session;
      }
    }
    panic!("the program never exited");
  }

  #[cfg(not(windows))]
  #[test]
  fn a_program_output_comes_back_whole_with_its_exit_code() {
    use std::os::unix::process::ExitStatusExt as _;

    let long_line = "x".repeat(100);
    let session = run_program(&format!(
      "printf '\\033[31mred\\033[0m plain\\n{long_line}\\n'; [ -t 1 ] && echo tty; exit 3"
    ));

    let plain = session.tail_text(super::TailBudget::Bytes(10_000), false);
    assert_eq!(plain.text, format!("red plain\n{long_line}\ntty\n"));
    assert!(!plain.clipped);

    let styled = session.tail_text(super::TailBudget::Lines(10), true);
    assert!(
      styled.text.starts_with("\u{1b}[0;31mred\u{1b}[0m plain\n"),
      "colors survive as SGR: {:?}",
      styled.text
    );

    let status = session.child_exit().expect("an exit status");
    assert_eq!(status.code(), Some(3));
    assert_eq!(status.signal(), None);
  }

  #[cfg(not(windows))]
  #[test]
  fn the_tail_keeps_the_last_lines_and_says_it_left_some_out() {
    let session = run_program("seq 1 50");

    let tail = session.tail_text(super::TailBudget::Lines(3), false);
    assert_eq!(tail.text, "48\n49\n50\n");
    assert!(tail.clipped);

    let bytes = session.tail_text(super::TailBudget::Bytes(5), false);
    assert_eq!(bytes.text, "9\n50\n", "the byte budget cuts inside a line");
    assert!(bytes.clipped);
  }

  #[cfg(not(windows))]
  #[test]
  fn killing_a_program_stops_its_whole_group() {
    use std::os::unix::process::ExitStatusExt as _;

    let mut session = TerminalSession::spawn_program(
      std::env::temp_dir(),
      TerminalBounds::default(),
      super::ProgramOptions {
        program: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), "sleep 30; echo late".to_string()],
        env: Default::default(),
        scrollback_lines: 1_000,
      },
    )
    .expect("the program starts");
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(session.kill());

    let events = session.event_receiver();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
      let Ok(event) = events.recv_blocking() else {
        break;
      };
      if session.process_events([event]).exited {
        let status = session.child_exit().expect("an exit status");
        assert_eq!(status.signal(), Some(9));
        return;
      }
    }
    panic!("the killed program never exited");
  }
}
