use std::path::Path;

use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{BranchType, Repository, Signature};

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

  if head.is_branch() {
    if let Ok(branch) = repo.find_branch(&name, BranchType::Local) {
      if let Ok(upstream) = branch.upstream() {
        has_upstream = true;
        if let (Some(local_oid), Some(upstream_oid)) =
          (branch.get().target(), upstream.get().target())
        {
          let (a, b) = repo.graph_ahead_behind(local_oid, upstream_oid)?;
          ahead = a;
          behind = b;
        }
      }
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
        .last()
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
  if base.kind == BranchKind::Remote {
    if let Ok(mut local_branch) = repo.find_branch(name, BranchType::Local) {
      let _ = local_branch.set_upstream(Some(&base.name));
    }
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
    checkout.safe();
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
