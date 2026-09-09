use gpui::{ElementId, Styled};
use gpui_component::{Theme, list::ListItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectableRowStyle {
  Flush,
  Inset,
}

pub fn selectable_list_item(
  ix: impl Into<ElementId>,
  selected: bool,
  style: SelectableRowStyle,
  theme: &Theme,
) -> ListItem {
  let item = ListItem::new(ix).selected(selected);
  if matches!(style, SelectableRowStyle::Inset) {
    item.rounded(theme.radius)
  } else {
    item
  }
}
