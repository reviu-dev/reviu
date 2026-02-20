use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{
  BranchType, CherrypickOptions, Cred, ErrorCode, FetchOptions, Rebase, RemoteCallbacks,
  Repository, RepositoryState, ResetType, Signature, StashFlags,
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
    return Ok(());
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
  Ok(())
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
  let mut repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let signature = repo_signature(&repo)?;

  let flags = include_untracked.then_some(StashFlags::INCLUDE_UNTRACKED);
  let message = message.map(str::trim).filter(|message| !message.is_empty());
  repo
    .stash_save2(&signature, message, flags)
    .context("create stash entry")?;

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
  let mut repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  repo
    .stash_apply(index, None)
    .with_context(|| format!("apply stash at index {index}"))?;
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
  let mut repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  repo
    .stash_pop(index, None)
    .with_context(|| format!("pop stash at index {index}"))?;
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
    let commits =
      crate::list_interactive_rebase_commits(&repo.path, &target).expect("list commits to rebase");
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
}
