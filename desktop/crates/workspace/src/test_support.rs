//! Git fixtures shared by the tests of every page and component, served by the
//! `git` crate's `test-support` feature.

pub(crate) use git::test_support::{
  TempBareRepo, TempDir, TempRepo, commit_text_file, head_oid, push_branch_to_remote,
  remote_branch_oid, set_remote_head, set_upstream, temp_path,
};
