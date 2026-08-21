pub(crate) mod constants;
mod gfm_markdown_viewer;
mod height_estimation;
pub(crate) mod image;
pub(crate) mod parse;
pub(crate) mod parse_html;
mod parsed_cache;
pub(crate) mod selection;
mod syntax_cache;
pub(crate) mod types;

pub use gfm_markdown_viewer::{
  MarkdownRenderOptions, MarkdownRenderState, render_github_code_reference_preview_card,
  render_markdown, render_parsed_markdown,
};
pub use height_estimation::{
  MarkdownTextMetrics, MarkdownTextWidthFn, estimate_github_code_reference_preview_height_px,
  estimate_markdown_height_px_with_suggestion_context,
  estimate_parsed_markdown_height_px_with_suggestion_context,
};
pub use image::is_github_user_attachment_url;
pub use parse::{extract_github_blob_line_references, parse_markdown};
pub use syntax_cache::SyntaxHighlightCache;
pub use types::{
  GithubBlobLineReference, GithubCodeReferencePreview, LinkAction, ParsedMarkdown,
  SuggestionActionContext, SuggestionContext,
};
