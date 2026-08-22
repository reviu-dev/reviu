use std::{
  collections::BTreeMap,
  path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use git2::{
  BranchType, Delta, Diff, DiffDelta, DiffFindOptions, DiffOptions, Oid, Patch, Repository, Sort,
  Tree,
};

#[derive(Clone, Debug)]
pub struct HistoryCommitNode {
  pub oid: String,
  pub short_oid: String,
  pub summary: String,
  pub author: String,
  pub parent_oids: Vec<String>,
  pub refs: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitFileChangeKind {
  Added,
  Deleted,
  Modified,
  Renamed,
  Copied,
  Typechange,
  Conflicted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitChangedFile {
  pub path: PathBuf,
  pub old_path: Option<PathBuf>,
  pub kind: CommitFileChangeKind,
}

#[derive(Clone, Debug)]
pub struct CommitFileDiff {
  pub file: CommitChangedFile,
  pub patch: String,
  pub content: String,
  pub binary_bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRevision {
  pub head_oid: Option<String>,
  pub head_label: Option<String>,
  pub refs: Vec<String>,
}

pub fn list_commit_history(repo_root: &Path, limit: usize) -> Result<Vec<HistoryCommitNode>> {
  if limit == 0 {
    return Ok(Vec::new());
  }

  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut refs_by_oid = refs_by_oid(&repo)?;

  let mut walk = repo.revwalk()?;
  // Keep topological ordering for a stable history traversal.
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
      .ok()
      .flatten()
      .unwrap_or("No commit message")
      .replace(['\n', '\r'], "");
    let author = commit.author().name().unwrap_or("Unknown").to_string();
    let parent_oids = commit
      .parent_ids()
      .map(|parent| parent.to_string())
      .collect();

    rows.push(HistoryCommitNode {
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

pub fn current_history_revision(repo_root: &Path) -> Result<HistoryRevision> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let (head_oid, head_label) = if let Some((oid, label)) = head_target(&repo) {
    (Some(oid.to_string()), Some(label))
  } else {
    (None, None)
  };

  let mut refs = refs_by_oid(&repo)?
    .into_iter()
    .flat_map(|(oid, mut labels)| {
      labels.sort();
      labels.dedup();
      labels
        .into_iter()
        .map(move |label| format!("{label}@{oid}"))
        .collect::<Vec<_>>()
    })
    .collect::<Vec<_>>();
  refs.sort();
  refs.dedup();

  Ok(HistoryRevision {
    head_oid,
    head_label,
    refs,
  })
}

pub fn list_commit_changed_files(
  repo_root: &Path,
  commit_oid: &str,
) -> Result<Vec<CommitChangedFile>> {
  let repo = open_repo(repo_root)?;
  let commit = parse_commit(&repo, commit_oid)?;
  let head_tree = commit.tree().context("read commit tree")?;
  let base_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());
  let diff = diff_trees(&repo, base_tree.as_ref(), &head_tree)?;

  Ok(changed_files_in(&diff))
}

/// Everything a range of commits changes, `base_oid` excluded and `head_oid`
/// included. For a pull request `base_oid` is the merge base, so the range holds
/// what the branch proposes and nothing the base branch did meanwhile.
pub fn list_range_changed_files(
  repo_root: &Path,
  base_oid: &str,
  head_oid: &str,
) -> Result<Vec<CommitChangedFile>> {
  let repo = open_repo(repo_root)?;
  let base_tree = parse_commit(&repo, base_oid)?
    .tree()
    .context("read base tree")?;
  let head_tree = parse_commit(&repo, head_oid)?
    .tree()
    .context("read head tree")?;
  let diff = diff_trees(&repo, Some(&base_tree), &head_tree)?;

  Ok(changed_files_in(&diff))
}

pub fn load_commit_file_diff(
  repo_root: &Path,
  commit_oid: &str,
  rel_path: &Path,
) -> Result<CommitFileDiff> {
  let repo = open_repo(repo_root)?;
  let commit = parse_commit(&repo, commit_oid)?;
  let head_tree = commit.tree().context("read commit tree")?;
  let base_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());
  let diff = diff_trees(&repo, base_tree.as_ref(), &head_tree)?;

  match file_diff_in(&repo, &diff, &head_tree, rel_path)? {
    Some(file_diff) => Ok(file_diff),
    None => bail!("commit {commit_oid} does not change path {:?}", rel_path),
  }
}

pub fn load_range_file_diff(
  repo_root: &Path,
  base_oid: &str,
  head_oid: &str,
  rel_path: &Path,
) -> Result<CommitFileDiff> {
  let repo = open_repo(repo_root)?;
  let base_tree = parse_commit(&repo, base_oid)?
    .tree()
    .context("read base tree")?;
  let head_tree = parse_commit(&repo, head_oid)?
    .tree()
    .context("read head tree")?;
  let diff = diff_trees(&repo, Some(&base_tree), &head_tree)?;

  match file_diff_in(&repo, &diff, &head_tree, rel_path)? {
    Some(file_diff) => Ok(file_diff),
    None => bail!("{base_oid}..{head_oid} does not change path {:?}", rel_path),
  }
}

/// The commit two branches share. A pull request is measured against it, not
/// against the tip of its base branch, which may have moved since.
pub fn merge_base(repo_root: &Path, one_oid: &str, other_oid: &str) -> Result<String> {
  let repo = open_repo(repo_root)?;
  let one = parse_commit(&repo, one_oid)?.id();
  let other = parse_commit(&repo, other_oid)?.id();

  repo
    .merge_base(one, other)
    .map(|oid| oid.to_string())
    .with_context(|| format!("find merge base of {one_oid} and {other_oid}"))
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
      Ok(name) if !name.is_empty() => format!("HEAD -> {name}"),
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

fn open_repo(repo_root: &Path) -> Result<Repository> {
  Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))
}

/// One side may be absent: the first commit of a repository has no parent tree.
fn diff_trees<'repo>(
  repo: &'repo Repository,
  base_tree: Option<&Tree<'repo>>,
  head_tree: &Tree<'repo>,
) -> Result<Diff<'repo>> {
  let mut options = DiffOptions::new();
  let mut diff = repo
    .diff_tree_to_tree(base_tree, Some(head_tree), Some(&mut options))
    .context("compute diff between trees")?;
  enable_rename_detection(&mut diff);
  Ok(diff)
}

fn changed_files_in(diff: &Diff<'_>) -> Vec<CommitChangedFile> {
  diff
    .deltas()
    .filter_map(|delta| commit_changed_file_from_delta(&delta))
    .collect()
}

/// The content comes from `head_tree`: a deleted file has none, which is what
/// `load_commit_file_content` already answers.
fn file_diff_in(
  repo: &Repository,
  diff: &Diff<'_>,
  head_tree: &Tree<'_>,
  rel_path: &Path,
) -> Result<Option<CommitFileDiff>> {
  let target_path = normalize_path(rel_path);
  for (delta_ix, delta) in diff.deltas().enumerate() {
    let Some(file) = commit_changed_file_from_delta(&delta) else {
      continue;
    };
    if normalize_path(&file.path) != target_path {
      continue;
    }

    let patch = patch_for_delta(diff, delta_ix)?;
    let (content, binary_bytes) = load_commit_file_content(repo, head_tree, &file.path)?;
    return Ok(Some(CommitFileDiff {
      file,
      patch,
      content,
      binary_bytes,
    }));
  }

  Ok(None)
}

fn parse_commit<'repo>(repo: &'repo Repository, commit_oid: &str) -> Result<git2::Commit<'repo>> {
  let oid = Oid::from_str(commit_oid).with_context(|| format!("invalid oid {commit_oid}"))?;
  repo
    .find_commit(oid)
    .with_context(|| format!("find commit {commit_oid}"))
}

fn enable_rename_detection(diff: &mut Diff<'_>) {
  let mut find_options = DiffFindOptions::new();
  find_options.renames(true).copies(true);
  let _ = diff.find_similar(Some(&mut find_options));
}

fn commit_changed_file_from_delta(delta: &DiffDelta<'_>) -> Option<CommitChangedFile> {
  let kind = commit_change_kind(delta.status())?;
  let old_path = delta.old_file().path().map(Path::to_path_buf);
  let new_path = delta.new_file().path().map(Path::to_path_buf);
  let path = if kind == CommitFileChangeKind::Deleted {
    old_path.clone().or(new_path.clone())?
  } else {
    new_path.clone().or(old_path.clone())?
  };
  let old_path = old_path.filter(|old| old != &path);

  Some(CommitChangedFile {
    path,
    old_path,
    kind,
  })
}

fn commit_change_kind(status: Delta) -> Option<CommitFileChangeKind> {
  match status {
    Delta::Added => Some(CommitFileChangeKind::Added),
    Delta::Deleted => Some(CommitFileChangeKind::Deleted),
    Delta::Modified => Some(CommitFileChangeKind::Modified),
    Delta::Renamed => Some(CommitFileChangeKind::Renamed),
    Delta::Copied => Some(CommitFileChangeKind::Copied),
    Delta::Typechange => Some(CommitFileChangeKind::Typechange),
    Delta::Conflicted => Some(CommitFileChangeKind::Conflicted),
    Delta::Ignored | Delta::Unreadable | Delta::Untracked | Delta::Unmodified => None,
  }
}

fn patch_for_delta(diff: &Diff<'_>, delta_ix: usize) -> Result<String> {
  let Some(mut patch) = Patch::from_diff(diff, delta_ix).context("build patch from diff")? else {
    return Ok(String::new());
  };
  let patch_buf = patch.to_buf().context("serialize patch")?;
  Ok(String::from_utf8_lossy(patch_buf.as_ref()).into_owned())
}

fn load_commit_file_content(
  repo: &Repository,
  commit_tree: &Tree<'_>,
  rel_path: &Path,
) -> Result<(String, Option<Vec<u8>>)> {
  let Ok(entry) = commit_tree.get_path(rel_path) else {
    return Ok((String::new(), None));
  };
  let blob = repo
    .find_blob(entry.id())
    .context("load blob from commit")?;
  match String::from_utf8(blob.content().to_vec()) {
    Ok(content) => Ok((content, None)),
    Err(err) => Ok((String::new(), Some(err.into_bytes()))),
  }
}

fn normalize_path(path: &Path) -> String {
  path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::TempRepo;
  use git2::Signature;

  fn commit_text_file(repo_root: &Path, rel_path: &Path, contents: &str, message: &str) -> String {
    crate::test_support::commit_text_file(repo_root, rel_path, contents, message).to_string()
  }

  fn rename_file_and_commit(
    repo_root: &Path,
    old_path: &Path,
    new_path: &Path,
    message: &str,
  ) -> String {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::rename(repo_root.join(old_path), repo_root.join(new_path)).expect("rename file");

    let mut index = repo.index().expect("open index");
    let _ = index.remove_path(old_path);
    index.add_path(new_path).expect("stage renamed file");
    index.write().expect("write index");

    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("find tree");
    let sig = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("parent commit");
    repo
      .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
      .expect("rename commit")
      .to_string()
  }

  fn delete_file_and_commit(repo_root: &Path, rel_path: &Path, message: &str) -> String {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::remove_file(repo_root.join(rel_path)).expect("delete worktree file");

    let mut index = repo.index().expect("open index");
    index.remove_path(rel_path).expect("remove from index");
    index.write().expect("write index");

    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("find tree");
    let sig = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("parent commit");
    repo
      .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
      .expect("delete commit")
      .to_string()
  }

  #[test]
  fn list_commit_history_returns_empty_when_limit_is_zero() {
    let repo = TempRepo::init("history-limit-zero");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "hello\n", "initial");

    let history = list_commit_history(&repo.path, 0).expect("list history");
    assert!(history.is_empty());
  }

  #[test]
  fn list_commit_history_returns_empty_for_repo_without_commits() {
    let repo = TempRepo::init("history-empty");
    let history = list_commit_history(&repo.path, 20).expect("list history");
    assert!(history.is_empty());
  }

  #[test]
  fn current_history_revision_is_empty_for_repo_without_commits() {
    let repo = TempRepo::init("history-revision-empty");
    let revision = current_history_revision(&repo.path).expect("history revision");
    assert_eq!(revision.head_oid, None);
    assert_eq!(revision.head_label, None);
    assert!(revision.refs.is_empty());
  }

  #[test]
  fn list_commit_changed_files_reports_modified_file() {
    let repo = TempRepo::init("history-modified");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    let commit_oid = commit_text_file(&repo.path, rel_path, "v2\n", "update");

    let files = list_commit_changed_files(&repo.path, &commit_oid).expect("changed files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, rel_path);
    assert_eq!(files[0].kind, CommitFileChangeKind::Modified);
  }

  #[test]
  fn list_commit_changed_files_reports_renamed_file() {
    let repo = TempRepo::init("history-renamed");
    let old_path = Path::new("old.txt");
    let new_path = Path::new("new.txt");
    let _ = commit_text_file(&repo.path, old_path, "same content\n", "initial");
    let rename_commit = rename_file_and_commit(&repo.path, old_path, new_path, "rename");

    let files = list_commit_changed_files(&repo.path, &rename_commit).expect("changed files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, new_path);
    assert_eq!(files[0].old_path.as_deref(), Some(old_path));
    assert_eq!(files[0].kind, CommitFileChangeKind::Renamed);
  }

  #[test]
  fn load_commit_file_diff_returns_renamed_file_content() {
    let repo = TempRepo::init("history-rename-diff");
    let old_path = Path::new("old.txt");
    let new_path = Path::new("new.txt");
    let _ = commit_text_file(&repo.path, old_path, "same content\n", "initial");
    let rename_commit = rename_file_and_commit(&repo.path, old_path, new_path, "rename");

    let diff =
      load_commit_file_diff(&repo.path, &rename_commit, new_path).expect("load renamed file diff");
    assert_eq!(diff.file.kind, CommitFileChangeKind::Renamed);
    assert_eq!(diff.file.path, new_path);
    assert_eq!(diff.file.old_path.as_deref(), Some(old_path));
    assert_eq!(diff.content, "same content\n");
  }

  #[test]
  fn load_commit_file_diff_returns_empty_content_for_deleted_file() {
    let repo = TempRepo::init("history-delete-diff");
    let rel_path = Path::new("delete-me.txt");
    let _ = commit_text_file(&repo.path, rel_path, "gone\n", "initial");
    let delete_commit = delete_file_and_commit(&repo.path, rel_path, "delete");

    let diff =
      load_commit_file_diff(&repo.path, &delete_commit, rel_path).expect("load deleted file diff");
    assert_eq!(diff.file.kind, CommitFileChangeKind::Deleted);
    assert_eq!(diff.file.path, rel_path);
    assert_eq!(diff.content, "");
    assert!(
      diff.patch.contains("deleted file mode")
        || (diff.patch.contains("delete-me.txt") && diff.patch.contains("/dev/null"))
    );
  }

  #[test]
  fn the_first_commit_of_a_repository_has_no_side_to_compare_against() {
    let repo = TempRepo::init("history-first-commit");
    let initial = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    // No parent tree: everything the commit holds counts as added.
    let files = list_commit_changed_files(&repo.path, &initial).expect("changed files");

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].kind, CommitFileChangeKind::Added);
    assert_eq!(files[0].path, PathBuf::from("README.md"));
  }

  #[test]
  fn a_range_lists_what_every_commit_of_it_changed() {
    let repo = TempRepo::init("history-range-union");
    let base = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("src/one.rs"), "one\n", "add one");
    let head = commit_text_file(&repo.path, Path::new("src/two.rs"), "two\n", "add two");

    let files = list_range_changed_files(&repo.path, &base, &head).expect("list range");

    let mut paths = files
      .iter()
      .map(|file| file.path.to_string_lossy().to_string())
      .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(paths, vec!["src/one.rs", "src/two.rs"]);
    // The base itself is excluded: README.md landed there.
    assert!(!paths.iter().any(|path| path == "README.md"));
  }

  #[test]
  fn a_file_added_then_deleted_in_the_range_never_shows_up() {
    let repo = TempRepo::init("history-range-transient");
    let base = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("temp.txt"), "scratch\n", "add temp");
    let head = delete_file_and_commit(&repo.path, Path::new("temp.txt"), "drop temp");

    let files = list_range_changed_files(&repo.path, &base, &head).expect("list range");

    assert!(
      files.is_empty(),
      "the range changed nothing in the end, got {files:?}"
    );
  }

  #[test]
  fn a_range_detects_a_rename_and_keeps_the_old_path() {
    let repo = TempRepo::init("history-range-rename");
    commit_text_file(
      &repo.path,
      Path::new("src/old.rs"),
      "fn main() {}\n",
      "initial",
    );
    let base = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "readme");
    let head = rename_file_and_commit(
      &repo.path,
      Path::new("src/old.rs"),
      Path::new("src/new.rs"),
      "rename",
    );

    let files = list_range_changed_files(&repo.path, &base, &head).expect("list range");

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].kind, CommitFileChangeKind::Renamed);
    assert_eq!(files[0].path, PathBuf::from("src/new.rs"));
    assert_eq!(files[0].old_path, Some(PathBuf::from("src/old.rs")));
  }

  #[test]
  fn a_range_file_diff_carries_the_patch_and_the_head_content() {
    let repo = TempRepo::init("history-range-file");
    let base = commit_text_file(&repo.path, Path::new("src/main.rs"), "one\n", "initial");
    commit_text_file(&repo.path, Path::new("src/main.rs"), "two\n", "second");
    let head = commit_text_file(&repo.path, Path::new("src/main.rs"), "three\n", "third");

    let diff = load_range_file_diff(&repo.path, &base, &head, Path::new("src/main.rs"))
      .expect("load range file diff");

    assert_eq!(diff.file.kind, CommitFileChangeKind::Modified);
    // The whole range in one patch: from the base content to the head content.
    assert!(diff.patch.contains("-one"));
    assert!(diff.patch.contains("+three"));
    assert!(!diff.patch.contains("two"));
    assert_eq!(diff.content, "three\n");
  }

  #[test]
  fn a_range_file_diff_refuses_a_path_the_range_never_touched() {
    let repo = TempRepo::init("history-range-untouched");
    let base = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let head = commit_text_file(&repo.path, Path::new("src/main.rs"), "one\n", "add main");

    let error = load_range_file_diff(&repo.path, &base, &head, Path::new("README.md"))
      .expect_err("README.md is untouched by the range");

    assert!(error.to_string().contains("does not change path"));
  }

  #[test]
  fn an_unknown_commit_is_an_error_not_a_panic() {
    let repo = TempRepo::init("history-range-unknown");
    let head = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let missing = "0".repeat(40);

    let error = list_range_changed_files(&repo.path, &missing, &head).expect_err("unknown base");

    assert!(error.to_string().contains("find commit"));
  }

  #[test]
  fn the_merge_base_of_two_branches_is_where_they_parted() {
    let repo = TempRepo::init("history-merge-base");
    let base = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let head = commit_text_file(&repo.path, Path::new("src/main.rs"), "one\n", "add main");

    // A linear history: the older commit is the base of the newer one.
    assert_eq!(
      merge_base(&repo.path, &base, &head).expect("merge base"),
      base
    );
  }
}
