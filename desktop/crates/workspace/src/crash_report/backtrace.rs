/// Keeps the frames `RUST_BACKTRACE=1` would print, minus the panic machinery, and turns
/// CI build paths into something short enough to read and to fit the report limits.
pub(super) fn clean_backtrace(raw: &str) -> String {
  let frames = split_frames(raw);

  // std marks where the interesting part of a stack ends and begins with these two frames.
  let start = frames
    .iter()
    .rposition(|frame| frame.symbol().contains("__rust_end_short_backtrace"))
    .map_or(0, |index| index + 1);
  let end = frames
    .iter()
    .skip(start)
    .position(|frame| frame.symbol().contains("__rust_begin_short_backtrace"))
    .map_or(frames.len(), |offset| start + offset);

  let Some(kept) = frames.get(start..end) else {
    return raw.trim().to_string();
  };
  let kept: Vec<&Frame> = kept
    .iter()
    .skip_while(|frame| is_panic_machinery(frame.symbol()))
    .collect();
  if kept.is_empty() {
    return raw.trim().to_string();
  }

  kept
    .iter()
    .flat_map(|frame| frame.lines.iter())
    .map(|line| shorten_location_line(line))
    .collect::<Vec<_>>()
    .join("\n")
}

/// A panic location or a backtrace `at` path, without the CI runner and cargo cache prefixes.
pub(super) fn shorten_path(path: &str) -> String {
  let path = path.replace('\\', "/");

  if let Some((_, rest)) = path.split_once("/.cargo/git/checkouts/") {
    // `zed-ebe293394c5963c9/f5ee27f/crates/...` names the repository and the pinned rev.
    let mut parts = rest.splitn(3, '/');
    if let (Some(checkout), Some(rev), Some(inner)) = (parts.next(), parts.next(), parts.next()) {
      let name = checkout.rsplit_once('-').map_or(checkout, |(name, _)| name);
      return format!("{name}@{rev}/{inner}");
    }
  }
  if let Some((_, rest)) = path.split_once("/.cargo/registry/src/")
    && let Some((_, inner)) = rest.split_once('/')
  {
    return inner.to_string();
  }
  if let Some((_, rest)) = path.split_once("/rustc/")
    && let Some((_, inner)) = rest.split_once('/')
  {
    return format!("rust/{inner}");
  }
  if let Some(index) = path.find("/desktop/crates/") {
    return path[index + 1..].to_string();
  }
  path
}

struct Frame<'a> {
  lines: Vec<&'a str>,
}

impl Frame<'_> {
  fn symbol(&self) -> &str {
    self
      .lines
      .first()
      .and_then(|line| line.trim_start().split_once(": "))
      .map_or("", |(_, symbol)| symbol)
  }
}

fn split_frames(raw: &str) -> Vec<Frame<'_>> {
  let mut frames: Vec<Frame<'_>> = Vec::new();
  for line in raw.lines() {
    if is_frame_header(line) || frames.is_empty() {
      frames.push(Frame { lines: vec![line] });
    } else if let Some(frame) = frames.last_mut() {
      frame.lines.push(line);
    }
  }
  frames
}

fn is_frame_header(line: &str) -> bool {
  line
    .trim_start()
    .split_once(": ")
    .is_some_and(|(index, _)| !index.is_empty() && index.chars().all(|ch| ch.is_ascii_digit()))
}

fn is_panic_machinery(symbol: &str) -> bool {
  symbol.contains("rust_begin_unwind") || symbol.starts_with("core::panicking::")
}

fn shorten_location_line(line: &str) -> String {
  match line.split_once("at ") {
    Some((indent, path)) if indent.trim().is_empty() => {
      format!("{indent}at {}", shorten_path(path))
    }
    _ => line.to_string(),
  }
}

#[cfg(test)]
mod tests {
  use super::{clean_backtrace, shorten_path};

  const RAW: &str = "   0: from_panic_info
             at /home/runner/work/reviu/reviu/desktop/crates/workspace/src/crash_report.rs:77:23
   5: std::sys::backtrace::__rust_end_short_backtrace::<std::panicking::panic_handler::{closure#0}, !>
             at /rustc/48a229ce/library/std/src/sys/backtrace.rs:182:18
   6: __rustc::rust_begin_unwind
             at /rustc/48a229ce/library/std/src/panicking.rs:679:5
   7: core::panicking::panic_fmt
             at /rustc/48a229ce/library/core/src/panicking.rs:80:14
   8: core::cell::panic_already_borrowed
             at /rustc/48a229ce/library/core/src/panic.rs:177:9
  12: with_common<()>
             at /home/runner/.cargo/git/checkouts/zed-ebe293394c5963c9/f5ee27f/crates/gpui_linux/src/linux/x11/client.rs:1537:23
  30: process_events<calloop::Generic>
             at /home/runner/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/calloop-0.14.4/src/sources/generic.rs:290:9
  38: main
             at /home/runner/work/reviu/reviu/desktop/crates/reviu/src/main.rs:299:7
  40: __rust_begin_short_backtrace<fn(), ()>
             at /rustc/48a229ce/library/std/src/sys/backtrace.rs:166:18
  41: std::rt::lang_start::<()>::{closure#0}
  51: main";

  #[test]
  fn keeps_only_the_frames_that_explain_the_panic() {
    assert_eq!(
      clean_backtrace(RAW),
      "   8: core::cell::panic_already_borrowed
             at rust/library/core/src/panic.rs:177:9
  12: with_common<()>
             at zed@f5ee27f/crates/gpui_linux/src/linux/x11/client.rs:1537:23
  30: process_events<calloop::Generic>
             at calloop-0.14.4/src/sources/generic.rs:290:9
  38: main
             at desktop/crates/reviu/src/main.rs:299:7"
    );
  }

  #[test]
  fn an_unrecognised_backtrace_is_kept_whole() {
    assert_eq!(
      clean_backtrace("disabled backtrace\n"),
      "disabled backtrace"
    );
  }

  #[test]
  fn windows_paths_are_shortened_too() {
    assert_eq!(
      shorten_path(r"D:\a\reviu\reviu\desktop\crates\editor\src\editor.rs:42:7"),
      "desktop/crates/editor/src/editor.rs:42:7"
    );
    assert_eq!(
      shorten_path(
        r"C:\Users\runneradmin\.cargo\git\checkouts\zed-ebe29\f5ee27f\crates\gpui\src\app.rs:1:1"
      ),
      "zed@f5ee27f/crates/gpui/src/app.rs:1:1"
    );
  }
}
