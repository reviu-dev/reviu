//! Fetch, pull and push, with their notifications.

use super::*;

impl GitPage {
  pub(super) fn push_branch_switch_error_notification(
    &self,
    branch_name: &str,
    error: SharedString,
    cx: &mut Context<Self>,
  ) {
    let title = format!("Failed to switch to {branch_name}");
    self.push_git_error_notification_with_id::<GitBranchSwitchNotificationId>(title, error, cx);
  }

  pub(super) fn push_git_action_error_notification(
    &self,
    title: impl Into<SharedString>,
    error: SharedString,
    cx: &mut Context<Self>,
  ) {
    self.push_git_error_notification_with_id::<GitActionErrorNotificationId>(title, error, cx);
  }

  pub(super) fn push_git_action_error_notification_in_window(
    &self,
    title: impl Into<SharedString>,
    error: SharedString,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    window.push_notification(
      Notification::error(error)
        .id::<GitActionErrorNotificationId>()
        .title(title),
      cx,
    );
  }

  pub(super) fn push_git_action_success_notification(
    &self,
    message: SharedString,
    cx: &mut Context<Self>,
  ) {
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      window.push_notification(Notification::success(message), cx);
    });
  }

  pub(super) fn should_show_pro_push_hint(
    already_shown: bool,
    has_github_access: bool,
    has_github_branch_context: bool,
  ) -> bool {
    !already_shown && !has_github_access && has_github_branch_context
  }

  pub(super) fn maybe_show_pro_push_hint(&mut self, cx: &mut Context<Self>) {
    if !Self::should_show_pro_push_hint(
      self.pro_push_hint_shown,
      AuthStateStore::has_github_access(cx),
      self.github_branch_context(cx).is_some(),
    ) {
      return;
    }

    self.pro_push_hint_shown = true;
    crate::analytics::track_with(
      cx,
      "pro_teaser_shown",
      Some(serde_json::json!({ "source": "post_push_notification" })),
    );
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      window.push_notification(
        Notification::new()
          .id::<GitProPushHintNotificationId>()
          .title("Review pull requests in Reviu")
          .message(
            "Reviu Pro brings GitHub pull requests, reviews, and notifications into the app. 14-day free trial.",
          )
          .content(move |_, _, _cx| {
            div()
              .flex()
              .mt_3()
              .child(
                Button::new("git-pro-push-hint-open")
                  .primary()
                  .compact()
                  .small()
                  .label("See Reviu Pro")
                  .on_click(move |_, window, cx| {
                    crate::analytics::track_with(
                      cx,
                      "pro_teaser_clicked",
                      Some(serde_json::json!({ "source": "post_push_notification" })),
                    );
                    NavigationHistory::navigate("/billing", cx);
                    window.on_next_frame(|window, cx| {
                      window.remove_notification::<GitProPushHintNotificationId>(cx);
                    });
                  }),
              )
              .into_any_element()
          }),
        cx,
      );
    });
  }

  pub(super) fn push_git_error_notification_with_id<T: Sized + 'static>(
    &self,
    title: impl Into<SharedString>,
    error: SharedString,
    cx: &mut Context<Self>,
  ) {
    let title = title.into();
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      window.push_notification(Notification::error(error).id::<T>().title(title), cx);
    });
  }

  pub(super) fn pull_changes_action(
    &mut self,
    _: &crate::PullChanges,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };

    self.pull_repository(repo_root, cx);
    cx.stop_propagation();
  }

  pub(super) fn push_changes_shortcut_action(
    &mut self,
    _: &crate::PushChanges,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.push_changes_action(cx);
    cx.stop_propagation();
  }

  pub(super) fn force_push_changes_shortcut_action(
    &mut self,
    _: &crate::ForcePushChanges,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.force_push_changes_action(cx);
    cx.stop_propagation();
  }

  pub(super) fn fetch_action(
    &mut self,
    _: &gpui::ClickEvent,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.fetch_repository(repo_root, cx);
  }

  pub(super) fn fetch_repository(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    if self.fetch_in_progress {
      return;
    }
    self.add_git_breadcrumb("Fetch started", Map::new());
    crate::analytics::track(cx, "fetch_done");
    self.fetch_in_progress = true;
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || RepoCommand::Fetch.run(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.fetch_in_progress = false;
        match result {
          Ok(_) => {
            this.add_git_breadcrumb("Fetch succeeded", Map::new());
            this.push_git_action_success_notification("Fetched from remotes".into(), cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Fetch failed", data.clone());
            this.record_git_unexpected_error("git.fetch", error_message.as_str(), data);
            this.push_git_action_error_notification("Fetch failed", error_message.into(), cx);
          }
        }
        this.reload_status(cx);
        this.refresh_branches(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  pub(super) fn pull_repository(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    if self.push_pull_in_progress {
      return;
    }
    if !self.should_show_pull_palette_command() {
      return;
    }
    self.add_git_breadcrumb("Pull started", Map::new());
    self.push_pull_in_progress = true;
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || RepoCommand::Pull.run(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.push_pull_in_progress = false;
        match result {
          Ok(RepoCommandOutcome::UpToDate { .. }) => {
            this.add_git_breadcrumb("Pull already up to date", Map::new());
            this.push_git_action_success_notification("Already up to date".into(), cx);
          }
          Ok(_) => {
            this.add_git_breadcrumb("Pull succeeded", Map::new());
            this.push_git_action_success_notification("Pulled".into(), cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Pull failed", data.clone());
            this.record_git_unexpected_error("git.pull", error_message.as_str(), data);
            this.push_git_action_error_notification("Pull failed", error_message.into(), cx);
          }
        }
        this.reload_status(cx);
        this.refresh_branches(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  pub(super) fn push_changes_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.can_push {
      return;
    }

    self.add_git_breadcrumb("Push started", Map::new());
    crate::analytics::track(cx, "push_done");
    self.push_pull_in_progress = true;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || RepoCommand::Push.run(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.push_pull_in_progress = false;
        match result {
          Ok(_) => {
            this.force_push_after_rebase = false;
            this.add_git_breadcrumb("Push succeeded", Map::new());
            this.push_git_action_success_notification("Pushed".into(), cx);
            this.maybe_show_pro_push_hint(cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Push failed", data.clone());
            this.record_git_unexpected_error("git.push", error_message.as_str(), data);
            this.push_git_action_error_notification("Push failed", error_message.into(), cx);
          }
        }
        this.reload_status(cx);
      });
    });

    self.status_task = Some(task);
  }

  pub(super) fn force_push_changes_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.can_force_push {
      return;
    }

    self.add_git_breadcrumb("Force push started", Map::new());
    crate::analytics::track(cx, "force_push_done");
    self.push_pull_in_progress = true;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || RepoCommand::ForcePush.run(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.push_pull_in_progress = false;
        match result {
          Ok(_) => {
            this.force_push_after_rebase = false;
            this.add_git_breadcrumb("Force push succeeded", Map::new());
            this.push_git_action_success_notification("Force-pushed".into(), cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Force push failed", data.clone());
            this.record_git_unexpected_error("git.force_push", error_message.as_str(), data);
            this.push_git_action_error_notification("Force push failed", error_message.into(), cx);
          }
        }
        this.reload_status(cx);
      });
    });

    self.status_task = Some(task);
  }

  pub(super) fn should_publish_branch(
    branch_status: Option<&BranchStatus>,
    has_head_commit: bool,
  ) -> bool {
    has_head_commit
      && matches!(
        branch_status,
        Some(status) if !status.has_upstream && !Self::is_detached_head(Some(status))
      )
  }

  pub(super) fn should_publish_branch_and_create_pull_request(
    branch_status: Option<&BranchStatus>,
    has_unpublished_branch_commits: bool,
  ) -> bool {
    Self::should_publish_branch(branch_status, true) && has_unpublished_branch_commits
  }

  pub(super) fn push_action_label(
    branch_status: Option<&BranchStatus>,
    has_head_commit: bool,
  ) -> &'static str {
    if Self::should_publish_branch(branch_status, has_head_commit) {
      "Push (Publish branch)"
    } else {
      "Push"
    }
  }

  pub(super) fn push_flags(
    branch_status: Option<&BranchStatus>,
    has_head_commit: bool,
    force_push_after_rebase: bool,
  ) -> (bool, bool) {
    let Some(status) = branch_status else {
      return (false, false);
    };
    if Self::should_publish_branch(Some(status), has_head_commit) {
      return (true, false);
    }
    if !status.has_upstream {
      return (false, false);
    }
    if force_push_after_rebase && status.ahead > 0 {
      return (false, true);
    }
    let can_push = status.ahead > 0 && status.behind == 0;
    let can_force_push = status.ahead > 0 && status.behind > 0;
    (can_push, can_force_push)
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::*;
  use git2::Repository;
  use gpui::TestAppContext;

  #[test]
  fn pro_push_hint_shows_once_for_free_users_on_github_repos() {
    assert!(GitPage::should_show_pro_push_hint(false, false, true));
    assert!(!GitPage::should_show_pro_push_hint(true, false, true));
    assert!(!GitPage::should_show_pro_push_hint(false, true, true));
    assert!(!GitPage::should_show_pro_push_hint(false, false, false));
  }

  #[test]
  fn push_flags_respect_upstream_and_divergence() {
    let no_upstream = make_branch_status("main", 3, 0, false);
    assert_eq!(
      GitPage::push_flags(Some(&no_upstream), false, false),
      (false, false)
    );
    assert_eq!(
      GitPage::push_flags(Some(&no_upstream), true, false),
      (true, false)
    );

    let clean_ahead = make_branch_status("main", 2, 0, true);
    assert_eq!(
      GitPage::push_flags(Some(&clean_ahead), true, false),
      (true, false)
    );

    let diverged = make_branch_status("main", 1, 2, true);
    assert_eq!(
      GitPage::push_flags(Some(&diverged), true, false),
      (false, true)
    );

    let behind_only = make_branch_status("main", 0, 2, true);
    assert_eq!(
      GitPage::push_flags(Some(&behind_only), true, false),
      (false, false)
    );
  }

  #[test]
  fn push_action_label_mentions_publish_branch_without_upstream() {
    let no_upstream = make_branch_status("feature", 0, 0, false);
    assert_eq!(
      GitPage::push_action_label(Some(&no_upstream), true),
      "Push (Publish branch)"
    );
    assert_eq!(
      GitPage::push_action_label(Some(&no_upstream), false),
      "Push"
    );

    let tracked = make_branch_status("main", 1, 0, true);
    assert_eq!(GitPage::push_action_label(Some(&tracked), true), "Push");
    let detached = make_branch_status("HEAD", 0, 0, false);
    assert_eq!(GitPage::push_action_label(Some(&detached), true), "Push");
    assert_eq!(GitPage::push_action_label(None, true), "Push");
  }

  #[gpui::test]
  async fn fetch_repository_failure_shows_error_notification(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-fetch-failure-notification");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let missing_remote = repo.path.join("missing-remote.git");
    Repository::open(&repo.path)
      .expect("open repo")
      .remote("origin", missing_remote.to_str().expect("remote path utf8"))
      .expect("add origin remote");

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.fetch_repository(repo.path.clone(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
  }

  #[gpui::test]
  async fn push_changes_action_requires_selected_repo_and_push_capability(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-push-guards");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_push = false;
      this.push_changes_action(cx);
      assert!(this.status_task.is_none());
      assert!(!this.push_pull_in_progress);

      this.selected_repo = None;
      this.can_push = true;
      this.push_changes_action(cx);
      assert!(this.status_task.is_none());
      assert!(!this.push_pull_in_progress);
    });
  }

  #[gpui::test]
  async fn pull_changes_action_requires_selected_repo_and_respects_existing_sync(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-pull-shortcut-guards");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = None;
      this.pull_changes_action(&crate::PullChanges, window, cx);
      assert!(this.status_task.is_none());
      assert!(!this.push_pull_in_progress);

      this.selected_repo = Some(repo.path.clone());
      this.push_pull_in_progress = true;
      this.pull_changes_action(&crate::PullChanges, window, cx);
      assert!(this.status_task.is_none());
      assert!(this.push_pull_in_progress);
    });
  }

  #[gpui::test]
  async fn force_push_changes_action_requires_selected_repo_and_force_capability(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-force-push-guards");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_force_push = false;
      this.force_push_changes_action(cx);
      assert!(this.status_task.is_none());
      assert!(!this.push_pull_in_progress);

      this.selected_repo = None;
      this.can_force_push = true;
      this.force_push_changes_action(cx);
      assert!(this.status_task.is_none());
      assert!(!this.push_pull_in_progress);
    });
  }

  #[gpui::test]
  async fn push_changes_action_pushes_to_remote_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let source = TempRepo::init("git-page-push-success-source");
    let remote = TempBareRepo::init("git-page-push-success-remote");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");
    set_upstream(&source.path, &branch_name, &format!("origin/{branch_name}"));
    set_remote_head(&remote.path, &branch_name);

    let _ = commit_text_file(&source.path, rel_path, "v2-source\n", "source change");
    let expected_head = head_oid(&source.path);
    assert_ne!(
      remote_branch_oid(&remote.path, &branch_name),
      expected_head,
      "remote should be behind before push"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let push_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(source.path.clone());
      this.force_push_after_rebase = true;
      this.can_push = true;
      this.push_changes_action(cx);
      this.status_task.take().expect("push task")
    });
    assert!(git_page.read_with(cx, |this, _| this.push_pull_in_progress));
    push_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert_eq!(remote_branch_oid(&remote.path, &branch_name), expected_head);
    let status = current_branch_status(&source.path).expect("status after push");
    assert_eq!(status.ahead, 0);
    assert!(!git_page.read_with(cx, |this, _| this.force_push_after_rebase));
    assert!(!git_page.read_with(cx, |this, _| this.push_pull_in_progress));
  }

  #[gpui::test]
  async fn force_push_changes_action_force_pushes_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let source = TempRepo::init("git-page-force-push-source");
    let remote = TempBareRepo::init("git-page-force-push-remote");
    let peer = TempDir::new("git-page-force-push-peer");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");
    set_upstream(&source.path, &branch_name, &format!("origin/{branch_name}"));
    set_remote_head(&remote.path, &branch_name);

    let _ = Repository::clone(remote.path.to_str().expect("remote path utf8"), &peer.path)
      .expect("clone remote into peer");

    let _ = commit_text_file(&source.path, rel_path, "v2-source\n", "source change");
    let expected_head = head_oid(&source.path);

    let _ = commit_text_file(&peer.path, rel_path, "v2-peer\n", "peer change");
    push_branch_to_remote(&peer.path, &branch_name, "origin");

    let non_force = push(&source.path, false).err();
    assert!(non_force.is_some(), "non-force push should fail");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let force_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(source.path.clone());
      this.force_push_after_rebase = true;
      this.can_force_push = true;
      this.force_push_changes_action(cx);
      this.status_task.take().expect("force push task")
    });
    assert!(git_page.read_with(cx, |this, _| this.push_pull_in_progress));
    force_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert_eq!(remote_branch_oid(&remote.path, &branch_name), expected_head);
    assert!(!git_page.read_with(cx, |this, _| this.force_push_after_rebase));
    assert!(!git_page.read_with(cx, |this, _| this.push_pull_in_progress));
  }
}
