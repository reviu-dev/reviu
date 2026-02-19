pub(crate) fn char_offset_to_byte_offset(text: &str, char_offset: usize) -> usize {
  if char_offset == 0 {
    return 0;
  }

  for (count, (idx, _)) in text.char_indices().enumerate() {
    if count == char_offset {
      return idx;
    }
  }

  text.len()
}

pub(crate) fn byte_offset_to_char_offset(text: &str, byte_offset: usize) -> usize {
  let mut byte_offset = byte_offset.min(text.len());
  while byte_offset > 0 && !text.is_char_boundary(byte_offset) {
    byte_offset -= 1;
  }
  text[..byte_offset].chars().count()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn char_offset_to_byte_offset_handles_emoji() {
    let text = "🤓 Branches";
    assert_eq!(char_offset_to_byte_offset(text, 0), 0);
    assert_eq!(char_offset_to_byte_offset(text, 1), 4);
    assert_eq!(char_offset_to_byte_offset(text, 2), 5);
  }

  #[test]
  fn byte_offset_to_char_offset_clamps_non_char_boundary() {
    let text = "✅ab";
    assert_eq!(byte_offset_to_char_offset(text, 0), 0);
    assert_eq!(byte_offset_to_char_offset(text, 1), 0);
    assert_eq!(byte_offset_to_char_offset(text, 3), 1);
  }
}
