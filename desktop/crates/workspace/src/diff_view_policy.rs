//! When a diff falls back to inline, whatever the user picked in settings.

use editor::DiffViewMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiffViewInputs {
  /// The mode the user asked for, from the toolbar or the settings.
  pub preferred: DiffViewMode,
  /// A binary file is showing its own preview instead of a diff.
  pub binary_preview: bool,
  /// The markdown or SVG preview is taking the pane.
  pub previewing: bool,
  /// Added, deleted or untracked: there is no other side to show.
  pub whole_file_change: bool,
}

impl Default for DiffViewInputs {
  fn default() -> Self {
    Self {
      preferred: DiffViewMode::Inline,
      binary_preview: false,
      previewing: false,
      whole_file_change: false,
    }
  }
}

pub(crate) fn effective_diff_view(inputs: DiffViewInputs) -> DiffViewMode {
  if inputs.binary_preview || inputs.previewing || inputs.whole_file_change {
    return DiffViewMode::Inline;
  }

  inputs.preferred
}

#[cfg(test)]
mod tests {
  use super::*;

  fn split() -> DiffViewInputs {
    DiffViewInputs {
      preferred: DiffViewMode::Split,
      ..DiffViewInputs::default()
    }
  }

  #[test]
  fn the_preferred_mode_wins_when_the_file_has_two_sides() {
    assert_eq!(effective_diff_view(split()), DiffViewMode::Split);
    assert_eq!(
      effective_diff_view(DiffViewInputs::default()),
      DiffViewMode::Inline
    );
  }

  #[test]
  fn split_falls_back_to_inline_when_there_is_nothing_to_compare() {
    for inputs in [
      DiffViewInputs {
        binary_preview: true,
        ..split()
      },
      DiffViewInputs {
        previewing: true,
        ..split()
      },
      DiffViewInputs {
        whole_file_change: true,
        ..split()
      },
    ] {
      assert_eq!(
        effective_diff_view(inputs),
        DiffViewMode::Inline,
        "{inputs:?}"
      );
    }
  }
}
