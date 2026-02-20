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

pub(crate) fn format_compact_datetime(value: &str) -> SharedString {
  let trimmed = value.trim();
  let Some(parsed) = parse_rfc3339(trimmed) else {
    return trimmed.to_string().into();
  };

  parsed
    .format(format_description!(
      "[month repr:short] [day padding:none], [year] [hour]:[minute]"
    ))
    .unwrap_or_else(|_| trimmed.to_string())
    .into()
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
  fn format_compact_datetime_keeps_source_offset_clock_time() {
    assert_eq!(
      format_compact_datetime("2026-02-20T12:34:56+02:00").as_ref(),
      "Feb 20, 2026 12:34"
    );
  }

  #[test]
  fn formatting_falls_back_to_raw_value_when_not_parseable() {
    assert_eq!(format_long_date("not-a-date").as_ref(), "not-a-date");
    assert_eq!(format_compact_datetime("2026-02-20").as_ref(), "2026-02-20");
  }

  #[test]
  fn format_long_date_opt_returns_dash_for_none() {
    assert_eq!(format_long_date_opt(None).as_ref(), "—");
  }
}
