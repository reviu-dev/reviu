use std::{collections::HashMap, fmt::Write, hint::black_box, path::Path, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use editor::{GapId, GapReveal, Projection, benchmark_word_diff_ranges};
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

fn html_stress_line(target_bytes: usize, variant: &str) -> String {
  let mut line = String::with_capacity(target_bytes.saturating_add(256));
  let mut idx = 0usize;
  while line.len() < target_bytes {
    let state = match variant {
      "old" => "stable",
      _ => "conflicted",
    };
    let class_name = match variant {
      "old" => "card shell border-neutral copy-muted",
      _ => "card shell border-emerald copy-strong shadow-2",
    };
    let emphasis = match variant {
      "old" => "Pending review payload",
      _ => "Resolved review payload",
    };
    let _ = write!(
      line,
      "<section data-row=\"{idx}\" data-state=\"{state}\" class=\"{class_name}\"><header><span class=\"chip\">row-{idx}</span><span class=\"status\">{emphasis}</span></header><div class=\"content\">rendered-html-fragment-{idx}-{}-{}</div><footer><a href=\"#row-{idx}\">inspect</a><button data-action=\"open-{variant}\">Open</button></footer></section>",
      idx % 17,
      idx % 29
    );
    idx += 1;
  }
  line.truncate(target_bytes);
  line
}

fn html_line_pair(target_bytes: usize) -> (String, String) {
  (
    html_stress_line(target_bytes, "old"),
    html_stress_line(target_bytes, "new"),
  )
}

fn word_diff_total_ranges(old_text: &str, new_text: &str) -> usize {
  let (removed, added) = benchmark_word_diff_ranges(old_text, new_text);
  removed.len() + added.len()
}

fn bench_projection_from_diffs(c: &mut Criterion) {
  let mut group = c.benchmark_group("editor/projection_from_diffs");
  let expanded_gaps: HashMap<GapId, GapReveal> = HashMap::new();

  for &(lines, align_modified) in &[
    (10_000usize, false),
    (10_000usize, true),
    (50_000usize, false),
    (50_000usize, true),
  ] {
    let base = base_text(lines);
    let modified = modified_text(lines);
    let doc_line_count = modified.lines().count();
    let bases = GitFileBases {
      head: Some(base.clone()),
      index: Some(base),
    };
    let diffs =
      compute_buffer_diffs(&bases, modified.as_str(), Path::new("bench.txt")).expect("diffs");

    let mode = if align_modified { "split" } else { "inline" };
    group.throughput(Throughput::Elements(doc_line_count as u64));
    group.bench_with_input(
      BenchmarkId::new(mode, lines),
      &(doc_line_count, align_modified),
      |b, &(doc_line_count, align_modified)| {
        b.iter(|| {
          Projection::from_diffs(
            black_box(doc_line_count),
            black_box(&diffs.uncommitted),
            black_box(&diffs.unstaged),
            black_box(&diffs.staged),
            black_box(&expanded_gaps),
            black_box(align_modified),
          )
        })
      },
    );
  }

  group.finish();
}

fn bench_word_diff_ranges(c: &mut Criterion) {
  let mut group = c.benchmark_group("editor/word_diff_ranges");

  for &bytes in &[1_024usize, 4_096usize, 16_384usize] {
    let (old_text, new_text) = html_line_pair(bytes);
    group.throughput(Throughput::Bytes((old_text.len() + new_text.len()) as u64));
    group.bench_with_input(BenchmarkId::new("html_dense", bytes), &bytes, |b, _| {
      b.iter(|| {
        black_box(word_diff_total_ranges(
          black_box(old_text.as_str()),
          black_box(new_text.as_str()),
        ))
      })
    });
  }

  group.finish();
}

fn bench_word_diff_visible_lines(c: &mut Criterion) {
  let mut group = c.benchmark_group("editor/word_diff_visible_lines");

  for &(visible_lines, bytes_per_line) in &[(32usize, 1_024usize), (64usize, 2_048usize)] {
    let line_pairs = (0..visible_lines)
      .map(|_| html_line_pair(bytes_per_line))
      .collect::<Vec<_>>();
    let total_bytes = line_pairs
      .iter()
      .map(|(old_text, new_text)| old_text.len() + new_text.len())
      .sum::<usize>();
    group.throughput(Throughput::Bytes(total_bytes as u64));
    group.bench_with_input(
      BenchmarkId::new("html_dense", format!("{visible_lines}x{bytes_per_line}")),
      &(visible_lines, bytes_per_line),
      |b, _| {
        b.iter(|| {
          black_box(
            line_pairs
              .iter()
              .map(|(old_text, new_text)| {
                word_diff_total_ranges(black_box(old_text.as_str()), black_box(new_text.as_str()))
              })
              .sum::<usize>(),
          )
        })
      },
    );
  }

  group.finish();
}

criterion_group! {
  name = benches;
  config = benchmark_criterion();
  targets = bench_projection_from_diffs, bench_word_diff_ranges, bench_word_diff_visible_lines
}
criterion_main!(benches);
