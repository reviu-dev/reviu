#[path = "../git_smoke_harness.rs"]
mod git_smoke_harness;

fn main() -> anyhow::Result<()> {
  let args = git_smoke_harness::parse_args(std::env::args().skip(1))?;
  git_smoke_harness::run(args)
}
