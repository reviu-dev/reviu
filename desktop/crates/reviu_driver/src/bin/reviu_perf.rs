#[path = "../perf_harness.rs"]
mod perf_harness;

fn main() -> anyhow::Result<()> {
  let args = perf_harness::parse_args(std::env::args().skip(1))?;
  perf_harness::run(args)
}
