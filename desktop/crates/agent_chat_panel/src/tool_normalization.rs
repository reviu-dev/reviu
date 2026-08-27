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
  pub tool_name: Option<String>,
  pub content: Vec<ToolCallContent>,
}

impl NormalizedToolCall {
  pub(crate) fn first_location_path(&self) -> Option<&PathBuf> {
    self.locations.first().map(|(path, _)| path)
  }
}

impl From<ToolCall> for NormalizedToolCall {
  fn from(call: ToolCall) -> Self {
    let tool_name = tool_name_from_meta(call.meta.as_ref());
    let locations = normalize_locations(call.locations);
    let kind = normalize_tool_kind(
      call.kind,
      tool_name.as_deref(),
      call.raw_input.as_ref(),
      &locations,
      &call.content,
    );
    Self {
      id: call.tool_call_id,
      title: call.title,
      kind,
      status: call.status,
      locations,
      raw_input: call.raw_input,
      raw_output: call.raw_output,
      meta: call.meta,
      tool_name,
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
  pub tool_name: Option<String>,
  pub content: Option<Vec<ToolCallContent>>,
}

impl From<ToolCallUpdate> for NormalizedToolCallUpdate {
  fn from(update: ToolCallUpdate) -> Self {
    let mut fields = update.fields;
    let tool_name = tool_name_from_meta(update.meta.as_ref());
    let locations = fields.locations.take().map(normalize_locations);
    let content = fields.content.take();
    let kind = fields.kind.map(|kind| {
      normalize_tool_kind(
        kind,
        tool_name.as_deref(),
        fields.raw_input.as_ref(),
        locations.as_deref().unwrap_or_default(),
        content.as_deref().unwrap_or_default(),
      )
    });
    Self {
      id: update.tool_call_id,
      kind,
      status: fields.status,
      title: fields.title,
      locations,
      raw_input: fields.raw_input,
      raw_output: fields.raw_output,
      meta: update.meta,
      tool_name,
      content,
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

pub(crate) fn tool_name_from_meta(meta: Option<&Meta>) -> Option<String> {
  fn non_empty_string(value: &serde_json::Value) -> Option<String> {
    value
      .as_str()
      .map(str::trim)
      .filter(|value| !value.is_empty())
      .map(str::to_string)
  }

  meta.and_then(|meta| {
    meta
      .get("tool_name")
      .and_then(non_empty_string)
      .or_else(|| meta.get("x.ai/tool.name").and_then(non_empty_string))
  })
}

fn raw_input_has_any(raw_input: Option<&serde_json::Value>, keys: &[&str]) -> bool {
  raw_input
    .and_then(|input| input.as_object())
    .is_some_and(|map| {
      keys
        .iter()
        .any(|key| map.get(*key).is_some_and(|value| !value.is_null()))
    })
}

fn normalized_tool_name(tool_name: &str) -> String {
  tool_name
    .chars()
    .map(|character| {
      if character.is_ascii_alphanumeric() {
        character.to_ascii_lowercase()
      } else {
        '_'
      }
    })
    .collect()
}

fn tool_name_has_any(tool_name: &str, tokens: &[&str]) -> bool {
  let normalized = normalized_tool_name(tool_name);
  tokens.iter().any(|token| normalized.contains(token))
}

fn has_diff_content(content: &[ToolCallContent]) -> bool {
  content
    .iter()
    .any(|content| matches!(content, ToolCallContent::Diff(_)))
}

fn normalize_tool_kind(
  kind: ToolKind,
  tool_name: Option<&str>,
  raw_input: Option<&serde_json::Value>,
  locations: &[(PathBuf, Option<u32>)],
  content: &[ToolCallContent],
) -> ToolKind {
  if !matches!(kind, ToolKind::Other) {
    return kind;
  }
  let Some(tool_name) = tool_name else {
    return kind;
  };

  if tool_name_has_any(tool_name, &["edit", "write", "replace"])
    && (has_diff_content(content) || raw_input_has_any(raw_input, &["file_path", "path"]))
  {
    return ToolKind::Edit;
  }
  if tool_name_has_any(tool_name, &["read", "view", "open"])
    && (!locations.is_empty() || raw_input_has_any(raw_input, &["file_path", "path"]))
  {
    return ToolKind::Read;
  }
  if tool_name_has_any(tool_name, &["search", "grep", "rg"])
    && raw_input_has_any(raw_input, &["query", "pattern", "regex"])
  {
    return ToolKind::Search;
  }
  if tool_name_has_any(tool_name, &["execute", "run", "bash", "shell"])
    && raw_input_has_any(raw_input, &["command", "cmd"])
  {
    return ToolKind::Execute;
  }
  if tool_name_has_any(tool_name, &["fetch", "web", "url"])
    && raw_input_has_any(raw_input, &["url", "uri"])
  {
    return ToolKind::Fetch;
  }

  kind
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
