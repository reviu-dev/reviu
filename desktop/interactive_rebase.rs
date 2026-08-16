
#[cfg(test)]
mod probe {
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};

  #[test]
  fn probe_rebase_message() {
    let repo = TempRepo::init("probe-rebase-message");
    commit_text_file(&repo.path, Path::new("a.txt"), "base\n", "initial");
    let base = git::current_branch_status(&repo.path).expect("branch").name;
    let feature = git::BranchRef { name: "feature".into(), kind: git::BranchKind::Local };
    git::create_branch(&repo.path, &feature.name).unwrap();
    git::switch_branch(&repo.path, &feature).unwrap();
    commit_text_file(&repo.path, Path::new("a.txt"), "feature\n", "feature work");
    let base_ref = git::BranchRef { name: base.clone(), kind: git::BranchKind::Local };
    git::switch_branch(&repo.path, &base_ref).unwrap();
    commit_text_file(&repo.path, Path::new("a.txt"), "main\n", "main work");
    git::switch_branch(&repo.path, &feature).unwrap();
    let target = InteractiveRebaseTarget::Branch(base_ref);
    let commits = prepare_commits(&repo.path, &target).unwrap().commits;
    let todo = commits.iter().map(|c| git::InteractiveRebaseTodoEntry { oid: c.oid.clone(), action: git::InteractiveRebaseAction::Pick }).collect::<Vec<_>>();
    let result = git::start_interactive_rebase(&repo.path, &target, &todo);
    println!("PROBE result: {result:?}");
    println!("PROBE in progress: {:?}", git::is_rebase_in_progress(&repo.path));
    println!("PROBE message: {:?}", git::current_rebase_commit_message(&repo.path));
    println!("PROBE conflicted: {:?}", crate::repo_command::first_conflicted_path(&repo.path));
  }
}
