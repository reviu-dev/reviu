pub(crate) mod constants;
mod gfm_markdown_viewer;
mod height_estimation;
pub(crate) mod image;
pub(crate) mod parse;
pub(crate) mod parse_html;
mod parsed_cache;
mod preview_segments;
pub(crate) mod selection;
mod syntax_cache;
pub(crate) mod types;

pub use gfm_markdown_viewer::{
  MarkdownRenderOptions, MarkdownRenderState, render_github_code_reference_preview_card,
  render_markdown, render_parsed_markdown,
};
pub use height_estimation::{
  estimate_github_code_reference_preview_height_px, estimate_markdown_height_px,
  estimate_parsed_markdown_height_px,
};
pub use image::is_github_user_attachment_url;
pub use parse::{
  extract_github_blob_line_references, parse_github_blob_line_reference, parse_markdown,
};
pub use syntax_cache::SyntaxHighlightCache;
pub use types::{
  GithubBlobLineReference, GithubCodeReferencePreview, GithubIssueReferenceContext, LinkAction,
  ParsedMarkdown, SuggestionContext,
};
