use std::path::Path;

use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{BranchType, CherrypickOptions, Repository, Signature};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchKind {
  Local,
  Remote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchRef {
  pub name: String,
  pub kind: BranchKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchStatus {
  pub name: String,
  pub ahead: usize,
  pub behind: usize,
  pub has_upstream: bool,
}

pub fn list_branches(repo_root: &Path) -> Result<Vec<BranchRef>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut branches = Vec::new();

  for branch in repo.branches(None)? {
    let (branch, kind) = branch?;
    let name = branch.name()?.unwrap_or("").to_string();
    if name.is_empty() {
      continue;
    }

    let kind = match kind {
      BranchType::Local => BranchKind::Local,
      BranchType::Remote => {
        if name.ends_with("/HEAD") {
          continue;
        }
        BranchKind::Remote
      }
    };

    branches.push(BranchRef { name, kind });
  }

  branches.sort_by(|a, b| match (a.kind, b.kind) {
    (BranchKind::Local, BranchKind::Remote) => std::cmp::Ordering::Less,
    (BranchKind::Remote, BranchKind::Local) => std::cmp::Ordering::Greater,
    _ => a.name.cmp(&b.name),
  });

  Ok(branches)
}

pub fn current_branch_status(repo_root: &Path) -> Result<BranchStatus> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head = repo.head()?;
  let name = head.shorthand().unwrap_or("HEAD").to_string();

  let mut ahead = 0;
  let mut behind = 0;
  let mut has_upstream = false;

  if head.is_branch()
    && let Ok(branch) = repo.find_branch(&name, BranchType::Local)
    && let Ok(upstream) = branch.upstream()
  {
    has_upstream = true;
    if let (Some(local_oid), Some(upstream_oid)) = (branch.get().target(), upstream.get().target())
    {
      let (a, b) = repo.graph_ahead_behind(local_oid, upstream_oid)?;
      ahead = a;
      behind = b;
    }
  }

  Ok(BranchStatus {
    name,
    ahead,
    behind,
    has_upstream,
  })
}

pub fn switch_branch(repo_root: &Path, branch: &BranchRef) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;

  let (refname, checkout_target) = match branch.kind {
    BranchKind::Local => (
      format!("refs/heads/{}", branch.name),
      format!("refs/heads/{}", branch.name),
    ),
    BranchKind::Remote => {
      let remote_ref = format!("refs/remotes/{}", branch.name);
      let oid = repo
        .refname_to_id(&remote_ref)
        .with_context(|| format!("resolve remote branch {:?}", branch.name))?;
      let commit = repo.find_commit(oid)?;

      let local_name = branch
        .name
        .split('/')
        .next_back()
        .unwrap_or(&branch.name)
        .to_string();
      if repo.find_branch(&local_name, BranchType::Local).is_err() {
        repo.branch(&local_name, &commit, false)?;
      }
      if let Ok(mut local_branch) = repo.find_branch(&local_name, BranchType::Local) {
        let _ = local_branch.set_upstream(Some(&branch.name));
      }

      (
        format!("refs/heads/{}", local_name),
        format!("refs/heads/{}", local_name),
      )
    }
  };

  repo
    .set_head(&refname)
    .with_context(|| format!("set HEAD to {:?}", refname))?;
  let mut checkout = CheckoutBuilder::new();
  checkout.safe();
  repo.checkout_head(Some(&mut checkout))?;
  repo.set_head(&checkout_target)?;
  Ok(())
}

pub fn create_branch(repo_root: &Path, name: &str) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head = repo
    .head()
    .and_then(|head| head.peel_to_commit())
    .context("read HEAD commit")?;
  repo.branch(name, &head, false)?;
  Ok(())
}

pub fn create_branch_from(repo_root: &Path, name: &str, base: &BranchRef) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let refname = match base.kind {
    BranchKind::Local => format!("refs/heads/{}", base.name),
    BranchKind::Remote => format!("refs/remotes/{}", base.name),
  };
  let oid = repo
    .refname_to_id(&refname)
    .with_context(|| format!("resolve branch {:?}", base.name))?;
  let commit = repo.find_commit(oid)?;
  repo.branch(name, &commit, false)?;
  if base.kind == BranchKind::Remote
    && let Ok(mut local_branch) = repo.find_branch(name, BranchType::Local)
  {
    let _ = local_branch.set_upstream(Some(&base.name));
  }
  Ok(())
}

pub fn merge_branch(repo_root: &Path, branch: &BranchRef) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head = repo
    .head()
    .and_then(|head| head.peel_to_commit())
    .context("read HEAD commit")?;

  let refname = match branch.kind {
    BranchKind::Local => format!("refs/heads/{}", branch.name),
    BranchKind::Remote => format!("refs/remotes/{}", branch.name),
  };
  let oid = repo
    .refname_to_id(&refname)
    .with_context(|| format!("resolve branch {:?}", branch.name))?;
  let target_commit = repo.find_commit(oid)?;
  let annotated = repo.find_annotated_commit(oid)?;

  let (analysis, _) = repo.merge_analysis(&[&annotated])?;
  if analysis.is_up_to_date() {
    return Ok(());
  }

  if analysis.is_fast_forward() {
    let head_ref = repo.head()?;
    let refname = head_ref
      .name()
      .ok_or_else(|| anyhow::anyhow!("invalid HEAD"))?;
    let mut reference = repo.find_reference(refname)?;
    reference.set_target(target_commit.id(), "Fast-Forward")?;
    repo.set_head(reference.name().unwrap())?;
    let mut checkout = CheckoutBuilder::new();
    checkout.force();
    repo.checkout_head(Some(&mut checkout))?;
    return Ok(());
  }

  if analysis.is_normal() {
    repo.merge(&[&annotated], None, None)?;
    let mut index = repo.index()?;
    if index.has_conflicts() {
      return Err(anyhow::anyhow!("merge has conflicts"));
    }
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let signature = repo
      .signature()
      .or_else(|_| Signature::now("reviu", "reviu@contact"))?;
    let message = format!("Merge branch '{}'", branch.name);
    repo.commit(
      Some("HEAD"),
      &signature,
      &signature,
      &message,
      &tree,
      &[&head, &target_commit],
    )?;
    let mut checkout = CheckoutBuilder::new();
    checkout.safe();
    repo.checkout_head(Some(&mut checkout))?;
    repo.cleanup_state()?;
    return Ok(());
  }

  bail!("unsupported merge analysis")
}

pub fn rebase_branch(repo_root: &Path, branch: &BranchRef) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let refname = match branch.kind {
    BranchKind::Local => format!("refs/heads/{}", branch.name),
    BranchKind::Remote => format!("refs/remotes/{}", branch.name),
  };
  let oid = repo
    .refname_to_id(&refname)
    .with_context(|| format!("resolve branch {:?}", branch.name))?;
  let upstream = repo.find_annotated_commit(oid)?;
  let signature = repo
    .signature()
    .or_else(|_| Signature::now("reviu", "reviu@contact"))?;
  let mut rebase = repo.rebase(None, Some(&upstream), None, None)?;

  while let Some(next_operation) = rebase.next() {
    if let Err(err) = next_operation {
      let _ = rebase.abort();
      return Err(err.into());
    }

    let index = repo.index()?;
    if index.has_conflicts() {
      let _ = rebase.abort();
      bail!("rebase has conflicts");
    }

    if let Err(err) = rebase.commit(None, &signature, None) {
      let _ = rebase.abort();
      return Err(err.into());
    }
  }

  rebase.finish(Some(&signature))?;
  let mut checkout = CheckoutBuilder::new();
  checkout.safe();
  repo.checkout_head(Some(&mut checkout))?;
  Ok(())
}

pub fn cherry_pick_commits(repo_root: &Path, commit_hashes: &[String]) -> Result<()> {
  if commit_hashes.is_empty() {
    bail!("no commits provided for cherry-pick");
  }

  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;

  for commit_hash in commit_hashes {
    let commit_hash = commit_hash.trim();
    if commit_hash.is_empty() {
      bail!("empty commit hash");
    }

    let object = repo
      .revparse_single(commit_hash)
      .with_context(|| format!("resolve commit {commit_hash:?}"))?;
    let commit = object
      .peel_to_commit()
      .with_context(|| format!("resolve commit {commit_hash:?}"))?;
    let head = repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .context("read HEAD commit")?;

    let mut options = CherrypickOptions::new();
    repo
      .cherrypick(&commit, Some(&mut options))
      .with_context(|| format!("cherry-pick commit {commit_hash:?}"))?;

    let mut index = repo.index()?;
    if index.has_conflicts() {
      bail!("cherry-pick has conflicts for {commit_hash}");
    }

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let signature = repo
      .signature()
      .or_else(|_| Signature::now("reviu", "reviu@contact"))?;
    let message = commit
      .message()
      .map(str::trim)
      .filter(|msg| !msg.is_empty());
    let message = message.unwrap_or("cherry-pick");

    repo
      .commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &[&head],
      )
      .with_context(|| format!("create cherry-pick commit for {commit_hash:?}"))?;
    repo.cleanup_state().context("cleanup cherry-pick state")?;
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use git2::RemoteCallbacks;
  use git2::build::CheckoutBuilder;
  use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
  };

  struct TempRepo {
    path: PathBuf,
  }

  impl TempRepo {
    fn init(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!("reviu-{prefix}-{}-{nanos}", std::process::id()));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Repository::init(&path).expect("init git repository");
      Self { path }
    }
  }

  impl Drop for TempRepo {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  struct TempDir {
    path: PathBuf,
  }

  impl TempDir {
    fn new(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!("reviu-{prefix}-dir-{}-{nanos}", std::process::id()));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Self { path }
    }
  }

  impl Drop for TempDir {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  struct TempBareRepo {
    path: PathBuf,
  }

  impl TempBareRepo {
    fn init(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!(
        "reviu-{prefix}-bare-{}-{nanos}",
        std::process::id()
      ));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Repository::init_bare(&path).expect("init bare git repository");
      Self { path }
    }
  }

  impl Drop for TempBareRepo {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  fn commit_text_file(
    repo_root: &Path,
    rel_path: &Path,
    contents: &str,
    message: &str,
  ) -> git2::Oid {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::write(repo_root.join(rel_path), contents).expect("write worktree file");

    let mut index = repo.index().expect("open index");
    index.add_path(rel_path).expect("stage file");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("find tree");
    let sig = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());

    match parent {
      Some(parent) => repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
        .expect("commit with parent"),
      None => repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
        .expect("initial commit"),
    }
  }

  fn push_branch_to_remote(repo_root: &Path, branch_name: &str, remote_name: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    let mut remote = repo.find_remote(remote_name).expect("find remote");
    let refspec = format!("refs/heads/{branch_name}:refs/heads/{branch_name}");
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_, _, _| git2::Cred::default());
    let mut push_options = git2::PushOptions::new();
    push_options.remote_callbacks(callbacks);
    remote
      .push(&[refspec], Some(&mut push_options))
      .expect("push branch");
  }

  fn force_checkout_head(repo_root: &Path) {
    let repo = Repository::open(repo_root).expect("open repo");
    let mut checkout = CheckoutBuilder::new();
    checkout.force();
    repo
      .checkout_head(Some(&mut checkout))
      .expect("force checkout head");
  }

  #[test]
  fn create_branch_creates_local_branch() {
    let repo = TempRepo::init("branch-create");
    commit_text_file(&repo.path, Path::new("README.md"), "hello\n", "initial");

    create_branch(&repo.path, "feature").expect("create branch");
    let branches = list_branches(&repo.path).expect("list branches");
    assert!(
      branches
        .iter()
        .any(|branch| branch.kind == BranchKind::Local && branch.name == "feature")
    );
  }

  #[test]
  fn switch_branch_updates_current_branch() {
    let repo = TempRepo::init("branch-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "hello\n", "initial");
    create_branch(&repo.path, "feature").expect("create branch");

    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch branch");

    let status = current_branch_status(&repo.path).expect("branch status");
    assert_eq!(status.name, "feature");
  }

  #[test]
  fn current_branch_status_uses_head_label_for_detached_head() {
    let repo = TempRepo::init("branch-detached");
    let oid = commit_text_file(&repo.path, Path::new("README.md"), "hello\n", "initial");
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    repo_handle.set_head_detached(oid).expect("detach head");

    let status = current_branch_status(&repo.path).expect("branch status");
    assert_eq!(status.name, "HEAD");
  }

  #[test]
  fn current_branch_status_reports_ahead_and_behind_with_upstream() {
    let remote = TempBareRepo::init("branch-upstream-remote");
    let local = TempRepo::init("branch-upstream-local");
    let peer = TempDir::new("branch-upstream-peer");

    let _ = commit_text_file(&local.path, Path::new("README.md"), "v1\n", "initial");
    let local_repo = Repository::open(&local.path).expect("open local repo");
    local_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&local.path)
      .expect("read local branch status")
      .name;

    push_branch_to_remote(&local.path, &branch_name, "origin");
    let mut local_branch = local_repo
      .find_branch(&branch_name, BranchType::Local)
      .expect("find local branch");
    local_branch
      .set_upstream(Some(&format!("origin/{branch_name}")))
      .expect("set upstream");

    let _ = commit_text_file(
      &local.path,
      Path::new("README.md"),
      "v2-local\n",
      "local change",
    );

    let peer_repo = Repository::clone(remote.path.to_str().expect("remote path utf8"), &peer.path)
      .expect("clone remote in peer");
    std::fs::write(peer.path.join("README.md"), "v2-peer\n").expect("update peer file");
    let mut index = peer_repo.index().expect("open peer index");
    index
      .add_path(Path::new("README.md"))
      .expect("stage peer file");
    index.write().expect("write peer index");
    let tree_id = index.write_tree().expect("write peer tree");
    let tree = peer_repo.find_tree(tree_id).expect("find peer tree");
    let sig = Signature::now("Reviu Tests", "tests@reviu.local").expect("peer signature");
    let parent = peer_repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("peer parent");
    peer_repo
      .commit(Some("HEAD"), &sig, &sig, "peer change", &tree, &[&parent])
      .expect("peer commit");
    push_branch_to_remote(&peer.path, &branch_name, "origin");

    {
      let mut remote = local_repo.find_remote("origin").expect("find origin");
      remote
        .fetch(&["refs/heads/*:refs/remotes/origin/*"], None, None)
        .expect("fetch remote updates");
    }

    let status = current_branch_status(&local.path).expect("branch status");
    assert_eq!(status.name, branch_name);
    assert!(status.has_upstream);
    assert!(
      status.ahead >= 1,
      "expected ahead >= 1, got {}",
      status.ahead
    );
    assert!(
      status.behind >= 1,
      "expected behind >= 1, got {}",
      status.behind
    );
  }

  #[test]
  fn merge_branch_fast_forward_moves_head_to_target_commit() {
    let repo = TempRepo::init("branch-merge-fast-forward");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_commit = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    force_checkout_head(&repo.path);

    merge_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("fast-forward merge");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.id(), feature_commit);
    assert_eq!(head.parent_count(), 1);
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read merged file"),
      "v2-feature\n"
    );
  }

  #[test]
  fn merge_branch_normal_creates_merge_commit_with_two_parents() {
    let repo = TempRepo::init("branch-merge-normal");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let _ = commit_text_file(
      &repo.path,
      Path::new("main.txt"),
      "base-main\n",
      "base main",
    );
    let _ = commit_text_file(
      &repo.path,
      Path::new("feature.txt"),
      "base-feature\n",
      "base feature",
    );
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, Path::new("main.txt"), "main\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("feature.txt"),
      "feature\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    force_checkout_head(&repo.path);

    merge_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("normal merge");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.parent_count(), 2);
    assert_eq!(head.message().unwrap_or_default(), "Merge branch 'feature'");
    assert_eq!(
      std::fs::read_to_string(repo.path.join("main.txt")).expect("read main side file"),
      "main\n"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("feature.txt")).expect("read feature side file"),
      "feature\n"
    );
  }

  #[test]
  fn merge_branch_returns_error_on_conflicts() {
    let repo = TempRepo::init("branch-merge-conflict");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    force_checkout_head(&repo.path);

    let error = merge_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("merge should fail with conflicts");

    assert!(
      error.to_string().contains("merge has conflicts"),
      "unexpected error: {error:?}"
    );
  }

  #[test]
  fn merge_branch_up_to_date_is_noop() {
    let repo = TempRepo::init("branch-merge-up-to-date");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head_before = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head before merge")
      .id();

    merge_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("up-to-date merge");

    let head_after = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head after merge")
      .id();
    assert_eq!(head_after, head_before);
  }

  #[test]
  fn rebase_branch_fast_forward_moves_head_to_target_commit() {
    let repo = TempRepo::init("branch-rebase-fast-forward");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_commit = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    force_checkout_head(&repo.path);

    rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("fast-forward rebase");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.id(), feature_commit);
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read rebased file"),
      "v2-feature\n"
    );
  }

  #[test]
  fn rebase_branch_returns_error_on_conflicts() {
    let repo = TempRepo::init("branch-rebase-conflict");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    force_checkout_head(&repo.path);

    let error = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");

    assert!(
      error.to_string().contains("rebase has conflicts"),
      "unexpected error: {error:?}"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed rebase")
        .name,
      base_branch
    );
  }

  #[test]
  fn cherry_pick_commits_applies_single_commit() {
    let repo = TempRepo::init("branch-cherry-pick-single");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;

    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_commit = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );

    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    force_checkout_head(&repo.path);

    cherry_pick_commits(&repo.path, &[feature_commit.to_string()]).expect("cherry-pick commit");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.message().unwrap_or_default(), "feature change");
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read cherry-picked file"),
      "v2-feature\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after cherry-pick")
        .name,
      base_branch
    );
  }

  #[test]
  fn cherry_pick_commits_applies_multiple_commits_in_order() {
    let repo = TempRepo::init("branch-cherry-pick-multiple");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;

    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let first = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature 1",
    );
    let second = commit_text_file(&repo.path, Path::new("extra.txt"), "extra\n", "feature 2");

    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    force_checkout_head(&repo.path);

    cherry_pick_commits(&repo.path, &[first.to_string(), second.to_string()])
      .expect("cherry-pick commits");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.message().unwrap_or_default(), "feature 2");
    let parent = head.parent(0).expect("head parent");
    assert_eq!(parent.message().unwrap_or_default(), "feature 1");
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read cherry-picked README"),
      "v2-feature\n"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("extra.txt")).expect("read cherry-picked extra file"),
      "extra\n"
    );
  }

  #[test]
  fn cherry_pick_commits_returns_error_when_commit_is_missing() {
    let repo = TempRepo::init("branch-cherry-pick-missing");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let error = cherry_pick_commits(&repo.path, &[String::from("deadbeef")])
      .expect_err("missing commit should fail");
    let message = error.to_string();
    assert!(
      message.contains("resolve commit"),
      "unexpected error message: {message}"
    );
  }

  #[test]
  fn switch_branch_remote_creates_local_branch_and_sets_upstream() {
    let remote = TempBareRepo::init("branch-switch-remote-origin");
    let source = TempRepo::init("branch-switch-remote-source");
    let clone_dir = TempDir::new("branch-switch-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");

    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");

    create_branch(&source.path, "feature").expect("create source feature branch");
    switch_branch(
      &source.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch source to feature");
    let _ = commit_text_file(
      &source.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");

    switch_branch(
      &clone_dir.path,
      &BranchRef {
        name: "origin/feature".to_string(),
        kind: BranchKind::Remote,
      },
    )
    .expect("switch to remote feature branch");

    let status = current_branch_status(&clone_dir.path).expect("branch status after switch");
    assert_eq!(status.name, "feature");
    assert!(status.has_upstream);

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone repo");
    let local_feature = clone_repo
      .find_branch("feature", BranchType::Local)
      .expect("find local feature branch");
    let upstream = local_feature
      .upstream()
      .expect("feature branch upstream")
      .name()
      .expect("upstream name")
      .expect("non-empty upstream")
      .to_string();
    assert_eq!(upstream, "origin/feature");
  }

  #[test]
  fn create_branch_from_remote_creates_branch_with_upstream() {
    let remote = TempBareRepo::init("branch-create-from-remote-origin");
    let source = TempRepo::init("branch-create-from-remote-source");
    let clone_dir = TempDir::new("branch-create-from-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");

    create_branch(&source.path, "feature").expect("create source feature branch");
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");

    create_branch_from(
      &clone_dir.path,
      "my-feature",
      &BranchRef {
        name: "origin/feature".to_string(),
        kind: BranchKind::Remote,
      },
    )
    .expect("create local branch from remote");

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone repo");
    let local_branch = clone_repo
      .find_branch("my-feature", BranchType::Local)
      .expect("find created local branch");
    let upstream = local_branch
      .upstream()
      .expect("branch upstream")
      .name()
      .expect("upstream name")
      .expect("non-empty upstream")
      .to_string();
    assert_eq!(upstream, "origin/feature");
  }

  #[test]
  fn create_branch_from_local_creates_branch_without_upstream() {
    let repo = TempRepo::init("branch-create-from-local");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_head = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );

    create_branch_from(
      &repo.path,
      "feature-copy",
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("create from local branch");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let copy = repo_handle
      .find_branch("feature-copy", BranchType::Local)
      .expect("find copied branch");
    assert_eq!(copy.get().target(), Some(feature_head));
    assert!(copy.upstream().is_err());
  }
}
