use std::{collections::HashMap, sync::Arc};

use crate::gfm_markdown_viewer::GithubCodeReferencePreview;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MarkdownRenderSegment {
  Markdown(String),
  Preview(GithubCodeReferencePreview),
}

fn markdown_link_target(trimmed: &str) -> Option<&str> {
  if !trimmed.starts_with('[') || !trimmed.ends_with(')') {
    return None;
  }
  let (_, rest) = trimmed.split_once("](")?;
  rest.strip_suffix(')')
}

pub(crate) fn split_markdown_preview_segments(
  source: &str,
  previews: &HashMap<Arc<str>, GithubCodeReferencePreview>,
) -> Vec<MarkdownRenderSegment> {
  if source.is_empty() || previews.is_empty() {
    return vec![MarkdownRenderSegment::Markdown(source.to_string())];
  }

  let mut segments = Vec::new();
  let mut markdown = String::new();
  let mut has_preview = false;

  for raw_line in source.split_inclusive('\n') {
    let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
    let trimmed = line.trim();
    let line_preview = if trimmed.is_empty() {
      None
    } else if let Some(inner) = trimmed
      .strip_prefix('<')
      .and_then(|inner| inner.strip_suffix('>'))
    {
      previews.get(inner).cloned()
    } else {
      let markdown_link_preview =
        markdown_link_target(trimmed).and_then(|target| previews.get(target).cloned());
      markdown_link_preview.or_else(|| previews.get(trimmed).cloned())
    };

    if let Some(preview) = line_preview {
      if !markdown.is_empty() {
        segments.push(MarkdownRenderSegment::Markdown(std::mem::take(
          &mut markdown,
        )));
      }
      segments.push(MarkdownRenderSegment::Preview(preview));
      has_preview = true;
    } else {
      markdown.push_str(raw_line);
    }
  }

  if !markdown.is_empty() {
    segments.push(MarkdownRenderSegment::Markdown(markdown));
  }

  if !has_preview {
    return vec![MarkdownRenderSegment::Markdown(source.to_string())];
  }

  segments
}

#[cfg(test)]
mod tests {
  use super::markdown_link_target;

  #[test]
  fn markdown_link_target_extracts_target() {
    assert_eq!(
      markdown_link_target("[preview](https://github.com/acme/widget)"),
      Some("https://github.com/acme/widget")
    );
  }

  #[test]
  fn markdown_link_target_rejects_invalid_markup() {
    assert_eq!(
      markdown_link_target("[preview] https://github.com/acme/widget"),
      None
    );
    assert_eq!(markdown_link_target("https://github.com/acme/widget"), None);
  }
}
