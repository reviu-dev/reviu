use tree_sitter::{Node as TsNode, Parser as TsParser};

use crate::image::{inline_contains_image, inline_image_data};
use crate::parse::*;
use crate::types::*;

pub(crate) fn parse_details_html(html: &str) -> Option<Details> {
  let trimmed = html.trim();
  let lower = trimmed.to_ascii_lowercase();
  let start = lower.find("<details")?;
  let open_tag_end = lower[start..].find('>')? + start;
  let open = has_open_attribute(&lower[start..=open_tag_end]);
  let end = find_last_details_close_end(&lower).unwrap_or(trimmed.len());
  if end <= open_tag_end {
    return None;
  }

  let inner = &trimmed[open_tag_end + 1..end];
  let (summary_text, body_text) = extract_summary(inner);
  let summary_inlines = summary_inlines_from_text(summary_text.as_deref().unwrap_or("Details"));
  let blocks = if body_text.trim().is_empty() {
    Vec::new()
  } else {
    parse_gfm(&body_text)
  };

  Some(Details {
    summary: summary_inlines,
    blocks,
    open,
  })
}

pub(crate) fn is_centered_div_open_tag(html: &str) -> bool {
  let trimmed = html.trim();
  let lower = trimmed.to_ascii_lowercase();
  if !lower.starts_with("<div")
    || lower.starts_with("</div")
    || lower.contains("</div")
    || trimmed.ends_with("/>")
  {
    return false;
  }

  if let Some(align) = extract_html_attribute(trimmed, "align")
    && align.trim().eq_ignore_ascii_case("center")
  {
    return true;
  }

  if let Some(style) = extract_html_attribute(trimmed, "style") {
    let normalized = style
      .chars()
      .filter(|ch| !ch.is_whitespace())
      .collect::<String>()
      .to_ascii_lowercase();
    if normalized.contains("text-align:center") {
      return true;
    }
  }

  false
}

pub(crate) fn is_centered_div_close_tag(html: &str) -> bool {
  html.trim_start().to_ascii_lowercase().starts_with("</div")
}

pub(crate) fn parse_centered_div_html(html: &str) -> Option<Vec<Block>> {
  let trimmed = html.trim();
  let lower = trimmed.to_ascii_lowercase();
  if !lower.starts_with("<div") || !lower.contains("</div>") {
    return None;
  }

  let open_end = lower.find('>')?;
  let open_tag = &trimmed[..=open_end];
  if !is_centered_div_open_tag(open_tag) {
    return None;
  }

  let close_start = lower.rfind("</div>")?;
  if close_start <= open_end {
    return None;
  }

  let body = &trimmed[open_end + 1..close_start];
  let blocks = parse_gfm(body);
  if blocks.is_empty() {
    return Some(Vec::new());
  }

  Some(vec![Block::Aligned {
    center: true,
    blocks,
  }])
}

pub(crate) fn is_html_comment_only_block(html: &str) -> bool {
  let mut rest = html.trim();
  if rest.is_empty() {
    return false;
  }

  while !rest.is_empty() {
    if !rest.starts_with("<!--") {
      return false;
    }

    let Some(end) = rest.find("-->") else {
      return false;
    };

    rest = rest[end + 3..].trim_start();
  }

  true
}

pub(crate) fn is_details_close_only_block(html: &str) -> bool {
  let mut rest = html.trim();
  if rest.is_empty() {
    return false;
  }

  while !rest.is_empty() {
    let lower = rest.to_ascii_lowercase();
    if !lower.starts_with("</details") {
      return false;
    }

    let Some(close_idx) = rest.find('>') else {
      return false;
    };
    rest = rest[close_idx + 1..].trim_start();
  }

  true
}

pub(crate) fn extract_summary(inner: &str) -> (Option<String>, String) {
  let lower = inner.to_ascii_lowercase();
  let summary_start = match lower.find("<summary") {
    Some(index) => index,
    None => return (None, inner.to_string()),
  };
  let tag_end = match lower[summary_start..].find('>') {
    Some(index) => summary_start + index,
    None => return (None, inner.to_string()),
  };
  let close_start = match lower[tag_end..].find("</summary>") {
    Some(index) => tag_end + index,
    None => return (None, inner.to_string()),
  };
  let summary_inner = inner[tag_end + 1..close_start].to_string();
  let mut body = String::new();
  body.push_str(&inner[..summary_start]);
  body.push_str(&inner[close_start + "</summary>".len()..]);
  (Some(summary_inner), body)
}

pub(crate) fn summary_inlines_from_text(summary_text: &str) -> Vec<Inline> {
  for block in parse_gfm(summary_text) {
    if let Block::Paragraph(inlines) = block {
      if !inline_to_plain_text(&inlines).is_empty() {
        return inlines;
      }
      break;
    }
  }
  vec![Inline::Text(strip_html_tags(summary_text))]
}

pub(crate) fn strip_html_tags(input: &str) -> String {
  let mut out = String::new();
  let mut in_tag = false;
  for ch in input.chars() {
    match ch {
      '<' => in_tag = true,
      '>' => in_tag = false,
      _ if !in_tag => out.push(ch),
      _ => {}
    }
  }
  out
}

pub(crate) fn parse_inline_html_image(html: &str) -> Option<Inline> {
  let lower = html.to_ascii_lowercase();
  let start = lower.find("<img")?;
  let end = lower[start..].find('>')? + start;
  let tag = &html[start..=end];
  let url = extract_html_attribute(tag, "src")?;
  let alt = extract_html_attribute(tag, "alt").unwrap_or_default();
  let title = extract_html_attribute(tag, "title");
  let width = extract_html_attribute(tag, "width")
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty());
  let height = extract_html_attribute(tag, "height")
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty());
  Some(Inline::Image {
    url,
    title,
    alt,
    width,
    height,
    dark_url: None,
    light_url: None,
  })
}

pub(crate) fn is_html_line_break_tag(html: &str) -> bool {
  let trimmed = html.trim();
  if !trimmed.starts_with('<') || !trimmed.ends_with('>') || trimmed.len() < 3 {
    return false;
  }

  let inner = &trimmed[1..trimmed.len() - 1];
  let inner = inner.trim();
  let inner = inner.strip_suffix('/').unwrap_or(inner).trim_end();
  inner.eq_ignore_ascii_case("br")
}

pub(crate) fn decode_basic_html_entities(segment: &str) -> String {
  segment
    .replace("&nbsp;", " ")
    .replace("&amp;", "&")
    .replace("&lt;", "<")
    .replace("&gt;", ">")
    .replace("&quot;", "\"")
    .replace("&#39;", "'")
}

pub(crate) fn push_html_text_segment(
  inlines: &mut Vec<Inline>,
  segment: &str,
  pending_space: &mut bool,
) {
  let segment = decode_basic_html_entities(segment);
  for ch in segment.chars() {
    if ch.is_whitespace() {
      *pending_space = true;
      continue;
    }

    if *pending_space {
      if let Some(Inline::Text(last)) = inlines.last_mut()
        && !last.is_empty()
        && !last.ends_with('\n')
      {
        last.push(' ');
      }
      *pending_space = false;
    }

    if let Some(Inline::Text(last)) = inlines.last_mut() {
      last.push(ch);
    } else {
      inlines.push(Inline::Text(ch.to_string()));
    }
  }
}

pub(crate) fn push_html_inline(
  inlines: &mut Vec<Inline>,
  inline: Inline,
  pending_space: &mut bool,
) {
  match inline {
    Inline::Text(text) => {
      push_html_text_segment(inlines, &text, pending_space);
    }
    Inline::HardBreak => {
      inlines.push(Inline::HardBreak);
      *pending_space = false;
    }
    other => {
      if *pending_space {
        if let Some(Inline::Text(last)) = inlines.last_mut()
          && !last.is_empty()
          && !last.ends_with('\n')
        {
          last.push(' ');
        } else if !inlines.is_empty() {
          inlines.push(Inline::Text(" ".to_string()));
        }
        *pending_space = false;
      }
      inlines.push(other);
    }
  }
}

pub(crate) fn push_html_inlines(
  inlines: &mut Vec<Inline>,
  items: Vec<Inline>,
  pending_space: &mut bool,
) {
  for inline in items {
    push_html_inline(inlines, inline, pending_space);
  }
}

pub(crate) fn html_inlines_have_visible_content(inlines: &[Inline]) -> bool {
  if inlines.is_empty() {
    return false;
  }
  if inlines.iter().any(inline_contains_image) {
    return true;
  }
  if inlines
    .iter()
    .any(|inline| matches!(inline, Inline::HardBreak))
  {
    return true;
  }
  !inline_to_plain_text(inlines).trim().is_empty()
}

pub(crate) fn flush_html_inline_buffer_to_blocks(
  blocks: &mut Vec<Block>,
  inlines: &mut Vec<Inline>,
) {
  let merged = merge_adjacent_text(inlines);
  inlines.clear();
  if html_inlines_have_visible_content(&merged) {
    blocks.push(Block::Paragraph(merged));
  }
}

pub(crate) fn blocks_from_html_fragment(html: &str) -> Vec<Block> {
  if let Some(nodes) = parse_html_fragment_nodes(html) {
    let blocks = html_nodes_to_blocks(&nodes);
    if !blocks.is_empty() {
      return blocks;
    }
  }

  let inlines = legacy_inlines_from_html_fragment(html);
  if inlines.is_empty() {
    Vec::new()
  } else {
    vec![Block::Paragraph(inlines)]
  }
}

pub(crate) fn parse_html_fragment_nodes(html: &str) -> Option<Vec<HtmlNode>> {
  let mut parser = TsParser::new();
  let language: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
  parser.set_language(&language).ok()?;
  let tree = parser.parse(html, None)?;
  let root = tree.root_node();

  let mut nodes = Vec::new();
  let mut cursor = root.walk();
  let mut previous_end = root.start_byte();
  for child in root.named_children(&mut cursor) {
    append_html_text_node_from_range(&mut nodes, html, previous_end, child.start_byte());
    if let Some(node) = html_node_from_tree_sitter(child, html) {
      nodes.push(node);
    }
    previous_end = child.end_byte();
  }
  append_html_text_node_from_range(&mut nodes, html, previous_end, root.end_byte());

  Some(nodes)
}

pub(crate) fn tree_sitter_node_text<'a>(node: TsNode<'a>, source: &'a str) -> &'a str {
  source.get(node.byte_range()).unwrap_or_default()
}

pub(crate) fn append_html_text_node_from_range(
  nodes: &mut Vec<HtmlNode>,
  source: &str,
  start: usize,
  end: usize,
) {
  if end <= start {
    return;
  }
  if let Some(text) = source.get(start..end)
    && !text.is_empty()
  {
    nodes.push(HtmlNode::Text(text.to_string()));
  }
}

pub(crate) fn html_node_from_tree_sitter(node: TsNode<'_>, source: &str) -> Option<HtmlNode> {
  match node.kind() {
    "element" => html_element_from_tree_sitter_element(node, source).map(HtmlNode::Element),
    "self_closing_tag" => {
      let (tag, attrs) = html_tag_from_tree_sitter(node, source)?;
      Some(HtmlNode::Element(HtmlElement {
        tag,
        attrs,
        children: Vec::new(),
      }))
    }
    "text" | "entity" => {
      let text = tree_sitter_node_text(node, source).to_string();
      if text.is_empty() {
        None
      } else {
        Some(HtmlNode::Text(text))
      }
    }
    _ => None,
  }
}

pub(crate) fn html_element_from_tree_sitter_element(
  node: TsNode<'_>,
  source: &str,
) -> Option<HtmlElement> {
  let mut tag_name = None;
  let mut attrs = Vec::new();
  let mut children = Vec::new();
  let mut content_start = None;

  let mut cursor = node.walk();
  for child in node.named_children(&mut cursor) {
    match child.kind() {
      "start_tag" | "self_closing_tag" => {
        if let Some((tag, parsed_attrs)) = html_tag_from_tree_sitter(child, source) {
          tag_name = Some(tag);
          attrs = parsed_attrs;
          content_start = Some(child.end_byte());
        }
      }
      "end_tag" | "erroneous_end_tag" => {
        if let Some(start) = content_start.take() {
          append_html_text_node_from_range(&mut children, source, start, child.start_byte());
        }
      }
      _ => {
        if let Some(start) = content_start {
          append_html_text_node_from_range(&mut children, source, start, child.start_byte());
        }
        if let Some(mapped) = html_node_from_tree_sitter(child, source) {
          children.push(mapped);
        }
        content_start = Some(child.end_byte());
      }
    }
  }
  if let Some(start) = content_start {
    append_html_text_node_from_range(&mut children, source, start, node.end_byte());
  }

  Some(HtmlElement {
    tag: tag_name?,
    attrs,
    children,
  })
}

pub(crate) fn html_tag_from_tree_sitter(
  node: TsNode<'_>,
  source: &str,
) -> Option<(String, Vec<HtmlAttribute>)> {
  let mut tag_name = None;
  let mut attrs = Vec::new();

  let mut cursor = node.walk();
  for child in node.named_children(&mut cursor) {
    match child.kind() {
      "tag_name" => {
        let name = tree_sitter_node_text(child, source).trim();
        if !name.is_empty() {
          tag_name = Some(name.to_ascii_lowercase());
        }
      }
      "attribute" => {
        if let Some(attr) = html_attribute_from_tree_sitter(child, source) {
          attrs.push(attr);
        }
      }
      _ => {}
    }
  }

  Some((tag_name?, attrs))
}

pub(crate) fn html_attribute_from_tree_sitter(
  node: TsNode<'_>,
  source: &str,
) -> Option<HtmlAttribute> {
  let mut name = None;
  let mut value = None;

  let mut cursor = node.walk();
  for child in node.named_children(&mut cursor) {
    match child.kind() {
      "attribute_name" => {
        let attr = tree_sitter_node_text(child, source).trim();
        if !attr.is_empty() {
          name = Some(attr.to_ascii_lowercase());
        }
      }
      "attribute_value" => {
        value = Some(tree_sitter_node_text(child, source).to_string());
      }
      "quoted_attribute_value" => {
        let mut quoted_cursor = child.walk();
        let mut parsed = None;
        for quoted_child in child.named_children(&mut quoted_cursor) {
          if quoted_child.kind() == "attribute_value" {
            parsed = Some(tree_sitter_node_text(quoted_child, source).to_string());
            break;
          }
        }
        value = Some(parsed.unwrap_or_default());
      }
      _ => {}
    }
  }

  Some(HtmlAttribute { name: name?, value })
}

pub(crate) fn html_attribute_value<'a>(element: &'a HtmlElement, name: &str) -> Option<&'a str> {
  element
    .attrs
    .iter()
    .find(|attr| attr.name.eq_ignore_ascii_case(name))
    .and_then(|attr| attr.value.as_deref())
}

pub(crate) fn parse_html_picture_source_url(srcset: &str) -> Option<String> {
  srcset.split(',').find_map(|candidate| {
    candidate
      .split_whitespace()
      .next()
      .map(str::trim)
      .filter(|value| !value.is_empty())
      .map(ToString::to_string)
  })
}

pub(crate) fn html_picture_theme_urls(element: &HtmlElement) -> (Option<String>, Option<String>) {
  let mut dark_url = None;
  let mut light_url = None;

  for child in &element.children {
    let HtmlNode::Element(source) = child else {
      continue;
    };
    if source.tag != "source" {
      continue;
    }

    let Some(srcset) = html_attribute_value(source, "srcset")
      .map(str::trim)
      .filter(|value| !value.is_empty())
    else {
      continue;
    };
    let Some(source_url) = parse_html_picture_source_url(srcset) else {
      continue;
    };
    let media = html_attribute_value(source, "media")
      .map(str::trim)
      .unwrap_or_default()
      .to_ascii_lowercase();
    if media.contains("prefers-color-scheme") && media.contains("dark") {
      if dark_url.is_none() {
        dark_url = Some(source_url);
      }
      continue;
    }
    if media.contains("prefers-color-scheme") && media.contains("light") && light_url.is_none() {
      light_url = Some(source_url);
    }
  }

  (dark_url, light_url)
}

pub(crate) fn html_element_is_centered(element: &HtmlElement) -> bool {
  if let Some(align) = html_attribute_value(element, "align")
    && align.trim().eq_ignore_ascii_case("center")
  {
    return true;
  }

  if let Some(style) = html_attribute_value(element, "style") {
    let normalized = style
      .chars()
      .filter(|ch| !ch.is_whitespace())
      .collect::<String>()
      .to_ascii_lowercase();
    if normalized.contains("text-align:center") {
      return true;
    }
  }

  false
}

pub(crate) fn is_html_block_level_tag(tag: &str) -> bool {
  matches!(tag, "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

pub(crate) fn html_nodes_to_blocks(nodes: &[HtmlNode]) -> Vec<Block> {
  let mut blocks = Vec::new();
  let mut inline_buffer = Vec::new();
  let mut pending_space = false;

  for node in nodes {
    match node {
      HtmlNode::Text(text) => {
        push_html_text_segment(&mut inline_buffer, text, &mut pending_space);
      }
      HtmlNode::Element(element) => {
        if is_html_block_level_tag(element.tag.as_str()) {
          flush_html_inline_buffer_to_blocks(&mut blocks, &mut inline_buffer);
          blocks.extend(html_element_to_blocks(element));
        } else {
          let inlines = html_element_to_inlines(element);
          push_html_inlines(&mut inline_buffer, inlines, &mut pending_space);
        }
      }
    }
  }

  flush_html_inline_buffer_to_blocks(&mut blocks, &mut inline_buffer);
  blocks
}

pub(crate) fn html_nodes_to_inlines(nodes: &[HtmlNode]) -> Vec<Inline> {
  let mut inlines = Vec::new();
  let mut pending_space = false;

  for node in nodes {
    match node {
      HtmlNode::Text(text) => {
        push_html_text_segment(&mut inlines, text, &mut pending_space);
      }
      HtmlNode::Element(element) => {
        let child_inlines = html_element_to_inlines(element);
        push_html_inlines(&mut inlines, child_inlines, &mut pending_space);
      }
    }
  }

  merge_adjacent_text(&inlines)
}

pub(crate) fn html_heading_level(tag: &str) -> Option<u8> {
  match tag {
    "h1" => Some(1),
    "h2" => Some(2),
    "h3" => Some(3),
    "h4" => Some(4),
    "h5" => Some(5),
    "h6" => Some(6),
    _ => None,
  }
}

pub(crate) fn html_element_to_blocks(element: &HtmlElement) -> Vec<Block> {
  if let Some(level) = html_heading_level(element.tag.as_str()) {
    let content = html_nodes_to_inlines(&element.children);
    if !html_inlines_have_visible_content(&content) {
      return Vec::new();
    }

    let heading = Block::Heading { level, content };
    if html_element_is_centered(element) {
      return vec![Block::Aligned {
        center: true,
        blocks: vec![heading],
      }];
    }
    return vec![heading];
  }

  let mut blocks = html_nodes_to_blocks(&element.children);
  if blocks.is_empty() {
    let fallback = html_nodes_to_inlines(&element.children);
    if html_inlines_have_visible_content(&fallback) {
      blocks.push(Block::Paragraph(fallback));
    }
  }

  if blocks.is_empty() {
    return blocks;
  }

  if html_element_is_centered(element) {
    vec![Block::Aligned {
      center: true,
      blocks,
    }]
  } else {
    blocks
  }
}

pub(crate) fn html_element_to_inlines(element: &HtmlElement) -> Vec<Inline> {
  match element.tag.as_str() {
    "br" => vec![Inline::HardBreak],
    "img" => {
      let Some(url) = html_attribute_value(element, "src")
        .map(str::trim)
        .filter(|value| !value.is_empty())
      else {
        return Vec::new();
      };
      vec![Inline::Image {
        url: url.to_string(),
        title: html_attribute_value(element, "title")
          .map(str::trim)
          .filter(|value| !value.is_empty())
          .map(ToString::to_string),
        alt: html_attribute_value(element, "alt")
          .map(str::trim)
          .unwrap_or_default()
          .to_string(),
        width: html_attribute_value(element, "width")
          .map(str::trim)
          .filter(|value| !value.is_empty())
          .map(ToString::to_string),
        height: html_attribute_value(element, "height")
          .map(str::trim)
          .filter(|value| !value.is_empty())
          .map(ToString::to_string),
        dark_url: None,
        light_url: None,
      }]
    }
    "a" => {
      let content = html_nodes_to_inlines(&element.children);
      let Some(url) = html_attribute_value(element, "href")
        .map(str::trim)
        .filter(|value| !value.is_empty())
      else {
        return content;
      };
      if content.is_empty() {
        return vec![Inline::Text(url.to_string())];
      }
      vec![Inline::Link {
        url: url.to_string(),
        title: html_attribute_value(element, "title")
          .map(str::trim)
          .filter(|value| !value.is_empty())
          .map(ToString::to_string),
        content,
      }]
    }
    "picture" => {
      let children = html_nodes_to_inlines(&element.children);
      let (picture_dark_url, picture_light_url) = html_picture_theme_urls(element);
      if let Some(image_data) = children.iter().find_map(inline_image_data) {
        vec![Inline::Image {
          url: image_data.url,
          title: None,
          alt: image_data.alt,
          width: image_data.width_hint,
          height: image_data.height_hint,
          dark_url: picture_dark_url.or(image_data.dark_url),
          light_url: picture_light_url.or(image_data.light_url),
        }]
      } else {
        children
      }
    }
    "source" => Vec::new(),
    "span" | "sub" | "sup" => html_nodes_to_inlines(&element.children),
    _ => html_nodes_to_inlines(&element.children),
  }
}

pub(crate) fn legacy_inlines_from_html_fragment(html: &str) -> Vec<Inline> {
  let mut inlines = Vec::new();
  let mut cursor = 0usize;
  let mut pending_space = false;

  while cursor < html.len() {
    let Some(rel_lt) = html[cursor..].find('<') else {
      push_html_text_segment(&mut inlines, &html[cursor..], &mut pending_space);
      break;
    };

    let lt = cursor + rel_lt;
    push_html_text_segment(&mut inlines, &html[cursor..lt], &mut pending_space);

    let Some(rel_gt) = html[lt..].find('>') else {
      push_html_text_segment(&mut inlines, &html[lt..], &mut pending_space);
      break;
    };

    let gt = lt + rel_gt + 1;
    let tag = &html[lt..gt];
    if let Some(image) = parse_inline_html_image(tag) {
      inlines.push(image);
    } else if is_html_line_break_tag(tag) {
      inlines.push(Inline::HardBreak);
      pending_space = false;
    }

    cursor = gt;
  }

  merge_adjacent_text(&inlines)
}

pub(crate) fn extract_html_attribute(tag: &str, name: &str) -> Option<String> {
  let lower = tag.to_ascii_lowercase();
  let pattern = format!("{name}=");
  let mut cursor = 0usize;

  while cursor < lower.len() {
    let Some(rel_start) = lower[cursor..].find(&pattern) else {
      break;
    };
    let start = cursor + rel_start;

    if start > 0 {
      let prev = lower.as_bytes()[start - 1];
      if prev.is_ascii_alphanumeric() || prev == b'-' || prev == b'_' {
        cursor = start + pattern.len();
        continue;
      }
    }

    let mut value_start = start + pattern.len();
    while value_start < tag.len() && tag.as_bytes()[value_start].is_ascii_whitespace() {
      value_start += 1;
    }
    if value_start >= tag.len() {
      return None;
    }

    let bytes = tag.as_bytes();
    let delimiter = bytes[value_start];
    if delimiter == b'"' || delimiter == b'\'' {
      let content_start = value_start + 1;
      let rest = &tag[content_start..];
      if let Some(close_rel) = rest.find(delimiter as char) {
        return Some(rest[..close_rel].to_string());
      }
      return Some(rest.to_string());
    }

    let mut value_end = value_start;
    while value_end < tag.len() {
      let ch = bytes[value_end];
      if ch.is_ascii_whitespace() || ch == b'>' || ch == b'/' {
        break;
      }
      value_end += 1;
    }
    return Some(tag[value_start..value_end].to_string());
  }

  None
}
