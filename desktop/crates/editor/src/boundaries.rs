use gpui::Context;
use unicode_segmentation::UnicodeSegmentation;

use crate::editor::Editor;

/// Move to the previous character boundary
pub fn previous_boundary(_editor: &Editor, offset: usize, _cx: &Context<Editor>) -> usize {
  if offset == 0 {
    return 0;
  }

  // Simply move back one char - Ropey handles char boundaries correctly
  offset.saturating_sub(1)
}

/// Move to the next character boundary
pub fn next_boundary(editor: &Editor, offset: usize, cx: &Context<Editor>) -> usize {
  let doc = editor.document.read(cx);
  let doc_len = doc.len();

  if offset >= doc_len {
    return doc_len;
  }

  // Simply move forward one char - Ropey handles char boundaries correctly
  (offset + 1).min(doc_len)
}

/// Move to the previous word boundary (start of current or previous word/token)
/// This includes punctuation as separate tokens
pub fn previous_word_boundary(editor: &Editor, offset: usize, cx: &Context<Editor>) -> usize {
  if offset == 0 {
    return 0;
  }

  let doc = editor.document.read(cx);
  let doc_len = doc.len();

  // Work on a slice around the cursor instead of entire buffer
  // Get up to 1000 chars before cursor and a bit after to check cursor position
  let start = offset.saturating_sub(1000);
  let end = (offset + 100).min(doc_len);
  let slice = doc.slice_to_string(start..end);
  let relative_offset = offset - start;

  // Find the last newline before cursor to detect line boundaries
  let mut last_newline_pos = None;
  for (idx, ch) in slice.char_indices() {
    if ch == '\n' && idx < relative_offset {
      last_newline_pos = Some(idx);
    }
  }

  // Find the start of the current line (after the newline)
  let line_start = last_newline_pos.map(|pos| pos + 1).unwrap_or(0);

  // Check if the current line has indentation (leading whitespace)

  let mut first_non_space_on_line = None;
  // Check up to and including cursor position
  let check_range_end = (relative_offset + 1).min(slice.len());
  for (idx, ch) in slice[line_start..check_range_end].char_indices() {
    if !ch.is_whitespace() {
      first_non_space_on_line = Some(line_start + idx);
      break;
    }
  }

  // If we have indentation and we're not at the start of indentation,
  // we should stay on the current line
  let should_stay_on_line = if let Some(first_non_space) = first_non_space_on_line {
    // We have non-whitespace on this line before or at cursor
    // Check if there's indentation (spaces between line_start and first_non_space)
    first_non_space > line_start && relative_offset >= first_non_space
  } else {
    false
  };

  // Find all segments using split_word_bound_indices
  let mut last_segment_start = 0;
  let mut last_segment_on_current_line = line_start;

  for (idx, segment) in slice.split_word_bound_indices() {
    let segment_end = idx + segment.len();

    // Skip whitespace-only segments (including newlines)
    if segment.trim().is_empty() {
      continue;
    }

    // Track segments on the current line
    if idx >= line_start && idx < relative_offset {
      last_segment_on_current_line = idx;
    }

    // If we're exactly at the start of this segment, go to previous segment
    if idx == relative_offset {
      continue;
    }

    // If we're inside this segment, go to its start
    if idx < relative_offset && relative_offset <= segment_end {
      return start + idx;
    }

    // Track this as a potential previous segment
    if idx < relative_offset {
      last_segment_start = idx;
    }
  }

  // If we should stay on the current line, return the last segment on this line
  if should_stay_on_line && last_segment_on_current_line >= line_start {
    return start + last_segment_on_current_line;
  }

  // Otherwise, go to the previous segment (may cross lines)
  start + last_segment_start
}

/// Move to the next word boundary (end of current or next word/token)
/// This includes punctuation as separate tokens
pub fn next_word_boundary(editor: &Editor, offset: usize, cx: &Context<Editor>) -> usize {
  let doc = editor.document.read(cx);
  let doc_len = doc.len();

  if offset >= doc_len {
    return doc_len;
  }

  // Work on a slice from current position
  // We need to look backwards a bit to catch if we're in the middle of a segment
  let start = offset.saturating_sub(100);
  let end = (offset + 1000).min(doc_len);
  let slice = doc.slice_to_string(start..end);
  let relative_offset = offset - start;

  // Find all segments and their ends using split_word_bound_indices
  for (idx, segment) in slice.split_word_bound_indices() {
    let segment_end = idx + segment.len();

    // Skip whitespace-only segments
    if segment.trim().is_empty() {
      continue;
    }

    // If we're before or at the start of this segment, go to its end
    if relative_offset <= idx {
      return start + segment_end;
    }

    // If we're inside this segment, go to its end
    if relative_offset < segment_end {
      return start + segment_end;
    }
  }

  end
}

/// Find the word boundaries at the given offset (for double-click selection)
pub fn word_range_at_offset(
  editor: &Editor,
  offset: usize,
  cx: &Context<Editor>,
) -> (usize, usize) {
  let doc = editor.document.read(cx);
  let doc_len = doc.len();

  if offset >= doc_len {
    return (doc_len, doc_len);
  }

  // Get a slice around the offset
  let start = offset.saturating_sub(500);
  let end = (offset + 500).min(doc_len);
  let slice = doc.slice_to_string(start..end);
  let relative_offset = offset - start;

  // Find the segment containing the cursor
  for (idx, segment) in slice.split_word_bound_indices() {
    let segment_end = idx + segment.len();

    // Skip whitespace-only segments
    if segment.trim().is_empty() {
      continue;
    }

    // Check if cursor is within this segment
    if idx <= relative_offset && relative_offset < segment_end {
      return (start + idx, start + segment_end);
    }
  }

  // If no word found, return the offset itself
  (offset, offset)
}

/// Find the line boundaries at the given offset (for triple-click selection)
pub fn line_range_at_offset(
  editor: &Editor,
  offset: usize,
  cx: &Context<Editor>,
) -> (usize, usize) {
  let doc = editor.document.read(cx);
  let doc_len = doc.len();

  if doc_len == 0 {
    return (0, 0);
  }

  let line_idx = doc.char_to_line(offset.min(doc_len));

  if let Some(line_range) = doc.line_range(line_idx) {
    // Return the full line range including the newline
    (line_range.start, line_range.end)
  } else {
    (offset, offset)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::editor::tests::EditorTestContext;
  use gpui::TestAppContext;

  // ============================================================================
  // Word Boundary Tests
  // ============================================================================

  #[gpui::test]
  fn test_previous_word_boundary_simple(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world foo");

    // From middle of "world" (offset 8), go to start of "world" (offset 6)
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 8, cx)
    });
    assert_eq!(boundary, 6);
  }

  #[gpui::test]
  fn test_previous_word_boundary_at_start(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 0, cx)
    });
    assert_eq!(boundary, 0);
  }

  #[gpui::test]
  fn test_previous_word_boundary_multiple_spaces(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello   world");

    // From middle of "world" (offset 10) back to start of "world" (offset 8)
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 10, cx)
    });
    assert_eq!(boundary, 8);
  }

  #[gpui::test]
  fn test_previous_word_boundary_underscore(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "foo_bar_baz");

    // Underscores are part of words in unicode word segmentation
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 11, cx)
    });
    assert_eq!(boundary, 0); // Should go to start of entire word
  }

  #[gpui::test]
  fn test_previous_word_boundary_punctuation(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello, world!");

    // From middle of "world" (offset 9) back to start of "world" (offset 7)
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 9, cx)
    });
    assert_eq!(boundary, 7);
  }

  #[gpui::test]
  fn test_next_word_boundary_simple(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world foo");

    // From start of "hello" (offset 0), go to end of "hello" (offset 5)
    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 0, cx));
    assert_eq!(boundary, 5);
  }

  #[gpui::test]
  fn test_next_word_boundary_at_end(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    let doc_len = ctx.text().len();
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      next_word_boundary(editor, doc_len, cx)
    });
    assert_eq!(boundary, doc_len);
  }

  #[gpui::test]
  fn test_next_word_boundary_multiple_words(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "one two three");

    // From start of "one" (offset 0) to end of "one" (offset 3)
    let boundary1 = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 0, cx));
    assert_eq!(boundary1, 3);

    // From start of "two" (offset 4) to end of "two" (offset 7)
    let boundary2 = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 4, cx));
    assert_eq!(boundary2, 7);
  }

  #[gpui::test]
  fn test_next_word_boundary_punctuation(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello, world!");

    // From start of "hello" (offset 0) to end of "hello" (offset 5)
    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 0, cx));
    assert_eq!(boundary, 5);
  }

  #[gpui::test]
  fn test_word_boundary_unicode(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello 世界 world");

    // Previous word boundary from middle of "world" to start of "world"
    let prev = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 11, cx)
    });
    assert_eq!(prev, 9); // Start of "world"

    // Next word boundary from start of "hello" to end of "hello"
    let next = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 0, cx));
    assert_eq!(next, 5); // End of "hello"
  }

  #[gpui::test]
  fn test_next_word_boundary_in_enum(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "enum Color { Red, Green, Blue }");

    // From middle of "Color" (offset 7) should go to end of "Color" (offset 10)
    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 7, cx));
    assert_eq!(boundary, 10);

    // From end of "Color" (offset 10) should go to end of "{" (offset 12)
    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 10, cx));
    assert_eq!(boundary, 12);

    // From start of "Red" (offset 13) should go to end of "Red" (offset 16)
    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 13, cx));
    assert_eq!(boundary, 16);
  }

  #[gpui::test]
  fn test_previous_word_boundary_in_enum(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "    Red,\n    Green,\n    Blue,");

    // From middle of "Blue" (offset 26) should go to start of "Blue" (offset 24)
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 26, cx)
    });
    assert_eq!(boundary, 24);

    // From before "Blue" in whitespace (offset 23) should go to start of "," on previous line (18)
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 23, cx)
    });
    assert_eq!(boundary, 18); // Start of "," on previous line

    // From start of "Blue" (offset 24) should go to start of indentation (20)
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 24, cx)
    });
    assert_eq!(boundary, 20); // Start of indentation on same line

    // From start of indentation (offset 20) should go to "," on previous line (18)
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 20, cx)
    });
    assert_eq!(boundary, 18); // Start of "," on previous line (now crossing lines)
  }

  #[gpui::test]
  fn test_word_boundary_multiline_comprehensive(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(
      cx.clone(),
      "    Red,\n    Green,\n    Blue,\n    RGB(u8, u8, u8),",
    );

    // Test forward movement from "Green"
    // From start of "Green" (13) should go to end of "Green" (18)
    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 13, cx));
    assert_eq!(boundary, 18);

    // From end of "Green" (18) should go to end of "," (19)
    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 18, cx));
    assert_eq!(boundary, 19);

    // Test backward movement from "Blue"
    // From middle of "Blue" (26) should go to start of "Blue" (24)
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 26, cx)
    });
    assert_eq!(boundary, 24);

    // From start of "Blue" (24) should go to start of indentation (20) on same line
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 24, cx)
    });
    assert_eq!(boundary, 20);

    // From whitespace before "Blue" (22) should go to start of "," on previous line (18)
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 22, cx)
    });
    assert_eq!(boundary, 18);
  }

  #[gpui::test]
  fn test_previous_word_at_line_start_with_punctuation(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "    RGB(u8, u8, u8),\n}");

    // From start of line with "}" (offset 21, just after \n at position 20)
    // Should go to "," on previous line (offset 19)
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 21, cx)
    });
    assert_eq!(boundary, 19); // Start of "," on previous line
  }

  #[gpui::test]
  fn test_word_boundary_with_punctuation(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "enum Color {\n    Red,");

    // From start of "Red" (offset 17), go back should reach start of indentation (offset 13)
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 17, cx)
    });
    assert_eq!(boundary, 13); // Start of indentation, not "{"

    // From "Color" (offset 8), go forward should reach end of "Color" (offset 10)
    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 8, cx));
    assert_eq!(boundary, 10);

    // From end of "Color" (offset 10), go forward should reach end of "{" (offset 12)
    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_word_boundary(editor, 10, cx));
    assert_eq!(boundary, 12);
  }

  #[gpui::test]
  fn test_indented_line_stays_on_line(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "struct Person {\n    name: String,");

    // From middle of "name" (offset 21), should go to start of "name" (offset 20), not to "{"
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 21, cx)
    });
    assert_eq!(boundary, 20); // Start of "name" on same line

    // From start of "name" (offset 20), should go to start of indentation (offset 16), not to "{"
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 20, cx)
    });
    assert_eq!(boundary, 16); // Start of indentation on same line

    // From start of indentation (offset 16), then can go to previous line "{"
    let boundary = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      previous_word_boundary(editor, 16, cx)
    });
    assert_eq!(boundary, 14); // "{" on previous line
  }

  // ============================================================================
  // Character Boundary Tests
  // ============================================================================

  #[gpui::test]
  fn test_previous_boundary(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| previous_boundary(editor, 3, cx));
    assert_eq!(boundary, 2);
  }

  #[gpui::test]
  fn test_previous_boundary_at_start(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| previous_boundary(editor, 0, cx));
    assert_eq!(boundary, 0);
  }

  #[gpui::test]
  fn test_next_boundary(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_boundary(editor, 2, cx));
    assert_eq!(boundary, 3);
  }

  #[gpui::test]
  fn test_next_boundary_at_end(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "test");
    let boundary = ctx
      .editor
      .update(&mut ctx.cx, |editor, cx| next_boundary(editor, 4, cx));
    assert_eq!(boundary, 4);
  }

  #[gpui::test]
  fn test_word_range_at_offset_simple(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    // Click in middle of "hello"
    let (start, end) = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      word_range_at_offset(editor, 2, cx)
    });
    assert_eq!((start, end), (0, 5));

    // Click in middle of "world"
    let (start, end) = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      word_range_at_offset(editor, 8, cx)
    });
    assert_eq!((start, end), (6, 11));
  }

  #[gpui::test]
  fn test_word_range_at_offset_with_punctuation(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "RGB(u8, u8, u8)");

    // Click on "RGB"
    let (start, end) = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      word_range_at_offset(editor, 1, cx)
    });
    assert_eq!((start, end), (0, 3));

    // Click on "u8" (first one)
    let (start, end) = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      word_range_at_offset(editor, 5, cx)
    });
    assert_eq!((start, end), (4, 6));

    // Click on punctuation "("
    let (start, end) = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      word_range_at_offset(editor, 3, cx)
    });
    assert_eq!((start, end), (3, 4));
  }

  #[gpui::test]
  fn test_word_range_at_offset_whitespace(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello   world");

    // Click on whitespace - should return same position
    let (start, end) = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      word_range_at_offset(editor, 6, cx)
    });
    assert_eq!((start, end), (6, 6));
  }

  #[gpui::test]
  fn test_line_range_at_offset_simple(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    // Click on first line
    let (start, end) = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      line_range_at_offset(editor, 2, cx)
    });
    assert_eq!((start, end), (0, 6)); // Includes newline

    // Click on second line
    let (start, end) = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      line_range_at_offset(editor, 8, cx)
    });
    assert_eq!((start, end), (6, 12)); // Includes newline

    // Click on third line (no trailing newline)
    let (start, end) = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      line_range_at_offset(editor, 14, cx)
    });
    assert_eq!((start, end), (12, 17));
  }

  #[gpui::test]
  fn test_line_range_at_offset_empty_doc(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "");

    let (start, end) = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      line_range_at_offset(editor, 0, cx)
    });
    assert_eq!((start, end), (0, 0));
  }

  #[gpui::test]
  fn test_line_range_at_offset_single_line(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "single line");

    let (start, end) = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      line_range_at_offset(editor, 5, cx)
    });
    assert_eq!((start, end), (0, 11));
  }
}
