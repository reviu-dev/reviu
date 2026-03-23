pub(crate) mod constants;
mod gfm_markdown_viewer;
mod parsed_cache;
mod preview_segments;
mod syntax_cache;
pub(crate) mod types;

pub use gfm_markdown_viewer::{
  GithubBlobLineReference, GithubCodeReferencePreview, GithubIssueReferenceContext, LinkAction,
  MarkdownRenderOptions, MarkdownRenderState, ParsedMarkdown,
  estimate_github_code_reference_preview_height_px, estimate_markdown_height_px,
  estimate_parsed_markdown_height_px, extract_github_blob_line_references,
  parse_github_blob_line_reference, parse_markdown, render_github_code_reference_preview_card,
  render_markdown, render_parsed_markdown,
};
pub use syntax_cache::SyntaxHighlightCache;
