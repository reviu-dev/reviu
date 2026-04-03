use std::{fmt::Write, hint::black_box, path::Path, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use git::{GitFileBases, compute_buffer_diffs};

fn base_text(lines: usize) -> String {
  let mut text = String::with_capacity(lines.saturating_mul(48));
  for line in 0..lines {
    let _ = writeln!(
      text,
      "const value_{line:06} = {{ kind: \"stable\", idx: {}, bucket: {} }};",
      line % 97,
      line % 11
    );
  }
  text
}

fn modified_text(lines: usize) -> String {
  let mut text = String::with_capacity(lines.saturating_mul(64));
  for line in 0..lines {
    if line % 200 == 0 {
      let _ = writeln!(
        text,
        "<section id=\"row-{line}\" data-row=\"{line}\"><div class=\"copy\">row {line}</div><span class=\"hint\">shared html benchmark content {}</span></section>",
        line % 13
      );
      continue;
    }

    if line % 17 == 0 {
      let _ = writeln!(
        text,
        "const value_{line:06} = {{ kind: \"changed\", idx: {}, bucket: {}, extra: \"diff\" }};",
        (line * 7) % 101,
        line % 19
      );
      continue;
    }

    if line % 53 == 0 {
      let _ = writeln!(
        text,
        "const inserted_{line:06} = {{ before: {}, after: {}, tag: \"bench\" }};",
        line.saturating_sub(1),
        line + 1
      );
      let _ = writeln!(
        text,
        "const value_{line:06} = {{ kind: \"stable\", idx: {}, bucket: {} }};",
        line % 97,
        line % 11
      );
      continue;
    }

    let _ = writeln!(
      text,
      "const value_{line:06} = {{ kind: \"stable\", idx: {}, bucket: {} }};",
      line % 97,
      line % 11
    );
  }
  text
}

fn benchmark_criterion() -> Criterion {
  Criterion::default()
    .warm_up_time(Duration::from_secs(1))
    .measurement_time(Duration::from_secs(5))
    .sample_size(10)
}

fn bench_compute_buffer_diffs(c: &mut Criterion) {
  let mut group = c.benchmark_group("git/compute_buffer_diffs");

  for &lines in &[10_000usize, 50_000usize] {
    let base = base_text(lines);
    let modified = modified_text(lines);
    let bases = GitFileBases {
      head: Some(base.clone()),
      index: Some(base),
    };

    group.throughput(Throughput::Elements(lines as u64));
    group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
      b.iter(|| {
        compute_buffer_diffs(
          black_box(&bases),
          black_box(modified.as_str()),
          black_box(Path::new("bench.txt")),
        )
        .expect("compute buffer diffs")
      })
    });
  }

  group.finish();
}

criterion_group! {
  name = benches;
  config = benchmark_criterion();
  targets = bench_compute_buffer_diffs
}
criterion_main!(benches);
