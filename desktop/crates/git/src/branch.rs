use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{
  BranchType, CherrypickOptions, Cred, ErrorCode, FetchOptions, PushOptions, Rebase,
  RemoteCallbacks, Repository, RepositoryState, ResetType, Signature, StatusOptions,
};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubRemoteRepo {
  pub owner: String,
  pub repo: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
  pub index: usize,
  pub name: String,
  pub oid: String,
}

fn repo_signature(repo: &Repository) -> Result<Signature<'static>> {
  repo
    .signature()
    .map(|signature| signature.to_owned())
    .context("git user identity is not configured (set user.name and user.email)")
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

    let last_commit_time = branch
      .get()
      .peel_to_commit()
      .map(|c| c.time().seconds())
      .unwrap_or(0);

    branches.push((BranchRef { name, kind }, last_commit_time));
  }

  branches.sort_by(|a, b| match (a.0.kind, b.0.kind) {
    (BranchKind::Local, BranchKind::Remote) => std::cmp::Ordering::Less,
    (BranchKind::Remote, BranchKind::Local) => std::cmp::Ordering::Greater,
    _ => b.1.cmp(&a.1),
  });

  let branches = branches.into_iter().map(|(b, _)| b).collect();

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

pub fn current_branch_upstream(repo_root: &Path) -> Result<Option<BranchRef>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head = repo.head()?;
  if !head.is_branch() {
    return Ok(None);
  }

  let Some(local_name) = head.shorthand() else {
    return Ok(None);
  };
  let Ok(local_branch) = repo.find_branch(local_name, BranchType::Local) else {
    return Ok(None);
  };
  let Ok(upstream) = local_branch.upstream() else {
    return Ok(None);
  };

  branch_ref_from_full_name(upstream.name()?.unwrap_or(""))
}

pub fn branch_has_unpublished_commits(repo_root: &Path) -> Result<bool> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head = match repo.head() {
    Ok(head) => head,
    Err(_) => return Ok(false),
  };

  if !head.is_branch() {
    return Ok(false);
  }

  let branch_name = head.shorthand().unwrap_or("HEAD");
  let branch = match repo.find_branch(branch_name, BranchType::Local) {
    Ok(branch) => branch,
    Err(_) => return Ok(false),
  };
  let Some(local_oid) = branch.get().target() else {
    return Ok(false);
  };

  if let Ok(upstream) = branch.upstream()
    && let Some(upstream_oid) = upstream.get().target()
  {
    let (ahead, _) = repo.graph_ahead_behind(local_oid, upstream_oid)?;
    return Ok(ahead > 0);
  }

  for remote_branch in repo.branches(Some(BranchType::Remote))? {
    let (remote_branch, _) = remote_branch?;
    let Some(remote_oid) = remote_branch.get().target() else {
      continue;
    };

    if remote_oid == local_oid || repo.graph_descendant_of(remote_oid, local_oid)? {
      return Ok(false);
    }
  }

  Ok(true)
}

pub fn current_head_sha(repo_root: &Path) -> Result<Option<String>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  Ok(
    repo
      .head()
      .ok()
      .and_then(|head| head.target())
      .map(|oid| oid.to_string()),
  )
}

pub fn current_github_remote_repo(repo_root: &Path) -> Result<Option<GithubRemoteRepo>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  resolve_github_remote_repo(&repo)
}

pub fn detached_head_label(repo_root: &Path) -> Result<String> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head_commit = repo
    .head()
    .and_then(|head| head.peel_to_commit())
    .context("read HEAD commit")?;
  let head_oid = head_commit.id();

  let tag_names = repo.tag_names(None).context("list tags")?;
  let mut exact_tags = Vec::new();
  for tag_name in tag_names.iter().flatten() {
    let refname = format!("refs/tags/{tag_name}");
    let Ok(object) = repo.revparse_single(&refname) else {
      continue;
    };
    let Ok(commit) = object.peel_to_commit() else {
      continue;
    };
    if commit.id() == head_oid {
      exact_tags.push(tag_name.to_string());
    }
  }

  if let Some(tag) = exact_tags.into_iter().min() {
    return Ok(tag);
  }

  Ok(head_oid.to_string().chars().take(7).collect::<String>())
}

pub fn fetch(repo_root: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let remotes = repo.remotes().context("list remotes")?;

  for remote_name in remotes.iter().flatten() {
    let mut remote = repo
      .find_remote(remote_name)
      .with_context(|| format!("find remote {remote_name:?}"))?;
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_, username_from_url, _| {
      if let Some(username) = username_from_url {
        Cred::ssh_key_from_agent(username).or_else(|_| Cred::default())
      } else {
        Cred::default()
      }
    });
    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);
    remote
      .fetch(&[] as &[&str], Some(&mut fetch_options), None)
      .with_context(|| format!("fetch remote {remote_name:?}"))?;
  }

  Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullOutcome {
  AlreadyUpToDate,
  Pulled,
}

pub fn pull(repo_root: &Path) -> Result<PullOutcome> {
  let head_before = current_head_sha(repo_root).ok().flatten();

  let output = Command::new("git")
    .current_dir(repo_root)
    .args(["pull"])
    .output()
    .context("run git pull")?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("git pull failed: {}", stderr.trim())
  }

  let head_after = current_head_sha(repo_root).ok().flatten();
  if head_before == head_after {
    Ok(PullOutcome::AlreadyUpToDate)
  } else {
    Ok(PullOutcome::Pulled)
  }
}

pub fn clone(url: &str, destination: &Path) -> Result<()> {
  if destination.exists() {
    bail!("destination already exists: {}", destination.display());
  }

  let output = Command::new("git")
    .arg("clone")
    .arg("--")
    .arg(url)
    .arg(destination)
    .output()
    .context("run git clone")?;

  if output.status.success() {
    return Ok(());
  }

  let stderr = String::from_utf8_lossy(&output.stderr);
  bail!("git clone failed: {}", stderr.trim())
}

pub fn sync_current_branch_to_head(
  repo_root: &Path,
  branch_name: &str,
  target_head_sha: &str,
) -> Result<()> {
  let branch_name = branch_name.trim();
  if branch_name.is_empty() {
    bail!("branch name is empty");
  }

  let target_head_sha = target_head_sha.trim();
  if target_head_sha.is_empty() {
    bail!("target head sha is empty");
  }

  ensure_worktree_clean(repo_root)?;
  fetch(repo_root)?;

  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head = repo.head().context("read HEAD reference")?;
  if !head.is_branch() {
    bail!("current HEAD is detached");
  }

  let current_branch = head.shorthand().unwrap_or("HEAD");
  if current_branch != branch_name {
    bail!("current branch {current_branch:?} does not match expected branch {branch_name:?}");
  }

  let current_oid = head.target().context("read HEAD target")?;
  let target_oid =
    git2::Oid::from_str(target_head_sha).context("parse target pull request head sha")?;
  if current_oid == target_oid {
    return Ok(());
  }

  repo
    .find_commit(target_oid)
    .with_context(|| format!("find target commit {target_head_sha} after fetch"))?;

  if !repo
    .graph_descendant_of(target_oid, current_oid)
    .context("compare current branch and target pull request head")?
  {
    bail!("target pull request head is not a fast-forward from the current branch");
  }

  let branch = repo
    .find_branch(branch_name, BranchType::Local)
    .with_context(|| format!("find local branch {branch_name:?}"))?;
  branch
    .into_reference()
    .set_target(target_oid, "reviu: sync to PR head")
    .context("advance local branch to pull request head")?;

  let branch_refname = format!("refs/heads/{branch_name}");
  repo
    .set_head(&branch_refname)
    .context("point HEAD at synced local branch")?;
  let mut checkout = CheckoutBuilder::new();
  checkout.force();
  repo
    .checkout_head(Some(&mut checkout))
    .context("checkout synced pull request head")?;

  Ok(())
}

pub fn switch_branch(repo_root: &Path, branch: &BranchRef) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;

  let (refname, target_oid) = match branch.kind {
    BranchKind::Local => {
      let refname = format!("refs/heads/{}", branch.name);
      let oid = repo
        .refname_to_id(&refname)
        .with_context(|| format!("resolve local branch {:?}", branch.name))?;
      (refname, oid)
    }
    BranchKind::Remote => {
      let remote_ref = format!("refs/remotes/{}", branch.name);
      let oid = repo
        .refname_to_id(&remote_ref)
        .with_context(|| format!("resolve remote branch {:?}", branch.name))?;
      let commit = repo.find_commit(oid)?;

      let local_name = branch
        .name
        .split_once('/')
        .map(|(_, name)| name)
        .unwrap_or(&branch.name)
        .to_string();
      if repo.find_branch(&local_name, BranchType::Local).is_err() {
        repo.branch(&local_name, &commit, false)?;
      }
      if let Ok(mut local_branch) = repo.find_branch(&local_name, BranchType::Local) {
        let _ = local_branch.set_upstream(Some(&branch.name));
      }

      let refname = format!("refs/heads/{}", local_name);
      let oid = repo
        .refname_to_id(&refname)
        .with_context(|| format!("resolve local branch {:?}", local_name))?;
      (refname, oid)
    }
  };

  let object = repo
    .find_object(target_oid, None)
    .with_context(|| format!("find target object for {:?}", refname))?;
  let mut checkout = CheckoutBuilder::new();
  checkout.safe();
  repo
    .checkout_tree(&object, Some(&mut checkout))
    .with_context(|| format!("checkout target tree for {:?}", refname))?;
  repo
    .set_head(&refname)
    .with_context(|| format!("set HEAD to {:?}", refname))?;
  Ok(())
}

pub fn switch_to_branch_name(repo_root: &Path, branch_name: &str) -> Result<()> {
  let branch_name = branch_name.trim();
  if branch_name.is_empty() {
    bail!("branch name is empty");
  }

  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  if repo.find_branch(branch_name, BranchType::Local).is_ok() {
    return switch_branch(
      repo_root,
      &BranchRef {
        name: branch_name.to_string(),
        kind: BranchKind::Local,
      },
    );
  }

  fetch(repo_root)?;

  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let Some(remote_branch_name) = resolve_remote_branch_name(&repo, branch_name)? else {
    bail!("branch {branch_name:?} was not found locally or on any remote");
  };

  switch_branch(
    repo_root,
    &BranchRef {
      name: remote_branch_name,
      kind: BranchKind::Remote,
    },
  )
}

pub fn resolve_branch_ref(repo_root: &Path, branch_name: &str) -> Result<Option<BranchRef>> {
  let branch_name = branch_name.trim();
  if branch_name.is_empty() {
    bail!("branch name is empty");
  }

  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;

  if let Some(remote_branch_name) = resolve_remote_branch_name(&repo, branch_name)? {
    return Ok(Some(BranchRef {
      name: remote_branch_name,
      kind: BranchKind::Remote,
    }));
  }

  if repo.find_branch(branch_name, BranchType::Local).is_ok() {
    return Ok(Some(BranchRef {
      name: branch_name.to_string(),
      kind: BranchKind::Local,
    }));
  }

  Ok(None)
}

fn ensure_worktree_clean(repo_root: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut opts = StatusOptions::new();
  opts
    .include_untracked(true)
    .recurse_untracked_dirs(true)
    .include_ignored(false);
  let statuses = repo
    .statuses(Some(&mut opts))
    .context("read worktree status")?;
  if statuses.iter().any(|entry| !entry.status().is_empty()) {
    bail!("local changes detected");
  }
  Ok(())
}

fn preferred_remote_name(repo: &Repository) -> Result<Option<String>> {
  if let Ok(head) = repo.head()
    && head.is_branch()
    && let Some(local_name) = head.shorthand()
    && let Ok(local_branch) = repo.find_branch(local_name, BranchType::Local)
    && let Ok(upstream) = local_branch.upstream()
    && let Some(upstream_name) = upstream.name()?
    && let Some(remote_name) = upstream_name
      .strip_prefix("refs/remotes/")
      .and_then(|name| name.split('/').next())
  {
    return Ok(Some(remote_name.to_string()));
  }

  if repo.find_remote("origin").is_ok() {
    return Ok(Some("origin".to_string()));
  }

  let remotes = repo.remotes().context("list remotes")?;
  Ok(remotes.iter().flatten().next().map(str::to_string))
}

pub fn default_remote_branch(repo_root: &Path) -> Result<Option<BranchRef>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;

  let mut remote_names = Vec::new();
  if let Some(preferred_remote) = preferred_remote_name(&repo)? {
    remote_names.push(preferred_remote);
  }
  if !remote_names.iter().any(|name| name == "origin") && repo.find_remote("origin").is_ok() {
    remote_names.push("origin".to_string());
  }
  let remotes = repo.remotes().context("list remotes")?;
  for remote_name in remotes.iter().flatten() {
    if !remote_names.iter().any(|name| name == remote_name) {
      remote_names.push(remote_name.to_string());
    }
  }

  for remote_name in remote_names {
    let head_ref = format!("refs/remotes/{remote_name}/HEAD");
    if let Ok(reference) = repo.find_reference(&head_ref)
      && let Some(target) = reference.symbolic_target()
      && let Some(branch) = branch_ref_from_full_name(target)?
    {
      return Ok(Some(branch));
    }

    for candidate in ["main", "master"] {
      let branch_name = format!("{remote_name}/{candidate}");
      if repo.find_branch(&branch_name, BranchType::Remote).is_ok() {
        return Ok(Some(BranchRef {
          name: branch_name,
          kind: BranchKind::Remote,
        }));
      }
    }
  }

  Ok(None)
}

fn branch_ref_from_full_name(name: &str) -> Result<Option<BranchRef>> {
  if let Some(name) = name.strip_prefix("refs/remotes/") {
    return Ok(Some(BranchRef {
      name: name.to_string(),
      kind: BranchKind::Remote,
    }));
  }
  if let Some(name) = name.strip_prefix("refs/heads/") {
    return Ok(Some(BranchRef {
      name: name.to_string(),
      kind: BranchKind::Local,
    }));
  }
  if !name.is_empty() {
    return Ok(Some(BranchRef {
      name: name.to_string(),
      kind: BranchKind::Remote,
    }));
  }
  Ok(None)
}

fn resolve_remote_branch_name(repo: &Repository, branch_name: &str) -> Result<Option<String>> {
  let mut candidates = Vec::new();

  if let Some(preferred_remote) = preferred_remote_name(repo)? {
    candidates.push(format!("{preferred_remote}/{branch_name}"));
  }
  if !candidates
    .iter()
    .any(|name| name == &format!("origin/{branch_name}"))
  {
    candidates.push(format!("origin/{branch_name}"));
  }

  for candidate in candidates {
    if repo.find_branch(&candidate, BranchType::Remote).is_ok() {
      return Ok(Some(candidate));
    }
  }

  for branch in repo
    .branches(Some(BranchType::Remote))
    .context("list remote branches")?
  {
    let (branch, _) = branch?;
    let Some(name) = branch.name()? else {
      continue;
    };
    if name.ends_with("/HEAD") {
      continue;
    }
    if name
      .split_once('/')
      .map(|(_, remote_branch_name)| remote_branch_name == branch_name)
      .unwrap_or(false)
    {
      return Ok(Some(name.to_string()));
    }
  }

  Ok(None)
}

fn resolve_github_remote_repo(repo: &Repository) -> Result<Option<GithubRemoteRepo>> {
  if let Some(url) = preferred_remote_url(repo)?
    && let Some(parsed) = parse_github_remote_repo(url.as_str())
  {
    return Ok(Some(parsed));
  }

  let remotes = repo.remotes().context("list remotes")?;
  for remote_name in remotes.iter().flatten() {
    let Ok(remote) = repo.find_remote(remote_name) else {
      continue;
    };
    if let Some(url) = remote.url()
      && let Some(parsed) = parse_github_remote_repo(url)
    {
      return Ok(Some(parsed));
    }
  }

  Ok(None)
}

fn preferred_remote_url(repo: &Repository) -> Result<Option<String>> {
  if let Ok(head) = repo.head()
    && head.is_branch()
  {
    let local_name = head.shorthand().unwrap_or("HEAD");
    if local_name != "HEAD"
      && let Ok(local_branch) = repo.find_branch(local_name, BranchType::Local)
      && let Ok(upstream) = local_branch.upstream()
    {
      let upstream_name = upstream.name()?.unwrap_or("");
      if !upstream_name.is_empty() {
        let remote_name = upstream_name.split('/').next().unwrap_or("origin");
        if let Ok(remote) = repo.find_remote(remote_name)
          && let Some(url) = remote.url()
        {
          return Ok(Some(url.to_string()));
        }
      }
    }
  }

  if let Ok(remote) = repo.find_remote("origin")
    && let Some(url) = remote.url()
  {
    return Ok(Some(url.to_string()));
  }

  Ok(None)
}

fn parse_github_remote_repo(url: &str) -> Option<GithubRemoteRepo> {
  let url = url.trim().trim_end_matches('/');
  let path = if let Some(path) = url.strip_prefix("https://github.com/") {
    path
  } else if let Some(path) = url.strip_prefix("http://github.com/") {
    path
  } else if let Some(path) = url.strip_prefix("ssh://git@github.com/") {
    path
  } else {
    url.strip_prefix("git@github.com:")?
  };

  let path = path
    .trim_end_matches('/')
    .strip_suffix(".git")
    .unwrap_or(path);
  let mut parts = path.split('/').filter(|part| !part.is_empty());
  let owner = parts.next()?;
  let repo = parts.next()?;
  if parts.next().is_some() {
    return None;
  }

  Some(GithubRemoteRepo {
    owner: owner.to_string(),
    repo: repo.to_string(),
  })
}

pub fn checkout_detached_target(repo_root: &Path, target: &str) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let target = target.trim();
  if target.is_empty() {
    bail!("detached target cannot be empty");
  }
  let (object, reference) = repo
    .revparse_ext(target)
    .with_context(|| format!("resolve detached target {target:?}"))?;
  if let Some(reference) = reference.as_ref()
    && let Some(name) = reference.name()
    && (name == "HEAD" || name.starts_with("refs/heads/") || name.starts_with("refs/remotes/"))
  {
    bail!("detached target must be a commit hash or tag");
  }
  let target_commit = object
    .peel_to_commit()
    .with_context(|| format!("resolve commit for detached target {target:?}"))?;

  repo
    .set_head_detached(target_commit.id())
    .context("set HEAD to detached")?;

  let mut checkout = CheckoutBuilder::new();
  checkout.safe();
  repo
    .checkout_head(Some(&mut checkout))
    .context("checkout detached HEAD")?;
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
  Ok(())
}

pub fn delete_branch(repo_root: &Path, branch: &BranchRef) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  match branch.kind {
    BranchKind::Local => {
      let head = repo.head().context("read HEAD reference")?;
      if head.is_branch() && head.shorthand() == Some(branch.name.as_str()) {
        bail!("cannot delete the current branch")
      }

      let mut local_branch = repo
        .find_branch(&branch.name, BranchType::Local)
        .with_context(|| format!("find local branch {:?}", branch.name))?;

      local_branch
        .delete()
        .with_context(|| format!("delete local branch {:?}", branch.name))?;
      Ok(())
    }
    BranchKind::Remote => {
      let (remote_name, remote_branch_name) = branch
        .name
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("invalid remote branch {:?}", branch.name))?;

      {
        let mut remote = repo
          .find_remote(remote_name)
          .with_context(|| format!("find remote {:?}", remote_name))?;
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(|_, username_from_url, _| {
          if let Some(username) = username_from_url {
            Cred::ssh_key_from_agent(username).or_else(|_| Cred::default())
          } else {
            Cred::default()
          }
        });

        let mut options = PushOptions::new();
        options.remote_callbacks(callbacks);
        let refspec = format!(":refs/heads/{remote_branch_name}");
        remote
          .push(&[refspec.as_str()], Some(&mut options))
          .with_context(|| format!("delete remote branch {:?}", branch.name))?;
      }

      if let Ok(mut reference) = repo.find_reference(&format!("refs/remotes/{}", branch.name)) {
        let _ = reference.delete();
      }

      Ok(())
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeBranchOutcome {
  AlreadyUpToDate,
  Merged,
}

pub fn merge_branch(repo_root: &Path, branch: &BranchRef) -> Result<MergeBranchOutcome> {
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
    return Ok(MergeBranchOutcome::AlreadyUpToDate);
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
    return Ok(MergeBranchOutcome::Merged);
  }

  if analysis.is_normal() {
    repo.merge(&[&annotated], None, None)?;
    let mut index = repo.index()?;
    if index.has_conflicts() {
      return Err(anyhow::anyhow!("merge has conflicts"));
    }
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let signature = repo_signature(&repo)?;
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
    return Ok(MergeBranchOutcome::Merged);
  }

  bail!("unsupported merge analysis")
}

pub fn is_merge_in_progress(repo_root: &Path) -> Result<bool> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  Ok(repo.state() == RepositoryState::Merge)
}

pub fn is_rebase_in_progress(repo_root: &Path) -> Result<bool> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  Ok(matches!(
    repo.state(),
    RepositoryState::Rebase | RepositoryState::RebaseInteractive | RepositoryState::RebaseMerge
  ))
}

pub fn current_rebase_commit_message(repo_root: &Path) -> Result<Option<String>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  if !matches!(
    repo.state(),
    RepositoryState::Rebase | RepositoryState::RebaseInteractive | RepositoryState::RebaseMerge
  ) {
    return Ok(None);
  }

  let mut rebase = repo.open_rebase(None).context("open in-progress rebase")?;
  let Some(current_index) = rebase.operation_current() else {
    return Ok(None);
  };
  let Some(operation) = rebase.nth(current_index) else {
    return Ok(None);
  };
  let Ok(commit) = repo.find_commit(operation.id()) else {
    return Ok(None);
  };

  Ok(
    commit
      .summary()
      .or_else(|| commit.message())
      .map(str::trim)
      .filter(|message| !message.is_empty())
      .map(ToOwned::to_owned),
  )
}

pub fn abort_merge(repo_root: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  if repo.state() != RepositoryState::Merge {
    return Ok(());
  }

  let head = repo
    .head()
    .and_then(|head| head.peel_to_commit())
    .context("read HEAD commit")?;
  let mut checkout = CheckoutBuilder::new();
  checkout.force();
  repo
    .reset(head.as_object(), ResetType::Hard, Some(&mut checkout))
    .context("reset merge state to HEAD")?;
  repo.cleanup_state().context("cleanup merge state")?;
  Ok(())
}

pub fn abort_rebase(repo_root: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  if !matches!(
    repo.state(),
    RepositoryState::Rebase | RepositoryState::RebaseInteractive | RepositoryState::RebaseMerge
  ) {
    return Ok(());
  }

  match repo.open_rebase(None) {
    Ok(mut rebase) => rebase.abort().context("abort rebase")?,
    Err(_) => run_git_rebase_command(repo_root, "--abort", "abort rebase")?,
  }
  Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RebaseCommitOutcome {
  Committed,
  AlreadyApplied,
  Conflicts,
}

fn repo_has_conflicts(repo: &Repository) -> bool {
  repo
    .index()
    .map(|index| index.has_conflicts())
    .unwrap_or(false)
}

fn is_rebase_conflict_error(repo: &Repository, err: &git2::Error) -> bool {
  matches!(
    err.code(),
    ErrorCode::Unmerged | ErrorCode::MergeConflict | ErrorCode::Conflict
  ) || repo_has_conflicts(repo)
}

fn commit_rebase_operation(
  rebase: &mut Rebase<'_>,
  repo: &Repository,
  signature: &Signature<'_>,
) -> std::result::Result<RebaseCommitOutcome, git2::Error> {
  if repo_has_conflicts(repo) {
    return Ok(RebaseCommitOutcome::Conflicts);
  }

  match rebase.commit(None, signature, None) {
    Ok(_) => Ok(RebaseCommitOutcome::Committed),
    Err(err) if err.code() == ErrorCode::Applied => Ok(RebaseCommitOutcome::AlreadyApplied),
    Err(err) if is_rebase_conflict_error(repo, &err) => Ok(RebaseCommitOutcome::Conflicts),
    Err(err) => Err(err),
  }
}

fn rebase_command_output_details(output: &std::process::Output) -> String {
  let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
  let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
  [stderr, stdout]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

fn rebase_output_has_conflicts(details: &str) -> bool {
  let details = details.to_ascii_lowercase();
  details.contains("conflict")
    || details.contains("could not apply")
    || details.contains("resolve all conflicts")
    || details.contains("unmerged")
}

fn run_git_rebase_command(repo_root: &Path, flag: &str, operation_name: &str) -> Result<()> {
  let output = Command::new("git")
    .current_dir(repo_root)
    .args(["rebase", flag])
    .env("GIT_EDITOR", ":")
    .env("GIT_SEQUENCE_EDITOR", ":")
    .output()
    .with_context(|| format!("run git rebase {flag}"))?;

  if output.status.success() {
    return Ok(());
  }

  let details = rebase_command_output_details(&output);
  if rebase_output_has_conflicts(&details) {
    bail!("rebase has conflicts");
  }
  if details.is_empty() {
    bail!("{operation_name} failed");
  }

  bail!("{operation_name} failed: {details}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseBranchOutcome {
  AlreadyUpToDate,
  Rebased,
}

pub fn rebase_branch(repo_root: &Path, branch: &BranchRef) -> Result<RebaseBranchOutcome> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let refname = match branch.kind {
    BranchKind::Local => format!("refs/heads/{}", branch.name),
    BranchKind::Remote => format!("refs/remotes/{}", branch.name),
  };
  let target_oid = repo
    .refname_to_id(&refname)
    .with_context(|| format!("resolve branch {:?}", branch.name))?;
  let head_oid = repo.head()?.peel_to_commit()?.id();
  if head_oid == target_oid || repo.graph_descendant_of(head_oid, target_oid)? {
    return Ok(RebaseBranchOutcome::AlreadyUpToDate);
  }

  let upstream = repo.find_annotated_commit(target_oid)?;
  let signature = repo_signature(&repo)?;
  let mut rebase = repo.rebase(None, Some(&upstream), None, None)?;

  while let Some(next_operation) = rebase.next() {
    if let Err(err) = next_operation {
      if is_rebase_conflict_error(&repo, &err) {
        bail!("rebase has conflicts");
      }
      let _ = rebase.abort();
      return Err(err.into());
    }

    match commit_rebase_operation(&mut rebase, &repo, &signature) {
      Ok(RebaseCommitOutcome::Conflicts) => bail!("rebase has conflicts"),
      Ok(RebaseCommitOutcome::Committed | RebaseCommitOutcome::AlreadyApplied) => {}
      Err(err) => {
        let _ = rebase.abort();
        return Err(err.into());
      }
    }
  }

  rebase.finish(Some(&signature))?;
  let mut checkout = CheckoutBuilder::new();
  checkout.safe();
  repo.checkout_head(Some(&mut checkout))?;
  Ok(RebaseBranchOutcome::Rebased)
}

pub fn continue_rebase(repo_root: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  if !matches!(
    repo.state(),
    RepositoryState::Rebase | RepositoryState::RebaseInteractive | RepositoryState::RebaseMerge
  ) {
    return Ok(());
  }

  let signature = repo_signature(&repo)?;
  let mut rebase = match repo.open_rebase(None) {
    Ok(rebase) => rebase,
    Err(_) => return run_git_rebase_command(repo_root, "--continue", "continue rebase"),
  };

  if rebase.operation_current().is_some() {
    match commit_rebase_operation(&mut rebase, &repo, &signature) {
      Ok(RebaseCommitOutcome::Conflicts) => bail!("rebase has conflicts"),
      Ok(RebaseCommitOutcome::Committed | RebaseCommitOutcome::AlreadyApplied) => {}
      Err(err) => {
        let _ = rebase.abort();
        return Err(err.into());
      }
    }
  }

  while let Some(next_operation) = rebase.next() {
    if let Err(err) = next_operation {
      if is_rebase_conflict_error(&repo, &err) {
        bail!("rebase has conflicts");
      }
      let _ = rebase.abort();
      return Err(err.into());
    }

    match commit_rebase_operation(&mut rebase, &repo, &signature) {
      Ok(RebaseCommitOutcome::Conflicts) => bail!("rebase has conflicts"),
      Ok(RebaseCommitOutcome::Committed | RebaseCommitOutcome::AlreadyApplied) => {}
      Err(err) => {
        let _ = rebase.abort();
        return Err(err.into());
      }
    }
  }

  rebase.finish(Some(&signature))?;
  let mut checkout = CheckoutBuilder::new();
  checkout.safe();
  repo.checkout_head(Some(&mut checkout))?;
  Ok(())
}

pub fn skip_rebase(repo_root: &Path) -> Result<()> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  if !matches!(
    repo.state(),
    RepositoryState::Rebase | RepositoryState::RebaseInteractive | RepositoryState::RebaseMerge
  ) {
    return Ok(());
  }

  let signature = repo_signature(&repo)?;
  let mut rebase = match repo.open_rebase(None) {
    Ok(rebase) => rebase,
    Err(_) => return run_git_rebase_command(repo_root, "--skip", "skip rebase"),
  };

  if rebase.operation_current().is_some() {
    let head = repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .context("read HEAD commit for rebase skip")?;
    repo
      .reset(head.as_object(), ResetType::Hard, None)
      .context("reset conflicted rebase state before skip")?;
  }

  while let Some(next_operation) = rebase.next() {
    if let Err(err) = next_operation {
      if is_rebase_conflict_error(&repo, &err) {
        bail!("rebase has conflicts");
      }
      let _ = rebase.abort();
      return Err(err.into());
    }

    match commit_rebase_operation(&mut rebase, &repo, &signature) {
      Ok(RebaseCommitOutcome::Conflicts) => bail!("rebase has conflicts"),
      Ok(RebaseCommitOutcome::Committed | RebaseCommitOutcome::AlreadyApplied) => {}
      Err(err) => {
        let _ = rebase.abort();
        return Err(err.into());
      }
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
    let signature = repo_signature(&repo)?;
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

pub fn default_stash_message(repo_root: &Path) -> Result<String> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head = repo.head().context("read HEAD reference")?;
  let head_label = head.shorthand().unwrap_or("HEAD");
  let commit = head.peel_to_commit().context("read HEAD commit")?;
  let short_oid = commit.id().to_string().chars().take(7).collect::<String>();
  let summary = commit
    .summary()
    .or_else(|| commit.message())
    .map(str::trim)
    .filter(|message| !message.is_empty())
    .unwrap_or("WIP");

  Ok(format!("WIP on {head_label}: {short_oid} {summary}"))
}

pub fn create_stash(
  repo_root: &Path,
  include_untracked: bool,
  message: Option<&str>,
) -> Result<()> {
  let mut args = vec!["stash", "push"];
  if include_untracked {
    args.push("--include-untracked");
  }
  let trimmed;
  if let Some(msg) = message.map(str::trim).filter(|m| !m.is_empty()) {
    args.push("-m");
    trimmed = msg.to_string();
    args.push(&trimmed);
  }
  let output = Command::new("git")
    .args(&args)
    .current_dir(repo_root)
    .output()
    .context("run git stash push")?;
  if !output.status.success() {
    bail!(
      "git stash push failed: {}",
      String::from_utf8_lossy(&output.stderr).trim()
    );
  }
  Ok(())
}

pub fn list_stashes(repo_root: &Path) -> Result<Vec<StashEntry>> {
  let mut repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut stashes = Vec::new();

  repo
    .stash_foreach(|index, name, oid| {
      stashes.push(StashEntry {
        index,
        name: name.to_string(),
        oid: oid.to_string(),
      });
      true
    })
    .context("list stash entries")?;

  Ok(stashes)
}

pub fn apply_stash(repo_root: &Path, index: usize) -> Result<()> {
  let output = Command::new("git")
    .args(["stash", "apply", &format!("stash@{{{index}}}")])
    .current_dir(repo_root)
    .output()
    .with_context(|| format!("run git stash apply at index {index}"))?;
  if !output.status.success() {
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stash_output_has_conflicts(&stdout) {
      return Ok(());
    }
    bail!(
      "git stash apply failed: {}",
      String::from_utf8_lossy(&output.stderr).trim()
    );
  }
  Ok(())
}

pub fn drop_stash(repo_root: &Path, index: usize) -> Result<()> {
  let mut repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  repo
    .stash_drop(index)
    .with_context(|| format!("drop stash at index {index}"))?;
  Ok(())
}

pub fn pop_stash(repo_root: &Path, index: usize) -> Result<()> {
  let output = Command::new("git")
    .args(["stash", "pop", &format!("stash@{{{index}}}")])
    .current_dir(repo_root)
    .output()
    .with_context(|| format!("run git stash pop at index {index}"))?;
  if !output.status.success() {
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stash_output_has_conflicts(&stdout) {
      return Ok(());
    }
    bail!(
      "git stash pop failed: {}",
      String::from_utf8_lossy(&output.stderr).trim()
    );
  }
  Ok(())
}

fn stash_output_has_conflicts(output: &str) -> bool {
  let lower = output.to_ascii_lowercase();
  lower.contains("conflict") || lower.contains("unmerged")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::list_repo_status;
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
      let repo = Repository::init(&path).expect("init git repository");
      let mut config = repo.config().expect("open git config");
      config
        .set_str("user.name", "Reviu Tests")
        .expect("set git user.name");
      config
        .set_str("user.email", "tests@reviu.local")
        .expect("set git user.email");
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

  fn remote_branch_oid(remote_root: &Path, branch_name: &str) -> git2::Oid {
    let refname = format!("refs/heads/{branch_name}");
    Repository::open(remote_root)
      .expect("open remote")
      .refname_to_id(&refname)
      .expect("read remote branch oid")
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
  fn switch_branch_keeps_repo_status_clean_when_target_branch_is_clean() {
    let repo = TempRepo::init("branch-switch-clean-status");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "main\n", "initial");
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
    let _ = commit_text_file(&repo.path, rel_path, "feature\n", "feature commit");
    let feature_entries = list_repo_status(&repo.path).expect("status on feature branch");
    assert!(
      feature_entries.is_empty(),
      "feature branch should stay clean after commit, got: {feature_entries:?}"
    );

    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    let base_entries = list_repo_status(&repo.path).expect("status on base branch");
    assert!(
      base_entries.is_empty(),
      "base branch should stay clean after switch, got: {base_entries:?}"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read checked out file"),
      "main\n"
    );
  }

  #[test]
  fn checkout_detached_target_switches_to_detached_head_for_commit_hash() {
    let repo = TempRepo::init("branch-checkout-detached");
    let oid = commit_text_file(&repo.path, Path::new("README.md"), "hello\n", "initial");

    checkout_detached_target(&repo.path, &oid.to_string()).expect("checkout detached");

    let status = current_branch_status(&repo.path).expect("branch status");
    assert_eq!(status.name, "HEAD");
  }

  #[test]
  fn checkout_detached_target_accepts_tag() {
    let repo = TempRepo::init("branch-checkout-detached-tag");
    let oid = commit_text_file(&repo.path, Path::new("README.md"), "hello\n", "initial");
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let object = repo_handle
      .find_object(oid, None)
      .expect("find commit object");
    repo_handle
      .tag_lightweight("v1", &object, false)
      .expect("create lightweight tag");

    checkout_detached_target(&repo.path, "v1").expect("checkout detached tag");

    let status = current_branch_status(&repo.path).expect("branch status");
    assert_eq!(status.name, "HEAD");
  }

  #[test]
  fn checkout_detached_target_rejects_branch_name() {
    let repo = TempRepo::init("branch-checkout-detached-reject-branch");
    commit_text_file(&repo.path, Path::new("README.md"), "hello\n", "initial");
    let branch_name = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;

    let error = checkout_detached_target(&repo.path, &branch_name).expect_err("reject branch name");
    assert!(format!("{error:#}").contains("commit hash or tag"));
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
  fn detached_head_label_prefers_exact_tag() {
    let repo = TempRepo::init("branch-detached-label-tag");
    let oid = commit_text_file(&repo.path, Path::new("README.md"), "hello\n", "initial");
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let object = repo_handle
      .find_object(oid, None)
      .expect("find commit object");
    repo_handle
      .tag_lightweight("v1.0.0", &object, false)
      .expect("create lightweight tag");

    let label = detached_head_label(&repo.path).expect("detached head label");
    assert_eq!(label, "v1.0.0");
  }

  #[test]
  fn detached_head_label_falls_back_to_short_commit_hash() {
    let repo = TempRepo::init("branch-detached-label-hash");
    let oid = commit_text_file(&repo.path, Path::new("README.md"), "hello\n", "initial");

    let label = detached_head_label(&repo.path).expect("detached head label");
    let expected = oid.to_string().chars().take(7).collect::<String>();
    assert_eq!(label, expected);
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

    fetch(&local.path).expect("fetch remote updates");

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
  fn branch_has_unpublished_commits_is_false_for_new_branch_without_unique_commit() {
    let remote = TempBareRepo::init("branch-unpublished-commits-none-remote");
    let local = TempRepo::init("branch-unpublished-commits-none-local");

    let _ = commit_text_file(&local.path, Path::new("README.md"), "v1\n", "initial");
    let local_repo = Repository::open(&local.path).expect("open local repo");
    local_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&local.path)
      .expect("read local branch status")
      .name;
    push_branch_to_remote(&local.path, &branch_name, "origin");

    create_branch(&local.path, "feature").expect("create feature branch");
    switch_branch(
      &local.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");

    assert!(
      !branch_has_unpublished_commits(&local.path).expect("read unpublished commit state"),
      "new local branch should not publish until it has a unique commit"
    );
  }

  #[test]
  fn branch_has_unpublished_commits_is_true_for_unpublished_branch_with_unique_commit() {
    let remote = TempBareRepo::init("branch-unpublished-commits-remote");
    let local = TempRepo::init("branch-unpublished-commits-local");

    let _ = commit_text_file(&local.path, Path::new("README.md"), "v1\n", "initial");
    let local_repo = Repository::open(&local.path).expect("open local repo");
    local_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&local.path)
      .expect("read local branch status")
      .name;
    push_branch_to_remote(&local.path, &branch_name, "origin");

    create_branch(&local.path, "feature").expect("create feature branch");
    switch_branch(
      &local.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &local.path,
      Path::new("README.md"),
      "v2\n",
      "feature change",
    );

    assert!(
      branch_has_unpublished_commits(&local.path).expect("read unpublished commit state"),
      "unique local commit should enable publish branch actions"
    );
  }

  #[test]
  fn fetch_updates_remote_tracking_refs() {
    let remote = TempBareRepo::init("branch-fetch-remote");
    let source = TempRepo::init("branch-fetch-source");
    let clone_dir = TempDir::new("branch-fetch-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");

    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let tracking_ref = format!("refs/remotes/origin/{base_branch}");
    let before = Repository::open(&clone_dir.path)
      .expect("open clone")
      .refname_to_id(&tracking_ref)
      .expect("read remote-tracking branch before fetch");

    let _ = commit_text_file(
      &source.path,
      Path::new("README.md"),
      "v2\n",
      "source update",
    );
    push_branch_to_remote(&source.path, &base_branch, "origin");
    let expected = remote_branch_oid(&remote.path, &base_branch);

    fetch(&clone_dir.path).expect("fetch updates into clone");

    let after = Repository::open(&clone_dir.path)
      .expect("open clone")
      .refname_to_id(&tracking_ref)
      .expect("read remote-tracking branch after fetch");
    assert_ne!(before, after);
    assert_eq!(after, expected);
  }

  #[test]
  fn current_github_remote_repo_prefers_upstream_remote() {
    let remote = TempBareRepo::init("branch-github-remote-origin");
    let local = TempRepo::init("branch-github-remote-local");

    let _ = commit_text_file(&local.path, Path::new("README.md"), "v1\n", "initial");
    let local_repo = Repository::open(&local.path).expect("open local repo");
    local_repo
      .remote("origin", remote.path.to_str().expect("origin path utf8"))
      .expect("add origin remote");
    local_repo
      .remote("fork", "git@github.com:acme/widget.git")
      .expect("add fork remote");

    let branch_name = current_branch_status(&local.path)
      .expect("read local branch status")
      .name;
    push_branch_to_remote(&local.path, &branch_name, "origin");

    let mut local_branch = local_repo
      .find_branch(&branch_name, BranchType::Local)
      .expect("find local branch");
    let head_oid = local_repo
      .head()
      .expect("read local head")
      .target()
      .expect("local head target");
    local_repo
      .reference(
        &format!("refs/remotes/fork/{branch_name}"),
        head_oid,
        true,
        "test fork upstream",
      )
      .expect("create fork remote-tracking ref");
    local_branch
      .set_upstream(Some(&format!("fork/{branch_name}")))
      .expect("set upstream to fork");

    let remote_repo =
      current_github_remote_repo(&local.path).expect("resolve current github remote repo");
    assert_eq!(
      remote_repo,
      Some(GithubRemoteRepo {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
      })
    );
  }

  #[test]
  fn sync_current_branch_to_head_fast_forwards_clean_branch() {
    let remote = TempBareRepo::init("branch-sync-pr-head-remote");
    let source = TempRepo::init("branch-sync-pr-head-source");
    let clone_dir = TempDir::new("branch-sync-pr-head-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");

    let branch_name = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");

    let clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let mut clone_branch = clone_repo
      .find_branch(&branch_name, BranchType::Local)
      .expect("find clone branch");
    clone_branch
      .set_upstream(Some(&format!("origin/{branch_name}")))
      .expect("set clone upstream");

    let target_oid = commit_text_file(&source.path, Path::new("README.md"), "v2\n", "update");
    push_branch_to_remote(&source.path, &branch_name, "origin");

    let before = current_head_sha(&clone_dir.path)
      .expect("read clone head before sync")
      .expect("clone head should exist");
    assert_ne!(before, target_oid.to_string());

    sync_current_branch_to_head(&clone_dir.path, &branch_name, &target_oid.to_string())
      .expect("sync branch to pull request head");

    let after = current_head_sha(&clone_dir.path)
      .expect("read clone head after sync")
      .expect("clone head should exist");
    assert_eq!(after, target_oid.to_string());
    assert_eq!(
      std::fs::read_to_string(clone_dir.path.join("README.md")).expect("read synced file"),
      "v2\n"
    );
  }

  #[test]
  fn sync_current_branch_to_head_rejects_dirty_worktree() {
    let remote = TempBareRepo::init("branch-sync-pr-head-dirty-remote");
    let source = TempRepo::init("branch-sync-pr-head-dirty-source");
    let clone_dir = TempDir::new("branch-sync-pr-head-dirty-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");

    let branch_name = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");

    let target_oid = commit_text_file(&source.path, Path::new("README.md"), "v2\n", "update");
    push_branch_to_remote(&source.path, &branch_name, "origin");

    std::fs::write(clone_dir.path.join("local.txt"), "dirty\n").expect("write dirty file");

    let error = sync_current_branch_to_head(&clone_dir.path, &branch_name, &target_oid.to_string())
      .expect_err("dirty worktree should reject sync");
    assert!(error.to_string().contains("local changes detected"));
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
  fn is_merge_in_progress_reports_true_when_merge_conflicts_exist() {
    let repo = TempRepo::init("branch-merge-state-conflict");
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
    assert!(
      is_merge_in_progress(&repo.path).expect("read merge state"),
      "merge state should be active after conflict"
    );
  }

  #[test]
  fn abort_merge_clears_merge_state_and_conflict_markers() {
    let repo = TempRepo::init("branch-abort-merge");
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

    let _ = merge_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("merge should fail with conflicts");
    assert!(
      is_merge_in_progress(&repo.path).expect("read merge state"),
      "merge state should be active after conflict"
    );

    abort_merge(&repo.path).expect("abort merge");

    assert!(
      !is_merge_in_progress(&repo.path).expect("read merge state after abort"),
      "merge state should be cleaned after abort"
    );
    let readme = std::fs::read_to_string(repo.path.join("README.md")).expect("read README");
    assert!(
      !readme.contains("<<<<<<<"),
      "merge markers should be removed after abort: {readme}"
    );
    assert_eq!(readme, "main change\n");
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
    let conflicted =
      std::fs::read_to_string(repo.path.join("README.md")).expect("read conflicted README");
    assert!(
      conflicted.contains("<<<<<<<"),
      "expected conflict markers in README: {conflicted}"
    );
    assert!(
      conflicted.contains("feature change"),
      "expected feature change text in README: {conflicted}"
    );
    assert!(
      conflicted.contains("main change"),
      "expected main change text in README: {conflicted}"
    );
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );
    assert_eq!(
      current_rebase_commit_message(&repo.path).expect("read current rebase commit message"),
      Some("main change".to_string())
    );
  }

  #[test]
  fn abort_rebase_clears_rebase_state_and_conflict_markers() {
    let repo = TempRepo::init("branch-abort-rebase");
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

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );
    assert_eq!(
      current_rebase_commit_message(&repo.path).expect("read current rebase commit message"),
      Some("main change".to_string())
    );

    abort_rebase(&repo.path).expect("abort rebase");

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after abort"),
      "rebase state should be cleaned after abort"
    );
    assert_eq!(
      current_rebase_commit_message(&repo.path).expect("read current rebase commit message"),
      None
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read README after abort"),
      "main change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after abort rebase")
        .name,
      base_branch
    );
  }

  #[test]
  fn continue_rebase_completes_after_conflict_resolution() {
    let repo = TempRepo::init("branch-continue-rebase");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );
    assert_eq!(
      current_rebase_commit_message(&repo.path).expect("read current rebase commit message"),
      Some("main change".to_string())
    );

    std::fs::write(repo.path.join(rel_path), "resolved\n").expect("write resolved contents");
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let mut index = repo_handle.index().expect("open index");
    index.add_path(rel_path).expect("stage resolved file");
    index.write().expect("write index");

    continue_rebase(&repo.path).expect("continue rebase");

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after continue"),
      "rebase state should be cleaned after continue"
    );
    assert_eq!(
      current_rebase_commit_message(&repo.path).expect("read current rebase commit message"),
      None
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read README after continue"),
      "resolved\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after continue rebase")
        .name,
      base_branch
    );
  }

  #[test]
  fn continue_rebase_after_cli_interactive_rebase_conflict_uses_command_fallback() {
    let repo = TempRepo::init("branch-continue-cli-interactive-rebase");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
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
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");

    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    force_checkout_head(&repo.path);

    let target = crate::InteractiveRebaseTarget::Branch(BranchRef {
      name: "feature".to_string(),
      kind: BranchKind::Local,
    });
    let commits = crate::list_interactive_rebase_commits(&repo.path, &target)
      .expect("list commits to rebase")
      .commits;
    assert_eq!(commits.len(), 1);
    let todo = vec![crate::InteractiveRebaseTodoEntry {
      oid: commits[0].oid.clone(),
      action: crate::InteractiveRebaseAction::Pick,
    }];

    let _ = crate::start_interactive_rebase(&repo.path, &target, &todo)
      .expect_err("interactive rebase should stop on conflict");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after interactive rebase conflict"
    );

    std::fs::write(repo.path.join(rel_path), "resolved\n").expect("write resolved contents");
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let mut index = repo_handle.index().expect("open index");
    index.add_path(rel_path).expect("stage resolved file");
    index.write().expect("write index");

    continue_rebase(&repo.path).expect("continue rebase");

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after continue"),
      "rebase state should be cleaned after continue"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read README after continue"),
      "resolved\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after continue rebase")
        .name,
      base_branch
    );
  }

  #[test]
  fn skip_rebase_skips_conflicted_commit_and_completes_rebase() {
    let repo = TempRepo::init("branch-skip-rebase");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );

    skip_rebase(&repo.path).expect("skip rebase");

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after skip"),
      "rebase state should be cleaned after skip"
    );
    assert_eq!(
      current_rebase_commit_message(&repo.path).expect("read current rebase commit message"),
      None
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read README after skip"),
      "feature change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after skip rebase")
        .name,
      base_branch
    );
  }

  #[test]
  fn continue_rebase_skips_already_applied_commits_after_conflict_resolution() {
    let repo = TempRepo::init("branch-continue-rebase-applied");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, rel_path, "main conflict\n", "main conflict");
    let _ = commit_text_file(&repo.path, rel_path, "final\n", "main final");

    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    let _ = commit_text_file(&repo.path, rel_path, "final\n", "feature final");

    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should stop on conflict before already-applied commits");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );
    assert_eq!(
      current_rebase_commit_message(&repo.path).expect("read current rebase commit message"),
      Some("main conflict".to_string())
    );

    std::fs::write(repo.path.join(rel_path), "final\n").expect("write resolved contents");
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let mut index = repo_handle.index().expect("open index");
    index.add_path(rel_path).expect("stage resolved file");
    index.write().expect("write index");

    continue_rebase(&repo.path).expect("continue rebase while skipping already-applied commits");

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after continue"),
      "rebase state should be cleaned after continue"
    );
    assert_eq!(
      current_rebase_commit_message(&repo.path).expect("read current rebase commit message"),
      None
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read README after continue"),
      "final\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after continue rebase")
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
  fn switch_to_branch_name_switches_to_existing_local_branch() {
    let repo = TempRepo::init("branch-switch-by-name-local");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    create_branch(&repo.path, "feature").expect("create local branch");
    switch_to_branch_name(&repo.path, "feature").expect("switch to local branch by name");

    let status = current_branch_status(&repo.path).expect("branch status after switch");
    assert_eq!(status.name, "feature");
  }

  #[test]
  fn switch_to_branch_name_fetches_and_switches_to_remote_branch() {
    let remote = TempBareRepo::init("branch-switch-by-name-remote-origin");
    let source = TempRepo::init("branch-switch-by-name-remote-source");
    let clone_dir = TempDir::new("branch-switch-by-name-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");

    create_branch(&source.path, "feature/switch-me").expect("create source feature branch");
    switch_branch(
      &source.path,
      &BranchRef {
        name: "feature/switch-me".to_string(),
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
    push_branch_to_remote(&source.path, "feature/switch-me", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");

    switch_to_branch_name(&clone_dir.path, "feature/switch-me")
      .expect("switch to remote branch by name");

    let status = current_branch_status(&clone_dir.path).expect("branch status after switch");
    assert_eq!(status.name, "feature/switch-me");

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone repo");
    let local_feature = clone_repo
      .find_branch("feature/switch-me", BranchType::Local)
      .expect("find local feature branch");
    let upstream = local_feature
      .upstream()
      .expect("feature branch upstream")
      .name()
      .expect("upstream name")
      .expect("non-empty upstream")
      .to_string();
    assert_eq!(upstream, "origin/feature/switch-me");
  }

  #[test]
  fn create_branch_from_remote_creates_branch_without_upstream() {
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
    assert!(local_branch.upstream().is_err());
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

  #[test]
  fn delete_branch_removes_local_branch() {
    let repo = TempRepo::init("branch-delete-local");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    delete_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("delete merged local branch");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    assert!(
      repo_handle
        .find_branch("feature", BranchType::Local)
        .is_err()
    );
  }

  #[test]
  fn delete_branch_rejects_current_branch() {
    let repo = TempRepo::init("branch-delete-current");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let current_branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;

    let error = delete_branch(
      &repo.path,
      &BranchRef {
        name: current_branch,
        kind: BranchKind::Local,
      },
    )
    .expect_err("current branch delete should fail");

    assert_eq!(error.to_string(), "cannot delete the current branch");
  }

  #[test]
  fn delete_branch_rejects_remote_branch() {
    let repo = TempRepo::init("branch-delete-remote");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let error = delete_branch(
      &repo.path,
      &BranchRef {
        name: "invalid-remote-branch".to_string(),
        kind: BranchKind::Remote,
      },
    )
    .expect_err("invalid remote branch delete should fail");

    assert_eq!(
      error.to_string(),
      "invalid remote branch \"invalid-remote-branch\""
    );
  }

  #[test]
  fn delete_branch_removes_remote_branch() {
    let remote = TempBareRepo::init("branch-delete-remote-origin");
    let source = TempRepo::init("branch-delete-remote-source");
    let clone_dir = TempDir::new("branch-delete-remote-clone");

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

    delete_branch(
      &clone_dir.path,
      &BranchRef {
        name: "origin/feature".to_string(),
        kind: BranchKind::Remote,
      },
    )
    .expect("delete remote branch");

    let remote_repo = Repository::open(&remote.path).expect("open remote");
    assert!(remote_repo.refname_to_id("refs/heads/feature").is_err());
    assert!(
      !list_branches(&clone_dir.path)
        .expect("list branches after remote delete")
        .iter()
        .any(|branch| branch.kind == BranchKind::Remote && branch.name == "origin/feature")
    );
  }

  #[test]
  fn delete_branch_removes_unmerged_branch() {
    let repo = TempRepo::init("branch-delete-unmerged");
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
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch,
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");

    delete_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("force delete unmerged branch");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    assert!(
      repo_handle
        .find_branch("feature", BranchType::Local)
        .is_err()
    );
  }

  #[test]
  fn create_and_apply_stash_restores_tracked_changes() {
    let repo = TempRepo::init("branch-stash-apply");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");

    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write tracked change");
    create_stash(&repo.path, false, None).expect("create stash");

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after stash"),
      "v1\n"
    );

    let stashes = list_stashes(&repo.path).expect("list stashes after create");
    assert_eq!(stashes.len(), 1);

    apply_stash(&repo.path, stashes[0].index).expect("apply stash");

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after apply"),
      "v2\n"
    );
    assert_eq!(
      list_stashes(&repo.path)
        .expect("list stashes after apply")
        .len(),
      1
    );
  }

  #[test]
  fn default_stash_message_uses_head_branch_and_summary() {
    let repo = TempRepo::init("branch-stash-default-message");
    let head_oid = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;
    let short_head = head_oid.to_string().chars().take(7).collect::<String>();

    let default_message = default_stash_message(&repo.path).expect("read default stash message");
    assert_eq!(
      default_message,
      format!("WIP on {branch}: {short_head} initial")
    );
  }

  #[test]
  fn create_stash_uses_custom_message_when_provided() {
    let repo = TempRepo::init("branch-stash-custom-message");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write tracked change");

    create_stash(&repo.path, false, Some("checkpoint before refactor"))
      .expect("create stash with custom message");

    let stash = list_stashes(&repo.path)
      .expect("list stashes after create")
      .into_iter()
      .next()
      .expect("stash entry exists");
    assert!(
      stash.name.contains("checkpoint before refactor"),
      "stash name should contain custom message, got: {}",
      stash.name
    );
  }

  #[test]
  fn pop_stash_restores_changes_and_removes_entry() {
    let repo = TempRepo::init("branch-stash-pop");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");

    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write tracked change");
    create_stash(&repo.path, false, None).expect("create stash");

    let stashes = list_stashes(&repo.path).expect("list stashes");
    assert_eq!(stashes.len(), 1);

    pop_stash(&repo.path, stashes[0].index).expect("pop stash");

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after pop"),
      "v2\n"
    );
    assert!(
      list_stashes(&repo.path)
        .expect("list stashes after pop")
        .is_empty()
    );
  }

  #[test]
  fn drop_stash_removes_entry_without_applying() {
    let repo = TempRepo::init("branch-stash-drop");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");

    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write tracked change");
    create_stash(&repo.path, false, None).expect("create stash");

    let stashes = list_stashes(&repo.path).expect("list stashes");
    assert_eq!(stashes.len(), 1);
    drop_stash(&repo.path, stashes[0].index).expect("drop stash");

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after drop"),
      "v1\n"
    );
    assert!(
      list_stashes(&repo.path)
        .expect("list stashes after drop")
        .is_empty()
    );
  }

  #[test]
  fn create_stash_with_untracked_stashes_and_restores_untracked_file() {
    let repo = TempRepo::init("branch-stash-untracked");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let rel_path = Path::new("notes.txt");
    std::fs::write(repo.path.join(rel_path), "notes\n").expect("write untracked file");

    create_stash(&repo.path, true, None).expect("create stash with untracked");

    assert!(
      !repo.path.join(rel_path).exists(),
      "untracked file should be removed from worktree after stash"
    );

    let stashes = list_stashes(&repo.path).expect("list stashes");
    assert_eq!(stashes.len(), 1);

    pop_stash(&repo.path, stashes[0].index).expect("pop stash with untracked");

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read restored untracked file"),
      "notes\n"
    );
    assert!(
      list_stashes(&repo.path)
        .expect("list stashes after pop")
        .is_empty()
    );
  }

  #[test]
  fn apply_stash_with_conflicts_succeeds() {
    let repo = TempRepo::init("branch-stash-apply-conflict");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "line1\nline2\n", "initial");

    // Modify and stash
    std::fs::write(repo.path.join(rel_path), "line1\nstashed change\n").expect("write for stash");
    create_stash(&repo.path, false, None).expect("create stash");

    // Make a conflicting commit
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      "line1\ncommitted change\n",
      "conflict",
    );

    // Apply stash — should succeed even though there are conflicts
    let result = apply_stash(&repo.path, 0);
    assert!(
      result.is_ok(),
      "apply_stash should return Ok when conflicts occur, got: {:?}",
      result.err()
    );

    // Working tree should contain conflict markers
    let content = std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after apply");
    assert!(
      content.contains("<<<<<<<") && content.contains(">>>>>>>"),
      "file should contain conflict markers, got: {:?}",
      content
    );

    // Stash should still be in the list (apply doesn't remove it)
    assert_eq!(list_stashes(&repo.path).expect("list stashes").len(), 1);
  }

  #[test]
  fn pop_stash_with_conflicts_succeeds() {
    let repo = TempRepo::init("branch-stash-pop-conflict");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "line1\nline2\n", "initial");

    // Modify and stash
    std::fs::write(repo.path.join(rel_path), "line1\nstashed change\n").expect("write for stash");
    create_stash(&repo.path, false, None).expect("create stash");

    // Make a conflicting commit
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      "line1\ncommitted change\n",
      "conflict",
    );

    // Pop stash — should succeed even though there are conflicts
    let result = pop_stash(&repo.path, 0);
    assert!(
      result.is_ok(),
      "pop_stash should return Ok when conflicts occur, got: {:?}",
      result.err()
    );

    // Working tree should contain conflict markers
    let content = std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after pop");
    assert!(
      content.contains("<<<<<<<") && content.contains(">>>>>>>"),
      "file should contain conflict markers, got: {:?}",
      content
    );
  }
}
