use super::*;

fn terminal_relative_path(checkout_root: &Path, path: &Path) -> Option<PathBuf> {
  let path = path.canonicalize().ok()?;
  if !path.is_file() {
    return None;
  }
  path.strip_prefix(checkout_root).ok().map(Path::to_path_buf)
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
    self.remember_center_tab(tab.clone(), cx);
    self.focus_terminal_tab(&tab, window, cx);
    cx.notify();
  }

  pub(super) fn create_terminal_tab(
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
          this.persist_center_workspace_for_terminal(terminal_id, cx);
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
      ConfigStore::forget_center_workspace(&checkout_root);
    }
    ConfigStore::forget_center_workspaces_for_project(project_root);

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
