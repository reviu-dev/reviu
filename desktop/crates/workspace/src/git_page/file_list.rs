//! Sidebar file list: path labels, rows and the list delegate.

use super::*;

pub(super) fn format_git_file_name_label(path: &Path) -> SharedString {
  path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or("Untitled")
    .replace(['\n', '\r'], "")
    .into()
}

pub(super) fn format_git_path_label_parts(path: &Path) -> (SharedString, SharedString) {
  let label = path.to_string_lossy().replace(['\n', '\r'], "");
  let file_name = path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or(label.as_str())
    .replace(['\n', '\r'], "");
  let prefix = label
    .strip_suffix(file_name.as_str())
    .unwrap_or("")
    .to_string();
  (prefix.into(), file_name.into())
}

pub(super) fn render_git_path_label(
  theme: &gpui_component::Theme,
  path: &Path,
  muted_file: bool,
  line_through: bool,
) -> AnyElement {
  let (prefix_label, file_label) = format_git_path_label_parts(path);

  h_flex()
    .min_w_0()
    .overflow_hidden()
    .gap_0()
    .text_sm()
    .when(line_through, |this| this.line_through())
    .child(
      div()
        .whitespace_nowrap()
        .overflow_hidden()
        .text_ellipsis_start()
        .text_color(theme.muted_foreground)
        .child(prefix_label),
    )
    .child(
      div()
        .flex_shrink_0()
        .when(muted_file, |this| this.text_color(theme.muted_foreground))
        .child(file_label),
    )
    .into_any_element()
}

pub(super) fn render_git_status_path_label(
  theme: &gpui_component::Theme,
  status: RepoStatusKind,
  path: &Path,
  old_path: Option<&Path>,
) -> AnyElement {
  if status == RepoStatusKind::Renamed
    && let Some(old_path) = old_path
  {
    return h_flex()
      .min_w_0()
      .flex_1()
      .items_center()
      .gap_1()
      .child(render_git_path_label(theme, old_path, true, true))
      .child(
        Icon::new(IconName::ArrowRight)
          .size_3()
          .text_color(theme.muted_foreground),
      )
      .child(render_git_path_label(theme, path, false, false))
      .into_any_element();
  }

  render_git_path_label(theme, path, false, status == RepoStatusKind::Deleted)
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

#[derive(Clone, Debug)]
pub(super) struct GitFileRow {
  pub(super) entry: RepoStatusEntry,
}

impl GitFileRow {
  pub(super) fn new(entry: RepoStatusEntry) -> Self {
    Self { entry }
  }
}

pub(super) struct GitFileSection {
  label: SharedString,
  is_staged: bool,
  rows: Vec<Rc<GitFileRow>>,
}

pub(super) struct GitFileListDelegate {
  rows: Vec<Rc<GitFileRow>>,
  sections: Vec<GitFileSection>,
  split_sections: bool,
  selected_index: Option<IndexPath>,
  opened_path: Option<PathBuf>,
  git_page: WeakEntity<GitPage>,
}

impl GitFileListDelegate {
  pub(super) fn new(git_page: WeakEntity<GitPage>) -> Self {
    Self {
      rows: Vec::new(),
      sections: Vec::new(),
      split_sections: false,
      selected_index: None,
      opened_path: None,
      git_page,
    }
  }

  pub(super) fn set_rows(&mut self, entries: Vec<RepoStatusEntry>, split_sections: bool) {
    self.rows = entries
      .into_iter()
      .map(|entry| Rc::new(GitFileRow::new(entry)))
      .collect();
    self.split_sections = split_sections;
    self.rebuild_sections();
  }

  fn rebuild_sections(&mut self) {
    if !self.split_sections {
      self.sections = vec![GitFileSection {
        label: "".into(),
        is_staged: false,
        rows: self.rows.clone(),
      }];
      return;
    }

    let mut staged_rows = Vec::new();
    let mut unstaged_rows = Vec::new();
    for row in &self.rows {
      match row.entry.stage {
        RepoStage::Staged => staged_rows.push(row.clone()),
        RepoStage::Unstaged => unstaged_rows.push(row.clone()),
        RepoStage::PartiallyStaged => {
          staged_rows.push(row.clone());
          unstaged_rows.push(row.clone());
        }
      }
    }

    let mut sections = Vec::new();
    if !staged_rows.is_empty() {
      sections.push(GitFileSection {
        label: format!("Staged Changes ({})", staged_rows.len()).into(),
        is_staged: true,
        rows: staged_rows,
      });
    }
    if !unstaged_rows.is_empty() {
      sections.push(GitFileSection {
        label: format!("Changes ({})", unstaged_rows.len()).into(),
        is_staged: false,
        rows: unstaged_rows,
      });
    }
    self.sections = sections;
  }

  pub(super) fn row_at(&self, ix: IndexPath) -> Option<Rc<GitFileRow>> {
    self
      .sections
      .get(ix.section)
      .and_then(|s| s.rows.get(ix.row).cloned())
  }

  pub(super) fn find_index_for_path(&self, path: &Path) -> Option<IndexPath> {
    for (section_ix, section) in self.sections.iter().enumerate() {
      for (row_ix, row) in section.rows.iter().enumerate() {
        if row.entry.path == path {
          return Some(IndexPath {
            section: section_ix,
            row: row_ix,
            column: 0,
          });
        }
      }
    }
    None
  }

  pub(super) fn set_opened_path(&mut self, path: Option<PathBuf>) {
    self.opened_path = path;
  }
}

pub(super) fn file_list_base_item(
  ix: IndexPath,
  selected_index: Option<IndexPath>,
  theme: &gpui_component::Theme,
) -> ListItem {
  selectable_list_item(
    ix,
    selected_index
      .map(|selected| selected.eq_row(ix))
      .unwrap_or(false),
    SelectableRowStyle::Inset,
    theme,
  )
}

impl ListDelegate for GitFileListDelegate {
  type Item = ListItem;

  fn sections_count(&self, _cx: &App) -> usize {
    self.sections.len()
  }

  fn items_count(&self, section: usize, _cx: &App) -> usize {
    self.sections.get(section).map_or(0, |s| s.rows.len())
  }

  fn render_section_header(
    &mut self,
    section: usize,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<impl IntoElement> {
    if !self.split_sections {
      return None;
    }
    let section = self.sections.get(section)?;
    let theme = cx.theme();
    let (icon, icon_color) = if section.is_staged {
      (IconName::CircleCheck, theme.status_green())
    } else {
      (IconName::Minus, theme.muted_foreground)
    };
    Some(
      h_flex()
        .items_center()
        .py_1()
        .px_2()
        .gap_2()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(Icon::new(icon).size_3().text_color(icon_color))
        .child(div().min_w_0().flex_1().child(section.label.clone())),
    )
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = cx.theme().clone();
    let mut base_item = file_list_base_item(ix, self.selected_index, &theme);
    let row = self.row_at(ix)?;
    let is_opened = self
      .opened_path
      .as_ref()
      .map(|path| path == &row.entry.path)
      .unwrap_or(false);

    if is_opened {
      base_item = base_item.bg(theme.sidebar_accent.opacity(0.35));
    }

    let status_kind = row.entry.status;
    let status_letter = status_kind.short_code();
    let status_color = GitPage::status_color(status_kind, &theme);
    let status_tooltip = GitPage::status_tooltip(status_kind);
    let (stage_icon, stage_color, stage_tooltip) = GitPage::stage_style(row.entry.stage, &theme);
    let file_icon = file_icon_path_for_path_with_theme(&row.entry.path, &theme)
      .map(|path| {
        img(path)
          .size(px(FILE_ICON_SIZE_PX))
          .min_size(px(FILE_ICON_SIZE_PX))
          .into_any_element()
      })
      .unwrap_or_else(|| {
        Icon::new(IconName::File)
          .size_3()
          .text_color(theme.sidebar_foreground)
          .into_any_element()
      });

    let stage_icon = Icon::new(stage_icon).size_3().text_color(stage_color);
    let stage_element: AnyElement = if let Some(tooltip) = stage_tooltip {
      let tooltip_id = format!("git-stage-icon-{}", ix.row);
      div()
        .id(tooltip_id)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(stage_icon)
        .into_any_element()
    } else {
      div().child(stage_icon).into_any_element()
    };

    let status_element = div()
      .id(format!("git-status-letter-{}", ix.row))
      .w(px(15.))
      .min_w(px(15.))
      .text_xs()
      .text_color(status_color)
      .tooltip(move |window, cx| Tooltip::new(status_tooltip.clone()).build(window, cx))
      .child(status_letter);

    let file_label = render_git_status_path_label(
      &theme,
      row.entry.status,
      &row.entry.path,
      row.entry.old_path.as_deref(),
    );

    let rel_path = row.entry.path.clone();
    let old_path = row.entry.old_path.clone();
    let stage = row.entry.stage;
    let git_page = self.git_page.clone();

    // In split mode, the action is determined by the section the file is in,
    // not by the file's stage status (PartiallyStaged appears in both sections).
    let is_staged_section = self.split_sections
      && self
        .sections
        .get(ix.section)
        .map(|s| s.is_staged)
        .unwrap_or(false);
    let toggle_stage_action =
      GitPage::sidebar_toggle_stage_action(stage, self.split_sections, is_staged_section);
    let (toggle_stage_icon, toggle_stage_tooltip) = match toggle_stage_action {
      FileStageButtonAction::Stage => (IconName::Plus, "Stage file"),
      FileStageButtonAction::Unstage => (IconName::Minus, "Unstage file"),
    };
    let can_restore = GitPage::can_restore_file_stage(stage);

    Some(
      base_item.px_2().py_1().child(
        h_flex()
          .group("file-row")
          .size_full()
          .items_center()
          .relative()
          .gap_2()
          .child(
            h_flex()
              .items_center()
              .min_w_0()
              .gap_2()
              .child(status_element)
              .child(stage_element)
              .child(file_icon)
              .child(file_label),
          )
          .child(
            div()
              .absolute()
              .right_0()
              .opacity(0.0)
              .group_hover("file-row", |this| this.opacity(1.0))
              .bg(theme.sidebar)
              .rounded(theme.radius)
              .child(
                ButtonGroup::new(format!("file-actions-{}", ix.row))
                  .outline()
                  .child(
                    Button::new(format!("stage-{}", ix.row))
                      .icon(toggle_stage_icon)
                      .xsmall()
                      .tab_stop(false)
                      .tooltip(toggle_stage_tooltip)
                      .on_click({
                        let rel_path = rel_path.clone();
                        let git_page = git_page.clone();
                        move |_event, window, cx| {
                          let _ = git_page.update(cx, |page, cx| match toggle_stage_action {
                            FileStageButtonAction::Unstage => {
                              page.unstage_file_action(rel_path.clone(), cx);
                            }
                            FileStageButtonAction::Stage => {
                              page.stage_file_click_action(
                                window,
                                rel_path.clone(),
                                status_kind,
                                cx,
                              );
                            }
                          });
                        }
                      }),
                  )
                  .when(can_restore, |this| {
                    this.child(
                      Button::new(format!("restore-{}", ix.row))
                        .icon(IconName::Undo)
                        .xsmall()
                        .tab_stop(false)
                        .tooltip("Discard changes")
                        .on_click({
                          let rel_path = rel_path.clone();
                          let old_path = old_path.clone();
                          let git_page = git_page.clone();
                          move |_event, window, cx| {
                            let _ = git_page.update(cx, |page, cx| {
                              page.restore_file_click_action(
                                window,
                                rel_path.clone(),
                                old_path.clone(),
                                status_kind,
                                cx,
                              );
                            });
                          }
                        }),
                    )
                  }),
              ),
          ),
      ),
    )
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    div()
      .flex()
      .flex_col()
      .size_full()
      .items_center()
      .justify_center()
      .text_sm()
      .text_color(cx.theme().muted_foreground)
      .child("No changes")
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    self.selected_index = ix;
    cx.notify();
  }
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

  #[test]
  fn format_git_path_label_parts_splits_prefix_and_name() {
    let path = Path::new("desktop/crates/workspace/src/git_page.rs");
    let (prefix, name) = format_git_path_label_parts(path);

    assert_eq!(prefix.as_ref(), "desktop/crates/workspace/src/");
    assert_eq!(name.as_ref(), "git_page.rs");
  }

  #[test]
  fn git_file_row_keeps_entry_paths() {
    let row = GitFileRow::new(RepoStatusEntry {
      path: PathBuf::from("src/features/new_file.rs"),
      old_path: Some(PathBuf::from("src/features/old_file.rs")),
      status: RepoStatusKind::Renamed,
      stage: RepoStage::Unstaged,
    });

    assert_eq!(row.entry.path, PathBuf::from("src/features/new_file.rs"));
    assert_eq!(
      row.entry.old_path.as_deref(),
      Some(Path::new("src/features/old_file.rs"))
    );
  }

  #[test]
  fn should_refresh_file_list_only_in_changes_mode() {
    assert!(GitPage::should_refresh_file_list(GitSidebarMode::Changes));
    assert!(!GitPage::should_refresh_file_list(GitSidebarMode::History));
  }
}
