use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharType {
  Word,
  Whitespace,
  Newline,
  Other,
}

pub fn is_word_char(c: char) -> bool {
  matches!(c, '_')
    || c.is_ascii_alphanumeric()
    || matches!(c, '\u{00C0}'..='\u{00FF}')
    || matches!(c, '\u{0100}'..='\u{017F}')
    || matches!(c, '\u{0180}'..='\u{024F}')
    || matches!(c, '\u{0400}'..='\u{04FF}')
    || matches!(c, '\u{1E00}'..='\u{1EFF}')
    || matches!(c, '\u{0300}'..='\u{036F}')
}

impl From<char> for CharType {
  fn from(c: char) -> Self {
    match c {
      c if is_word_char(c) => CharType::Word,
      c if c == '\n' || c == '\r' => CharType::Newline,
      c if c.is_whitespace() => CharType::Whitespace,
      _ => CharType::Other,
    }
  }
}

impl CharType {
  fn is_connectable(self, c: char) -> bool {
    matches!(
      (self, CharType::from(c)),
      (CharType::Word, CharType::Word) | (CharType::Whitespace, CharType::Whitespace)
    )
  }
}

pub fn word_range_at(text: &str, offset: usize) -> Option<Range<usize>> {
  if text.is_empty() {
    return None;
  }
  let offset = clip_offset(text, offset);
  let c = text[offset..].chars().next()?;
  let char_type = CharType::from(c);
  let mut start = offset;
  let mut end = offset + c.len_utf8();

  for prev in text[..offset].chars().rev().take(128) {
    if char_type.is_connectable(prev) {
      start -= prev.len_utf8();
    } else {
      break;
    }
  }

  for next in text[end..].chars().take(128) {
    if char_type.is_connectable(next) {
      end += next.len_utf8();
    } else {
      break;
    }
  }

  Some(start..end)
}

pub fn line_range_at(text: &str, offset: usize) -> Range<usize> {
  let offset = clip_offset(text, offset).min(text.len());
  let start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
  let end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
  start..end
}

pub fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
  let offset = offset.min(text.len());
  if text.is_char_boundary(offset) {
    offset
  } else {
    text
      .char_indices()
      .map(|(ix, _)| ix)
      .take_while(|ix| *ix < offset)
      .last()
      .unwrap_or_default()
  }
}

fn clip_offset(text: &str, offset: usize) -> usize {
  let offset = offset.min(text.len());
  if offset == text.len() {
    return text.char_indices().next_back().map_or(0, |(ix, _)| ix);
  }
  if text.is_char_boundary(offset) {
    offset
  } else {
    text
      .char_indices()
      .map(|(ix, _)| ix)
      .take_while(|ix| *ix < offset)
      .last()
      .unwrap_or_default()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn char_type_word_chars() {
    assert_eq!(CharType::from('a'), CharType::Word);
    assert_eq!(CharType::from('Z'), CharType::Word);
    assert_eq!(CharType::from('0'), CharType::Word);
    assert_eq!(CharType::from('_'), CharType::Word);
  }

  #[test]
  fn char_type_latin_extended() {
    assert_eq!(CharType::from('é'), CharType::Word);
    assert_eq!(CharType::from('ñ'), CharType::Word);
  }

  #[test]
  fn char_type_whitespace_newline() {
    assert_eq!(CharType::from(' '), CharType::Whitespace);
    assert_eq!(CharType::from('\t'), CharType::Whitespace);
    assert_eq!(CharType::from('\n'), CharType::Newline);
  }

  #[test]
  fn char_type_other() {
    assert_eq!(CharType::from('.'), CharType::Other);
    assert_eq!(CharType::from('!'), CharType::Other);
    assert_eq!(CharType::from('汉'), CharType::Other);
  }

  #[test]
  fn word_range_expands_to_word_boundaries() {
    let text = "hello world";
    assert_eq!(word_range_at(text, 0), Some(0..5));
    assert_eq!(word_range_at(text, 4), Some(0..5));
    assert_eq!(word_range_at(text, 6), Some(6..11));
  }

  #[test]
  fn word_range_on_whitespace_groups_whitespace() {
    let text = "ab   cd";
    assert_eq!(word_range_at(text, 2), Some(2..5));
    assert_eq!(word_range_at(text, 3), Some(2..5));
  }

  #[test]
  fn word_range_on_punctuation_returns_single_char() {
    let text = "a, b";
    assert_eq!(word_range_at(text, 1), Some(1..2));
  }

  #[test]
  fn word_range_empty_text_returns_none() {
    assert_eq!(word_range_at("", 0), None);
  }

  #[test]
  fn word_range_handles_multibyte_chars() {
    let text = "café";
    assert_eq!(word_range_at(text, 0), Some(0..text.len()));
  }

  #[test]
  fn line_range_returns_line_bounds() {
    let text = "first\nsecond\nthird";
    assert_eq!(line_range_at(text, 0), 0..5);
    assert_eq!(line_range_at(text, 7), 6..12);
    assert_eq!(line_range_at(text, 14), 13..18);
  }

  #[test]
  fn line_range_handles_last_line_no_newline() {
    let text = "only line";
    assert_eq!(line_range_at(text, 5), 0..9);
  }

  #[test]
  fn clamp_to_char_boundary_snaps_back_to_boundary() {
    let text = "café";
    assert_eq!(clamp_to_char_boundary(text, 0), 0);
    assert_eq!(clamp_to_char_boundary(text, 3), 3);
    assert_eq!(clamp_to_char_boundary(text, 4), 3);
    assert_eq!(clamp_to_char_boundary(text, 99), text.len());
  }
}
