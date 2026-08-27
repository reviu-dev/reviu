pub(crate) fn sanitize_agent_markdown(text: &str) -> String {
  let text = strip_cc_memory_blocks(text);
  split_text_stuck_to_closing_code_fences(&text)
}

fn strip_cc_memory_blocks(text: &str) -> String {
  let mut remaining = text;
  let mut output = String::new();
  let mut changed = false;

  while let Some(start) = remaining.find("<cc-memory") {
    output.push_str(&remaining[..start]);
    let hidden = &remaining[start..];
    let Some(close) = hidden.find("</cc-memory>") else {
      output.push_str(hidden);
      return output;
    };

    remaining = &hidden[close + "</cc-memory>".len()..];
    changed = true;
  }

  if !changed {
    return text.to_string();
  }

  output.push_str(remaining);
  output
}

fn split_text_stuck_to_closing_code_fences(text: &str) -> String {
  let mut output = String::new();
  let mut in_fence = false;
  let mut fence_char = '`';
  let mut fence_len = 0;
  let mut changed = false;

  for line in text.split_inclusive('\n') {
    let (line_without_eol, eol) = match line.strip_suffix('\n') {
      Some(line) => (
        line.strip_suffix('\r').unwrap_or(line),
        if line.ends_with('\r') { "\r\n" } else { "\n" },
      ),
      None => (line, ""),
    };

    let Some(marker) = fence_marker(line_without_eol) else {
      output.push_str(line);
      continue;
    };

    if !in_fence {
      in_fence = true;
      fence_char = marker.ch;
      fence_len = marker.len;
      output.push_str(line);
      continue;
    }

    if marker.ch == fence_char && marker.len >= fence_len {
      let suffix = &line_without_eol[marker.suffix_start..];
      if suffix.trim().is_empty() {
        in_fence = false;
        output.push_str(line);
      } else if should_split_closing_fence_suffix(suffix) {
        output.push_str(&line_without_eol[..marker.suffix_start]);
        output.push_str(if eol.is_empty() { "\n" } else { eol });
        output.push_str(suffix);
        output.push_str(eol);
        in_fence = false;
        changed = true;
      } else {
        output.push_str(line);
      }
      continue;
    }

    output.push_str(line);
  }

  if changed { output } else { text.to_string() }
}

struct FenceMarker {
  ch: char,
  len: usize,
  suffix_start: usize,
}

fn fence_marker(line: &str) -> Option<FenceMarker> {
  let indent = line.chars().take_while(|ch| *ch == ' ').count();
  if indent > 3 {
    return None;
  }

  let mut chars = line[indent..].chars();
  let ch = chars.next()?;
  if ch != '`' && ch != '~' {
    return None;
  }

  let len = line[indent..]
    .chars()
    .take_while(|candidate| *candidate == ch)
    .count();
  if len < 3 {
    return None;
  }

  Some(FenceMarker {
    ch,
    len,
    suffix_start: indent + ch.len_utf8() * len,
  })
}

fn should_split_closing_fence_suffix(suffix: &str) -> bool {
  let suffix = suffix.trim_end();
  !suffix.is_empty()
    && !suffix
      .chars()
      .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+' | '#'))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn strips_claude_memory_tags() {
    assert_eq!(
      sanitize_agent_markdown("before<cc-memory filenames=\"note.md\">hidden</cc-memory>after",),
      "beforeafter",
    );
  }

  #[test]
  fn splits_text_stuck_to_closing_code_fence() {
    assert_eq!(
      sanitize_agent_markdown("```text\nmessage\n```Noté : done"),
      "```text\nmessage\n```\nNoté : done",
    );
  }

  #[test]
  fn does_not_touch_valid_fence_language() {
    assert_eq!(
      sanitize_agent_markdown("```rust\nfn main() {}\n```"),
      "```rust\nfn main() {}\n```",
    );
  }

  #[test]
  fn does_not_split_nested_language_fence_content() {
    assert_eq!(
      sanitize_agent_markdown("````markdown\n```rust\nfn main() {}\n```\n````"),
      "````markdown\n```rust\nfn main() {}\n```\n````",
    );
  }
}
