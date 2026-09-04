mod assets;
mod command_palette;
mod confirm_dialog;
mod dropdown_select;
mod github_emoji_completion;
mod github_url;
mod icons;
mod markdown_composer;
mod palette;
mod scroll_routing;
mod search_file_palette;
mod selectable_row;
mod status_theme_ext;
mod theme;
mod user_menu;
mod variable_list;

pub const GLOBAL_BAR_HEIGHT: f32 = 36.0;
pub const PAGE_HEADER_HEIGHT: f32 = 45.0;
pub const DETAILS_PAGE_CONTAINER_MAX_WIDTH: f32 = 900.0;

pub use assets::{AppAssets, REVIU_WORDMARK_WIDTH_PX, reviu_logo_path, set_runtime_asset_dir};
pub use command_palette::{
  COMMAND_PALETTE_CONTEXT, CommandPalette, CommandPaletteAction, CommandPaletteBranch,
  CommandPaletteBranchKind, CommandPaletteCommand, CommandPaletteCommandId, CommandPaletteConfig,
  CommandPaletteGithubRepoTab, CommandPaletteGroup, CommandPaletteHandler,
  CommandPaletteInitialScreen, CommandPalettePage, CommandPaletteProject, CommandPaletteStash,
  CommandPaletteUsageRecorder, CommandPaletteUsageRecorderGlobal, CommandPaletteUsageScorer,
  CommandPaletteUsageScorerGlobal, GlobalCommandsContext,
};
pub use confirm_dialog::ConfirmDialog;
pub use dropdown_select::{
  DropdownSelectConfig, DropdownSelectItem, DropdownSelectOption, dropdown_select,
};
pub use github_emoji_completion::GithubEmojiInput;
pub use github_url::parse_github_url_action;
pub use gpui::Anchor;
pub use gpui_component::Disableable;
pub use gpui_component::IconName;
pub use gpui_component::WindowExt;
pub use gpui_component::button::Button;
pub use gpui_component::button::ButtonVariants;
pub use gpui_component::input::{Input, InputState, Textarea, TextareaState};
pub use gpui_component::popover::Popover;
pub use gpui_component::resizable::{ResizableState, h_resizable, resizable_panel};
pub use gpui_component::select::{
  SearchableVec, Select, SelectEvent, SelectGroup, SelectItem, SelectState,
};
pub use gpui_component::sidebar::{Sidebar, SidebarItem};
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
pub use palette::open_palette_dialog;
pub use scroll_routing::{
  ScrollAxes, ScrollDispatcher, ScrollableNode, restrict_scroll_to_wheel_axis, scroll_dispatcher,
  scrollable_node,
};
pub use search_file_palette::{
  SearchFileEntry, SearchFileGroup, SearchFileHandler, SearchFileOpenRequest, SearchFilePalette,
  SearchFilePaletteConfig,
};
pub use selectable_row::{SelectableRowStyle, selectable_list_item};
pub use status_theme_ext::StatusThemeExt;
pub use theme::Theme;
pub use user_menu::{UserMenuConfig, UserMenuPage, UserMenuState, UserMenuUser, user_menu};
pub use variable_list::{VariableList, VariableListDelegate, VariableListEvent, VariableListState};

pub fn init(cx: &mut gpui::App) {
  variable_list::init(cx);
  load_bundled_fonts(cx);
}

/// Fonts shipped inside the app binary and registered at startup.
///
/// Lilex is the monospace face used for code and diffs. Inter is the
/// proportional UI face: macOS resolves `.SystemUIFont` natively, but Linux
/// and Windows have no such family and fall back to a monospace face, so we
/// ship Inter and point the theme at it (see reviu's `main.rs`).
const BUNDLED_FONTS: &[&str] = &[
  "fonts/lilex/Lilex-Regular.ttf",
  "fonts/inter/Inter-Regular.otf",
  "fonts/inter/Inter-Italic.otf",
  "fonts/inter/Inter-Medium.otf",
  "fonts/inter/Inter-MediumItalic.otf",
  "fonts/inter/Inter-SemiBold.otf",
  "fonts/inter/Inter-SemiBoldItalic.otf",
  "fonts/inter/Inter-Bold.otf",
  "fonts/inter/Inter-BoldItalic.otf",
];

fn load_bundled_fonts(cx: &mut gpui::App) {
  use gpui::AssetSource as _;

  let assets = AppAssets;
  let mut fonts = Vec::new();
  for &path in BUNDLED_FONTS {
    match assets.load(path) {
      Ok(Some(data)) => {
        log::info!("ui: loaded bundled font {path} ({} bytes)", data.len());
        #[cfg(target_os = "macos")]
        register_with_core_text(&data);
        fonts.push(data);
      }
      Ok(None) => log::warn!("ui: bundled font missing: {path}"),
      Err(err) => log::warn!("ui: failed to load bundled font {path}: {err}"),
    }
  }
  if fonts.is_empty() {
    return;
  }
  if let Err(err) = cx.text_system().add_fonts(fonts) {
    log::warn!("ui: text_system.add_fonts failed: {err}");
  }
}

#[cfg(target_os = "macos")]
fn register_with_core_text(data: &[u8]) {
  use core_graphics::data_provider::CGDataProvider;
  use core_graphics::font::CGFont;
  use foreign_types_shared::ForeignType;

  let provider = CGDataProvider::from_buffer(std::sync::Arc::new(data.to_vec()));
  let Ok(cg_font) = CGFont::from_data_provider(provider) else {
    log::warn!("ui: CGFont::from_data_provider failed");
    return;
  };

  unsafe {
    let mut error: core_foundation::error::CFErrorRef = std::ptr::null_mut();
    let ok = CTFontManagerRegisterGraphicsFont(cg_font.as_ptr(), &mut error);
    if !ok {
      log::warn!("ui: CTFontManagerRegisterGraphicsFont failed");
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

#[cfg(test)]
mod tests {
  use gpui::AssetSource as _;

  use super::{AppAssets, BUNDLED_FONTS};

  /// Every font we point the theme at must actually be embedded in the
  /// binary. If one goes missing, the UI silently falls back to a monospace
  /// face on Linux/Windows (the bug this bundling fixes), so guard it here.
  #[test]
  fn bundled_fonts_are_embedded() {
    for &path in BUNDLED_FONTS {
      let data = AppAssets
        .load(path)
        .unwrap_or_else(|err| panic!("loading bundled font {path} failed: {err}"))
        .unwrap_or_else(|| panic!("bundled font {path} is not embedded in the binary"));
      assert!(!data.is_empty(), "bundled font {path} is empty");
    }
  }

  /// The theme's fonts must be represented in the bundle so the interface and
  /// code/diff views render deterministically off macOS.
  #[test]
  fn bundle_covers_ui_and_mono_fonts() {
    assert!(
      BUNDLED_FONTS.iter().any(|p| p.contains("Inter")),
      "no bundled Inter (UI) font"
    );
    assert!(
      BUNDLED_FONTS.iter().any(|p| p.contains("Lilex")),
      "no bundled Lilex (mono) font"
    );
  }
}
