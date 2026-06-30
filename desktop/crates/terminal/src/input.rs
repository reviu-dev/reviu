use alacritty_terminal::{
  index::{Column, Line, Point},
  term::TermMode,
};
use gpui::{KeyDownEvent, Modifiers, MouseButton};

pub fn encode_key_down(event: &KeyDownEvent, mode: TermMode) -> Option<String> {
  let keystroke = &event.keystroke;
  let modifiers = keystroke.modifiers;

  if modifiers.function {
    return None;
  }

  if let Some(sequence) = encode_platform_key(keystroke.key.as_str(), modifiers) {
    return Some(sequence);
  }

  if modifiers.platform {
    return None;
  }

  if let Some(sequence) = encode_special_key(keystroke.key.as_str(), modifiers, mode) {
    return Some(sequence);
  }

  if modifiers.control
    && let Some(sequence) = encode_control_key(keystroke.key.as_str())
  {
    return Some(if modifiers.alt {
      format!("\u{1b}{sequence}")
    } else {
      sequence
    });
  }

  keystroke
    .key_char
    .as_ref()
    .filter(|text| !text.is_empty())
    .map(|text| {
      if modifiers.alt {
        format!("\u{1b}{text}")
      } else {
        text.clone()
      }
    })
}

pub fn encode_paste(text: &str, mode: TermMode) -> String {
  if mode.contains(TermMode::BRACKETED_PASTE) {
    format!("\u{1b}[200~{text}\u{1b}[201~")
  } else {
    text.to_string()
  }
}

pub fn mouse_mode_enabled(mode: TermMode) -> bool {
  mode.intersects(TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG)
}

pub fn can_report_mouse_move(mode: TermMode, pressed_button: Option<MouseButton>) -> bool {
  if !mode.intersects(TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG) {
    return false;
  }

  match pressed_button {
    Some(button) => encode_mouse_button(button).is_some(),
    None => mode.contains(TermMode::MOUSE_MOTION),
  }
}

pub fn encode_mouse_press(
  button: MouseButton,
  row: usize,
  col: usize,
  modifiers: Modifiers,
  mode: TermMode,
) -> Option<String> {
  let button_code = encode_mouse_button(button)?;
  encode_mouse_report(button_code, true, row, col, modifiers, mode)
}

pub fn encode_mouse_release(
  button: MouseButton,
  row: usize,
  col: usize,
  modifiers: Modifiers,
  mode: TermMode,
) -> Option<String> {
  let button_code = encode_mouse_button(button)?;
  encode_mouse_report(button_code, false, row, col, modifiers, mode)
}

pub fn encode_mouse_move(
  row: usize,
  col: usize,
  pressed_button: Option<MouseButton>,
  modifiers: Modifiers,
  mode: TermMode,
) -> Option<String> {
  if !can_report_mouse_move(mode, pressed_button) {
    return None;
  }

  let button_code = match pressed_button {
    Some(MouseButton::Left) => 32,
    Some(MouseButton::Middle) => 33,
    Some(MouseButton::Right) => 34,
    Some(_) => return None,
    None if mode.contains(TermMode::MOUSE_MOTION) => 35,
    None => return None,
  };

  encode_mouse_report(button_code, true, row, col, modifiers, mode)
}

pub fn encode_scroll(
  delta_lines: i32,
  row: usize,
  col: usize,
  modifiers: Modifiers,
  mode: TermMode,
) -> Option<String> {
  if delta_lines == 0 {
    return None;
  }

  if mouse_mode_enabled(mode) {
    let button_code = if delta_lines > 0 { 64 } else { 65 };
    let count = delta_lines.abs().min(10) as usize;
    let mut sequence = String::new();
    for _ in 0..count {
      sequence.push_str(&encode_mouse_report(
        button_code,
        true,
        row,
        col,
        modifiers,
        mode,
      )?);
    }
    return Some(sequence);
  }

  if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) && !modifiers.shift {
    let count = delta_lines.abs().min(5) as usize;
    let arrow = if delta_lines > 0 {
      cursor_sequence(mode, "A")
    } else {
      cursor_sequence(mode, "B")
    };
    return Some(arrow.repeat(count));
  }

  None
}

fn cursor_sequence(mode: TermMode, suffix: &str) -> String {
  if mode.contains(TermMode::APP_CURSOR) {
    format!("\u{1b}O{suffix}")
  } else {
    format!("\u{1b}[{suffix}")
  }
}

fn encode_platform_key(key: &str, modifiers: Modifiers) -> Option<String> {
  match key {
    "backspace" if modifiers.platform && !modifiers.control && !modifiers.alt => {
      Some("\u{15}".to_string())
    }
    _ => None,
  }
}

fn encode_special_key(key: &str, modifiers: Modifiers, mode: TermMode) -> Option<String> {
  match key {
    "enter" if modifiers.shift => Some("\n".to_string()),
    "enter" if modifiers.alt => Some("\u{1b}\r".to_string()),
    "enter" => Some("\r".to_string()),
    "tab" if modifiers.shift && !modifiers.alt && !modifiers.control => {
      Some("\u{1b}[Z".to_string())
    }
    "tab" if !modifiers.alt && !modifiers.control => Some("\t".to_string()),
    "backspace" if modifiers.control && !modifiers.alt && !modifiers.shift => {
      Some("\u{8}".to_string())
    }
    "backspace" if modifiers.alt && !modifiers.control => Some("\u{1b}\u{7f}".to_string()),
    "backspace" => Some("\u{7f}".to_string()),
    "escape" if !modifiers.shift && !modifiers.alt && !modifiers.control => {
      Some("\u{1b}".to_string())
    }
    "up" | "down" | "right" | "left" => encode_arrow_key(key, modifiers, mode),
    "home" | "end" => encode_home_end_key(key, modifiers, mode),
    "insert" => encode_tilde_key(2, modifiers),
    "delete" => encode_tilde_key(3, modifiers),
    "pageup" => encode_tilde_key(5, modifiers),
    "pagedown" => encode_tilde_key(6, modifiers),
    _ => None,
  }
}

fn encode_arrow_key(key: &str, modifiers: Modifiers, mode: TermMode) -> Option<String> {
  let suffix = match key {
    "up" => 'A',
    "down" => 'B',
    "right" => 'C',
    "left" => 'D',
    _ => return None,
  };

  match modifier_parameter(modifiers) {
    Some(parameter) => Some(format!("\u{1b}[1;{parameter}{suffix}")),
    None => Some(cursor_sequence(mode, &suffix.to_string())),
  }
}

fn encode_home_end_key(key: &str, modifiers: Modifiers, mode: TermMode) -> Option<String> {
  let suffix = match key {
    "home" => 'H',
    "end" => 'F',
    _ => return None,
  };

  match modifier_parameter(modifiers) {
    Some(parameter) => Some(format!("\u{1b}[1;{parameter}{suffix}")),
    None => Some(if mode.contains(TermMode::APP_CURSOR) {
      format!("\u{1b}O{suffix}")
    } else {
      format!("\u{1b}[{suffix}")
    }),
  }
}

fn encode_tilde_key(code: u8, modifiers: Modifiers) -> Option<String> {
  match modifier_parameter(modifiers) {
    Some(parameter) => Some(format!("\u{1b}[{code};{parameter}~")),
    None if !modifiers.shift && !modifiers.alt && !modifiers.control => {
      Some(format!("\u{1b}[{code}~"))
    }
    None => None,
  }
}

fn modifier_parameter(modifiers: Modifiers) -> Option<u8> {
  let mut bits = 0;
  if modifiers.shift {
    bits |= 1;
  }
  if modifiers.alt {
    bits |= 2;
  }
  if modifiers.control {
    bits |= 4;
  }

  (bits != 0).then_some(bits + 1)
}

fn encode_control_key(key: &str) -> Option<String> {
  let sequence = match key {
    "enter" => "\n".to_string(),
    "space" => "\0".to_string(),
    "[" => "\u{1b}".to_string(),
    "\\" => "\u{1c}".to_string(),
    "]" => "\u{1d}".to_string(),
    "^" => "\u{1e}".to_string(),
    "_" | "/" => "\u{1f}".to_string(),
    "backspace" | "?" => "\u{7f}".to_string(),
    _ => {
      let mut chars = key.chars();
      let ch = chars.next()?;
      if chars.next().is_some() || !ch.is_ascii_alphabetic() {
        return None;
      }
      let uppercase = ch.to_ascii_uppercase() as u8;
      char::from(uppercase - b'@').to_string()
    }
  };

  Some(sequence)
}

fn encode_mouse_button(button: MouseButton) -> Option<u8> {
  match button {
    MouseButton::Left => Some(0),
    MouseButton::Middle => Some(1),
    MouseButton::Right => Some(2),
    MouseButton::Navigate(_) => None,
  }
}

fn encode_mouse_report(
  button_code: u8,
  pressed: bool,
  row: usize,
  col: usize,
  modifiers: Modifiers,
  mode: TermMode,
) -> Option<String> {
  if !mouse_mode_enabled(mode) {
    return None;
  }

  let row = row.min(2014);
  let col = col.min(2014);
  let button = button_code + encode_mouse_modifiers(modifiers);

  if mode.contains(TermMode::SGR_MOUSE) {
    let suffix = if pressed { 'M' } else { 'm' };
    return Some(format!("\u{1b}[<{button};{};{}{suffix}", col + 1, row + 1));
  }

  let point = Point::new(Line(row as i32), Column(col));
  let final_button = if pressed {
    button
  } else {
    3 + encode_mouse_modifiers(modifiers)
  };
  encode_normal_mouse_report(point, final_button, mode)
}

fn encode_normal_mouse_report(point: Point, button: u8, mode: TermMode) -> Option<String> {
  let Point { line, column } = point;
  let utf8 = mode.contains(TermMode::UTF8_MOUSE);
  let max_point = if utf8 { 2015 } else { 223 };
  if line.0 < 0 || line.0 as usize >= max_point || column.0 >= max_point {
    return None;
  }

  let mut bytes = vec![b'\x1b', b'[', b'M', 32 + button];
  if utf8 && column >= Column(95) {
    bytes.extend(utf8_mouse_position(column.0));
  } else {
    bytes.push(32 + 1 + column.0 as u8);
  }

  if utf8 && line.0 >= 95 {
    bytes.extend(utf8_mouse_position(line.0 as usize));
  } else {
    bytes.push(32 + 1 + line.0 as u8);
  }

  String::from_utf8(bytes).ok()
}

fn utf8_mouse_position(position: usize) -> [u8; 2] {
  let pos = 32 + 1 + position;
  let first = 0xC0 + pos / 64;
  let second = 0x80 + (pos & 63);
  [first as u8, second as u8]
}

fn encode_mouse_modifiers(modifiers: Modifiers) -> u8 {
  let mut encoded = 0;
  if modifiers.shift {
    encoded |= 4;
  }
  if modifiers.alt {
    encoded |= 8;
  }
  if modifiers.control {
    encoded |= 16;
  }
  encoded
}

#[cfg(test)]
mod tests {
  use super::{
    can_report_mouse_move, encode_key_down, encode_mouse_move, encode_mouse_press,
    encode_mouse_release, encode_paste, encode_scroll, mouse_mode_enabled,
  };
  use alacritty_terminal::term::TermMode;
  use gpui::{KeyDownEvent, Keystroke, Modifiers, MouseButton, NavigationDirection};

  fn key_event(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> KeyDownEvent {
    KeyDownEvent {
      keystroke: Keystroke {
        modifiers,
        key: key.to_string(),
        key_char: key_char.map(ToOwned::to_owned),
      },
      is_held: false,
      prefer_character_input: false,
    }
  }

  #[test]
  fn encode_key_down_uses_text_input() {
    assert_eq!(
      encode_key_down(
        &key_event("a", Some("a"), Modifiers::default()),
        TermMode::SHOW_CURSOR,
      ),
      Some("a".to_string())
    );
  }

  #[test]
  fn encode_key_down_supports_control_sequences() {
    let mut modifiers = Modifiers::default();
    modifiers.control = true;

    assert_eq!(
      encode_key_down(&key_event("c", None, modifiers), TermMode::SHOW_CURSOR),
      Some("\u{3}".to_string())
    );
  }

  #[test]
  fn encode_key_down_supports_platform_backspace_as_kill_line() {
    let mut modifiers = Modifiers::default();
    modifiers.platform = true;

    assert_eq!(
      encode_key_down(
        &key_event("backspace", None, modifiers),
        TermMode::SHOW_CURSOR
      ),
      Some("\u{15}".to_string())
    );
  }

  #[test]
  fn encode_key_down_uses_app_cursor_when_requested() {
    assert_eq!(
      encode_key_down(
        &key_event("up", None, Modifiers::default()),
        TermMode::APP_CURSOR
      ),
      Some("\u{1b}OA".to_string())
    );
  }

  #[test]
  fn encode_key_down_supports_modified_navigation_keys() {
    let mut modifiers = Modifiers::default();
    modifiers.alt = true;

    assert_eq!(
      encode_key_down(&key_event("left", None, modifiers), TermMode::SHOW_CURSOR),
      Some("\u{1b}[1;3D".to_string())
    );

    modifiers = Modifiers::default();
    modifiers.control = true;

    assert_eq!(
      encode_key_down(&key_event("pageup", None, modifiers), TermMode::SHOW_CURSOR),
      Some("\u{1b}[5;5~".to_string())
    );
  }

  #[test]
  fn encode_key_down_supports_modified_enter_and_backspace() {
    let mut shift = Modifiers::default();
    shift.shift = true;
    assert_eq!(
      encode_key_down(&key_event("enter", None, shift), TermMode::SHOW_CURSOR),
      Some("\n".to_string())
    );

    let mut alt = Modifiers::default();
    alt.alt = true;
    assert_eq!(
      encode_key_down(&key_event("backspace", None, alt), TermMode::SHOW_CURSOR),
      Some("\u{1b}\u{7f}".to_string())
    );
  }

  #[test]
  fn encode_key_down_prefixes_escape_for_alt_control_characters() {
    let mut modifiers = Modifiers::default();
    modifiers.alt = true;
    modifiers.control = true;

    assert_eq!(
      encode_key_down(&key_event("c", None, modifiers), TermMode::SHOW_CURSOR),
      Some("\u{1b}\u{3}".to_string())
    );
  }

  #[test]
  fn encode_paste_wraps_bracketed_mode() {
    assert_eq!(
      encode_paste("git status", TermMode::BRACKETED_PASTE),
      "\u{1b}[200~git status\u{1b}[201~".to_string()
    );
  }

  #[test]
  fn mouse_mode_enabled_checks_any_mouse_flag() {
    assert!(mouse_mode_enabled(TermMode::MOUSE_REPORT_CLICK));
    assert!(!mouse_mode_enabled(TermMode::SHOW_CURSOR));
  }

  #[test]
  fn encode_mouse_press_uses_sgr_when_available() {
    assert_eq!(
      encode_mouse_press(
        MouseButton::Left,
        2,
        4,
        Modifiers::default(),
        TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE,
      ),
      Some("\u{1b}[<0;5;3M".to_string())
    );
  }

  #[test]
  fn encode_mouse_release_uses_normal_mode_without_sgr() {
    assert_eq!(
      encode_mouse_release(
        MouseButton::Right,
        1,
        2,
        Modifiers::default(),
        TermMode::MOUSE_REPORT_CLICK,
      ),
      Some("\u{1b}[M##\"".to_string())
    );
  }

  #[test]
  fn encode_mouse_move_uses_drag_code() {
    let mut modifiers = Modifiers::default();
    modifiers.alt = true;

    assert_eq!(
      encode_mouse_move(
        3,
        1,
        Some(MouseButton::Left),
        modifiers,
        TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE,
      ),
      Some("\u{1b}[<40;2;4M".to_string())
    );
  }

  #[test]
  fn can_report_mouse_move_requires_supported_modes_and_buttons() {
    assert!(can_report_mouse_move(
      TermMode::MOUSE_DRAG,
      Some(MouseButton::Left)
    ));
    assert!(can_report_mouse_move(TermMode::MOUSE_MOTION, None));
    assert!(!can_report_mouse_move(
      TermMode::MOUSE_REPORT_CLICK,
      Some(MouseButton::Left)
    ));
    assert!(!can_report_mouse_move(
      TermMode::MOUSE_DRAG,
      Some(MouseButton::Navigate(NavigationDirection::Back))
    ));
  }

  #[test]
  fn encode_scroll_uses_arrow_keys_in_alt_screen() {
    assert_eq!(
      encode_scroll(
        2,
        0,
        0,
        Modifiers::default(),
        TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL,
      ),
      Some("\u{1b}[A\u{1b}[A".to_string())
    );
  }

  #[test]
  fn encode_scroll_uses_mouse_reports_in_mouse_mode() {
    assert_eq!(
      encode_scroll(
        -2,
        3,
        5,
        Modifiers::default(),
        TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE,
      ),
      Some("\u{1b}[<65;6;4M\u{1b}[<65;6;4M".to_string())
    );
  }
}
