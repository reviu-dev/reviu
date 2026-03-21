use gpui::{ElementId, Styled};
use gpui_component::{Theme, list::ListItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectableRowStyle {
  Flush,
  Inset,
}

impl SelectableRowStyle {
  fn rounds_row(self) -> bool {
    matches!(self, Self::Inset)
  }
}

pub fn selectable_list_item(
  ix: impl Into<ElementId>,
  selected: bool,
  style: SelectableRowStyle,
  theme: &Theme,
) -> ListItem {
  let item = ListItem::new(ix).selected(selected);
  if style.rounds_row() {
    item.rounded(theme.radius)
  } else {
    item
  }
}

#[cfg(test)]
mod tests {
  use super::SelectableRowStyle;

  #[test]
  fn inset_rows_round_and_flush_rows_do_not() {
    assert!(!SelectableRowStyle::Flush.rounds_row());
    assert!(SelectableRowStyle::Inset.rounds_row());
  }
}
