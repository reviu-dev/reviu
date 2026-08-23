//! Walking a list is not choosing from it. One shows what the row holds, the
//! other hands the keyboard to the editor.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenIntent {
  /// Moving through a list: the centre follows, the focus stays where it is.
  Browse,
  /// Enter or a click: the editor takes over, and the reading starts.
  Open,
}

impl OpenIntent {
  /// Whether the editor takes the keyboard once the file is there.
  pub(crate) fn takes_focus(self) -> bool {
    matches!(self, Self::Open)
  }
}
