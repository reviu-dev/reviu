pub(crate) fn format_compact_number(value: u64) -> String {
  if value >= 1_000_000 {
    let n = value as f64 / 1_000_000.0;
    let formatted = format!("{:.1}", n);
    let formatted = formatted.trim_end_matches(".0");
    format!("{formatted}M")
  } else if value >= 1_000 {
    let n = value as f64 / 1_000.0;
    let formatted = format!("{:.1}", n);
    let formatted = formatted.trim_end_matches(".0");
    format!("{formatted}k")
  } else {
    value.to_string()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn formats_small_numbers() {
    assert_eq!(format_compact_number(0), "0");
    assert_eq!(format_compact_number(1), "1");
    assert_eq!(format_compact_number(999), "999");
  }

  #[test]
  fn formats_thousands() {
    assert_eq!(format_compact_number(1_000), "1k");
    assert_eq!(format_compact_number(1_100), "1.1k");
    assert_eq!(format_compact_number(1_500), "1.5k");
    assert_eq!(format_compact_number(9_999), "10k");
    assert_eq!(format_compact_number(24_794), "24.8k");
    assert_eq!(format_compact_number(100_000), "100k");
    assert_eq!(format_compact_number(999_999), "1000k");
  }

  #[test]
  fn formats_millions() {
    assert_eq!(format_compact_number(1_000_000), "1M");
    assert_eq!(format_compact_number(1_200_000), "1.2M");
    assert_eq!(format_compact_number(10_500_000), "10.5M");
    assert_eq!(format_compact_number(100_000_000), "100M");
  }
}
