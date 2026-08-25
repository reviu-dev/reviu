//! Linked git worktrees: one isolated checkout per agent session, created next
//! to the repository, on a named branch so the work can be reviewed and merged.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use git2::{BranchType, Repository};

use crate::checkpoint::run_git;

/// No slash on purpose: codex ≤0.144.x derives a broken sandbox mount from a
/// worktree whose branch name contains one.
pub const WORKTREE_BRANCH_PREFIX: &str = "reviu-";

const NAME_ATTEMPTS: u64 = 50;

const ADJECTIVES: [&str; 24] = [
  "amber", "bold", "brisk", "calm", "clever", "crisp", "eager", "fable", "gentle", "keen",
  "lively", "lucid", "mellow", "nimble", "plain", "quiet", "rapid", "solid", "spry", "steady",
  "sunny", "swift", "tidy", "vivid",
];

const NOUNS: [&str; 24] = [
  "aspen", "badger", "brook", "cedar", "comet", "coral", "crane", "dune", "ember", "falcon",
  "fjord", "glade", "harbor", "heron", "lagoon", "linden", "maple", "meadow", "otter", "pebble",
  "reef", "ridge", "river", "willow",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedWorktree {
  pub path: PathBuf,
  pub branch: String,
  pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedWorktree {
  pub path: PathBuf,
  /// None while the worktree sits on a detached HEAD.
  pub branch: Option<String>,
  pub head: Option<String>,
}

/// Worktrees live next to the repository, user-visible: they are working
/// checkouts, not app data.
pub fn worktrees_root_for(repo_root: &Path) -> Result<PathBuf> {
  let name = repo_root
    .file_name()
    .with_context(|| format!("repository path {repo_root:?} has no directory name"))?
    .to_string_lossy()
    .into_owned();
  let parent = repo_root
    .parent()
    .with_context(|| format!("repository path {repo_root:?} has no parent directory"))?;
  Ok(parent.join(format!("{name}-worktrees")))
}

/// The commit a fresh worktree starts from, without ever fetching: the local
/// branch matching origin's default when it exists, then the remote-tracking
/// ref itself, then the current branch, then HEAD.
pub fn default_worktree_base(repo_root: &Path) -> Result<String> {
  let repo = Repository::open(repo_root).with_context(|| format!("open repo at {repo_root:?}"))?;
  if let Ok(Some(remote_default)) = crate::branch::default_remote_branch(repo_root) {
    if let Some((_, short_name)) = remote_default.name.split_once('/')
      && repo.find_branch(short_name, BranchType::Local).is_ok()
    {
      return Ok(short_name.to_string());
    }
    return Ok(remote_default.name);
  }
  if let Ok(head) = repo.head()
    && head.is_branch()
    && let Ok(name) = head.shorthand()
  {
    return Ok(name.to_string());
  }
  Ok("HEAD".to_string())
}

/// Creates a linked worktree on a fresh `reviu-<name>` branch. `base` names
/// the starting commit; `None` picks [`default_worktree_base`].
pub fn create_worktree(repo_root: &Path, base: Option<&str>) -> Result<CreatedWorktree> {
  let base = match base {
    Some(base) => base.to_string(),
    None => default_worktree_base(repo_root)?,
  };
  run_git(
    repo_root,
    &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
    &[],
  )
  .with_context(|| format!("the base `{base}` does not name a commit"))?;

  let root = worktrees_root_for(repo_root)?;
  std::fs::create_dir_all(&root).with_context(|| format!("create {root:?}"))?;
  let taken_branches = local_branch_names(repo_root)?;
  let name = pick_name(&root, &taken_branches)?;
  let branch = format!("{WORKTREE_BRANCH_PREFIX}{name}");
  let path = root.join(&name);
  let path_arg = path.to_string_lossy().into_owned();

  run_git(
    repo_root,
    &["worktree", "add", "-b", &branch, &path_arg, &base],
    &[],
  )
  .with_context(|| format!("create the worktree at {path:?}"))?;

  Ok(CreatedWorktree { path, branch, name })
}

/// The repository's linked worktrees; the main checkout is not one of them.
pub fn list_worktrees(repo_root: &Path) -> Result<Vec<LinkedWorktree>> {
  let output = run_git(repo_root, &["worktree", "list", "--porcelain"], &[])?;
  let mut worktrees = Vec::new();
  for (index, stanza) in output.split("\n\n").enumerate() {
    // The first stanza is the main checkout.
    if index == 0 {
      continue;
    }
    let mut path = None;
    let mut head = None;
    let mut branch = None;
    for line in stanza.lines() {
      if let Some(value) = line.strip_prefix("worktree ") {
        path = Some(PathBuf::from(value));
      } else if let Some(value) = line.strip_prefix("HEAD ") {
        head = Some(value.to_string());
      } else if let Some(value) = line.strip_prefix("branch ") {
        branch = Some(
          value
            .strip_prefix("refs/heads/")
            .unwrap_or(value)
            .to_string(),
        );
      }
    }
    if let Some(path) = path {
      worktrees.push(LinkedWorktree { path, branch, head });
    }
  }
  Ok(worktrees)
}

/// Removes a linked worktree, dirty or not, and the `reviu-` branch it sits
/// on. A branch the user checked out or renamed themselves is left alone.
pub fn remove_worktree(repo_root: &Path, worktree_path: &Path) -> Result<()> {
  let branch = worktree_current_branch(worktree_path);
  let path_arg = worktree_path.to_string_lossy().into_owned();
  if run_git(
    repo_root,
    &["worktree", "remove", "--force", &path_arg],
    &[],
  )
  .is_err()
  {
    // Git refused (already half-gone, locked): take the directory out by hand,
    // prune below drops the stale metadata.
    if worktree_path.exists() {
      std::fs::remove_dir_all(worktree_path)
        .with_context(|| format!("remove the worktree directory {worktree_path:?}"))?;
    }
  }
  prune_worktrees(repo_root)?;
  if let Some(branch) = branch
    && branch.starts_with(WORKTREE_BRANCH_PREFIX)
  {
    // Best-effort: the branch may be checked out in another worktree.
    let _ = run_git(repo_root, &["branch", "-D", &branch], &[]);
  }
  Ok(())
}

/// Drops the metadata of worktrees whose directories are gone.
pub fn prune_worktrees(repo_root: &Path) -> Result<()> {
  run_git(repo_root, &["worktree", "prune"], &[])?;
  Ok(())
}

/// The main checkout root when `path` is a linked worktree, `None` otherwise.
/// A linked worktree carries a `.git` FILE pointing at
/// `<root>/.git/worktrees/<name>`; anything else is not one.
pub fn linked_worktree_root(path: &Path) -> Option<PathBuf> {
  let git_file = path.join(".git");
  if !std::fs::metadata(&git_file).ok()?.is_file() {
    return None;
  }
  let contents = std::fs::read_to_string(&git_file).ok()?;
  let target = contents.lines().next()?.strip_prefix("gitdir:")?.trim();
  let target = if Path::new(target).is_absolute() {
    PathBuf::from(target)
  } else {
    // `worktree.useRelativePaths` writes `../..` hops; resolve them.
    std::fs::canonicalize(path.join(target)).ok()?
  };
  let worktrees_dir = target.parent()?;
  if worktrees_dir.file_name()? != "worktrees" {
    return None;
  }
  let git_dir = worktrees_dir.parent()?;
  if git_dir.file_name()? != ".git" {
    return None;
  }
  git_dir.parent().map(Path::to_path_buf)
}

fn local_branch_names(repo_root: &Path) -> Result<Vec<String>> {
  let repo = Repository::open(repo_root).with_context(|| format!("open repo at {repo_root:?}"))?;
  let mut names = Vec::new();
  for branch in repo.branches(Some(BranchType::Local))? {
    let (branch, _) = branch?;
    if let Some(name) = branch.name()? {
      names.push(name.to_string());
    }
  }
  Ok(names)
}

fn worktree_current_branch(worktree_path: &Path) -> Option<String> {
  let repo = Repository::open(worktree_path).ok()?;
  let head = repo.head().ok()?;
  if !head.is_branch() {
    return None;
  }
  head.shorthand().ok().map(str::to_string)
}

fn pick_name(root: &Path, taken_branches: &[String]) -> Result<String> {
  let seed = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|duration| duration.subsec_nanos() as u64)
    .unwrap_or(0);
  for attempt in 0..NAME_ATTEMPTS {
    let mixed = seed.wrapping_add(attempt.wrapping_mul(0x9E37_79B9));
    let adjective = ADJECTIVES[(mixed % ADJECTIVES.len() as u64) as usize];
    let noun = NOUNS[((mixed / ADJECTIVES.len() as u64) % NOUNS.len() as u64) as usize];
    let name = format!("{adjective}-{noun}");
    if name_is_free(root, taken_branches, &name) {
      return Ok(name);
    }
  }
  // Every combination the walk visited is taken: a unique suffix settles it.
  let name = format!("worktree-{seed:08x}");
  if name_is_free(root, taken_branches, &name) {
    return Ok(name);
  }
  bail!("could not allocate a worktree name under {root:?}")
}

fn name_is_free(root: &Path, taken_branches: &[String], name: &str) -> bool {
  !root.join(name).exists()
    && !taken_branches
      .iter()
      .any(|branch| branch == &format!("{WORKTREE_BRANCH_PREFIX}{name}"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use std::process::Command;

  fn git(repo_root: &Path, args: &[&str]) {
    let output = Command::new("git")
      .current_dir(repo_root)
      .args(args)
      .output()
      .expect("run git");
    assert!(
      output.status.success(),
      "git {args:?} failed: {}",
      String::from_utf8_lossy(&output.stderr)
    );
  }

  fn cleanup_worktrees_root(repo_root: &Path) {
    if let Ok(root) = worktrees_root_for(repo_root) {
      let _ = std::fs::remove_dir_all(root);
    }
  }

  #[test]
  fn create_worktree_makes_an_isolated_checkout_on_a_named_branch() {
    let repo = TempRepo::init("worktree-create");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let head_before = crate::test_support::head_oid(&repo.path);

    let created = create_worktree(&repo.path, None).expect("create worktree");

    assert!(created.path.is_dir());
    assert_eq!(
      created.path.parent(),
      worktrees_root_for(&repo.path).ok().as_deref(),
      "the worktree lives next to the repository"
    );
    assert!(created.branch.starts_with(WORKTREE_BRANCH_PREFIX));
    assert!(
      !created.branch.contains('/'),
      "no slash: some agents derive a broken sandbox mount from it"
    );
    assert!(
      created.path.join(".git").is_file(),
      "a linked worktree has a .git FILE"
    );
    assert_eq!(
      std::fs::read_to_string(created.path.join("README.md")).expect("read seeded file"),
      "v1\n",
      "the worktree starts from the base commit"
    );
    // The main checkout was never touched.
    assert_eq!(crate::test_support::head_oid(&repo.path), head_before);

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn two_worktrees_never_share_a_name_a_path_or_a_branch() {
    let repo = TempRepo::init("worktree-collision");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let first = create_worktree(&repo.path, None).expect("first worktree");
    let second = create_worktree(&repo.path, None).expect("second worktree");

    assert_ne!(first.name, second.name);
    assert_ne!(first.path, second.path);
    assert_ne!(first.branch, second.branch);

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn the_base_prefers_the_local_branch_matching_origins_default() {
    let repo = TempRepo::init("worktree-base-origin");
    commit_text_file(&repo.path, Path::new("README.md"), "main\n", "on main");
    let main_oid = crate::test_support::head_oid(&repo.path).to_string();
    let main_name = Repository::open(&repo.path)
      .expect("open repo")
      .head()
      .expect("head")
      .shorthand()
      .expect("branch name")
      .to_string();

    // origin's default points at the branch we are about to leave. The remote
    // must be configured for the lookup to consider its refs.
    git(&repo.path, &["remote", "add", "origin", "."]);
    git(
      &repo.path,
      &[
        "update-ref",
        &format!("refs/remotes/origin/{main_name}"),
        &main_oid,
      ],
    );
    git(
      &repo.path,
      &[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        &format!("refs/remotes/origin/{main_name}"),
      ],
    );

    // Move to a feature branch with an extra commit: the worktree must NOT
    // start from here.
    git(&repo.path, &["switch", "-c", "feature"]);
    commit_text_file(&repo.path, Path::new("extra.txt"), "x\n", "on feature");

    assert_eq!(
      default_worktree_base(&repo.path).expect("base"),
      main_name,
      "the local default branch wins over the current one"
    );
    let created = create_worktree(&repo.path, None).expect("create worktree");
    assert_eq!(
      crate::test_support::head_oid(&created.path).to_string(),
      main_oid,
      "the worktree starts from origin's default, not the checked-out branch"
    );

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn without_a_remote_the_base_is_the_current_branch_and_a_named_base_wins() {
    let repo = TempRepo::init("worktree-base-local");
    commit_text_file(&repo.path, Path::new("README.md"), "main\n", "on main");
    let main_name = Repository::open(&repo.path)
      .expect("open repo")
      .head()
      .expect("head")
      .shorthand()
      .expect("branch name")
      .to_string();
    let main_oid = crate::test_support::head_oid(&repo.path).to_string();
    git(&repo.path, &["switch", "-c", "feature"]);
    commit_text_file(&repo.path, Path::new("extra.txt"), "x\n", "on feature");
    let feature_oid = crate::test_support::head_oid(&repo.path).to_string();

    assert_eq!(default_worktree_base(&repo.path).expect("base"), "feature");
    let from_current = create_worktree(&repo.path, None).expect("worktree from current");
    assert_eq!(
      crate::test_support::head_oid(&from_current.path).to_string(),
      feature_oid
    );

    let from_named = create_worktree(&repo.path, Some(&main_name)).expect("worktree from main");
    assert_eq!(
      crate::test_support::head_oid(&from_named.path).to_string(),
      main_oid,
      "an explicit base overrides the default"
    );

    let refused = create_worktree(&repo.path, Some("no-such-ref"));
    assert!(refused.is_err(), "an unknown base is refused up front");

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn listing_reports_linked_worktrees_and_skips_the_main_checkout() {
    let repo = TempRepo::init("worktree-list");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    assert!(
      list_worktrees(&repo.path).expect("empty list").is_empty(),
      "the main checkout is not a linked worktree"
    );

    let created = create_worktree(&repo.path, None).expect("create worktree");
    let listed = list_worktrees(&repo.path).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
      listed[0]
        .path
        .canonicalize()
        .expect("canonical listed path"),
      created.path.canonicalize().expect("canonical created path")
    );
    assert_eq!(listed[0].branch.as_deref(), Some(created.branch.as_str()));
    assert!(listed[0].head.is_some());

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn removing_a_dirty_worktree_deletes_it_and_its_branch() {
    let repo = TempRepo::init("worktree-remove");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");

    // Dirty on purpose: an agent's half-done work must not block the delete.
    std::fs::write(created.path.join("wip.txt"), "half done").expect("dirty the worktree");

    remove_worktree(&repo.path, &created.path).expect("remove worktree");

    assert!(!created.path.exists(), "the directory is gone");
    assert!(
      list_worktrees(&repo.path).expect("list").is_empty(),
      "the metadata is pruned"
    );
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    assert!(
      repo_handle
        .find_branch(&created.branch, BranchType::Local)
        .is_err(),
      "the reviu- branch went with the worktree"
    );

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn removing_a_worktree_leaves_a_branch_the_user_made_theirs() {
    let repo = TempRepo::init("worktree-remove-user-branch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");

    // The user took over: their branch is not ours to delete.
    git(&created.path, &["switch", "-c", "my-own-work"]);

    remove_worktree(&repo.path, &created.path).expect("remove worktree");

    assert!(!created.path.exists());
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    assert!(
      repo_handle
        .find_branch("my-own-work", BranchType::Local)
        .is_ok(),
      "the user's branch survives the removal"
    );

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn removing_survives_a_directory_already_gone() {
    let repo = TempRepo::init("worktree-remove-gone");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");

    std::fs::remove_dir_all(&created.path).expect("simulate an external delete");
    remove_worktree(&repo.path, &created.path).expect("remove worktree");

    assert!(
      list_worktrees(&repo.path).expect("list").is_empty(),
      "the stale metadata is pruned"
    );

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn linked_worktree_root_resolves_the_main_checkout_and_nothing_else() {
    let repo = TempRepo::init("worktree-root");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");

    assert_eq!(
      linked_worktree_root(&created.path)
        .expect("a linked worktree resolves")
        .canonicalize()
        .expect("canonical resolved root"),
      repo.path.canonicalize().expect("canonical repo root")
    );
    assert_eq!(
      linked_worktree_root(&repo.path),
      None,
      "the main checkout is not a linked worktree"
    );

    let plain = crate::test_support::TempDir::new("worktree-root-plain");
    assert_eq!(linked_worktree_root(&plain.path), None);

    // A .git file that does not point inside `<root>/.git/worktrees/`.
    let malformed = crate::test_support::TempDir::new("worktree-root-malformed");
    std::fs::write(malformed.path.join(".git"), "gitdir: /somewhere/else").expect("write");
    assert_eq!(linked_worktree_root(&malformed.path), None);

    cleanup_worktrees_root(&repo.path);
  }
}
