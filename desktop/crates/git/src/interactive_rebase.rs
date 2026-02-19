use std::{
  collections::{HashMap, HashSet},
  fs,
  path::{Path, PathBuf},
  process::Command,
  time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use git2::{Oid, Repository, Sort};

use crate::{BranchKind, BranchRef};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InteractiveRebaseTarget {
  Branch(BranchRef),
  HeadCount(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractiveRebaseAction {
  Pick,
  Squash,
  Fixup,
  Drop,
}

impl InteractiveRebaseAction {
  fn todo_keyword(self) -> &'static str {
    match self {
      InteractiveRebaseAction::Pick => "pick",
      InteractiveRebaseAction::Squash => "squash",
      InteractiveRebaseAction::Fixup => "fixup",
      InteractiveRebaseAction::Drop => "drop",
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractiveRebaseCommit {
  pub oid: String,
  pub short_oid: String,
  pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractiveRebaseTodoEntry {
  pub oid: String,
  pub action: InteractiveRebaseAction,
}

pub fn list_interactive_rebase_commits(
  repo_root: &Path,
  target: &InteractiveRebaseTarget,
) -> Result<Vec<InteractiveRebaseCommit>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let base = resolve_target_base_oid(&repo, target)?;
  collect_interactive_rebase_commits(&repo, base)
}

pub fn start_interactive_rebase(
  repo_root: &Path,
  target: &InteractiveRebaseTarget,
  todo_entries: &[InteractiveRebaseTodoEntry],
) -> Result<()> {
  let commits = list_interactive_rebase_commits(repo_root, target)?;
  if commits.is_empty() {
    bail!("no commits to rebase");
  }
  validate_interactive_rebase_todo(&commits, todo_entries)?;

  let summaries_by_oid = commits
    .iter()
    .map(|commit| (commit.oid.clone(), commit.summary.clone()))
    .collect::<HashMap<_, _>>();
  let todo_contents = build_todo_contents(todo_entries, &summaries_by_oid)?;
  let todo_file = write_temp_file("interactive-rebase-todo", ".txt", &todo_contents)?;
  let editor_script = write_temp_file(
    "interactive-rebase-editor",
    script_suffix(),
    &sequence_editor_script(&todo_file.path),
  )?;
  make_script_executable_if_supported(&editor_script.path)?;

  let mut command = Command::new("git");
  command
    .current_dir(repo_root)
    .arg("rebase")
    .arg("-i")
    .arg(target_command_arg(target))
    .env("GIT_SEQUENCE_EDITOR", &editor_script.path)
    .env("GIT_EDITOR", ":")
    .env("GIT_AUTHOR_NAME", "Reviu")
    .env("GIT_AUTHOR_EMAIL", "reviu@contact")
    .env("GIT_COMMITTER_NAME", "Reviu")
    .env("GIT_COMMITTER_EMAIL", "reviu@contact");

  let output = command.output().context("run interactive rebase")?;
  if output.status.success() {
    return Ok(());
  }

  let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
  let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
  let details = [stderr, stdout]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
  if details.is_empty() {
    bail!("interactive rebase failed");
  }

  bail!("interactive rebase failed: {details}")
}

fn resolve_target_base_oid(repo: &Repository, target: &InteractiveRebaseTarget) -> Result<Oid> {
  match target {
    InteractiveRebaseTarget::Branch(branch) => resolve_branch_oid(repo, branch),
    InteractiveRebaseTarget::HeadCount(count) => resolve_head_count_base_oid(repo, *count),
  }
}

fn resolve_branch_oid(repo: &Repository, branch: &BranchRef) -> Result<Oid> {
  let refname = match branch.kind {
    BranchKind::Local => format!("refs/heads/{}", branch.name),
    BranchKind::Remote => format!("refs/remotes/{}", branch.name),
  };
  repo
    .refname_to_id(&refname)
    .with_context(|| format!("resolve branch {:?}", branch.name))
}

fn resolve_head_count_base_oid(repo: &Repository, count: usize) -> Result<Oid> {
  if count < 2 {
    bail!("interactive rebase HEAD~n requires n >= 2");
  }

  let mut commit = repo
    .head()
    .and_then(|head| head.peel_to_commit())
    .context("read HEAD commit")?;
  for _ in 0..count {
    commit = commit
      .parent(0)
      .with_context(|| format!("HEAD~{count} is outside of commit history"))?;
  }
  Ok(commit.id())
}

fn collect_interactive_rebase_commits(
  repo: &Repository,
  base: Oid,
) -> Result<Vec<InteractiveRebaseCommit>> {
  let head_oid = repo
    .head()
    .and_then(|head| head.peel_to_commit())
    .context("read HEAD commit")?
    .id();

  let mut walk = repo.revwalk().context("open revwalk")?;
  walk
    .set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)
    .context("configure revwalk sort")?;
  walk.push(head_oid).context("push HEAD onto revwalk")?;
  walk
    .hide(base)
    .with_context(|| format!("hide base commit {base} from revwalk"))?;

  let mut commits = Vec::new();
  for oid in walk {
    let oid = oid.context("read revwalk entry")?;
    let commit = repo.find_commit(oid).context("read commit")?;
    if commit.parent_count() > 1 {
      bail!("interactive rebase does not support merge commits in this range");
    }
    let summary = commit
      .summary()
      .or_else(|| commit.message())
      .map(str::trim)
      .filter(|value| !value.is_empty())
      .unwrap_or("No commit message")
      .replace(['\n', '\r'], "");
    let oid_text = oid.to_string();
    commits.push(InteractiveRebaseCommit {
      short_oid: oid_text.chars().take(7).collect(),
      oid: oid_text,
      summary,
    });
  }

  Ok(commits)
}

fn validate_interactive_rebase_todo(
  commits: &[InteractiveRebaseCommit],
  todo_entries: &[InteractiveRebaseTodoEntry],
) -> Result<()> {
  if commits.len() != todo_entries.len() {
    bail!("interactive rebase todo does not match selected commits");
  }

  let expected = commits
    .iter()
    .map(|commit| commit.oid.as_str())
    .collect::<HashSet<_>>();
  let mut seen = HashSet::new();
  for entry in todo_entries {
    if !expected.contains(entry.oid.as_str()) {
      bail!("interactive rebase todo contains unknown commit");
    }
    if !seen.insert(entry.oid.as_str()) {
      bail!("interactive rebase todo contains duplicate commits");
    }
  }

  let Some(first_kept) = todo_entries
    .iter()
    .find(|entry| entry.action != InteractiveRebaseAction::Drop)
  else {
    bail!("interactive rebase todo cannot drop all commits");
  };
  if first_kept.action != InteractiveRebaseAction::Pick {
    bail!("the first non-dropped commit must use pick");
  }

  let mut has_previous_kept = false;
  for entry in todo_entries {
    match entry.action {
      InteractiveRebaseAction::Pick => {
        has_previous_kept = true;
      }
      InteractiveRebaseAction::Drop => {}
      InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup => {
        if !has_previous_kept {
          bail!("squash/fixup require a previous picked commit");
        }
        has_previous_kept = true;
      }
    }
  }

  Ok(())
}

fn build_todo_contents(
  entries: &[InteractiveRebaseTodoEntry],
  summaries_by_oid: &HashMap<String, String>,
) -> Result<String> {
  let mut lines = Vec::with_capacity(entries.len());
  for entry in entries {
    let Some(summary) = summaries_by_oid.get(&entry.oid) else {
      bail!("interactive rebase todo contains unknown commit summary");
    };
    lines.push(format!(
      "{} {} {}",
      entry.action.todo_keyword(),
      entry.oid,
      summary.replace(['\n', '\r'], "")
    ));
  }
  let mut contents = lines.join("\n");
  contents.push('\n');
  Ok(contents)
}

fn target_command_arg(target: &InteractiveRebaseTarget) -> String {
  match target {
    InteractiveRebaseTarget::Branch(branch) => branch.name.clone(),
    InteractiveRebaseTarget::HeadCount(count) => format!("HEAD~{count}"),
  }
}

fn script_suffix() -> &'static str {
  if cfg!(windows) { ".cmd" } else { ".sh" }
}

fn sequence_editor_script(todo_path: &Path) -> String {
  let todo = todo_path.to_string_lossy();
  if cfg!(windows) {
    format!("@echo off\r\ntype \"{todo}\" > %1\r\n")
  } else {
    format!(
      "#!/bin/sh\ncat {} > \"$1\"\n",
      shell_single_quote(todo.as_ref())
    )
  }
}

fn shell_single_quote(value: &str) -> String {
  format!("'{}'", value.replace('\'', "'\"'\"'"))
}

struct TempPath {
  path: PathBuf,
}

impl Drop for TempPath {
  fn drop(&mut self) {
    let _ = fs::remove_file(&self.path);
  }
}

fn write_temp_file(prefix: &str, suffix: &str, contents: &str) -> Result<TempPath> {
  let mut path = std::env::temp_dir();
  let nanos = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .context("system clock before unix epoch")?
    .as_nanos();
  path.push(format!(
    "reviu-{prefix}-{}-{nanos}{suffix}",
    std::process::id()
  ));
  fs::write(&path, contents).with_context(|| format!("write temporary file {:?}", path))?;
  Ok(TempPath { path })
}

#[cfg(unix)]
fn make_script_executable_if_supported(path: &Path) -> Result<()> {
  use std::os::unix::fs::PermissionsExt;

  let mut permissions = fs::metadata(path)
    .with_context(|| format!("read metadata for {:?}", path))?
    .permissions();
  permissions.set_mode(0o755);
  fs::set_permissions(path, permissions)
    .with_context(|| format!("set executable mode for {:?}", path))
}

#[cfg(not(unix))]
fn make_script_executable_if_supported(_path: &Path) -> Result<()> {
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use git2::{Repository, Signature, build::CheckoutBuilder};

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

  fn commit_text_file(repo_root: &Path, rel_path: &Path, contents: &str, message: &str) -> Oid {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::write(repo_root.join(rel_path), contents).expect("write worktree file");

    let mut index = repo.index().expect("open index");
    index.add_path(rel_path).expect("stage file");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("find tree");
    let signature = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());

    match parent {
      Some(parent) => repo
        .commit(
          Some("HEAD"),
          &signature,
          &signature,
          message,
          &tree,
          &[&parent],
        )
        .expect("commit with parent"),
      None => repo
        .commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
        .expect("initial commit"),
    }
  }

  fn switch_to_branch(repo_root: &Path, branch_name: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    let refname = format!("refs/heads/{branch_name}");
    repo.set_head(&refname).expect("set branch head");
    let mut checkout = CheckoutBuilder::new();
    checkout.force();
    repo
      .checkout_head(Some(&mut checkout))
      .expect("checkout branch");
  }

  fn create_branch_at_head(repo_root: &Path, name: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    let head = repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read HEAD commit");
    repo.branch(name, &head, false).expect("create branch");
  }

  fn current_branch_name(repo_root: &Path) -> String {
    Repository::open(repo_root)
      .expect("open repo")
      .head()
      .ok()
      .and_then(|head| head.shorthand().map(ToString::to_string))
      .unwrap_or_else(|| "HEAD".to_string())
  }

  fn head_messages(repo_root: &Path, limit: usize) -> Vec<String> {
    let repo = Repository::open(repo_root).expect("open repo");
    let head = repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    let mut walk = repo.revwalk().expect("open revwalk");
    walk.push(head.id()).expect("push head");
    walk
      .set_sorting(Sort::TOPOLOGICAL)
      .expect("set revwalk sorting");
    walk
      .take(limit)
      .map(|oid| {
        let oid = oid.expect("read oid");
        repo
          .find_commit(oid)
          .expect("read commit")
          .summary()
          .unwrap_or_default()
          .to_string()
      })
      .collect()
  }

  fn commit_count(repo_root: &Path) -> usize {
    let repo = Repository::open(repo_root).expect("open repo");
    let head = repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    let mut walk = repo.revwalk().expect("open revwalk");
    walk.push(head.id()).expect("push head");
    walk.count()
  }

  #[test]
  fn list_interactive_rebase_commits_head_count_requires_two_or_more() {
    let repo = TempRepo::init("interactive-rebase-head-count-min");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let error = list_interactive_rebase_commits(&repo.path, &InteractiveRebaseTarget::HeadCount(1))
      .expect_err("head count 1 should fail");
    assert!(
      error.to_string().contains("n >= 2"),
      "unexpected error: {error}"
    );
  }

  #[test]
  fn list_interactive_rebase_commits_for_head_count_returns_oldest_to_newest() {
    let repo = TempRepo::init("interactive-rebase-head-count-order");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let _ = commit_text_file(&repo.path, Path::new("a.txt"), "a\n", "commit a");
    let _ = commit_text_file(&repo.path, Path::new("b.txt"), "b\n", "commit b");
    let _ = commit_text_file(&repo.path, Path::new("c.txt"), "c\n", "commit c");

    let commits =
      list_interactive_rebase_commits(&repo.path, &InteractiveRebaseTarget::HeadCount(3))
        .expect("list commits for head count");
    let summaries = commits
      .iter()
      .map(|commit| commit.summary.as_str())
      .collect::<Vec<_>>();
    assert_eq!(summaries, vec!["commit a", "commit b", "commit c"]);
  }

  #[test]
  fn list_interactive_rebase_commits_for_branch_returns_only_branch_specific_commits() {
    let repo = TempRepo::init("interactive-rebase-branch-range");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_name(&repo.path);
    create_branch_at_head(&repo.path, "feature");

    let _ = commit_text_file(
      &repo.path,
      Path::new("main.txt"),
      "main change\n",
      "main change",
    );
    switch_to_branch(&repo.path, "feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("feature.txt"),
      "feature 1\n",
      "feature 1",
    );
    let _ = commit_text_file(
      &repo.path,
      Path::new("feature-2.txt"),
      "feature 2\n",
      "feature 2",
    );

    let commits = list_interactive_rebase_commits(
      &repo.path,
      &InteractiveRebaseTarget::Branch(BranchRef {
        name: base_branch,
        kind: BranchKind::Local,
      }),
    )
    .expect("list commits for branch target");
    let summaries = commits
      .iter()
      .map(|commit| commit.summary.as_str())
      .collect::<Vec<_>>();
    assert_eq!(summaries, vec!["feature 1", "feature 2"]);
  }

  #[test]
  fn list_interactive_rebase_commits_rejects_merge_commits_in_selected_range() {
    let repo = TempRepo::init("interactive-rebase-merge-reject");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_name(&repo.path);
    create_branch_at_head(&repo.path, "feature");

    let _ = commit_text_file(&repo.path, Path::new("main.txt"), "main\n", "main change");
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let main_commit_oid = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read main commit");
    let main_commit_oid = main_commit_oid.id();

    switch_to_branch(&repo.path, "feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("feature.txt"),
      "feature\n",
      "feature change",
    );
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let feature_commit_oid = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read feature commit");
    let feature_commit_oid = feature_commit_oid.id();

    switch_to_branch(&repo.path, &base_branch);
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let main_commit = repo_handle
      .find_commit(main_commit_oid)
      .expect("find main commit");
    let feature_commit = repo_handle
      .find_commit(feature_commit_oid)
      .expect("find feature commit");
    let tree = main_commit.tree().expect("read tree");
    let signature = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let _ = repo_handle
      .commit(
        Some("HEAD"),
        &signature,
        &signature,
        "merge commit",
        &tree,
        &[&main_commit, &feature_commit],
      )
      .expect("create merge commit");

    let error = list_interactive_rebase_commits(&repo.path, &InteractiveRebaseTarget::HeadCount(2))
      .expect_err("merge commit should be rejected");
    assert!(
      error.to_string().contains("does not support merge commits"),
      "unexpected error: {error}"
    );
  }

  #[test]
  fn start_interactive_rebase_reorders_and_drops_commits() {
    let repo = TempRepo::init("interactive-rebase-start-reorder-drop");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let _ = commit_text_file(&repo.path, Path::new("a.txt"), "a\n", "commit a");
    let _ = commit_text_file(&repo.path, Path::new("b.txt"), "b\n", "commit b");
    let _ = commit_text_file(&repo.path, Path::new("c.txt"), "c\n", "commit c");

    let commits =
      list_interactive_rebase_commits(&repo.path, &InteractiveRebaseTarget::HeadCount(3))
        .expect("list commits");
    assert_eq!(commits.len(), 3);

    let todo = vec![
      InteractiveRebaseTodoEntry {
        oid: commits[2].oid.clone(),
        action: InteractiveRebaseAction::Pick,
      },
      InteractiveRebaseTodoEntry {
        oid: commits[0].oid.clone(),
        action: InteractiveRebaseAction::Pick,
      },
      InteractiveRebaseTodoEntry {
        oid: commits[1].oid.clone(),
        action: InteractiveRebaseAction::Drop,
      },
    ];

    start_interactive_rebase(&repo.path, &InteractiveRebaseTarget::HeadCount(3), &todo)
      .expect("run interactive rebase");

    let messages = head_messages(&repo.path, 3);
    assert_eq!(messages[0], "commit a");
    assert_eq!(messages[1], "commit c");
    assert!(!repo.path.join("b.txt").exists());
  }

  #[test]
  fn start_interactive_rebase_supports_squash_and_fixup_actions() {
    let repo = TempRepo::init("interactive-rebase-start-squash-fixup");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let _ = commit_text_file(&repo.path, Path::new("main.txt"), "a\n", "commit a");
    let _ = commit_text_file(&repo.path, Path::new("main.txt"), "a\nb\n", "commit b");
    let _ = commit_text_file(&repo.path, Path::new("main.txt"), "a\nb\nc\n", "commit c");

    let commits =
      list_interactive_rebase_commits(&repo.path, &InteractiveRebaseTarget::HeadCount(3))
        .expect("list commits");

    let todo = vec![
      InteractiveRebaseTodoEntry {
        oid: commits[0].oid.clone(),
        action: InteractiveRebaseAction::Pick,
      },
      InteractiveRebaseTodoEntry {
        oid: commits[1].oid.clone(),
        action: InteractiveRebaseAction::Squash,
      },
      InteractiveRebaseTodoEntry {
        oid: commits[2].oid.clone(),
        action: InteractiveRebaseAction::Fixup,
      },
    ];

    start_interactive_rebase(&repo.path, &InteractiveRebaseTarget::HeadCount(3), &todo)
      .expect("run interactive rebase");

    assert_eq!(
      std::fs::read_to_string(repo.path.join("main.txt")).expect("read squashed file"),
      "a\nb\nc\n"
    );
    assert_eq!(commit_count(&repo.path), 2);
  }

  #[test]
  fn start_interactive_rebase_rejects_todo_when_first_non_drop_is_not_pick() {
    let repo = TempRepo::init("interactive-rebase-start-invalid-first-action");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let _ = commit_text_file(&repo.path, Path::new("a.txt"), "a\n", "commit a");
    let _ = commit_text_file(&repo.path, Path::new("b.txt"), "b\n", "commit b");

    let commits =
      list_interactive_rebase_commits(&repo.path, &InteractiveRebaseTarget::HeadCount(2))
        .expect("list commits");
    let todo = vec![
      InteractiveRebaseTodoEntry {
        oid: commits[0].oid.clone(),
        action: InteractiveRebaseAction::Fixup,
      },
      InteractiveRebaseTodoEntry {
        oid: commits[1].oid.clone(),
        action: InteractiveRebaseAction::Pick,
      },
    ];

    let error = start_interactive_rebase(&repo.path, &InteractiveRebaseTarget::HeadCount(2), &todo)
      .expect_err("todo should be rejected");
    assert!(
      error
        .to_string()
        .contains("first non-dropped commit must use pick"),
      "unexpected error: {error}"
    );
  }
}
