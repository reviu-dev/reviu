//! Working-tree checkpoints under hidden refs: snapshot before each agent turn,
//! restore on rollback. Never touches HEAD, the current branch, or the user's index.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use git2::{ObjectType, Repository, TreeWalkMode, TreeWalkResult};

use crate::status::list_repo_worktree_files;

const CHECKPOINT_REF_ROOT: &str = "refs/reviu/checkpoints";
const KEEP_CHECKPOINTS_PER_SESSION: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
  pub ref_name: String,
  pub created_at_ms: u64,
}

fn now_ms() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_millis() as u64)
    .unwrap_or(0)
}

fn run_git(repo_root: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
  let mut command = Command::new("git");
  command.current_dir(repo_root).args(args);
  for (key, value) in env {
    command.env(key, value);
  }
  let output = command
    .output()
    .with_context(|| format!("run git {}", args.join(" ")))?;
  if !output.status.success() {
    bail!(
      "git {} failed: {}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr).trim()
    );
  }
  Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn temp_index_path(repo_root: &Path) -> PathBuf {
  std::env::temp_dir().join(format!(
    "reviu-checkpoint-index-{}-{}",
    std::process::id(),
    blake3::hash(repo_root.to_string_lossy().as_bytes()).to_hex()
  ))
}

/// Snapshot the full working tree (untracked included, ignored excluded) as a commit
/// under a hidden ref. Uses a temporary index so the user's index is untouched.
pub fn create_checkpoint(repo_root: &Path, session_id: &str) -> Result<Checkpoint> {
  let index_path = temp_index_path(repo_root);
  let _ = std::fs::remove_file(&index_path);
  let index_env = index_path.to_string_lossy().into_owned();
  let env: &[(&str, &str)] = &[("GIT_INDEX_FILE", index_env.as_str())];

  let result = (|| -> Result<Checkpoint> {
    run_git(repo_root, &["add", "-A", "--", "."], env)?;
    let tree_oid = run_git(repo_root, &["write-tree"], env)?;

    let repo = Repository::open(repo_root).with_context(|| format!("open repo {repo_root:?}"))?;
    let head_arg = repo
      .head()
      .ok()
      .and_then(|head| head.peel_to_commit().ok())
      .map(|commit| commit.id().to_string());

    let created_at_ms = now_ms();
    let message = format!("reviu checkpoint {session_id} {created_at_ms}");
    let commit_oid = match head_arg.as_deref() {
      Some(parent) => run_git(
        repo_root,
        &["commit-tree", &tree_oid, "-p", parent, "-m", &message],
        env,
      )?,
      None => run_git(repo_root, &["commit-tree", &tree_oid, "-m", &message], env)?,
    };

    let ref_name = format!("{CHECKPOINT_REF_ROOT}/{session_id}/{created_at_ms}");
    run_git(repo_root, &["update-ref", &ref_name, &commit_oid], &[])?;

    prune_checkpoints(repo_root, session_id, KEEP_CHECKPOINTS_PER_SESSION)?;

    Ok(Checkpoint {
      ref_name,
      created_at_ms,
    })
  })();

  let _ = std::fs::remove_file(&index_path);
  result
}

pub fn list_checkpoints(repo_root: &Path, session_id: &str) -> Result<Vec<Checkpoint>> {
  let repo = Repository::open(repo_root).with_context(|| format!("open repo {repo_root:?}"))?;
  let prefix = format!("{CHECKPOINT_REF_ROOT}/{session_id}/");
  let mut checkpoints = Vec::new();
  for reference in repo.references_glob(&format!("{prefix}*"))? {
    let reference = reference?;
    let Some(ref_name) = reference.name() else {
      continue;
    };
    let created_at_ms = ref_name
      .rsplit('/')
      .next()
      .and_then(|ts| ts.parse::<u64>().ok())
      .unwrap_or(0);
    checkpoints.push(Checkpoint {
      ref_name: ref_name.to_string(),
      created_at_ms,
    });
  }
  checkpoints.sort_by_key(|checkpoint| checkpoint.created_at_ms);
  Ok(checkpoints)
}

pub fn prune_checkpoints(repo_root: &Path, session_id: &str, keep_last: usize) -> Result<usize> {
  let checkpoints = list_checkpoints(repo_root, session_id)?;
  if checkpoints.len() <= keep_last {
    return Ok(0);
  }
  let repo = Repository::open(repo_root).with_context(|| format!("open repo {repo_root:?}"))?;
  let excess = checkpoints.len() - keep_last;
  for checkpoint in checkpoints.into_iter().take(excess) {
    if let Ok(mut reference) = repo.find_reference(&checkpoint.ref_name) {
      let _ = reference.delete();
    }
  }
  Ok(excess)
}

/// Delete every checkpoint ref of a session (e.g. when the conversation is deleted).
pub fn delete_session_checkpoints(repo_root: &Path, session_id: &str) -> Result<()> {
  prune_checkpoints(repo_root, session_id, 0).map(|_| ())
}

fn checkpoint_tree_files(repo: &Repository, ref_name: &str) -> Result<HashSet<PathBuf>> {
  let commit = repo
    .find_reference(ref_name)?
    .peel_to_commit()
    .with_context(|| format!("resolve checkpoint {ref_name}"))?;
  let tree = commit.tree()?;
  let mut files = HashSet::new();
  tree.walk(TreeWalkMode::PreOrder, |root, entry| {
    if entry.kind() == Some(ObjectType::Blob)
      && let Some(name) = entry.name()
    {
      files.insert(PathBuf::from(format!("{root}{name}")));
    }
    TreeWalkResult::Ok
  })?;
  Ok(files)
}

/// Make the working tree exactly match the checkpoint: restore file contents and
/// delete files that did not exist at checkpoint time (ignored files are left alone).
/// HEAD, the current branch, and the index are untouched.
pub fn restore_checkpoint(repo_root: &Path, ref_name: &str) -> Result<()> {
  let repo = Repository::open(repo_root).with_context(|| format!("open repo {repo_root:?}"))?;
  let target_files = checkpoint_tree_files(&repo, ref_name)?;

  // Delete files present now but absent from the checkpoint (tracked or untracked;
  // ignored files never appear in this listing).
  let current_files = list_repo_worktree_files(repo_root)?;
  for file in current_files {
    if !target_files.contains(&file) {
      let absolute = repo_root.join(&file);
      std::fs::remove_file(&absolute).with_context(|| format!("remove {absolute:?}"))?;
      // Clean up directories left empty by the removal.
      let mut parent = absolute.parent().map(Path::to_path_buf);
      while let Some(dir) = parent {
        if dir == repo_root || std::fs::remove_dir(&dir).is_err() {
          break;
        }
        parent = dir.parent().map(Path::to_path_buf);
      }
    }
  }

  if !target_files.is_empty() {
    // --worktree only: the user's index stays as-is.
    run_git(
      repo_root,
      &["restore", "--source", ref_name, "--worktree", "--", ":/"],
      &[],
    )?;
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use git2::Signature;
  use std::time::{SystemTime, UNIX_EPOCH};

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

  fn commit_file(repo_root: &Path, rel_path: &Path, contents: &str, message: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    if let Some(parent) = rel_path.parent() {
      std::fs::create_dir_all(repo_root.join(parent)).expect("create parent dirs");
    }
    std::fs::write(repo_root.join(rel_path), contents).expect("write file");
    let mut index = repo.index().expect("open index");
    index.add_path(rel_path).expect("stage");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("find tree");
    let signature = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    let parents: Vec<_> = parent.iter().collect();
    repo
      .commit(Some("HEAD"), &signature, &signature, message, &tree, &parents)
      .expect("commit");
  }

  fn read(repo_root: &Path, rel: &str) -> String {
    std::fs::read_to_string(repo_root.join(rel)).expect("read file")
  }

  #[test]
  fn checkpoint_captures_and_restores_tracked_and_untracked_changes() {
    let repo = TempRepo::init("checkpoint-roundtrip");
    commit_file(&repo.path, Path::new("src/main.rs"), "v1\n", "initial");

    std::fs::write(repo.path.join("src/main.rs"), "v2\n").expect("modify tracked");
    std::fs::write(repo.path.join("untracked.txt"), "new\n").expect("write untracked");

    let checkpoint = create_checkpoint(&repo.path, "session-a").expect("create checkpoint");

    // Diverge: change tracked, delete untracked, add another file.
    std::fs::write(repo.path.join("src/main.rs"), "v3\n").expect("modify again");
    std::fs::remove_file(repo.path.join("untracked.txt")).expect("delete untracked");
    std::fs::write(repo.path.join("added-later.txt"), "later\n").expect("add later file");

    restore_checkpoint(&repo.path, &checkpoint.ref_name).expect("restore");

    assert_eq!(read(&repo.path, "src/main.rs"), "v2\n");
    assert_eq!(read(&repo.path, "untracked.txt"), "new\n");
    assert!(!repo.path.join("added-later.txt").exists());
  }

  #[test]
  fn checkpoint_does_not_touch_head_branch_or_index() {
    let repo = TempRepo::init("checkpoint-head");
    commit_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let repo_handle = Repository::open(&repo.path).expect("open");
    let head_before = repo_handle.head().expect("head").target();

    std::fs::write(repo.path.join("a.txt"), "dirty\n").expect("dirty");
    let checkpoint = create_checkpoint(&repo.path, "session-b").expect("create");
    restore_checkpoint(&repo.path, &checkpoint.ref_name).expect("restore");

    let head_after = repo_handle.head().expect("head").target();
    assert_eq!(head_before, head_after);
    // The user's index must not contain the checkpoint's staged-everything state.
    let statuses = crate::status::list_repo_status(&repo.path).expect("status");
    assert!(
      statuses
        .iter()
        .all(|entry| matches!(entry.stage, crate::status::RepoStage::Unstaged))
    );
  }

  #[test]
  fn restore_deletes_files_created_after_checkpoint_in_subdirectories() {
    let repo = TempRepo::init("checkpoint-clean-dirs");
    commit_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let checkpoint = create_checkpoint(&repo.path, "session-c").expect("create");

    std::fs::create_dir_all(repo.path.join("newdir/nested")).expect("mkdirs");
    std::fs::write(repo.path.join("newdir/nested/file.txt"), "x\n").expect("write");

    restore_checkpoint(&repo.path, &checkpoint.ref_name).expect("restore");

    assert!(!repo.path.join("newdir").exists());
    assert_eq!(read(&repo.path, "a.txt"), "v1\n");
  }

  #[test]
  fn checkpoints_list_sorted_and_prune_keeps_latest() {
    let repo = TempRepo::init("checkpoint-prune");
    commit_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let first = create_checkpoint(&repo.path, "session-d").expect("first");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = create_checkpoint(&repo.path, "session-d").expect("second");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let third = create_checkpoint(&repo.path, "session-d").expect("third");

    let listed = list_checkpoints(&repo.path, "session-d").expect("list");
    assert_eq!(
      listed.iter().map(|c| c.ref_name.clone()).collect::<Vec<_>>(),
      vec![
        first.ref_name.clone(),
        second.ref_name.clone(),
        third.ref_name.clone()
      ]
    );

    prune_checkpoints(&repo.path, "session-d", 1).expect("prune");
    let remaining = list_checkpoints(&repo.path, "session-d").expect("list after prune");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].ref_name, third.ref_name);

    delete_session_checkpoints(&repo.path, "session-d").expect("delete all");
    assert!(
      list_checkpoints(&repo.path, "session-d")
        .expect("list after delete")
        .is_empty()
    );
  }

  #[test]
  fn checkpoint_works_in_repo_without_initial_commit() {
    let repo = TempRepo::init("checkpoint-no-head");
    std::fs::write(repo.path.join("draft.txt"), "wip\n").expect("write untracked");

    let checkpoint = create_checkpoint(&repo.path, "session-e").expect("create without HEAD");

    std::fs::write(repo.path.join("draft.txt"), "changed\n").expect("modify");
    restore_checkpoint(&repo.path, &checkpoint.ref_name).expect("restore");

    assert_eq!(read(&repo.path, "draft.txt"), "wip\n");
  }

  #[test]
  fn restore_leaves_ignored_files_alone() {
    let repo = TempRepo::init("checkpoint-ignored");
    commit_file(&repo.path, Path::new(".gitignore"), "target/\n", "ignore");

    let checkpoint = create_checkpoint(&repo.path, "session-f").expect("create");

    std::fs::create_dir_all(repo.path.join("target")).expect("mkdir");
    std::fs::write(repo.path.join("target/artifact.bin"), "build\n").expect("write ignored");

    restore_checkpoint(&repo.path, &checkpoint.ref_name).expect("restore");

    // Created after the checkpoint but ignored: must survive the restore.
    assert_eq!(read(&repo.path, "target/artifact.bin"), "build\n");
  }

  #[test]
  fn checkpoints_are_isolated_per_session() {
    let repo = TempRepo::init("checkpoint-sessions");
    commit_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    create_checkpoint(&repo.path, "session-x").expect("x");
    create_checkpoint(&repo.path, "session-y").expect("y");

    assert_eq!(list_checkpoints(&repo.path, "session-x").expect("x").len(), 1);
    assert_eq!(list_checkpoints(&repo.path, "session-y").expect("y").len(), 1);
    delete_session_checkpoints(&repo.path, "session-x").expect("delete x");
    assert_eq!(list_checkpoints(&repo.path, "session-y").expect("y").len(), 1);
  }
}
