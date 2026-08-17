use std::{path::Path, sync::Arc};

use gpui::{Image, ImageFormat};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilePreviewKind {
  Markdown,
  Svg,
  RasterImage(ImageFormat),
  UnsupportedBinary,
}

/// The right-hand pane of a preview: the rendered SVG, or the markdown.
pub(crate) const PREVIEW_CONTENT_DEBUG_SELECTOR: &str = "file-preview-content";

pub(crate) fn render_preview_pane(
  id: &'static str,
  editor: &gpui::Entity<editor::Editor>,
  svg_preview: &gpui::Entity<crate::svg_preview::SvgPreview>,
  is_svg: bool,
  window: &mut gpui::Window,
  cx: &mut gpui::App,
) -> gpui::AnyElement {
  use gpui::{InteractiveElement as _, IntoElement as _, ParentElement as _, Styled as _};
  use gpui_component::ActiveTheme as _;

  if is_svg {
    let editor = editor.clone();
    svg_preview.update(cx, |preview, cx| {
      preview.refresh_from_editor(&editor, window, cx);
    });
    return svg_preview.read(cx).render(cx);
  }

  let markdown = {
    let document = editor.read(cx).document().read(cx);
    document.slice_to_string(0..document.len())
  };

  gpui::div()
    .flex_1()
    .min_h_0()
    .min_w(gpui::px(0.0))
    .bg(cx.theme().background)
    .occlude()
    .debug_selector(|| PREVIEW_CONTENT_DEBUG_SELECTOR.to_string())
    .child(
      gpui::div().size_full().pb_4().px_4().child(
        gpui_component::text::TextView::markdown(id, markdown)
          .size_full()
          .selectable(true)
          .scrollable(true),
      ),
    )
    .into_any_element()
}

pub fn is_markdown_path(path: &Path) -> bool {
  matches!(
    path
      .extension()
      .and_then(|ext| ext.to_str())
      .map(|ext| ext.to_ascii_lowercase())
      .as_deref(),
    Some("md" | "markdown" | "mdx")
  )
}

pub fn is_svg_path(path: &Path) -> bool {
  matches!(
    path
      .extension()
      .and_then(|ext| ext.to_str())
      .map(|ext| ext.to_ascii_lowercase())
      .as_deref(),
    Some("svg")
  )
}

pub fn raster_image_format_for_path(path: &Path) -> Option<ImageFormat> {
  match path
    .extension()
    .and_then(|ext| ext.to_str())
    .map(|ext| ext.to_ascii_lowercase())
    .as_deref()
  {
    Some("png") => Some(ImageFormat::Png),
    Some("jpg" | "jpeg") => Some(ImageFormat::Jpeg),
    Some("webp") => Some(ImageFormat::Webp),
    Some("gif") => Some(ImageFormat::Gif),
    Some("bmp") => Some(ImageFormat::Bmp),
    Some("tif" | "tiff") => Some(ImageFormat::Tiff),
    Some("ico") => Some(ImageFormat::Ico),
    _ => None,
  }
}

pub fn file_preview_kind(path: &Path) -> Option<FilePreviewKind> {
  if is_markdown_path(path) {
    Some(FilePreviewKind::Markdown)
  } else if is_svg_path(path) {
    Some(FilePreviewKind::Svg)
  } else if let Some(format) = raster_image_format_for_path(path) {
    Some(FilePreviewKind::RasterImage(format))
  } else if is_known_binary_path(path) {
    Some(FilePreviewKind::UnsupportedBinary)
  } else {
    None
  }
}

pub fn is_probably_binary_bytes(bytes: &[u8]) -> bool {
  let sample_len = bytes.len().min(1024);
  let sample = &bytes[..sample_len];
  sample.contains(&0) || std::str::from_utf8(sample).is_err()
}

pub fn should_show_unsupported_binary_placeholder(path: &Path, bytes: Option<&[u8]>) -> bool {
  matches!(
    file_preview_kind(path),
    Some(FilePreviewKind::UnsupportedBinary)
  ) || bytes.is_some_and(is_probably_binary_bytes)
}

pub fn raster_image_from_bytes(path: &Path, bytes: Vec<u8>) -> Option<Arc<Image>> {
  let format = raster_image_format_for_path(path)?;
  Some(Arc::new(Image::from_bytes(format, bytes)))
}

fn is_known_binary_path(path: &Path) -> bool {
  matches!(
    path
      .extension()
      .and_then(|ext| ext.to_str())
      .map(|ext| ext.to_ascii_lowercase())
      .as_deref(),
    Some(
      "pdf"
        | "zip"
        | "tar"
        | "gz"
        | "tgz"
        | "bz2"
        | "xz"
        | "7z"
        | "rar"
        | "mp3"
        | "wav"
        | "flac"
        | "ogg"
        | "aac"
        | "m4a"
        | "mp4"
        | "mov"
        | "avi"
        | "mkv"
        | "webm"
        | "ttf"
        | "otf"
        | "woff"
        | "woff2"
        | "eot"
        | "jar"
        | "exe"
        | "dll"
        | "dylib"
        | "so"
        | "wasm"
        | "class"
        | "db"
        | "sqlite"
    )
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn markdown_path_detection_is_case_insensitive_and_extension_based() {
    assert!(is_markdown_path(Path::new("README.md")));
    assert!(is_markdown_path(Path::new("docs/GUIDE.MD")));
    assert!(is_markdown_path(Path::new("notes.markdown")));
    assert!(is_markdown_path(Path::new("post.MdX")));
    assert!(!is_markdown_path(Path::new("README")));
    assert!(!is_markdown_path(Path::new("icon.svg")));
    assert!(!is_markdown_path(Path::new("note.md.txt")));
  }

  #[test]
  fn svg_path_detection_is_case_insensitive_and_extension_based() {
    assert!(is_svg_path(Path::new("icon.svg")));
    assert!(is_svg_path(Path::new("ICON.SVG")));
    assert!(!is_svg_path(Path::new("icon.svgz")));
    assert!(!is_svg_path(Path::new("README.md")));
    assert!(!is_svg_path(Path::new("icon")));
  }

  #[test]
  fn previewable_path_detection_accepts_markdown_and_svg_only() {
    assert!(is_markdown_path(Path::new("README.md")));
    assert!(is_svg_path(Path::new("icon.svg")));
    assert!(!is_markdown_path(Path::new("script.ts")));
    assert!(!is_svg_path(Path::new("script.ts")));
  }

  #[test]
  fn raster_image_format_detection_maps_supported_formats() {
    assert_eq!(
      raster_image_format_for_path(Path::new("photo.png")),
      Some(ImageFormat::Png)
    );
    assert_eq!(
      raster_image_format_for_path(Path::new("photo.JPEG")),
      Some(ImageFormat::Jpeg)
    );
    assert_eq!(
      raster_image_format_for_path(Path::new("photo.webp")),
      Some(ImageFormat::Webp)
    );
    assert_eq!(
      raster_image_format_for_path(Path::new("photo.gif")),
      Some(ImageFormat::Gif)
    );
    assert_eq!(
      raster_image_format_for_path(Path::new("photo.bmp")),
      Some(ImageFormat::Bmp)
    );
    assert_eq!(
      raster_image_format_for_path(Path::new("photo.tiff")),
      Some(ImageFormat::Tiff)
    );
    assert_eq!(
      raster_image_format_for_path(Path::new("photo.ico")),
      Some(ImageFormat::Ico)
    );
    assert_eq!(raster_image_format_for_path(Path::new("photo.avif")), None);
  }

  #[test]
  fn file_preview_kind_distinguishes_text_image_and_binary_files() {
    assert_eq!(
      file_preview_kind(Path::new("README.md")),
      Some(FilePreviewKind::Markdown)
    );
    assert_eq!(
      file_preview_kind(Path::new("icon.svg")),
      Some(FilePreviewKind::Svg)
    );
    assert_eq!(
      file_preview_kind(Path::new("photo.png")),
      Some(FilePreviewKind::RasterImage(ImageFormat::Png))
    );
    assert_eq!(
      file_preview_kind(Path::new("slides.pdf")),
      Some(FilePreviewKind::UnsupportedBinary)
    );
    assert_eq!(file_preview_kind(Path::new("notes.txt")), None);
  }

  #[test]
  fn binary_placeholder_detection_uses_path_and_bytes() {
    assert!(should_show_unsupported_binary_placeholder(
      Path::new("slides.pdf"),
      None
    ));
    assert!(should_show_unsupported_binary_placeholder(
      Path::new("unknown.bin"),
      Some(b"\0\0\0")
    ));
    assert!(!should_show_unsupported_binary_placeholder(
      Path::new("notes.txt"),
      Some(b"hello world")
    ));
  }
}
