use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Indentation {
  pub width: usize,
  pub hard_tabs: bool,
}

impl Indentation {
  pub fn detect<'a>(lines: impl Iterator<Item = Cow<'a, str>>, language: Option<&str>) -> Self {
    let fallback = Self::for_language(language);
    let mut tabs = 0;
    let mut spaces = 0;
    let mut previous = 0usize;
    let mut widths = [0usize; 9];
    for line in lines.take(1000) {
      let body = line.trim_start_matches([' ', '\t']);
      if body.is_empty()
        || ["//", "#", "/*", "*", "<!--"]
          .iter()
          .any(|prefix| body.starts_with(prefix))
      {
        continue;
      }
      let prefix = &line[..line.len() - body.len()];
      if prefix.contains('\t') {
        tabs += 1;
        previous = 0;
        continue;
      }
      let indent = prefix.len();
      if indent > 0 {
        spaces += 1;
      }
      let difference = indent.abs_diff(previous);
      if difference > 0
        && let Some(votes) = widths.get_mut(difference)
      {
        *votes += 1;
      }
      previous = indent;
    }
    if tabs > 0 && tabs >= spaces {
      return Self {
        hard_tabs: true,
        ..fallback
      };
    }
    let width = widths
      .iter()
      .enumerate()
      .skip(1)
      .max_by_key(|(width, votes)| (**votes, *width == fallback.width, std::cmp::Reverse(*width)));
    match width.filter(|(_, votes)| **votes > 0) {
      Some((width, _)) => Self {
        width,
        hard_tabs: false,
      },
      None => fallback,
    }
  }

  fn for_language(language: Option<&str>) -> Self {
    let width = match language {
      Some(
        "typescript" | "javascript" | "json" | "yaml" | "html" | "css" | "scss" | "astro"
        | "svelte" | "vue" | "ruby",
      ) => 2,
      Some("make") => 8,
      _ => 4,
    };
    Self {
      width,
      hard_tabs: matches!(language, Some("go" | "make")),
    }
  }

  pub fn columns(self, text: &str) -> usize {
    text.chars().fold(0, |column, character| {
      if character == '\t' {
        column + self.width - column % self.width
      } else {
        column + 1
      }
    })
  }

  pub fn unit(self) -> String {
    if self.hard_tabs {
      "\t".to_string()
    } else {
      " ".repeat(self.width)
    }
  }

  pub fn tab_at(self, column: usize) -> String {
    if self.hard_tabs {
      "\t".to_string()
    } else {
      " ".repeat(self.width - column % self.width)
    }
  }
}

pub(crate) fn leading_whitespace(text: &str) -> &str {
  &text[..text.len() - text.trim_start_matches([' ', '\t']).len()]
}

#[cfg(test)]
mod tests {
  use super::*;

  fn detect(text: &str) -> Indentation {
    Indentation::detect(text.lines().map(Cow::Borrowed), None)
  }

  #[test]
  fn detects_space_widths_without_treating_alignment_as_a_level() {
    for width in [1, 2, 3, 4, 8] {
      let unit = " ".repeat(width);
      let text = format!("root\n{unit}one\n{unit}{unit}two\n{unit}three\nroot\n");
      assert_eq!(
        detect(&text),
        Indentation {
          width,
          hard_tabs: false
        }
      );
    }
    assert_eq!(
      detect("function(\n         aligned);\n  one\n    two\n  three\nend").width,
      2
    );
    assert_eq!(
      detect("/**\n * docs\n */\nfn main() {\n    code();\n}\n").width,
      4
    );
  }

  #[test]
  fn detects_tabs_and_ignores_blank_lines() {
    assert!(detect("root\n\tchild\n\t\tgrandchild\n  \n\tend").hard_tabs);
    assert_eq!(detect("root\n\n  child\n    \n  sibling").width, 2);
  }

  #[test]
  fn uses_language_defaults_only_without_file_evidence() {
    assert_eq!(
      Indentation::detect(std::iter::empty(), Some("json")).width,
      2
    );
    assert!(Indentation::detect(std::iter::empty(), Some("go")).hard_tabs);
    assert_eq!(
      Indentation::detect([Cow::Borrowed("    value")].into_iter(), Some("json")).width,
      4
    );
  }

  #[test]
  fn tabs_advance_to_stops_even_after_mixed_whitespace() {
    let indent = Indentation {
      width: 4,
      hard_tabs: false,
    };
    assert_eq!(indent.columns(" \t é"), 6);
    assert_eq!(indent.tab_at(6), "  ");
    assert_eq!(indent.tab_at(8), "    ");
  }
}
