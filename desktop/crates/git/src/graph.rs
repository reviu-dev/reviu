use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};
use git2::{BranchType, Oid, Repository, Sort};

#[derive(Clone, Debug)]
pub struct CommitGraphNode {
  pub oid: String,
  pub short_oid: String,
  pub summary: String,
  pub author: String,
  pub parent_oids: Vec<String>,
  pub refs: Vec<String>,
}

pub fn list_commit_graph(repo_root: &Path, limit: usize) -> Result<Vec<CommitGraphNode>> {
  if limit == 0 {
    return Ok(Vec::new());
  }

  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut refs_by_oid = refs_by_oid(&repo)?;

  let mut walk = repo.revwalk()?;
  // Match git graph-style traversal: strict topological order.
  // Combining with TIME (date-order) can interleave branches differently.
  walk.set_sorting(Sort::TOPOLOGICAL)?;

  let Some((head_oid, head_label)) = head_target(&repo) else {
    return Ok(Vec::new());
  };
  if walk.push(head_oid).is_err() {
    return Ok(Vec::new());
  }
  insert_ref_label(&mut refs_by_oid, head_oid, head_label);

  let mut rows = Vec::with_capacity(limit);
  for oid_result in walk.take(limit) {
    let oid = oid_result?;
    let Ok(commit) = repo.find_commit(oid) else {
      continue;
    };

    let mut refs = refs_by_oid.remove(&oid).unwrap_or_default();
    refs.sort();
    refs.dedup();

    let summary = commit
      .summary()
      .unwrap_or("No commit message")
      .replace(['\n', '\r'], "");
    let author = commit.author().name().unwrap_or("Unknown").to_string();
    let parent_oids = commit
      .parent_ids()
      .map(|parent| parent.to_string())
      .collect();

    rows.push(CommitGraphNode {
      oid: oid.to_string(),
      short_oid: short_oid(oid),
      summary,
      author,
      parent_oids,
      refs,
    });
  }

  Ok(rows)
}

fn refs_by_oid(repo: &Repository) -> Result<BTreeMap<Oid, Vec<String>>> {
  let mut by_oid = BTreeMap::new();

  for branch in repo.branches(Some(BranchType::Local))? {
    let (branch, _) = branch?;
    let name = branch.name()?.unwrap_or("").to_string();
    if name.is_empty() {
      continue;
    }
    if let Some(oid) = branch.get().target() {
      insert_ref_label(&mut by_oid, oid, name);
    }
  }

  for branch in repo.branches(Some(BranchType::Remote))? {
    let (branch, _) = branch?;
    let name = branch.name()?.unwrap_or("").to_string();
    if name.is_empty() || name.ends_with("/HEAD") {
      continue;
    }
    if let Some(oid) = branch.get().target() {
      insert_ref_label(&mut by_oid, oid, name);
    }
  }

  Ok(by_oid)
}

fn head_target(repo: &Repository) -> Option<(Oid, String)> {
  let head = repo.head().ok()?;
  let oid = head
    .target()
    .or_else(|| head.peel_to_commit().ok().map(|commit| commit.id()))?;
  let label = if head.is_branch() {
    match head.shorthand() {
      Some(name) if !name.is_empty() => format!("HEAD -> {name}"),
      _ => "HEAD".to_string(),
    }
  } else {
    "HEAD".to_string()
  };

  Some((oid, label))
}

fn short_oid(oid: Oid) -> String {
  let value = oid.to_string();
  value.chars().take(7).collect()
}

fn insert_ref_label(by_oid: &mut BTreeMap<Oid, Vec<String>>, oid: Oid, label: String) {
  if label.is_empty() {
    return;
  }
  by_oid.entry(oid).or_default().push(label);
}
