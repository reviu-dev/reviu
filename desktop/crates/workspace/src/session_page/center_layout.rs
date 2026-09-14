use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::center_tab::{CenterTab, CenterTabKind, CenterTabSnapshot};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum PersistedCenterTab {
  Chat {
    conversation_id: Option<String>,
  },
  File {
    path: std::path::PathBuf,
  },
  Diff {
    path: std::path::PathBuf,
    snapshot: Option<CenterTabSnapshot>,
  },
  Terminal {
    key: u64,
  },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PersistedCenterSplitDirection {
  Up,
  Down,
  Left,
  Right,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum PersistedCenterNode {
  Pane {
    tabs: Vec<PersistedCenterTab>,
    active_tab: PersistedCenterTab,
  },
  Split {
    direction: PersistedCenterSplitDirection,
    first: Box<PersistedCenterNode>,
    second: Box<PersistedCenterNode>,
  },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct PersistedCenterLayout {
  pub(super) root: PersistedCenterNode,
  pub(super) active_tab: PersistedCenterTab,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CenterSurface {
  Chat(CenterTab),
  Editor(CenterTab),
  InteractiveRebase(CenterTab),
  Terminal(CenterTab),
}

impl CenterSurface {
  pub(super) fn from_tab(tab: CenterTab) -> Self {
    match tab.kind {
      CenterTabKind::Chat => Self::Chat(tab),
      CenterTabKind::File | CenterTabKind::Diff => Self::Editor(tab),
      CenterTabKind::InteractiveRebase => Self::InteractiveRebase(tab),
      CenterTabKind::Terminal => Self::Terminal(tab),
    }
  }

  pub(super) fn tab(&self) -> &CenterTab {
    match self {
      Self::Chat(tab) | Self::Editor(tab) | Self::InteractiveRebase(tab) | Self::Terminal(tab) => {
        tab
      }
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
pub(super) struct CenterSplitId(u64);

impl CenterSplitId {
  pub(super) fn as_u64(self) -> u64 {
    self.0
  }
}

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

  fn replace_tab(&mut self, old_tab: &CenterTab, new_tab: &CenterTab) {
    for surface in &mut self.surfaces {
      if surface.tab() == old_tab {
        *surface = CenterSurface::from_tab(new_tab.clone());
      }
    }
    if self.active_surface.tab() == old_tab {
      self.active_surface = CenterSurface::from_tab(new_tab.clone());
    }
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

    self.remove_surface_at(index);
    true
  }

  fn remove_surface(&mut self, tab: &CenterTab) -> Option<CenterSurface> {
    let index = self
      .surfaces
      .iter()
      .position(|surface| surface.tab() == tab)?;
    Some(self.remove_surface_at(index))
  }

  fn remove_surface_at(&mut self, index: usize) -> CenterSurface {
    let surface = self.surfaces.remove(index);
    if self.active_surface.tab() == surface.tab() {
      let next_index = index.min(self.surfaces.len().saturating_sub(1));
      if let Some(next_surface) = self.surfaces.get(next_index).cloned() {
        self.active_surface = next_surface;
      }
    }
    surface
  }

  fn surface_count(&self) -> usize {
    self.surfaces.len()
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CenterSplit {
  id: CenterSplitId,
  direction: CenterSplitDirection,
  first: Box<CenterNode>,
  second: Box<CenterNode>,
}

impl CenterSplit {
  fn new(
    id: CenterSplitId,
    old_node: CenterNode,
    new_node: CenterNode,
    direction: CenterSplitDirection,
  ) -> Self {
    let (first, second) = match direction {
      CenterSplitDirection::Up | CenterSplitDirection::Left => (new_node, old_node),
      CenterSplitDirection::Down | CenterSplitDirection::Right => (old_node, new_node),
    };
    Self {
      id,
      direction,
      first: Box::new(first),
      second: Box::new(second),
    }
  }

  pub(super) fn id(&self) -> CenterSplitId {
    self.id
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

struct ExtractResult {
  surface: Option<CenterSurface>,
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

  fn replace_tab(&mut self, old_tab: &CenterTab, new_tab: &CenterTab) {
    match self {
      Self::Pane(pane) => pane.replace_tab(old_tab, new_tab),
      Self::Split(split) => {
        split.first.replace_tab(old_tab, new_tab);
        split.second.replace_tab(old_tab, new_tab);
      }
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
    new_split_id: CenterSplitId,
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
        *self = Self::Split(CenterSplit::new(
          new_split_id,
          old_node,
          new_node,
          direction,
        ));
        true
      }
      Self::Split(split) => {
        split.first.split_pane(
          pane_id,
          new_pane_id,
          new_split_id,
          surface.clone(),
          direction,
        ) || split
          .second
          .split_pane(pane_id, new_pane_id, new_split_id, surface, direction)
      }
    }
  }

  #[cfg(test)]
  fn split_active(
    &mut self,
    active_tab: &CenterTab,
    new_pane_id: CenterPaneId,
    new_split_id: CenterSplitId,
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
        *self = Self::Split(CenterSplit::new(
          new_split_id,
          old_node,
          new_node,
          direction,
        ));
        true
      }
      Self::Split(split) => {
        split.first.split_active(
          active_tab,
          new_pane_id,
          new_split_id,
          surface.clone(),
          direction,
        ) || split
          .second
          .split_active(active_tab, new_pane_id, new_split_id, surface, direction)
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

  fn extract_surface(&mut self, tab: &CenterTab) -> ExtractResult {
    match self {
      Self::Pane(pane) => {
        let surface = pane.remove_surface(tab);
        ExtractResult {
          prune_node: surface.is_some() && pane.surface_count() == 0,
          surface,
        }
      }
      Self::Split(split) => {
        let first_result = split.first.extract_surface(tab);
        if first_result.surface.is_some() {
          if first_result.prune_node {
            *self = split.second.as_ref().clone();
          }
          return ExtractResult {
            surface: first_result.surface,
            prune_node: false,
          };
        }

        let second_result = split.second.extract_surface(tab);
        if second_result.surface.is_some() && second_result.prune_node {
          *self = split.first.as_ref().clone();
        }
        ExtractResult {
          surface: second_result.surface,
          prune_node: false,
        }
      }
    }
  }

  fn surface_at_edge(&self, tab: &CenterTab, direction: CenterSplitDirection) -> bool {
    match self {
      Self::Pane(pane) => pane.contains_tab(tab),
      Self::Split(split) => match (split.direction, direction) {
        (CenterSplitDirection::Left | CenterSplitDirection::Right, CenterSplitDirection::Left) => {
          split.first.surface_at_edge(tab, direction)
        }
        (CenterSplitDirection::Left | CenterSplitDirection::Right, CenterSplitDirection::Right) => {
          split.second.surface_at_edge(tab, direction)
        }
        (CenterSplitDirection::Up | CenterSplitDirection::Down, CenterSplitDirection::Up) => {
          split.first.surface_at_edge(tab, direction)
        }
        (CenterSplitDirection::Up | CenterSplitDirection::Down, CenterSplitDirection::Down) => {
          split.second.surface_at_edge(tab, direction)
        }
        _ => false,
      },
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

pub(super) fn persisted_center_tab(
  tab: &CenterTab,
  terminal_keys: &HashMap<u64, u64>,
) -> Option<PersistedCenterTab> {
  match tab.kind {
    CenterTabKind::Chat => Some(PersistedCenterTab::Chat {
      conversation_id: tab.conversation_id.clone(),
    }),
    CenterTabKind::File => Some(PersistedCenterTab::File {
      path: tab.path.clone()?,
    }),
    CenterTabKind::Diff => Some(PersistedCenterTab::Diff {
      path: tab.path.clone()?,
      snapshot: tab.snapshot.clone(),
    }),
    CenterTabKind::Terminal => Some(PersistedCenterTab::Terminal {
      key: terminal_keys.get(&tab.terminal_id()?).copied()?,
    }),
    CenterTabKind::InteractiveRebase => None,
  }
}

fn persisted_center_node(
  node: &CenterNode,
  terminal_keys: &HashMap<u64, u64>,
) -> Option<PersistedCenterNode> {
  match node {
    CenterNode::Pane(pane) => {
      let tabs = pane
        .surfaces
        .iter()
        .filter_map(|surface| persisted_center_tab(surface.tab(), terminal_keys))
        .collect::<Vec<_>>();
      let active_tab = persisted_center_tab(pane.active_surface.tab(), terminal_keys)
        .filter(|active| tabs.contains(active))
        .or_else(|| tabs.first().cloned())?;
      Some(PersistedCenterNode::Pane { tabs, active_tab })
    }
    CenterNode::Split(split) => {
      let first = persisted_center_node(&split.first, terminal_keys);
      let second = persisted_center_node(&split.second, terminal_keys);
      match (first, second) {
        (Some(first), Some(second)) => Some(PersistedCenterNode::Split {
          direction: match split.direction {
            CenterSplitDirection::Up => PersistedCenterSplitDirection::Up,
            CenterSplitDirection::Down => PersistedCenterSplitDirection::Down,
            CenterSplitDirection::Left => PersistedCenterSplitDirection::Left,
            CenterSplitDirection::Right => PersistedCenterSplitDirection::Right,
          },
          first: Box::new(first),
          second: Box::new(second),
        }),
        (Some(node), None) | (None, Some(node)) => Some(node),
        (None, None) => None,
      }
    }
  }
}

fn first_persisted_tab(node: &PersistedCenterNode) -> Option<PersistedCenterTab> {
  match node {
    PersistedCenterNode::Pane { tabs, .. } => tabs.first().cloned(),
    PersistedCenterNode::Split { first, .. } => first_persisted_tab(first),
  }
}

pub(super) fn collect_persisted_tabs(
  node: &PersistedCenterNode,
  tabs: &mut Vec<PersistedCenterTab>,
) {
  match node {
    PersistedCenterNode::Pane {
      tabs: pane_tabs, ..
    } => {
      for tab in pane_tabs {
        if !tabs.contains(tab) {
          tabs.push(tab.clone());
        }
      }
    }
    PersistedCenterNode::Split { first, second, .. } => {
      collect_persisted_tabs(first, tabs);
      collect_persisted_tabs(second, tabs);
    }
  }
}

fn center_node_from_persisted(
  node: &PersistedCenterNode,
  tabs: &HashMap<PersistedCenterTab, CenterTab>,
  next_id: &mut u64,
) -> Option<CenterNode> {
  match node {
    PersistedCenterNode::Pane {
      tabs: persisted_tabs,
      active_tab,
    } => {
      let surfaces = persisted_tabs
        .iter()
        .filter_map(|tab| tabs.get(tab).cloned())
        .map(CenterSurface::from_tab)
        .collect::<Vec<_>>();
      let active_surface = tabs
        .get(active_tab)
        .filter(|tab| surfaces.iter().any(|surface| surface.tab() == *tab))
        .cloned()
        .map(CenterSurface::from_tab)
        .or_else(|| surfaces.first().cloned())?;
      let pane_id = CenterPaneId(*next_id);
      *next_id = (*next_id).saturating_add(1);
      Some(CenterNode::Pane(CenterPane {
        id: pane_id,
        surfaces,
        active_surface,
      }))
    }
    PersistedCenterNode::Split {
      direction,
      first,
      second,
    } => {
      let first = center_node_from_persisted(first, tabs, next_id);
      let second = center_node_from_persisted(second, tabs, next_id);
      match (first, second) {
        (Some(first), Some(second)) => {
          let split_id = CenterSplitId(*next_id);
          *next_id = (*next_id).saturating_add(1);
          Some(CenterNode::Split(CenterSplit {
            id: split_id,
            direction: match direction {
              PersistedCenterSplitDirection::Up => CenterSplitDirection::Up,
              PersistedCenterSplitDirection::Down => CenterSplitDirection::Down,
              PersistedCenterSplitDirection::Left => CenterSplitDirection::Left,
              PersistedCenterSplitDirection::Right => CenterSplitDirection::Right,
            },
            first: Box::new(first),
            second: Box::new(second),
          }))
        }
        (Some(node), None) | (None, Some(node)) => Some(node),
        (None, None) => None,
      }
    }
  }
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

  pub(super) fn persisted_center_layout(
    &self,
    terminal_keys: &HashMap<u64, u64>,
  ) -> Option<PersistedCenterLayout> {
    let root = persisted_center_node(&self.root, terminal_keys)?;
    let active_tab = persisted_center_tab(self.active_tab(), terminal_keys)
      .filter(|active| {
        let mut tabs = Vec::new();
        collect_persisted_tabs(&root, &mut tabs);
        tabs.contains(active)
      })
      .or_else(|| first_persisted_tab(&root))?;
    Some(PersistedCenterLayout { root, active_tab })
  }

  pub(super) fn from_persisted_center_layout(
    persisted: &PersistedCenterLayout,
    tabs: &HashMap<PersistedCenterTab, CenterTab>,
  ) -> Option<Self> {
    let mut next_id = 0;
    let root = center_node_from_persisted(&persisted.root, tabs, &mut next_id)?;
    let active_tab = tabs
      .get(&persisted.active_tab)
      .filter(|tab| root.contains_tab(tab))
      .cloned()
      .unwrap_or_else(|| root.first_active_surface().tab().clone());
    Some(Self {
      root,
      active_tab,
      next_pane_id: next_id,
    })
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

  pub(super) fn replace_tab(&mut self, old_tab: &CenterTab, new_tab: &CenterTab) {
    self.root.replace_tab(old_tab, new_tab);
    if &self.active_tab == old_tab {
      self.active_tab = new_tab.clone();
    }
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
    let new_split_id = CenterSplitId(new_pane_id.0);
    if self
      .root
      .split_pane(pane_id, new_pane_id, new_split_id, surface, direction)
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
    let new_split_id = CenterSplitId(new_pane_id.0);
    if self.root.split_active(
      &self.active_tab,
      new_pane_id,
      new_split_id,
      surface,
      direction,
    ) {
      self.active_tab = tab;
      true
    } else {
      false
    }
  }

  pub(super) fn can_move_surface_to_edge(
    &self,
    tab: &CenterTab,
    direction: CenterSplitDirection,
  ) -> bool {
    self.root.surface_count() > 1
      && self.root.contains_tab(tab)
      && !self.root.surface_at_edge(tab, direction)
  }

  pub(super) fn move_surface_to_edge(
    &mut self,
    tab: &CenterTab,
    direction: CenterSplitDirection,
  ) -> bool {
    if !self.can_move_surface_to_edge(tab, direction) {
      return false;
    }
    let Some(surface) = self.extract_surface(tab) else {
      return false;
    };
    let old_node = self.root.clone();
    let new_pane_id = self.allocate_pane_id();
    let new_node = CenterNode::Pane(CenterPane::new(new_pane_id, surface));
    self.root = CenterNode::Split(CenterSplit::new(
      CenterSplitId(new_pane_id.0),
      old_node,
      new_node,
      direction,
    ));
    self.active_tab = tab.clone();
    true
  }

  pub(super) fn extract_surface(&mut self, tab: &CenterTab) -> Option<CenterSurface> {
    if self.root.surface_count() <= 1 {
      return None;
    }
    let result = self.root.extract_surface(tab);
    let surface = result.surface?;
    if &self.active_tab == tab {
      self.active_tab = self.root.first_active_surface().tab().clone();
    }
    Some(surface)
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
  fn center_layout_round_trips_mixed_surfaces_with_fresh_terminal_ids() {
    let terminal = CenterTab::terminal(1);
    let file = CenterTab::file(PathBuf::from("README.md"));
    let mut layout = CenterLayout::single(CenterSurface::from_tab(terminal.clone()));
    let pane_id = root_pane_id(&layout);
    assert!(layout.split_pane(
      pane_id,
      CenterSurface::from_tab(file.clone()),
      CenterSplitDirection::Right,
    ));
    let persisted = layout
      .persisted_center_layout(&HashMap::from([(1, 10)]))
      .expect("persisted center layout");

    let restored_terminal = CenterTab::terminal(101);
    let restored = CenterLayout::from_persisted_center_layout(
      &persisted,
      &HashMap::from([
        (
          PersistedCenterTab::Terminal { key: 10 },
          restored_terminal.clone(),
        ),
        (
          PersistedCenterTab::File {
            path: PathBuf::from("README.md"),
          },
          file.clone(),
        ),
      ]),
    )
    .expect("restored center layout");

    assert_split_tabs(
      &restored,
      CenterSplitDirection::Right,
      &restored_terminal,
      &file,
    );
    assert_eq!(restored.active_tab(), &file);
  }

  #[test]
  fn center_layout_drops_transient_rebase_panes() {
    let terminal = CenterTab::terminal(1);
    let rebase = CenterTab::interactive_rebase();
    let mut layout = CenterLayout::single(CenterSurface::from_tab(terminal.clone()));
    let pane_id = root_pane_id(&layout);
    assert!(layout.split_pane(
      pane_id,
      CenterSurface::from_tab(rebase),
      CenterSplitDirection::Right,
    ));

    let persisted = layout
      .persisted_center_layout(&HashMap::from([(1, 10)]))
      .expect("persisted center layout");
    assert_eq!(
      persisted.root,
      PersistedCenterNode::Pane {
        tabs: vec![PersistedCenterTab::Terminal { key: 10 }],
        active_tab: PersistedCenterTab::Terminal { key: 10 },
      }
    );
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
  fn extract_surface_collapses_the_split_without_closing_the_tab() {
    let readme = CenterTab::file(PathBuf::from("README.md"));
    let lib = CenterTab::file(PathBuf::from("src/lib.rs"));
    let mut layout = CenterLayout::single(CenterSurface::from_tab(readme.clone()));
    assert!(layout.split_active(
      CenterSurface::from_tab(lib.clone()),
      CenterSplitDirection::Right
    ));

    let extracted = layout.extract_surface(&lib).expect("extracted surface");

    assert_eq!(extracted.tab(), &lib);
    assert_eq!(layout.active_tab(), &readme);
    assert_eq!(
      layout.root,
      CenterNode::Pane(CenterPane::new(CenterPaneId(0), file("README.md")))
    );
  }

  #[test]
  fn move_surface_to_edge_reorders_a_split() {
    let readme = CenterTab::file(PathBuf::from("README.md"));
    let lib = CenterTab::file(PathBuf::from("src/lib.rs"));
    let mut layout = CenterLayout::single(CenterSurface::from_tab(readme.clone()));
    assert!(layout.split_active(
      CenterSurface::from_tab(lib.clone()),
      CenterSplitDirection::Right
    ));

    assert!(layout.can_move_surface_to_edge(&readme, CenterSplitDirection::Right));
    assert!(layout.move_surface_to_edge(&readme, CenterSplitDirection::Right));

    assert_eq!(layout.active_tab(), &readme);
    assert!(!layout.can_move_surface_to_edge(&readme, CenterSplitDirection::Right));
    assert_split_tabs(&layout, CenterSplitDirection::Right, &lib, &readme);
  }

  #[test]
  fn move_surface_to_edge_can_change_split_orientation() {
    let readme = CenterTab::file(PathBuf::from("README.md"));
    let lib = CenterTab::file(PathBuf::from("src/lib.rs"));
    let mut layout = CenterLayout::single(CenterSurface::from_tab(readme.clone()));
    assert!(layout.split_active(
      CenterSurface::from_tab(lib.clone()),
      CenterSplitDirection::Right
    ));

    assert!(layout.move_surface_to_edge(&lib, CenterSplitDirection::Up));

    assert_split_tabs(&layout, CenterSplitDirection::Up, &lib, &readme);
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
