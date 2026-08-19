//! Fenced code blocks in chat markdown, rendered with the app's own grammar
//! stack: gpui-component's highlighter is compiled out (no `tree-sitter`
//! feature), so its built-in code blocks paint unstyled monospace.

use gpui::{
  App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _, Window,
  div, px,
};
use gpui_component::{
  ActiveTheme as _,
  clipboard::Clipboard,
  h_flex,
  text::{MarkdownExtensions, MarkdownNode, MarkdownParseContext, markdown_ast},
  v_flex,
};
use selectable_text::{SelectableText, SelectionRegistry};
use syntax::{HighlightSpan, SyntaxHighlighter, highlights_to_text_runs, languages};

const NODE_NAME: &str = "chat-code-block";

struct CodeBlock {
  lang: Option<SharedString>,
  code: SharedString,
  spans: Vec<HighlightSpan>,
  text_id: u64,
}

/// Build once per panel and reuse: every mutation of `MarkdownExtensions`
/// bumps a global revision that busts TextView's parse cache.
pub(crate) fn extensions(registry: SelectionRegistry) -> MarkdownExtensions {
  MarkdownExtensions::default()
    .block_parser(parse)
    .block_renderer(NODE_NAME, move |node, window, cx| {
      render(node, &registry, window, cx)
    })
}

/// Runs on the parse task, off the UI thread: highlighting happens here so the
/// renderer only paints precomputed spans.
fn parse(node: &markdown_ast::Node, cx: &MarkdownParseContext) -> Option<MarkdownNode> {
  let markdown_ast::Node::Code(code) = node else {
    return None;
  };
  let lang = normalized_lang(code.lang.as_deref());
  let spans = highlight(lang.as_deref(), &code.value);
  let markdown = cx
    .node_source(node)
    .unwrap_or(code.value.as_str())
    .to_string();
  let block = CodeBlock {
    lang: lang.map(SharedString::from),
    code: SharedString::from(code.value.clone()),
    spans,
    text_id: fnv1a(code.value.as_bytes()),
  };
  Some(
    MarkdownNode::new(NODE_NAME, block)
      .text(code.value.clone())
      .markdown(markdown),
  )
}

fn render(
  node: &MarkdownNode,
  registry: &SelectionRegistry,
  _window: &mut Window,
  cx: &mut App,
) -> gpui::AnyElement {
  let Some(block) = node.data::<CodeBlock>() else {
    return gpui::Empty.into_any_element();
  };
  let theme = cx.theme();
  let syntax_theme = ui::Theme::new(theme.mode.is_dark()).syntax();
  let runs = highlights_to_text_runs(
    &block.spans,
    &block.code,
    theme.foreground,
    crate::mono_font_for(theme),
    &syntax_theme,
  );

  let selector_lang = block
    .lang
    .clone()
    .unwrap_or_else(|| SharedString::from("text"));
  v_flex()
    .debug_selector(move || format!("chat-code-block-{selector_lang}"))
    .my_1()
    .w_full()
    .rounded(px(6.))
    .border_1()
    .border_color(theme.border)
    .bg(theme.background)
    .overflow_hidden()
    .child(
      h_flex()
        .items_center()
        .justify_between()
        .pl_2()
        .pr_1()
        .py_0p5()
        .border_b_1()
        .border_color(theme.border)
        .child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(block.lang.clone().unwrap_or_else(|| "text".into())),
        )
        .child(
          div().debug_selector(|| "chat-code-copy".to_string()).child(
            Clipboard::new(SharedString::from(format!(
              "chat-code-copy-{:x}",
              block.text_id
            )))
            .value(block.code.clone())
            .tooltip("Copy code"),
          ),
        ),
    )
    .child(
      div()
        .px_2()
        .py_1()
        .font_family("monospace")
        .text_xs()
        .text_color(theme.foreground)
        .whitespace_normal()
        .child(SelectableText::new(
          block.text_id,
          block.code.clone(),
          runs,
          registry.clone(),
        )),
    )
    .into_any_element()
}

/// First word of the fence info string, lowercased ("Rust title=x" -> "rust").
fn normalized_lang(info: Option<&str>) -> Option<String> {
  info
    .and_then(|l| l.split_whitespace().next())
    .map(str::to_lowercase)
}

fn highlight(lang: Option<&str>, code: &str) -> Vec<HighlightSpan> {
  lang
    .and_then(languages::language_config_for_name)
    .and_then(|cfg| SyntaxHighlighter::new(cfg).highlight_text(code).ok())
    .unwrap_or_default()
}

pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
  let mut hash = 0xcbf2_9ce4_8422_2325u64;
  for byte in bytes {
    hash ^= u64::from(*byte);
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
  }
  hash
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_rust_fence_gets_highlight_spans() {
    let spans = highlight(Some("rust"), "fn main() { let x = 1; }");
    assert!(!spans.is_empty());
  }

  #[test]
  fn an_unknown_language_yields_no_spans() {
    assert!(highlight(Some("nosuchlang"), "whatever").is_empty());
    assert!(highlight(None, "whatever").is_empty());
  }

  #[test]
  fn the_fence_info_keeps_only_its_first_word_lowercased() {
    assert_eq!(normalized_lang(Some("Rust title=x")), Some("rust".into()));
    assert_eq!(normalized_lang(Some("")), None);
    assert_eq!(normalized_lang(None), None);
  }
}
