use super::*;
use crate::persistence::{list_conversations_in, load_conversation_file};
use agent_acp::PermissionPromptOption;
use agent_client_protocol::schema::{
  Diff, ImageContent, ResourceLink, SessionConfigSelect, SessionConfigSelectOption,
  ToolCallContent, ToolCallLocation, ToolCallUpdateFields,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// Two fixtures created in the same clock tick would otherwise share a directory.
static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_dir(prefix: &str) -> std::path::PathBuf {
  let nanos = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .expect("system clock before unix epoch")
    .as_nanos();
  let unique = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
  let dir = std::env::temp_dir().join(format!(
    "reviu-{prefix}-{}-{nanos}-{unique}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).expect("create temp dir");
  dir
}

fn call(id: &str, title: &str, kind: ToolKind) -> ToolCall {
  let arc: std::sync::Arc<str> = std::sync::Arc::from(id);
  let mut c = ToolCall::new(ToolCallId::new(arc), title.to_string());
  c.kind = kind;
  c
}

fn test_cwd() -> &'static std::path::Path {
  std::path::Path::new("/")
}

fn user_message(text: &str) -> ChatItem {
  ChatItem::Message(ChatMessage {
    role: ChatRole::User,
    text: text.to_string(),
    images: 0,
    image_data: Vec::new(),
  })
}

fn agent_message(text: &str) -> ChatItem {
  ChatItem::Message(ChatMessage {
    role: ChatRole::Agent,
    text: text.to_string(),
    images: 0,
    image_data: Vec::new(),
  })
}

fn checkpoint_marker(ref_name: &str) -> ChatItem {
  ChatItem::Checkpoint(CheckpointMarker {
    ref_name: ref_name.to_string(),
    created_at_secs: 0,
  })
}

#[test]
fn checkpoint_insert_index_lands_before_last_user_prompt() {
  let items = vec![
    user_message("first"),
    agent_message("done"),
    user_message("second"),
  ];
  assert_eq!(checkpoint_insert_index(&items), 2);

  let empty: Vec<ChatItem> = Vec::new();
  assert_eq!(checkpoint_insert_index(&empty), 0);

  let review_only = vec![ChatItem::Message(ChatMessage {
    role: ChatRole::ReviewExport,
    text: "### a.rs:L1 (new side)\nfix\n".to_string(),
    images: 0,
    image_data: Vec::new(),
  })];
  assert_eq!(checkpoint_insert_index(&review_only), 0);
}

#[test]
fn checkpoint_truncate_len_keeps_marker_and_drops_rest() {
  let items = vec![
    checkpoint_marker("refs/reviu/checkpoints/s/1"),
    user_message("first"),
    agent_message("done"),
    checkpoint_marker("refs/reviu/checkpoints/s/2"),
    user_message("second"),
    agent_message("done again"),
  ];

  assert_eq!(
    checkpoint_truncate_len(&items, "refs/reviu/checkpoints/s/2"),
    Some(4)
  );
  assert_eq!(
    checkpoint_truncate_len(&items, "refs/reviu/checkpoints/s/1"),
    Some(1)
  );
  assert_eq!(checkpoint_truncate_len(&items, "refs/unknown"), None);
}

#[test]
fn tool_index_tracks_positions_after_checkpoint_insertion_and_truncation() {
  let tool_view = |id: &str, title: &str, kind: ToolKind| {
    let arc: std::sync::Arc<str> = std::sync::Arc::from(id);
    ChatItem::Tool(ToolCallView {
      id: ToolCallId::new(arc),
      title: title.to_string(),
      kind,
      status: ToolCallStatus::Completed,
      tool_name: None,
      locations: Vec::new(),
      diffs: Vec::new(),
      outputs: Vec::new(),
      terminals: Vec::new(),
      read_start_line: None,
      content_fp: 0,
    })
  };
  let mut items = vec![
    user_message("prompt"),
    tool_view("tool-1", "Read", ToolKind::Read),
    agent_message("done"),
    tool_view("tool-2", "Edit", ToolKind::Edit),
  ];

  // Marker inserted before the prompt shifts every tool index by one.
  items.insert(
    checkpoint_insert_index(&items),
    checkpoint_marker("refs/reviu/checkpoints/s/1"),
  );
  let index = tool_index_for_items(&items);
  let tool_1: std::sync::Arc<str> = std::sync::Arc::from("tool-1");
  let tool_2: std::sync::Arc<str> = std::sync::Arc::from("tool-2");
  assert_eq!(index.get(&ToolCallId::new(tool_1.clone())), Some(&2));
  assert_eq!(index.get(&ToolCallId::new(tool_2.clone())), Some(&4));

  // Truncating at the marker leaves no tool entries behind.
  let keep_len =
    checkpoint_truncate_len(&items, "refs/reviu/checkpoints/s/1").expect("marker present");
  items.truncate(keep_len);
  let index = tool_index_for_items(&items);
  assert!(index.is_empty());
}

#[test]
fn place_checkpoint_marker_replaces_trailing_marker_instead_of_stacking() {
  // After a rollback the marker is the last item; the next prompt's checkpoint
  // must replace it, not stack a second divider.
  let mut items = vec![
    user_message("first"),
    agent_message("done"),
    checkpoint_marker("refs/reviu/checkpoints/s/1"),
    user_message("second"),
  ];
  // The new prompt "second" was just pushed; its checkpoint lands before it.
  place_checkpoint_marker(&mut items, checkpoint_marker("refs/reviu/checkpoints/s/2"));
  assert_eq!(items.len(), 4);
  assert!(
    matches!(&items[2], ChatItem::Checkpoint(marker) if marker.ref_name == "refs/reviu/checkpoints/s/2")
  );

  // No marker before the prompt: a fresh one is inserted.
  let mut items = vec![user_message("first")];
  place_checkpoint_marker(&mut items, checkpoint_marker("refs/reviu/checkpoints/s/3"));
  assert_eq!(items.len(), 2);
  assert!(matches!(&items[0], ChatItem::Checkpoint(_)));
}

#[test]
fn deduped_model_entries_collapses_effort_variants() {
  let model = |id: &str, name: &str, description: &str| {
    let arc: std::sync::Arc<str> = std::sync::Arc::from(id);
    let mut info = ModelInfo::new(ModelId::new(arc), name.to_string());
    info.description = Some(description.to_string());
    info
  };
  let models = vec![
    model("sol-low", "GPT-5.6-Sol (low)", "Fast"),
    model("sol-high", "GPT-5.6-Sol (high)", "Deep"),
    model("terra-low", "GPT-5.6-Terra (low)", "Balanced"),
  ];
  let current: std::sync::Arc<str> = std::sync::Arc::from("sol-high");
  let current = ModelId::new(current);

  let entries = deduped_model_entries(&models, Some(&current));

  assert_eq!(entries.len(), 2);
  assert_eq!(entries[0].0, "GPT-5.6-Sol");
  // Click target is the first variant of the group (the model's default effort).
  assert_eq!(entries[0].1.0.as_ref(), "sol-low");
  // The group is marked current even though the current id is another variant.
  assert!(entries[0].3);
  assert_eq!(entries[1].0, "GPT-5.6-Terra");
  assert!(!entries[1].3);
}

#[test]
fn checkpoint_marker_survives_persistence_roundtrip() {
  let marker = CheckpointMarker {
    ref_name: "refs/reviu/checkpoints/s/1".to_string(),
    created_at_secs: 42,
  };
  let json =
    serde_json::to_string(&PersistedChatItem::Checkpoint(marker.clone())).expect("serialize");
  let restored: PersistedChatItem = serde_json::from_str(&json).expect("deserialize");
  match restored {
    PersistedChatItem::Checkpoint(restored) => assert_eq!(restored, marker),
    _ => panic!("expected checkpoint item"),
  }
}

fn edit_tool(id: &str, diffs: Vec<(&str, u32, u32)>) -> ChatItem {
  let arc: std::sync::Arc<str> = std::sync::Arc::from(id);
  ChatItem::Tool(ToolCallView {
    id: ToolCallId::new(arc),
    title: "Edit".to_string(),
    kind: ToolKind::Edit,
    status: ToolCallStatus::Completed,
    tool_name: None,
    locations: Vec::new(),
    diffs: diffs
      .into_iter()
      .map(|(path, added, removed)| DiffSummary {
        path: path.to_string(),
        added,
        removed,
        lines: Vec::new(),
        expanded: false,
      })
      .collect(),
    outputs: Vec::new(),
    terminals: Vec::new(),
    read_start_line: None,
    content_fp: 0,
  })
}

#[test]
fn turn_edit_stats_aggregates_the_last_turn_only() {
  let items = vec![
    checkpoint_marker("cp-1"),
    user_message("first"),
    edit_tool("t1", vec![("old.rs", 9, 9)]),
    ChatItem::TurnSummary(TurnSummaryView {
      files: vec![TurnFileStat {
        path: "old.rs".into(),
        added: 9,
        removed: 9,
      }],
      checkpoint_ref: Some("cp-1".into()),
      undone: false,
      duration_secs: None,
      expanded: false,
      work_expanded: false,
    }),
    checkpoint_marker("cp-2"),
    user_message("second"),
    edit_tool("t2", vec![("a.rs", 2, 1)]),
    edit_tool("t3", vec![("a.rs", 3, 0), ("b.rs", 1, 1)]),
  ];
  let (files, checkpoint_ref) = turn_edit_stats(&items);
  assert_eq!(checkpoint_ref, Some("cp-2".to_string()));
  assert_eq!(
    files,
    vec![
      TurnFileStat {
        path: "a.rs".into(),
        added: 5,
        removed: 1
      },
      TurnFileStat {
        path: "b.rs".into(),
        added: 1,
        removed: 1
      },
    ]
  );
}

#[test]
fn turn_edit_stats_is_empty_for_a_turn_without_edit_diffs() {
  let items = vec![
    checkpoint_marker("cp-1"),
    user_message("hi"),
    edit_tool("t1", vec![]),
    agent_message("done"),
  ];
  let (files, checkpoint_ref) = turn_edit_stats(&items);
  assert!(files.is_empty());
  assert_eq!(checkpoint_ref, Some("cp-1".to_string()));
}

fn summary_item(checkpoint_ref: &str) -> ChatItem {
  ChatItem::TurnSummary(TurnSummaryView {
    files: vec![TurnFileStat {
      path: "a.rs".into(),
      added: 1,
      removed: 0,
    }],
    checkpoint_ref: Some(checkpoint_ref.to_string()),
    undone: false,
    duration_secs: None,
    expanded: false,
    work_expanded: false,
  })
}

#[test]
fn is_trailing_turn_summary_only_for_the_latest_turn() {
  let items = vec![
    checkpoint_marker("cp-1"),
    user_message("first"),
    summary_item("cp-1"),
    checkpoint_marker("cp-2"),
    user_message("second"),
    summary_item("cp-2"),
    agent_message("late words"),
  ];
  assert!(!is_trailing_turn_summary(&items, 2), "an older turn's card");
  assert!(
    is_trailing_turn_summary(&items, 5),
    "trailing prose does not retire the latest card"
  );
}

#[gpui::test]
async fn undoing_a_turn_flags_only_the_matching_card(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.items = vec![
      checkpoint_marker("cp-1"),
      user_message("first"),
      summary_item("cp-1"),
      checkpoint_marker("cp-2"),
      user_message("second"),
      summary_item("cp-2"),
    ];
    panel.sync_list_count();
    panel.mark_turn_undone("cp-2", cx);
    let flags: Vec<bool> = panel
      .items
      .iter()
      .filter_map(|item| match item {
        ChatItem::TurnSummary(s) => Some(s.undone),
        _ => None,
      })
      .collect();
    assert_eq!(flags, vec![false, true]);
  });
}

#[gpui::test]
async fn only_the_latest_turn_summary_offers_undo_and_undone_retires_it(
  cx: &mut gpui::TestAppContext,
) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![
      checkpoint_marker("cp-1"),
      user_message("first"),
      summary_item("cp-1"),
      checkpoint_marker("cp-2"),
      user_message("second"),
      summary_item("cp-2"),
    ];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("turn-summary-undo").is_some(),
    "the latest card offers Undo"
  );

  // Retire the latest card: no card is trailing any more, Undo disappears
  // everywhere while Review stays on the standing one.
  panel.update(cx, |panel, cx| {
    panel.items.truncate(3);
    panel.items.push(checkpoint_marker("cp-2"));
    panel.items.push(user_message("second"));
    panel.items.push(agent_message("no edits this time"));
    panel.sync_list_count();
    let count = panel.messages_list.item_count();
    panel.messages_list.remeasure_items(0..count);
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("turn-summary-undo").is_none(),
    "an older turn's card cannot undo over later turns"
  );
  assert!(
    cx.debug_bounds("turn-summary-review").is_some(),
    "an older card keeps Review while its changes stand"
  );

  // A single undone card: state shown, both actions gone.
  panel.update(cx, |panel, cx| {
    panel.items = vec![
      checkpoint_marker("cp-1"),
      user_message("first"),
      summary_item("cp-1"),
    ];
    panel.sync_list_count();
    panel.mark_turn_undone("cp-1", cx);
    let count = panel.messages_list.item_count();
    panel.messages_list.remeasure_items(0..count);
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("turn-summary-undone").is_some(),
    "the undone card shows its state"
  );
  assert!(
    cx.debug_bounds("turn-summary-undo").is_none(),
    "an undone card cannot be undone again"
  );
  assert!(
    cx.debug_bounds("turn-summary-review").is_none(),
    "the undone card has nothing left to review"
  );
}

#[test]
fn auto_approve_picks_allow_once_then_allow_always_and_never_reject() {
  let opt = |id: &str, kind: PermissionOptionKind| PermissionPromptOption {
    option_id: id.into(),
    label: id.into(),
    kind,
  };
  let mixed = vec![
    opt("reject", PermissionOptionKind::RejectOnce),
    opt("always", PermissionOptionKind::AllowAlways),
    opt("once", PermissionOptionKind::AllowOnce),
  ];
  assert_eq!(auto_approve_option(&mixed), Some("once".to_string()));
  let always_only = vec![
    opt("reject", PermissionOptionKind::RejectOnce),
    opt("always", PermissionOptionKind::AllowAlways),
  ];
  assert_eq!(
    auto_approve_option(&always_only),
    Some("always".to_string())
  );
  let reject_only = vec![opt("reject", PermissionOptionKind::RejectAlways)];
  assert_eq!(auto_approve_option(&reject_only), None);
}

fn bench_diff_tool(id: &str, lines: usize) -> ChatItem {
  let arc: std::sync::Arc<str> = std::sync::Arc::from(id);
  let diff_lines: Vec<crate::diff::DiffLine> = (0..lines)
    .map(|i| crate::diff::DiffLine {
      kind: if i % 3 == 0 {
        DiffLineKind::Removed
      } else {
        DiffLineKind::Added
      },
      old_line: Some(i as u32 + 10),
      new_line: Some(i as u32 + 10),
      text: format!("let value_{i} = compute_something(input_{i}, {i});"),
      spans: Vec::new(),
      syntax_spans: Vec::new(),
    })
    .collect();
  ChatItem::Tool(ToolCallView {
    id: ToolCallId::new(arc),
    title: "Edit".to_string(),
    kind: ToolKind::Edit,
    status: ToolCallStatus::Completed,
    tool_name: None,
    locations: Vec::new(),
    diffs: vec![DiffSummary {
      path: "src/main.rs".to_string(),
      added: (lines * 2 / 3) as u32,
      removed: (lines / 3) as u32,
      lines: diff_lines,
      expanded: false,
    }],
    outputs: Vec::new(),
    terminals: Vec::new(),
    read_start_line: None,
    content_fp: 0,
  })
}

fn bench_output_tool(id: &str, lines: usize) -> ChatItem {
  let arc: std::sync::Arc<str> = std::sync::Arc::from(id);
  let text: String = (0..lines)
    .map(|i| format!("fn helper_{i}() -> usize {{ {i} * 42 }}\n"))
    .collect();
  ChatItem::Tool(ToolCallView {
    id: ToolCallId::new(arc),
    title: "Read".to_string(),
    kind: ToolKind::Read,
    status: ToolCallStatus::Completed,
    tool_name: None,
    locations: vec![(std::path::PathBuf::from("src/lib.rs"), Some(1))],
    diffs: Vec::new(),
    outputs: vec![ToolOutput {
      text,
      start_line: Some(1),
      expanded: false,
      syntax_spans: Vec::new(),
    }],
    terminals: Vec::new(),
    read_start_line: None,
    content_fp: 0,
  })
}

fn bench_markdown_message(i: usize) -> ChatItem {
  let text = format!(
    "Step {i}: here is what changed and why it matters for the build.\n\n\
```rust\nfn example_{i}(input: usize) -> usize {{\n  let doubled = input * 2;\n  doubled + {i}\n}}\n```\n\n\
- point one about the change\n- point two with `inline_code`\n"
  );
  agent_message(&text)
}

/// Manual probe: `cargo test -p agent_chat_panel --lib draw_cost_probe -- --ignored --nocapture`
#[gpui::test]
#[ignore = "draw-cost probe, run manually with --nocapture"]
async fn draw_cost_probe(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  let scenarios: Vec<(&str, Vec<ChatItem>)> = vec![
    (
      "plain-messages-x40",
      (0..40)
        .map(|i| {
          if i % 2 == 0 {
            user_message(&format!("question {i} about the code"))
          } else {
            agent_message(&format!("short answer {i}"))
          }
        })
        .collect(),
    ),
    (
      "diff-tools-x8-40lines",
      (0..8)
        .map(|i| bench_diff_tool(&format!("d{i}"), 40))
        .collect(),
    ),
    (
      "output-tools-x8-20lines",
      (0..8)
        .map(|i| bench_output_tool(&format!("o{i}"), 20))
        .collect(),
    ),
    (
      "markdown-messages-x20",
      (0..20).map(bench_markdown_message).collect(),
    ),
  ];

  let viewport = cx.update(|window, _| window.viewport_size());
  eprintln!("[draw-cost] viewport {viewport:?}");

  for (label, items) in scenarios {
    panel.update(cx, |panel, cx| {
      panel.status = Status::Ready;
      panel.items = items;
      panel.rebuild_tool_index();
      panel.sync_list_count();
      cx.notify();
    });
    cx.run_until_parked();
    let painted = cx.debug_bounds("agent-tool-card").is_some()
      || cx.debug_bounds("agent-chat-composer").is_some();

    // In test mode the draw runs synchronously inside flush_effects at the
    // end of update(), so the update call itself is what gets timed.
    let mut warm = std::time::Duration::from_secs(999);
    for _ in 0..10 {
      let t = std::time::Instant::now();
      panel.update(cx, |_, cx| cx.notify());
      warm = warm.min(t.elapsed());
    }
    let mut remeasure = std::time::Duration::from_secs(999);
    for _ in 0..10 {
      let t = std::time::Instant::now();
      panel.update(cx, |panel, cx| {
        let count = panel.messages_list.item_count();
        panel.messages_list.remeasure_items(0..count);
        cx.notify();
      });
      remeasure = remeasure.min(t.elapsed());
    }
    eprintln!("[draw-cost] {label}: painted={painted} warm {warm:?}, remeasure {remeasure:?}");
  }
}

#[test]
fn fold_adjacent_chunks_merges_same_kind_text_and_keeps_boundaries() {
  let folded = events::fold_adjacent_chunks(vec![
    text_chunk("Hello, "),
    text_chunk("world"),
    thought_chunk("hmm "),
    thought_chunk("ok"),
    text_chunk("!"),
  ]);
  let texts: Vec<(&'static str, String)> = folded
    .iter()
    .map(|ev| match ev {
      AgentEvent::AgentMessageChunk(c) => ("msg", chunk_text(c)),
      AgentEvent::AgentThoughtChunk(c) => ("thought", chunk_text(c)),
      _ => panic!("unexpected event kind"),
    })
    .collect();
  assert_eq!(
    texts,
    vec![
      ("msg", "Hello, world".to_string()),
      ("thought", "hmm ok".to_string()),
      ("msg", "!".to_string()),
    ],
    "same-kind neighbours fold, kind changes stay boundaries"
  );
}

#[test]
fn fold_adjacent_chunks_keeps_non_chunk_events_in_place() {
  let folded = events::fold_adjacent_chunks(vec![
    text_chunk("before "),
    AgentEvent::ToolCall(call("t1", "Read file", ToolKind::Read)),
    text_chunk("after"),
  ]);
  assert_eq!(folded.len(), 3, "a tool call is an ordering boundary");
}

fn chunk_text(chunk: &agent_client_protocol::schema::ContentChunk) -> String {
  match &chunk.content {
    ContentBlock::Text(t) => t.text.clone(),
    _ => panic!("expected text content"),
  }
}

#[gpui::test]
async fn a_burst_of_chunks_lands_as_one_update(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  let (tx, rx) = async_channel::unbounded();
  panel.update(cx, |panel, cx| {
    panel.start_event_forwarder(rx, cx);
  });

  let notifies = std::rc::Rc::new(std::cell::Cell::new(0usize));
  cx.update(|_, cx| {
    let notifies = notifies.clone();
    cx.observe(&panel, move |_, _| notifies.set(notifies.get() + 1))
      .detach();
  });

  for part in ["a", "b", "c", "d", "e"] {
    tx.send_blocking(text_chunk(part)).expect("channel open");
  }
  cx.run_until_parked();

  assert_eq!(
    notifies.get(),
    1,
    "the whole burst commits as one update + notify"
  );
  panel.read_with(cx, |panel, _| {
    assert_eq!(panel.pending_agent, "abcde", "no chunk is lost by folding");
  });

  // A second burst holds the commit floor, then lands as one more commit.
  for part in ["f", "g"] {
    tx.send_blocking(text_chunk(part)).expect("channel open");
  }
  cx.executor()
    .advance_clock(std::time::Duration::from_millis(
      events::STREAM_COMMIT_MS + 10,
    ));
  cx.run_until_parked();
  assert_eq!(
    notifies.get(),
    2,
    "the floor coalesces the second burst too"
  );
  panel.read_with(cx, |panel, _| {
    assert_eq!(panel.pending_agent, "abcdefg");
  });
}

#[gpui::test]
async fn a_scheduled_persist_waits_for_the_throttle_window(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-persist-throttle");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = vec![user_message("hi")];
    panel.schedule_persist(cx);
    panel.schedule_persist(cx);
  });
  let conv_id = panel.read_with(cx, |panel, _| panel.current_conv.id.clone());
  let path = dir.join(format!("{conv_id}.json"));
  assert!(
    !path.exists(),
    "a streamed change must not hit the disk immediately"
  );
  cx.executor()
    .advance_clock(std::time::Duration::from_millis(500));
  cx.run_until_parked();
  assert!(
    path.exists(),
    "the write lands once the throttle window closes"
  );
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn a_direct_persist_supersedes_the_armed_throttle(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-persist-direct");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = vec![user_message("hi")];
    panel.schedule_persist(cx);
    panel.persist_state(cx);
  });
  cx.run_until_parked();
  let conv_id = panel.read_with(cx, |panel, _| panel.current_conv.id.clone());
  assert!(
    dir.join(format!("{conv_id}.json")).exists(),
    "the direct write lands without waiting for the throttle window"
  );
  panel.read_with(cx, |panel, cx| {
    let pending = panel
      .store
      .as_ref()
      .is_some_and(|store| store.read(cx).has_pending_save());
    assert!(!pending, "nothing stays queued after the direct write");
  });
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn a_legacy_file_without_version_loads_and_rewrites_versioned(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-store-version");
  std::fs::create_dir_all(&dir).unwrap();
  let legacy = r#"{"meta":{"id":"old-conv","started_at_secs":1,"updated_at_secs":2,"title":"old","message_count":1},"items":[{"type":"Message","role":"User","text":"hi","images":0}]}"#;
  std::fs::write(dir.join("old-conv.json"), legacy).unwrap();
  std::fs::write(dir.join("active.txt"), "old-conv").unwrap();

  let (meta, items, ..) =
    load_conversation_file(&dir.join("old-conv.json")).expect("legacy file loads");
  assert_eq!(meta.id, "old-conv");
  assert_eq!(meta.agent_id, default_agent_id());
  assert_eq!(items.len(), 1);

  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.current_conv = meta;
    panel.items = items;
    panel.persist_state(cx);
  });
  cx.run_until_parked();
  let raw = std::fs::read_to_string(dir.join("old-conv.json")).unwrap();
  assert!(
    raw.contains("\"version\":1"),
    "the rewrite carries the format version"
  );
  assert!(
    raw.contains("\"agent_id\":\"claude-acp\""),
    "the rewrite records the conversation agent"
  );
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn the_listing_comes_from_the_index_without_reading_transcripts(
  cx: &mut gpui::TestAppContext,
) {
  let dir = temp_dir("agent-store-index");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = vec![user_message("question"), agent_message("answer")];
    panel.persist_state(cx);
  });
  cx.run_until_parked();
  let conv_id = panel.read_with(cx, |panel, _| panel.current_conv.id.clone());

  // Corrupt the transcript: a listing that parsed it would lose the row.
  std::fs::write(dir.join(format!("{conv_id}.json")), "not json").unwrap();
  let listed = crate::store::ConversationStore::new(dir.clone()).list();
  assert_eq!(listed.len(), 1, "the index alone serves the listing");
  assert_eq!(listed[0].id, conv_id);
  assert_eq!(
    listed[0].preview, "answer",
    "the index row carries the last-message preview"
  );

  // Without the index the store falls back to scanning the files.
  std::fs::remove_file(crate::persistence::index_path(&dir)).unwrap();
  let rebuilt = crate::store::ConversationStore::new(dir.clone()).list();
  assert!(
    rebuilt.is_empty(),
    "the corrupted transcript cannot be scanned, so the rebuild is empty"
  );
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn coalesced_saves_land_the_last_snapshot(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-store-lastwins");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = vec![user_message("first wording")];
    panel.persist_state(cx);
    panel.items = vec![user_message("second wording")];
    panel.persist_state(cx);
  });
  cx.run_until_parked();
  let conv_id = panel.read_with(cx, |panel, _| panel.current_conv.id.clone());
  let (_, items, ..) =
    load_conversation_file(&dir.join(format!("{conv_id}.json"))).expect("reloads");
  let ChatItem::Message(m) = &items[0] else {
    panic!("message expected");
  };
  assert_eq!(m.text, "second wording", "the last snapshot wins");
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn a_resumed_panel_hydrates_in_the_background(cx: &mut gpui::TestAppContext) {
  set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
  let dir = temp_dir("agent-store-resume");
  let (panel, cx) = add_panel_window(cx);
  let first_meta = panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = vec![user_message("first conversation")];
    panel.persist_state(cx);
    panel.current_conv.clone()
  });
  cx.run_until_parked();
  let store = panel.read_with(cx, |panel, _| panel.store.clone().expect("store"));

  let resumed = resumed_panel(&store, Some(first_meta.clone()), cx);
  resumed.read_with(cx, |panel, _| {
    assert_eq!(
      panel.loading_conversation_id(),
      Some(first_meta.id.as_str()),
      "the conversation is marked as hydrating until its transcript lands"
    );
    assert!(panel.items.is_empty());
  });
  cx.run_until_parked();
  resumed.read_with(cx, |panel, _| {
    assert_eq!(panel.loading_conversation_id(), None);
    assert_eq!(panel.current_conv.id, first_meta.id);
    let ChatItem::Message(m) = &panel.items[0] else {
      panic!("message expected");
    };
    assert_eq!(m.text, "first conversation", "the transcript hydrated");
  });
  set_backend_command_override(None);
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn composer_drafts_follow_their_conversation(cx: &mut gpui::TestAppContext) {
  set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
  let dir = temp_dir("agent-drafts");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = vec![user_message("hello")];
    panel.persist_state(cx);
  });
  cx.run_until_parked();
  let first_meta = panel.read_with(cx, |panel, _| panel.current_conv.clone());
  let first_id = first_meta.id.clone();
  let store = panel.read_with(cx, |panel, _| panel.store.clone().expect("store"));

  cx.update(|window, cx| {
    panel.update(cx, |panel, cx| {
      let input = panel.input.clone();
      input.update(cx, |state, cx| {
        state.set_value("half-typed message", window, cx)
      });
      // Real typing emits InputEvent::Change; set_value is silent in tests.
      panel.schedule_draft_save(cx);
    });
  });
  cx.run_until_parked();

  // A fresh session starts with an empty composer; the draft stays behind.
  let fresh = resumed_panel(&store, None, cx);
  cx.run_until_parked();
  fresh.read_with(cx, |panel, cx| {
    assert_eq!(
      panel.input.read(cx).value().as_ref(),
      "",
      "a fresh conversation must not inherit the draft"
    );
  });

  // A panel resuming the first conversation gets the draft back.
  let resumed = resumed_panel(&store, Some(first_meta), cx);
  cx.run_until_parked();
  resumed.read_with(cx, |panel, cx| {
    assert_eq!(
      panel.input.read(cx).value().as_ref(),
      "half-typed message",
      "the draft follows its conversation"
    );
  });

  // Clearing the input (what sending does) drops the stored draft.
  cx.update(|window, cx| {
    resumed.update(cx, |panel, cx| {
      let input = panel.input.clone();
      input.update(cx, |state, cx| state.set_value("", window, cx));
      panel.schedule_draft_save(cx);
    });
  });
  cx.run_until_parked();
  store.update(cx, |store, _| store.flush_on_quit());
  let relaunched = crate::store::ConversationStore::new(dir.clone());
  assert_eq!(
    relaunched.draft(&first_id),
    None,
    "a sent draft does not come back after a relaunch"
  );
  set_backend_command_override(None);
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn the_reading_position_follows_its_conversation(cx: &mut gpui::TestAppContext) {
  set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
  let dir = temp_dir("agent-scroll");
  let (panel, cx) = add_panel_window(cx);
  let first_meta = panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = (0..60)
      .map(|i| user_message(&format!("message {i}")))
      .collect();
    panel.sync_list_count();
    panel.persist_state(cx);
    panel.current_conv.clone()
  });
  cx.run_until_parked();
  let store = panel.read_with(cx, |panel, _| panel.store.clone().expect("store"));

  // The reader scrolls away from the tail; render-time capture persists it.
  panel.update(cx, |panel, cx| {
    panel.messages_list.scroll_to(gpui::ListOffset {
      item_ix: 12,
      offset_in_item: px(4.),
    });
    panel.save_scroll_position(cx);
  });
  cx.run_until_parked();

  let fresh = resumed_panel(&store, None, cx);
  cx.run_until_parked();
  fresh.read_with(cx, |panel, _| {
    assert!(
      panel.messages_list.is_following_tail(),
      "a fresh conversation follows the tail"
    );
  });

  let resumed = resumed_panel(&store, Some(first_meta), cx);
  cx.run_until_parked();
  resumed.read_with(cx, |panel, _| {
    assert!(
      !panel.messages_list.is_following_tail(),
      "a stored offset pauses tail-following"
    );
    assert_eq!(
      panel.messages_list.logical_scroll_top().item_ix,
      12,
      "the reading position came back"
    );
  });
  set_backend_command_override(None);
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn the_reading_position_survives_the_generating_row_settling(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, _| {
    panel.items = (0..60).map(|i| user_message(&format!("m {i}"))).collect();
    // Connecting shows the trailing Generating row.
    panel.status = Status::Connecting;
    panel.sync_list_count();
    panel.messages_list.scroll_to(gpui::ListOffset {
      item_ix: 12,
      offset_in_item: px(4.),
    });
    // The row settles once the session is ready: one fewer list item.
    panel.status = Status::Ready;
    panel.sync_list_count();
    assert_eq!(
      panel.messages_list.logical_scroll_top().item_ix,
      12,
      "a tail-side shrink must not reset the reading position"
    );
  });
}

#[gpui::test]
async fn a_draft_survives_a_relaunch(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-drafts-relaunch");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = vec![user_message("hello")];
    panel.persist_state(cx);
  });
  cx.run_until_parked();
  let conv_id = panel.read_with(cx, |panel, _| panel.current_conv.id.clone());
  cx.update(|window, cx| {
    panel.update(cx, |panel, cx| {
      let input = panel.input.clone();
      input.update(cx, |state, cx| state.set_value("survives quit", window, cx));
      panel.schedule_draft_save(cx);
    });
  });
  // No parked run: the debounce is still pending, quit must flush it.
  panel.update(cx, |panel, cx| {
    if let Some(store) = panel.store.clone() {
      store.update(cx, |store, _| store.flush_on_quit());
    }
  });
  let relaunched = crate::store::ConversationStore::new(dir.clone());
  assert_eq!(
    relaunched.draft(&conv_id).as_deref(),
    Some("survives quit"),
    "the quit flush lands the pending draft"
  );
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn deleting_a_conversation_scrubs_its_traces(cx: &mut gpui::TestAppContext) {
  set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
  let dir = temp_dir("agent-delete");
  let (panel, cx) = add_panel_window(cx);
  let conv_id = panel.update(cx, |panel, cx| {
    let store = cx.new(|_| crate::store::ConversationStore::new(dir.clone()));
    let id = panel.current_conv.id.clone();
    store.update(cx, |store, cx| {
      store.set_draft(&id, "unsent words", cx);
      store.set_scroll(&id, Some((7, 2.0)), cx);
    });
    panel.store = Some(store);
    // The shown panel writes the active pointer with each save.
    panel.set_active_conversation(true);
    panel.items = vec![user_message("to be deleted")];
    panel.persist_state(cx);
    id
  });
  cx.run_until_parked();
  let path = dir.join(format!("{conv_id}.json"));
  assert!(path.exists());
  assert!(dir.join("active.txt").exists());
  let store = panel.read_with(cx, |panel, _| panel.store.clone().expect("store"));

  // A throttled save is still queued when the delete lands: it must not
  // resurrect the file.
  panel.update(cx, |panel, cx| {
    panel.items.push(agent_message("late change"));
    panel.schedule_persist(cx);
  });
  store.update(cx, |store, cx| {
    store.delete(&conv_id, cx);
    store.set_active(None, cx);
  });
  cx.executor()
    .advance_clock(std::time::Duration::from_millis(600));
  cx.run_until_parked();

  assert!(!path.exists(), "the transcript file is gone and stays gone");
  assert!(
    !dir.join("active.txt").exists(),
    "deleting the active conversation clears the active pointer"
  );
  let relaunched = crate::store::ConversationStore::new(dir.clone());
  assert!(relaunched.list().is_empty(), "the index dropped the row");
  assert_eq!(relaunched.draft(&conv_id), None, "the draft was scrubbed");
  assert_eq!(relaunched.scroll(&conv_id), None, "the scroll was scrubbed");
  set_backend_command_override(None);
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn a_pending_transcript_save_lands_on_quit(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-quit-flush");
  let (panel, cx) = add_panel_window(cx);
  let conv_id = panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = vec![user_message("about to quit")];
    panel.schedule_persist(cx);
    panel.current_conv.id.clone()
  });
  // No parked run: the throttle window is still open at quit time.
  let path = dir.join(format!("{conv_id}.json"));
  assert!(!path.exists(), "the throttled write has not landed yet");
  panel.update(cx, |panel, cx| {
    if let Some(store) = panel.store.clone() {
      store.update(cx, |store, _| store.flush_on_quit());
    }
  });
  let (_, items, ..) = load_conversation_file(&path).expect("the quit flush wrote the transcript");
  assert_eq!(items.len(), 1);
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn a_background_panel_never_writes_the_active_pointer(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-active-background");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = vec![user_message("streaming in the background")];
    panel.persist_state(cx);
  });
  cx.run_until_parked();
  assert!(
    !dir.join("active.txt").exists(),
    "a save from a background panel must not steal the active pointer"
  );

  panel.update(cx, |panel, cx| {
    panel.set_active_conversation(true);
    panel.items.push(agent_message("now shown"));
    panel.persist_state(cx);
  });
  cx.run_until_parked();
  let active = std::fs::read_to_string(dir.join("active.txt")).expect("active pointer exists");
  let conv_id = panel.read_with(cx, |panel, _| panel.current_conv.id.clone());
  assert_eq!(active.trim(), conv_id, "the shown panel writes the pointer");
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn the_turn_gate_refuses_a_second_concurrent_turn(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  let gate = panel.read_with(cx, |panel, _| panel.turn_gate.clone());
  gate.acquire(Path::new("."), "some-other-conversation");

  panel.update(cx, |panel, cx| {
    let dispatched = panel.dispatch_prompt("hello".to_string(), cx);
    assert!(!dispatched, "the gate blocks a turn while another runs");
    let ChatItem::Message(m) = panel.items.last().expect("a system message") else {
      panic!("message expected");
    };
    assert!(matches!(m.role, ChatRole::System));
    assert!(m.text.contains("Another session is running"));
  });

  gate.release(Path::new("."), "some-other-conversation");
  panel.update(cx, |panel, cx| {
    // Free gate: the dispatch now fails on the missing session, not the gate.
    let items_before = panel.items.len();
    let dispatched = panel.dispatch_prompt("hello".to_string(), cx);
    assert!(!dispatched, "no session is connected in this fixture");
    assert_eq!(
      panel.items.len(),
      items_before,
      "a free gate adds no refusal message"
    );
  });

  // A turn running in ANOTHER checkout never blocks this panel's dispatch:
  // the refusal path is not taken, only the missing session stops it.
  gate.acquire(Path::new("/some/other/worktree"), "worktree-conversation");
  panel.update(cx, |panel, cx| {
    let items_before = panel.items.len();
    let dispatched = panel.dispatch_prompt("hello".to_string(), cx);
    assert!(!dispatched, "no session is connected in this fixture");
    assert_eq!(
      panel.items.len(),
      items_before,
      "a busy foreign checkout adds no refusal message"
    );
  });
}

#[gpui::test]
async fn awaiting_permission_tracks_the_last_unresolved_card(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    let permission = |resolved: Option<String>| {
      let update = update_fields(
        ToolCallUpdateFields::new()
          .kind(ToolKind::Execute)
          .raw_input(serde_json::json!({ "command": "cargo build" })),
      );
      let detail = permission_detail(&update, std::path::Path::new("."));
      ChatItem::Permission(Box::new(PermissionItem {
        prompt: PermissionPrompt {
          id: 7,
          tool_call_title: "Run cargo build".into(),
          tool_call: update,
          options: Vec::new(),
        },
        detail,
        resolved,
        auto: false,
      }))
    };

    panel.items = vec![user_message("do it"), permission(None)];
    assert!(
      !panel.awaiting_permission(),
      "an unresolved card without a running turn is a stale reload, not a wait"
    );

    panel.pretend_turn_in_flight_for_test(cx);
    assert!(panel.awaiting_permission(), "the turn waits on the card");

    panel.items = vec![user_message("do it"), permission(Some("allow".into()))];
    assert!(
      !panel.awaiting_permission(),
      "an answered card is a working turn again"
    );
  });
}

#[test]
fn wrapped_row_estimates_track_newlines_and_long_lines() {
  assert_eq!(estimate_wrapped_rows(""), 1);
  assert_eq!(estimate_wrapped_rows("short"), 1);
  assert_eq!(estimate_wrapped_rows("a\nb\nc"), 3);
  assert_eq!(estimate_wrapped_rows(&"x".repeat(80)), 2);
  assert_eq!(
    estimate_wrapped_rows(&format!("{}\n{}", "x".repeat(80), "y")),
    3
  );
}

#[test]
fn agent_error_hints_name_the_classes_users_hit() {
  assert_eq!(
    agent_error_hint("Codex error: The usage limit has been reached"),
    Some("The provider refused this turn: usage limit or credits exhausted.")
  );
  assert_eq!(
    agent_error_hint("insufficient_quota: you have run out of credits"),
    Some("The provider refused this turn: usage limit or credits exhausted.")
  );
  assert_eq!(
    agent_error_hint("HTTP 429 Too Many Requests"),
    Some("Rate limited by the provider. Wait a moment and retry.")
  );
  assert_eq!(
    agent_error_hint("connection reset by peer"),
    Some("The provider looks unreachable. Check your connection and retry.")
  );
  assert_eq!(agent_error_hint("something exotic went wrong"), None);
}

#[gpui::test]
async fn a_failed_turn_is_loud_everywhere(cx: &mut gpui::TestAppContext) {
  use std::cell::RefCell;
  use std::rc::Rc;

  let (panel, cx) = add_panel_window(cx);
  let failures: Rc<RefCell<Vec<String>>> = Rc::default();
  cx.update(|_, cx| {
    let failures = failures.clone();
    cx.subscribe(&panel, move |_, event: &AgentChatPanelEvent, _| {
      if let AgentChatPanelEvent::TurnFailed { message } = event {
        failures.borrow_mut().push(message.clone());
      }
    })
    .detach();
  });

  panel.update(cx, |panel, cx| {
    panel.pretend_turn_in_flight_for_test(cx);
    panel.complete_prompt(
      Err(anyhow::anyhow!(
        "Codex error: The usage limit has been reached"
      )),
      cx,
    );
  });
  cx.run_until_parked();

  panel.read_with(cx, |panel, _| {
    assert!(panel.last_turn_failed(), "the sidebar can show Failed");
    let ChatItem::Message(m) = panel
      .items
      .iter()
      .rev()
      .find(|item| matches!(item, ChatItem::Message(m) if m.text.starts_with("[error]")))
      .expect("an error line landed in the transcript")
    else {
      panic!("message expected");
    };
    assert!(
      m.text.contains("usage limit or credits exhausted"),
      "the error names its class: {}",
      m.text
    );
  });
  assert_eq!(
    failures.borrow().as_slice(),
    ["The provider refused this turn: usage limit or credits exhausted.".to_string()],
    "the host was told, so it can toast"
  );

  // The next attempt clears the flag; and a turn that answered stays quiet.
  panel.update(cx, |panel, cx| {
    panel.items.push(user_message("again"));
    panel.items.push(agent_message("a real reply"));
    panel.pretend_turn_in_flight_for_test(cx);
    assert!(!panel.last_turn_failed());
    panel.complete_prompt(Ok(agent_client_protocol::schema::StopReason::EndTurn), cx);
  });
  panel.read_with(cx, |panel, _| assert!(!panel.last_turn_failed()));
}

#[gpui::test]
async fn an_empty_turn_is_reported_even_when_the_adapter_stays_silent(
  cx: &mut gpui::TestAppContext,
) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    // The pi shape: the prompt "succeeds", the turn holds nothing at all.
    panel.items.push(user_message("t la ?"));
    panel.pretend_turn_in_flight_for_test(cx);
    panel.complete_prompt(Ok(agent_client_protocol::schema::StopReason::EndTurn), cx);
  });
  panel.read_with(cx, |panel, _| {
    assert!(panel.last_turn_failed());
    let ChatItem::Message(m) = panel.items.last().expect("an error line") else {
      panic!("message expected");
    };
    assert!(m.text.contains("without a reply"), "{}", m.text);
  });
}

#[gpui::test]
async fn an_error_already_streamed_as_a_bubble_is_not_shown_twice(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    // The codex shape: the same text arrives as an agent bubble AND as the
    // prompt error.
    panel.items.push(user_message("t la ?"));
    panel.items.push(agent_message(
      "You've hit your usage limit. Upgrade to Pro.",
    ));
    panel.pretend_turn_in_flight_for_test(cx);
    panel.complete_prompt(
      Err(anyhow::anyhow!(
        "You've hit your usage limit. Upgrade to Pro."
      )),
      cx,
    );
  });
  panel.read_with(cx, |panel, _| {
    assert!(panel.last_turn_failed(), "the failure still registers");
    let error_lines = panel
      .items
      .iter()
      .filter(|item| matches!(item, ChatItem::Message(m) if m.text.starts_with("[error]")))
      .count();
    assert_eq!(error_lines, 0, "the bubble already says it once");
  });
}

#[gpui::test]
async fn the_title_settles_exactly_once(cx: &mut gpui::TestAppContext) {
  use std::cell::RefCell;
  use std::rc::Rc;

  let dir = temp_dir("agent-title-settled");
  let (panel, cx) = add_panel_window(cx);
  let titles: Rc<RefCell<Vec<String>>> = Rc::default();
  cx.update(|_, cx| {
    let titles = titles.clone();
    cx.subscribe(&panel, move |_, event: &AgentChatPanelEvent, _| {
      if let AgentChatPanelEvent::TitleSettled { title } = event {
        titles.borrow_mut().push(title.clone());
      }
    })
    .detach();
  });

  panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    // No user message yet: nothing to announce.
    panel.items = vec![agent_message("hello")];
    panel.persist_state(cx);
  });
  cx.run_until_parked();
  assert!(titles.borrow().is_empty(), "no title, no event");

  panel.update(cx, |panel, cx| {
    panel.items.push(user_message("Fix the scroll jump"));
    panel.persist_state(cx);
    // Streaming keeps persisting; the announcement must not repeat.
    panel.items.push(agent_message("on it"));
    panel.persist_state(cx);
  });
  cx.run_until_parked();
  assert_eq!(
    titles.borrow().as_slice(),
    ["Fix the scroll jump".to_string()],
    "one title, one event"
  );
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_turn_gate_is_scoped_to_the_checkout() {
  let gate = TurnGate::default();
  let main = std::path::Path::new("/repo");
  let worktree = std::path::Path::new("/repo-worktrees/calm-river");

  gate.acquire(main, "conversation-a");
  assert!(
    gate.can_start(worktree, "conversation-b"),
    "a session in its own worktree runs in parallel"
  );
  assert!(
    !gate.can_start(main, "conversation-b"),
    "two sessions sharing the main checkout still serialize"
  );
  assert!(
    gate.can_start(main, "conversation-a"),
    "the holder re-enters"
  );

  gate.acquire(worktree, "conversation-b");
  gate.release(main, "conversation-a");
  assert!(gate.can_start(main, "conversation-c"));
  assert!(
    !gate.can_start(worktree, "conversation-c"),
    "releasing one checkout leaves the other held"
  );
}

#[gpui::test]
async fn worktree_bindings_roundtrip_and_die_with_their_conversation(
  cx: &mut gpui::TestAppContext,
) {
  let dir = temp_dir("agent-worktree-bindings");
  let store = cx.new(|_| crate::store::ConversationStore::new(dir.clone()));
  let binding = crate::persistence::WorktreeBinding {
    path: PathBuf::from("/somewhere/repo-worktrees/calm-river"),
    branch: "reviu-calm-river".to_string(),
  };
  store.update(cx, |store, cx| {
    store.set_worktree("conv-1", Some(binding.clone()), cx);
  });
  cx.run_until_parked();

  // A relaunch reads the binding back.
  let relaunched = crate::store::ConversationStore::new(dir.clone());
  assert_eq!(relaunched.worktree("conv-1"), Some(binding));

  // Deleting the conversation scrubs it.
  store.update(cx, |store, cx| store.delete("conv-1", cx));
  cx.run_until_parked();
  let relaunched = crate::store::ConversationStore::new(dir.clone());
  assert_eq!(relaunched.worktree("conv-1"), None);
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn a_turn_completing_in_the_background_persists_and_frees_the_gate(
  cx: &mut gpui::TestAppContext,
) {
  let dir = temp_dir("agent-background-turn");
  let (panel, cx) = add_panel_window(cx);
  let gate = panel.read_with(cx, |panel, _| panel.turn_gate.clone());
  let conv_id = panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    // Parked in the background: not the shown panel.
    panel.set_active_conversation(false);
    panel.items = vec![user_message("work on this")];
    panel.pretend_turn_in_flight_for_test(cx);
    panel.current_conv.id.clone()
  });
  assert!(
    !gate.can_start(Path::new("."), "another-conversation"),
    "the turn holds the gate"
  );

  panel.update(cx, |panel, cx| {
    panel.complete_prompt(Ok(agent_client_protocol::schema::StopReason::EndTurn), cx);
  });
  cx.run_until_parked();

  panel.read_with(cx, |panel, _| assert!(!panel.is_turn_in_flight()));
  assert!(
    gate.can_start(Path::new("."), "another-conversation"),
    "a settled background turn releases the gate for other sessions"
  );
  assert!(
    dir.join(format!("{conv_id}.json")).exists(),
    "the background transcript landed on disk"
  );
  assert!(
    !dir.join("active.txt").exists(),
    "a background completion never touches the active pointer"
  );
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn dropping_a_panel_mid_turn_frees_the_shared_gate(cx: &mut gpui::TestAppContext) {
  set_backend_command_override(Some("/nonexistent-agent-binary".to_string()));
  let (_host, cx) = add_panel_window(cx);
  let gate = TurnGate::default();
  let panel = cx.update(|window, cx| {
    cx.new(|cx| {
      AgentChatPanel::new(
        default_agent_id(),
        PathBuf::from("."),
        PathBuf::from("."),
        None,
        None,
        gate.clone(),
        window,
        cx,
      )
    })
  });
  panel.update(cx, |panel, cx| panel.pretend_turn_in_flight_for_test(cx));
  assert!(
    !gate.can_start(Path::new("."), "other"),
    "the turn holds the gate"
  );

  // Deleting a running session drops its panel; the gate must not stay held.
  drop(panel);
  // Entity release happens on the next effect flush, not at handle drop.
  cx.update(|_, _| {});
  cx.run_until_parked();
  assert!(
    gate.can_start(Path::new("."), "other"),
    "the drop released the gate"
  );
  set_backend_command_override(None);
}

#[gpui::test]
async fn a_submit_blocked_by_the_gate_keeps_the_composer_text(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  let gate = panel.read_with(cx, |panel, _| panel.turn_gate.clone());
  gate.acquire(Path::new("."), "the-busy-conversation");

  cx.update(|window, cx| {
    panel.update(cx, |panel, cx| {
      let input = panel.input.clone();
      input.update(cx, |state, cx| state.set_value("my prompt", window, cx));
      panel.submit(window, cx);
    });
  });

  panel.read_with(cx, |panel, cx| {
    assert_eq!(
      panel.input.read(cx).value().as_ref(),
      "my prompt",
      "a blocked submit must not swallow what was typed"
    );
    let ChatItem::Message(m) = panel.items.last().expect("a system message") else {
      panic!("message expected");
    };
    assert!(m.text.contains("Another session is running"));
  });
}

#[gpui::test]
async fn a_refused_queue_drain_keeps_the_message_queued(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.queue_prompt_for_test("typed mid-turn", cx);
    panel.pretend_turn_in_flight_for_test(cx);
    // The session died during the turn: the drain's dispatch is refused.
    panel.complete_prompt(Ok(agent_client_protocol::schema::StopReason::EndTurn), cx);
  });
  panel.read_with(cx, |panel, _| {
    assert_eq!(
      panel.queued_prompt_texts(),
      vec!["typed mid-turn".to_string()],
      "a refused dispatch puts the message back instead of losing it"
    );
  });
}

#[gpui::test]
async fn code_lines_wraps_long_rows_and_keeps_short_ones_single(cx: &mut gpui::TestAppContext) {
  let (_panel, cx) = add_panel_window(cx);
  let mono = Font {
    family: "monospace".into(),
    ..Default::default()
  };
  let muted = gpui::hsla(0., 0., 0.5, 1.);
  let band = gpui::hsla(0., 0., 0., 0.);
  let rows = vec![
    crate::code_lines::CodeLineRow {
      gutter: Some("   1    1".into()),
      text: "short".into(),
      runs: Vec::new(),
      band,
    },
    crate::code_lines::CodeLineRow {
      gutter: Some("   2    2".into()),
      text: "a very long line of code that cannot possibly fit in two hundred pixels of width and must wrap"
        .into(),
      runs: Vec::new(),
      band,
    },
  ];
  let element = crate::code_lines::CodeLines::new(rows, px(70.), muted, muted, gpui::black(), mono);
  let probe = element.probe();
  cx.draw(
    gpui::point(px(0.), px(0.)),
    gpui::size(px(300.), px(600.)),
    |_, _| element,
  );
  let heights = probe.row_heights();
  assert_eq!(heights.len(), 2, "both rows were laid out");
  assert!(
    heights[1] > heights[0] * 1.5,
    "the long row wraps to more visual lines (short {} vs long {})",
    heights[0],
    heights[1]
  );
}

fn tool_call_view_from_fixture(call: ToolCall) -> ToolCallView {
  let mut items = Vec::new();
  let mut index = HashMap::new();
  upsert_tool_call_pure(&mut items, &mut index, call, test_cwd());
  let Some(ChatItem::Tool(view)) = items.pop() else {
    panic!("tool expected");
  };
  view
}

fn text_tool_content(text: &str) -> ToolCallContent {
  ToolCallContent::from(ContentBlock::Text(TextContent::new(text)))
}

#[test]
fn acp_registry_read_fixtures_normalize_to_reviu_numbered_outputs() {
  let mut pi_read = call("pi-read", "Read README.md", ToolKind::Read);
  pi_read.locations = vec![ToolCallLocation::new("README.md").line(3_u32)];
  pi_read.content = vec![text_tool_content("# Reviu\n\nKeyboard-first")];

  let mut claude_numbered_read = call("claude-numbered", "Read src/lib.rs", ToolKind::Read);
  claude_numbered_read.locations = vec![ToolCallLocation::new("src/lib.rs")];
  claude_numbered_read.content = vec![text_tool_content("   845\tlet a = 1;\n   846\tlet b = 2;")];

  let mut claude_fenced_read = call("claude-fenced", "Read src/lib.rs", ToolKind::Read);
  claude_fenced_read.locations = vec![ToolCallLocation::new("src/lib.rs")];
  claude_fenced_read.raw_input = Some(serde_json::json!({ "offset": 350 }));
  claude_fenced_read.content = vec![text_tool_content(
    "```rust\n   410\tfn from_block() {}\n   411\tfn next() {}\n```",
  )];

  let mut codex_raw_output_read = call("codex-raw", "Read src/main.rs", ToolKind::Read);
  codex_raw_output_read.raw_input = Some(serde_json::json!({ "offset": 12 }));
  codex_raw_output_read.raw_output = Some(serde_json::json!({
    "formatted_output": "fn main() {\n    println!(\"hi\");\n}"
  }));

  let fixtures = [
    (
      pi_read,
      "# Reviu\n\nKeyboard-first",
      Some(3),
      "Pi ACP raw read content keeps the location gutter",
    ),
    (
      claude_numbered_read,
      "let a = 1;\nlet b = 2;",
      Some(845),
      "Claude numbered read content is stripped before Reviu adds its gutter",
    ),
    (
      claude_fenced_read,
      "fn from_block() {}\nfn next() {}",
      Some(410),
      "Claude fenced numbered read content uses the fenced first line",
    ),
    (
      codex_raw_output_read,
      "fn main() {\n    println!(\"hi\");\n}",
      Some(12),
      "Codex raw output read content falls back to formatted_output and rawInput offset",
    ),
  ];

  for (call, expected_text, expected_start_line, message) in fixtures {
    let view = tool_call_view_from_fixture(call);
    assert!(matches!(view.kind, ToolKind::Read), "{message}");
    assert!(view.diffs.is_empty(), "{message}");
    assert_eq!(view.outputs.len(), 1, "{message}");
    assert_eq!(view.outputs[0].text, expected_text, "{message}");
    assert_eq!(view.read_start_line, expected_start_line, "{message}");
    assert_eq!(view.outputs[0].start_line, expected_start_line, "{message}");
  }
}

#[test]
fn acp_registry_location_only_read_fixture_keeps_the_header_without_preview() {
  let mut codex_location_only = call("codex-location", "Read src/main.rs", ToolKind::Read);
  codex_location_only.locations = vec![ToolCallLocation::new("src/main.rs").line(27_u32)];

  let view = tool_call_view_from_fixture(codex_location_only);

  assert_eq!(view.read_start_line, Some(27));
  assert_eq!(
    view.locations,
    vec![(PathBuf::from("src/main.rs"), Some(27))]
  );
  assert!(view.outputs.is_empty());
  assert!(view.diffs.is_empty());
}

#[gpui::test]
async fn read_without_output_snapshots_the_local_file_once(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-read-snapshot");
  std::fs::create_dir_all(dir.join("src")).expect("create src dir");
  std::fs::write(dir.join("src/main.rs"), "fn first() {}\nfn second() {}\n").expect("write file");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.cwd = dir.clone();
    panel.status = Status::Ready;
    panel.in_flight = true;
    let mut call = call("read1", "Read src/main.rs", ToolKind::Read);
    call.locations = vec![ToolCallLocation::new("src/main.rs").line(2_u32)];
    panel.on_event(AgentEvent::ToolCall(call), cx);
  });

  std::fs::write(dir.join("src/main.rs"), "changed\n").expect("rewrite file");
  panel.update(cx, |panel, cx| {
    panel.on_event(
      AgentEvent::ToolCallUpdate(ToolCallUpdate::new(
        ToolCallId::new("read1"),
        ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
      )),
      cx,
    );
  });

  panel.read_with(cx, |panel, _| {
    let Some(ChatItem::Tool(view)) = panel.items.last() else {
      panic!("tool expected");
    };
    assert_eq!(view.outputs.len(), 1);
    assert_eq!(view.outputs[0].text, "fn second() {}\n");
    assert_eq!(view.outputs[0].start_line, Some(2));
  });
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn read_snapshot_ignores_files_outside_the_session_root(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-read-snapshot-root");
  let outside = temp_dir("agent-read-snapshot-outside");
  std::fs::write(outside.join("secret.rs"), "fn secret() {}\n").expect("write file");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.cwd = dir.clone();
    panel.status = Status::Ready;
    panel.in_flight = true;
    let mut call = call("read1", "Read secret.rs", ToolKind::Read);
    call.locations = vec![ToolCallLocation::new(outside.join("secret.rs"))];
    panel.on_event(AgentEvent::ToolCall(call), cx);
  });

  panel.read_with(cx, |panel, _| {
    let Some(ChatItem::Tool(view)) = panel.items.last() else {
      panic!("tool expected");
    };
    assert!(view.outputs.is_empty());
  });
  std::fs::remove_dir_all(&dir).ok();
  std::fs::remove_dir_all(&outside).ok();
}

#[test]
fn read_snapshot_rejects_binary_or_too_large_files() {
  let binary_dir = temp_dir("agent-read-snapshot-binary");
  std::fs::write(binary_dir.join("binary.dat"), b"abc\0def").expect("write binary");
  assert_eq!(
    local_read_snapshot(&binary_dir, Path::new("binary.dat"), Some(1)),
    None
  );
  std::fs::remove_dir_all(&binary_dir).ok();

  let large_dir = temp_dir("agent-read-snapshot-large");
  let large = (0..=READ_SNAPSHOT_MAX_LINES)
    .map(|line| format!("line {line}"))
    .collect::<Vec<_>>()
    .join("\n");
  std::fs::write(large_dir.join("large.rs"), large).expect("write large file");
  assert_eq!(
    local_read_snapshot(&large_dir, Path::new("large.rs"), Some(1)),
    None
  );
  std::fs::remove_dir_all(&large_dir).ok();
}

#[gpui::test]
async fn read_location_update_can_fill_an_empty_snapshot(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-read-snapshot-update");
  std::fs::create_dir_all(dir.join("src")).expect("create src dir");
  std::fs::write(dir.join("src/lib.rs"), "fn one() {}\nfn two() {}\n").expect("write file");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.cwd = dir.clone();
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.on_event(
      AgentEvent::ToolCall(call("read1", "Read file", ToolKind::Read)),
      cx,
    );
    panel.on_event(
      AgentEvent::ToolCallUpdate(ToolCallUpdate::new(
        ToolCallId::new("read1"),
        ToolCallUpdateFields::new()
          .locations(vec![ToolCallLocation::new("src/lib.rs")])
          .status(ToolCallStatus::Completed),
      )),
      cx,
    );
  });

  panel.read_with(cx, |panel, _| {
    let Some(ChatItem::Tool(view)) = panel.items.last() else {
      panic!("tool expected");
    };
    assert_eq!(view.outputs[0].text, "fn one() {}\nfn two() {}\n");
    assert_eq!(view.outputs[0].start_line, Some(1));
  });
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn acp_registry_edit_diff_fixture_keeps_diff_render_data() {
  let mut edit_call = call("edit-diff", "Edit fixtures/acp/edit.rs", ToolKind::Edit);
  edit_call.content = vec![ToolCallContent::Diff(
    Diff::new("fixtures/acp/edit.rs", "fn b() {}\n").old_text(Some("fn a() {}\n".to_string())),
  )];

  let view = tool_call_view_from_fixture(edit_call);

  assert!(matches!(view.kind, ToolKind::Edit));
  assert!(view.outputs.is_empty());
  assert_eq!(view.diffs.len(), 1);
  assert_eq!(view.diffs[0].path, "fixtures/acp/edit.rs");
  assert_eq!(view.diffs[0].added, 1);
  assert_eq!(view.diffs[0].removed, 1);
  assert_eq!(view.diffs[0].lines.len(), 2);
  assert_eq!(view.diffs[0].lines[0].old_line, Some(1));
  assert_eq!(view.diffs[0].lines[1].new_line, Some(1));
}

#[test]
fn acp_registry_non_text_content_fixture_does_not_create_text_preview() {
  let mut image_call = call("image-read", "Read image", ToolKind::Read);
  image_call.content = vec![ToolCallContent::from(ContentBlock::Image(
    ImageContent::new("iVBORw0KGgo=", "image/png"),
  ))];

  let mut resource_call = call("resource-read", "Read resource", ToolKind::Read);
  resource_call.content = vec![ToolCallContent::from(ContentBlock::ResourceLink(
    ResourceLink::new("asset", "file:///tmp/asset.png"),
  ))];

  for call in [image_call, resource_call] {
    let view = tool_call_view_from_fixture(call);
    assert!(view.outputs.is_empty());
    assert!(view.diffs.is_empty());
  }
}

#[test]
fn acp_registry_large_output_fixture_is_collapsed_by_default() {
  let output = (1..=MAX_TOOL_OUTPUT_LINES_COLLAPSED + 5)
    .map(|line| format!("line {line}"))
    .collect::<Vec<_>>()
    .join("\n");
  let mut read_call = call("large-read", "Read large.rs", ToolKind::Read);
  read_call.locations = vec![ToolCallLocation::new("large.rs")];
  read_call.content = vec![text_tool_content(&output)];

  let view = tool_call_view_from_fixture(read_call);
  let output = &view.outputs[0];
  let total = output.text.lines().count();
  let visible = if output.expanded {
    total
  } else {
    total.min(MAX_TOOL_OUTPUT_LINES_COLLAPSED)
  };

  assert_eq!(total, MAX_TOOL_OUTPUT_LINES_COLLAPSED + 5);
  assert!(!output.expanded);
  assert_eq!(visible, MAX_TOOL_OUTPUT_LINES_COLLAPSED);
  assert_eq!(
    visible_output_line_ranges(&output.text, visible).len(),
    MAX_TOOL_OUTPUT_LINES_COLLAPSED
  );
}

#[test]
fn a_read_with_an_offset_numbers_from_the_real_file_line() {
  use crate::transcript::read_tool_start_line;

  // The raw input's offset fills in when the location carries no line.
  let locations = vec![(std::path::PathBuf::from("src/render.rs"), None)];
  let input = serde_json::json!({ "file_path": "src/render.rs", "offset": 350 });
  assert_eq!(
    read_tool_start_line(&ToolKind::Read, &locations, Some(&input)),
    Some(350)
  );
  // An explicit location line stays authoritative.
  let located = vec![(std::path::PathBuf::from("src/render.rs"), Some(42))];
  assert_eq!(
    read_tool_start_line(&ToolKind::Read, &located, Some(&input)),
    Some(42)
  );
  // No location and no offset: nothing proves this reads a file.
  assert_eq!(read_tool_start_line(&ToolKind::Read, &[], None), None);
  // An offset alone is enough, and 0 clamps to the first line.
  let zero = serde_json::json!({ "offset": 0 });
  assert_eq!(
    read_tool_start_line(&ToolKind::Read, &[], Some(&zero)),
    Some(1)
  );
  assert_eq!(read_tool_start_line(&ToolKind::Fetch, &located, None), None);

  // End to end through the tool-call pipeline: the outputs carry the offset.
  let mut items = Vec::new();
  let mut index = HashMap::new();
  let mut call = ToolCall::new(ToolCallId::new("read1"), "Read render.rs");
  call.kind = ToolKind::Read;
  call.locations = vec![ToolCallLocation::new("src/render.rs")];
  call.raw_input = Some(serde_json::json!({ "offset": 350 }));
  call.content = vec![ToolCallContent::from(
    agent_client_protocol::schema::ContentBlock::Text(TextContent::new("let a = 1;\nlet b = 2;")),
  )];
  upsert_tool_call_pure(&mut items, &mut index, call, test_cwd());
  let ChatItem::Tool(view) = &items[0] else {
    panic!("tool expected");
  };
  assert_eq!(view.read_start_line, Some(350));
  assert_eq!(view.outputs[0].start_line, Some(350));
}

#[test]
fn a_cat_numbered_read_uses_the_agent_line_numbers_once() {
  use crate::transcript::upsert_tool_call_pure;

  let mut items = Vec::new();
  let mut index = HashMap::new();
  let mut call = ToolCall::new(ToolCallId::new("read1"), "Read render.rs");
  call.kind = ToolKind::Read;
  call.locations = vec![ToolCallLocation::new("src/render.rs")];
  call.content = vec![ToolCallContent::from(
    agent_client_protocol::schema::ContentBlock::Text(TextContent::new(
      "   845\tlet a = 1;\n   846\tlet b = 2;",
    )),
  )];
  upsert_tool_call_pure(&mut items, &mut index, call, test_cwd());

  let ChatItem::Tool(view) = &items[0] else {
    panic!("tool expected");
  };
  assert_eq!(view.read_start_line, Some(845));
  assert_eq!(view.outputs[0].start_line, Some(845));
  assert_eq!(view.outputs[0].text, "let a = 1;\nlet b = 2;");
}

#[test]
fn read_offset_survives_status_updates_without_raw_input() {
  use crate::transcript::{apply_tool_call_update_pure, upsert_tool_call_pure};

  let mut items = Vec::new();
  let mut index = HashMap::new();
  let mut call = ToolCall::new(ToolCallId::new("read1"), "Read render.rs");
  call.kind = ToolKind::Read;
  call.locations = vec![ToolCallLocation::new("src/render.rs")];
  call.raw_input = Some(serde_json::json!({ "offset": 350 }));
  call.content = vec![ToolCallContent::from(
    agent_client_protocol::schema::ContentBlock::Text(TextContent::new("let a = 1;\nlet b = 2;")),
  )];
  upsert_tool_call_pure(&mut items, &mut index, call, test_cwd());

  let update = ToolCallUpdate::new(
    ToolCallId::new("read1"),
    ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
  );
  apply_tool_call_update_pure(&mut items, &index, update, test_cwd());

  let ChatItem::Tool(view) = &items[0] else {
    panic!("tool expected");
  };
  assert_eq!(view.read_start_line, Some(350));
  assert_eq!(view.outputs[0].start_line, Some(350));
}

#[test]
fn tool_call_normalization_preserves_wire_fields() {
  let mut meta = agent_client_protocol::schema::Meta::new();
  meta.insert("tool_name".into(), serde_json::json!("read_file"));
  let raw_input = serde_json::json!({ "file_path": "src/lib.rs", "offset": 12 });
  let raw_output = serde_json::json!({ "output": "fn main() {}" });
  let content = vec![ToolCallContent::from(
    agent_client_protocol::schema::ContentBlock::Text(TextContent::new("fn main() {}")),
  )];
  let mut call = ToolCall::new(ToolCallId::new("read1"), "Read src/lib.rs");
  call.kind = ToolKind::Read;
  call.status = ToolCallStatus::Completed;
  call.locations = vec![ToolCallLocation::new("src/lib.rs").line(Some(12))];
  call.raw_input = Some(raw_input.clone());
  call.raw_output = Some(raw_output.clone());
  call.meta = Some(meta.clone());
  call.content = content.clone();

  let normalized = NormalizedToolCall::from(call);

  assert_eq!(normalized.id, ToolCallId::new("read1"));
  assert_eq!(normalized.title, "Read src/lib.rs");
  assert_eq!(normalized.kind, ToolKind::Read);
  assert_eq!(normalized.status, ToolCallStatus::Completed);
  assert_eq!(
    normalized.locations,
    vec![(PathBuf::from("src/lib.rs"), Some(12))]
  );
  assert_eq!(normalized.raw_input, Some(raw_input));
  assert_eq!(normalized.raw_output, Some(raw_output));
  assert_eq!(normalized.meta, Some(meta));
  assert_eq!(normalized.tool_name.as_deref(), Some("read_file"));
  assert_eq!(normalized.content, content);
}

#[test]
fn tool_update_normalization_keeps_shape_fields_optional() {
  let mut meta = agent_client_protocol::schema::Meta::new();
  meta.insert("tool_name".into(), serde_json::json!("read_file"));
  let raw_output = serde_json::json!({ "output": "done" });
  let update = ToolCallUpdate::new(
    ToolCallId::new("read1"),
    ToolCallUpdateFields::new()
      .status(ToolCallStatus::Completed)
      .raw_output(raw_output.clone()),
  )
  .meta(meta.clone());

  let normalized = NormalizedToolCallUpdate::from(update);

  assert_eq!(normalized.id, ToolCallId::new("read1"));
  assert_eq!(normalized.kind, None);
  assert_eq!(normalized.status, Some(ToolCallStatus::Completed));
  assert_eq!(normalized.title, None);
  assert_eq!(normalized.locations, None);
  assert_eq!(normalized.raw_input, None);
  assert_eq!(normalized.raw_output, Some(raw_output));
  assert_eq!(normalized.meta, Some(meta));
  assert_eq!(normalized.tool_name.as_deref(), Some("read_file"));
  assert_eq!(normalized.content, None);
}

#[test]
fn tool_name_metadata_can_hint_kind_without_parsing_titles() {
  let mut meta = agent_client_protocol::schema::Meta::new();
  meta.insert("tool_name".into(), serde_json::json!("read_file"));
  let mut call = ToolCall::new(ToolCallId::new("read1"), "Inspecting target");
  call.kind = ToolKind::Other;
  call.locations = vec![ToolCallLocation::new("src/lib.rs")];
  call.meta = Some(meta);
  call.content = vec![text_tool_content("   12\tfn main() {}")];

  let view = tool_call_view_from_fixture(call);

  assert!(matches!(view.kind, ToolKind::Read));
  assert_eq!(view.tool_name.as_deref(), Some("read_file"));
  assert_eq!(view.read_start_line, Some(12));
  assert_eq!(view.outputs[0].start_line, Some(12));
  assert_eq!(view.outputs[0].text, "fn main() {}");
}

#[test]
fn grok_style_tool_name_metadata_is_captured() {
  let mut meta = agent_client_protocol::schema::Meta::new();
  meta.insert("x.ai/tool.name".into(), serde_json::json!("shell_exec"));
  let mut call = ToolCall::new(ToolCallId::new("run1"), "Working");
  call.kind = ToolKind::Other;
  call.raw_input = Some(serde_json::json!({ "command": "cargo test" }));
  call.meta = Some(meta);

  let normalized = NormalizedToolCall::from(call);

  assert_eq!(normalized.tool_name.as_deref(), Some("shell_exec"));
  assert!(matches!(normalized.kind, ToolKind::Execute));
}

#[test]
fn tool_name_metadata_does_not_override_explicit_kind() {
  let mut meta = agent_client_protocol::schema::Meta::new();
  meta.insert("tool_name".into(), serde_json::json!("read_file"));
  let mut call = ToolCall::new(ToolCallId::new("run1"), "Run tests");
  call.kind = ToolKind::Execute;
  call.raw_input = Some(serde_json::json!({ "command": "cargo test" }));
  call.locations = vec![ToolCallLocation::new("src/lib.rs")];
  call.meta = Some(meta);

  let normalized = NormalizedToolCall::from(call);

  assert_eq!(normalized.tool_name.as_deref(), Some("read_file"));
  assert!(matches!(normalized.kind, ToolKind::Execute));
}

#[test]
fn tool_name_metadata_alone_is_not_enough_to_change_kind() {
  let mut meta = agent_client_protocol::schema::Meta::new();
  meta.insert("tool_name".into(), serde_json::json!("read_file"));
  let mut call = ToolCall::new(ToolCallId::new("read1"), "Working");
  call.kind = ToolKind::Other;
  call.meta = Some(meta);

  let normalized = NormalizedToolCall::from(call);

  assert_eq!(normalized.tool_name.as_deref(), Some("read_file"));
  assert!(matches!(normalized.kind, ToolKind::Other));
}

#[test]
fn read_location_update_refreshes_existing_output_line_numbers() {
  use crate::transcript::{apply_tool_call_update_pure, upsert_tool_call_pure};

  let mut items = Vec::new();
  let mut index = HashMap::new();
  let mut call = ToolCall::new(ToolCallId::new("read1"), "Read render.rs");
  call.kind = ToolKind::Read;
  call.locations = vec![ToolCallLocation::new("src/render.rs")];
  call.content = vec![ToolCallContent::from(
    agent_client_protocol::schema::ContentBlock::Text(TextContent::new("let a = 1;\nlet b = 2;")),
  )];
  upsert_tool_call_pure(&mut items, &mut index, call, test_cwd());

  let update = ToolCallUpdate::new(
    ToolCallId::new("read1"),
    ToolCallUpdateFields::new()
      .locations(vec![ToolCallLocation::new("src/render.rs").line(Some(42))]),
  );
  apply_tool_call_update_pure(&mut items, &index, update, test_cwd());

  let ChatItem::Tool(view) = &items[0] else {
    panic!("tool expected");
  };
  assert_eq!(view.read_start_line, Some(42));
  assert_eq!(view.outputs[0].start_line, Some(42));
}

#[test]
fn raw_output_only_tool_call_builds_a_preview() {
  use crate::transcript::upsert_tool_call_pure;

  let mut items = Vec::new();
  let mut index = HashMap::new();
  let mut call = ToolCall::new(ToolCallId::new("read1"), "Read render.rs");
  call.kind = ToolKind::Read;
  call.locations = vec![ToolCallLocation::new("src/render.rs")];
  call.raw_output = Some(serde_json::json!({
    "output": "   845\tlet a = 1;\n   846\tlet b = 2;"
  }));
  upsert_tool_call_pure(&mut items, &mut index, call, test_cwd());

  let ChatItem::Tool(view) = &items[0] else {
    panic!("tool expected");
  };
  assert_eq!(view.read_start_line, Some(845));
  assert_eq!(view.outputs.len(), 1);
  assert_eq!(view.outputs[0].text, "let a = 1;\nlet b = 2;");
  assert_eq!(view.outputs[0].start_line, Some(845));
}

#[test]
fn raw_output_only_tool_update_fills_an_empty_preview() {
  use crate::transcript::{apply_tool_call_update_pure, upsert_tool_call_pure};

  let mut items = Vec::new();
  let mut index = HashMap::new();
  let mut call = ToolCall::new(ToolCallId::new("run1"), "Run tests");
  call.kind = ToolKind::Execute;
  upsert_tool_call_pure(&mut items, &mut index, call, test_cwd());

  let update = ToolCallUpdate::new(
    ToolCallId::new("run1"),
    ToolCallUpdateFields::new().raw_output(serde_json::json!({ "stdout": "tests ok" })),
  );
  apply_tool_call_update_pure(&mut items, &index, update, test_cwd());

  let ChatItem::Tool(view) = &items[0] else {
    panic!("tool expected");
  };
  assert_eq!(view.outputs.len(), 1);
  assert_eq!(view.outputs[0].text, "tests ok");
  assert_eq!(view.outputs[0].start_line, None);
}

#[test]
fn raw_output_update_does_not_replace_existing_content_preview() {
  use crate::transcript::{apply_tool_call_update_pure, upsert_tool_call_pure};

  let mut items = Vec::new();
  let mut index = HashMap::new();
  let mut call = ToolCall::new(ToolCallId::new("run1"), "Run tests");
  call.kind = ToolKind::Execute;
  call.content = vec![ToolCallContent::from(
    agent_client_protocol::schema::ContentBlock::Text(TextContent::new("live output")),
  )];
  upsert_tool_call_pure(&mut items, &mut index, call, test_cwd());

  let update = ToolCallUpdate::new(
    ToolCallId::new("run1"),
    ToolCallUpdateFields::new().raw_output(serde_json::json!({ "stdout": "final output" })),
  );
  apply_tool_call_update_pure(&mut items, &index, update, test_cwd());

  let ChatItem::Tool(view) = &items[0] else {
    panic!("tool expected");
  };
  assert_eq!(view.outputs.len(), 1);
  assert_eq!(view.outputs[0].text, "live output");
}

#[test]
fn terminal_tail_ranges_slice_their_lines_and_strip_ansi() {
  // Colored lines: the selectable text must be the stripped one.
  let output = (0..30)
    .map(|i| format!("\u{1b}[32mline {i}\u{1b}[0m"))
    .collect::<Vec<_>>()
    .join("\n");
  let parsed = crate::ansi::parse_ansi(&output);
  let start = parsed.len().saturating_sub(24);
  assert!(start > 0, "30 lines against a 24-line tail clips the top");
  let (text, ranges) = join_terminal_tail(&parsed[start..]);
  assert_eq!(ranges.len(), 24);
  assert_eq!(&text[ranges[0].clone()], "line 6", "the tail keeps the end");
  assert_eq!(&text[ranges[23].clone()], "line 29");
  assert!(
    !text.contains('\u{1b}'),
    "escape sequences never reach the selection text"
  );

  let parsed = crate::ansi::parse_ansi("only\ntwo");
  let (text, ranges) = join_terminal_tail(&parsed);
  assert_eq!(&text[ranges[0].clone()], "only");
  assert_eq!(&text[ranges[1].clone()], "two");

  let (text, ranges) = join_terminal_tail(&[]);
  assert!(text.is_empty());
  assert!(ranges.is_empty());
}

#[test]
fn ansi_tool_output_rows_strip_escape_sequences_and_keep_style_runs() {
  let font = Font {
    family: "monospace".into(),
    ..Default::default()
  };
  let (rows, text, ranges) = ansi_output_rows(
    "plain\n\u{1b}[1;32mgreen\u{1b}[0m",
    &font,
    gpui::black(),
    true,
    gpui::transparent_black(),
  )
  .expect("ansi output rows");

  assert_eq!(text, "plain\ngreen");
  assert_eq!(ranges.len(), 2);
  assert_eq!(&text[ranges[0].clone()], "plain");
  assert_eq!(&text[ranges[1].clone()], "green");
  assert!(!text.contains('\u{1b}'));
  assert!(rows[0].runs.is_empty());
  assert!(!rows[1].runs.is_empty());
  assert_eq!(rows[1].text.as_ref(), "green");
}

#[test]
fn ansi_tool_output_rows_handle_carriage_return_progress() {
  let font = Font {
    family: "monospace".into(),
    ..Default::default()
  };
  let (rows, text, ranges) = ansi_output_rows(
    "progress 10%\rprogress 100%\n",
    &font,
    gpui::black(),
    true,
    gpui::transparent_black(),
  )
  .expect("carriage return output rows");

  assert_eq!(text, "progress 100%");
  assert_eq!(ranges.len(), 1);
  assert_eq!(&text[ranges[0].clone()], "progress 100%");
  assert_eq!(rows[0].text.as_ref(), "progress 100%");
}

#[test]
fn ansi_tool_output_rows_leave_plain_crlf_output_on_the_text_path() {
  let font = Font {
    family: "monospace".into(),
    ..Default::default()
  };

  assert!(
    ansi_output_rows(
      "windows\r\nline",
      &font,
      gpui::black(),
      true,
      gpui::transparent_black(),
    )
    .is_none()
  );
}

#[gpui::test]
async fn a_terminal_burst_lands_as_one_update(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  let (tx, rx) = async_channel::unbounded();
  panel.update(cx, |panel, cx| {
    panel.start_terminal_forwarder(rx, cx);
  });
  let notifies = std::rc::Rc::new(std::cell::Cell::new(0usize));
  cx.update(|_, cx| {
    let notifies = notifies.clone();
    cx.observe(&panel, move |_, _| notifies.set(notifies.get() + 1))
      .detach();
  });
  for _ in 0..5 {
    tx.send_blocking("term-1".to_string())
      .expect("channel open");
  }
  tx.send_blocking("term-2".to_string())
    .expect("channel open");
  cx.run_until_parked();
  assert_eq!(
    notifies.get(),
    1,
    "a PTY flood coalesces into one update + notify"
  );
}

#[gpui::test]
async fn the_auto_approve_flag_survives_a_conversation_reload(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-auto-approve");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = vec![user_message("hi")];
    panel.toggle_auto_approve(cx);
  });
  cx.run_until_parked();
  let conv_id = panel.read_with(cx, |panel, _| panel.current_conv.id.clone());
  let (_, _, _, _, auto_approve) =
    load_conversation_file(&dir.join(format!("{conv_id}.json"))).expect("reloads");
  assert!(auto_approve, "the toggle persists with the conversation");
}

#[test]
fn persisted_tools_backfill_line_numbers_and_highlights_on_load() {
  let dir = temp_dir("agent-tool-reload");
  let conv_id = "legacy-lines";
  let path = dir.join(format!("{conv_id}.json"));
  let conversation = PersistedConversation {
    version: crate::persistence::CONVERSATION_FORMAT_VERSION,
    meta: ConversationMeta {
      id: conv_id.to_string(),
      started_at_secs: 0,
      updated_at_secs: 0,
      title: "legacy".to_string(),
      message_count: 1,
      agent_id: default_agent_id(),
      session_id: None,
      preview: String::new(),
    },
    items: vec![PersistedChatItem::Tool(ToolCallView {
      id: ToolCallId::new(std::sync::Arc::from("tool-1")),
      title: "Edit src/main.rs".to_string(),
      kind: ToolKind::Edit,
      status: ToolCallStatus::Completed,
      tool_name: None,
      locations: vec![(PathBuf::from("src/main.rs"), Some(12))],
      diffs: vec![DiffSummary {
        path: "src/main.rs".to_string(),
        added: 1,
        removed: 1,
        lines: vec![
          crate::diff::DiffLine {
            kind: DiffLineKind::Removed,
            old_line: None,
            new_line: None,
            text: "fn old() {}".to_string(),
            spans: Vec::new(),
            syntax_spans: Vec::new(),
          },
          crate::diff::DiffLine {
            kind: DiffLineKind::Added,
            old_line: None,
            new_line: None,
            text: "fn new() {}".to_string(),
            spans: Vec::new(),
            syntax_spans: Vec::new(),
          },
        ],
        expanded: false,
      }],
      outputs: vec![ToolOutput {
        text: "fn output() {}".to_string(),
        start_line: None,
        expanded: false,
        syntax_spans: Vec::new(),
      }],
      terminals: Vec::new(),
      read_start_line: None,
      content_fp: 0,
    })],
    group_pins: HashMap::new(),
    auto_approve: false,
  };
  std::fs::write(
    &path,
    serde_json::to_string(&conversation).expect("serialize conversation"),
  )
  .expect("write conversation");

  let (_, items, _, _, _) = load_conversation_file(&path).expect("reloads");
  let ChatItem::Tool(tool) = &items[0] else {
    panic!("tool item");
  };
  assert_eq!(tool.diffs[0].lines[0].old_line, Some(12));
  assert_eq!(tool.diffs[0].lines[0].new_line, None);
  assert_eq!(tool.diffs[0].lines[1].old_line, None);
  assert_eq!(tool.diffs[0].lines[1].new_line, Some(12));
  assert!(
    !tool.diffs[0].lines[0].syntax_spans.is_empty(),
    "diff syntax is restored after serde skipped it"
  );
  assert!(
    !tool.outputs[0].syntax_spans.is_empty(),
    "output syntax is restored after serde skipped it"
  );

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn turn_summary_survives_persistence_roundtrip_collapsed() {
  let view = TurnSummaryView {
    files: vec![TurnFileStat {
      path: "src/lib.rs".into(),
      added: 12,
      removed: 3,
    }],
    checkpoint_ref: Some("refs/reviu/checkpoints/s/1".into()),
    undone: false,
    duration_secs: Some(125),
    expanded: true,
    work_expanded: true,
  };
  let json =
    serde_json::to_string(&PersistedChatItem::TurnSummary(view.clone())).expect("serialize");
  let restored: PersistedChatItem = serde_json::from_str(&json).expect("deserialize");
  match restored {
    PersistedChatItem::TurnSummary(restored) => {
      assert_eq!(restored.files, view.files);
      assert_eq!(restored.checkpoint_ref, view.checkpoint_ref);
      assert_eq!(restored.duration_secs, Some(125));
      // Expansion is a live-view state, not part of the transcript.
      assert!(!restored.expanded);
      assert!(!restored.work_expanded);
    }
    _ => panic!("expected turn summary item"),
  }
}

#[test]
fn a_settled_turn_folds_its_work_but_not_the_final_answer() {
  let items = vec![
    checkpoint_marker("cp-1"),
    user_message("do it"),
    ChatItem::Thought(ThoughtView {
      text: "planning".into(),
      collapsed: true,
    }),
    edit_tool("t1", vec![("a.rs", 1, 0)]),
    agent_message("first I will"),
    edit_tool("t2", vec![("a.rs", 1, 0)]),
    agent_message("done, part one"),
    agent_message("and part two"),
    summary_item("cp-1"),
  ];
  let summary_idx = 8;
  assert_eq!(hiding_turn_summary(&items, 2), Some(summary_idx), "thought");
  assert_eq!(hiding_turn_summary(&items, 3), Some(summary_idx), "tool");
  assert_eq!(
    hiding_turn_summary(&items, 4),
    Some(summary_idx),
    "interim prose followed by a tool is work"
  );
  assert_eq!(
    hiding_turn_summary(&items, 6),
    None,
    "the trailing answer run stays visible"
  );
  assert_eq!(hiding_turn_summary(&items, 7), None);
  assert_eq!(hiding_turn_summary(&items, 1), None, "the user message");
  assert_eq!(hiding_turn_summary(&items, 8), None, "the card itself");
  assert_eq!(folded_step_count(&items, summary_idx), 4);

  // A turn without a card folds nothing.
  let open = vec![
    checkpoint_marker("cp-1"),
    user_message("do it"),
    edit_tool("t1", vec![("a.rs", 1, 0)]),
    agent_message("working"),
  ];
  assert_eq!(hiding_turn_summary(&open, 2), None);
  assert_eq!(hiding_turn_summary(&open, 3), None);
}

#[test]
fn format_turn_duration_reads_naturally() {
  assert_eq!(format_turn_duration(0), "0s");
  assert_eq!(format_turn_duration(12), "12s");
  assert_eq!(format_turn_duration(60), "1m");
  assert_eq!(format_turn_duration(125), "2m 5s");
}

#[gpui::test]
async fn clicking_the_work_row_unfolds_the_turn(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![
      checkpoint_marker("cp-1"),
      user_message("do it"),
      edit_tool("t1", vec![("a.rs", 1, 0)]),
      agent_message("done"),
      summary_item("cp-1"),
    ];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  let work = cx
    .debug_bounds("turn-summary-work")
    .expect("the folded-work row is painted");
  cx.simulate_click(work.center(), gpui::Modifiers::default());
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(matches!(
      panel.items.last(),
      Some(ChatItem::TurnSummary(s)) if s.work_expanded
    ));
  });

  let work = cx
    .debug_bounds("turn-summary-work")
    .expect("the row stays while expanded");
  cx.simulate_click(work.center(), gpui::Modifiers::default());
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(matches!(
      panel.items.last(),
      Some(ChatItem::TurnSummary(s)) if !s.work_expanded
    ));
  });
}

#[gpui::test]
async fn a_turn_with_edits_appends_a_summary_card(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, _| {
    panel.items = vec![
      checkpoint_marker("cp-1"),
      user_message("edit things"),
      edit_tool("t1", vec![("a.rs", 4, 2)]),
    ];
    panel.append_turn_summary();
    match panel.items.last() {
      Some(ChatItem::TurnSummary(s)) => {
        assert_eq!(s.checkpoint_ref, Some("cp-1".to_string()));
        assert_eq!(s.files.len(), 1);
      }
      other => panic!("expected a turn summary, got {other:?}"),
    }
    // A turn without edits must not add a second card.
    let len = panel.items.len();
    panel.append_turn_summary();
    assert_eq!(panel.items.len(), len);
  });
}

#[test]
fn strip_effort_suffix_removes_known_levels_only() {
  assert_eq!(strip_effort_suffix("GPT-5.6-Sol (low)"), "GPT-5.6-Sol");
  assert_eq!(strip_effort_suffix("GPT-5.6-Sol (xhigh)"), "GPT-5.6-Sol");
  assert_eq!(strip_effort_suffix("GPT-5.6-Sol"), "GPT-5.6-Sol");
  // Parenthesized content that is not an effort level stays.
  assert_eq!(strip_effort_suffix("Claude (latest)"), "Claude (latest)");
  // A name that is only a suffix stays untouched.
  assert_eq!(strip_effort_suffix(" (low)"), " (low)");
}

#[test]
fn humanize_agent_error_extracts_nested_detail() {
  let raw = r#"acp prompt error: Internal error: {
    "message": "{\"detail\":\"The 'gpt-5.2-codex' model is not supported when using Codex with a ChatGPT account.\"}",
    "codex_error_info": "other"
  }"#;
  assert_eq!(
    humanize_agent_error(raw).as_deref(),
    Some("The 'gpt-5.2-codex' model is not supported when using Codex with a ChatGPT account.")
  );

  let flat = r#"error: {"message": "rate limited"}"#;
  assert_eq!(humanize_agent_error(flat).as_deref(), Some("rate limited"));

  assert_eq!(humanize_agent_error("plain text failure"), None);
  assert_eq!(humanize_agent_error("error: {not json"), None);
}

#[test]
fn agent_settings_json_keeps_backend_and_models_independent() {
  let settings = serde_json::json!({});
  let settings = settings_with_model(settings, "codex", "gpt-5.6-sol");
  let settings = settings_with_backend(settings, "claude");
  let settings = settings_with_model(settings, "claude", "claude-opus-5");

  assert_eq!(settings["backend"], "claude");
  assert_eq!(
    model_choice_from_settings(&settings, "codex").as_deref(),
    Some("gpt-5.6-sol")
  );
  assert_eq!(
    model_choice_from_settings(&settings, "claude").as_deref(),
    Some("claude-opus-5")
  );
  assert_eq!(model_choice_from_settings(&settings, "unknown"), None);
}

#[test]
fn thought_preview_skips_blank_lines_and_strips_emphasis() {
  assert_eq!(
    thought_preview("\n\n**Planning readme inspection strategy**\ndetails"),
    Some("Planning readme inspection strategy".to_string())
  );
  assert_eq!(thought_preview("   \n\n  "), None);
  assert_eq!(thought_preview(""), None);
}

#[test]
fn review_export_label_counts_sections() {
  assert_eq!(review_export_label("no sections here"), "1 review comment");
  assert_eq!(
    review_export_label("### a.rs:L1 (new side)\nfix\n"),
    "1 review comment"
  );
  assert_eq!(
    review_export_label("### a.rs:L1 (new side)\nfix\n\n### b.rs:L2 (new side)\nrename\n"),
    "2 review comments"
  );
}

#[gpui::test]
async fn review_export_renders_as_a_prompt_card(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![ChatItem::Message(ChatMessage {
      role: ChatRole::ReviewExport,
      text: "### a.rs:L1 (new side)\nfix this\n".to_string(),
      images: 0,
      image_data: Vec::new(),
    })];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("agent-chat-review-card").is_some(),
    "review exports render as prompt cards"
  );
  assert!(
    cx.debug_bounds("agent-chat-review-header").is_some(),
    "review exports use the shared card header"
  );
}

#[test]
fn review_export_role_survives_persistence_roundtrip() {
  let message = ChatMessage {
    role: ChatRole::ReviewExport,
    text: "### a.rs:L1 (new side)\nfix\n".to_string(),
    images: 0,
    image_data: Vec::new(),
  };
  let json = serde_json::to_string(&message).expect("serialize");
  let restored: ChatMessage = serde_json::from_str(&json).expect("deserialize");
  assert_eq!(restored.role, ChatRole::ReviewExport);
  assert_eq!(restored.text, message.text);
}

#[test]
fn upsert_inserts_new_tool_call() {
  let mut items: Vec<ChatItem> = Vec::new();
  let mut index = HashMap::new();
  upsert_tool_call_pure(
    &mut items,
    &mut index,
    call("a", "Read foo", ToolKind::Read),
    test_cwd(),
  );
  assert_eq!(items.len(), 1);
  assert_eq!(index.len(), 1);
  let ChatItem::Tool(view) = &items[0] else {
    panic!("expected Tool item")
  };
  assert_eq!(view.title, "Read foo");
}

#[test]
fn upsert_replaces_existing_by_id() {
  let mut items: Vec<ChatItem> = Vec::new();
  let mut index = HashMap::new();
  upsert_tool_call_pure(
    &mut items,
    &mut index,
    call("a", "Old", ToolKind::Read),
    test_cwd(),
  );
  upsert_tool_call_pure(
    &mut items,
    &mut index,
    call("a", "New", ToolKind::Edit),
    test_cwd(),
  );
  assert_eq!(items.len(), 1);
  let ChatItem::Tool(view) = &items[0] else {
    panic!()
  };
  assert_eq!(view.title, "New");
  assert!(matches!(view.kind, ToolKind::Edit));
}

#[test]
fn upsert_appends_distinct_ids_in_order() {
  let mut items: Vec<ChatItem> = Vec::new();
  let mut index = HashMap::new();
  upsert_tool_call_pure(
    &mut items,
    &mut index,
    call("a", "A", ToolKind::Read),
    test_cwd(),
  );
  upsert_tool_call_pure(
    &mut items,
    &mut index,
    call("b", "B", ToolKind::Edit),
    test_cwd(),
  );
  assert_eq!(items.len(), 2);
  assert_eq!(index.get(&ToolCallId::from("a")), Some(&0));
  assert_eq!(index.get(&ToolCallId::from("b")), Some(&1));
}

#[test]
fn apply_update_merges_partial_fields() {
  let mut items: Vec<ChatItem> = Vec::new();
  let mut index = HashMap::new();
  upsert_tool_call_pure(
    &mut items,
    &mut index,
    call("a", "Initial", ToolKind::Read),
    test_cwd(),
  );

  let mut fields = ToolCallUpdateFields::default();
  fields.status = Some(ToolCallStatus::InProgress);
  let update = ToolCallUpdate::new(ToolCallId::from("a"), fields);
  apply_tool_call_update_pure(&mut items, &index, update, test_cwd());

  let ChatItem::Tool(view) = &items[0] else {
    panic!()
  };
  assert_eq!(view.title, "Initial");
  assert!(matches!(view.status, ToolCallStatus::InProgress));
}

#[test]
fn apply_update_unknown_id_is_noop() {
  let mut items: Vec<ChatItem> = Vec::new();
  let index = HashMap::new();
  let mut fields = ToolCallUpdateFields::default();
  fields.status = Some(ToolCallStatus::Completed);
  let update = ToolCallUpdate::new(ToolCallId::from("ghost"), fields);
  apply_tool_call_update_pure(&mut items, &index, update, test_cwd());
  assert!(items.is_empty());
}

#[test]
fn apply_update_replaces_locations() {
  let mut items: Vec<ChatItem> = Vec::new();
  let mut index = HashMap::new();
  upsert_tool_call_pure(
    &mut items,
    &mut index,
    call("a", "Edit", ToolKind::Edit),
    test_cwd(),
  );

  let mut fields = ToolCallUpdateFields::default();
  fields.locations = Some(vec![ToolCallLocation::new("foo.rs").line(42_u32)]);
  let update = ToolCallUpdate::new(ToolCallId::from("a"), fields);
  apply_tool_call_update_pure(&mut items, &index, update, test_cwd());

  let ChatItem::Tool(view) = &items[0] else {
    panic!()
  };
  assert_eq!(view.locations.len(), 1);
  assert_eq!(view.locations[0].1, Some(42));
}

#[test]
fn diff_expansion_defaults_to_collapsed() {
  use agent_client_protocol::schema::{Diff, ToolCallContent};
  let content = vec![ToolCallContent::Diff(
    Diff::new("foo.rs", "new\n").old_text(Some("old\n".to_string())),
  )];
  let diffs = crate::diff::extract_diffs(&content, test_cwd());
  assert!(!diffs[0].expanded);
}

#[test]
fn session_info_update_sets_title_value_and_null_clears() {
  let mut meta = new_conversation_meta();
  meta.title = "hello".into();
  let value_update = SessionInfoUpdate::new().title("renamed");
  if let Some(title) = value_update.title.value() {
    meta.title = title.clone();
  }
  assert_eq!(meta.title, "renamed");

  let null_update = SessionInfoUpdate::new().title(None::<String>);
  if null_update.title.is_null() {
    meta.title.clear();
  }
  assert!(meta.title.is_empty());

  let undefined_update = SessionInfoUpdate::new();
  let snapshot = meta.title.clone();
  if let Some(title) = undefined_update.title.value() {
    meta.title = title.clone();
  } else if undefined_update.title.is_null() {
    meta.title.clear();
  }
  assert_eq!(meta.title, snapshot);
}

#[test]
fn plan_view_from_acp_maps_status_and_priority() {
  use agent_client_protocol::schema::PlanEntry;
  let plan = Plan::new(vec![
    PlanEntry::new(
      "do thing",
      PlanEntryPriority::High,
      PlanEntryStatus::InProgress,
    ),
    PlanEntry::new(
      "done thing",
      PlanEntryPriority::Low,
      PlanEntryStatus::Completed,
    ),
  ]);
  let view = plan_view_from_acp(&plan);
  assert_eq!(view.entries.len(), 2);
  assert_eq!(view.entries[0].content, "do thing");
  assert_eq!(view.entries[0].priority, PlanEntryPriorityView::High);
  assert_eq!(view.entries[0].status, PlanEntryStatusView::InProgress);
  assert_eq!(view.entries[1].status, PlanEntryStatusView::Completed);
  assert_eq!(view.entries[1].priority, PlanEntryPriorityView::Low);
}

#[test]
fn permission_destructive_kinds_match() {
  assert!(permission_option_is_destructive(
    &PermissionOptionKind::RejectOnce
  ));
  assert!(permission_option_is_destructive(
    &PermissionOptionKind::RejectAlways
  ));
  assert!(!permission_option_is_destructive(
    &PermissionOptionKind::AllowOnce
  ));
  assert!(!permission_option_is_destructive(
    &PermissionOptionKind::AllowAlways
  ));
}

#[test]
fn prune_old_state_deletes_files_older_than_threshold() {
  let dir = temp_dir("agent-prune");
  std::fs::write(dir.join("a.json"), "[]").unwrap();
  std::fs::write(dir.join("b.json"), "[]").unwrap();
  // sleep a hair so files have age > 0
  std::thread::sleep(std::time::Duration::from_millis(20));
  let pruned = AgentChatPanel::prune_old_state(&dir, std::time::Duration::from_millis(1));
  assert_eq!(pruned, 2);
  assert!(!dir.join("a.json").exists());
  assert!(!dir.join("b.json").exists());
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn prune_old_state_keeps_recent_files() {
  let dir = temp_dir("agent-prune-keep");
  std::fs::write(dir.join("fresh.json"), "[]").unwrap();
  let pruned = AgentChatPanel::prune_old_state(&dir, std::time::Duration::from_secs(60));
  assert_eq!(pruned, 0);
  assert!(dir.join("fresh.json").exists());
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn prune_old_state_never_touches_the_metadata_files() {
  let dir = temp_dir("agent-prune-metadata");
  // Old age says nothing about these: an untouched worktrees.json may still
  // bind live checkouts, active.txt still names the session to reopen.
  for name in [
    "index.json",
    "active.txt",
    "drafts.json",
    "scroll.json",
    "worktrees.json",
  ] {
    std::fs::write(dir.join(name), "{}").unwrap();
  }
  std::fs::write(dir.join("old-conversation.json"), "{}").unwrap();
  std::thread::sleep(std::time::Duration::from_millis(20));

  let pruned = AgentChatPanel::prune_old_state(&dir, std::time::Duration::from_millis(1));

  assert_eq!(pruned, 1, "only the conversation transcript is stale");
  assert!(!dir.join("old-conversation.json").exists());
  for name in [
    "index.json",
    "active.txt",
    "drafts.json",
    "scroll.json",
    "worktrees.json",
  ] {
    assert!(dir.join(name).exists(), "{name} must survive the prune");
  }
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn list_conversations_sorted_by_updated_at_desc() {
  let dir = temp_dir("agent-sort");
  let mk = |id: &str, started: u64, updated: u64| {
    let conv = PersistedConversation {
      version: crate::persistence::CONVERSATION_FORMAT_VERSION,
      meta: ConversationMeta {
        id: id.to_string(),
        started_at_secs: started,
        updated_at_secs: updated,
        title: id.to_string(),
        message_count: 1,
        agent_id: default_agent_id(),
        session_id: None,
        preview: String::new(),
      },
      items: vec![PersistedChatItem::Message(ChatMessage {
        role: ChatRole::User,
        text: "hi".into(),
        images: 0,
        image_data: Vec::new(),
      })],
      group_pins: HashMap::new(),
      auto_approve: false,
    };
    std::fs::write(
      dir.join(format!("{id}.json")),
      serde_json::to_string(&conv).unwrap(),
    )
    .unwrap();
  };
  mk("old", 1000, 1000);
  mk("recent_started_stale", 5000, 5000);
  mk("old_started_recent_updated", 2000, 9000);

  let metas = list_conversations_in(&dir);
  assert_eq!(metas[0].id, "old_started_recent_updated");
  assert_eq!(metas[1].id, "recent_started_stale");
  assert_eq!(metas[2].id, "old");
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn short_model_label_extracts_identifier_from_description() {
  assert_eq!(
    short_model_label(
      "Default (recommended)",
      Some("Opus 4.7 with 1M context · Most capable for complex work"),
    ),
    "Opus 4.7"
  );
  assert_eq!(
    short_model_label("Sonnet", Some("Sonnet 4.6 · Best for everyday tasks")),
    "Sonnet 4.6"
  );
  assert_eq!(
    short_model_label("Haiku", Some("Haiku 4.5 · Fastest for quick answers")),
    "Haiku 4.5"
  );
}

#[test]
fn short_model_label_falls_back_to_name_when_no_description() {
  assert_eq!(short_model_label("Sonnet", None), "Sonnet");
  assert_eq!(short_model_label("Sonnet", Some("")), "Sonnet");
}

#[test]
fn short_model_label_uses_name_when_description_has_no_separator() {
  // The default-effort suffix is dropped: the effort selector shows the applied value.
  assert_eq!(
    short_model_label(
      "gpt-5.2-codex (high)",
      Some("Frontier agentic coding model. Greater reasoning depth for complex problems"),
    ),
    "gpt-5.2-codex"
  );
  assert_eq!(
    short_model_label(
      "GPT-5.5 (high)",
      Some(
        "Frontier model for complex coding, research, and real-world work. Greater reasoning depth for complex problems"
      ),
    ),
    "GPT-5.5"
  );
}

fn select_option(
  id: &str,
  name: &str,
  current: &str,
  values: &[&str],
  category: Option<SessionConfigOptionCategory>,
) -> SessionConfigOption {
  let mut option = SessionConfigOption::new(
    SessionConfigId::new(std::sync::Arc::from(id)),
    name.to_string(),
    SessionConfigKind::Select(SessionConfigSelect::new(
      SessionConfigValueId::new(std::sync::Arc::from(current)),
      values
        .iter()
        .map(|value| {
          SessionConfigSelectOption::new(
            SessionConfigValueId::new(std::sync::Arc::from(*value)),
            value.to_string(),
          )
        })
        .collect::<Vec<_>>(),
    )),
  );
  option.category = category;
  option
}

#[test]
fn selectable_config_options_skips_primary_control_categories() {
  let options = vec![
    select_option(
      "model",
      "Model",
      "gpt-5.5",
      &["gpt-5.5"],
      Some(SessionConfigOptionCategory::Model),
    ),
    select_option(
      "mode",
      "Mode",
      "agent",
      &["agent"],
      Some(SessionConfigOptionCategory::Mode),
    ),
    select_option(
      "thinking",
      "Thinking",
      "low",
      &["low", "high"],
      Some(SessionConfigOptionCategory::ThoughtLevel),
    ),
    select_option("sandbox", "Sandbox", "off", &["off", "on"], None),
  ];

  let selectors = selectable_config_options(&options);

  assert_eq!(selectors.len(), 1);
  assert_eq!(selectors[0].name.as_ref(), "Sandbox");
  assert_eq!(selectors[0].current_label, "off");
}

#[test]
fn selectable_config_options_skips_reasoning_name_without_category() {
  let options = vec![
    select_option(
      "reasoning",
      "Reasoning effort",
      "low",
      &["low", "high"],
      None,
    ),
    select_option("sandbox", "Sandbox", "off", &["off", "on"], None),
  ];

  let selectors = selectable_config_options(&options);

  assert_eq!(selectors.len(), 1);
  assert_eq!(selectors[0].name.as_ref(), "Sandbox");
}

#[test]
fn dedicated_config_selectors_find_model_mode_and_reasoning() {
  let options = vec![
    select_option(
      "model",
      "Model",
      "sonnet",
      &["sonnet", "opus"],
      Some(SessionConfigOptionCategory::Model),
    ),
    select_option(
      "mode",
      "Mode",
      "build",
      &["plan", "build"],
      Some(SessionConfigOptionCategory::Mode),
    ),
    select_option(
      "thinking",
      "Thinking",
      "high",
      &["low", "high"],
      Some(SessionConfigOptionCategory::ThoughtLevel),
    ),
  ];

  assert_eq!(
    model_config_selector(&options).unwrap().current_label,
    "sonnet"
  );
  assert_eq!(
    mode_config_selector(&options).unwrap().current_label,
    "build"
  );
  assert_eq!(
    reasoning_config_selector(&options).unwrap().current_label,
    "high"
  );
}

#[test]
fn access_config_selector_claims_native_permission_policies() {
  let options = vec![
    select_option(
      "mode",
      "Mode",
      "Approve for me",
      &["Ask for approval", "Approve for me", "Full access"],
      Some(SessionConfigOptionCategory::Mode),
    ),
    select_option("detail", "Detail", "brief", &["brief", "verbose"], None),
  ];

  assert_eq!(
    access_config_selector(&options).unwrap().current_label,
    "Approve for me"
  );
  assert!(mode_config_selector(&options).is_none());

  let secondary = selectable_config_options(&options);
  assert_eq!(secondary.len(), 1);
  assert_eq!(secondary[0].name.as_ref(), "Detail");
}

#[test]
fn access_config_selector_ignores_secondary_mode_options() {
  let options = vec![
    select_option(
      "collaboration",
      "Collaboration mode",
      "default",
      &["Default", "Plan"],
      None,
    ),
    select_option("fast", "Fast mode", "off", &["Off", "On"], None),
  ];

  assert!(access_config_selector(&options).is_none());
  assert_eq!(selectable_config_options(&options).len(), 2);
}

#[test]
fn reasoning_labels_drop_provider_prefixes() {
  assert_eq!(reasoning_label("Thinking: high"), "High");
  assert_eq!(reasoning_label("Reasoning: low"), "Low");
  assert_eq!(reasoning_label("Xhigh"), "Xhigh");
}

#[test]
fn mode_reasoning_detection_keeps_build_modes_separate() {
  let mode = |id: &str, name: &str| {
    SessionMode::new(
      SessionModeId::new(std::sync::Arc::from(id)),
      name.to_string(),
    )
  };

  assert!(modes_are_reasoning(&[
    mode("low", "Low"),
    mode("high", "High")
  ]));
  assert!(!modes_are_reasoning(&[
    mode("plan", "Plan"),
    mode("build", "Build")
  ]));
}

#[test]
fn native_access_modes_are_not_regular_modes() {
  let mode = |id: &str, name: &str| {
    SessionMode::new(
      SessionModeId::new(std::sync::Arc::from(id)),
      name.to_string(),
    )
  };

  assert!(modes_are_access(&[
    mode("ask", "Ask for approval"),
    mode("approve", "Approve for me"),
    mode("full", "Full access")
  ]));
  assert!(!modes_are_access(&[
    mode("plan", "Plan"),
    mode("build", "Build")
  ]));
}

#[test]
fn selectable_config_options_falls_back_to_the_option_name_for_unknown_values() {
  let options = vec![select_option(
    "sandbox",
    "Sandbox",
    "gone",
    &["off", "on"],
    None,
  )];

  let selectors = selectable_config_options(&options);

  assert_eq!(selectors[0].current_label, "Sandbox");
}

#[test]
fn config_summary_joins_effective_values() {
  let options = vec![
    select_option("detail", "Detail", "verbose", &["brief", "verbose"], None),
    select_option("sandbox", "Sandbox", "off", &["off", "on"], None),
  ];

  let selectors = selectable_config_options(&options);

  assert_eq!(config_summary(&selectors), "verbose · off");
  assert_eq!(config_summary(&[]), "");
}

#[test]
fn config_customized_only_when_a_value_left_its_advertised_default() {
  let options = vec![select_option(
    "detail",
    "Detail",
    "brief",
    &["brief", "verbose"],
    None,
  )];
  let selectors = selectable_config_options(&options);
  let mut defaults = HashMap::new();
  defaults.insert(
    selectors[0].id.clone(),
    SessionConfigValueId::new(std::sync::Arc::from("brief")),
  );

  assert!(!config_customized(&selectors, &defaults));

  let changed = selectable_config_options(&[select_option(
    "detail",
    "Detail",
    "verbose",
    &["brief", "verbose"],
    None,
  )]);
  assert!(config_customized(&changed, &defaults));
}

#[test]
fn config_customized_is_false_without_a_recorded_default() {
  let selectors = selectable_config_options(&[select_option(
    "detail",
    "Detail",
    "verbose",
    &["brief", "verbose"],
    None,
  )]);

  assert!(!config_customized(&selectors, &HashMap::new()));
}

#[test]
fn working_word_holds_for_seven_seconds_then_moves_on() {
  let seed = working_word_seed("conv-1");

  assert_eq!(working_word(seed, 0), working_word(seed, 6));
  assert_ne!(working_word(seed, 6), working_word(seed, 7));
  assert_eq!(working_word(seed, 7), working_word(seed, 13));
}

#[test]
fn working_word_differs_between_conversations_at_the_same_moment() {
  let words: std::collections::HashSet<&str> = (0..8)
    .map(|ix| working_word(working_word_seed(&format!("conv-{ix}")), 0))
    .collect();

  assert!(words.len() > 1, "seeds must spread across the vocabulary");
}

#[test]
fn working_word_cycles_through_the_whole_vocabulary() {
  let seed = working_word_seed("conv-1");
  let cycle: std::collections::HashSet<&str> = (0..WORKING_WORDS.len() as u64)
    .map(|step| working_word(seed, step * WORKING_WORD_ROTATE_SECS))
    .collect();

  assert_eq!(cycle.len(), WORKING_WORDS.len());
}

#[test]
fn tool_kind_labels_cover_main_kinds() {
  assert_eq!(tool_kind_label(&ToolKind::Read), "Read");
  assert_eq!(tool_kind_label(&ToolKind::Edit), "Edit");
  assert_eq!(tool_kind_label(&ToolKind::Execute), "Run");
}

/// A real `new` panel sharing `store`, hydrating `resume`; the backend
/// override must point at a nonexistent binary so no process spawns.
fn resumed_panel(
  store: &gpui::Entity<crate::store::ConversationStore>,
  resume: Option<ConversationMeta>,
  cx: &mut gpui::VisualTestContext,
) -> gpui::Entity<AgentChatPanel> {
  let store = store.clone();
  cx.update(|window, cx| {
    cx.new(|cx| {
      AgentChatPanel::new(
        default_agent_id(),
        PathBuf::from("."),
        PathBuf::from("."),
        Some(store),
        resume,
        TurnGate::default(),
        window,
        cx,
      )
    })
  })
}

fn add_panel_window(
  cx: &mut gpui::TestAppContext,
) -> (gpui::Entity<AgentChatPanel>, &mut gpui::VisualTestContext) {
  cx.update(gpui_component::init);
  let mut mounted: Option<gpui::Entity<AgentChatPanel>> = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new_disconnected(PathBuf::from("."), window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  (mounted.expect("agent chat panel"), cx)
}

#[gpui::test]
async fn mounting_the_panel_spawns_no_agent_and_paints(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  cx.run_until_parked();

  panel.read_with(cx, |panel, _| {
    assert!(panel.session.is_none(), "no agent process was connected");
    assert!(panel._connect_task.is_none());
    // Connecting shows the generating row, plus the runway spacer.
    assert_eq!(panel.messages_list.item_count(), 2);
  });
}

#[gpui::test]
async fn connecting_panel_shows_loading_controls(cx: &mut gpui::TestAppContext) {
  let (_panel, cx) = add_panel_window(cx);
  cx.run_until_parked();

  assert!(cx.debug_bounds("agent-chat-controls-loading").is_some());
  assert!(cx.debug_bounds("agent-chat-auto-approve").is_none());
}

#[gpui::test]
async fn secondary_options_render_as_one_config_button(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.set_config_options(vec![
      select_option(
        "collaboration",
        "Collaboration mode",
        "default",
        &["Default", "Plan"],
        None,
      ),
      select_option("fast", "Fast mode", "off", &["Off", "On"], None),
    ]);
    cx.notify();
  });
  cx.run_until_parked();

  assert!(cx.debug_bounds("agent-chat-config").is_some());
}

#[gpui::test]
async fn native_access_control_hides_reviu_auto_approve(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.set_config_options(vec![select_option(
      "mode",
      "Mode",
      "Approve for me",
      &["Ask for approval", "Approve for me", "Full access"],
      Some(SessionConfigOptionCategory::Mode),
    )]);
    cx.notify();
  });
  cx.run_until_parked();

  assert!(cx.debug_bounds("agent-chat-access").is_some());
  assert!(cx.debug_bounds("agent-chat-auto-approve").is_none());
}

#[gpui::test]
async fn started_conversations_keep_their_agent_but_allow_model_changes(
  cx: &mut gpui::TestAppContext,
) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.items.push(user_message("started"));
    assert!(!panel.can_switch_backend());
    assert!(panel.can_switch_model());

    panel.switch_backend(AgentId::new("pi-acp"), cx);
    assert_eq!(panel.backend_kind(), &default_agent_id());
    assert_eq!(panel.current_conversation().agent_id, default_agent_id());
  });
}

#[gpui::test]
async fn model_changes_remain_available_during_a_turn(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items.push(user_message("started"));
    panel.pretend_turn_in_flight_for_test(cx);

    assert!(panel.can_switch_model());
  });
}

#[gpui::test]
async fn model_changes_during_a_turn_are_deferred_until_the_turn_settles(
  cx: &mut gpui::TestAppContext,
) {
  let (panel, cx) = add_panel_window(cx);
  let model = |id: &str, name: &str| ModelInfo::new(ModelId::new(id.to_string()), name);

  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.available_models = vec![
      model("stub-small", "Stub Small"),
      model("stub-large", "Stub Large"),
    ];
    panel.current_model_id = Some(ModelId::new("stub-small"));
    panel.items.push(user_message("started"));
    panel.pretend_turn_in_flight_for_test(cx);

    panel.set_model(ModelId::new("stub-large"), cx);
    assert!(panel.pending_model_selection.is_some());
    assert_eq!(
      panel.current_model_id.as_ref().map(|id| id.0.as_ref()),
      Some("stub-large")
    );

    panel.items.push(agent_message("done"));
    panel.complete_prompt(Ok(agent_client_protocol::schema::StopReason::EndTurn), cx);
    assert!(panel.pending_model_selection.is_none());
  });
}

#[gpui::test]
async fn pre_conversation_agent_output_does_not_lock_choices(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.pending_agent = "\n".to_string();

    assert!(panel.can_switch_backend());
    assert!(panel.can_switch_model());

    panel.switch_backend(AgentId::new("pi-acp"), cx);
    assert_eq!(panel.backend_kind(), &AgentId::new("pi-acp"));
    assert!(panel.pending_agent.is_empty());
  });
}

#[gpui::test]
async fn close_control_is_host_controlled(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  cx.run_until_parked();

  assert!(cx.debug_bounds("agent-chat-close").is_none());

  panel.update(cx, |panel, cx| panel.set_close_control_visible(true, cx));
  cx.run_until_parked();

  assert!(cx.debug_bounds("agent-chat-close").is_some());

  panel.update(cx, |panel, cx| panel.set_close_control_visible(false, cx));
  cx.run_until_parked();

  assert!(cx.debug_bounds("agent-chat-close").is_none());
}

#[gpui::test]
async fn a_missing_binary_paints_its_install_hint(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::MissingBinary {
      command: "npx".to_string(),
      hint: "Install Node.js to get npx".to_string(),
    };
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("agent-chat-missing-binary").is_some(),
    "the notice is painted"
  );
  panel.read_with(cx, |panel, _| {
    // The notice row plus the runway spacer.
    assert_eq!(panel.messages_list.item_count(), 2);
  });
}

#[gpui::test]
async fn a_loaded_conversation_renders_one_row_per_item(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![user_message("hello"), agent_message("hi there")];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  panel.read_with(cx, |panel, _| {
    // Ready and idle: no generating row, one list row per message,
    // plus the runway spacer.
    assert_eq!(panel.messages_list.item_count(), 3);
  });
}

#[gpui::test]
async fn typing_enter_without_a_session_sends_nothing(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  cx.run_until_parked();

  let input_focus = panel.read_with(cx, |panel, cx| panel.input.read(cx).focus_handle(cx));
  cx.update(|window, cx| window.focus(&input_focus, cx));
  cx.simulate_input("do the thing");
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "do the thing");
  });
  cx.simulate_keystrokes("enter");
  cx.run_until_parked();

  panel.read_with(cx, |panel, cx| {
    // Nothing was dispatched, and the composer keeps the user's text.
    assert_eq!(panel.input.read(cx).value(), "do the thing");
    assert!(panel.items.is_empty(), "no prompt was recorded");
    assert!(!panel.in_flight);
  });
}

#[gpui::test]
async fn agent_code_blocks_are_highlighted_and_copyable(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![agent_message("```rust\nfn main() { let x = 1; }\n```")];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("chat-code-block-rust").is_some(),
    "the custom code block is painted"
  );
  let copy = cx
    .debug_bounds("chat-code-copy")
    .expect("copy button painted");
  cx.simulate_click(copy.center(), gpui::Modifiers::default());
  cx.run_until_parked();

  let copied = cx
    .update(|_, cx| cx.read_from_clipboard())
    .and_then(|item| item.text());
  assert_eq!(
    copied.as_deref(),
    Some("fn main() { let x = 1; }"),
    "copy takes the code without the fences"
  );
}

fn text_chunk(text: &str) -> AgentEvent {
  AgentEvent::AgentMessageChunk(agent_client_protocol::schema::ContentChunk::new(
    ContentBlock::Text(TextContent::new(text)),
  ))
}

fn thought_chunk(text: &str) -> AgentEvent {
  AgentEvent::AgentThoughtChunk(agent_client_protocol::schema::ContentChunk::new(
    ContentBlock::Text(TextContent::new(text)),
  ))
}

fn item_kinds(items: &[ChatItem]) -> Vec<&'static str> {
  items
    .iter()
    .map(|item| match item {
      ChatItem::Message(m) if m.role == ChatRole::Agent => "agent",
      ChatItem::Message(_) => "other",
      ChatItem::Tool(_) => "tool",
      ChatItem::Thought(_) => "thought",
      ChatItem::Plan(_) => "plan",
      ChatItem::Permission(_) => "permission",
      ChatItem::Checkpoint(_) => "checkpoint",
      ChatItem::TurnSummary(_) => "turn-summary",
    })
    .collect()
}

#[gpui::test]
async fn prose_keeps_its_place_between_tool_calls(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.on_event(text_chunk("Let me read the file."), cx);
    panel.on_event(
      AgentEvent::ToolCall(call("t1", "Read foo", ToolKind::Read)),
      cx,
    );
    panel.on_event(text_chunk("Done reading."), cx);
    panel.flush_turn_buffers();
    panel.end_turn();
  });

  panel.read_with(cx, |panel, _| {
    assert_eq!(item_kinds(&panel.items), vec!["agent", "tool", "agent"]);
    let ChatItem::Message(first) = &panel.items[0] else {
      unreachable!()
    };
    assert_eq!(first.text, "Let me read the file.");
    let ChatItem::Message(last) = &panel.items[2] else {
      unreachable!()
    };
    assert_eq!(last.text, "Done reading.");
  });
}

#[gpui::test]
async fn a_thought_closes_before_prose_and_prose_before_a_tool(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.on_event(thought_chunk("weighing options"), cx);
    panel.on_event(text_chunk("Here is the plan."), cx);
    panel.on_event(
      AgentEvent::ToolCall(call("t1", "Edit foo", ToolKind::Edit)),
      cx,
    );
    panel.flush_turn_buffers();
    panel.end_turn();
  });

  panel.read_with(cx, |panel, _| {
    assert_eq!(item_kinds(&panel.items), vec!["thought", "agent", "tool"]);
  });
}

#[gpui::test]
async fn whitespace_only_prose_makes_no_message_item(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.on_event(text_chunk("\n\n"), cx);
    panel.on_event(
      AgentEvent::ToolCall(call("t1", "Read foo", ToolKind::Read)),
      cx,
    );
    panel.flush_turn_buffers();
    panel.end_turn();
  });

  panel.read_with(cx, |panel, _| {
    assert_eq!(item_kinds(&panel.items), vec!["tool"]);
  });
}

#[gpui::test]
async fn a_tool_update_does_not_split_the_following_prose(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.on_event(text_chunk("Reading."), cx);
    panel.on_event(
      AgentEvent::ToolCall(call("t1", "Read foo", ToolKind::Read)),
      cx,
    );
    panel.on_event(text_chunk("All good."), cx);
    // The same call again is an in-place update, not a new timeline entry.
    panel.on_event(
      AgentEvent::ToolCall(call("t1", "Read foo", ToolKind::Read)),
      cx,
    );
    panel.flush_turn_buffers();
    panel.end_turn();
  });

  panel.read_with(cx, |panel, _| {
    assert_eq!(item_kinds(&panel.items), vec!["agent", "tool", "agent"]);
  });
}

fn update_fields(
  fields: agent_client_protocol::schema::ToolCallUpdateFields,
) -> agent_client_protocol::schema::ToolCallUpdate {
  agent_client_protocol::schema::ToolCallUpdate::new(
    ToolCallId::new(std::sync::Arc::from("perm-tool")),
    fields,
  )
}

fn permission_item_for_subtitle(
  title: &str,
  update: ToolCallUpdate,
  detail: PermissionDetail,
) -> PermissionItem {
  PermissionItem {
    prompt: PermissionPrompt {
      id: 1,
      tool_call_title: title.into(),
      tool_call: update,
      options: Vec::new(),
    },
    detail,
    resolved: None,
    auto: false,
  }
}

#[test]
fn permission_header_subtitle_uses_the_tool_kind_for_known_requests() {
  let command_update = update_fields(
    ToolCallUpdateFields::new()
      .kind(ToolKind::Execute)
      .raw_input(serde_json::json!({ "command": "cargo test" })),
  );
  let command_detail = permission_detail(&command_update, test_cwd());
  let command_item = permission_item_for_subtitle("cargo test", command_update, command_detail);
  assert_eq!(
    permission_header_subtitle(&command_item).as_deref(),
    Some("Terminal command")
  );

  let fetch_update = update_fields(
    ToolCallUpdateFields::new()
      .kind(ToolKind::Fetch)
      .raw_input(serde_json::json!({ "url": "https://example.com" })),
  );
  let fetch_detail = permission_detail(&fetch_update, test_cwd());
  let fetch_item =
    permission_item_for_subtitle("Fetch https://example.com", fetch_update, fetch_detail);
  assert_eq!(
    permission_header_subtitle(&fetch_item).as_deref(),
    Some("Network request")
  );
}

#[test]
fn permission_header_subtitle_summarizes_file_requests() {
  let edit_update = update_fields(
    ToolCallUpdateFields::new()
      .kind(ToolKind::Edit)
      .content(vec![ToolCallContent::Diff(
        Diff::new("foo.rs", "one\ntwo\n").old_text(Some("one\n".to_string())),
      )]),
  );
  let edit_detail = permission_detail(&edit_update, test_cwd());
  let edit_item = permission_item_for_subtitle("Edit foo.rs", edit_update, edit_detail);
  assert_eq!(
    permission_header_subtitle(&edit_item).as_deref(),
    Some("File edit")
  );

  let read_update = update_fields(
    ToolCallUpdateFields::new()
      .kind(ToolKind::Read)
      .locations(vec![ToolCallLocation::new("secret.rs")]),
  );
  let read_detail = permission_detail(&read_update, test_cwd());
  let read_item = permission_item_for_subtitle("Read secret.rs", read_update, read_detail);
  assert_eq!(
    permission_header_subtitle(&read_item).as_deref(),
    Some("File read")
  );
}

#[test]
fn permission_header_subtitle_keeps_unknown_request_titles() {
  let update = update_fields(
    ToolCallUpdateFields::new()
      .kind(ToolKind::Other)
      .raw_input(serde_json::json!({ "table": "users" })),
  );
  let detail = permission_detail(&update, test_cwd());
  let item = permission_item_for_subtitle("Query users table", update, detail);
  assert_eq!(
    permission_header_subtitle(&item).as_deref(),
    Some("Query users table")
  );
}

#[test]
fn permission_detail_extracts_the_command_of_a_run() {
  let update = update_fields(
    ToolCallUpdateFields::new()
      .kind(ToolKind::Execute)
      .raw_input(serde_json::json!({ "command": "rm -rf build" })),
  );
  let detail = permission_detail(&update, test_cwd());
  assert_eq!(detail.invocation.as_deref(), Some("rm -rf build"));
  assert!(detail.diff_stats.is_empty());
}

#[test]
fn permission_detail_extracts_the_url_of_a_fetch() {
  let update = update_fields(
    ToolCallUpdateFields::new()
      .kind(ToolKind::Fetch)
      .raw_input(serde_json::json!({ "url": "https://example.com" })),
  );
  let detail = permission_detail(&update, test_cwd());
  assert_eq!(detail.invocation.as_deref(), Some("https://example.com"));
}

#[test]
fn permission_detail_summarizes_an_edit_as_diff_counts() {
  use agent_client_protocol::schema::Diff;
  let update = update_fields(
    ToolCallUpdateFields::new()
      .kind(ToolKind::Edit)
      .content(vec![ToolCallContent::Diff(
        Diff::new("foo.rs", "one\ntwo\n").old_text(Some("one\n".to_string())),
      )])
      .locations(vec![agent_client_protocol::schema::ToolCallLocation::new(
        "foo.rs",
      )]),
  );
  let detail = permission_detail(&update, test_cwd());
  assert!(detail.invocation.is_none());
  assert_eq!(detail.diff_stats.len(), 1);
  let (path, added, removed) = &detail.diff_stats[0];
  assert!(path.ends_with("foo.rs"));
  assert_eq!((*added, *removed), (1, 0));
}

#[test]
fn permission_detail_falls_back_to_pretty_json_for_unknown_tools() {
  let update = update_fields(
    ToolCallUpdateFields::new()
      .kind(ToolKind::Other)
      .raw_input(serde_json::json!({ "table": "users" })),
  );
  let detail = permission_detail(&update, test_cwd());
  let invocation = detail.invocation.expect("json fallback");
  assert!(invocation.contains("\"table\": \"users\""));
}

#[gpui::test]
async fn the_permission_card_shows_the_command_being_approved(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    let update = update_fields(
      ToolCallUpdateFields::new()
        .kind(ToolKind::Execute)
        .raw_input(serde_json::json!({ "command": "cargo build" })),
    );
    let detail = permission_detail(&update, std::path::Path::new("."));
    panel.items = vec![
      user_message("do it"),
      ChatItem::Permission(Box::new(PermissionItem {
        prompt: PermissionPrompt {
          id: 7,
          tool_call_title: "Run cargo build".into(),
          tool_call: update,
          options: vec![
            PermissionPromptOption {
              option_id: "reject".into(),
              label: "Deny".into(),
              kind: PermissionOptionKind::RejectOnce,
            },
            PermissionPromptOption {
              option_id: "allow-always".into(),
              label: "Always Allow".into(),
              kind: PermissionOptionKind::AllowAlways,
            },
            PermissionPromptOption {
              option_id: "allow".into(),
              label: "Allow Once".into(),
              kind: PermissionOptionKind::AllowOnce,
            },
          ],
        },
        detail,
        resolved: None,
        auto: false,
      })),
    ];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("perm-card").is_some(),
    "the permission card uses the review layout"
  );
  assert!(
    cx.debug_bounds("perm-header").is_some(),
    "the permission header is painted"
  );
  assert!(
    cx.debug_bounds("perm-actions").is_some(),
    "the permission actions are grouped on the card"
  );
  assert!(
    cx.debug_bounds("perm-status-pill").is_some(),
    "unanswered permissions show the awaiting approval tag"
  );
  assert!(
    cx.debug_bounds("perm-invocation").is_some(),
    "the command is painted on the card"
  );
}

#[gpui::test]
async fn the_answered_permission_card_shows_a_status_row(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    let update = update_fields(
      ToolCallUpdateFields::new()
        .kind(ToolKind::Execute)
        .raw_input(serde_json::json!({ "command": "cargo build" })),
    );
    let detail = permission_detail(&update, std::path::Path::new("."));
    panel.items = vec![ChatItem::Permission(Box::new(PermissionItem {
      prompt: PermissionPrompt {
        id: 8,
        tool_call_title: "Run cargo build".into(),
        tool_call: update,
        options: vec![PermissionPromptOption {
          option_id: "reject".into(),
          label: "Deny".into(),
          kind: PermissionOptionKind::RejectOnce,
        }],
      },
      detail,
      resolved: Some("reject".into()),
      auto: false,
    }))];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("perm-status").is_some(),
    "answered permission cards show the outcome"
  );
}

#[test]
fn thought_peek_tail_keeps_short_thoughts_whole() {
  let (tail, truncated) = thought_peek_tail("one\ntwo\nthree");
  assert_eq!(tail, "one\ntwo\nthree");
  assert!(!truncated);
}

#[test]
fn thought_peek_tail_collapses_blank_runs_and_markdown_noise() {
  let (tail, _) = thought_peek_tail("**Adding new file**\n\n\n\nnext `step`");
  assert_eq!(tail, "Adding new file\n\nnext step");
  let (leading, _) = thought_peek_tail("\n\n\nAdding new file");
  assert_eq!(leading, "Adding new file");
}

#[test]
fn thought_peek_tail_keeps_only_the_last_lines() {
  let text: String = (1..=20)
    .map(|i| format!("line {i}\n"))
    .collect::<Vec<_>>()
    .join("");
  let (tail, truncated) = thought_peek_tail(&text);
  assert!(truncated);
  assert!(tail.starts_with("line 9\n"), "tail starts at {tail:?}");
  assert!(tail.ends_with("line 20"));
  assert_eq!(tail.lines().count(), THINKING_PEEK_TAIL_LINES);
}

#[test]
fn thought_peek_tail_snaps_a_byte_cut_to_the_next_line_start() {
  // Two long lines: the byte cap lands mid line one, the peek opens on line two.
  let text = format!("{}\n{}", "a".repeat(3000), "b".repeat(3000));
  let (tail, truncated) = thought_peek_tail(&text);
  assert!(truncated);
  assert!(tail.starts_with('b'), "tail starts at {:?}", &tail[..8]);
}

#[test]
fn thought_peek_tail_respects_char_boundaries() {
  // One long line of multibyte chars: the byte cut must not split a char.
  let text = "é".repeat(THINKING_PEEK_TAIL_BYTES);
  let (tail, truncated) = thought_peek_tail(&text);
  assert!(truncated);
  assert!(tail.chars().all(|c| c == 'é'));
}

#[gpui::test]
async fn the_streaming_thought_shows_a_peek_then_settles_collapsed(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.items = vec![user_message("think about it")];
    panel.on_event(thought_chunk("first I should check the tests"), cx);
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  panel.update(cx, |panel, cx| {
    panel.mark_last_item_changed();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("agent-chat-thinking-peek").is_some(),
    "the live thought is visible while streaming"
  );
  assert!(
    cx.debug_bounds("agent-chat-thinking-fade").is_none(),
    "a short thought carries no top fade"
  );

  panel.update(cx, |panel, cx| {
    let long_think: String = (1..=40)
      .map(|i| format!("step {i}\n"))
      .collect::<Vec<_>>()
      .join("");
    panel.pending_thought = long_think;
    panel.mark_last_item_changed();
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-chat-thinking-fade").is_some(),
    "a clipped thought fades out at the top"
  );

  panel.update(cx, |panel, cx| {
    panel.flush_turn_buffers();
    panel.end_turn();
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("agent-chat-thinking-peek").is_none(),
    "the peek leaves with the turn"
  );
  panel.read_with(cx, |panel, _| {
    assert!(matches!(
      panel.items.last(),
      Some(ChatItem::Thought(t)) if t.collapsed
    ));
  });
}

#[test]
fn slash_token_only_opens_the_message() {
  assert_eq!(slash_token_at_cursor("/com", 4), Some("com".to_string()));
  assert_eq!(slash_token_at_cursor("/", 1), Some(String::new()));
  assert_eq!(
    slash_token_at_cursor("/cmd args", 4),
    Some("cmd".to_string())
  );
  // The cursor past the token is prose, not a command being typed.
  assert_eq!(slash_token_at_cursor("/cmd args", 7), None);
  // A slash after any text is a path.
  assert_eq!(slash_token_at_cursor("see /tmp", 8), None);
  assert_eq!(slash_token_at_cursor("/cmd", 0), None);
}

fn command(name: &str, description: &str) -> agent_client_protocol::schema::AvailableCommand {
  agent_client_protocol::schema::AvailableCommand::new(name, description)
}

#[gpui::test]
async fn typing_slash_opens_the_command_menu_and_enter_inserts(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.available_commands = vec![
      command("compact", "Compact the conversation"),
      command("review", "Review the changes"),
    ];
    cx.notify();
  });
  cx.run_until_parked();

  let input_focus = panel.read_with(cx, |panel, cx| panel.input.read(cx).focus_handle(cx));
  cx.update(|window, cx| window.focus(&input_focus, cx));
  cx.simulate_input("/co");
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-slash-menu").is_some(),
    "the menu opens on a leading slash"
  );

  cx.simulate_keystrokes("enter");
  cx.run_until_parked();
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "/compact ");
    assert!(panel.items.is_empty(), "accepting did not submit");
  });
  assert!(
    cx.debug_bounds("agent-slash-menu").is_none(),
    "the menu closes after accepting"
  );

  // Enter now submits the command text as a prompt; refused here (no
  // session), so the composer keeps the draft instead of re-inserting.
  cx.simulate_keystrokes("enter");
  cx.run_until_parked();
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "/compact ");
    assert!(panel.items.is_empty());
  });
}

#[gpui::test]
async fn slash_and_mention_menus_never_open_together(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.available_commands = vec![command("compact", "Compact the conversation")];
    cx.notify();
  });
  cx.run_until_parked();

  let input_focus = panel.read_with(cx, |panel, cx| panel.input.read(cx).focus_handle(cx));
  cx.update(|window, cx| window.focus(&input_focus, cx));

  cx.simulate_input("@");
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-mention-menu").is_some(),
    "@ opens the mention menu"
  );
  assert!(cx.debug_bounds("agent-slash-menu").is_none());

  panel.update_in(cx, |panel, window, cx| {
    panel.input.update(cx, |state, cx| {
      state.set_value("", window, cx);
    });
    cx.notify();
  });
  cx.simulate_input("/c");
  cx.run_until_parked();
  assert!(cx.debug_bounds("agent-slash-menu").is_some());
  assert!(
    cx.debug_bounds("agent-mention-menu").is_none(),
    "a leading slash is not a mention"
  );
}

#[gpui::test]
async fn escaping_the_slash_menu_dismisses_until_the_token_changes(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.available_commands = vec![command("compact", "Compact the conversation")];
    cx.notify();
  });
  cx.run_until_parked();

  let input_focus = panel.read_with(cx, |panel, cx| panel.input.read(cx).focus_handle(cx));
  cx.update(|window, cx| window.focus(&input_focus, cx));
  cx.simulate_input("/co");
  cx.run_until_parked();
  assert!(cx.debug_bounds("agent-slash-menu").is_some());

  cx.simulate_keystrokes("escape");
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-slash-menu").is_none(),
    "escape closes the menu"
  );

  cx.simulate_input("m");
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-slash-menu").is_some(),
    "a changed token reopens the menu"
  );
}

#[gpui::test]
async fn accepting_a_command_keeps_the_typed_arguments(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update_in(cx, |panel, window, cx| {
    panel.status = Status::Ready;
    panel.available_commands = vec![command("compact", "Compact the conversation")];
    panel.input.update(cx, |state, cx| {
      state.set_value("/co keep this", window, cx);
    });
    panel.insert_slash_command("compact", window, cx);
  });
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "/compact keep this");
  });
}

#[gpui::test]
async fn a_slash_menu_never_opens_mid_text_or_without_matches(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.available_commands = vec![command("compact", "Compact the conversation")];
    cx.notify();
  });
  cx.run_until_parked();

  let input_focus = panel.read_with(cx, |panel, cx| panel.input.read(cx).focus_handle(cx));
  cx.update(|window, cx| window.focus(&input_focus, cx));
  cx.simulate_input("see /tmp");
  cx.run_until_parked();
  assert!(cx.debug_bounds("agent-slash-menu").is_none());

  panel.update_in(cx, |panel, window, cx| {
    panel.input.update(cx, |state, cx| {
      state.set_value("/nomatch", window, cx);
    });
    cx.notify();
  });
  cx.run_until_parked();
  assert!(cx.debug_bounds("agent-slash-menu").is_none());
}

fn checkpoint_item(ref_name: &str) -> ChatItem {
  ChatItem::Checkpoint(CheckpointMarker {
    ref_name: ref_name.to_string(),
    created_at_secs: 0,
  })
}

#[test]
fn checkpoint_ref_before_finds_the_guarding_marker() {
  let items = vec![
    checkpoint_item("cp-1"),
    user_message("first"),
    agent_message("reply"),
    checkpoint_item("cp-2"),
    user_message("second"),
  ];
  assert_eq!(checkpoint_ref_before(&items, 1), Some("cp-1".to_string()));
  assert_eq!(checkpoint_ref_before(&items, 4), Some("cp-2".to_string()));
  // A prompt with another prompt in between has no guarding marker.
  let unguarded = vec![
    checkpoint_item("cp-1"),
    user_message("first"),
    user_message("second"),
  ];
  assert_eq!(checkpoint_ref_before(&unguarded, 2), None);
  assert_eq!(checkpoint_ref_before(&unguarded, 0), None);
}

#[test]
fn conversation_ids_are_unique_within_the_same_instant() {
  let a = crate::persistence::unique_conversation_id();
  let b = crate::persistence::unique_conversation_id();
  assert_ne!(a, b);
}

#[gpui::test]
async fn late_chunks_after_a_turn_are_kept_not_dropped(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = false;
    panel.on_event(text_chunk("trailing words"), cx);
    // The next boundary (here the next turn's flush) surfaces them in order.
    panel.flush_turn_buffers();
  });
  panel.read_with(cx, |panel, _| {
    assert_eq!(item_kinds(&panel.items), vec!["agent"]);
    let ChatItem::Message(m) = &panel.items[0] else {
      unreachable!()
    };
    assert_eq!(m.text, "trailing words");
  });
}

#[gpui::test]
async fn a_plan_updates_in_place_across_interleaved_tool_calls(cx: &mut gpui::TestAppContext) {
  use agent_client_protocol::schema::{Plan as AcpPlan, PlanEntry, PlanEntryStatus};
  let plan_with = |text: &str, status: PlanEntryStatus| {
    AcpPlan::new(vec![PlanEntry::new(
      text.to_string(),
      agent_client_protocol::schema::PlanEntryPriority::Medium,
      status,
    )])
  };
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.on_event(
      AgentEvent::Plan(plan_with("step", PlanEntryStatus::Pending)),
      cx,
    );
    panel.on_event(
      AgentEvent::ToolCall(call("t1", "Read foo", ToolKind::Read)),
      cx,
    );
    panel.on_event(
      AgentEvent::Plan(plan_with("step", PlanEntryStatus::Completed)),
      cx,
    );
    panel.flush_turn_buffers();
    panel.end_turn();
  });
  panel.read_with(cx, |panel, _| {
    assert_eq!(
      item_kinds(&panel.items),
      vec!["plan", "tool"],
      "the interleaved tool call must not duplicate the plan"
    );
  });

  // A plan in a new turn starts its own block.
  panel.update(cx, |panel, cx| {
    panel.items.push(user_message("next"));
    panel.on_event(
      AgentEvent::Plan(plan_with("other", PlanEntryStatus::Pending)),
      cx,
    );
  });
  panel.read_with(cx, |panel, _| {
    assert_eq!(
      item_kinds(&panel.items),
      vec!["plan", "tool", "other", "plan"]
    );
  });
}

#[gpui::test]
async fn permissions_survive_a_reload_with_dead_buttons(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-perm-persist");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    let update = update_fields(
      ToolCallUpdateFields::new()
        .kind(ToolKind::Execute)
        .raw_input(serde_json::json!({ "command": "rm -rf build" })),
    );
    let detail = permission_detail(&update, test_cwd());
    panel.items = vec![
      user_message("do it"),
      ChatItem::Permission(Box::new(PermissionItem {
        prompt: PermissionPrompt {
          id: 3,
          tool_call_title: "Run rm".into(),
          tool_call: update,
          options: vec![],
        },
        detail,
        resolved: None,
        auto: false,
      })),
    ];
    panel.persist_state(cx);
    cx.notify();
  });
  cx.run_until_parked();

  let conv_id = panel.read_with(cx, |panel, _| panel.current_conv.id.clone());
  let (_, items, _, _, _) =
    load_conversation_file(&dir.join(format!("{conv_id}.json"))).expect("reloads");
  let ChatItem::Permission(p) = &items[1] else {
    panic!("permission persisted, got {:?}", item_kinds(&items));
  };
  assert_eq!(
    p.detail.invocation.as_deref(),
    Some("rm -rf build"),
    "the approved command survives the reload"
  );
  assert_eq!(
    p.resolved.as_deref(),
    Some("unanswered"),
    "a pending card cannot offer live buttons after a reload"
  );

  // An already answered card keeps its answer as-is.
  panel.update(cx, |panel, cx| {
    if let Some(ChatItem::Permission(p)) = panel.items.get_mut(1) {
      p.resolved = Some("allow".to_string());
    }
    panel.persist_state(cx);
    cx.notify();
  });
  cx.run_until_parked();
  let (_, items, _, _, _) =
    load_conversation_file(&dir.join(format!("{conv_id}.json"))).expect("reloads");
  let ChatItem::Permission(p) = &items[1] else {
    unreachable!()
  };
  assert_eq!(p.resolved.as_deref(), Some("allow"));
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn the_copy_button_copies_the_message_markdown(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![user_message("copy **me** please")];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  let copy = cx.debug_bounds("chat-msg-copy").expect("copy painted");
  cx.simulate_click(copy.center(), gpui::Modifiers::default());
  cx.run_until_parked();

  let copied = cx
    .update(|_, cx| cx.read_from_clipboard())
    .and_then(|item| item.text());
  assert_eq!(copied.as_deref(), Some("copy **me** please"));
}

#[gpui::test]
async fn the_tool_command_copy_button_copies_the_command(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![ChatItem::Tool(tool_view(
      "t1",
      ToolKind::Execute,
      ToolCallStatus::Completed,
    ))];
    if let Some(ChatItem::Tool(tool)) = panel.items.last_mut() {
      tool.title = "Run rg needle src".to_string();
    }
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  let copy = cx
    .debug_bounds("agent-tool-copy-command")
    .expect("copy painted");
  cx.simulate_click(copy.center(), gpui::Modifiers::default());
  cx.run_until_parked();

  let copied = cx
    .update(|_, cx| cx.read_from_clipboard())
    .and_then(|item| item.text());
  assert_eq!(copied.as_deref(), Some("rg needle src"));
}

#[gpui::test]
async fn editing_a_prompt_arms_the_rewind_and_truncate_resubmits(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![
      checkpoint_item("cp-1"),
      user_message("old prompt"),
      agent_message("old answer"),
    ];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("chat-msg-edit").is_some(),
    "a guarded prompt offers editing"
  );

  panel.update_in(cx, |panel, window, cx| {
    panel.begin_message_edit(1, window, cx);
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-chat-edit-message").is_some(),
    "the bubble becomes an inline editor"
  );

  panel.update_in(cx, |panel, window, cx| {
    let input = panel.edit_input.clone().expect("edit editor");
    input.update(cx, |state, cx| state.set_value("new prompt", window, cx));
    panel.submit_message_edit(cx);
  });
  panel.read_with(cx, |panel, _| {
    assert_eq!(
      panel.pending_edit_resubmit,
      Some(("cp-1".to_string(), "new prompt".to_string()))
    );
    assert!(panel.editing_message.is_none());
  });

  // The shell restores the worktree then calls truncate; the panel arms the
  // resubmit for the fresh session and drops the edited turn.
  panel.update(cx, |panel, cx| {
    assert!(panel.truncate_at_checkpoint("cp-1", cx));
  });
  panel.read_with(cx, |panel, _| {
    assert_eq!(panel.resubmit_after_connect, Some("new prompt".to_string()));
    // The marker itself survives: the resubmitted prompt stays guarded by it.
    assert_eq!(item_kinds(&panel.items), vec!["checkpoint"]);
  });
}

#[gpui::test]
async fn editing_is_refused_mid_turn_and_cancel_restores_the_bubble(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.items = vec![checkpoint_item("cp-1"), user_message("prompt")];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("chat-msg-edit").is_none(),
    "no edit button while a turn runs"
  );
  panel.update_in(cx, |panel, window, cx| {
    panel.begin_message_edit(1, window, cx);
    assert!(panel.editing_message.is_none(), "editing refused mid-turn");
    panel.in_flight = false;
    panel.begin_message_edit(1, window, cx);
    assert_eq!(panel.editing_message, Some(1));
    panel.cancel_message_edit(cx);
    assert!(panel.editing_message.is_none());
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("agent-chat-edit-message").is_none(),
    "cancel restores the bubble"
  );

  // A prompt without a guarding checkpoint offers no edit either.
  panel.update(cx, |panel, cx| {
    panel.items = vec![user_message("unguarded")];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  assert!(cx.debug_bounds("chat-msg-edit").is_none());
}

#[gpui::test]
async fn escape_cancels_the_message_edit_and_refocuses_the_composer(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![checkpoint_item("cp-1"), user_message("prompt")];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  panel.update_in(cx, |panel, window, cx| {
    panel.begin_message_edit(1, window, cx);
  });
  cx.run_until_parked();
  assert!(cx.debug_bounds("agent-chat-edit-message").is_some());

  cx.simulate_keystrokes("escape");
  cx.run_until_parked();

  panel.read_with(cx, |panel, _| assert!(panel.editing_message.is_none()));
  assert!(
    cx.debug_bounds("agent-chat-edit-message").is_none(),
    "escape restores the reading bubble"
  );
  panel.update_in(cx, |panel, window, cx| {
    assert!(
      panel.input.read(cx).focus_handle(cx).is_focused(window),
      "escape hands focus back to the composer"
    );
  });
}

#[gpui::test]
async fn enter_submits_the_message_edit_and_shift_enter_stays_a_newline(
  cx: &mut gpui::TestAppContext,
) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![checkpoint_item("cp-1"), user_message("old prompt")];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  panel.update_in(cx, |panel, window, cx| {
    panel.begin_message_edit(1, window, cx);
  });
  cx.run_until_parked();

  cx.simulate_keystrokes("shift-enter");
  cx.run_until_parked();
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.editing_message, Some(1), "shift-enter keeps editing");
    let value = panel
      .edit_input
      .as_ref()
      .expect("edit editor")
      .read(cx)
      .value()
      .to_string();
    assert!(
      value.contains('\n'),
      "shift-enter inserts a newline: {value:?}"
    );
  });

  panel.update_in(cx, |panel, window, cx| {
    let input = panel.edit_input.clone().expect("edit editor");
    input.update(cx, |state, cx| state.set_value("new prompt", window, cx));
  });
  cx.simulate_keystrokes("enter");
  cx.run_until_parked();
  panel.read_with(cx, |panel, cx| {
    assert_eq!(
      panel.pending_edit_resubmit,
      Some(("cp-1".to_string(), "new prompt".to_string())),
      "enter submits the edit"
    );
    assert!(panel.editing_message.is_none());
    let composer = panel.input.read(cx).value().to_string();
    assert!(
      composer.is_empty(),
      "no newline leaked anywhere: {composer:?}"
    );
  });
}

#[gpui::test]
async fn the_copy_button_works_on_agent_messages_too(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![agent_message("the agent said this")];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  let copy = cx
    .debug_bounds("chat-msg-copy-agent")
    .expect("agent copy painted");
  cx.simulate_click(copy.center(), gpui::Modifiers::default());
  cx.run_until_parked();

  let copied = cx
    .update(|_, cx| cx.read_from_clipboard())
    .and_then(|item| item.text());
  assert_eq!(copied.as_deref(), Some("the agent said this"));
}

#[gpui::test]
async fn the_rollback_pill_sleeps_while_a_turn_runs(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.items = vec![
      checkpoint_item("cp-1"),
      user_message("prompt"),
      agent_message("reply"),
    ];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("chat-checkpoint-rollback").is_none(),
    "no clickable rollback during a turn"
  );

  panel.update(cx, |panel, cx| {
    panel.end_turn();
    let count = panel.messages_list.item_count();
    panel.messages_list.remeasure_items(0..count);
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("chat-checkpoint-rollback").is_some(),
    "the pill wakes up once the turn settles"
  );
}

#[gpui::test]
async fn a_rollback_truncate_repaints_without_the_agent_reply(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![
      checkpoint_item("cp-1"),
      user_message("do the thing"),
      agent_message("here is my long reply"),
    ];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("chat-msg-copy-agent").is_some(),
    "the agent reply is painted before the rollback"
  );

  panel.update(cx, |panel, cx| {
    assert!(panel.truncate_at_checkpoint("cp-1", cx));
  });
  cx.run_until_parked();

  panel.read_with(cx, |panel, _| {
    assert_eq!(item_kinds(&panel.items), vec!["checkpoint"]);
  });
  assert!(
    cx.debug_bounds("chat-msg-copy-agent").is_none(),
    "the agent reply is gone from the very next paint"
  );
}

#[gpui::test]
async fn a_truncate_for_another_checkpoint_never_resubmits(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![
      checkpoint_item("cp-1"),
      user_message("one"),
      checkpoint_item("cp-2"),
      user_message("two"),
    ];
    panel.pending_edit_resubmit = Some(("cp-2".to_string(), "edited".to_string()));
    assert!(panel.truncate_at_checkpoint("cp-1", cx));
  });
  panel.read_with(cx, |panel, _| {
    assert!(panel.resubmit_after_connect.is_none());
    assert!(
      panel.pending_edit_resubmit.is_none(),
      "a mismatched truncate disarms the edit instead of leaving it live"
    );
  });
}

#[test]
fn tool_headers_never_stutter_the_kind() {
  let mut view = tool_view("t", ToolKind::Edit, ToolCallStatus::Completed);
  // The title carries the verb: the bold label steps aside.
  view.title = "Editing files".to_string();
  assert_eq!(
    tool_header_parts(&view),
    (None, "Editing files".to_string())
  );
  view.title = "Edit src/main.rs".to_string();
  assert_eq!(
    tool_header_parts(&view),
    (Some("Edit"), "src/main.rs".to_string())
  );
  view.title = "Edit".to_string();
  assert_eq!(tool_header_parts(&view), (Some("Edit"), String::new()));
  view.title = "Reformat everything".to_string();
  assert_eq!(
    tool_header_parts(&view),
    (Some("Edit"), "Reformat everything".to_string())
  );
}

#[test]
fn read_tools_number_outputs_from_their_location() {
  let locations = vec![(PathBuf::from("src/main.rs"), Some(42))];

  assert_eq!(
    read_tool_output_start_line(&ToolKind::Read, &locations),
    Some(42)
  );
  assert_eq!(
    read_tool_output_start_line(&ToolKind::Execute, &locations),
    None
  );
}

#[test]
fn read_tools_without_line_start_at_one() {
  let locations = vec![(PathBuf::from("src/main.rs"), None)];

  assert_eq!(
    read_tool_output_start_line(&ToolKind::Read, &locations),
    Some(1)
  );
}

#[test]
fn visible_output_line_ranges_preserve_blank_lines() {
  let ranges = visible_output_line_ranges("first\n\nthird\n", 3);

  assert_eq!(ranges, vec![0..5, 6..6, 7..12]);
}

#[test]
fn syntax_spans_for_range_skips_spans_before_the_visible_line() {
  let spans = vec![
    HighlightSpan {
      byte_range: 0..3,
      token_type: syntax::TokenType::Keyword,
    },
    HighlightSpan {
      byte_range: 6..11,
      token_type: syntax::TokenType::String,
    },
  ];

  let clipped = syntax_spans_for_range(&spans, 5..10);

  assert_eq!(clipped.len(), 1);
  assert_eq!(clipped[0].byte_range, 1..5);
  assert_eq!(clipped[0].token_type, syntax::TokenType::String);
}

#[test]
fn mini_diff_blank_lines_keep_text_height() {
  assert_eq!(mini_diff_line_text_for_layout("").as_ref(), " ");
  assert_eq!(
    mini_diff_line_text_for_layout("let x = 1;").as_ref(),
    "let x = 1;"
  );
}

#[test]
fn old_conversations_load_with_zero_images() {
  let message: ChatMessage =
    serde_json::from_str(r#"{"role":"User","text":"hi"}"#).expect("legacy message loads");
  assert_eq!(message.images, 0);
}

#[test]
fn tool_group_span_rides_over_thoughts_and_needs_two_tools() {
  let thought = || {
    ChatItem::Thought(ThoughtView {
      text: "hmm".into(),
      collapsed: true,
    })
  };
  let items = vec![
    user_message("go"),
    ChatItem::Tool(tool_view("a", ToolKind::Read, ToolCallStatus::Completed)),
    thought(),
    ChatItem::Tool(tool_view("b", ToolKind::Execute, ToolCallStatus::Completed)),
    agent_message("done"),
    ChatItem::Tool(tool_view("d", ToolKind::Read, ToolCallStatus::Completed)),
    thought(),
  ];
  // Thoughts between calls stay inside the span.
  assert_eq!(tool_group_span(&items, 1), Some((1, 3, 2)));
  assert_eq!(tool_group_span(&items, 2), Some((1, 3, 2)));
  assert_eq!(tool_group_span(&items, 3), Some((1, 3, 2)));
  // A single tool, even with a thought, is no group.
  assert_eq!(tool_group_span(&items, 5), None);
  assert_eq!(tool_group_span(&items, 6), None);
  // Prose is a hard boundary.
  assert_eq!(tool_group_span(&items, 4), None);
}

#[test]
fn tool_group_summary_counts_by_kind_and_failures() {
  let a = tool_view("a", ToolKind::Execute, ToolCallStatus::Completed);
  let b = tool_view("b", ToolKind::Execute, ToolCallStatus::Failed);
  let c = tool_view("c", ToolKind::Edit, ToolCallStatus::Completed);
  let d = tool_view("d", ToolKind::Read, ToolCallStatus::Completed);
  assert_eq!(
    tool_group_summary(&[&a, &b, &c, &d]),
    "Ran 2 commands · Edited 1 file · Read 1 file · 1 failed"
  );
  assert_eq!(tool_group_summary(&[&c]), "Edited 1 file");
}

fn tool_view(id: &str, kind: ToolKind, status: ToolCallStatus) -> ToolCallView {
  let mut items: Vec<ChatItem> = Vec::new();
  let mut index = HashMap::new();
  upsert_tool_call_pure(&mut items, &mut index, call(id, id, kind), test_cwd());
  let Some(ChatItem::Tool(mut view)) = items.pop() else {
    unreachable!()
  };
  view.status = status;
  view
}

#[gpui::test]
async fn streaming_chunks_feed_the_incremental_markdown_state(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.on_event(text_chunk("Hello, "), cx);
    panel.on_event(text_chunk("**world**."), cx);
    assert!(
      panel.pending_md_state.is_some(),
      "the incremental parser state exists while streaming"
    );
    assert_eq!(panel.pending_agent, "Hello, **world**.");

    panel.flush_turn_buffers();
    assert!(
      panel.pending_md_state.is_none(),
      "flushing retires the streaming state"
    );
  });
}

#[gpui::test]
async fn a_resent_tool_call_with_identical_content_keeps_the_expansion(
  cx: &mut gpui::TestAppContext,
) {
  use agent_client_protocol::schema::Diff;
  let (panel, cx) = add_panel_window(cx);
  let call_with_diff = || {
    let mut c = call("t1", "Edit foo", ToolKind::Edit);
    c.content = vec![ToolCallContent::Diff(
      Diff::new("foo.rs", "one\ntwo\n").old_text(Some("one\n".to_string())),
    )];
    c
  };
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.on_event(AgentEvent::ToolCall(call_with_diff()), cx);
    if let Some(ChatItem::Tool(t)) = panel.items.last_mut() {
      t.diffs[0].expanded = true;
    }
    // The same call again: identical content must not rebuild the diffs.
    panel.on_event(AgentEvent::ToolCall(call_with_diff()), cx);
  });
  panel.read_with(cx, |panel, _| {
    let Some(ChatItem::Tool(t)) = panel.items.last() else {
      panic!("tool item");
    };
    assert!(t.diffs[0].expanded, "identical content kept the view state");
  });
}

#[gpui::test]
async fn changed_content_or_location_invalidates_the_tool_cache(cx: &mut gpui::TestAppContext) {
  use agent_client_protocol::schema::{Diff, ToolCallLocation};
  let (panel, cx) = add_panel_window(cx);
  let call_with = |new_text: &str, path: &str| {
    let mut c = call("t1", "Edit foo", ToolKind::Edit);
    c.content = vec![ToolCallContent::Diff(
      Diff::new("foo.rs", new_text.to_string()).old_text(Some("one\n".to_string())),
    )];
    c.locations = vec![ToolCallLocation::new(path)];
    c
  };
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.on_event(AgentEvent::ToolCall(call_with("one\ntwo\n", "foo.rs")), cx);
    let added_before = match panel.items.last() {
      Some(ChatItem::Tool(t)) => t.diffs[0].added,
      _ => panic!("tool item"),
    };
    assert_eq!(added_before, 1);

    // New content: the diff recomputes.
    panel.on_event(
      AgentEvent::ToolCall(call_with("one\ntwo\nthree\n", "foo.rs")),
      cx,
    );
    match panel.items.last() {
      Some(ChatItem::Tool(t)) => assert_eq!(t.diffs[0].added, 2, "fresh content recomputed"),
      _ => panic!("tool item"),
    }

    // Same content, moved file: the language changes, so the cache must miss.
    let fp_before = match panel.items.last() {
      Some(ChatItem::Tool(t)) => t.content_fp,
      _ => panic!("tool item"),
    };
    panel.on_event(
      AgentEvent::ToolCall(call_with("one\ntwo\nthree\n", "bar.py")),
      cx,
    );
    match panel.items.last() {
      Some(ChatItem::Tool(t)) => {
        assert_ne!(
          t.content_fp, fp_before,
          "a moved location changes the fingerprint"
        );
      }
      _ => panic!("tool item"),
    }
  });
}

#[gpui::test]
async fn the_stateful_streaming_view_renders_custom_code_blocks(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.items = vec![user_message("show me code")];
    panel.on_event(text_chunk("```toml\nkey = 1\n```"), cx);
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  panel.update(cx, |panel, cx| {
    panel.mark_last_item_changed();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("chat-code-block-toml").is_some(),
    "the incremental streaming view carries the custom block extensions"
  );
}

#[gpui::test]
async fn a_status_only_update_skips_the_rehighlight(cx: &mut gpui::TestAppContext) {
  use agent_client_protocol::schema::ToolCallUpdateFields;
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    let mut c = call("t1", "Run ls", ToolKind::Execute);
    c.content = vec![ToolCallContent::from(ContentBlock::Text(TextContent::new(
      "some output",
    )))];
    panel.on_event(AgentEvent::ToolCall(c), cx);
    // Sentinel: recomputing spans would wipe it.
    if let Some(ChatItem::Tool(t)) = panel.items.last_mut() {
      t.outputs[0].syntax_spans = vec![HighlightSpan {
        byte_range: 0..1,
        token_type: syntax::TokenType::Keyword,
      }];
    }
    panel.on_event(
      AgentEvent::ToolCallUpdate(agent_client_protocol::schema::ToolCallUpdate::new(
        ToolCallId::new(std::sync::Arc::from("t1")),
        ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
      )),
      cx,
    );
  });
  panel.read_with(cx, |panel, _| {
    let Some(ChatItem::Tool(t)) = panel.items.last() else {
      panic!("tool item");
    };
    assert_eq!(t.status, ToolCallStatus::Completed);
    assert_eq!(
      t.outputs[0].syntax_spans.len(),
      1,
      "a status flip must not recompute highlights"
    );
  });
}

#[gpui::test]
async fn tool_runs_fold_once_the_turn_settles_and_pin_on_click(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.items = vec![user_message("go")];
    panel.on_event(
      AgentEvent::ToolCall(call("t1", "Read a", ToolKind::Read)),
      cx,
    );
    panel.on_event(
      AgentEvent::ToolCall(call("t2", "Run b", ToolKind::Execute)),
      cx,
    );
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("agent-tool-group").is_some(),
    "a run of two tools grows a group header"
  );
  assert!(
    cx.debug_bounds("agent-tool-card").is_some(),
    "the trailing run streams expanded"
  );

  panel.update(cx, |panel, cx| {
    panel.end_turn();
    let count = panel.messages_list.item_count();
    panel.messages_list.remeasure_items(0..count);
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-tool-card").is_none(),
    "the settled turn folds the group"
  );

  let header = cx.debug_bounds("agent-tool-group").expect("header painted");
  cx.simulate_click(header.center(), gpui::Modifiers::default());
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-tool-card").is_some(),
    "clicking pins the group open"
  );

  let header = cx.debug_bounds("agent-tool-group").expect("header painted");
  cx.simulate_click(header.center(), gpui::Modifiers::default());
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-tool-card").is_none(),
    "clicking again pins it closed"
  );
}

#[gpui::test]
async fn a_pin_beats_streaming_and_prose_ends_the_trailing_run(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.items = vec![user_message("go")];
    panel.on_event(
      AgentEvent::ToolCall(call("t1", "Read a", ToolKind::Read)),
      cx,
    );
    panel.on_event(
      AgentEvent::ToolCall(call("t2", "Read b", ToolKind::Read)),
      cx,
    );
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  assert!(cx.debug_bounds("agent-tool-card").is_some());

  // Folding while the agent still streams into the group wins over the
  // trailing default.
  let header = cx.debug_bounds("agent-tool-group").expect("header painted");
  cx.simulate_click(header.center(), gpui::Modifiers::default());
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-tool-card").is_none(),
    "the pin folds the group mid-stream"
  );

  // Narration then another tool: the old group is no longer trailing, the
  // new single tool renders as its own plain row.
  panel.update(cx, |panel, cx| {
    panel.on_event(text_chunk("checking something"), cx);
    panel.on_event(
      AgentEvent::ToolCall(call("t3", "Run c", ToolKind::Execute)),
      cx,
    );
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-tool-card").is_some(),
    "the new single tool paints its plain card"
  );
  assert!(
    cx.debug_bounds("agent-tool-group").is_some(),
    "the folded group keeps its summary header"
  );
}

#[gpui::test]
async fn a_single_tool_call_keeps_its_plain_row(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![
      user_message("go"),
      ChatItem::Tool(tool_view("solo", ToolKind::Read, ToolCallStatus::Completed)),
    ];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(cx.debug_bounds("agent-tool-group").is_none());
  assert!(cx.debug_bounds("agent-tool-card").is_some());
}

#[gpui::test]
async fn group_pins_survive_a_reload(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-group-pins");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.store = Some(cx.new(|_| crate::store::ConversationStore::new(dir.clone())));
    panel.items = vec![
      user_message("go"),
      ChatItem::Tool(tool_view("t1", ToolKind::Read, ToolCallStatus::Completed)),
      ChatItem::Tool(tool_view("t2", ToolKind::Read, ToolCallStatus::Completed)),
    ];
    panel.sync_list_count();
    panel.toggle_tool_group(1, cx);
  });
  cx.run_until_parked();
  let conv_id = panel.read_with(cx, |panel, _| panel.current_conv.id.clone());
  let (_, _, _, pins, _) =
    load_conversation_file(&dir.join(format!("{conv_id}.json"))).expect("reloads");
  assert_eq!(pins.len(), 1);
  assert!(
    pins.values().all(|&expanded| expanded),
    "the settled group was folded, so the click pinned it open"
  );
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn image_formats_are_detected_by_extension() {
  use std::path::Path;
  assert!(image_format_for_path(Path::new("shot.PNG")).is_some());
  assert!(image_format_for_path(Path::new("photo.jpeg")).is_some());
  assert!(image_format_for_path(Path::new("main.rs")).is_none());
  assert!(image_format_for_path(Path::new("noext")).is_none());
}

#[gpui::test]
async fn staged_images_render_and_attach_only_when_supported(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  let png = gpui::Image::from_bytes(gpui::ImageFormat::Png, vec![1, 2, 3]);

  // Without the capability, staging is refused.
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.stage_image(png.clone(), cx);
    assert_eq!(panel.staged_images.len(), 0);

    panel.supports_images = true;
    panel.stage_image(png.clone(), cx);
    panel.stage_image(png, cx);
    assert_eq!(panel.staged_images.len(), 2);
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-chat-attachments").is_some(),
    "the thumbnail strip is painted"
  );

  panel.update(cx, |panel, cx| {
    panel.remove_staged_image(0, cx);
    assert_eq!(panel.staged_images.len(), 1);
  });
}

#[gpui::test]
async fn pasting_an_image_stages_it_and_text_still_pastes(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.supports_images = true;
    cx.notify();
  });
  cx.run_until_parked();

  let input_focus = panel.read_with(cx, |panel, cx| panel.input.read(cx).focus_handle(cx));
  cx.update(|window, cx| window.focus(&input_focus, cx));

  #[cfg(target_os = "macos")]
  let paste_keystroke = "cmd-v";
  #[cfg(not(target_os = "macos"))]
  let paste_keystroke = "ctrl-v";

  let image = gpui::Image::from_bytes(gpui::ImageFormat::Png, vec![9, 9, 9]);
  cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image)));
  cx.simulate_keystrokes(paste_keystroke);
  cx.run_until_parked();
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.staged_images.len(), 1, "the image staged");
    assert_eq!(
      panel.input.read(cx).value(),
      "",
      "nothing was typed into the composer"
    );
  });

  cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("plain words".into())));
  cx.simulate_keystrokes(paste_keystroke);
  cx.run_until_parked();
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "plain words");
    assert_eq!(panel.staged_images.len(), 1, "text pastes stage nothing");
  });
}

#[gpui::test]
async fn a_refused_send_keeps_the_staged_images(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update_in(cx, |panel, window, cx| {
    panel.status = Status::Ready;
    panel.supports_images = true;
    panel.stage_image(gpui::Image::from_bytes(gpui::ImageFormat::Png, vec![1]), cx);
    panel.input.update(cx, |state, cx| {
      state.set_value("send me", window, cx);
    });
    // No session: the dispatch is refused before the staging is drained.
    panel.submit(window, cx);
    assert_eq!(panel.staged_images.len(), 1);
  });
}

#[gpui::test]
async fn dropped_paths_stage_images_and_mention_other_files(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-drop");
  let image_path = dir.join("shot.png");
  std::fs::write(&image_path, [137, 80, 78, 71]).unwrap();
  let (panel, cx) = add_panel_window(cx);
  panel.update_in(cx, |panel, window, cx| {
    panel.status = Status::Ready;
    panel.supports_images = true;
    panel.cwd = dir.clone();
    let inside = dir.join("src/lib.rs");
    let outside = PathBuf::from("/somewhere/else/notes.txt");
    panel.handle_dropped_paths(&[image_path.clone(), inside, outside], window, cx);
    assert_eq!(panel.staged_images.len(), 1, "the png staged as an image");
    assert_eq!(
      panel.input.read(cx).value().trim(),
      "@src/lib.rs /somewhere/else/notes.txt",
      "a repo file lands as a mention token, a foreign file as its path"
    );
  });
  std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
async fn typing_enter_mid_turn_queues_the_message(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    cx.notify();
  });
  cx.run_until_parked();

  let input_focus = panel.read_with(cx, |panel, cx| panel.input.read(cx).focus_handle(cx));
  cx.update(|window, cx| window.focus(&input_focus, cx));
  cx.simulate_input("do this next");
  cx.simulate_keystrokes("enter");
  cx.run_until_parked();

  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.queued_prompts, vec!["do this next".to_string()]);
    assert_eq!(panel.input.read(cx).value(), "", "the composer drained");
    assert!(
      panel.items.is_empty(),
      "nothing was dispatched while the turn runs"
    );
  });
  assert!(
    cx.debug_bounds("agent-chat-queued").is_some(),
    "the queued card is painted above the composer"
  );
}

#[gpui::test]
async fn arrow_keys_browse_sent_prompt_history(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update_in(cx, |panel, window, cx| {
    panel.status = Status::Ready;
    panel.items = vec![
      user_message("first prompt"),
      agent_message("first answer"),
      user_message("second prompt"),
    ];
    panel.sync_list_count();
    panel.set_composer_value("draft prompt", window, cx);
    window.focus(&panel.input.read(cx).focus_handle(cx), cx);
    cx.notify();
  });
  cx.run_until_parked();

  cx.simulate_keystrokes("up");
  cx.run_until_parked();
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "second prompt");
  });

  cx.simulate_keystrokes("up");
  cx.run_until_parked();
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "first prompt");
  });

  cx.simulate_keystrokes("down");
  cx.run_until_parked();
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "second prompt");
  });

  cx.simulate_keystrokes("down");
  cx.run_until_parked();
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "draft prompt");
  });
}

#[gpui::test]
async fn arrow_up_inside_multiline_composer_moves_the_cursor(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update_in(cx, |panel, window, cx| {
    panel.status = Status::Ready;
    panel.items = vec![user_message("history prompt")];
    panel.sync_list_count();
    panel.set_composer_value("top\nbottom", window, cx);
    window.focus(&panel.input.read(cx).focus_handle(cx), cx);
    cx.notify();
  });
  cx.run_until_parked();

  cx.simulate_keystrokes("up");
  cx.run_until_parked();

  panel.read_with(cx, |panel, cx| {
    let input = panel.input.read(cx);
    assert_eq!(input.value(), "top\nbottom");
    assert_eq!(input.cursor_position(cx).line, 0);
  });
}

#[gpui::test]
async fn queued_messages_stack_behind_the_composer(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.queued_prompts = vec!["first".into(), "second".into(), "third".into()];
    cx.notify();
  });
  cx.run_until_parked();

  let composer = cx
    .debug_bounds("agent-chat-composer")
    .expect("composer is painted");
  let first = cx
    .debug_bounds("agent-chat-queued-card-0")
    .expect("next queued prompt is painted");
  let second = cx
    .debug_bounds("agent-chat-queued-card-1")
    .expect("second queued prompt is painted");
  let third = cx
    .debug_bounds("agent-chat-queued-card-2")
    .expect("third queued prompt is painted");

  assert!(
    f32::from(third.top()) < f32::from(second.top())
      && f32::from(second.top()) < f32::from(first.top())
      && f32::from(first.top()) < f32::from(composer.top()),
    "queued prompts stack above the composer in reverse send order"
  );
  assert!(
    f32::from(first.bottom()) > f32::from(composer.top()),
    "the next queued prompt tucks behind the composer"
  );
  assert!(
    f32::from(composer.left()) < f32::from(first.left())
      && f32::from(first.left()) < f32::from(second.left())
      && f32::from(second.left()) < f32::from(third.left()),
    "older queued prompts are progressively inset"
  );
}

#[gpui::test]
async fn editing_a_queued_message_swaps_it_with_the_draft(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update_in(cx, |panel, window, cx| {
    panel.status = Status::Ready;
    panel.queued_prompts = vec!["first".into(), "second".into()];
    // Empty draft: popping removes the row.
    panel.pop_queued_to_composer(0, window, cx);
  });
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "first");
    assert_eq!(panel.queued_prompts, vec!["second".to_string()]);
  });

  panel.update_in(cx, |panel, window, cx| {
    // Non-empty draft: popping swaps so nothing is lost.
    panel.pop_queued_to_composer(0, window, cx);
  });
  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "second");
    assert_eq!(panel.queued_prompts, vec!["first".to_string()]);
  });

  panel.update(cx, |panel, cx| panel.delete_queued(0, cx));
  panel.read_with(cx, |panel, _| {
    assert!(panel.queued_prompts.is_empty());
  });
}

#[gpui::test]
async fn scrolling_away_shows_the_jump_to_bottom_pill(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = (0..30)
      .map(|i| agent_message(&format!("message {i}")))
      .collect();
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-chat-jump-bottom").is_none(),
    "following the tail shows no pill"
  );

  // Merely pausing at the tail is not "scrolled away": no pill.
  panel.update(cx, |panel, cx| {
    panel.messages_list.pause_following_tail();
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-chat-jump-bottom").is_none(),
    "resting at the tail shows no pill"
  );

  panel.update(cx, |panel, cx| {
    panel.messages_list.scroll_by(px(-200.));
    cx.notify();
  });
  cx.run_until_parked();
  let pill = cx
    .debug_bounds("agent-chat-jump-bottom")
    .expect("pill painted once the reader leaves the tail");

  cx.simulate_click(pill.center(), gpui::Modifiers::default());
  cx.run_until_parked();
  // The pill reads the spacer bounds of the previous frame; settle one more.
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-chat-jump-bottom").is_none(),
    "clicking returns to the tail and hides the pill"
  );
}

#[gpui::test]
async fn a_disconnected_panel_offers_reconnect(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Error("Agent disconnected".into());
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("agent-chat-reconnect").is_some(),
    "the error state carries a reconnect button"
  );
}

#[gpui::test]
async fn a_turn_without_tools_still_makes_one_message(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.on_event(text_chunk("Hello, "), cx);
    panel.on_event(text_chunk("world."), cx);
    panel.flush_turn_buffers();
    panel.end_turn();
  });

  panel.read_with(cx, |panel, _| {
    assert_eq!(item_kinds(&panel.items), vec!["agent"]);
    let ChatItem::Message(m) = &panel.items[0] else {
      unreachable!()
    };
    assert_eq!(m.text, "Hello, world.");
  });
}

#[gpui::test]
async fn a_disconnect_keeps_the_streamed_prose(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.on_event(thought_chunk("half a thought"), cx);
    panel.on_event(text_chunk("Half an answer"), cx);
    panel.on_agent_disconnected(cx);
  });

  panel.read_with(cx, |panel, _| {
    // Thought, prose, then the system notice; nothing streamed is lost.
    assert_eq!(item_kinds(&panel.items), vec!["thought", "agent", "other"]);
    assert!(panel.pending_agent.is_empty());
    assert!(panel.pending_thought.is_empty());
  });
}

#[gpui::test]
async fn a_code_block_streams_highlighted_in_the_generating_row(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  // A streaming reply always follows a prompt; alone at index 0 the
  // generating row does not paint under the test list, prompt included it does.
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.items = vec![user_message("show me toml")];
    panel.pending_agent = "```toml\nkey = 1\n```".to_string();
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  // The parse settles in the background; streaming remeasures the tail row
  // on every chunk, which repaints it.
  panel.update(cx, |panel, cx| {
    panel.mark_last_item_changed();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(
    cx.debug_bounds("chat-code-block-toml").is_some(),
    "the streaming pending markdown renders the custom code block"
  );
}

#[gpui::test]
async fn an_unknown_language_block_still_renders_with_its_copy_button(
  cx: &mut gpui::TestAppContext,
) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![agent_message("```nosuchlang\nplain body\n```")];
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();

  assert!(cx.debug_bounds("chat-code-block-nosuchlang").is_some());
  let copy = cx
    .debug_bounds("chat-code-copy")
    .expect("copy button painted");
  cx.simulate_click(copy.center(), gpui::Modifiers::default());
  cx.run_until_parked();

  let copied = cx
    .update(|_, cx| cx.read_from_clipboard())
    .and_then(|item| item.text());
  assert_eq!(copied.as_deref(), Some("plain body"));
}

#[gpui::test]
async fn shift_enter_inserts_a_newline_without_submitting(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  cx.run_until_parked();

  let input_focus = panel.read_with(cx, |panel, cx| panel.input.read(cx).focus_handle(cx));
  cx.update(|window, cx| window.focus(&input_focus, cx));
  cx.simulate_input("line one");
  cx.simulate_keystrokes("shift-enter");
  cx.simulate_input("line two");
  cx.run_until_parked();

  panel.read_with(cx, |panel, cx| {
    assert_eq!(panel.input.read(cx).value(), "line one\nline two");
    assert!(panel.items.is_empty(), "no prompt was recorded");
    assert!(!panel.in_flight);
  });
}

#[gpui::test]
async fn the_runway_holds_the_prompt_at_the_top_while_the_reply_grows(
  cx: &mut gpui::TestAppContext,
) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    // Enough history to make the list scrollable past one viewport.
    panel.items = (0..30)
      .flat_map(|i| {
        vec![
          user_message(&format!("question {i}")),
          agent_message(&format!("a long answer {i}\nwith\nseveral\nlines")),
        ]
      })
      .collect();
    panel.items.push(user_message("the runway prompt"));
    panel.sync_list_count();
    panel.arm_runway();
    cx.notify();
  });
  cx.run_until_parked();
  // Two frames: the first paint measures, the next one trues up the spacer.
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();

  let (anchor_top, viewport_top, end_space) = panel.read_with(cx, |panel, _| {
    let anchor_ix = panel.list_ix_for_item(panel.runway_anchor_item().unwrap());
    let bounds = panel.messages_list.bounds_for_item(anchor_ix);
    let viewport = panel.messages_list.viewport_bounds();
    (
      bounds.map(|b| f32::from(b.top())),
      f32::from(viewport.top()),
      panel.runway_end_space,
    )
  });
  let anchor_top = anchor_top.expect("the anchored prompt is painted");
  assert!(
    (anchor_top - viewport_top - RUNWAY_TOP_MARGIN_PX).abs() < 2.0,
    "the prompt sits a margin below the viewport top: {anchor_top} vs {viewport_top}"
  );
  assert!(end_space > 0.0, "space is reserved below the prompt");

  // The streaming reply eats the reservation without moving the prompt.
  panel.update(cx, |panel, cx| {
    panel.items.push(agent_message("the reply starts\nhere"));
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();
  let (anchor_top_after, shrunk) = panel.read_with(cx, |panel, _| {
    let anchor_ix = panel.list_ix_for_item(panel.runway_anchor_item().unwrap());
    let bounds = panel.messages_list.bounds_for_item(anchor_ix);
    (bounds.map(|b| f32::from(b.top())), panel.runway_end_space)
  });
  assert_eq!(
    anchor_top_after.map(|t| t.round()),
    Some(anchor_top.round()),
    "the prompt did not move as the reply grew"
  );
  assert!(
    shrunk < end_space,
    "the reservation shrank as the reply grew: {shrunk} vs {end_space}"
  );

  // A reply taller than the reservation retires the runway.
  panel.update(cx, |panel, cx| {
    let tall: String = (0..200).map(|i| format!("line {i}\n")).collect();
    panel.items.push(agent_message(&tall));
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(
      !panel.runway_active,
      "an overflowing reply retires the runway"
    );
  });
}

#[gpui::test]
async fn a_wheel_scroll_releases_the_runway_hold(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = (0..30)
      .flat_map(|i| {
        vec![
          user_message(&format!("question {i}")),
          agent_message(&format!("answer {i}\nlines\nlines")),
        ]
      })
      .collect();
    panel.items.push(user_message("held prompt"));
    panel.sync_list_count();
    panel.arm_runway();
    cx.notify();
  });
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| assert!(panel.runway_following));

  let center = panel.read_with(cx, |panel, _| {
    let b = panel.messages_list.viewport_bounds();
    gpui::point(b.center().x, b.center().y)
  });
  cx.simulate_event(gpui::ScrollWheelEvent {
    position: center,
    delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(60.))),
    ..Default::default()
  });
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(
      !panel.runway_following,
      "the wheel hands the scroll back to the reader"
    );
    assert!(panel.runway_active, "the reservation itself stays");
  });
}

async fn add_retired_runway_panel(
  cx: &mut gpui::TestAppContext,
) -> (
  gpui::Entity<AgentChatPanel>,
  &mut gpui::VisualTestContext,
  gpui::Point<gpui::Pixels>,
) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = (0..30)
      .flat_map(|i| {
        vec![
          user_message(&format!("question {i}")),
          agent_message(&format!("answer {i}\nlines\nlines")),
        ]
      })
      .collect();
    panel.items.push(user_message("held prompt"));
    panel.sync_list_count();
    panel.arm_runway();
    cx.notify();
  });
  cx.run_until_parked();
  panel.update(cx, |panel, cx| {
    let tall: String = (0..200).map(|i| format!("line {i}\n")).collect();
    panel.items.push(agent_message(&tall));
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(!panel.runway_active, "the tall reply retired the runway");
  });
  let center = panel.read_with(cx, |panel, _| {
    let b = panel.messages_list.viewport_bounds();
    gpui::point(b.center().x, b.center().y)
  });
  (panel, cx, center)
}

#[gpui::test]
async fn consuming_the_runway_shows_a_jump_to_bottom_pill(cx: &mut gpui::TestAppContext) {
  let (panel, cx, _) = add_retired_runway_panel(cx).await;

  panel.read_with(cx, |panel, _| {
    assert!(!panel.runway_active, "the tall reply retired the runway");
    assert!(panel.show_jump_pill, "the reader can rejoin the tail");
  });
  let pill = cx
    .debug_bounds("agent-chat-jump-bottom")
    .expect("the jump button is painted after the runway is consumed");

  cx.simulate_click(pill.center(), gpui::Modifiers::default());
  cx.run_until_parked();
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(panel.messages_list.is_following_tail());
    assert!(!panel.runway_active);
    assert!(!panel.show_jump_pill);
  });
}

#[gpui::test]
async fn the_jump_pill_keeps_following_the_tail(cx: &mut gpui::TestAppContext) {
  let (panel, cx, center) = add_retired_runway_panel(cx).await;

  // The reader scrolls up, away from the stream.
  cx.simulate_event(gpui::ScrollWheelEvent {
    position: center,
    delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(120.))),
    ..Default::default()
  });
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(!panel.messages_list.is_following_tail());
  });

  panel.update(cx, |panel, cx| {
    panel.jump_to_tail();
    cx.notify();
  });
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(
      panel.messages_list.is_following_tail(),
      "the pill engages sticky follow, not a one-shot scroll"
    );
  });

  // The stream keeps growing: the reader stays glued without another click.
  panel.update(cx, |panel, cx| {
    panel.items.push(agent_message("more\nstreamed\nlines"));
    panel.sync_list_count();
    cx.notify();
  });
  cx.run_until_parked();
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(
      panel.messages_list.is_following_tail(),
      "growth does not shake the follow off"
    );
    assert!(!panel.show_jump_pill, "no pill while following");
  });
}

#[gpui::test]
async fn scrolling_back_to_the_end_reengages_follow(cx: &mut gpui::TestAppContext) {
  let (panel, cx, center) = add_retired_runway_panel(cx).await;

  cx.simulate_event(gpui::ScrollWheelEvent {
    position: center,
    delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(120.))),
    ..Default::default()
  });
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(!panel.messages_list.is_following_tail());
  });

  // Wheeling all the way down lands on the end: follow re-engages by itself.
  cx.simulate_event(gpui::ScrollWheelEvent {
    position: center,
    delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-100000.))),
    ..Default::default()
  });
  cx.run_until_parked();
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(
      panel.messages_list.is_following_tail(),
      "reaching the end by wheel re-engages sticky follow"
    );
  });
}

#[gpui::test]
async fn scrolling_up_detaches_from_the_tail(cx: &mut gpui::TestAppContext) {
  let (panel, cx, center) = add_retired_runway_panel(cx).await;

  panel.update(cx, |panel, cx| {
    panel.jump_to_tail();
    cx.notify();
  });
  cx.run_until_parked();

  cx.simulate_event(gpui::ScrollWheelEvent {
    position: center,
    delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(120.))),
    ..Default::default()
  });
  cx.run_until_parked();
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();
  panel.read_with(cx, |panel, _| {
    assert!(
      !panel.messages_list.is_following_tail(),
      "a wheel up hands the scroll back to the reader"
    );
    assert!(panel.show_jump_pill, "the pill offers the way back");
  });
}

#[gpui::test]
async fn a_first_prompt_leaves_no_phantom_scroll(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.items = vec![checkpoint_marker("cp-1"), user_message("first prompt")];
    panel.sync_list_count();
    panel.arm_runway();
    cx.notify();
  });
  cx.run_until_parked();
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();

  panel.read_with(cx, |panel, _| {
    let viewport = panel.messages_list.viewport_bounds();
    let spacer = panel
      .messages_list
      .bounds_for_item(panel.runway_spacer_ix())
      .expect("spacer painted");
    assert!(
      f32::from(spacer.bottom()) <= f32::from(viewport.bottom()) + 1.0,
      "a short transcript fits the viewport: {} vs {}",
      f32::from(spacer.bottom()),
      f32::from(viewport.bottom())
    );
    assert!(!panel.show_jump_pill, "nothing to jump to");
  });
}

#[gpui::test]
async fn an_agent_without_steering_hides_the_steer_button(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.supports_steering = true;
    panel.queued_prompts = vec!["do this next".into()];
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-chat-queued-steer").is_some(),
    "a steering agent offers to send the queued message into the turn"
  );

  panel.update(cx, |panel, cx| {
    panel.supports_steering = false;
    cx.notify();
  });
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-chat-queued-steer").is_none(),
    "an agent without the steering extension hides the button"
  );
}

#[gpui::test]
async fn cmd_enter_queues_instead_of_steering_when_unsupported(cx: &mut gpui::TestAppContext) {
  let steer_keystroke = if cfg!(target_os = "macos") {
    "cmd-enter"
  } else {
    "ctrl-enter"
  };
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.supports_steering = true;
    cx.notify();
  });
  cx.run_until_parked();

  let input_focus = panel.read_with(cx, |panel, cx| panel.input.read(cx).focus_handle(cx));
  cx.update(|window, cx| window.focus(&input_focus, cx));
  cx.simulate_input("actually, skip the tests");
  cx.simulate_keystrokes(steer_keystroke);
  cx.run_until_parked();

  // The disconnected panel has no session to steer, so the attempt fails and
  // the draft stays put: proof the keystroke took the steer path, since a
  // plain Enter would have queued it.
  panel.read_with(cx, |panel, cx| {
    assert!(panel.queued_prompts.is_empty());
    assert_eq!(panel.input.read(cx).value(), "actually, skip the tests");
  });

  panel.update(cx, |panel, cx| {
    panel.supports_steering = false;
    cx.notify();
  });
  cx.run_until_parked();
  cx.update(|window, cx| window.focus(&input_focus, cx));
  cx.simulate_keystrokes(steer_keystroke);
  cx.run_until_parked();

  panel.read_with(cx, |panel, cx| {
    assert_eq!(
      panel.queued_prompts,
      vec!["actually, skip the tests".to_string()],
      "the message waits for the next turn instead of being refused mid-flight"
    );
    assert_eq!(panel.input.read(cx).value(), "", "the composer drained");
    assert!(
      panel.items.is_empty(),
      "no optimistic bubble for a steer that never happened"
    );
  });
}

#[gpui::test]
async fn steer_prompt_refuses_when_the_agent_cannot_steer(cx: &mut gpui::TestAppContext) {
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.status = Status::Ready;
    panel.in_flight = true;
    panel.supports_steering = false;
    assert!(!panel.steer_prompt("mid-turn".to_string(), cx));
    assert!(panel.items.is_empty());
  });
}

#[test]
fn an_agent_icon_prefers_the_embedded_mark_then_the_registry_then_a_generic_one() {
  // Our own agents keep their embedded mark whatever the cache holds.
  for (id, expected) in [
    ("claude-acp", UiIconName::Claude),
    ("codex-acp", UiIconName::OpenAi),
    ("pi-acp", UiIconName::Pi),
  ] {
    for cached in [false, true] {
      assert_eq!(
        backend_icon_source(&AgentId::new(id), cached),
        BackendIconSource::Embedded(expected),
        "{id} keeps its embedded mark (cached: {cached})"
      );
    }
  }

  assert_eq!(
    backend_icon_source(&AgentId::new("gemini"), true),
    BackendIconSource::Registry("agent-icons/gemini.svg".to_string()),
    "a cached registry icon is used once it is on disk"
  );
  assert_eq!(
    backend_icon_source(&AgentId::new("gemini"), false),
    BackendIconSource::Generic,
    "before the download lands, the generic mark stands in"
  );

  // An id that could escape the cache directory never becomes a path, even if
  // something claims it is cached.
  assert_eq!(
    backend_icon_source(&AgentId::new("../evil"), true),
    BackendIconSource::Generic
  );
}
