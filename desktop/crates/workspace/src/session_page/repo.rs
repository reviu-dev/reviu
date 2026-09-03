//! Which repository the shell works on: switch, open and forget.

use super::*;

impl SessionPage {
  /// Switches the FALLBACK repo: what the app stands on when no session is
  /// shown, and the repo reopened next launch. Running sessions are untouched
  /// (the repo is an attribute of each session, not a mode of the app).
  pub(super) fn set_fallback_repo(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    // A folder that is not a repository would be remembered as the one to open
    // on the next launch, so it is refused before anything is stored.
    let Some(repo_root) = git::discover_repository_root(&repo_root) else {
      return Err("This folder is not a git repository.".into());
    };
    let shown_elsewhere = self
      .agent_chat_view
      .as_ref()
      .is_some_and(|panel| panel.read(cx).repo_root() != repo_root.as_path());
    if self.fallback_repo.as_deref() == Some(repo_root.as_path()) && !shown_elsewhere {
      return Ok(());
    }

    if self.editor_is_dirty(cx) && self.target_checkout_differs_from_editor(&repo_root, cx) {
      self.open_unsaved_editor_dialog(
        UnsavedEditorAction::SetFallbackRepo { repo_root },
        window,
        cx,
      );
      return Ok(());
    }
    self.set_fallback_repo_without_unsaved_prompt(repo_root, window, cx)
  }

  pub(super) fn set_fallback_repo_without_unsaved_prompt(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    ConfigStore::persist_recent_repository(&repo_root);
    self.apply_fallback_repo(Some(repo_root.clone()), window, cx);
    // Switching repository means going to work there: reopen the session you
    // left active in it, or open a blank one so the screen, the git surfaces
    // and the New Session button all agree on where you are.
    let resume = self
      .chat_store
      .as_ref()
      .and_then(|store| store.read(cx).active_meta())
      .map(|meta| meta.id);
    match resume {
      Some(id) => {
        let already_shown = self
          .agent_chat_view
          .as_ref()
          .is_some_and(|panel| panel.read(cx).current_conversation().id == id);
        if !already_shown {
          self.select_session(&id, window, cx);
        }
      }
      None => self.new_session_in_without_unsaved_prompt(repo_root, window, cx),
    }
    Ok(())
  }

  #[doc(hidden)]
  pub fn open_repository_for_driver(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    self.set_fallback_repo(repo_root, window, cx)
  }

  pub(super) fn apply_fallback_repo(
    &mut self,
    repo_root: Option<PathBuf>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.fallback_repo = repo_root;
    // The fallback repo's store takes over; its first visit sweeps its
    // orphaned worktrees.
    self.chat_store = None;
    if self.fallback_repo.is_some()
      && let Some(evicted_repo) = self.ensure_chat_store(cx)
    {
      self.push_repo_hidden_notification(&evicted_repo, window, cx);
    }
    self.refresh_session_list(cx);
    // With no session on screen the git surfaces follow the fallback repo;
    // with one, they stay on the session's checkout and this is a no-op.
    self.sync_active_checkout(window, cx);
    cx.notify();
  }

  pub(super) fn forget_repository(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    if self.agent_turn_in_flight_for_repo(&repo_root, cx) {
      return Err("Wait for the agent to finish before forgetting this repository.".into());
    }
    let forgetting_active_checkout = self
      .agent_chat_view
      .as_ref()
      .is_some_and(|panel| panel.read(cx).repo_root() == repo_root.as_path())
      || self.fallback_repo.as_deref() == Some(repo_root.as_path());
    if self.editor_is_dirty(cx) && forgetting_active_checkout {
      self.open_unsaved_editor_dialog(
        UnsavedEditorAction::ForgetRepository { repo_root },
        window,
        cx,
      );
      return Ok(());
    }
    self.forget_repository_without_unsaved_prompt(repo_root, window, cx)
  }

  pub(super) fn forget_repository_without_unsaved_prompt(
    &mut self,
    repo_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    if self.agent_turn_in_flight_for_repo(&repo_root, cx) {
      return Err("Wait for the agent to finish before forgetting this repository.".into());
    }

    ConfigStore::forget_recent_repository(&repo_root);
    // Its sessions stop here (conversations stay on disk); a forgotten repo
    // keeps nothing running.
    let mut doomed_panels: Vec<Entity<AgentChatPanel>> = Vec::new();
    self.background_chat_panels.retain(|(_, panel)| {
      if panel.read(cx).repo_root() == repo_root.as_path() {
        doomed_panels.push(panel.clone());
        false
      } else {
        true
      }
    });
    let active_was_doomed = self
      .agent_chat_view
      .as_ref()
      .is_some_and(|panel| panel.read(cx).repo_root() == repo_root.as_path());
    if active_was_doomed && let Some(panel) = self.agent_chat_view.take() {
      doomed_panels.push(panel);
    }
    for panel in doomed_panels {
      panel.update(cx, |panel, cx| panel.persist_now(cx));
    }
    self.conversation_hub.drop_store(&repo_root, cx);
    self.swept_repos.remove(&repo_root);

    let forgetting_fallback = self.fallback_repo.as_deref() == Some(repo_root.as_path());
    if !forgetting_fallback {
      let _ = self.backfill_session_sidebar_repository(cx);
      // The shown session may have gone with the repo: a fresh fallback-repo
      // session takes over so the centre and the git surfaces never point at
      // a dead checkout.
      if active_was_doomed {
        let view = self.build_fallback_chat_panel(None, window, cx);
        view.update(cx, |panel, _| panel.set_active_conversation(true));
        self.agent_chat_view = Some(view);
        self.sync_active_checkout(window, cx);
      }
      self.refresh_session_list(cx);
      cx.notify();
      return Ok(());
    }

    let next_repo = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| repo.path)
      .find(|path| path != &repo_root);
    self.apply_fallback_repo(next_repo, window, cx);
    if self.backfill_session_sidebar_repository(cx) {
      self.refresh_session_list(cx);
      cx.notify();
    }
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
        if let Err(error) = this.set_fallback_repo(path, window, cx) {
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
  async fn switching_repository_waits_for_dirty_file_choice(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-dirty-repo-switch");
    let other = TempRepo::init("session-page-dirty-repo-switch-b");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    let editor = page
      .read_with(cx, |page, _| page.editor.clone())
      .expect("editor");
    editor.update(cx, |editor, cx| {
      editor.document.update(cx, |document, cx| {
        document.replace_all("unsaved\n", cx);
      });
      editor.is_dirty = true;
    });

    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("set repo");
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    assert!(
      cx.debug_bounds(UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR)
        .is_some()
    );
    page.read_with(cx, |page, _| {
      assert_eq!(page.fallback_repo.as_deref(), Some(repo.path.as_path()));
    });

    page.update_in(cx, |page, window, cx| {
      page.discard_unsaved_editor_for_test(
        UnsavedEditorAction::SetFallbackRepo {
          repo_root: other.path.clone(),
        },
        window,
        cx,
      );
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert_eq!(page.fallback_repo.as_deref(), Some(other.path.as_path()));
    });
  }

  #[gpui::test]
  async fn a_batch_on_disk_reaches_the_panel_without_opening_a_file(cx: &mut TestAppContext) {
    use crate::agent_review::{LocalAgentReviewComment, LocalAgentReviewCommentState};
    use editor::ReviewCommentSide;

    let repo = TempRepo::init("session-page-review-reload");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let state_dir = std::env::temp_dir().join(format!(
      "reviu-review-reload-{}-{:?}",
      std::process::id(),
      std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&state_dir);
    // A previous run of the app left a batch behind.
    write_review(
      &review_path_for_repo(&state_dir, &repo.path),
      &[LocalAgentReviewComment {
        id: 4,
        in_reply_to_id: None,
        path: PathBuf::from("README.md"),
        line: 0,
        side: ReviewCommentSide::Right,
        start_line: None,
        start_side: None,
        body: std::sync::Arc::from("from the last run"),
        original_start_line: Some(1),
        original_lines: vec!["v1".to_string()],
        state: LocalAgentReviewCommentState::Draft,
      }],
      5,
    );

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    page.update(cx, |page, cx| {
      page.review_state_dir = Some(state_dir.clone());
      page.reload_review_for_repo(cx);
    });

    page.read_with(cx, |page, cx| {
      // The rail badge reads the page, so it was right even while the panel
      // stayed empty: the panel needs its own sync after a load.
      assert_eq!(page.draft_review_comment_count(), 1);
      let rows = page
        .dock_panel
        .read(cx)
        .review_list
        .read(cx)
        .comments(ReviewSection::Agent);
      assert_eq!(rows.len(), 1);
      assert_eq!(rows[0].excerpt, "from the last run");
    });

    let _ = std::fs::remove_dir_all(&state_dir);
  }

  #[gpui::test]
  async fn each_repository_keeps_its_own_batch_across_switches(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-review-persist-a");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    let other = TempRepo::init("session-page-review-persist-b");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let state_dir = std::env::temp_dir().join(format!(
      "reviu-review-persist-{}-{:?}",
      std::process::id(),
      std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&state_dir);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    // The page mounted before this test could name a directory, so point it
    // there by hand; every later switch resolves it on its own.
    page.update(cx, |page, _| {
      page.review_state_dir = Some(state_dir.clone());
      page.review_store_path = review_store_path_for(Some(&repo.path), Some(&state_dir));
    });

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "keep this"), window, cx);
    });
    let first_id = page.read_with(cx, |page, _| page.agent_review.all()[0].id);

    // Drive checkout sync directly so this review test does not spawn an agent process.
    page.update_in(cx, |page, window, cx| {
      page.apply_fallback_repo(Some(other.path.clone()), window, cx);
    });
    page.read_with(cx, |page, cx| {
      assert!(page.agent_review.all().is_empty());
      assert!(
        page
          .dock_panel
          .read(cx)
          .review_list
          .read(cx)
          .comments(ReviewSection::Agent)
          .is_empty(),
        "the panel must not keep the rows of the repository we left"
      );
    });

    page.update_in(cx, |page, window, cx| {
      page.apply_fallback_repo(Some(repo.path.clone()), window, cx);
    });
    page.read_with(cx, |page, cx| {
      let comments = page.agent_review.all();
      assert_eq!(comments.len(), 1);
      assert_eq!(comments[0].body.as_ref(), "keep this");
      assert_eq!(comments[0].id, first_id);
      assert_eq!(
        page
          .dock_panel
          .read(cx)
          .review_list
          .read(cx)
          .comments(ReviewSection::Agent)
          .len(),
        1
      );
    });

    // A new comment must not take an id the reloaded batch already holds.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "and this"), window, cx);
    });
    page.read_with(cx, |page, _| {
      let ids = page
        .agent_review
        .all()
        .iter()
        .map(|comment| comment.id)
        .collect::<Vec<_>>();
      assert_eq!(ids.len(), 2);
      assert_ne!(ids[0], ids[1]);
    });

    // Discarding takes the file with it: nothing comes back on the next visit.
    page.update(cx, |page, cx| page.discard_agent_review(cx));
    page.update_in(cx, |page, window, cx| {
      page.apply_fallback_repo(Some(other.path.clone()), window, cx);
    });
    page.update_in(cx, |page, window, cx| {
      page.apply_fallback_repo(Some(repo.path.clone()), window, cx);
    });
    page.read_with(cx, |page, _| assert!(page.agent_review.all().is_empty()));

    let _ = std::fs::remove_dir_all(&state_dir);
  }

  #[gpui::test]
  async fn switching_repository_takes_you_there(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-switch-from");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    let other = TempRepo::init("session-page-switch-to");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, window, cx| {
      page.create_agent_review_comment(create_request(0, "keep this"), window, cx);
    });

    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("switch repository");
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert_eq!(page.fallback_repo.as_deref(), Some(other.path.as_path()));
      // No session was active there: a blank one opens so the screen, the
      // git surfaces and New Session all agree on where you are.
      let panel = page.agent_chat_view.as_ref().expect("a session is shown");
      assert_eq!(panel.read(cx).repo_root(), other.path.as_path());
      assert!(!panel.read(cx).has_persistable_content());
      // The open diff and its draft comments belong to the previous repo.
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.editor.is_none());
      assert!(page.selected_file.is_none());
      assert!(page.agent_review.is_empty());
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
    // Never cleared: the override is process-wide and other tests mount panels.
    agent_chat_panel::set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
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
        .set_fallback_repo(other.path.clone(), window, cx)
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
  async fn switching_fallback_keeps_running_sessions_but_forgetting_their_repo_waits(
    cx: &mut TestAppContext,
  ) {
    let repo = TempRepo::init("session-page-turn-guard-from");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-turn-guard-to");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();
    ConfigStore::persist_recent_repository(&repo.path);
    page.update(cx, |page, _| page.pretend_agent_turn_in_flight = true);

    // The repo is an attribute of the session, not a mode: pointing the
    // fallback elsewhere never interrupts a running agent.
    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(other.path.clone(), window, cx)
        .expect("switching fallback is always allowed")
    });
    page.read_with(cx, |page, _| {
      assert_eq!(page.fallback_repo.as_deref(), Some(other.path.as_path()));
    });

    // Forgetting a repo tears its sessions down, so a running one refuses.
    let forget = page.update_in(cx, |page, window, cx| {
      page.forget_repository(repo.path.clone(), window, cx)
    });
    assert_eq!(
      forget
        .expect_err("forgetting a repo with a running agent is refused")
        .as_ref(),
      "Wait for the agent to finish before forgetting this repository."
    );
    assert!(
      ConfigStore::load_recent_repositories()
        .iter()
        .any(|recent| recent.path == repo.path),
      "a refused forget must not drop the repository from the list"
    );

    // The turn ends: the forget goes through.
    page.update(cx, |page, _| page.pretend_agent_turn_in_flight = false);
    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository(repo.path.clone(), window, cx)
        .expect("forgetting once the agent is idle")
    });
    assert!(
      !ConfigStore::load_recent_repositories()
        .iter()
        .any(|recent| recent.path == repo.path)
    );
  }

  #[gpui::test]
  async fn switching_to_the_same_repository_is_a_noop(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-switch-same");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(
        PathBuf::from("README.md"),
        None,
        OpenIntent::Open,
        window,
        cx,
      );
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page
        .set_fallback_repo(repo.path.clone(), window, cx)
        .expect("same repository");
    });

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert!(page.editor.is_some());
    });
  }

  #[gpui::test]
  async fn forgetting_the_fallback_repository_falls_back_to_the_next_recent_one(
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
      assert_eq!(page.fallback_repo.as_deref(), Some(other.path.as_path()));
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
        page.fallback_repo.is_none(),
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
      page.set_fallback_repo(plain_folder.clone(), window, cx)
    });
    assert_eq!(
      refused.expect_err("a plain folder is refused").as_ref(),
      "This folder is not a git repository."
    );
    page.read_with(cx, |page, _| {
      assert_eq!(
        page.fallback_repo.as_deref(),
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
        page.set_fallback_repo(nested_other.clone(), window, cx)
      })
      .expect("a folder inside a repository is accepted");
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      let selected = page.fallback_repo.clone().expect("selected repository");
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

    page.read_with(cx, |page, _| assert!(page.fallback_repo.is_none()));
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
      assert_eq!(page.fallback_repo.as_deref(), Some(repo.path.as_path()));
    });
  }

  #[gpui::test]
  async fn removing_a_visible_repository_backfills_from_hidden_recents(cx: &mut TestAppContext) {
    agent_chat_panel::set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
    let repos = (0..=crate::conversation_hub::MAX_TRACKED_REPOS)
      .map(|index| {
        let repo = TempRepo::init(&format!("session-page-forget-backfill-{index}"));
        commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
        repo
      })
      .collect::<Vec<_>>();
    let (page, cx) = add_session_page_window(repos[0].path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      ConfigStore::persist_recent_repository(&repos[0].path);
      page.apply_fallback_repo(Some(repos[0].path.clone()), window, cx);
      for repo in repos.iter().skip(1) {
        page
          .set_fallback_repo(repo.path.clone(), window, cx)
          .expect("switch repository");
      }
    });
    cx.run_until_parked();

    let (hidden_repo, removed_repo) = page.read_with(cx, |page, cx| {
      let visible = page.session_list.read(cx).section_order_for_test().to_vec();
      assert_eq!(visible.len(), crate::conversation_hub::MAX_TRACKED_REPOS);
      let recent = ConfigStore::load_recent_repositories()
        .into_iter()
        .map(|repo| repo.path)
        .collect::<Vec<_>>();
      assert_eq!(recent.len(), crate::conversation_hub::MAX_TRACKED_REPOS + 1);
      let hidden = recent
        .iter()
        .find(|repo| !visible.contains(repo))
        .expect("a hidden recent repository")
        .clone();
      let fallback = page.fallback_repo.as_ref().expect("fallback repo");
      let removed = visible
        .into_iter()
        .find(|repo| repo != fallback)
        .expect("visible non-fallback repo");
      (hidden, removed)
    });

    page.update_in(cx, |page, window, cx| {
      page
        .forget_repository_without_unsaved_prompt(removed_repo.clone(), window, cx)
        .expect("remove repository");
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let visible = page.session_list.read(cx).section_order_for_test();
      assert_eq!(visible.len(), crate::conversation_hub::MAX_TRACKED_REPOS);
      assert!(visible.contains(&hidden_repo));
      assert!(!visible.contains(&removed_repo));
    });
    let recent = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| repo.path)
      .collect::<Vec<_>>();
    assert!(recent.contains(&hidden_repo));
    assert!(!recent.contains(&removed_repo));
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
      assert_eq!(page.fallback_repo.as_deref(), Some(repo.path.as_path()));
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
      assert!(page.fallback_repo.is_none());
      assert!(page.dock_panel.read(cx).repo_root().is_none());
      assert!(page.repo_snapshot.read(cx).branch_status().is_none());
    });
  }
}
