use gpui::{AppContext as _, Context, ParentElement, Styled, Window};
use ui::{
  SearchFileEntry, SearchFileHandler, SearchFilePalette, SearchFilePaletteConfig, WindowExt,
};

pub fn open_file_search_palette<T: 'static>(
  window: &mut Window,
  cx: &mut Context<T>,
  mut entries: Vec<SearchFileEntry>,
  handler: SearchFileHandler,
  sort_alphabetically: bool,
) {
  if entries.is_empty() {
    return;
  }

  if sort_alphabetically {
    entries.sort_by(|a, b| a.label.cmp(&b.label));
  }

  let palette =
    cx.new(|cx| SearchFilePalette::new(window, cx, SearchFilePaletteConfig::new(entries, handler)));
  let palette_for_dialog = palette.clone();

  window.open_dialog(cx, move |dialog, _, _| {
    dialog
      .p_0()
      .border_0()
      .min_h_0()
      .overlay_closable(true)
      .keyboard(true)
      .close_button(false)
      .child(palette_for_dialog.clone())
  });
}
