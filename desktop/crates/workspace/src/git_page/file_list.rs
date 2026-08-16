//! File name and path labels for the Git page headers.

use super::*;

pub(super) fn format_git_file_name_label(path: &Path) -> SharedString {
  path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or("Untitled")
    .replace(['\n', '\r'], "")
    .into()
}

pub(super) fn render_repo_status_label(
  theme: &gpui_component::Theme,
  status: Option<RepoStatusKind>,
  label: SharedString,
  old_label: Option<SharedString>,
) -> AnyElement {
  if status == Some(RepoStatusKind::Renamed)
    && let Some(old_label) = old_label
  {
    return h_flex()
      .min_w_0()
      .flex_1()
      .items_center()
      .text_sm()
      .gap_1()
      .child(
        div()
          .min_w_0()
          .overflow_hidden()
          .text_ellipsis_start()
          .text_color(theme.muted_foreground)
          .line_through()
          .child(old_label),
      )
      .child(
        Icon::new(IconName::ArrowRight)
          .size_3()
          .text_color(theme.muted_foreground),
      )
      .child(
        div()
          .min_w_0()
          .flex_1()
          .overflow_hidden()
          .text_ellipsis_start()
          .child(label),
      )
      .into_any_element();
  }

  div()
    .min_w_0()
    .flex_1()
    .overflow_hidden()
    .text_sm()
    .text_ellipsis_start()
    .when(status == Some(RepoStatusKind::Deleted), |this| {
      this.line_through()
    })
    .child(label)
    .into_any_element()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn format_git_file_name_label_extracts_file_name() {
    let path = Path::new("src/features/renamed_file.rs");

    assert_eq!(format_git_file_name_label(path).as_ref(), "renamed_file.rs");
  }

  #[test]
  fn format_git_file_name_label_strips_newlines() {
    let path = Path::new("src/renamed\n_file.rs");

    assert_eq!(format_git_file_name_label(path).as_ref(), "renamed_file.rs");
  }
}
