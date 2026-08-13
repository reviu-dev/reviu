use gpui::{AppContext as _, Context, Window};
use ui::{SearchFileEntry, SearchFileHandler, SearchFilePalette, SearchFilePaletteConfig};

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
  ui::open_palette_dialog(palette, window, cx);
}
