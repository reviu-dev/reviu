use std::{
  collections::HashMap,
  sync::{Arc, Mutex},
  time::{SystemTime, UNIX_EPOCH},
};

use gpui::{App, Global};
use ui::{
  CommandPaletteCommandId, CommandPaletteUsageRecorderGlobal, CommandPaletteUsageScorerGlobal,
};

use crate::config::{COMMAND_USAGE_TIMESTAMP_CAP, ConfigStore};

pub const FRECENCY_DECAY_LAMBDA_PER_DAY: f64 = 0.05;

#[derive(Clone, Default)]
pub struct CommandUsageStore {
  usages: Arc<Mutex<HashMap<String, Vec<i64>>>>,
}

impl Global for CommandUsageStore {}

impl CommandUsageStore {
  pub fn load() -> Self {
    Self {
      usages: Arc::new(Mutex::new(ConfigStore::load_command_usages())),
    }
  }

  pub fn record(cx: &App, id: &str) {
    let now = current_unix_secs();
    let snapshot = {
      let store = cx.global::<Self>();
      let Ok(mut guard) = store.usages.lock() else {
        return;
      };
      let entry = guard.entry(id.to_string()).or_default();
      entry.push(now);
      if entry.len() > COMMAND_USAGE_TIMESTAMP_CAP {
        let overflow = entry.len() - COMMAND_USAGE_TIMESTAMP_CAP;
        entry.drain(..overflow);
      }
      entry.clone()
    };
    ConfigStore::persist_command_usage(id, &snapshot);
  }

  pub fn score(cx: &App, id: &str, now_secs: i64) -> f64 {
    let store = cx.global::<Self>();
    let Ok(guard) = store.usages.lock() else {
      return 0.0;
    };
    let Some(timestamps) = guard.get(id) else {
      return 0.0;
    };
    score_timestamps(timestamps, now_secs)
  }
}

fn record_usage(id: CommandPaletteCommandId, cx: &App) {
  CommandUsageStore::record(cx, id.as_str());
}

fn score_usage(cx: &App, id: CommandPaletteCommandId, now_secs: i64) -> f64 {
  CommandUsageStore::score(cx, id.as_str(), now_secs)
}

pub fn install_palette_usage_recorder(cx: &mut App) {
  cx.set_global(CommandPaletteUsageRecorderGlobal(record_usage));
  cx.set_global(CommandPaletteUsageScorerGlobal(score_usage));
}

fn score_timestamps(timestamps: &[i64], now_secs: i64) -> f64 {
  timestamps
    .iter()
    .map(|&ts| {
      let age_days = ((now_secs - ts).max(0) as f64) / 86_400.0;
      let weight = (-FRECENCY_DECAY_LAMBDA_PER_DAY * age_days).exp();
      if weight.is_finite() { weight } else { 0.0 }
    })
    .sum()
}

pub fn current_unix_secs() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
  use super::*;

  const DAY_SECS: i64 = 86_400;

  #[test]
  fn score_is_zero_for_unknown_id() {
    assert_eq!(score_timestamps(&[], 0), 0.0);
  }

  #[test]
  fn recent_usage_scores_higher_than_old() {
    let now = 100 * DAY_SECS;
    let recent = score_timestamps(&[now - DAY_SECS], now);
    let old = score_timestamps(&[now - 30 * DAY_SECS], now);
    assert!(recent > old);
    assert!(recent > 0.9);
    assert!(old < 0.25);
  }

  #[test]
  fn frequency_boosts_score() {
    let now = 100 * DAY_SECS;
    let single = score_timestamps(&[now - DAY_SECS], now);
    let many = score_timestamps(&[now - DAY_SECS; 5], now);
    assert!(many > single * 4.5);
  }

  #[test]
  fn negative_age_is_clamped() {
    let now = 100 * DAY_SECS;
    let future = score_timestamps(&[now + DAY_SECS], now);
    let present = score_timestamps(&[now], now);
    assert!((future - present).abs() < 1e-9);
  }

  #[test]
  fn half_life_is_roughly_two_weeks() {
    let now = 100 * DAY_SECS;
    let at_zero = score_timestamps(&[now], now);
    let at_half_life = score_timestamps(&[now - 14 * DAY_SECS], now);
    let ratio = at_half_life / at_zero;
    assert!(ratio > 0.45 && ratio < 0.55, "ratio was {}", ratio);
  }
}
