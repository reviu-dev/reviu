use gpui::{AppContext as _, Context, Entity, Window};
use ui::{SearchFileEntry, SearchFileHandler, SearchFilePalette, SearchFilePaletteConfig};

pub fn open_file_search_palette<T: 'static>(
  window: &mut Window,
  cx: &mut Context<T>,
  entries: Vec<SearchFileEntry>,
  handler: SearchFileHandler,
  loading: bool,
) -> Entity<SearchFilePalette> {
  let palette = cx.new(|cx| {
    SearchFilePalette::new(
      window,
      cx,
      SearchFilePaletteConfig::new(entries, handler).loading(loading),
    )
  });
  ui::open_palette_dialog(palette.clone(), window, cx);
  palette
}
