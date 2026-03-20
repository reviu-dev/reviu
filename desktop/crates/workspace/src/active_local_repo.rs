use std::{
  path::PathBuf,
  sync::{Arc, Mutex},
};

use gpui::{App, Global};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveLocalRepo {
  pub repo_root: PathBuf,
  pub github_owner: Option<String>,
  pub github_repo: Option<String>,
  pub current_branch: Option<String>,
  pub head_sha: Option<String>,
  pub has_uncommitted_changes: bool,
}

#[derive(Clone, Default)]
pub struct ActiveLocalRepoStore {
  state: Arc<Mutex<Option<ActiveLocalRepo>>>,
}

impl Global for ActiveLocalRepoStore {}

impl ActiveLocalRepoStore {
  pub fn get(cx: &App) -> Option<ActiveLocalRepo> {
    cx.global::<Self>()
      .state
      .lock()
      .ok()
      .and_then(|state| state.clone())
  }

  pub fn set(cx: &mut App, repo: Option<ActiveLocalRepo>) {
    if let Ok(mut state) = cx.global::<Self>().state.lock() {
      *state = repo;
    }
  }
}
