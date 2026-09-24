//! Archived worktrees: the checkout leaves the disk, its exact state stays in
//! git and comes back on restore. Two detached commits carry it (zed's
//! recipe): the index as "staged", then everything on disk as "unstaged" on
//! top. One ref per archive pins the chain against gc and carries the
//! metadata in its commit message, so an archive needs nothing outside git.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use git2::Repository;

use crate::checkpoint::{run_git, temp_index_path};
use crate::worktree::{
  WORKTREE_BRANCH_PREFIX, belongs_to_repository, remove_worktree_checkout, worktree_current_branch,
};

const ARCHIVE_REF_ROOT: &str = "refs/reviu/archived-worktrees";
const PATH_TRAILER: &str = "Reviu-Worktree-Path: ";
const BRANCH_TRAILER: &str = "Reviu-Worktree-Branch: ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchivedWorktree {
  pub ref_name: String,
  pub path: PathBuf,
  /// None when the worktree sat on a detached HEAD.
  pub branch: Option<String>,
  pub original_head: String,
  pub staged_commit: String,
  pub unstaged_commit: String,
  pub archived_at_secs: u64,
}

/// Snapshots the worktree, pins the snapshot, then takes the checkout off
/// disk. Its branch is kept: nothing moved it, and restore checks it out.
pub fn archive_worktree(repo_root: &Path, worktree_path: &Path) -> Result<ArchivedWorktree> {
  if !belongs_to_repository(repo_root, worktree_path) {
    bail!("{worktree_path:?} is not a linked worktree of {repo_root:?}");
  }
  let original_head = run_git(worktree_path, &["rev-parse", "HEAD"], &[])
    .context("the worktree has no commit to archive from")?;
  let branch = worktree_current_branch(worktree_path);

  let staged_tree = run_git(worktree_path, &["write-tree"], &[])
    .context("resolve the worktree's conflicts before archiving it")?;
  let staged_commit = run_git(
    worktree_path,
    &[
      "commit-tree",
      &staged_tree,
      "-p",
      &original_head,
      "-m",
      "reviu archived worktree (staged)",
    ],
    &[],
  )?;

  let mut message = format!(
    "reviu archived worktree\n\n{PATH_TRAILER}{}",
    worktree_path.display()
  );
  if let Some(branch) = &branch {
    message.push_str(&format!("\n{BRANCH_TRAILER}{branch}"));
  }
  let index_path = temp_index_path(worktree_path);
  let _ = std::fs::remove_file(&index_path);
  let index_env = index_path.to_string_lossy().into_owned();
  let env: &[(&str, &str)] = &[("GIT_INDEX_FILE", index_env.as_str())];
  let unstaged_commit = (|| -> Result<String> {
    run_git(worktree_path, &["add", "-A", "--", "."], env)?;
    let full_tree = run_git(worktree_path, &["write-tree"], env)?;
    run_git(
      worktree_path,
      &[
        "commit-tree",
        &full_tree,
        "-p",
        &staged_commit,
        "-m",
        &message,
      ],
      env,
    )
  })();
  let _ = std::fs::remove_file(&index_path);
  let unstaged_commit = unstaged_commit?;

  let archived_at_ms = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|duration| duration.as_millis())
    .unwrap_or(0);
  let ref_name = format!("{ARCHIVE_REF_ROOT}/{archived_at_ms}");
  // Without the ref, gc collects the WIP commits and a later restore fails.
  run_git(repo_root, &["update-ref", &ref_name, &unstaged_commit], &[])?;
  if let Err(error) = remove_worktree_checkout(repo_root, worktree_path) {
    let _ = run_git(repo_root, &["update-ref", "-d", &ref_name], &[]);
    return Err(error);
  }

  Ok(ArchivedWorktree {
    ref_name,
    path: worktree_path.to_path_buf(),
    branch,
    original_head,
    staged_commit,
    unstaged_commit,
    archived_at_secs: (archived_at_ms / 1000) as u64,
  })
}

/// Newest first.
pub fn list_archived_worktrees(repo_root: &Path) -> Result<Vec<ArchivedWorktree>> {
  let repo = Repository::open(repo_root).with_context(|| format!("open repo at {repo_root:?}"))?;
  let mut archives = Vec::new();
  for reference in repo.references_glob(&format!("{ARCHIVE_REF_ROOT}/*"))? {
    let reference = reference?;
    let Ok(ref_name) = reference.name().map(str::to_string) else {
      continue;
    };
    let Ok(unstaged) = reference.peel_to_commit() else {
      continue;
    };
    let Ok(staged) = unstaged.parent(0) else {
      continue;
    };
    let Ok(original) = staged.parent(0) else {
      continue;
    };
    let message = unstaged.message().unwrap_or_default();
    let trailer = |prefix: &str| {
      message
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .map(str::to_string)
    };
    let Some(path) = trailer(PATH_TRAILER) else {
      continue;
    };
    archives.push(ArchivedWorktree {
      path: PathBuf::from(path),
      branch: trailer(BRANCH_TRAILER),
      original_head: original.id().to_string(),
      staged_commit: staged.id().to_string(),
      unstaged_commit: unstaged.id().to_string(),
      archived_at_secs: u64::try_from(unstaged.time().seconds()).unwrap_or(0),
      ref_name,
    });
  }
  archives.sort_by(|a, b| b.ref_name.cmp(&a.ref_name));
  Ok(archives)
}

/// Brings the checkout back at its old path, on its branch when the branch
/// has not moved since (detached at the archived commit otherwise), with the
/// working tree and index exactly as archived. The archive is consumed.
pub fn restore_archived_worktree(repo_root: &Path, archived: &ArchivedWorktree) -> Result<()> {
  let path = &archived.path;
  if path.exists() {
    bail!("{} already exists", path.display());
  }
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent).with_context(|| format!("create {parent:?}"))?;
  }
  let path_arg = path.to_string_lossy().into_owned();
  run_git(
    repo_root,
    &[
      "worktree",
      "add",
      "--detach",
      &path_arg,
      &archived.original_head,
    ],
    &[],
  )
  .with_context(|| format!("recreate the worktree at {path:?}"))?;

  let restored = (|| -> Result<()> {
    if let Some(branch) = &archived.branch {
      reattach_branch(repo_root, path, branch, &archived.original_head);
    }
    // --reset -u lays the full tree on disk, deletions included; the second
    // read-tree only sets the index, leaving the files alone.
    run_git(
      path,
      &["read-tree", "--reset", "-u", &archived.unstaged_commit],
      &[],
    )?;
    run_git(path, &["read-tree", &archived.staged_commit], &[])?;
    Ok(())
  })();
  if let Err(error) = restored {
    let _ = remove_worktree_checkout(repo_root, path);
    return Err(error.context("restore the archived state"));
  }
  run_git(repo_root, &["update-ref", "-d", &archived.ref_name], &[])?;
  Ok(())
}

/// Best-effort: a branch that moved, or is checked out elsewhere, leaves the
/// worktree detached at the archived commit, which is still exactly right.
fn reattach_branch(repo_root: &Path, path: &Path, branch: &str, original_head: &str) {
  let branch_ref = format!("refs/heads/{branch}");
  match run_git(repo_root, &["rev-parse", "--verify", &branch_ref], &[]) {
    Ok(tip) if tip == original_head => {
      let _ = run_git(path, &["checkout", branch], &[]);
    }
    Ok(_) => {}
    Err(_) => {
      let _ = run_git(path, &["checkout", "-b", branch], &[]);
    }
  }
}

/// Drops the archive for good, and its `reviu-` branch when that branch still
/// points where the archive left it (a moved branch holds newer work).
pub fn delete_archived_worktree(repo_root: &Path, archived: &ArchivedWorktree) -> Result<()> {
  run_git(repo_root, &["update-ref", "-d", &archived.ref_name], &[])?;
  if let Some(branch) = &archived.branch
    && branch.starts_with(WORKTREE_BRANCH_PREFIX)
  {
    let branch_ref = format!("refs/heads/{branch}");
    if run_git(repo_root, &["rev-parse", "--verify", &branch_ref], &[])
      .is_ok_and(|tip| tip == archived.original_head)
    {
      let _ = run_git(repo_root, &["branch", "-D", branch], &[]);
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use crate::{create_worktree, list_worktrees, worktrees_root_for};
  use std::process::Command;

  fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
      .current_dir(path)
      .args(args)
      .output()
      .expect("run git");
    assert!(
      output.status.success(),
      "git {args:?} failed: {}",
      String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
  }

  fn cleanup(repo_root: &Path) {
    if let Ok(root) = worktrees_root_for(repo_root) {
      let _ = std::fs::remove_dir_all(root);
    }
  }

  #[test]
  fn archive_then_restore_brings_back_the_exact_working_tree_and_index() {
    let repo = TempRepo::init("archive-roundtrip");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("gone.txt"), "bye\n", "to delete");
    let created = create_worktree(&repo.path, None).expect("create worktree");
    let worktree = &created.path;
    commit_text_file(
      worktree,
      Path::new("agent.txt"),
      "committed\n",
      "agent work",
    );
    std::fs::write(worktree.join("staged.txt"), "staged\n").expect("write staged");
    git(worktree, &["add", "staged.txt"]);
    std::fs::write(worktree.join("README.md"), "v2 unstaged\n").expect("modify");
    std::fs::write(worktree.join("untracked.txt"), "new\n").expect("untracked");
    std::fs::remove_file(worktree.join("gone.txt")).expect("delete");
    let head_before = git(worktree, &["rev-parse", "HEAD"]);

    let archived = archive_worktree(&repo.path, worktree).expect("archive");

    assert!(!worktree.exists(), "the checkout left the disk");
    assert!(list_worktrees(&repo.path).expect("list").is_empty());
    assert_eq!(
      list_archived_worktrees(&repo.path).expect("list archives"),
      vec![archived.clone()]
    );
    assert_eq!(archived.branch.as_deref(), Some(created.branch.as_str()));
    assert_eq!(archived.path, *worktree);
    assert!(
      git(&repo.path, &["branch", "--list", &created.branch]).contains(&created.branch),
      "archiving keeps the branch"
    );

    restore_archived_worktree(&repo.path, &archived).expect("restore");

    assert_eq!(git(worktree, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
      git(worktree, &["branch", "--show-current"]),
      created.branch,
      "back on its branch"
    );
    assert_eq!(
      std::fs::read_to_string(worktree.join("README.md")).expect("read"),
      "v2 unstaged\n"
    );
    assert_eq!(
      std::fs::read_to_string(worktree.join("untracked.txt")).expect("read"),
      "new\n"
    );
    assert!(!worktree.join("gone.txt").exists());
    assert_eq!(
      git(worktree, &["diff", "--cached", "--name-only"]),
      "staged.txt",
      "only the staged file is in the index"
    );
    assert!(
      git(worktree, &["status", "--porcelain"]).contains("?? untracked.txt"),
      "the untracked file is untracked again"
    );
    assert!(
      list_archived_worktrees(&repo.path)
        .expect("list after restore")
        .is_empty(),
      "restoring consumes the archive"
    );

    cleanup(&repo.path);
  }

  #[test]
  fn restore_stays_detached_when_the_branch_moved_meanwhile() {
    let repo = TempRepo::init("archive-branch-moved");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");
    let archived = archive_worktree(&repo.path, &created.path).expect("archive");
    git(&repo.path, &["switch", &created.branch]);
    commit_text_file(&repo.path, Path::new("later.txt"), "later\n", "moved on");
    git(&repo.path, &["switch", "-"]);

    restore_archived_worktree(&repo.path, &archived).expect("restore");

    assert_eq!(
      git(&created.path, &["rev-parse", "HEAD"]),
      archived.original_head
    );
    assert_eq!(git(&created.path, &["branch", "--show-current"]), "");

    cleanup(&repo.path);
  }

  #[test]
  fn deleting_an_archive_drops_its_ref_and_its_untouched_reviu_branch() {
    let repo = TempRepo::init("archive-delete");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");
    let archived = archive_worktree(&repo.path, &created.path).expect("archive");

    delete_archived_worktree(&repo.path, &archived).expect("delete");

    assert!(
      list_archived_worktrees(&repo.path)
        .expect("list")
        .is_empty()
    );
    assert_eq!(git(&repo.path, &["branch", "--list", &created.branch]), "");

    cleanup(&repo.path);
  }

  #[test]
  fn a_new_worktree_never_takes_the_path_of_an_archived_one() {
    let repo = TempRepo::init("archive-path-reserved");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");
    // Renamed, so the kept branch no longer reserves the folder name.
    git(&created.path, &["branch", "-m", "reviu-renamed"]);
    let archived = archive_worktree(&repo.path, &created.path).expect("archive");

    for _ in 0..30 {
      let other = create_worktree(&repo.path, None).expect("create another");
      assert_ne!(other.path, archived.path);
    }

    cleanup(&repo.path);
  }
}
