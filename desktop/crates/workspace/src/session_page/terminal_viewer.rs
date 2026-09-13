use super::*;

const TERMINAL_WORKSPACE_VERSION: u8 = 1;

#[derive(serde::Deserialize, serde::Serialize)]
struct PersistedTerminalWorkspace {
  version: u8,
  terminals: Vec<PersistedTerminal>,
  layouts: Vec<PersistedTerminalLayoutEntry>,
  active_terminal: Option<u64>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PersistedTerminal {
  key: u64,
  working_directory: PathBuf,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PersistedTerminalLayoutEntry {
  representative: u64,
  layout: PersistedTerminalLayout,
}

fn terminal_relative_path(checkout_root: &Path, path: &Path) -> Option<PathBuf> {
  let path = path.canonicalize().ok()?;
  if !path.is_file() {
    return None;
  }
  path.strip_prefix(checkout_root).ok().map(Path::to_path_buf)
}

fn collect_persisted_terminal_keys(node: &PersistedTerminalNode, keys: &mut HashSet<u64>) {
  match node {
    PersistedTerminalNode::Pane { terminals, .. } => keys.extend(terminals),
    PersistedTerminalNode::Split { first, second, .. } => {
      collect_persisted_terminal_keys(first, keys);
      collect_persisted_terminal_keys(second, keys);
    }
  }
}

impl SessionPage {
  pub(super) fn new_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(working_directory) = self.checkout_root(cx).or_else(|| self.creation_root(cx)) else {
      window.push_notification(
        Notification::warning("Open a project before starting a terminal."),
        cx,
      );
      return;
    };
    let project_root = self
      .project_root(cx)
      .unwrap_or_else(|| working_directory.clone());
    let project_root = project_root.canonicalize().unwrap_or(project_root);
    let checkout_root = working_directory
      .canonicalize()
      .unwrap_or_else(|_| working_directory.clone());

    let tab = self.create_terminal_tab(working_directory, project_root, checkout_root, window, cx);
    self.center = CenterView::Terminal;
    self.remember_center_tab(tab.clone());
    self.focus_terminal_tab(&tab, window, cx);
    self.persist_current_terminal_workspace(cx);
    cx.notify();
  }

  fn create_terminal_tab(
    &mut self,
    working_directory: PathBuf,
    project_root: PathBuf,
    checkout_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> CenterTab {
    let terminal_id = self.next_terminal_id;
    self.next_terminal_id = self.next_terminal_id.saturating_add(1);
    let terminal = cx.new(|cx| TerminalView::new(Some(working_directory), cx));
    cx.subscribe_in(
      &terminal,
      window,
      move |this, _terminal, event: &TerminalViewEvent, window, cx| match event {
        TerminalViewEvent::OpenFile { path, line, column } => {
          this.open_file_from_terminal(terminal_id, path, *line, *column, window, cx)
        }
        TerminalViewEvent::WorkingDirectoryChanged { .. } => {
          this.persist_terminal_workspace_for_terminal(terminal_id, cx);
          cx.notify();
        }
      },
    )
    .detach();
    self.terminal_views.insert(
      terminal_id,
      TerminalPane {
        project_root,
        checkout_root,
        view: terminal,
      },
    );
    CenterTab::terminal(terminal_id)
  }

  pub(super) fn open_file_from_terminal(
    &mut self,
    terminal_id: u64,
    path: &Path,
    line: Option<u32>,
    column: Option<u32>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(terminal) = self.terminal_views.get(&terminal_id) else {
      return;
    };
    let checkout_root = terminal.checkout_root.clone();
    let Some(relative_path) = terminal_relative_path(&checkout_root, path) else {
      window.push_notification(
        Notification::warning("This file is outside the terminal's checkout."),
        cx,
      );
      return;
    };
    let active_checkout = self
      .checkout_root(cx)
      .and_then(|path| path.canonicalize().ok());
    if active_checkout.as_deref() != Some(checkout_root.as_path()) {
      window.push_notification(
        Notification::warning("Return to this terminal's checkout before opening its files."),
        cx,
      );
      return;
    }

    self.open_file(relative_path, line, column, OpenIntent::Open, window, cx);
  }

  pub(super) fn restore_terminal_workspace(
    &mut self,
    checkout_root: &Path,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.center_tabs_by_checkout.contains_key(checkout_root) {
      return;
    }
    let Some(state) = ConfigStore::load_terminal_workspace(checkout_root) else {
      return;
    };
    let Ok(state) = serde_json::from_str::<PersistedTerminalWorkspace>(&state) else {
      log::warn!(
        "Failed to parse persisted terminal workspace for {}",
        checkout_root.display()
      );
      ConfigStore::forget_terminal_workspace(checkout_root);
      return;
    };
    if state.version != TERMINAL_WORKSPACE_VERSION {
      return;
    }

    let project_root = self
      .project_root(cx)
      .unwrap_or_else(|| checkout_root.to_path_buf());
    let project_root = project_root.canonicalize().unwrap_or(project_root);
    let mut terminals = HashMap::new();
    for persisted in state.terminals {
      let working_directory = if persisted.working_directory.is_dir() {
        persisted.working_directory
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
    if terminals.is_empty() {
      ConfigStore::forget_terminal_workspace(checkout_root);
      return;
    }

    let mut tabs = Vec::new();
    let mut represented_terminals = HashSet::new();
    for persisted in state.layouts {
      let Some(representative) = terminals.get(&persisted.representative).cloned() else {
        continue;
      };
      let Some(layout) =
        CenterLayout::from_persisted_terminal_layout(&persisted.layout, &terminals)
      else {
        continue;
      };
      collect_persisted_terminal_keys(&persisted.layout.root, &mut represented_terminals);
      if !tabs.contains(&representative) {
        tabs.push(representative.clone());
      }
      self.center_layouts_by_tab.insert(representative, layout);
    }
    let mut remaining_terminal_keys = terminals
      .keys()
      .filter(|key| !represented_terminals.contains(key))
      .copied()
      .collect::<Vec<_>>();
    remaining_terminal_keys.sort_unstable();
    for key in remaining_terminal_keys {
      if let Some(terminal) = terminals.get(&key) {
        tabs.push(terminal.clone());
      }
    }
    let tabs = CenterTab::with_chat_tab(tabs);
    let active_tab = state
      .active_terminal
      .and_then(|key| terminals.get(&key).cloned())
      .filter(|tab| tabs.contains(tab))
      .unwrap_or_else(CenterTab::chat);
    self
      .center_tabs_by_checkout
      .insert(checkout_root.to_path_buf(), tabs);
    self
      .center_active_tab_by_checkout
      .insert(checkout_root.to_path_buf(), active_tab);
  }

  pub(super) fn persist_current_terminal_workspace(&mut self, cx: &App) {
    let Some(checkout_root) = self.checkout_root(cx) else {
      return;
    };
    let checkout_root = checkout_root.canonicalize().unwrap_or(checkout_root);
    self.persist_terminal_workspace(&checkout_root, cx);
  }

  fn persist_terminal_workspace_for_terminal(&mut self, terminal_id: u64, cx: &App) {
    let Some(checkout_root) = self
      .terminal_views
      .get(&terminal_id)
      .map(|terminal| terminal.checkout_root.clone())
    else {
      return;
    };
    self.persist_terminal_workspace(&checkout_root, cx);
  }

  pub(super) fn persist_terminal_workspace(&mut self, checkout_root: &Path, cx: &App) {
    let mut terminal_ids = self
      .terminal_views
      .iter()
      .filter(|(_, terminal)| terminal.checkout_root == checkout_root)
      .map(|(terminal_id, _)| *terminal_id)
      .collect::<Vec<_>>();
    terminal_ids.sort_unstable();
    if terminal_ids.is_empty() {
      ConfigStore::forget_terminal_workspace(checkout_root);
      return;
    }

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

    let current_checkout = self.synced_checkout.as_deref() == Some(checkout_root);
    let tabs = if current_checkout {
      self.center_tabs.clone()
    } else {
      self
        .center_tabs_by_checkout
        .get(checkout_root)
        .cloned()
        .unwrap_or_default()
    };
    let mut layouts = Vec::new();
    let mut represented_terminals = HashSet::new();
    for representative in tabs {
      let Some(terminal_id) = representative.terminal_id() else {
        continue;
      };
      let Some(representative_key) = terminal_keys.get(&terminal_id).copied() else {
        continue;
      };
      let layout = if current_checkout && self.active_center_tab.as_ref() == Some(&representative) {
        Some(&self.center_layout)
      } else {
        self.center_layouts_by_tab.get(&representative)
      };
      let persisted = layout
        .and_then(|layout| layout.persisted_terminal_layout(&terminal_keys))
        .or_else(|| {
          CenterLayout::single(CenterSurface::from_tab(representative.clone()))
            .persisted_terminal_layout(&terminal_keys)
        });
      let Some(layout) = persisted else {
        continue;
      };
      collect_persisted_terminal_keys(&layout.root, &mut represented_terminals);
      layouts.push(PersistedTerminalLayoutEntry {
        representative: representative_key,
        layout,
      });
    }
    for terminal_id in terminal_ids {
      let Some(key) = terminal_keys.get(&terminal_id).copied() else {
        continue;
      };
      if represented_terminals.contains(&key) {
        continue;
      }
      let tab = CenterTab::terminal(terminal_id);
      let Some(layout) = CenterLayout::single(CenterSurface::from_tab(tab))
        .persisted_terminal_layout(&terminal_keys)
      else {
        continue;
      };
      represented_terminals.insert(key);
      layouts.push(PersistedTerminalLayoutEntry {
        representative: key,
        layout,
      });
    }

    let active_tab = if current_checkout {
      self.active_center_tab.as_ref()
    } else {
      self.center_active_tab_by_checkout.get(checkout_root)
    };
    let active_terminal = active_tab
      .and_then(CenterTab::terminal_id)
      .and_then(|terminal_id| terminal_keys.get(&terminal_id).copied());
    let state = PersistedTerminalWorkspace {
      version: TERMINAL_WORKSPACE_VERSION,
      terminals,
      layouts,
      active_terminal,
    };
    let project_root = self
      .terminal_views
      .values()
      .find(|terminal| terminal.checkout_root == checkout_root)
      .map(|terminal| terminal.project_root.as_path())
      .unwrap_or(checkout_root);
    match serde_json::to_string(&state) {
      Ok(state) => ConfigStore::persist_terminal_workspace(checkout_root, project_root, &state),
      Err(error) => log::warn!("Failed to serialize terminal workspace: {error}"),
    }
  }

  pub(super) fn terminal_for_tab(&self, tab: &CenterTab) -> Option<Entity<TerminalView>> {
    let terminal_id = tab.terminal_id()?;
    self
      .terminal_views
      .get(&terminal_id)
      .map(|terminal| terminal.view.clone())
  }

  pub(super) fn terminal_label(&self, tab: &CenterTab, cx: &App) -> String {
    let name = match tab.terminal_id() {
      Some(1) | None => "Terminal".to_string(),
      Some(id) => format!("Terminal {id}"),
    };
    let directory = self
      .terminal_for_tab(tab)
      .and_then(|terminal| terminal.read(cx).working_directory().map(Path::to_path_buf))
      .map(|path| {
        path
          .file_name()
          .unwrap_or(path.as_os_str())
          .to_string_lossy()
          .into_owned()
      });
    directory.map_or(name.clone(), |directory| format!("{name} - {directory}"))
  }

  pub(super) fn focus_terminal_tab(
    &self,
    tab: &CenterTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(terminal) = self.terminal_for_tab(tab) else {
      return;
    };
    let focus_handle = terminal.read(cx).focus_handle(cx);
    window.focus(&focus_handle, cx);
    window.on_next_frame(move |window, cx| {
      let focus_handle = terminal.read(cx).focus_handle(cx);
      window.focus(&focus_handle, cx);
    });
  }

  pub(super) fn clear_terminal_tab(&mut self, tab: &CenterTab) {
    if let Some(terminal_id) = tab.terminal_id() {
      self.terminal_views.remove(&terminal_id);
    }
  }

  pub(super) fn clear_terminals_for_project(&mut self, project_root: &Path) {
    let checkout_roots = self
      .terminal_views
      .values()
      .filter(|terminal| terminal.project_root == project_root)
      .map(|terminal| terminal.checkout_root.clone())
      .collect::<HashSet<_>>();
    for checkout_root in checkout_roots {
      ConfigStore::forget_terminal_workspace(&checkout_root);
    }
    ConfigStore::forget_terminal_workspaces_for_project(project_root);

    let terminal_tabs = self
      .terminal_views
      .iter()
      .filter(|(_, terminal)| terminal.project_root == project_root)
      .map(|(terminal_id, _)| CenterTab::terminal(*terminal_id))
      .collect::<Vec<_>>();
    if terminal_tabs.is_empty() {
      return;
    }

    self
      .terminal_views
      .retain(|_, terminal| terminal.project_root != project_root);
    self
      .center_tabs
      .retain(|tab| !terminal_tabs.iter().any(|terminal_tab| terminal_tab == tab));
    self
      .center_tab_history
      .retain(|tab| !terminal_tabs.iter().any(|terminal_tab| terminal_tab == tab));
    for tabs in self.center_tabs_by_checkout.values_mut() {
      tabs.retain(|tab| !terminal_tabs.iter().any(|terminal_tab| terminal_tab == tab));
    }
    self
      .center_active_tab_by_checkout
      .retain(|_, tab| !terminal_tabs.iter().any(|terminal_tab| terminal_tab == tab));
    self.center_layouts_by_tab.retain(|tab, layout| {
      !terminal_tabs.iter().any(|terminal_tab| terminal_tab == tab)
        && !layout
          .tabs()
          .iter()
          .any(|tab| terminal_tabs.iter().any(|terminal_tab| terminal_tab == tab))
    });
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::session_page::center_layout::CenterNode;
  use crate::session_page::test_support::{
    add_session_page_window_from_config, isolate_config_store_for_test,
  };
  use gpui::TestAppContext;

  #[gpui::test]
  fn terminal_tabs_cwds_and_split_layout_restore_in_fresh_page(cx: &mut TestAppContext) {
    isolate_config_store_for_test();
    let project = crate::test_support::TempDir::new("terminal-persistence");
    let first_directory = project.path.join("first");
    let second_directory = project.path.join("second");
    std::fs::create_dir_all(&first_directory).expect("create first directory");
    std::fs::create_dir_all(&second_directory).expect("create second directory");
    ConfigStore::persist_recent_project(&project.path);

    let (page, cx) = add_session_page_window_from_config(cx);
    page.update_in(cx, |page, window, cx| {
      page.new_terminal_tab(window, cx);
      page.new_terminal_tab(window, cx);

      let mut terminal_tabs = page
        .terminal_views
        .keys()
        .copied()
        .map(CenterTab::terminal)
        .collect::<Vec<_>>();
      terminal_tabs.sort_by_key(CenterTab::terminal_id);
      let first = terminal_tabs.first().cloned().expect("first terminal");
      let second = terminal_tabs.get(1).cloned().expect("second terminal");
      page
        .terminal_for_tab(&first)
        .expect("first terminal view")
        .update(cx, |terminal, cx| {
          terminal.set_working_directory(Some(first_directory.clone()), cx)
        });
      page
        .terminal_for_tab(&second)
        .expect("second terminal view")
        .update(cx, |terminal, cx| {
          terminal.set_working_directory(Some(second_directory.clone()), cx)
        });

      let pane_id = match page.center_layout.root() {
        CenterNode::Pane(pane) => pane.id(),
        CenterNode::Split(_) => panic!("new terminal should start in one pane"),
      };
      assert!(page.center_layout.split_pane(
        pane_id,
        CenterSurface::from_tab(first),
        CenterSplitDirection::Left,
      ));
      page.remember_center_layout_tab(second);
      page.persist_current_terminal_workspace(cx);
    });

    std::fs::remove_dir_all(&first_directory).expect("remove stale first directory");
    let restored = cx.update(|window, cx| cx.new(|cx| SessionPage::new(window, cx)));
    restored.update_in(cx, |page, window, cx| page.activate(window, cx));
    cx.run_until_parked();
    restored.read_with(cx, |page, cx| {
      assert_eq!(page.terminal_views.len(), 2);
      assert_eq!(page.center_layout.surface_count(), 2);
      let CenterNode::Split(split) = page.center_layout.root() else {
        panic!("terminal split should restore");
      };
      assert_eq!(split.direction(), CenterSplitDirection::Left);
      let working_directories = page
        .terminal_views
        .values()
        .filter_map(|terminal| {
          terminal
            .view
            .read(cx)
            .working_directory()
            .map(Path::to_path_buf)
        })
        .collect::<HashSet<_>>();
      assert_eq!(
        working_directories,
        HashSet::from([
          project.path.canonicalize().expect("canonical project"),
          second_directory,
        ])
      );
    });
  }
}
