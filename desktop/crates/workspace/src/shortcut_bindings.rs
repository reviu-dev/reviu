use gpui::{KeyBinding, Keystroke};

use super::shortcuts::WORKSPACE_SHORTCUT_RECORDING_CONTEXT;
use ui::COMMAND_PALETTE_CONTEXT;

pub(super) struct EditorShortcut {
  pub title: &'static str,
  pub keystroke: &'static str,
  binding: fn(&str, Option<&str>) -> KeyBinding,
}

macro_rules! editor_shortcuts {
  ($(($title:literal, $key:literal, $action:ident)),* $(,)?) => {
    &[$(EditorShortcut { title: $title, keystroke: $key,
      binding: |key, context| KeyBinding::new(key, editor::$action, context),
    }),*]
  };
}

pub(super) const EDITOR_SHORTCUTS: &[EditorShortcut] = editor_shortcuts![
  ("Newline", "enter", Enter),
  ("Newline", "shift-enter", Enter),
  ("Indent or Insert Tab", "tab", Tab),
  ("Outdent", "shift-tab", Outdent),
  ("Indent Line", "cmd-]", Indent),
  ("Outdent Line", "cmd-[", Outdent),
  ("Move Line Up", "alt-up", MoveLineUp),
  ("Move Line Down", "alt-down", MoveLineDown),
  ("Duplicate Line Up", "alt-shift-up", DuplicateLineUp),
  ("Duplicate Line Down", "alt-shift-down", DuplicateLineDown),
  ("Delete Line", "cmd-shift-k", DeleteLine),
  ("Insert Line Below", "cmd-enter", NewlineBelow),
  ("Insert Line Above", "cmd-shift-enter", NewlineAbove),
  ("Toggle Comments", "cmd-/", ToggleComments),
  ("Delete Backward", "backspace", Backspace),
  ("Delete Backward", "shift-backspace", Backspace),
  ("Delete Word Backward", "alt-backspace", BackspaceWord),
  ("Delete to Start of Line", "cmd-backspace", BackspaceAll),
  ("Delete Forward", "delete", Delete),
  ("Move Up", "up", Up),
  ("Move Down", "down", Down),
  ("Page Up", "pageup", PageUp),
  ("Page Down", "pagedown", PageDown),
  ("Select Page Up", "shift-pageup", SelectPageUp),
  ("Select Page Down", "shift-pagedown", SelectPageDown),
  ("Go to Line", "ctrl-g", GoToLine),
  ("Add Cursor Above", "cmd-alt-up", AddSelectionAbove),
  ("Add Cursor Below", "cmd-alt-down", AddSelectionBelow),
  ("Select Next Occurrence", "cmd-d", SelectNextOccurrence),
  (
    "Select All Occurrences",
    "cmd-shift-l",
    SelectAllOccurrences
  ),
  ("Select All Occurrences", "cmd-f2", SelectAllOccurrences),
  ("Move Left", "left", Left),
  ("Move Word Left", "alt-left", AltLeft),
  ("Move to Line Start", "cmd-left", CmdLeft),
  ("Move Right", "right", Right),
  ("Move Word Right", "alt-right", AltRight),
  ("Move to Line End", "cmd-right", CmdRight),
  ("Move to Document Start", "cmd-up", CmdUp),
  ("Move to Document End", "cmd-down", CmdDown),
  ("Move to Document Start", "cmd-home", CmdUp),
  ("Move to Document End", "cmd-end", CmdDown),
  ("Select Up", "shift-up", SelectUp),
  ("Select Down", "shift-down", SelectDown),
  ("Select to Line Start", "cmd-shift-left", SelectCmdLeft),
  ("Select to Line End", "cmd-shift-right", SelectCmdRight),
  ("Select to Document Start", "cmd-shift-up", SelectCmdUp),
  ("Select to Document End", "cmd-shift-down", SelectCmdDown),
  ("Select to Document Start", "cmd-shift-home", SelectCmdUp),
  ("Select to Document End", "cmd-shift-end", SelectCmdDown),
  ("Select Left", "shift-left", SelectLeft),
  ("Select Word Left", "alt-shift-left", SelectWordLeft),
  ("Select Right", "shift-right", SelectRight),
  ("Select Word Right", "alt-shift-right", SelectWordRight),
  ("Move to Line Start", "home", Home),
  ("Move to Line End", "end", End),
  ("Select to Line Start", "shift-home", SelectCmdLeft),
  ("Select to Line End", "shift-end", SelectCmdRight),
];

pub(super) const EDITOR_SURFACE_SHORTCUTS: &[EditorShortcut] = editor_shortcuts![
  ("Replace", "cmd-alt-f", FindReplace),
  ("Find Next", "cmd-g", FindNext),
  ("Find Previous", "cmd-shift-g", FindPrevious),
  (
    "Toggle Find Case Sensitive",
    "cmd-alt-c",
    ToggleFindCaseSensitive
  ),
  ("Toggle Find Whole Word", "cmd-alt-w", ToggleFindWholeWord),
  ("Toggle Find Regex", "cmd-alt-r", ToggleFindRegex),
  ("Close Find or Clear Extra Selections", "escape", CloseFind),
];

pub(super) const APPLICATION_SHORTCUTS: &[EditorShortcut] = editor_shortcuts![
  ("Select All", "cmd-a", SelectAll),
  ("Paste", "cmd-v", Paste),
  ("Copy", "cmd-c", Copy),
  ("Cut", "cmd-x", Cut),
  ("Undo", "cmd-z", Undo),
  ("Redo", "cmd-shift-z", Redo),
  ("Save", "cmd-s", Save),
  ("Find", "cmd-f", Find),
  ("Character Palette", "ctrl-cmd-space", ShowCharacterPalette),
  ("Quit", "cmd-q", Quit),
];

pub(super) const RESERVED_EDITOR_SHORTCUTS: &[(&str, &str)] = &[
  ("cmd-k", "Editor Chord Prefix"),
  ("cmd-u", "Undo Cursor or Selection"),
  ("cmd-shift-u", "Redo Cursor or Selection"),
  ("cmd-shift-t", "Reopen Closed Tab"),
  ("cmd-t", "Project Symbols"),
  ("cmd-shift-o", "File Symbols"),
  ("cmd-shift-h", "Replace in Files"),
  ("cmd-l", "Select Line"),
  ("cmd-shift-b", "Build or Outline"),
  ("cmd-shift-r", "Run Task"),
  ("cmd-shift-j", "Search Details"),
  ("cmd-shift-c", "External Terminal"),
  ("cmd-y", "Stage or Keep Change"),
  ("cmd-shift-y", "Unstage Change"),
  ("ctrl-enter", "Inline Assistant"),
  ("ctrl-shift-up", "Select to Start of Paragraph"),
  ("ctrl-shift-down", "Select to End of Paragraph"),
  ("cmd-delete", "Delete to End of Line"),
  ("alt-delete", "Delete Word Forward"),
  ("ctrl-alt-backspace", "Delete Subword Backward"),
  ("ctrl-alt-delete", "Delete Subword Forward"),
  ("ctrl-alt-h", "Delete Subword Backward"),
  ("ctrl-alt-d", "Delete Subword Forward"),
  ("ctrl-alt-left", "Previous Subword"),
  ("ctrl-alt-right", "Next Subword"),
  ("ctrl-alt-shift-left", "Select Previous Subword"),
  ("ctrl-alt-shift-right", "Select Next Subword"),
  ("ctrl-a", "Line Start"),
  ("ctrl-e", "Line End"),
  ("ctrl-b", "Move Left"),
  ("ctrl-f", "Move Right"),
  ("ctrl-p", "Move Up"),
  ("ctrl-n", "Move Down"),
  ("ctrl-h", "Delete Backward"),
  ("ctrl-d", "Delete Forward"),
  ("ctrl-k", "Kill to End of Line"),
  ("ctrl-y", "Yank"),
  ("ctrl-t", "Transpose"),
  ("ctrl-l", "Center Cursor"),
  ("ctrl-space", "Completions"),
  ("cmd-.", "Code Actions"),
  ("cmd-i", "Inline Chat or Signature Help"),
  ("f2", "Rename Symbol"),
  ("f12", "Go to Definition"),
  ("alt-f12", "Peek Definition"),
  ("cmd-f12", "Go to Implementation or Type"),
  ("shift-f12", "References or Implementation"),
  ("ctrl-shift-left", "Shrink Selection"),
  ("ctrl-shift-right", "Expand Selection"),
  ("cmd-ctrl-left", "Shrink Selection"),
  ("cmd-ctrl-right", "Expand Selection"),
  ("alt-shift-i", "Split Selection into Lines"),
  ("alt-shift-f", "Format Document"),
  ("cmd-shift-i", "Format Document"),
  ("alt-shift-a", "Toggle Block Comment"),
  ("cmd-alt-[", "Fold"),
  ("cmd-alt-]", "Unfold"),
  ("cmd-\\", "Split Editor"),
  ("cmd-shift-\\", "Matching Bracket"),
  ("ctrl-tab", "Next Recent Tab"),
  ("ctrl-shift-tab", "Previous Recent Tab"),
  ("ctrl-`", "Toggle Terminal"),
  ("ctrl-shift-`", "New Terminal"),
  ("cmd-b", "Toggle Sidebar"),
  ("cmd-j", "Toggle Bottom Panel"),
  ("cmd-shift-x", "Extensions"),
  ("cmd-shift-m", "Problems"),
  ("cmd-alt-s", "Save All"),
  ("cmd-shift-n", "New Window"),
  ("cmd-shift-w", "Close Window"),
  ("cmd-+", "Zoom In"),
  ("cmd-=", "Zoom In"),
  ("cmd--", "Zoom Out"),
  ("cmd-0", "Reset Zoom"),
  ("cmd-h", "Hide Application"),
  ("cmd-m", "Minimize"),
  ("f1", "Command Palette"),
  ("f5", "Debug"),
  ("f8", "Next Diagnostic"),
  ("shift-f8", "Previous Diagnostic"),
  ("f9", "Breakpoint"),
  ("f10", "Debug Step Over"),
  ("f11", "Debug Step Into"),
];

pub(super) fn same_keystroke(text: &str, key: &Keystroke) -> bool {
  Keystroke::parse(text)
    .is_ok_and(|parsed| parsed.key == key.key && parsed.modifiers == key.modifiers)
}

pub(super) fn reserved_editor_shortcut(key: &Keystroke) -> Option<&'static str> {
  EDITOR_SHORTCUTS
    .iter()
    .chain(EDITOR_SURFACE_SHORTCUTS)
    .chain(APPLICATION_SHORTCUTS)
    .find(|binding| same_keystroke(binding.keystroke, key))
    .map(|binding| binding.title)
    .or_else(|| {
      RESERVED_EDITOR_SHORTCUTS
        .iter()
        .find(|(text, _)| same_keystroke(text, key))
        .map(|(_, title)| *title)
    })
}

pub(super) fn editor_key_bindings() -> Vec<KeyBinding> {
  let context = format!(
    "Editor && !Input && !{COMMAND_PALETTE_CONTEXT} && !{WORKSPACE_SHORTCUT_RECORDING_CONTEXT}"
  );
  let surface_context =
    format!("Editor && !{COMMAND_PALETTE_CONTEXT} && !{WORKSPACE_SHORTCUT_RECORDING_CONTEXT}");
  EDITOR_SHORTCUTS
    .iter()
    .map(|shortcut| (shortcut.binding)(shortcut.keystroke, Some(&context)))
    .chain(
      EDITOR_SURFACE_SHORTCUTS
        .iter()
        .map(|shortcut| (shortcut.binding)(shortcut.keystroke, Some(&surface_context))),
    )
    .collect()
}

pub(super) fn application_key_bindings() -> Vec<KeyBinding> {
  APPLICATION_SHORTCUTS
    .iter()
    .map(|shortcut| (shortcut.binding)(shortcut.keystroke, None))
    .collect()
}
