//! Pure functions over the transcript: turns, tool calls, checkpoints.

use super::*;

/// The marker goes right before the prompt that triggered it (the last user-authored
/// message), so a rollback lands on the state that preceded that prompt.
pub(crate) fn checkpoint_insert_index(items: &[ChatItem]) -> usize {
  items
    .iter()
    .rposition(|item| {
      matches!(
        item,
        ChatItem::Message(ChatMessage {
          role: ChatRole::User | ChatRole::ReviewExport,
          ..
        })
      )
    })
    .unwrap_or(items.len())
}

pub(crate) fn tool_index_for_items(items: &[ChatItem]) -> HashMap<ToolCallId, usize> {
  items
    .iter()
    .enumerate()
    .filter_map(|(ix, item)| match item {
      ChatItem::Tool(tool) => Some((tool.id.clone(), ix)),
      _ => None,
    })
    .collect()
}

/// Number of items to keep so the checkpoint marker is the last remaining item.
pub(crate) fn checkpoint_truncate_len(items: &[ChatItem], ref_name: &str) -> Option<usize> {
  items
    .iter()
    .position(|item| matches!(item, ChatItem::Checkpoint(marker) if marker.ref_name == ref_name))
    .map(|marker_ix| marker_ix + 1)
}

pub(crate) fn populate_syntax_spans(view: &mut ToolCallView) {
  let out_lang = view
    .locations
    .first()
    .and_then(|(p, _)| languages::detect_language_config_for_path(p));
  if let Some(cfg) = out_lang {
    for out in &mut view.outputs {
      let mut h = SyntaxHighlighter::new(cfg);
      out.syntax_spans = h.highlight_text(&out.text).unwrap_or_default();
    }
  }
  for d in &mut view.diffs {
    let Some(cfg) = languages::detect_language_config_for_path(std::path::Path::new(&d.path))
    else {
      continue;
    };
    for line in &mut d.lines {
      let mut h = SyntaxHighlighter::new(cfg);
      line.syntax_spans = h.highlight_text(&line.text).unwrap_or_default();
    }
  }
}

pub(crate) fn read_tool_output_start_line(
  kind: &ToolKind,
  locations: &[(PathBuf, Option<u32>)],
) -> Option<u32> {
  read_tool_start_line(kind, locations, None)
}

fn read_tool_explicit_start_line(
  kind: &ToolKind,
  locations: &[(PathBuf, Option<u32>)],
  raw_input: Option<&serde_json::Value>,
) -> Option<u32> {
  if !matches!(kind, ToolKind::Read) {
    return None;
  }
  let from_location = locations.first().and_then(|(_, line)| *line);
  let from_input = raw_input
    .and_then(|input| input.get("offset"))
    .and_then(|offset| offset.as_u64())
    .map(|offset| offset.clamp(1, u32::MAX as u64) as u32);
  from_location.or(from_input)
}

/// First file line of a Read tool's output: an explicit location line wins,
/// then the `offset` in the tool's raw input (Claude-style Read), then 1 when
/// a location at least proves this is a file read.
pub(crate) fn read_tool_start_line(
  kind: &ToolKind,
  locations: &[(PathBuf, Option<u32>)],
  raw_input: Option<&serde_json::Value>,
) -> Option<u32> {
  read_tool_explicit_start_line(kind, locations, raw_input)
    .or_else(|| (matches!(kind, ToolKind::Read) && !locations.is_empty()).then_some(1))
}

/// Fingerprint of the pieces that feed diffs, outputs and highlight spans.
pub(crate) fn tool_content_fp(
  content: &[ToolCallContent],
  first_location: Option<&PathBuf>,
) -> u64 {
  let mut bytes = serde_json::to_vec(content).unwrap_or_default();
  if let Some(path) = first_location {
    bytes.extend_from_slice(path.to_string_lossy().as_bytes());
  }
  code_block::fnv1a(&bytes)
}

pub(crate) fn upsert_tool_call_pure(
  items: &mut Vec<ChatItem>,
  index: &mut HashMap<ToolCallId, usize>,
  call: ToolCall,
  cwd: &std::path::Path,
) {
  let locations: Vec<(PathBuf, Option<u32>)> = call
    .locations
    .into_iter()
    .map(|l| (l.path, l.line))
    .collect();
  let content_fp = tool_content_fp(&call.content, locations.first().map(|(p, _)| p));
  let read_start_line = read_tool_start_line(&call.kind, &locations, call.raw_input.as_ref());
  if let Some(&idx) = index.get(&call.tool_call_id)
    && let Some(ChatItem::Tool(existing)) = items.get_mut(idx)
  {
    existing.title = call.title;
    existing.kind = call.kind;
    existing.status = call.status;
    existing.locations = locations;
    existing.read_start_line = read_start_line.or(existing.read_start_line);
    // Same content: keep diffs, outputs, spans and their expansion state.
    if existing.content_fp != content_fp {
      existing.diffs = extract_diffs(&call.content, cwd);
      existing.outputs = extract_outputs(&call.content, existing.read_start_line);
      existing.terminals = extract_terminals(&call.content);
      existing.content_fp = content_fp;
      populate_syntax_spans(existing);
    }
    return;
  }
  let mut view = ToolCallView {
    id: call.tool_call_id.clone(),
    title: call.title,
    kind: call.kind,
    status: call.status,
    locations,
    diffs: extract_diffs(&call.content, cwd),
    outputs: extract_outputs(&call.content, read_start_line),
    terminals: extract_terminals(&call.content),
    read_start_line,
    content_fp,
  };
  populate_syntax_spans(&mut view);
  let idx = items.len();
  index.insert(call.tool_call_id, idx);
  items.push(ChatItem::Tool(view));
}

pub(crate) fn apply_tool_call_update_pure(
  items: &mut [ChatItem],
  index: &HashMap<ToolCallId, usize>,
  update: ToolCallUpdate,
  cwd: &std::path::Path,
) {
  let Some(&idx) = index.get(&update.tool_call_id) else {
    return;
  };
  let Some(ChatItem::Tool(view)) = items.get_mut(idx) else {
    return;
  };
  if let Some(kind) = update.fields.kind {
    view.kind = kind;
  }
  if let Some(status) = update.fields.status {
    view.status = status;
  }
  if let Some(title) = update.fields.title {
    view.title = title;
  }
  if let Some(locs) = update.fields.locations {
    view.locations = locs.into_iter().map(|l| (l.path, l.line)).collect();
  }
  // Updates rarely repeat raw input; only an explicit line or offset can
  // replace the one captured at the initial call.
  view.read_start_line = read_tool_explicit_start_line(
    &view.kind,
    &view.locations,
    update.fields.raw_input.as_ref(),
  )
  .or(view.read_start_line)
  .or_else(|| read_tool_output_start_line(&view.kind, &view.locations));
  if let Some(content) = update.fields.content {
    let content_fp = tool_content_fp(&content, view.locations.first().map(|(p, _)| p));
    if view.content_fp != content_fp {
      view.diffs = extract_diffs(&content, cwd);
      view.outputs = extract_outputs(&content, view.read_start_line);
      view.terminals = extract_terminals(&content);
      view.content_fp = content_fp;
      // Only fresh content is worth a re-highlight; a status flip is not.
      populate_syntax_spans(view);
    }
  }
}

/// Bounds of the consecutive tool-call run containing `idx`, thoughts
/// included: the agent thinks between its calls, and the fold hides the whole
/// work span. Returns the tool count; fewer than two tools is no group.
pub(crate) fn tool_group_span(items: &[ChatItem], idx: usize) -> Option<(usize, usize, usize)> {
  let in_span = |item: &ChatItem| matches!(item, ChatItem::Tool(_) | ChatItem::Thought(_));
  if !items.get(idx).is_some_and(in_span) {
    return None;
  }
  let mut start = idx;
  while start > 0 && in_span(&items[start - 1]) {
    start -= 1;
  }
  let mut end = idx;
  while end + 1 < items.len() && in_span(&items[end + 1]) {
    end += 1;
  }
  let tools = items[start..=end]
    .iter()
    .filter(|item| matches!(item, ChatItem::Tool(_)))
    .count();
  (tools >= 2).then_some((start, end, tools))
}

pub(crate) fn first_tool_id_in(items: &[ChatItem], start: usize, end: usize) -> Option<ToolCallId> {
  items[start..=end].iter().find_map(|item| match item {
    ChatItem::Tool(t) => Some(t.id.clone()),
    _ => None,
  })
}

/// "Ran 2 commands · Edited 1 file · 1 failed" for a run of tool calls.
/// Failures stay a tail count, never a red header: one failed step does not
/// taint the whole group.
pub(crate) fn tool_group_summary(tools: &[&ToolCallView]) -> String {
  let mut order: Vec<&'static str> = Vec::new();
  let mut counts: HashMap<&'static str, usize> = HashMap::new();
  let mut failed = 0usize;
  for tool in tools {
    let label = tool_kind_label(&tool.kind);
    if !counts.contains_key(label) {
      order.push(label);
    }
    *counts.entry(label).or_insert(0) += 1;
    if matches!(tool.status, ToolCallStatus::Failed) {
      failed += 1;
    }
  }
  let mut parts: Vec<String> = order
    .into_iter()
    .map(|label| {
      let n = counts[label];
      let s = if n > 1 { "s" } else { "" };
      match label {
        "Run" => format!("Ran {n} command{s}"),
        "Edit" => format!("Edited {n} file{s}"),
        "Read" => format!("Read {n} file{s}"),
        "Search" => format!("Ran {n} search{s}"),
        "Delete" => format!("Deleted {n} file{s}"),
        "Move" => format!("Moved {n} file{s}"),
        "Fetch" => format!("Fetched {n} page{s}"),
        _ => format!("{n} tool call{s}"),
      }
    })
    .collect();
  if failed > 0 {
    parts.push(format!("{failed} failed"));
  }
  parts.join(" · ")
}

/// The checkpoint guarding the prompt at `idx`: the nearest marker above it
/// with no other user prompt in between.
pub(crate) fn checkpoint_ref_before(items: &[ChatItem], idx: usize) -> Option<String> {
  for item in items.get(..idx)?.iter().rev() {
    match item {
      ChatItem::Checkpoint(marker) => return Some(marker.ref_name.clone()),
      ChatItem::Message(m) if m.role == ChatRole::User => return None,
      _ => {}
    }
  }
  None
}

/// The summary card hiding this item, if any: everything a settled turn did
/// folds behind its card, except the trailing prose run (the final answer).
pub(crate) fn hiding_turn_summary(items: &[ChatItem], idx: usize) -> Option<usize> {
  match items.get(idx)? {
    ChatItem::Message(m) if !matches!(m.role, ChatRole::Agent) => return None,
    ChatItem::Checkpoint(_) | ChatItem::TurnSummary(_) => return None,
    _ => {}
  }
  let mut trailing_prose = matches!(
    items.get(idx),
    Some(ChatItem::Message(m)) if m.role == ChatRole::Agent
  );
  let mut j = idx;
  loop {
    j += 1;
    match items.get(j) {
      None => return None,
      Some(ChatItem::TurnSummary(_)) => break,
      Some(ChatItem::Message(m)) if m.role == ChatRole::Agent => {}
      // A turn boundary before any card: this turn has no summary to fold into.
      Some(ChatItem::Checkpoint(_)) | Some(ChatItem::Message(_)) => return None,
      // Work after this item disqualifies it from the trailing answer.
      Some(_) => trailing_prose = false,
    }
  }
  if trailing_prose { None } else { Some(j) }
}

/// The number of items a card folds away.
pub(crate) fn folded_step_count(items: &[ChatItem], summary_idx: usize) -> usize {
  (0..summary_idx)
    .rev()
    .take_while(|&i| {
      !matches!(
        items.get(i),
        Some(ChatItem::Checkpoint(_))
          | Some(ChatItem::TurnSummary(_))
          | Some(ChatItem::Message(ChatMessage {
            role: ChatRole::User | ChatRole::ReviewExport | ChatRole::System,
            ..
          }))
      )
    })
    .filter(|&i| hiding_turn_summary(items, i) == Some(summary_idx))
    .count()
}

/// Whether this summary card closes the latest turn. Undoing an older turn's
/// files would clobber the turns that came after it.
pub(crate) fn is_trailing_turn_summary(items: &[ChatItem], idx: usize) -> bool {
  items.get(idx + 1..).is_none_or(|rest| {
    !rest.iter().any(|item| {
      matches!(
        item,
        ChatItem::Message(ChatMessage {
          role: ChatRole::User | ChatRole::ReviewExport,
          ..
        }) | ChatItem::Checkpoint(_)
          | ChatItem::TurnSummary(_)
      )
    })
  })
}

/// Files edited since the current turn began, aggregated by path in first-edit
/// order, plus the checkpoint guarding that turn.
pub(crate) fn turn_edit_stats(items: &[ChatItem]) -> (Vec<TurnFileStat>, Option<String>) {
  let mut boundary = 0usize;
  for (idx, item) in items.iter().enumerate().rev() {
    match item {
      ChatItem::Message(m) if matches!(m.role, ChatRole::User | ChatRole::ReviewExport) => {
        boundary = idx;
        break;
      }
      ChatItem::Checkpoint(_) | ChatItem::TurnSummary(_) => {
        boundary = idx;
        break;
      }
      _ => {}
    }
  }
  let checkpoint_ref = match items.get(boundary) {
    Some(ChatItem::Checkpoint(marker)) => Some(marker.ref_name.clone()),
    _ => checkpoint_ref_before(items, boundary),
  };
  let mut files: Vec<TurnFileStat> = Vec::new();
  let mut by_path: HashMap<String, usize> = HashMap::new();
  for item in &items[boundary..] {
    let ChatItem::Tool(tool) = item else { continue };
    for diff in &tool.diffs {
      let ix = *by_path.entry(diff.path.clone()).or_insert_with(|| {
        files.push(TurnFileStat {
          path: diff.path.clone(),
          added: 0,
          removed: 0,
        });
        files.len() - 1
      });
      files[ix].added += diff.added;
      files[ix].removed += diff.removed;
    }
  }
  (files, checkpoint_ref)
}

pub(crate) fn place_checkpoint_marker(items: &mut Vec<ChatItem>, marker: ChatItem) {
  let insert_ix = checkpoint_insert_index(items);
  if insert_ix > 0 && matches!(items.get(insert_ix - 1), Some(ChatItem::Checkpoint(_))) {
    items[insert_ix - 1] = marker;
  } else {
    items.insert(insert_ix, marker);
  }
}
