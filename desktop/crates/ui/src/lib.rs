mod assets;
mod command_palette;
mod confirm_dialog;
mod dropdown_select;
mod github_emoji_completion;
mod github_search_palette;
mod github_url;
mod icons;
mod markdown_composer;
mod reaction_bar;
mod scroll_routing;
mod search_file_palette;
mod selectable_row;
mod status_alert;
mod status_surface;
mod status_tag;
mod status_theme_ext;
mod theme;
mod user_menu;
mod variable_list;

pub const GLOBAL_BAR_HEIGHT: f32 = 36.0;
pub const PAGE_HEADER_HEIGHT: f32 = 45.0;
pub const DETAILS_PAGE_CONTAINER_MAX_WIDTH: f32 = 900.0;

pub use assets::AppAssets;
pub use command_palette::{
  COMMAND_PALETTE_CONTEXT, CommandPalette, CommandPaletteAction, CommandPaletteBranch,
  CommandPaletteBranchKind, CommandPaletteCommand, CommandPaletteCommandId, CommandPaletteConfig,
  CommandPaletteGithubRepoTab, CommandPaletteGroup, CommandPaletteHandler,
  CommandPaletteInitialScreen, CommandPalettePage, CommandPaletteRepository, CommandPaletteStash,
  CommandPaletteUsageRecorder, CommandPaletteUsageRecorderGlobal, CommandPaletteUsageScorer,
  CommandPaletteUsageScorerGlobal,
};
pub use confirm_dialog::ConfirmDialog;
pub use dropdown_select::{
  DropdownSelectConfig, DropdownSelectItem, DropdownSelectOption, dropdown_select,
};
pub use github_emoji_completion::GithubEmojiInput;
pub use github_search_palette::{
  GithubRepoSearchFn, GithubRepoSelectFn, GithubSearchPalette, GithubSearchPaletteConfig,
  GithubSearchRepoEntry,
};
pub use github_url::parse_github_url_action;
pub use gpui_component::Disableable;
pub use gpui_component::WindowExt;
pub use gpui_component::button::Button;
pub use gpui_component::button::ButtonVariants;
pub use gpui_component::input::{Input, InputState};
pub use gpui_component::popover::Popover;
pub use gpui_component::resizable::{ResizableState, h_resizable, resizable_panel};
pub use gpui_component::select::{
  SearchableVec, Select, SelectEvent, SelectGroup, SelectItem, SelectState,
};
pub use gpui_component::sidebar::{Sidebar, SidebarItem};
pub use gpui_component::{Anchor, IconName};
pub use gpui_component::{Collapsible, Sizable};
pub use icons::{
  FILE_ICON_SIZE_PX, FileIcon, UiIconName, file_icon_for_name, file_icon_for_path,
  file_icon_path_for_name, file_icon_path_for_name_with_theme, file_icon_path_for_path,
  file_icon_path_for_path_with_theme,
};
pub use markdown_composer::{
  MARKDOWN_COMPOSER_CHROME_HEIGHT_PX, MARKDOWN_COMPOSER_TAB_BAR_GAP_PX,
  MARKDOWN_COMPOSER_TAB_BAR_HEIGHT_PX, MarkdownComposer,
};
pub use reaction_bar::{ReactionBar, ReactionGroup, ReactionOption, ReactionToggle};
pub use scroll_routing::{
  ScrollAxes, ScrollDispatcher, ScrollableNode, restrict_scroll_to_wheel_axis, scroll_dispatcher,
  scrollable_node,
};
pub use search_file_palette::{
  SearchFileEntry, SearchFileHandler, SearchFilePalette, SearchFilePaletteConfig,
};
pub use selectable_row::{SelectableRowStyle, selectable_list_item};
pub use status_alert::StatusAlert;
pub use status_tag::StatusTag;
pub use status_theme_ext::StatusThemeExt;
pub use theme::Theme;
pub use user_menu::{UserMenuConfig, UserMenuPage, UserMenuState, UserMenuUser, user_menu};
pub use variable_list::{VariableList, VariableListDelegate, VariableListEvent, VariableListState};

pub fn init(cx: &mut gpui::App) {
  variable_list::init(cx);
  load_bundled_fonts(cx);
}

fn load_bundled_fonts(cx: &mut gpui::App) {
  use gpui::AssetSource as _;

  let assets = AppAssets;
  let mut fonts = Vec::new();
  for path in ["fonts/lilex/Lilex-Regular.ttf"] {
    match assets.load(path) {
      Ok(Some(data)) => {
        eprintln!("ui: loaded bundled font {path} ({} bytes)", data.len());
        #[cfg(target_os = "macos")]
        register_with_core_text(&data);
        fonts.push(data);
      }
      Ok(None) => eprintln!("ui: bundled font missing: {path}"),
      Err(err) => eprintln!("ui: failed to load bundled font {path}: {err}"),
    }
  }
  if fonts.is_empty() {
    return;
  }
  if let Err(err) = cx.text_system().add_fonts(fonts) {
    eprintln!("ui: text_system.add_fonts failed: {err}");
  }
}

#[cfg(target_os = "macos")]
fn register_with_core_text(data: &[u8]) {
  use core_graphics::data_provider::CGDataProvider;
  use core_graphics::font::CGFont;
  use foreign_types_shared::ForeignType;

  let provider = CGDataProvider::from_buffer(std::sync::Arc::new(data.to_vec()));
  let Ok(cg_font) = CGFont::from_data_provider(provider) else {
    eprintln!("ui: CGFont::from_data_provider failed");
    return;
  };

  unsafe {
    let mut error: core_foundation::error::CFErrorRef = std::ptr::null_mut();
    let ok = CTFontManagerRegisterGraphicsFont(cg_font.as_ptr(), &mut error);
    if !ok {
      eprintln!("ui: CTFontManagerRegisterGraphicsFont failed");
    }
  }
}

#[cfg(target_os = "macos")]
#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
  fn CTFontManagerRegisterGraphicsFont(
    font: *mut core_graphics::sys::CGFont,
    error: *mut core_foundation::error::CFErrorRef,
  ) -> bool;
}
