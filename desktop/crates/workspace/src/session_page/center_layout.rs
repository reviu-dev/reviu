use super::center_tab::{CenterTab, CenterTabKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CenterSurface {
  Chat(CenterTab),
  Editor(CenterTab),
  InteractiveRebase(CenterTab),
}

impl CenterSurface {
  pub(super) fn from_tab(tab: CenterTab) -> Self {
    match tab.kind {
      CenterTabKind::Chat => Self::Chat(tab),
      CenterTabKind::File | CenterTabKind::Diff => Self::Editor(tab),
      CenterTabKind::InteractiveRebase => Self::InteractiveRebase(tab),
    }
  }

  pub(super) fn tab(&self) -> &CenterTab {
    match self {
      Self::Chat(tab) | Self::Editor(tab) | Self::InteractiveRebase(tab) => tab,
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CenterPane {
  active_surface: CenterSurface,
}

impl CenterPane {
  fn new(active_surface: CenterSurface) -> Self {
    Self { active_surface }
  }

  fn active_surface(&self) -> &CenterSurface {
    &self.active_surface
  }

  fn set_active_surface(&mut self, surface: CenterSurface) {
    self.active_surface = surface;
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CenterLayout {
  root: CenterPane,
}

impl CenterLayout {
  pub(super) fn single(active_surface: CenterSurface) -> Self {
    Self {
      root: CenterPane::new(active_surface),
    }
  }

  pub(super) fn active_surface(&self) -> &CenterSurface {
    self.root.active_surface()
  }

  pub(super) fn active_tab(&self) -> &CenterTab {
    self.active_surface().tab()
  }

  pub(super) fn set_active_surface(&mut self, surface: CenterSurface) {
    self.root.set_active_surface(surface);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  #[test]
  fn single_pane_layout_tracks_the_active_surface() {
    let mut layout = CenterLayout::single(CenterSurface::from_tab(CenterTab::chat()));
    assert_eq!(layout.active_tab(), &CenterTab::chat());

    let file = CenterTab::file(PathBuf::from("README.md"));
    layout.set_active_surface(CenterSurface::from_tab(file.clone()));
    assert_eq!(layout.active_tab(), &file);
  }
}
