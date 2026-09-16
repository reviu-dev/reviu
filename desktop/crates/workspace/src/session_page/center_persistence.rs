use std::path::Component;

use super::*;

const CENTER_WORKSPACE_VERSION: u8 = 2;

#[derive(serde::Deserialize, serde::Serialize)]
struct PersistedCenterWorkspace {
  version: u8,
  terminals: Vec<PersistedTerminal>,
  tabs: Vec<PersistedCenterTab>,
  layouts: Vec<PersistedCenterLayoutEntry>,
  active_tab: Option<PersistedCenterTab>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PersistedTerminal {
  key: u64,
  working_directory: PathBuf,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PersistedCenterLayoutEntry {
  representative: PersistedCenterTab,
  layout: PersistedCenterLayout,
}

fn safe_relative_path(path: &Path) -> bool {
  !path.as_os_str().is_empty()
    && path
      .components()
      .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn collect_workspace_tabs(state: &PersistedCenterWorkspace) -> Vec<PersistedCenterTab> {
  let mut tabs = state.tabs.clone();
  for entry in &state.layouts {
    if !tabs.contains(&entry.representative) {
      tabs.push(entry.representative.clone());
    }
    collect_persisted_tabs(&entry.layout.root, &mut tabs);
  }
  if let Some(active_tab) = state.active_tab.as_ref()
    && !tabs.contains(active_tab)
  {
    tabs.push(active_tab.clone());
  }
  tabs
}

impl SessionPage {
  fn restored_center_tab(
    &self,
    persisted: &PersistedCenterTab,
    terminals: &HashMap<u64, CenterTab>,
    checkout_root: &Path,
    cx: &App,
  ) -> Option<CenterTab> {
    match persisted {
      PersistedCenterTab::Chat { conversation_id } => match conversation_id {
        Some(id) => {
          let (project_root, store, _) = self.conversation_hub.find_conversation(id, cx)?;
          let conversation_checkout = store
            .read(cx)
            .worktree(id)
            .map(|binding| binding.path)
            .unwrap_or(project_root);
          (Self::canonical_repo(&conversation_checkout) == checkout_root)
            .then(|| CenterTab::chat_for(id.clone()))
        }
        None => Some(CenterTab::chat()),
      },
      PersistedCenterTab::File { path } => (safe_relative_path(path)
        && checkout_root.join(path).is_file())
      .then(|| CenterTab::file(path.clone())),
      PersistedCenterTab::Diff { path, snapshot } => safe_relative_path(path).then(|| CenterTab {
        kind: CenterTabKind::Diff,
        path: Some(path.clone()),
        conversation_id: None,
        snapshot: snapshot.clone(),
        terminal_id: None,
        untitled_id: None,
      }),
      PersistedCenterTab::ProjectSearch => Some(CenterTab::project_search()),
      PersistedCenterTab::Terminal { key } => terminals.get(key).cloned(),
    }
  }

  pub(super) fn restore_center_workspace(
    &mut self,
    checkout_root: &Path,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.center_tabs_by_checkout.contains_key(checkout_root) {
      return;
    }
    let Some(state) = ConfigStore::load_center_workspace(checkout_root) else {
      return;
    };
    let Ok(state) = serde_json::from_str::<PersistedCenterWorkspace>(&state) else {
      log::warn!(
        "Failed to parse persisted center workspace for {}",
        checkout_root.display()
      );
      ConfigStore::forget_center_workspace(checkout_root);
      return;
    };
    if state.version != CENTER_WORKSPACE_VERSION {
      ConfigStore::forget_center_workspace(checkout_root);
      return;
    }

    let project_root = self
      .project_root(cx)
      .unwrap_or_else(|| checkout_root.to_path_buf());
    let project_root = Self::canonical_repo(&project_root);
    let mut terminals = HashMap::new();
    for persisted in &state.terminals {
      let working_directory = if persisted.working_directory.is_dir() {
        persisted.working_directory.clone()
      } else {
        checkout_root.to_path_buf()
      };
      let tab = self.create_terminal_tab(
        working_directory,
        project_root.clone(),
        checkout_root.to_path_buf(),
        window,
        cx,
      );
      terminals.insert(persisted.key, tab);
    }

    let runtime_tabs = collect_workspace_tabs(&state)
      .into_iter()
      .filter_map(|persisted| {
        self
          .restored_center_tab(&persisted, &terminals, checkout_root, cx)
          .map(|tab| (persisted, tab))
      })
      .collect::<HashMap<_, _>>();
    let mut tabs = state
      .tabs
      .iter()
      .filter_map(|tab| runtime_tabs.get(tab).cloned())
      .collect::<Vec<_>>();
    let mut restored_layouts = HashMap::new();
    for entry in &state.layouts {
      let Some(representative) = runtime_tabs.get(&entry.representative).cloned() else {
        continue;
      };
      let Some(layout) = CenterLayout::from_persisted_center_layout(&entry.layout, &runtime_tabs)
      else {
        continue;
      };
      if !tabs.contains(&representative) {
        tabs.push(representative.clone());
      }
      restored_layouts.insert(representative, layout);
    }
    if tabs.is_empty() {
      ConfigStore::forget_center_workspace(checkout_root);
      return;
    }

    let tabs = CenterTab::with_chat_tab(tabs);
    let active_tab = state
      .active_tab
      .as_ref()
      .and_then(|tab| runtime_tabs.get(tab).cloned())
      .filter(|tab| tabs.contains(tab))
      .unwrap_or_else(|| tabs.last().cloned().unwrap_or_else(CenterTab::chat));
    self.center_layouts_by_tab.extend(restored_layouts);
    self
      .center_tabs_by_checkout
      .insert(checkout_root.to_path_buf(), tabs);
    self
      .center_active_tab_by_checkout
      .insert(checkout_root.to_path_buf(), active_tab);
  }

  pub(super) fn persist_current_center_workspace(&mut self, cx: &App) {
    let checkout_root = self
      .synced_checkout
      .clone()
      .or_else(|| self.checkout_root(cx));
    let Some(checkout_root) = checkout_root else {
      return;
    };
    let checkout_root = Self::canonical_repo(&checkout_root);
    self.persist_center_workspace(&checkout_root, cx);
  }

  pub(super) fn persist_center_workspace_for_terminal(&mut self, terminal_id: u64, cx: &App) {
    let Some(checkout_root) = self
      .terminal_views
      .get(&terminal_id)
      .map(|terminal| terminal.checkout_root.clone())
    else {
      return;
    };
    self.persist_center_workspace(&checkout_root, cx);
  }

  pub(super) fn persist_center_workspace(&mut self, checkout_root: &Path, cx: &App) {
    let mut terminal_ids = self
      .terminal_views
      .iter()
      .filter(|(_, terminal)| terminal.checkout_root == checkout_root)
      .map(|(terminal_id, _)| *terminal_id)
      .collect::<Vec<_>>();
    terminal_ids.sort_unstable();
    let terminal_keys = terminal_ids
      .iter()
      .enumerate()
      .map(|(index, terminal_id)| (*terminal_id, index as u64 + 1))
      .collect::<HashMap<_, _>>();
    let terminals = terminal_ids
      .iter()
      .filter_map(|terminal_id| {
        let terminal = self.terminal_views.get(terminal_id)?;
        let working_directory = terminal
          .view
          .read(cx)
          .working_directory()
          .map(Path::to_path_buf)
          .unwrap_or_else(|| terminal.checkout_root.clone());
        Some(PersistedTerminal {
          key: terminal_keys.get(terminal_id).copied()?,
          working_directory,
        })
      })
      .collect::<Vec<_>>();

    let current_checkout = self.synced_checkout.as_deref() == Some(checkout_root)
      || (self.synced_checkout.is_none()
        && self
          .checkout_root(cx)
          .map(|path| Self::canonical_repo(&path))
          .as_deref()
          == Some(checkout_root));
    let runtime_tabs = if current_checkout {
      self.center_tabs.clone()
    } else {
      self
        .center_tabs_by_checkout
        .get(checkout_root)
        .cloned()
        .unwrap_or_default()
    };
    let tabs = runtime_tabs
      .iter()
      .filter_map(|tab| persisted_center_tab(tab, &terminal_keys))
      .collect::<Vec<_>>();
    let only_placeholder = matches!(
      tabs.as_slice(),
      [PersistedCenterTab::Chat {
        conversation_id: None
      }]
    );
    if tabs.is_empty() || (only_placeholder && terminals.is_empty()) {
      ConfigStore::forget_center_workspace(checkout_root);
      return;
    }

    let mut layouts = Vec::new();
    for representative in runtime_tabs {
      let Some(persisted_representative) = persisted_center_tab(&representative, &terminal_keys)
      else {
        continue;
      };
      let layout = if current_checkout && self.active_center_tab.as_ref() == Some(&representative) {
        Some(&self.center_layout)
      } else {
        self.center_layouts_by_tab.get(&representative)
      };
      let persisted_layout = layout
        .and_then(|layout| layout.persisted_center_layout(&terminal_keys))
        .or_else(|| {
          CenterLayout::single(CenterSurface::from_tab(representative.clone()))
            .persisted_center_layout(&terminal_keys)
        });
      if let Some(layout) = persisted_layout {
        layouts.push(PersistedCenterLayoutEntry {
          representative: persisted_representative,
          layout,
        });
      }
    }

    let active_tab = if current_checkout {
      self.active_center_tab.as_ref()
    } else {
      self.center_active_tab_by_checkout.get(checkout_root)
    }
    .and_then(|tab| persisted_center_tab(tab, &terminal_keys));
    let state = PersistedCenterWorkspace {
      version: CENTER_WORKSPACE_VERSION,
      terminals,
      tabs,
      layouts,
      active_tab,
    };
    let project_root = self
      .terminal_views
      .values()
      .find(|terminal| terminal.checkout_root == checkout_root)
      .map(|terminal| terminal.project_root.clone())
      .or_else(|| self.project_root(cx))
      .unwrap_or_else(|| checkout_root.to_path_buf());
    match serde_json::to_string(&state) {
      Ok(state) => ConfigStore::persist_center_workspace(checkout_root, &project_root, &state),
      Err(error) => log::warn!("Failed to serialize center workspace: {error}"),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::session_page::center_layout::{CenterNode, CenterPaneId};
  use crate::session_page::test_support::{
    add_session_page_window_from_config, isolate_config_store_for_test,
  };
  use gpui::TestAppContext;

  fn pane_for_tab(node: &CenterNode, tab: &CenterTab) -> Option<CenterPaneId> {
    match node {
      CenterNode::Pane(pane) => (pane.active_surface().tab() == tab).then(|| pane.id()),
      CenterNode::Split(split) => {
        pane_for_tab(split.first(), tab).or_else(|| pane_for_tab(split.second(), tab))
      }
    }
  }

  fn persist_chat(
    panel: &Entity<AgentChatPanel>,
    text: &str,
    cx: &mut gpui::VisualTestContext,
  ) -> String {
    panel.update(cx, |panel, cx| {
      panel.seed_user_message_for_test(text, cx);
      panel.persist_now(cx);
      if let Some(store) = panel.store() {
        store.update(cx, |store, _| store.flush_on_quit());
      }
    });
    panel.read_with(cx, |panel, _| panel.current_conversation().id.clone())
  }

  #[gpui::test]
  fn mixed_center_split_restores_every_surface_in_a_fresh_page(cx: &mut TestAppContext) {
    isolate_config_store_for_test();
    let project = crate::test_support::TempDir::new("center-persistence");
    std::fs::write(project.path.join("README.md"), "readme\n").expect("write readme");
    std::fs::write(project.path.join("main.rs"), "fn main() {}\n").expect("write main");
    let terminal_directory = project.path.join("terminal");
    std::fs::create_dir_all(&terminal_directory).expect("create terminal directory");
    ConfigStore::persist_recent_project(&project.path);

    let (page, cx) = add_session_page_window_from_config(cx);
    page.update_in(cx, |page, window, cx| page.activate(window, cx));
    cx.run_until_parked();
    let first_panel = page
      .read_with(cx, |page, _| page.agent_chat_view.clone())
      .expect("first chat panel");
    let first_chat_id = persist_chat(&first_panel, "first", cx);
    page.update_in(cx, |page, window, cx| page.new_session(window, cx));
    cx.run_until_parked();
    let second_panel = page
      .read_with(cx, |page, _| page.agent_chat_view.clone())
      .expect("second chat panel");
    let second_chat_id = persist_chat(&second_panel, "second", cx);

    page.update_in(cx, |page, window, cx| {
      page.new_terminal_tab(window, cx);
      let terminal = page.center_layout.active_tab().clone();
      page
        .terminal_for_tab(&terminal)
        .expect("terminal")
        .update(cx, |terminal, cx| {
          terminal.set_working_directory(Some(terminal_directory.clone()), cx)
        });
      let first_chat = CenterTab::chat_for(first_chat_id.clone());
      let second_chat = CenterTab::chat_for(second_chat_id.clone());
      let file = CenterTab::file(PathBuf::from("README.md"));
      let diff = CenterTab::diff(PathBuf::from("main.rs"));
      let mut layout = CenterLayout::single(CenterSurface::from_tab(first_chat.clone()));
      let first_pane = pane_for_tab(layout.root(), &first_chat).expect("first chat pane");
      assert!(layout.split_pane(
        first_pane,
        CenterSurface::from_tab(second_chat.clone()),
        CenterSplitDirection::Right,
      ));
      let second_pane = pane_for_tab(layout.root(), &second_chat).expect("second chat pane");
      assert!(layout.split_pane(
        second_pane,
        CenterSurface::from_tab(file.clone()),
        CenterSplitDirection::Down,
      ));
      let file_pane = pane_for_tab(layout.root(), &file).expect("file pane");
      assert!(layout.split_pane(
        file_pane,
        CenterSurface::from_tab(diff.clone()),
        CenterSplitDirection::Right,
      ));
      let diff_pane = pane_for_tab(layout.root(), &diff).expect("diff pane");
      assert!(layout.split_pane(
        diff_pane,
        CenterSurface::from_tab(terminal.clone()),
        CenterSplitDirection::Down,
      ));
      layout.set_active_surface(CenterSurface::from_tab(second_chat));
      page.center_layout = layout;
      page.center = CenterView::Conversation;
      page.remember_center_layout_tab(terminal);
      page.persist_current_center_workspace(cx);
    });
    cx.run_until_parked();
    std::fs::remove_dir_all(&terminal_directory).expect("remove stale terminal directory");

    let restored = cx.update(|window, cx| cx.new(|cx| SessionPage::new(window, cx)));
    restored.update_in(cx, |page, window, cx| page.activate(window, cx));
    cx.run_until_parked();
    restored.read_with(cx, |page, cx| {
      assert_eq!(page.center_layout.surface_count(), 5);
      assert_eq!(page.center, CenterView::Conversation);
      assert_eq!(
        page.center_layout.active_tab(),
        &CenterTab::chat_for(second_chat_id.clone())
      );
      assert_eq!(
        page
          .agent_chat_view
          .as_ref()
          .expect("active restored chat")
          .read(cx)
          .current_conversation()
          .id,
        second_chat_id
      );
      let tabs = page.center_layout.tabs();
      let chat_ids = tabs
        .iter()
        .filter_map(|tab| tab.conversation_id().map(ToOwned::to_owned))
        .collect::<HashSet<_>>();
      assert_eq!(chat_ids, HashSet::from([first_chat_id, second_chat_id]));
      for tab in tabs
        .iter()
        .filter(|tab| matches!(tab.kind, CenterTabKind::File | CenterTabKind::Diff))
      {
        assert!(
          page
            .editor_states
            .get(tab)
            .and_then(|state| state.editor.as_ref())
            .is_some(),
          "restored editor should finish loading: {tab:?}"
        );
      }
      let terminal = tabs
        .iter()
        .find(|tab| tab.kind == CenterTabKind::Terminal)
        .and_then(|tab| page.terminal_for_tab(tab))
        .expect("restored terminal");
      let project_root = project.path.canonicalize().expect("canonical project");
      assert_eq!(
        terminal.read(cx).working_directory(),
        Some(project_root.as_path())
      );
    });
  }
}
