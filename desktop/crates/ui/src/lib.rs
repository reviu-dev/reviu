mod assets;
mod command_palette;
mod confirm_dialog;
mod github_url;
mod icons;
mod search_file_palette;
mod status_theme_ext;
mod theme;
mod user_menu;

pub const GLOBAL_BAR_HEIGHT: f32 = 36.0;
pub const PAGE_HEADER_HEIGHT: f32 = 40.0;
pub const DETAILS_PAGE_CONTAINER_MAX_WIDTH: f32 = 900.0;

pub use assets::AppAssets;
pub use command_palette::{
  CommandPalette, CommandPaletteAction, CommandPaletteBranch, CommandPaletteBranchKind,
  CommandPaletteCommand, CommandPaletteCommandId, CommandPaletteConfig,
  CommandPaletteGithubRepoTab, CommandPaletteHandler, CommandPalettePage, CommandPaletteRepository,
  CommandPaletteStash,
};
pub use confirm_dialog::ConfirmDialog;
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
pub use search_file_palette::{
  SearchFileEntry, SearchFileHandler, SearchFilePalette, SearchFilePaletteConfig,
};
pub use status_theme_ext::StatusThemeExt;
pub use theme::Theme;
pub use user_menu::{UserMenuConfig, UserMenuPage, UserMenuState, UserMenuUser, user_menu};
