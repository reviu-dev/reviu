use super::*;

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

    let terminal_id = self.next_terminal_id;
    self.next_terminal_id = self.next_terminal_id.saturating_add(1);
    let terminal = cx.new(|cx| TerminalView::new(Some(working_directory), cx));
    self.terminal_views.insert(
      terminal_id,
      TerminalPane {
        project_root,
        view: terminal,
      },
    );

    let tab = CenterTab::terminal(terminal_id);
    self.center = CenterView::Terminal;
    self.remember_center_tab(tab.clone());
    self.focus_terminal_tab(&tab, window, cx);
    cx.notify();
  }

  pub(super) fn terminal_for_tab(&self, tab: &CenterTab) -> Option<Entity<TerminalView>> {
    let terminal_id = tab.terminal_id()?;
    self
      .terminal_views
      .get(&terminal_id)
      .map(|terminal| terminal.view.clone())
  }

  pub(super) fn terminal_label(&self, tab: &CenterTab) -> String {
    match tab.terminal_id() {
      Some(1) | None => "Terminal".to_string(),
      Some(id) => format!("Terminal {id}"),
    }
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
