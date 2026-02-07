mod assets;
mod command_palette;
mod confirm_dialog;
mod icons;
mod search_file_palette;
mod theme;

pub const HEADER_HEIGHT: f32 = 50.0;

pub use assets::AppAssets;
pub use command_palette::{
  CommandPalette, CommandPaletteAction, CommandPaletteBranch, CommandPaletteBranchKind,
  CommandPaletteCommand, CommandPaletteCommandId, CommandPaletteConfig, CommandPaletteHandler,
};
pub use confirm_dialog::ConfirmDialog;
pub use icons::{
  FileIcon, UiIconName, FILE_ICON_SIZE_PX, file_icon_for_name, file_icon_for_path,
  file_icon_path_for_name, file_icon_path_for_name_with_theme, file_icon_path_for_path,
  file_icon_path_for_path_with_theme,
};
pub use search_file_palette::{
  SearchFileEntry, SearchFileHandler, SearchFilePalette, SearchFilePaletteConfig,
};
pub use theme::Theme;
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
