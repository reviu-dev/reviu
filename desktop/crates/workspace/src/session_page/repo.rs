//! Which repository the shell works on: switch, open and forget.

use super::*;

impl SessionPage {
  /// A session belongs to a repository: switching swaps the conversation set,
  /// the changes panel and the branch, so the agent is respawned on the new cwd.
  pub(super) fn set_selected_repo(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    if self.selected_repo.as_deref() == Some(repo_root.as_path()) {
      return Ok(());
    }
    // A folder that is not a repository would be remembered as the one to open
    // on the next launch, so it is refused before anything is stored.
    let Some(repo_root) = git::discover_repository_root(&repo_root) else {
      return Err("This folder is not a git repository.".into());
    };
    if self.selected_repo.as_deref() == Some(repo_root.as_path()) {
      return Ok(());
    }
    if self.agent_turn_in_flight(cx) {
      return Err("Wait for the agent to finish before switching repository.".into());
    }

    ConfigStore::persist_recent_repository(&repo_root);
    self.apply_selected_repo(Some(repo_root), window, cx);
    Ok(())
  }

  pub(super) fn apply_selected_repo(
    &mut self,
    repo_root: Option<PathBuf>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.selected_repo = repo_root.clone();
    self.close_diff(window, cx);
    self.center = CenterView::Conversation;
    self.editor = None;
    self.binary_preview = None;
    self.selected_file = None;
    self.open_file_task = None;
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    self.agent_review.clear();
    self.pending_review_export = None;
    self.branch_status = None;
    // Conversations are stored per repository, so the panel is rebuilt on the
    // next render with the new cwd and state directory.
    self.agent_chat_view = None;
    self.dock_panel.update(cx, |panel, cx| {
      panel.set_repo_root(repo_root, cx);
      panel.refresh(cx);
    });
    self.refresh_branch(cx);
    // Without a repository there is no branch refresh to publish from.
    if self.selected_repo.is_none() {
      self.publish_active_local_repo(cx);
    }
    cx.notify();
  }

  pub(super) fn forget_repository(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let forgetting_selected = self.selected_repo.as_deref() == Some(repo_root.as_path());
    if forgetting_selected && self.agent_turn_in_flight(cx) {
      return Err("Wait for the agent to finish before forgetting this repository.".into());
    }

    ConfigStore::forget_recent_repository(&repo_root);
    if !forgetting_selected {
      cx.notify();
      return Ok(());
    }

    let next_repo = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| repo.path)
      .find(|path| path != &repo_root);
    self.apply_selected_repo(next_repo, window, cx);
    Ok(())
  }

  pub(super) fn start_open_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
      files: false,
      directories: true,
      multiple: false,
      prompt: Some("Select a repository".into()),
    });

    cx.spawn_in(window, async move |this, cx| {
      let Ok(Ok(Some(paths))) = receiver.await else {
        return;
      };
      let Some(path) = paths.into_iter().next() else {
        return;
      };

      let _ = this.update_in(cx, |this, window, cx| {
        if let Err(error) = this.set_selected_repo(path, window, cx) {
          window.push_notification(Notification::warning(error), cx);
        }
      });
    })
    .detach();
  }
}
