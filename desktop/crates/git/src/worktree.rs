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
/// Refuses a directory that is not a linked worktree of this repository:
/// the fallback deletion must never reach an arbitrary folder.
pub fn remove_worktree(repo_root: &Path, worktree_path: &Path) -> Result<()> {
  if worktree_path.exists() && !belongs_to_repository(repo_root, worktree_path) {
    bail!("{worktree_path:?} is not a linked worktree of {repo_root:?}");
  }
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

/// Whether `path` is a linked worktree whose main checkout is `repo_root`.
/// Two proofs accepted: the `.git` file points back at the repository, or the
/// repository's own worktree registry lists the path (covers a worktree whose
/// `.git` file was deleted by hand).
fn belongs_to_repository(repo_root: &Path, path: &Path) -> bool {
  let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
  if let Some(resolved_root) = linked_worktree_root(path)
    && canonical(&resolved_root) == canonical(repo_root)
  {
    return true;
  }
  let target = canonical(path);
  list_worktrees(repo_root)
    .map(|worktrees| {
      worktrees
        .iter()
        .any(|worktree| canonical(&worktree.path) == target)
    })
    .unwrap_or(false)
}

/// Drops the metadata of worktrees whose directories are gone.
pub fn prune_worktrees(repo_root: &Path) -> Result<()> {
  run_git(repo_root, &["worktree", "prune"], &[])?;
  Ok(())
}

/// A branch slug from a conversation title: ASCII lowercase, everything else
/// collapsed to `-`, truncated. No slash on purpose (same codex bug as the
/// prefix). `None` when nothing printable survives.
pub fn worktree_branch_slug(title: &str) -> Option<String> {
  const MAX_SLUG_BYTES: usize = 48;
  let mut slug = String::new();
  let mut last_dash = true;
  for character in title.chars() {
    let character = character.to_ascii_lowercase();
    if character.is_ascii_alphanumeric() {
      slug.push(character);
      last_dash = false;
    } else if !last_dash {
      slug.push('-');
      last_dash = true;
    }
    if slug.len() >= MAX_SLUG_BYTES {
      break;
    }
  }
  let slug = slug.trim_matches('-').to_string();
  if slug.is_empty() { None } else { Some(slug) }
}

/// Renames a worktree's generated branch after its conversation earned a
/// title, comet-style. Returns the branch the worktree ends on, which is the
/// old one whenever the rename must not happen:
/// - the worktree left `expected_branch` (the user checked out or renamed:
///   the branch is theirs now),
/// - `expected_branch` is not the generated `reviu-<folder>` name any more
///   (already renamed once; one title, one rename),
/// - the slug collides even with a stable suffix.
pub fn rename_worktree_branch(
  repo_root: &Path,
  worktree_path: &Path,
  expected_branch: &str,
  title: &str,
) -> Result<String> {
  let keep = || expected_branch.to_string();
  let Some(current) = worktree_current_branch(worktree_path) else {
    return Ok(keep());
  };
  if current != expected_branch {
    return Ok(keep());
  }
  let generated = worktree_path
    .file_name()
    .map(|name| format!("{WORKTREE_BRANCH_PREFIX}{}", name.to_string_lossy()));
  if generated.as_deref() != Some(expected_branch) {
    return Ok(keep());
  }
  let Some(slug) = worktree_branch_slug(title) else {
    return Ok(keep());
  };
  let mut new_branch = format!("{WORKTREE_BRANCH_PREFIX}{slug}");
  if new_branch == expected_branch {
    return Ok(keep());
  }
  if branch_exists(repo_root, &new_branch)? {
    // Stable suffix, not a counter: the same worktree always retries the
    // same name, so a crashed rename never mints a second candidate.
    let suffix = blake3::hash(worktree_path.to_string_lossy().as_bytes()).to_hex();
    new_branch = format!("{new_branch}-{}", &suffix.as_str()[..6]);
    if branch_exists(repo_root, &new_branch)? {
      return Ok(keep());
    }
  }
  run_git(
    repo_root,
    &["branch", "-m", expected_branch, &new_branch],
    &[],
  )
  .with_context(|| format!("rename {expected_branch} to {new_branch}"))?;
  // Re-read rather than trust ourselves: a concurrent external checkout in
  // the worktree wins the metadata race.
  Ok(worktree_current_branch(worktree_path).unwrap_or(new_branch))
}

fn branch_exists(repo_root: &Path, branch: &str) -> Result<bool> {
  let repo = Repository::open(repo_root).with_context(|| format!("open repo at {repo_root:?}"))?;
  Ok(repo.find_branch(branch, BranchType::Local).is_ok())
}

/// The private gitdir of a linked worktree (`<root>/.git/worktrees/<name>`),
/// read from its `.git` FILE; `None` for a main checkout or a non-repo.
fn linked_gitdir(path: &Path) -> Option<PathBuf> {
  let git_file = path.join(".git");
  if !std::fs::metadata(&git_file).ok()?.is_file() {
    return None;
  }
  let contents = std::fs::read_to_string(&git_file).ok()?;
  let target = contents.lines().next()?.strip_prefix("gitdir:")?.trim();
  if Path::new(target).is_absolute() {
    Some(PathBuf::from(target))
  } else {
    // `worktree.useRelativePaths` writes `../..` hops; resolve them.
    std::fs::canonicalize(path.join(target)).ok()
  }
}

/// Where a checkout's index file lives: `.git/index` for a main checkout,
/// inside the worktree's private gitdir for a linked one. Watching
/// `<checkout>/.git/index` silently breaks in a worktree: `.git` is a file.
pub fn index_path(checkout: &Path) -> PathBuf {
  match linked_gitdir(checkout) {
    Some(gitdir) => gitdir.join("index"),
    None => checkout.join(".git").join("index"),
  }
}

/// The main checkout root when `path` is a linked worktree, `None` otherwise.
/// A linked worktree carries a `.git` FILE pointing at
/// `<root>/.git/worktrees/<name>`; anything else is not one.
pub fn linked_worktree_root(path: &Path) -> Option<PathBuf> {
  let target = linked_gitdir(path)?;
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
  fn removing_refuses_a_directory_that_is_not_ours() {
    let repo = TempRepo::init("worktree-remove-refuse");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    // A plain folder full of user files.
    let plain = crate::test_support::TempDir::new("worktree-remove-refuse-plain");
    std::fs::write(plain.path.join("precious.txt"), "do not touch").expect("write");
    assert!(
      remove_worktree(&repo.path, &plain.path).is_err(),
      "a folder that is no worktree of ours is refused"
    );
    assert!(
      plain.path.join("precious.txt").exists(),
      "and nothing in it was deleted"
    );

    // A real worktree, but of ANOTHER repository.
    let other = TempRepo::init("worktree-remove-refuse-other");
    commit_text_file(&other.path, Path::new("README.md"), "v1\n", "initial");
    let foreign = create_worktree(&other.path, None).expect("foreign worktree");
    assert!(
      remove_worktree(&repo.path, &foreign.path).is_err(),
      "a worktree of another repository is refused"
    );
    assert!(foreign.path.exists());

    cleanup_worktrees_root(&repo.path);
    cleanup_worktrees_root(&other.path);
  }

  #[test]
  fn a_worktree_whose_git_file_was_deleted_is_still_ours_to_remove() {
    let repo = TempRepo::init("worktree-remove-corrupt");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");

    // The pointer is gone but git's registry still names the path.
    std::fs::remove_file(created.path.join(".git")).expect("corrupt the worktree");
    remove_worktree(&repo.path, &created.path).expect("remove the corrupted worktree");

    assert!(!created.path.exists());
    assert!(list_worktrees(&repo.path).expect("list").is_empty());

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn checkpoints_work_from_inside_a_worktree_and_share_the_refs() {
    let repo = TempRepo::init("worktree-checkpoints");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");

    // The agent edits in its worktree; the turn snapshots from there.
    std::fs::write(created.path.join("README.md"), "agent edit\n").expect("edit");
    let checkpoint =
      crate::create_checkpoint(&created.path, "conv-1").expect("checkpoint from the worktree");

    // Checkpoint refs live in the shared .git: the main checkout sees them.
    let from_main = crate::list_checkpoints(&repo.path, "conv-1").expect("list from main");
    assert_eq!(from_main.len(), 1);
    assert_eq!(from_main[0].ref_name, checkpoint.ref_name);

    // Restoring inside the worktree rewinds the worktree, not the main checkout.
    std::fs::write(created.path.join("README.md"), "later edit\n").expect("edit again");
    crate::restore_checkpoint(&created.path, &checkpoint.ref_name).expect("restore");
    assert_eq!(
      std::fs::read_to_string(created.path.join("README.md")).expect("read"),
      "agent edit\n"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read main"),
      "v1\n",
      "the main checkout never moved"
    );

    // Deleting the conversation's refs from the main checkout clears them.
    crate::delete_session_checkpoints(&repo.path, "conv-1").expect("delete refs");
    assert!(
      crate::list_checkpoints(&repo.path, "conv-1")
        .expect("list again")
        .is_empty()
    );

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn the_status_pipeline_reads_a_worktree_like_any_checkout() {
    let repo = TempRepo::init("worktree-status");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");

    assert_eq!(
      crate::discover_repository_root(&created.path)
        .expect("a worktree is a repository root")
        .canonicalize()
        .expect("canonical discovered root"),
      created.path.canonicalize().expect("canonical worktree")
    );

    std::fs::write(created.path.join("README.md"), "dirty\n").expect("edit");
    let entries = crate::list_repo_status(&created.path).expect("status from the worktree");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, PathBuf::from("README.md"));
    assert!(
      crate::list_repo_status(&repo.path)
        .expect("status from main")
        .is_empty(),
      "the main checkout stays clean"
    );

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn linked_worktree_root_handles_a_relative_gitdir() {
    let repo = TempRepo::init("worktree-root-relative");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");

    // What `worktree.useRelativePaths` writes: hops instead of an absolute path.
    let git_file = created.path.join(".git");
    let absolute = std::fs::read_to_string(&git_file).expect("read .git file");
    let target = absolute
      .lines()
      .next()
      .and_then(|line| line.strip_prefix("gitdir:"))
      .expect("gitdir line")
      .trim();
    let relative = pathdiff_relative(Path::new(target), &created.path);
    std::fs::write(&git_file, format!("gitdir: {}\n", relative.display())).expect("rewrite");

    assert_eq!(
      linked_worktree_root(&created.path)
        .expect("a relative gitdir still resolves")
        .canonicalize()
        .expect("canonical resolved root"),
      repo.path.canonicalize().expect("canonical repo root")
    );

    cleanup_worktrees_root(&repo.path);
  }

  /// Minimal relative-path builder for the fixture: walk up from `base`, then
  /// down into `target`.
  fn pathdiff_relative(target: &Path, base: &Path) -> PathBuf {
    let target = target.canonicalize().expect("canonical target");
    let base = base.canonicalize().expect("canonical base");
    let target_parts: Vec<_> = target.components().collect();
    let base_parts: Vec<_> = base.components().collect();
    let shared = target_parts
      .iter()
      .zip(base_parts.iter())
      .take_while(|(a, b)| a == b)
      .count();
    let mut relative = PathBuf::new();
    for _ in shared..base_parts.len() {
      relative.push("..");
    }
    for part in &target_parts[shared..] {
      relative.push(part);
    }
    relative
  }

  #[test]
  fn a_detached_head_still_gives_a_working_base() {
    let repo = TempRepo::init("worktree-base-detached");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let head = crate::test_support::head_oid(&repo.path).to_string();
    git(&repo.path, &["checkout", "--detach", &head]);

    assert_eq!(default_worktree_base(&repo.path).expect("base"), "HEAD");
    let created = create_worktree(&repo.path, None).expect("create from detached HEAD");
    assert_eq!(
      crate::test_support::head_oid(&created.path).to_string(),
      head
    );

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn branch_slugs_are_ascii_dashed_and_bounded() {
    assert_eq!(
      worktree_branch_slug("Fix the scroll jump!").as_deref(),
      Some("fix-the-scroll-jump")
    );
    assert_eq!(
      worktree_branch_slug("café/crème & braces").as_deref(),
      Some("caf-cr-me-braces"),
      "no slash, no unicode, no punctuation"
    );
    assert_eq!(worktree_branch_slug("   ***   "), None);
    let long = worktree_branch_slug(&"word ".repeat(30)).expect("slug");
    assert!(long.len() <= 48);
    assert!(!long.ends_with('-'));
  }

  #[test]
  fn the_generated_branch_is_renamed_after_the_title_once() {
    let repo = TempRepo::init("worktree-rename");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");

    let renamed =
      rename_worktree_branch(&repo.path, &created.path, &created.branch, "Fix the scroll")
        .expect("rename");
    assert_eq!(renamed, "reviu-fix-the-scroll");
    assert_eq!(
      worktree_current_branch(&created.path).as_deref(),
      Some("reviu-fix-the-scroll"),
      "the worktree rode along with its branch"
    );

    // One title, one rename: the branch no longer matches the folder's
    // generated name, so a second title change leaves it alone.
    let again = rename_worktree_branch(&repo.path, &created.path, &renamed, "Another title")
      .expect("second rename attempt");
    assert_eq!(again, renamed);

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn a_branch_the_user_moved_is_never_renamed() {
    let repo = TempRepo::init("worktree-rename-user");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");
    git(&created.path, &["switch", "-c", "my-own-work"]);

    let kept = rename_worktree_branch(&repo.path, &created.path, &created.branch, "A title")
      .expect("rename attempt");
    assert_eq!(kept, created.branch, "the user's checkout wins");
    assert_eq!(
      worktree_current_branch(&created.path).as_deref(),
      Some("my-own-work")
    );

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn a_colliding_title_gets_a_stable_suffix() {
    let repo = TempRepo::init("worktree-rename-collision");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    git(&repo.path, &["branch", "reviu-same-title"]);
    let created = create_worktree(&repo.path, None).expect("create worktree");

    let renamed = rename_worktree_branch(&repo.path, &created.path, &created.branch, "Same title")
      .expect("rename");
    assert!(
      renamed.starts_with("reviu-same-title-") && renamed.len() == "reviu-same-title-".len() + 6,
      "a taken slug gets a short stable suffix: {renamed}"
    );

    cleanup_worktrees_root(&repo.path);
  }

  #[test]
  fn index_path_points_inside_the_worktrees_private_gitdir() {
    let repo = TempRepo::init("worktree-index-path");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let created = create_worktree(&repo.path, None).expect("create worktree");

    let main_index = index_path(&repo.path);
    assert_eq!(main_index, repo.path.join(".git").join("index"));
    assert!(main_index.exists());

    let worktree_index = index_path(&created.path);
    assert!(
      worktree_index.exists(),
      "the linked worktree has its own index"
    );
    assert_ne!(
      worktree_index, main_index,
      "watching the main index would miss the worktree's staging"
    );
    assert!(
      worktree_index
        .to_string_lossy()
        .contains(&format!(".git/worktrees/{}", created.name)),
      "the index lives in the worktree's private gitdir"
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
