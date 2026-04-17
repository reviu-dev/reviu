mod assets;
mod command_palette;
mod confirm_dialog;
mod dropdown_select;
mod github_url;
mod icons;
mod reaction_bar;
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
  CommandPaletteGithubRepoTab, CommandPaletteHandler, CommandPaletteInitialScreen,
  CommandPalettePage, CommandPaletteRepository, CommandPaletteStash,
};
pub use confirm_dialog::ConfirmDialog;
pub use dropdown_select::{
  DropdownSelectConfig, DropdownSelectItem, DropdownSelectOption, dropdown_select,
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
pub use reaction_bar::{ReactionBar, ReactionGroup, ReactionOption, ReactionToggle};
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
}
