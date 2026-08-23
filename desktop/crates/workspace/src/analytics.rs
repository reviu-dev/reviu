use std::time::Duration;

use gpui::{App, AppContext as _, Global};
use reqwest::blocking::Client;
use serde_json::{Value, json};

use crate::AppProfile;
use crate::config::{AppSettings, ConfigStore};

const UMAMI_ENDPOINT: &str = "https://analytics.jorisgallot.dev/api/send";
const UMAMI_WEBSITE_ID: &str = "6dea885c-63c9-45d3-8dd0-df46756e5b31";
const UMAMI_HOSTNAME: &str = "app.reviu.dev";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct Analytics {
  client: Client,
  device_id: String,
}

impl Global for Analytics {}

impl Analytics {
  pub fn init(cx: &mut App) {
    if AppProfile::current().is_dev() {
      return;
    }
    let Some(device_id) = ConfigStore::load_or_create_analytics_device_id() else {
      return;
    };
    let user_agent = format!(
      "Reviu/{} ({})",
      env!("CARGO_PKG_VERSION"),
      std::env::consts::OS
    );
    let client = match Client::builder()
      .timeout(REQUEST_TIMEOUT)
      .user_agent(user_agent)
      .build()
    {
      Ok(client) => client,
      Err(err) => {
        app_log::log!("Failed to build analytics client: {}", err);
        return;
      }
    };
    cx.set_global(Analytics { client, device_id });
  }
}

pub fn track(cx: &mut App, name: &'static str) {
  track_with(cx, name, None);
}

pub fn track_with(cx: &mut App, name: &'static str, data: Option<Value>) {
  let analytics_enabled = cx
    .try_global::<AppSettings>()
    .is_some_and(|settings| settings.analytics_enabled);
  if !analytics_enabled {
    return;
  }
  let Some(analytics) = cx.try_global::<Analytics>().cloned() else {
    return;
  };
  let payload = build_payload(&analytics.device_id, name, data);

  cx.background_spawn(async move {
    let _ = analytics.client.post(UMAMI_ENDPOINT).json(&payload).send();
  })
  .detach();
}

fn build_payload(device_id: &str, name: &str, data: Option<Value>) -> Value {
  let mut event_data = json!({
    "device_id": device_id,
    "version": env!("CARGO_PKG_VERSION"),
    "os": std::env::consts::OS,
  });
  if let (Some(extra), Some(obj)) = (data, event_data.as_object_mut()) {
    obj.insert("extra".to_string(), extra);
  }

  json!({
    "type": "event",
    "payload": {
      "website": UMAMI_WEBSITE_ID,
      "hostname": UMAMI_HOSTNAME,
      "name": name,
      "url": "/",
      "data": event_data,
    }
  })
}
