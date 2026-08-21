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

  #[doc(hidden)]
  pub fn open_repository_for_driver(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    self.set_selected_repo(repo_root, window, cx)
  }

  pub(super) fn apply_selected_repo(
    &mut self,
    repo_root: Option<PathBuf>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let should_rebuild_agent = self.agent_chat_view.is_some();
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
    self.repo_snapshot.update(cx, |snapshot, cx| {
      snapshot.set_repo_root(repo_root.clone(), cx)
    });
    // Conversations are stored per repository, so the panel is rebuilt with
    // the new cwd and state directory when the shell is already active.
    self.agent_chat_view = None;
    self.sync_session_list(cx);
    if should_rebuild_agent {
      self.ensure_agent_chat_view(window, cx);
    }
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

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;

  #[gpui::test]
  async fn switching_repository_resets_the_shell_state(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-switch-from");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    let other = TempRepo::init("session-page-switch-to");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, _window, cx| {
      page.create_agent_review_comment(create_request(0, "keep this"), cx);
    });

    page.update_in(cx, |page, window, cx| {
      page
        .set_selected_repo(other.path.clone(), window, cx)
        .expect("switch repository");
    });

    page.read_with(cx, |page, cx| {
      assert_eq!(page.selected_repo.as_deref(), Some(other.path.as_path()));
      // The open diff and its draft comments belong to the previous repository.
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.editor.is_none());
      assert!(page.selected_file.is_none());
      assert!(page.agent_review.is_empty());
      // This test never activated the agent panel, so switching does not start it.
      assert!(page.agent_chat_view.is_none());
      assert_eq!(
        page.dock_panel.read(cx).repo_root(),
        Some(other.path.as_path())
      );
    });
  }

  #[gpui::test]
  async fn switching_repository_reloads_sessions_when_the_agent_panel_is_active(
    cx: &mut TestAppContext,
  ) {
    let repo = TempRepo::init("session-page-active-switch-from");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-active-switch-to");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let state_dir = agent_chat_state_dir()
      .map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &other.path))
      .expect("agent chat state dir");
    let _ = std::fs::remove_dir_all(&state_dir);
    std::fs::create_dir_all(&state_dir).expect("create agent chat state dir");
    let index = serde_json::json!({
      "version": 1,
      "conversations": [{
        "id": "session-in-other-repo",
        "started_at_secs": 1,
        "updated_at_secs": 2,
        "title": "Other repo session",
        "message_count": 1,
        "session_id": null,
        "preview": "hello"
      }]
    });
    std::fs::write(state_dir.join("index.json"), index.to_string()).expect("write session index");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update_in(cx, |page, window, cx| page.activate(window, cx));
    page.read_with(cx, |page, _| assert!(page.agent_chat_view.is_some()));

    page.update_in(cx, |page, window, cx| {
      page
        .set_selected_repo(other.path.clone(), window, cx)
        .expect("switch repository");
    });

    page.read_with(cx, |page, cx| {
      assert!(page.agent_chat_view.is_some());
      assert_eq!(
        page.session_list.read(cx).conversation_ids(),
        vec!["session-in-other-repo".to_string()]
      );
    });

    let _ = std::fs::remove_dir_all(&state_dir);
  }

  #[gpui::test]
  async fn a_repository_cannot_move_under_a_running_agent(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-turn-guard-from");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-turn-guard-to");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    ConfigStore::persist_recent_repository(&repo.path);
    page.update(cx, |page, _| page.pretend_agent_turn_in_flight = true);

    let switch = page.update_in(cx, |page, window, cx| {
      page.set_selected_repo(other.path.clone(), window, cx)
    });
    assert_eq!(
      switch.expect_err("switching is refused mid-turn").as_ref(),
      "Wait for the agent to finish before switching repository."
    );

    let forget = page.update_in(cx, |page, window, cx| {
      page.forget_repository(repo.path.clone(), window, cx)
    });
    assert_eq!(
      forget
        .expect_err("forgetting the open repository is refused mid-turn")
        .as_ref(),
      "Wait for the agent to finish before forgetting this repository."
    );

    // The shell stayed where it was.
    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_repo.as_deref(), Some(repo.path.as_path()));
    });
    assert!(
      ConfigStore::load_recent_repositories()
        .iter()
        .any(|recent| recent.path == repo.path),
      "a refused forget must not drop the repository from the list"
    );

    // The turn ends: the switch goes through.
    page.update(cx, |page, _| page.pretend_agent_turn_in_flight = false);
    page.update_in(cx, |page, window, cx| {
      page
        .set_selected_repo(other.path.clone(), window, cx)
        .expect("switching once the agent is idle")
    });
    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_repo.as_deref(), Some(other.path.as_path()));
    });
  }

  #[gpui::test]
  async fn switching_to_the_same_repository_is_a_noop(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-switch-same");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page
        .set_selected_repo(repo.path.clone(), window, cx)
        .expect("same repository");
    });

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert!(page.editor.is_some());
    });
  }

  #[gpui::test]
  async fn forgetting_the_selected_repository_falls_back_to_the_next_recent_one(
    cx: &mut TestAppContext,
  ) {
    let repo = TempRepo::init("session-page-forget-selected");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-forget-fallback");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    ConfigStore::persist_recent_repository(&other.path);
    ConfigStore::persist_recent_repository(&repo.path);

    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository(repo.path.clone(), window, cx)
        .expect("forget repository");
    });

    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_repo.as_deref(), Some(other.path.as_path()));
    });
    assert!(
      !ConfigStore::load_recent_repositories()
        .iter()
        .any(|recent| recent.path == repo.path)
    );
  }

  #[gpui::test]
  async fn picking_a_folder_that_is_not_a_repository_leaves_the_shell_empty(
    cx: &mut TestAppContext,
  ) {
    let plain_folder = crate::test_support::temp_path("session-page-picker-not-a-repo");
    std::fs::create_dir_all(&plain_folder).expect("create plain folder");

    let (page, cx) = add_session_page_window_without_repo(cx);
    cx.run_until_parked();

    let row = cx
      .debug_bounds(OPEN_REPOSITORY_ROW_DEBUG_SELECTOR)
      .expect("the sidebar offers to open a repository");
    let picked = plain_folder.clone();
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.simulate_path_prompt_response(move |_| Some(vec![picked]));
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert!(
        page.selected_repo.is_none(),
        "a folder without a repository is not selected"
      );
    });
    assert!(
      cx.debug_bounds(OPEN_REPOSITORY_ROW_DEBUG_SELECTOR)
        .is_some(),
      "the sidebar still asks for a repository"
    );
    assert!(
      ConfigStore::load_recent_repositories().is_empty(),
      "and nothing was remembered"
    );

    let _ = std::fs::remove_dir_all(&plain_folder);
  }

  #[gpui::test]
  async fn a_folder_without_a_repository_is_refused_and_not_remembered(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-repo-validation");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let plain_folder = crate::test_support::temp_path("session-page-not-a-repo");
    std::fs::create_dir_all(&plain_folder).expect("create plain folder");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    let refused = page.update_in(cx, |page, window, cx| {
      page.set_selected_repo(plain_folder.clone(), window, cx)
    });
    assert_eq!(
      refused.expect_err("a plain folder is refused").as_ref(),
      "This folder is not a git repository."
    );
    page.read_with(cx, |page, _| {
      assert_eq!(
        page.selected_repo.as_deref(),
        Some(repo.path.as_path()),
        "the shell stays on the repository it had"
      );
    });
    assert!(
      !ConfigStore::load_recent_repositories()
        .iter()
        .any(|recent| recent.path == plain_folder),
      "a refused folder must not come back as the repository to open next launch"
    );

    // A directory inside a repository is accepted, as its root.
    let nested = repo.path.join("src/deep");
    std::fs::create_dir_all(&nested).expect("create nested dirs");
    let other = TempRepo::init("session-page-repo-validation-other");
    commit_text_file(&other.path, Path::new("README.md"), "v1\n", "initial");
    let nested_other = other.path.join("src");
    std::fs::create_dir_all(&nested_other).expect("create nested dir");

    page
      .update_in(cx, |page, window, cx| {
        page.set_selected_repo(nested_other.clone(), window, cx)
      })
      .expect("a folder inside a repository is accepted");
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      let selected = page.selected_repo.clone().expect("selected repository");
      assert_eq!(
        selected.canonicalize().expect("canonical selection"),
        other.path.canonicalize().expect("canonical repo"),
        "the root is selected, not the folder that was picked"
      );
    });

    let _ = std::fs::remove_dir_all(&plain_folder);
  }

  #[gpui::test]
  async fn forgetting_the_only_repository_brings_the_open_row_back(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-forget-only");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    ConfigStore::persist_recent_repository(&repo.path);
    assert!(cx.debug_bounds(REPO_CONTEXT_DEBUG_SELECTOR).is_some());

    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository(repo.path.clone(), window, cx)
        .expect("forget repository");
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(page.selected_repo.is_none()));
    assert!(
      cx.debug_bounds(OPEN_REPOSITORY_ROW_DEBUG_SELECTOR)
        .is_some(),
      "forgetting the last repository must not leave the shell without a way back"
    );
  }

  #[gpui::test]
  async fn switching_to_a_repository_that_moved_reports_an_error(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-switch-missing");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let missing = std::env::temp_dir().join("reviu-session-page-not-a-repo");
    let _ = std::fs::remove_dir_all(&missing);

    let error = page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(
          CommandPaletteAction::SwitchRepository(ui::CommandPaletteRepository {
            path: missing.to_string_lossy().to_string().into(),
          }),
          window,
          cx,
        )
        .expect_err("missing repository")
    });

    assert!(error.contains("Repository not found"), "{error}");
    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_repo.as_deref(), Some(repo.path.as_path()));
    });
  }

  #[gpui::test]
  async fn forgetting_another_repository_keeps_the_open_one(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-forget-other");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-forget-other-recent");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    ConfigStore::persist_recent_repository(&repo.path);
    ConfigStore::persist_recent_repository(&other.path);

    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository(other.path.clone(), window, cx)
        .expect("forget repository");
    });

    page.read_with(cx, |page, _| {
      assert_eq!(page.selected_repo.as_deref(), Some(repo.path.as_path()));
    });
    let recents = ConfigStore::load_recent_repositories();
    assert!(recents.iter().any(|recent| recent.path == repo.path));
    assert!(!recents.iter().any(|recent| recent.path == other.path));
  }

  #[gpui::test]
  async fn forgetting_the_last_repository_clears_the_selection(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-forget-last");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    ConfigStore::persist_recent_repository(&repo.path);

    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository(repo.path.clone(), window, cx)
        .expect("forget repository");
    });

    page.read_with(cx, |page, cx| {
      assert!(page.selected_repo.is_none());
      assert!(page.dock_panel.read(cx).repo_root().is_none());
      assert!(page.repo_snapshot.read(cx).branch_status().is_none());
    });
  }

  #[gpui::test(iterations = 10)]
  async fn switching_repository_mid_publish_does_not_publish_the_old_one(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-active-repo-race");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-active-repo-race-other");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // The read is in flight when the user switches repository.
    let publish = page.update(cx, |page, cx| {
      page.publish_active_local_repo(cx);
      page._active_repo_task.take().expect("publish task")
    });
    page.update(cx, |page, _| page.selected_repo = Some(other.path.clone()));
    publish.await;
    cx.run_until_parked();

    assert_eq!(
      cx.update(|_, cx| crate::active_local_repo::ActiveLocalRepoStore::get(cx)),
      None,
      "the pull request page must never be pointed at the repository we just left"
    );
  }
}
