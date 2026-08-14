//! Shared file-viewing pieces: header title (icon + name + path) and binary previews.

use std::path::Path;
use std::sync::Arc;

use gpui::{AnyElement, Image, ObjectFit, SharedString, div, img, prelude::*, px};
use gpui_component::{ActiveTheme as _, Icon, IconName, h_flex, v_flex};

use crate::file_preview::{
  file_preview_kind, raster_image_from_bytes, should_show_unsupported_binary_placeholder,
  FilePreviewKind,
};
use ui::{FILE_ICON_SIZE_PX, StatusThemeExt as _, file_icon_path_for_path_with_theme};

/// Kept stable: UI tests locate the preview pane through this selector.
pub(crate) const BINARY_PREVIEW_DEBUG_SELECTOR: &str = "git-binary-preview-render-pane";

#[derive(Clone)]
pub(crate) enum BinaryPreview {
  RasterImage(Arc<Image>),
  UnsupportedBinary,
}

/// Decide what to show for a file that could not be loaded as text.
pub(crate) fn build_binary_preview(
  path: &Path,
  binary_bytes: Option<Vec<u8>>,
) -> Option<BinaryPreview> {
  if let Some(bytes) = binary_bytes {
    if let Some(image) = raster_image_from_bytes(path, bytes.clone()) {
      return Some(BinaryPreview::RasterImage(image));
    }
    if should_show_unsupported_binary_placeholder(path, Some(bytes.as_slice())) {
      return Some(BinaryPreview::UnsupportedBinary);
    }
    return None;
  }

  matches!(
    file_preview_kind(path),
    Some(FilePreviewKind::UnsupportedBinary)
  )
  .then_some(BinaryPreview::UnsupportedBinary)
}

pub(crate) fn file_name_label(path: &Path) -> SharedString {
  path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or("Untitled")
    .replace(['\n', '\r'], "")
    .into()
}

pub(crate) fn file_dir_label(path: &Path) -> String {
  path
    .parent()
    .and_then(|parent| parent.to_str())
    .unwrap_or("")
    .to_string()
}

fn preview_status_message(message: impl Into<SharedString>, color: gpui::Hsla) -> AnyElement {
  div()
    .w(px(280.0))
    .max_w_full()
    .px_3()
    .text_sm()
    .text_center()
    .whitespace_normal()
    .text_color(color)
    .child(message.into())
    .into_any_element()
}

pub(crate) fn render_raster_image(image: Arc<Image>, cx: &gpui::App) -> AnyElement {
  let theme = cx.theme();
  let loading_color = theme.muted_foreground;
  let error_color = theme.status_red();

  div()
    .flex_1()
    .min_h_0()
    .min_w(px(0.0))
    .overflow_hidden()
    .bg(theme.background)
    .occlude()
    .debug_selector(|| BINARY_PREVIEW_DEBUG_SELECTOR.to_string())
    .child(
      div().relative().size_full().child(
        div()
          .absolute()
          .top_0()
          .left_0()
          .right_0()
          .bottom_0()
          .p_4()
          .flex()
          .items_center()
          .justify_center()
          .child(
            img(image)
              .max_w_full()
              .max_h_full()
              .object_fit(ObjectFit::Contain)
              .with_loading(move || {
                preview_status_message("Rendering image preview...", loading_color)
              })
              .with_fallback(move || {
                preview_status_message("Unable to render image preview", error_color)
              }),
          ),
      ),
    )
    .into_any_element()
}

pub(crate) fn render_unsupported_binary(cx: &gpui::App) -> AnyElement {
  let theme = cx.theme();
  div()
    .flex_1()
    .min_h_0()
    .min_w(px(0.0))
    .bg(theme.background)
    .occlude()
    .debug_selector(|| BINARY_PREVIEW_DEBUG_SELECTOR.to_string())
    .child(
      v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(
          Icon::new(IconName::File)
            .size_6()
            .text_color(theme.muted_foreground),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Binary file preview is not available."),
        ),
    )
    .into_any_element()
}

pub(crate) fn render_binary_preview(preview: &BinaryPreview, cx: &gpui::App) -> AnyElement {
  match preview {
    BinaryPreview::RasterImage(image) => render_raster_image(image.clone(), cx),
    BinaryPreview::UnsupportedBinary => render_unsupported_binary(cx),
  }
}

/// File title used in editor/diff headers: type icon, file name, then the
/// directory path trailing on the right.
pub(crate) fn render_file_title(path: &Path, cx: &gpui::App) -> AnyElement {
  let theme = cx.theme().clone();
  let dir = file_dir_label(path);

  h_flex()
    .items_center()
    .gap_2()
    .min_w_0()
    .flex_1()
    .child(
      file_icon_path_for_path_with_theme(path, &theme)
        .map(|icon| img(icon).size(px(FILE_ICON_SIZE_PX)).into_any_element())
        .unwrap_or_else(|| {
          Icon::new(IconName::File)
            .size_3()
            .text_color(theme.foreground)
            .into_any_element()
        }),
    )
    .child(
      div()
        .flex_shrink_0()
        .text_sm()
        .text_color(theme.foreground)
        .child(file_name_label(path)),
    )
    .when(!dir.is_empty(), |this| {
      this.child(
        div()
          .min_w_0()
          .flex_1()
          .overflow_hidden()
          .text_ellipsis_start()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(format!("- {dir}")),
      )
    })
    .into_any_element()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn file_name_label_uses_file_name_and_strips_newlines() {
    assert_eq!(file_name_label(Path::new("src/main.rs")).as_ref(), "main.rs");
    assert_eq!(file_name_label(Path::new("a/b\nc.rs")).as_ref(), "bc.rs");
    assert_eq!(file_name_label(Path::new("")).as_ref(), "Untitled");
  }

  #[test]
  fn file_dir_label_returns_parent_or_empty() {
    assert_eq!(file_dir_label(Path::new("src/api/client.rs")), "src/api");
    assert_eq!(file_dir_label(Path::new("README.md")), "");
  }

  #[test]
  fn build_binary_preview_flags_unsupported_binaries_by_extension() {
    assert!(matches!(
      build_binary_preview(Path::new("doc.pdf"), None),
      Some(BinaryPreview::UnsupportedBinary)
    ));
    assert!(build_binary_preview(Path::new("main.rs"), None).is_none());
  }
}
