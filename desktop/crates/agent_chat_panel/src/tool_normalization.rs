use agent_client_protocol::schema::{
  Meta, ToolCall, ToolCallContent, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct NormalizedToolCall {
  pub id: ToolCallId,
  pub title: String,
  pub kind: ToolKind,
  pub status: ToolCallStatus,
  pub locations: Vec<(PathBuf, Option<u32>)>,
  pub raw_input: Option<serde_json::Value>,
  pub raw_output: Option<serde_json::Value>,
  pub meta: Option<Meta>,
  pub content: Vec<ToolCallContent>,
}

impl NormalizedToolCall {
  pub(crate) fn first_location_path(&self) -> Option<&PathBuf> {
    self.locations.first().map(|(path, _)| path)
  }
}

impl From<ToolCall> for NormalizedToolCall {
  fn from(call: ToolCall) -> Self {
    Self {
      id: call.tool_call_id,
      title: call.title,
      kind: call.kind,
      status: call.status,
      locations: normalize_locations(call.locations),
      raw_input: call.raw_input,
      raw_output: call.raw_output,
      meta: call.meta,
      content: call.content,
    }
  }
}

#[derive(Clone, Debug)]
pub(crate) struct NormalizedToolCallUpdate {
  pub id: ToolCallId,
  pub kind: Option<ToolKind>,
  pub status: Option<ToolCallStatus>,
  pub title: Option<String>,
  pub locations: Option<Vec<(PathBuf, Option<u32>)>>,
  pub raw_input: Option<serde_json::Value>,
  pub raw_output: Option<serde_json::Value>,
  pub meta: Option<Meta>,
  pub content: Option<Vec<ToolCallContent>>,
}

impl From<ToolCallUpdate> for NormalizedToolCallUpdate {
  fn from(update: ToolCallUpdate) -> Self {
    Self {
      id: update.tool_call_id,
      kind: update.fields.kind,
      status: update.fields.status,
      title: update.fields.title,
      locations: update.fields.locations.map(normalize_locations),
      raw_input: update.fields.raw_input,
      raw_output: update.fields.raw_output,
      meta: update.meta,
      content: update.fields.content,
    }
  }
}

pub(crate) fn normalize_locations(
  locations: Vec<agent_client_protocol::schema::ToolCallLocation>,
) -> Vec<(PathBuf, Option<u32>)> {
  locations
    .into_iter()
    .map(|location| (location.path, location.line))
    .collect()
}

pub(crate) fn tool_payload_fp(
  content: &[ToolCallContent],
  first_location: Option<&PathBuf>,
  raw_output: Option<&serde_json::Value>,
  meta: Option<&Meta>,
) -> u64 {
  let mut bytes = serde_json::to_vec(content).unwrap_or_default();
  if let Some(path) = first_location {
    bytes.extend_from_slice(path.to_string_lossy().as_bytes());
  }
  if let Some(raw_output) = raw_output {
    bytes.extend_from_slice(&serde_json::to_vec(raw_output).unwrap_or_default());
  }
  if let Some(meta) = meta {
    bytes.extend_from_slice(&serde_json::to_vec(meta).unwrap_or_default());
  }
  crate::code_block::fnv1a(&bytes)
}
