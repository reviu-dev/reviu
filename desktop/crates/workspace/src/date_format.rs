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

fn pluralized_unit(value: i64, unit: &str) -> String {
  if value == 1 {
    format!("1 {unit} ago")
  } else {
    format!("{value} {unit}s ago")
  }
}

pub(crate) fn format_relative_time_at(value: &str, now: OffsetDateTime) -> SharedString {
  let trimmed = value.trim();
  let Some(parsed) = parse_rfc3339(trimmed) else {
    return trimmed.to_string().into();
  };

  let elapsed_seconds = (now - parsed).whole_seconds();
  if elapsed_seconds <= 60 {
    return "just now".into();
  }

  let elapsed_minutes = elapsed_seconds / 60;
  if elapsed_minutes < 60 {
    return pluralized_unit(elapsed_minutes, "minute").into();
  }

  let elapsed_hours = elapsed_seconds / 3_600;
  if elapsed_hours < 24 {
    return pluralized_unit(elapsed_hours, "hour").into();
  }

  let elapsed_days = elapsed_seconds / 86_400;
  if elapsed_days == 1 {
    return "yesterday".into();
  }
  if elapsed_days < 30 {
    return pluralized_unit(elapsed_days, "day").into();
  }

  let elapsed_months = elapsed_days / 30;
  if elapsed_months < 12 {
    return pluralized_unit(elapsed_months, "month").into();
  }

  let elapsed_years = elapsed_days / 365;
  pluralized_unit(elapsed_years.max(1), "year").into()
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
  fn format_relative_time_at_formats_recent_relative_timestamps() {
    let now = OffsetDateTime::parse("2026-02-20T12:00:00Z", &Rfc3339).expect("parse now");

    assert_eq!(
      format_relative_time_at("2026-02-20T11:59:30Z", now).as_ref(),
      "just now"
    );
    assert_eq!(
      format_relative_time_at("2026-02-20T10:00:00Z", now).as_ref(),
      "2 hours ago"
    );
    assert_eq!(
      format_relative_time_at("2026-02-19T12:00:00Z", now).as_ref(),
      "yesterday"
    );
    assert_eq!(
      format_relative_time_at("2026-02-17T12:00:00Z", now).as_ref(),
      "3 days ago"
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
