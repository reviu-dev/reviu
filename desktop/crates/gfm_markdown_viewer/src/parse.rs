use std::collections::HashSet;
use std::sync::Arc;

use comrak::nodes::{AstNode, ListType, NodeValue};
use comrak::{Arena, Options, parse_document};

use crate::parse_html::*;
use crate::types::*;

pub fn parse_gfm(source: &str) -> Vec<Block> {
  let mut blocks = Vec::new();
  for segment in split_details_segments(source) {
    match segment {
      Segment::Markdown(markdown) => blocks.extend(parse_comrak(&markdown)),
      Segment::Details {
        summary,
        body,
        open,
      } => {
        let summary_inlines = summary_inlines_from_text(summary.as_deref().unwrap_or("Details"));
        let body_blocks = parse_gfm(&body);
        blocks.push(Block::Details(Details {
          summary: summary_inlines,
          blocks: body_blocks,
          open,
        }));
      }
    }
  }
  blocks
}

pub fn parse_markdown(source: &str) -> ParsedMarkdown {
  ParsedMarkdown {
    blocks: Arc::new(parse_gfm(source)),
  }
}

pub(crate) fn comrak_options() -> Options<'static> {
  let mut options = Options::default();
  options.extension.strikethrough = true;
  options.extension.table = true;
  options.extension.tasklist = true;
  options.extension.autolink = true;
  options.extension.tagfilter = true;
  options.parse.smart = true;
  options
}

pub(crate) fn parse_comrak(source: &str) -> Vec<Block> {
  let arena = Arena::new();
  let options = comrak_options();
  let root = parse_document(&arena, source, &options);
  let mut blocks = Vec::new();
  let mut centered_div_depth = 0usize;
  let mut centered_div_blocks = Vec::new();

  for node in root.children() {
    let mut is_centered_open = false;
    let mut is_centered_close = false;

    if let NodeValue::HtmlBlock(html) = &node.data.borrow().value {
      is_centered_open = is_centered_div_open_tag(&html.literal);
      is_centered_close = is_centered_div_close_tag(&html.literal);
    }

    if is_centered_open {
      centered_div_depth = centered_div_depth.saturating_add(1);
      continue;
    }

    if is_centered_close {
      centered_div_depth = centered_div_depth.saturating_sub(1);
      if centered_div_depth == 0 && !centered_div_blocks.is_empty() {
        blocks.push(Block::Aligned {
          center: true,
          blocks: std::mem::take(&mut centered_div_blocks),
        });
      }
      continue;
    }

    let node_blocks = blocks_from_node(node);
    if centered_div_depth > 0 {
      centered_div_blocks.extend(node_blocks);
    } else {
      blocks.extend(node_blocks);
    }
  }

  if !centered_div_blocks.is_empty() {
    blocks.push(Block::Aligned {
      center: true,
      blocks: centered_div_blocks,
    });
  }

  blocks
}

pub(crate) fn split_details_segments(source: &str) -> Vec<Segment> {
  let mut segments = Vec::new();
  let mut buffer = String::new();
  let mut fence: Option<(char, usize)> = None;
  let mut lines = source.lines();
  let mut pending_line: Option<String> = None;

  while let Some(line) = pending_line
    .take()
    .or_else(|| lines.next().map(|line| line.to_string()))
  {
    update_fence_state(&line, &mut fence);
    if fence.is_none()
      && let Some(start_idx) = find_details_start(&line)
    {
      let (prefix, rest) = line.split_at(start_idx);
      if !prefix.is_empty() {
        buffer.push_str(prefix);
        buffer.push('\n');
      }

      if !buffer.is_empty() {
        segments.push(Segment::Markdown(buffer));
        buffer = String::new();
      }

      let open = has_open_attribute(rest);
      let mut details_lines = Vec::new();
      let mut details_fence: Option<(char, usize)> = None;
      let mut depth = 0isize;

      let (first_part, trailing, new_depth) = split_details_line(rest, depth);
      depth = new_depth;
      details_lines.push(first_part);
      if depth == 0 {
        if let Some(trailing) = trailing {
          pending_line = Some(trailing);
        }
      } else {
        while depth > 0 {
          let Some(next_line) = lines.next() else {
            break;
          };
          let next_line = next_line.to_string();
          update_fence_state(&next_line, &mut details_fence);
          if details_fence.is_some() {
            details_lines.push(next_line);
            continue;
          }

          let (part, trailing, new_depth) = split_details_line(&next_line, depth);
          depth = new_depth;
          details_lines.push(part);
          if depth == 0 {
            if let Some(trailing) = trailing {
              pending_line = Some(trailing);
            }
            break;
          }
        }
      }

      let details_source = details_lines.join("\n");
      if let Some((summary, body)) = parse_details_block(&details_source) {
        segments.push(Segment::Details {
          summary,
          body,
          open,
        });
      } else {
        segments.push(Segment::Markdown(details_source));
      }
      continue;
    }

    buffer.push_str(&line);
    buffer.push('\n');
  }

  if !buffer.is_empty() {
    segments.push(Segment::Markdown(buffer));
  }

  segments
}

pub(crate) fn update_fence_state(line: &str, fence: &mut Option<(char, usize)>) {
  let trimmed = line.trim_start();
  let mut chars = trimmed.chars();
  let Some(first) = chars.next() else {
    return;
  };
  if first != '`' && first != '~' {
    return;
  }
  let mut count = 1usize;
  for ch in chars {
    if ch == first {
      count += 1;
    } else {
      break;
    }
  }
  if count < 3 {
    return;
  }
  match fence {
    None => {
      *fence = Some((first, count));
    }
    Some((fence_char, fence_len)) if *fence_char == first && count >= *fence_len => {
      *fence = None;
    }
    _ => {}
  }
}

pub(crate) fn find_details_start(line: &str) -> Option<usize> {
  let lower = line.to_ascii_lowercase();
  lower.find("<details")
}

pub(crate) fn has_open_attribute(line: &str) -> bool {
  let lower = line.to_ascii_lowercase();
  if let Some(end) = lower.find('>') {
    let tag = &lower[..end];
    tag
      .split_whitespace()
      .any(|part| part == "open" || part.starts_with("open="))
  } else {
    false
  }
}

pub(crate) fn split_details_line(line: &str, mut depth: isize) -> (String, Option<String>, isize) {
  let lower = line.to_ascii_lowercase();
  let mut idx = 0usize;
  let mut split_at: Option<usize> = None;

  while idx < lower.len() {
    let next_open = lower[idx..].find("<details").map(|pos| idx + pos);
    let next_close = lower[idx..].find("</details").map(|pos| idx + pos);

    let (next_pos, is_open) = match (next_open, next_close) {
      (None, None) => break,
      (Some(pos), None) => (pos, true),
      (None, Some(pos)) => (pos, false),
      (Some(open_pos), Some(close_pos)) => {
        if open_pos <= close_pos {
          (open_pos, true)
        } else {
          (close_pos, false)
        }
      }
    };

    if is_open {
      depth += 1;
      idx = next_pos + "<details".len();
      continue;
    }

    depth -= 1;
    let close_end = match lower[next_pos..].find('>') {
      Some(rel) => next_pos + rel + 1,
      None => next_pos + "</details".len(),
    };
    idx = close_end;
    if depth <= 0 {
      depth = 0;
      split_at = Some(close_end);
      break;
    }
  }

  if let Some(end) = split_at {
    let (part, trailing) = line.split_at(end);
    let trailing = if trailing.is_empty() {
      None
    } else {
      Some(trailing.to_string())
    };
    return (part.to_string(), trailing, depth);
  }

  (line.to_string(), None, depth)
}

pub(crate) fn parse_details_block(source: &str) -> Option<(Option<String>, String)> {
  let lower = source.to_ascii_lowercase();
  let start = lower.find("<details")?;
  let open_tag_end = lower[start..].find('>')? + start;
  let end = find_last_details_close_end(&lower).unwrap_or(source.len());
  if end <= open_tag_end {
    return None;
  }
  let inner = &source[open_tag_end + 1..end];
  let (summary, body) = extract_summary(inner);
  Some((summary, body))
}

pub(crate) fn find_last_details_close_end(lower: &str) -> Option<usize> {
  let mut cursor = 0usize;
  let mut last_end = None;

  while cursor < lower.len() {
    let Some(rel_start) = lower[cursor..].find("</details") else {
      break;
    };
    let start = cursor + rel_start;
    let Some(rel_end) = lower[start..].find('>') else {
      break;
    };
    let end = start + rel_end + 1;
    last_end = Some(end);
    cursor = end;
  }

  last_end
}

pub(crate) fn blocks_from_node<'a>(node: &'a AstNode<'a>) -> Vec<Block> {
  match &node.data.borrow().value {
    NodeValue::Paragraph => vec![Block::Paragraph(inlines_from_nodes(node.children()))],
    NodeValue::Heading(heading) => vec![Block::Heading {
      level: heading.level,
      content: inlines_from_nodes(node.children()),
    }],
    NodeValue::List(list) => vec![Block::List(List {
      ordered: matches!(list.list_type, ListType::Ordered),
      start: if matches!(list.list_type, ListType::Ordered) {
        Some(list.start as u64)
      } else {
        None
      },
      items: node.children().filter_map(list_item_from_node).collect(),
    })],
    NodeValue::CodeBlock(code) => vec![Block::CodeBlock(CodeBlock {
      lang: code
        .info
        .split_whitespace()
        .next()
        .map(|value| value.to_string()),
      value: code.literal.clone(),
    })],
    NodeValue::BlockQuote => vec![Block::BlockQuote(
      node.children().flat_map(blocks_from_node).collect(),
    )],
    NodeValue::ThematicBreak => vec![Block::ThematicBreak],
    NodeValue::Table(_) => vec![Block::Table(table_from_node(node))],
    NodeValue::Item(_) => node.children().flat_map(blocks_from_node).collect(),
    NodeValue::HtmlBlock(html) => {
      if is_html_comment_only_block(&html.literal) || is_details_close_only_block(&html.literal) {
        Vec::new()
      } else if let Some(details) = parse_details_html(&html.literal) {
        vec![Block::Details(details)]
      } else if let Some(centered) = parse_centered_div_html(&html.literal) {
        centered
      } else {
        blocks_from_html_fragment(&html.literal)
      }
    }
    NodeValue::Text(text) => vec![Block::Paragraph(vec![Inline::Text(text.to_string())])],
    _ => {
      let text = collect_text(node);
      if text.is_empty() {
        Vec::new()
      } else {
        vec![Block::Paragraph(vec![Inline::Text(text)])]
      }
    }
  }
}

pub(crate) fn list_item_from_node<'a>(node: &'a AstNode<'a>) -> Option<ListItem> {
  // Task items sit directly under the list; they are not Item nodes wrapping a marker.
  let checked = match &node.data.borrow().value {
    NodeValue::Item(_) => None,
    NodeValue::TaskItem(marker) => Some(marker.symbol.is_some()),
    _ => return None,
  };
  Some(ListItem {
    blocks: node.children().flat_map(blocks_from_node).collect(),
    checked,
  })
}

pub(crate) fn table_from_node<'a>(node: &'a AstNode<'a>) -> Table {
  let mut headers: Vec<Vec<Inline>> = Vec::new();
  let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();

  for row_node in node.children() {
    let is_header = match row_node.data.borrow().value {
      NodeValue::TableRow(is_header) => is_header,
      _ => continue,
    };
    let mut row_cells: Vec<Vec<Inline>> = Vec::new();
    for cell_node in row_node.children() {
      if !matches!(cell_node.data.borrow().value, NodeValue::TableCell) {
        continue;
      }
      row_cells.push(inlines_from_nodes(cell_node.children()));
    }
    if is_header {
      headers = row_cells;
    } else {
      rows.push(row_cells);
    }
  }

  Table { headers, rows }
}

pub(crate) fn inlines_from_nodes<'a>(nodes: impl Iterator<Item = &'a AstNode<'a>>) -> Vec<Inline> {
  let mut inlines = Vec::new();
  for node in nodes {
    match &node.data.borrow().value {
      NodeValue::Text(text) => inlines.push(Inline::Text(text.to_string())),
      NodeValue::Code(code) => inlines.push(Inline::Code(code.literal.clone())),
      NodeValue::LineBreak => inlines.push(Inline::HardBreak),
      NodeValue::SoftBreak => inlines.push(Inline::SoftBreak),
      NodeValue::Strong => inlines.push(Inline::Strong(inlines_from_nodes(node.children()))),
      NodeValue::Emph => inlines.push(Inline::Emphasis(inlines_from_nodes(node.children()))),
      NodeValue::Strikethrough => {
        inlines.push(Inline::Strikethrough(inlines_from_nodes(node.children())))
      }
      NodeValue::Link(link) => inlines.push(Inline::Link {
        url: link.url.clone(),
        title: if link.title.is_empty() {
          None
        } else {
          Some(link.title.clone())
        },
        content: inlines_from_nodes(node.children()),
      }),
      NodeValue::Image(image) => {
        let alt = inline_to_plain_text(&inlines_from_nodes(node.children()));
        inlines.push(Inline::Image {
          url: image.url.clone(),
          title: if image.title.is_empty() {
            None
          } else {
            Some(image.title.clone())
          },
          alt,
          width: None,
          height: None,
          dark_url: None,
          light_url: None,
        });
      }
      NodeValue::HtmlInline(html) => {
        if let Some(nodes) = parse_html_fragment_nodes(html) {
          let parsed = html_nodes_to_inlines(&nodes);
          if !parsed.is_empty() {
            inlines.extend(parsed);
            continue;
          }
        }

        if let Some(image) = parse_inline_html_image(html) {
          inlines.push(image);
        } else if is_html_line_break_tag(html) {
          inlines.push(Inline::HardBreak);
        } else {
          let text = strip_html_tags(html);
          if !text.is_empty() {
            inlines.push(Inline::Text(text));
          }
        }
      }
      NodeValue::TaskItem(_) => {}
      _ => {
        let text = collect_text(node);
        if !text.is_empty() {
          inlines.push(Inline::Text(text));
        }
      }
    }
  }

  merge_adjacent_text(&inlines)
}

pub(crate) fn merge_adjacent_text(inlines: &[Inline]) -> Vec<Inline> {
  let mut merged = Vec::new();
  for inline in inlines {
    match (merged.last_mut(), inline) {
      (Some(Inline::Text(existing)), Inline::Text(new_text)) => {
        existing.push_str(new_text);
      }
      _ => merged.push(inline.clone()),
    }
  }
  merged
}

pub(crate) fn inline_to_plain_text(inlines: &[Inline]) -> String {
  inline_to_plain_text_with_soft_break(inlines, ' ')
}

/// A soft break is a space in plain markdown and a real break in hardbreaks mode.
pub(crate) fn inline_to_plain_text_with_soft_break(inlines: &[Inline], soft_break: char) -> String {
  let mut text = String::new();
  for inline in inlines {
    match inline {
      Inline::Text(value) => text.push_str(value),
      Inline::Code(value) => text.push_str(value),
      Inline::SoftBreak => text.push(soft_break),
      Inline::HardBreak => text.push('\n'),
      Inline::Strong(children) | Inline::Emphasis(children) | Inline::Strikethrough(children) => {
        text.push_str(&inline_to_plain_text_with_soft_break(children, soft_break))
      }
      Inline::Link { content, .. } => {
        text.push_str(&inline_to_plain_text_with_soft_break(content, soft_break))
      }
      Inline::Image { alt, .. } => text.push_str(alt),
    }
  }
  text
}

pub(crate) fn collect_text<'a>(node: &'a AstNode<'a>) -> String {
  match &node.data.borrow().value {
    NodeValue::Text(text) => text.to_string(),
    NodeValue::Code(code) => code.literal.clone(),
    NodeValue::Paragraph | NodeValue::Heading(_) => {
      inline_to_plain_text(&inlines_from_nodes(node.children()))
    }
    NodeValue::Link(link) => link.url.clone(),
    _ => String::new(),
  }
}

pub(crate) fn extract_url_token_candidates(text: &str) -> Vec<String> {
  let mut tokens = Vec::new();
  let mut remaining = text;

  loop {
    let https_idx = remaining.find("https://");
    let http_idx = remaining.find("http://");
    let start = match (https_idx, http_idx) {
      (None, None) => break,
      (Some(https), None) => https,
      (None, Some(http)) => http,
      (Some(https), Some(http)) => https.min(http),
    };
    remaining = &remaining[start..];

    let end = remaining
      .find(char::is_whitespace)
      .unwrap_or(remaining.len());
    let token = remaining[..end]
      .trim_matches(|ch: char| {
        matches!(
          ch,
          '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | ',' | ';'
        )
      })
      .trim_end_matches('.')
      .to_string();
    if !token.is_empty() {
      tokens.push(token);
    }
    remaining = &remaining[end..];
  }

  tokens
}

pub fn parse_github_blob_line_reference(url: &str) -> Option<GithubBlobLineReference> {
  let trimmed = url.trim();
  let rest = trimmed
    .strip_prefix("https://github.com/")
    .or_else(|| trimmed.strip_prefix("http://github.com/"))?;
  let (path_and_blob, fragment) = rest.split_once('#')?;
  let path_and_blob = path_and_blob.split('?').next().unwrap_or(path_and_blob);
  let fragment = fragment.strip_prefix('L')?;
  let start_digits: String = fragment
    .chars()
    .take_while(|ch| ch.is_ascii_digit())
    .collect();
  if start_digits.is_empty() {
    return None;
  }
  let start_line = start_digits.parse::<usize>().ok()?;
  if start_line == 0 {
    return None;
  }
  let fragment_tail = &fragment[start_digits.len()..];
  let end_line = if fragment_tail.is_empty() {
    start_line
  } else {
    let fragment_tail = fragment_tail.strip_prefix('-')?;
    let fragment_tail = fragment_tail.strip_prefix('L').unwrap_or(fragment_tail);
    let end_digits: String = fragment_tail
      .chars()
      .take_while(|ch| ch.is_ascii_digit())
      .collect();
    if end_digits.is_empty() {
      return None;
    }
    let parsed = end_digits.parse::<usize>().ok()?;
    if parsed == 0 {
      return None;
    }
    parsed
  };
  let (start_line, end_line) = if start_line <= end_line {
    (start_line, end_line)
  } else {
    (end_line, start_line)
  };

  let (repo_path, blob_path) = path_and_blob.split_once("/blob/")?;
  let mut repo_parts = repo_path.split('/');
  let owner = repo_parts.next()?.trim();
  let repo = repo_parts.next()?.trim();
  if owner.is_empty() || repo.is_empty() || repo_parts.next().is_some() {
    return None;
  }

  let (reference, file_path) = blob_path.split_once('/')?;
  if reference.is_empty() || file_path.is_empty() {
    return None;
  }

  Some(GithubBlobLineReference {
    url: trimmed.to_string(),
    owner: owner.to_string(),
    repo: repo.to_string(),
    reference: reference.to_string(),
    path: file_path.to_string(),
    start_line,
    end_line,
  })
}

pub fn extract_github_blob_line_references(text: &str) -> Vec<GithubBlobLineReference> {
  let mut references = Vec::new();
  let mut seen = HashSet::new();

  for candidate in extract_url_token_candidates(text) {
    if let Some(reference) = parse_github_blob_line_reference(&candidate)
      && seen.insert(reference.url.clone())
    {
      references.push(reference);
    }
  }

  references
}
