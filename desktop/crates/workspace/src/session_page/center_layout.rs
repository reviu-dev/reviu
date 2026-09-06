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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CenterSplitDirection {
  Up,
  Down,
  Left,
  Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CenterPaneId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CenterDropTarget {
  pub(super) pane_id: CenterPaneId,
  pub(super) direction: Option<CenterSplitDirection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CenterPane {
  id: CenterPaneId,
  surfaces: Vec<CenterSurface>,
  active_surface: CenterSurface,
}

impl CenterPane {
  fn new(id: CenterPaneId, active_surface: CenterSurface) -> Self {
    Self {
      id,
      surfaces: vec![active_surface.clone()],
      active_surface,
    }
  }

  pub(super) fn id(&self) -> CenterPaneId {
    self.id
  }

  pub(super) fn active_surface(&self) -> &CenterSurface {
    &self.active_surface
  }

  fn contains_tab(&self, tab: &CenterTab) -> bool {
    self.surfaces.iter().any(|surface| surface.tab() == tab)
  }

  fn add_surface(&mut self, surface: CenterSurface) {
    if !self.contains_tab(surface.tab()) {
      self.surfaces.push(surface.clone());
    }
    self.active_surface = surface;
  }

  fn set_active_surface(&mut self, surface: CenterSurface) {
    self.add_surface(surface);
  }

  fn close_surface(&mut self, tab: &CenterTab) -> bool {
    let Some(index) = self
      .surfaces
      .iter()
      .position(|surface| surface.tab() == tab)
    else {
      return false;
    };
    if self.surfaces.len() == 1 {
      return true;
    }

    self.surfaces.remove(index);
    if self.active_surface.tab() == tab {
      let next_index = index.min(self.surfaces.len().saturating_sub(1));
      if let Some(next_surface) = self.surfaces.get(next_index).cloned() {
        self.active_surface = next_surface;
      }
    }
    true
  }

  fn surface_count(&self) -> usize {
    self.surfaces.len()
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CenterSplit {
  direction: CenterSplitDirection,
  first: Box<CenterNode>,
  second: Box<CenterNode>,
}

impl CenterSplit {
  fn new(old_node: CenterNode, new_node: CenterNode, direction: CenterSplitDirection) -> Self {
    let (first, second) = match direction {
      CenterSplitDirection::Up | CenterSplitDirection::Left => (new_node, old_node),
      CenterSplitDirection::Down | CenterSplitDirection::Right => (old_node, new_node),
    };
    Self {
      direction,
      first: Box::new(first),
      second: Box::new(second),
    }
  }

  pub(super) fn direction(&self) -> CenterSplitDirection {
    self.direction
  }

  pub(super) fn first(&self) -> &CenterNode {
    &self.first
  }

  pub(super) fn second(&self) -> &CenterNode {
    &self.second
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CenterNode {
  Pane(CenterPane),
  Split(CenterSplit),
}

struct CloseResult {
  removed: bool,
  prune_node: bool,
}

impl CenterNode {
  fn first_active_surface(&self) -> &CenterSurface {
    match self {
      Self::Pane(pane) => pane.active_surface(),
      Self::Split(split) => split.first.first_active_surface(),
    }
  }

  fn active_surface_for_tab(&self, tab: &CenterTab) -> Option<&CenterSurface> {
    match self {
      Self::Pane(pane) => pane.contains_tab(tab).then(|| pane.active_surface()),
      Self::Split(split) => split
        .first
        .active_surface_for_tab(tab)
        .or_else(|| split.second.active_surface_for_tab(tab)),
    }
  }

  fn contains_tab(&self, tab: &CenterTab) -> bool {
    match self {
      Self::Pane(pane) => pane.contains_tab(tab),
      Self::Split(split) => split.first.contains_tab(tab) || split.second.contains_tab(tab),
    }
  }

  fn collect_tabs(&self, tabs: &mut Vec<CenterTab>) {
    match self {
      Self::Pane(pane) => {
        for surface in &pane.surfaces {
          let tab = surface.tab();
          if !tabs.iter().any(|existing| existing == tab) {
            tabs.push(tab.clone());
          }
        }
      }
      Self::Split(split) => {
        split.first.collect_tabs(tabs);
        split.second.collect_tabs(tabs);
      }
    }
  }

  fn pane_contains_tab(&self, pane_id: CenterPaneId, tab: &CenterTab) -> bool {
    match self {
      Self::Pane(pane) => pane.id == pane_id && pane.contains_tab(tab),
      Self::Split(split) => {
        split.first.pane_contains_tab(pane_id, tab) || split.second.pane_contains_tab(pane_id, tab)
      }
    }
  }

  fn pane_surface_count(&self, pane_id: CenterPaneId) -> Option<usize> {
    match self {
      Self::Pane(pane) => (pane.id == pane_id).then(|| pane.surface_count()),
      Self::Split(split) => split
        .first
        .pane_surface_count(pane_id)
        .or_else(|| split.second.pane_surface_count(pane_id)),
    }
  }

  fn activate_surface(&mut self, surface: CenterSurface) -> bool {
    match self {
      Self::Pane(pane) => {
        if pane.contains_tab(surface.tab()) {
          pane.set_active_surface(surface);
          true
        } else {
          false
        }
      }
      Self::Split(split) => {
        split.first.activate_surface(surface.clone()) || split.second.activate_surface(surface)
      }
    }
  }

  fn set_active_surface(&mut self, current_active_tab: &CenterTab, surface: CenterSurface) -> bool {
    match self {
      Self::Pane(pane) => {
        if pane.contains_tab(current_active_tab) {
          pane.set_active_surface(surface);
          true
        } else {
          false
        }
      }
      Self::Split(split) => {
        split
          .first
          .set_active_surface(current_active_tab, surface.clone())
          || split.second.set_active_surface(current_active_tab, surface)
      }
    }
  }

  fn split_pane(
    &mut self,
    pane_id: CenterPaneId,
    new_pane_id: CenterPaneId,
    surface: CenterSurface,
    direction: CenterSplitDirection,
  ) -> bool {
    match self {
      Self::Pane(pane) => {
        if pane.id != pane_id {
          return false;
        }
        let old_node = self.clone();
        let new_node = Self::Pane(CenterPane::new(new_pane_id, surface));
        *self = Self::Split(CenterSplit::new(old_node, new_node, direction));
        true
      }
      Self::Split(split) => {
        split
          .first
          .split_pane(pane_id, new_pane_id, surface.clone(), direction)
          || split
            .second
            .split_pane(pane_id, new_pane_id, surface, direction)
      }
    }
  }

  #[cfg(test)]
  fn split_active(
    &mut self,
    active_tab: &CenterTab,
    new_pane_id: CenterPaneId,
    surface: CenterSurface,
    direction: CenterSplitDirection,
  ) -> bool {
    match self {
      Self::Pane(pane) => {
        if !pane.contains_tab(active_tab) {
          return false;
        }
        let old_node = self.clone();
        let new_node = Self::Pane(CenterPane::new(new_pane_id, surface));
        *self = Self::Split(CenterSplit::new(old_node, new_node, direction));
        true
      }
      Self::Split(split) => {
        split
          .first
          .split_active(active_tab, new_pane_id, surface.clone(), direction)
          || split
            .second
            .split_active(active_tab, new_pane_id, surface, direction)
      }
    }
  }

  fn close_surface(&mut self, tab: &CenterTab) -> CloseResult {
    match self {
      Self::Pane(pane) => {
        let removed = pane.close_surface(tab);
        CloseResult {
          removed,
          prune_node: removed && pane.surface_count() == 1 && pane.contains_tab(tab),
        }
      }
      Self::Split(split) => {
        let first_result = split.first.close_surface(tab);
        if first_result.removed {
          if first_result.prune_node {
            *self = split.second.as_ref().clone();
          }
          return CloseResult {
            removed: true,
            prune_node: false,
          };
        }

        let second_result = split.second.close_surface(tab);
        if second_result.removed && second_result.prune_node {
          *self = split.first.as_ref().clone();
        }
        CloseResult {
          removed: second_result.removed,
          prune_node: false,
        }
      }
    }
  }

  fn surface_count(&self) -> usize {
    match self {
      Self::Pane(pane) => pane.surface_count(),
      Self::Split(split) => split.first.surface_count() + split.second.surface_count(),
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CenterLayout {
  root: CenterNode,
  active_tab: CenterTab,
  next_pane_id: u64,
}

impl CenterLayout {
  pub(super) fn single(active_surface: CenterSurface) -> Self {
    let active_tab = active_surface.tab().clone();
    Self {
      root: CenterNode::Pane(CenterPane::new(CenterPaneId(0), active_surface)),
      active_tab,
      next_pane_id: 1,
    }
  }

  fn allocate_pane_id(&mut self) -> CenterPaneId {
    let pane_id = CenterPaneId(self.next_pane_id);
    self.next_pane_id = self.next_pane_id.saturating_add(1);
    pane_id
  }

  pub(super) fn root(&self) -> &CenterNode {
    &self.root
  }

  pub(super) fn active_surface(&self) -> &CenterSurface {
    self
      .root
      .active_surface_for_tab(&self.active_tab)
      .unwrap_or_else(|| self.root.first_active_surface())
  }

  pub(super) fn active_tab(&self) -> &CenterTab {
    self.active_surface().tab()
  }

  pub(super) fn contains_tab(&self, tab: &CenterTab) -> bool {
    self.root.contains_tab(tab)
  }

  pub(super) fn tabs(&self) -> Vec<CenterTab> {
    let mut tabs = Vec::new();
    self.root.collect_tabs(&mut tabs);
    tabs
  }

  pub(super) fn surface_count(&self) -> usize {
    self.root.surface_count()
  }

  pub(super) fn set_active_surface(&mut self, surface: CenterSurface) {
    let tab = surface.tab().clone();
    if !self.root.activate_surface(surface.clone())
      && !self
        .root
        .set_active_surface(&self.active_tab, surface.clone())
    {
      self.root = CenterNode::Pane(CenterPane::new(self.allocate_pane_id(), surface));
    }
    self.active_tab = tab;
  }

  pub(super) fn split_pane(
    &mut self,
    pane_id: CenterPaneId,
    surface: CenterSurface,
    direction: CenterSplitDirection,
  ) -> bool {
    let tab = surface.tab().clone();
    if self.root.pane_contains_tab(pane_id, &tab)
      && self
        .root
        .pane_surface_count(pane_id)
        .is_some_and(|count| count <= 1)
    {
      return false;
    }
    if self.contains_tab(&tab) && !self.close_surface(&tab) {
      return false;
    }
    let new_pane_id = self.allocate_pane_id();
    if self
      .root
      .split_pane(pane_id, new_pane_id, surface, direction)
    {
      self.active_tab = tab;
      true
    } else {
      false
    }
  }

  #[cfg(test)]
  pub(super) fn split_active(
    &mut self,
    surface: CenterSurface,
    direction: CenterSplitDirection,
  ) -> bool {
    let tab = surface.tab().clone();
    let new_pane_id = self.allocate_pane_id();
    if self
      .root
      .split_active(&self.active_tab, new_pane_id, surface, direction)
    {
      self.active_tab = tab;
      true
    } else {
      false
    }
  }

  pub(super) fn close_surface(&mut self, tab: &CenterTab) -> bool {
    if self.root.surface_count() <= 1 {
      return false;
    }
    let result = self.root.close_surface(tab);
    if !result.removed {
      return false;
    }
    if &self.active_tab == tab {
      self.active_tab = self.root.first_active_surface().tab().clone();
    }
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  fn file(name: &str) -> CenterSurface {
    CenterSurface::from_tab(CenterTab::file(PathBuf::from(name)))
  }

  fn assert_split_tabs(
    layout: &CenterLayout,
    direction: CenterSplitDirection,
    first: &CenterTab,
    second: &CenterTab,
  ) {
    let CenterNode::Split(split) = &layout.root else {
      panic!("layout should be split");
    };
    assert_eq!(split.direction, direction);
    assert_eq!(split.first.first_active_surface().tab(), first);
    assert_eq!(split.second.first_active_surface().tab(), second);
  }

  fn root_pane_id(layout: &CenterLayout) -> CenterPaneId {
    let CenterNode::Pane(pane) = &layout.root else {
      panic!("layout should be a single pane");
    };
    pane.id
  }

  #[test]
  fn single_pane_layout_tracks_the_active_surface() {
    let mut layout = CenterLayout::single(CenterSurface::from_tab(CenterTab::chat()));
    assert_eq!(layout.active_tab(), &CenterTab::chat());

    let file = CenterTab::file(PathBuf::from("README.md"));
    layout.set_active_surface(CenterSurface::from_tab(file.clone()));
    assert_eq!(layout.active_tab(), &file);
  }

  #[test]
  fn split_pane_targets_the_requested_pane() {
    let readme = CenterTab::file(PathBuf::from("README.md"));
    let lib = CenterTab::file(PathBuf::from("src/lib.rs"));
    let license = CenterTab::file(PathBuf::from("LICENSE"));
    let mut layout = CenterLayout::single(CenterSurface::from_tab(readme.clone()));
    let first_pane_id = root_pane_id(&layout);
    assert!(layout.split_pane(
      first_pane_id,
      CenterSurface::from_tab(lib.clone()),
      CenterSplitDirection::Right,
    ));
    let CenterNode::Split(split) = &layout.root else {
      panic!("layout should be split");
    };
    let left_pane_id = match split.first.as_ref() {
      CenterNode::Pane(pane) => pane.id,
      CenterNode::Split(_) => panic!("left side should be a pane"),
    };

    assert!(layout.split_pane(
      left_pane_id,
      CenterSurface::from_tab(license.clone()),
      CenterSplitDirection::Down,
    ));
    let CenterNode::Split(split) = &layout.root else {
      panic!("layout should stay split");
    };
    let CenterNode::Split(left_split) = split.first.as_ref() else {
      panic!("requested pane should be split in place");
    };
    assert_eq!(left_split.direction, CenterSplitDirection::Down);
    assert_eq!(left_split.first.first_active_surface().tab(), &readme);
    assert_eq!(left_split.second.first_active_surface().tab(), &license);
    assert_eq!(split.second.first_active_surface().tab(), &lib);
  }

  #[test]
  fn split_active_places_the_new_surface_on_the_requested_side() {
    let readme = CenterTab::file(PathBuf::from("README.md"));
    let lib = CenterTab::file(PathBuf::from("src/lib.rs"));
    let mut layout = CenterLayout::single(CenterSurface::from_tab(readme.clone()));

    assert!(layout.split_active(
      CenterSurface::from_tab(lib.clone()),
      CenterSplitDirection::Right
    ));
    assert_eq!(layout.active_tab(), &lib);
    assert_split_tabs(&layout, CenterSplitDirection::Right, &readme, &lib);

    let mut layout = CenterLayout::single(CenterSurface::from_tab(readme.clone()));
    assert!(layout.split_active(
      CenterSurface::from_tab(lib.clone()),
      CenterSplitDirection::Left
    ));
    assert_eq!(layout.active_tab(), &lib);
    assert_split_tabs(&layout, CenterSplitDirection::Left, &lib, &readme);
  }

  #[test]
  fn close_surface_collapses_a_split_to_the_remaining_pane() {
    let readme = CenterTab::file(PathBuf::from("README.md"));
    let lib = CenterTab::file(PathBuf::from("src/lib.rs"));
    let mut layout = CenterLayout::single(CenterSurface::from_tab(readme.clone()));
    assert!(layout.split_active(
      CenterSurface::from_tab(lib.clone()),
      CenterSplitDirection::Right
    ));

    assert!(layout.close_surface(&lib));
    assert_eq!(layout.active_tab(), &readme);
    assert_eq!(
      layout.root,
      CenterNode::Pane(CenterPane::new(
        CenterPaneId(0),
        CenterSurface::from_tab(readme)
      ))
    );
  }

  #[test]
  fn close_surface_keeps_the_last_surface_alive() {
    let mut layout = CenterLayout::single(file("README.md"));
    let readme = CenterTab::file(PathBuf::from("README.md"));

    assert!(!layout.close_surface(&readme));
    assert_eq!(layout.active_tab(), &readme);
  }

  #[test]
  fn split_and_close_supports_all_directions() {
    for direction in [
      CenterSplitDirection::Up,
      CenterSplitDirection::Down,
      CenterSplitDirection::Left,
      CenterSplitDirection::Right,
    ] {
      let readme = CenterTab::file(PathBuf::from("README.md"));
      let lib = CenterTab::file(PathBuf::from("src/lib.rs"));
      let mut layout = CenterLayout::single(CenterSurface::from_tab(readme.clone()));

      assert!(layout.split_active(CenterSurface::from_tab(lib.clone()), direction));
      assert!(layout.close_surface(&readme));
      assert_eq!(layout.active_tab(), &lib);
    }
  }
}
