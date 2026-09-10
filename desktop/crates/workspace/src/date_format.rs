use gpui::SharedString;
use time::{OffsetDateTime, format_description::well_known::Rfc3339, macros::format_description};

pub(crate) fn parse_rfc3339(value: &str) -> Option<OffsetDateTime> {
  OffsetDateTime::parse(value.trim(), &Rfc3339).ok()
}

pub(crate) fn format_long_date(value: &str) -> SharedString {
  let trimmed = value.trim();
  let Some(parsed) = parse_rfc3339(trimmed) else {
    return trimmed.to_string().into();
  };

  parsed
    .format(format_description!(
      "[month repr:long] [day padding:none], [year]"
    ))
    .unwrap_or_else(|_| trimmed.to_string())
    .into()
}

pub(crate) fn format_long_date_opt(value: Option<&str>) -> SharedString {
  let Some(value) = value else {
    return "—".into();
  };
  format_long_date(value)
}

pub(crate) fn format_relative_secs(updated_at_secs: u64, now_secs: u64) -> String {
  let delta = now_secs.saturating_sub(updated_at_secs);
  match delta {
    0..=59 => "now".to_string(),
    60..=3_599 => format!("{}m", delta / 60),
    3_600..=86_399 => format!("{}h", delta / 3_600),
    _ => format!("{}d", delta / 86_400),
  }
}

pub(crate) fn format_relative_age(updated_at_secs: u64, now_secs: u64) -> String {
  match format_relative_secs(updated_at_secs, now_secs).as_str() {
    "now" => "now".to_string(),
    label => format!("{label} ago"),
  }
}

pub(crate) fn format_relative_time_at(value: &str, now: OffsetDateTime) -> SharedString {
  let trimmed = value.trim();
  let Some(parsed) = parse_rfc3339(trimmed) else {
    return trimmed.to_string().into();
  };

  let Some(updated_at_secs) = u64::try_from(parsed.unix_timestamp()).ok() else {
    return "now".into();
  };
  let now_secs = u64::try_from(now.unix_timestamp()).unwrap_or(0);
  format_relative_age(updated_at_secs, now_secs).into()
}

pub(crate) fn format_relative_time(value: &str) -> SharedString {
  format_relative_time_at(value, OffsetDateTime::now_utc())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn format_long_date_parses_rfc3339_z() {
    assert_eq!(
      format_long_date("2026-02-20T15:42:30Z").as_ref(),
      "February 20, 2026"
    );
  }

  #[test]
  fn formatting_falls_back_to_raw_value_when_not_parseable() {
    assert_eq!(format_long_date("not-a-date").as_ref(), "not-a-date");
  }

  #[test]
  fn format_long_date_opt_returns_dash_for_none() {
    assert_eq!(format_long_date_opt(None).as_ref(), "—");
  }

  #[test]
  fn format_relative_secs_buckets() {
    assert_eq!(format_relative_secs(100, 100), "now");
    assert_eq!(format_relative_secs(100, 159), "now");
    assert_eq!(format_relative_secs(100, 160), "1m");
    assert_eq!(format_relative_secs(100, 100 + 3_600), "1h");
    assert_eq!(format_relative_secs(100, 100 + 86_400), "1d");
    assert_eq!(format_relative_secs(100, 100 + 3 * 86_400), "3d");
  }

  #[test]
  fn format_relative_secs_clamps_future_timestamps() {
    assert_eq!(format_relative_secs(200, 100), "now");
  }

  #[test]
  fn format_relative_age_adds_context_to_elapsed_time() {
    assert_eq!(format_relative_age(100, 100), "now");
    assert_eq!(format_relative_age(100, 100 + 60), "1m ago");
  }

  #[test]
  fn format_relative_time_at_formats_compact_relative_timestamps() {
    let now = OffsetDateTime::parse("2026-02-20T12:00:00Z", &Rfc3339).expect("parse now");

    assert_eq!(
      format_relative_time_at("2026-02-20T11:59:30Z", now).as_ref(),
      "now"
    );
    assert_eq!(
      format_relative_time_at("2026-02-20T10:00:00Z", now).as_ref(),
      "2h ago"
    );
    assert_eq!(
      format_relative_time_at("2026-02-19T12:00:00Z", now).as_ref(),
      "1d ago"
    );
    assert_eq!(
      format_relative_time_at("2026-02-17T12:00:00Z", now).as_ref(),
      "3d ago"
    );
  }

  #[test]
  fn format_relative_time_at_falls_back_to_raw_value_when_not_parseable() {
    let now = OffsetDateTime::parse("2026-02-20T12:00:00Z", &Rfc3339).expect("parse now");
    assert_eq!(
      format_relative_time_at("not-a-date", now).as_ref(),
      "not-a-date"
    );
  }
}
