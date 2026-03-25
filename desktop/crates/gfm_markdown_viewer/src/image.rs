use std::{
  collections::HashMap,
  fs,
  hash::{DefaultHasher, Hash, Hasher},
  path::PathBuf,
  sync::{Arc, Mutex},
  time::Duration,
};

use base64::{Engine as _, engine::general_purpose};
use gpui::{
  AnyElement, App, Hsla, ImageCacheError, ImgResourceLoader, ObjectFit, RenderImage, Resource,
  SharedString, Window, div, img, prelude::*, px, relative,
};
use gpui_component::{ActiveTheme as _, h_flex};
use once_cell::sync::Lazy;
use reqwest::header::CONTENT_TYPE;

use crate::constants::*;
use crate::gfm_markdown_viewer::{
  LinkHandlerFn, MarkdownRenderOptions, RenderContext, render_inline_text,
};
use crate::parse_html::extract_html_attribute;
use crate::types::*;

pub(crate) static BADGE_IMAGE_SOURCE_CACHE: Lazy<Mutex<HashMap<String, BadgeResolveState>>> =
  Lazy::new(|| Mutex::new(HashMap::new()));

static GITHUB_ASSET_URL_CACHE: Lazy<Mutex<HashMap<String, GithubAssetResolveState>>> =
  Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug)]
enum GithubAssetResolveState {
  Pending,
  Resolved(String),
  Failed,
}

pub fn is_github_user_attachment_url(url: &str) -> bool {
  url.starts_with("https://github.com/user-attachments/assets/")
}

fn resolve_github_asset_url_async(
  url: &str,
  resolver: &Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
) -> Option<String> {
  {
    let cache = GITHUB_ASSET_URL_CACHE.lock().unwrap();
    if let Some(state) = cache.get(url) {
      return match state {
        GithubAssetResolveState::Resolved(signed_url) => Some(signed_url.clone()),
        GithubAssetResolveState::Pending | GithubAssetResolveState::Failed => None,
      };
    }
  }

  GITHUB_ASSET_URL_CACHE
    .lock()
    .unwrap()
    .insert(url.to_string(), GithubAssetResolveState::Pending);

  let url = url.to_string();
  let resolver = resolver.clone();
  std::thread::spawn(move || {
    let state = if let Some(signed_url) = resolver(&url) {
      GithubAssetResolveState::Resolved(signed_url)
    } else {
      GithubAssetResolveState::Failed
    };
    GITHUB_ASSET_URL_CACHE.lock().unwrap().insert(url, state);
  });

  None
}

pub(crate) fn resolve_badge_image_source_async(url: &str) -> Option<BadgeImageSource> {
  {
    let cache = BADGE_IMAGE_SOURCE_CACHE.lock().unwrap();
    if let Some(state) = cache.get(url) {
      return match state {
        BadgeResolveState::Ready(source) => Some(source.clone()),
        BadgeResolveState::Pending | BadgeResolveState::Failed => None,
      };
    }
  }

  BADGE_IMAGE_SOURCE_CACHE
    .lock()
    .unwrap()
    .insert(url.to_string(), BadgeResolveState::Pending);

  let url = url.to_string();
  std::thread::spawn(move || {
    let source = fetch_badge_image_source(&url);
    let state = if let Some(source) = source {
      BadgeResolveState::Ready(source)
    } else {
      BadgeResolveState::Failed
    };
    BADGE_IMAGE_SOURCE_CACHE.lock().unwrap().insert(url, state);
  });

  None
}

pub(crate) fn load_badge_image_data(
  source: &BadgeImageSource,
  window: &mut Window,
  cx: &mut App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
  let resource = match source {
    BadgeImageSource::Remote(url) => Resource::Uri(url.clone().into()),
    BadgeImageSource::Local(path) => Resource::from(path.clone()),
  };
  window.use_asset::<ImgResourceLoader>(&resource, cx)
}

pub(crate) fn fetch_badge_image_source(url: &str) -> Option<BadgeImageSource> {
  if let Some(state) = BADGE_IMAGE_SOURCE_CACHE.lock().unwrap().get(url)
    && let BadgeResolveState::Ready(source) = state
  {
    return Some(source.clone());
  }

  let source = fetch_badge_image_source_blocking(url)?;
  BADGE_IMAGE_SOURCE_CACHE
    .lock()
    .unwrap()
    .insert(url.to_string(), BadgeResolveState::Ready(source.clone()));
  Some(source)
}

pub(crate) fn fetch_badge_image_source_blocking(url: &str) -> Option<BadgeImageSource> {
  let client = match reqwest::blocking::Client::builder()
    .timeout(Duration::from_secs(4))
    .build()
  {
    Ok(client) => client,
    Err(_) => return Some(BadgeImageSource::Remote(url.to_string())),
  };

  let response = match client.get(url).send() {
    Ok(response) => response,
    Err(_) => return Some(BadgeImageSource::Remote(url.to_string())),
  };

  let content_type = response
    .headers()
    .get(CONTENT_TYPE)
    .and_then(|value| value.to_str().ok())
    .unwrap_or("unknown")
    .to_string();

  let bytes = match response.bytes() {
    Ok(bytes) => bytes,
    Err(_) => return Some(BadgeImageSource::Remote(url.to_string())),
  };

  if (content_type.contains("svg") || bytes.starts_with(b"<svg"))
    && let Ok(svg) = String::from_utf8(bytes.to_vec())
    && should_resolve_svg_embedded_image(&svg)
    && let Some(href) = extract_svg_image_href(&svg)
    && let Some(source) = resolve_badge_href(url, &href)
  {
    return Some(source);
  }

  Some(BadgeImageSource::Remote(url.to_string()))
}

pub(crate) fn should_resolve_svg_embedded_image(svg: &str) -> bool {
  let lower = svg.to_ascii_lowercase();
  if lower.match_indices("<image").count() != 1 {
    return false;
  }

  let has_badge_like_shape_or_text = [
    "<text",
    "<rect",
    "<path",
    "<line",
    "<polyline",
    "<polygon",
    "<circle",
    "<ellipse",
  ]
  .iter()
  .any(|pattern| lower.contains(pattern));

  !has_badge_like_shape_or_text
}

pub(crate) fn extract_svg_image_href(svg: &str) -> Option<String> {
  let lower = svg.to_ascii_lowercase();
  let start = lower.find("<image")?;
  let end = lower[start..].find('>')? + start;
  let tag = &svg[start..=end];
  extract_html_attribute(tag, "xlink:href").or_else(|| extract_html_attribute(tag, "href"))
}

pub(crate) fn resolve_badge_href(base_url: &str, href: &str) -> Option<BadgeImageSource> {
  if href.starts_with("data:") {
    return data_uri_to_temp_file(href).map(BadgeImageSource::Local);
  }

  if href.starts_with("http://") || href.starts_with("https://") {
    return Some(BadgeImageSource::Remote(href.to_string()));
  }

  if href.starts_with("//") {
    return Some(BadgeImageSource::Remote(format!("https:{href}")));
  }

  if let Ok(base) = reqwest::Url::parse(base_url)
    && let Ok(joined) = base.join(href)
  {
    return Some(BadgeImageSource::Remote(joined.to_string()));
  }

  None
}

pub(crate) fn markdown_image_repo_root_url(base: &reqwest::Url) -> Option<reqwest::Url> {
  let root_segments = base
    .path_segments()
    .map(|segments| {
      segments
        .filter(|segment| !segment.is_empty())
        .take(3)
        .collect::<Vec<_>>()
    })
    .unwrap_or_default();
  if root_segments.len() < 3 {
    return None;
  }
  let mut root = base.clone();
  let root_path = format!("/{}/", root_segments.join("/"));
  root.set_path(root_path.as_str());
  Some(root)
}

pub(crate) fn resolve_markdown_image_url(url: &str, image_base_url: Option<&str>) -> String {
  let trimmed = url.trim();
  if trimmed.is_empty() {
    return String::new();
  }

  if trimmed.starts_with("data:")
    || trimmed.starts_with("http://")
    || trimmed.starts_with("https://")
  {
    return trimmed.to_string();
  }

  if trimmed.starts_with("//") {
    return format!("https:{trimmed}");
  }

  let Some(base_url) = image_base_url else {
    return trimmed.to_string();
  };
  let Ok(base) = reqwest::Url::parse(base_url) else {
    return trimmed.to_string();
  };

  if trimmed.starts_with('/')
    && let Some(repo_root) = markdown_image_repo_root_url(&base)
    && let Ok(joined) = repo_root.join(trimmed.trim_start_matches('/'))
  {
    return joined.to_string();
  }

  if let Ok(joined) = base.join(trimmed) {
    return joined.to_string();
  }

  trimmed.to_string()
}

pub(crate) fn data_uri_to_temp_file(data_uri: &str) -> Option<PathBuf> {
  let (meta, payload) = data_uri.split_once(',')?;
  if !meta.contains(";base64") {
    return None;
  }

  let extension = if meta.starts_with("data:image/png") {
    "png"
  } else if meta.starts_with("data:image/jpeg") || meta.starts_with("data:image/jpg") {
    "jpg"
  } else if meta.starts_with("data:image/webp") {
    "webp"
  } else if meta.starts_with("data:image/gif") {
    "gif"
  } else if meta.starts_with("data:image/svg+xml") {
    "svg"
  } else {
    "bin"
  };

  let mut hasher = DefaultHasher::new();
  data_uri.hash(&mut hasher);
  let path = std::env::temp_dir().join(format!("reviu-badge-{:x}.{extension}", hasher.finish()));
  if path.exists() {
    return Some(path);
  }

  let sanitized = payload.replace(char::is_whitespace, "");
  let bytes = general_purpose::STANDARD
    .decode(sanitized.as_bytes())
    .ok()?;
  fs::write(&path, bytes).ok()?;
  Some(path)
}

pub(crate) fn inline_contains_image(inline: &Inline) -> bool {
  match inline {
    Inline::Image { .. } => true,
    Inline::Link { url, content, .. } => {
      is_bare_github_user_attachment_link(url, content) || content.iter().any(inline_contains_image)
    }
    Inline::Strong(content) | Inline::Emphasis(content) | Inline::Strikethrough(content) => {
      content.iter().any(inline_contains_image)
    }
    _ => false,
  }
}

fn is_bare_github_user_attachment_link(url: &str, content: &[Inline]) -> bool {
  is_github_user_attachment_url(url)
    && content.len() == 1
    && matches!(&content[0], Inline::Text(text) if text == url)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InlineImageData {
  pub(crate) url: String,
  pub(crate) alt: String,
  pub(crate) link_url: Option<String>,
  pub(crate) width_hint: Option<String>,
  pub(crate) height_hint: Option<String>,
  pub(crate) dark_url: Option<String>,
  pub(crate) light_url: Option<String>,
}

impl InlineImageData {
  pub(crate) fn themed_url(&self, is_dark_mode: bool) -> String {
    select_markdown_image_url_for_theme(
      &self.url,
      self.dark_url.as_deref(),
      self.light_url.as_deref(),
      is_dark_mode,
    )
  }

  pub(crate) fn with_parent_link(mut self, link_url: &str) -> Self {
    if self.link_url.is_none() {
      self.link_url = Some(link_url.to_string());
    }
    self
  }

  /// Returns true if this image has explicit dimensions or is a GitHub
  /// user-attachment, meaning it should be rendered as a block (on its own
  /// line) rather than inline next to text.
  pub(crate) fn is_block_sized(&self) -> bool {
    self.width_hint.is_some()
      || self.height_hint.is_some()
      || is_github_user_attachment_url(&self.url)
  }
}

pub(crate) fn inline_image_data(inline: &Inline) -> Option<InlineImageData> {
  match inline {
    Inline::Image {
      url,
      alt,
      width,
      height,
      dark_url,
      light_url,
      ..
    } => Some(InlineImageData {
      url: url.clone(),
      alt: alt.clone(),
      link_url: None,
      width_hint: width.clone(),
      height_hint: height.clone(),
      dark_url: dark_url.clone(),
      light_url: light_url.clone(),
    }),
    Inline::Link {
      url: link_url,
      content,
      ..
    } => {
      if is_bare_github_user_attachment_link(link_url, content) {
        return Some(InlineImageData {
          url: link_url.clone(),
          alt: "Attachment".to_string(),
          link_url: Some(link_url.clone()),
          width_hint: None,
          height_hint: None,
          dark_url: None,
          light_url: None,
        });
      }
      for child in content {
        if let Some(image) = inline_image_data(child) {
          return Some(image.with_parent_link(link_url));
        }
      }
      None
    }
    Inline::Strong(content) | Inline::Emphasis(content) | Inline::Strikethrough(content) => {
      content.iter().find_map(inline_image_data)
    }
    _ => None,
  }
}

pub(crate) fn split_inlines_by_hard_breaks(inlines: &[Inline]) -> Vec<Vec<Inline>> {
  let mut rows = Vec::new();
  let mut current_row = Vec::new();

  for inline in inlines {
    if matches!(inline, Inline::HardBreak) {
      rows.push(current_row);
      current_row = Vec::new();
      continue;
    }
    current_row.push(inline.clone());
  }

  rows.push(current_row);
  rows
}

pub(crate) fn single_inline_image_data(inlines: &[Inline]) -> Option<InlineImageData> {
  if inlines.len() != 1 {
    return None;
  }
  inline_image_data(&inlines[0])
}

pub(crate) fn render_table_cell_inlines(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  if !inlines.iter().any(inline_contains_image) {
    return render_inline_text(inlines, options, cx, ctx);
  }

  let mut row = h_flex().items_center().gap_1();
  let mut text_chunk: Vec<Inline> = Vec::new();
  let is_dark_mode = cx.theme().mode.is_dark();

  for inline in inlines {
    if let Some(image_data) = inline_image_data(inline) {
      if !text_chunk.is_empty() {
        row = row.child(render_inline_text(&text_chunk, options, cx, ctx));
        text_chunk.clear();
      }

      let badge_label = if image_data.alt.is_empty() {
        "image".to_string()
      } else {
        image_data.alt.clone()
      };
      let themed_url = image_data.themed_url(is_dark_mode);
      let badge_url = resolve_markdown_image_url(
        &themed_url,
        options.image_base_url.as_ref().map(SharedString::as_ref),
      );
      row = row.child(
        img(move |window: &mut Window, cx: &mut App| {
          if let Some(source) = resolve_badge_image_source_async(&badge_url) {
            return load_badge_image_data(&source, window, cx);
          }

          window.request_animation_frame();
          None
        })
        .h(px(18.0))
        .object_fit(ObjectFit::Contain)
        .with_loading({
          let badge_label = badge_label.clone();
          move || render_badge_placeholder(&badge_label)
        })
        .with_fallback(move || render_badge_placeholder(&badge_label)),
      );
    } else {
      text_chunk.push(inline.clone());
    }
  }

  if !text_chunk.is_empty() {
    row = row.child(render_inline_text(&text_chunk, options, cx, ctx));
  }

  row.into_any_element()
}

pub(crate) fn render_badge_placeholder(label: &str) -> AnyElement {
  let text = label.trim();
  let text = if text.is_empty() {
    "badge".to_string()
  } else {
    text.to_string()
  };
  div()
    .h(px(18.0))
    .px_2()
    .rounded_sm()
    .bg(Hsla {
      h: 220.0 / 360.0,
      s: 0.18,
      l: 0.58,
      a: 1.0,
    })
    .text_xs()
    .text_color(Hsla {
      h: 0.0,
      s: 0.0,
      l: 1.0,
      a: 1.0,
    })
    .child(text)
    .into_any_element()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MarkdownImageDimension {
  Pixels(f32),
  Fraction(f32),
}

#[derive(Clone)]
pub(crate) struct MarkdownImageRenderContext<'a> {
  pub(crate) on_link: Option<Arc<LinkHandlerFn>>,
  pub(crate) interactive: bool,
  pub(crate) is_dark_mode: bool,
  pub(crate) image_base_url: Option<&'a str>,
  pub(crate) asset_url_resolver: Option<&'a Arc<dyn Fn(&str) -> Option<String> + Send + Sync>>,
}

impl MarkdownImageRenderContext<'_> {
  pub(crate) fn resolve_url(&self, image_data: &InlineImageData) -> String {
    let url = resolve_markdown_image_url(
      &image_data.themed_url(self.is_dark_mode),
      self.image_base_url,
    );
    if is_github_user_attachment_url(&url) {
      if let Some(resolver) = self.asset_url_resolver {
        if let Some(resolved) = resolve_github_asset_url_async(&url, resolver) {
          return resolved;
        }
      }
    }
    url
  }
}

pub(crate) fn parse_markdown_image_dimension(
  dimension_hint: Option<&str>,
) -> Option<MarkdownImageDimension> {
  let dimension_hint = dimension_hint
    .map(str::trim)
    .filter(|hint| !hint.is_empty())?;
  let lower = dimension_hint.to_ascii_lowercase();

  if let Some(percent) = lower.strip_suffix('%')
    && let Ok(value) = percent.trim().parse::<f32>()
    && value.is_finite()
    && value > 0.0
  {
    return Some(MarkdownImageDimension::Fraction((value / 100.0).min(1.0)));
  }

  let px_value = lower.strip_suffix("px").unwrap_or(lower.as_str()).trim();
  if let Ok(value) = px_value.parse::<f32>()
    && value.is_finite()
    && value > 0.0
  {
    return Some(MarkdownImageDimension::Pixels(value));
  }

  None
}

pub(crate) fn select_markdown_image_url_for_theme(
  url: &str,
  dark_url: Option<&str>,
  light_url: Option<&str>,
  is_dark_mode: bool,
) -> String {
  let fallback = {
    let trimmed = url.trim();
    if trimmed.is_empty() { url } else { trimmed }
  };
  let themed = (if is_dark_mode { dark_url } else { light_url })
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .unwrap_or(fallback);
  themed.to_string()
}

pub(crate) fn render_image_node(
  url: &str,
  alt: &str,
  width_hint: Option<&str>,
  height_hint: Option<&str>,
) -> impl IntoElement {
  let label = if alt.trim().is_empty() {
    "image".to_string()
  } else {
    alt.trim().to_string()
  };
  let image_url = url.to_string();
  let mut image = img(move |window: &mut Window, cx: &mut App| {
    if let Some(source) = resolve_badge_image_source_async(&image_url) {
      return load_badge_image_data(&source, window, cx);
    }

    window.request_animation_frame();
    None
  })
  .max_h(px(MARKDOWN_INLINE_IMAGE_MAX_HEIGHT_PX))
  .object_fit(ObjectFit::Contain);
  if let Some(width) = parse_markdown_image_dimension(width_hint) {
    image = match width {
      MarkdownImageDimension::Pixels(value) => image.w(px(value)),
      MarkdownImageDimension::Fraction(value) => image.w(relative(value)),
    };
  }
  if let Some(height) = parse_markdown_image_dimension(height_hint) {
    image = match height {
      MarkdownImageDimension::Pixels(value) => image.h(px(value)),
      MarkdownImageDimension::Fraction(value) => image.h(relative(value)),
    };
  }

  image
    .with_loading({
      let label = label.clone();
      move || render_badge_placeholder(&label)
    })
    .with_fallback(move || render_badge_placeholder(&label))
}

pub(crate) fn render_block_image_node(
  url: &str,
  alt: &str,
  width_hint: Option<&str>,
  height_hint: Option<&str>,
) -> impl IntoElement {
  let label = if alt.trim().is_empty() {
    "image".to_string()
  } else {
    alt.trim().to_string()
  };
  let image_url = url.to_string();
  let mut image = img(move |window: &mut Window, cx: &mut App| {
    if let Some(source) = resolve_badge_image_source_async(&image_url) {
      return load_badge_image_data(&source, window, cx);
    }

    window.request_animation_frame();
    None
  })
  .max_w_full()
  .h_auto();
  if let Some(width) = parse_markdown_image_dimension(width_hint) {
    image = match width {
      MarkdownImageDimension::Pixels(value) => image.w(px(value)),
      MarkdownImageDimension::Fraction(value) => image.w(relative(value)),
    };
  }
  if let Some(height) = parse_markdown_image_dimension(height_hint) {
    image = match height {
      MarkdownImageDimension::Pixels(value) => image.h(px(value)),
      MarkdownImageDimension::Fraction(value) => image.h(relative(value)),
    };
  }

  image
    .with_loading({
      let label = label.clone();
      move || render_badge_placeholder(&label)
    })
    .with_fallback(move || render_badge_placeholder(&label))
}

pub(crate) fn attach_image_link_handler(
  image: AnyElement,
  url: &str,
  link_url: Option<&str>,
  on_link: Option<Arc<LinkHandlerFn>>,
  interactive: bool,
) -> AnyElement {
  let mut hasher = DefaultHasher::new();
  url.hash(&mut hasher);
  link_url.hash(&mut hasher);
  let image_id: SharedString = format!("markdown-inline-image-{:x}", hasher.finish()).into();

  let mut container = div().id(image_id).child(image);
  if interactive && let Some(link_url) = link_url {
    let link_url = link_url.to_string();
    let on_link = on_link.clone();
    container = container.cursor_pointer().on_click(move |_, window, cx| {
      let handled = on_link
        .as_ref()
        .is_some_and(|handler| matches!(handler(&link_url, window, cx), LinkAction::Handled));
      if !handled {
        cx.open_url(&link_url);
      }
    });
  }

  container.into_any_element()
}

pub(crate) fn render_inline_image(
  image_data: &InlineImageData,
  context: &MarkdownImageRenderContext<'_>,
) -> AnyElement {
  let resolved_url = context.resolve_url(image_data);
  let image = render_image_node(
    &resolved_url,
    &image_data.alt,
    image_data.width_hint.as_deref(),
    image_data.height_hint.as_deref(),
  )
  .into_any_element();
  attach_image_link_handler(
    image,
    &resolved_url,
    image_data.link_url.as_deref(),
    context.on_link.clone(),
    context.interactive,
  )
}

pub(crate) fn render_block_image(
  image_data: &InlineImageData,
  context: &MarkdownImageRenderContext<'_>,
) -> AnyElement {
  let resolved_url = context.resolve_url(image_data);
  let mut hasher = DefaultHasher::new();
  resolved_url.hash(&mut hasher);
  image_data.link_url.hash(&mut hasher);
  let image_scroll_id: SharedString =
    format!("markdown-inline-image-scroll-{:x}", hasher.finish()).into();

  div()
    .id(image_scroll_id)
    .w_full()
    .child(attach_image_link_handler(
      render_block_image_node(
        &resolved_url,
        &image_data.alt,
        image_data.width_hint.as_deref(),
        image_data.height_hint.as_deref(),
      )
      .into_any_element(),
      &resolved_url,
      image_data.link_url.as_deref(),
      context.on_link.clone(),
      context.interactive,
    ))
    .into_any_element()
}
