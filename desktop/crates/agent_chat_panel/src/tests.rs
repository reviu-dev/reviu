use super::*;
use agent_acp::PermissionPromptOption;
use agent_client_protocol::schema::{
  SessionConfigSelect, SessionConfigSelectOption, ToolCallContent, ToolCallLocation,
  ToolCallUpdateFields,
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
      locations: Vec::new(),
      diffs: Vec::new(),
      outputs: Vec::new(),
      terminals: Vec::new(),
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

#[gpui::test]
async fn the_auto_approve_flag_survives_a_conversation_reload(cx: &mut gpui::TestAppContext) {
  let dir = temp_dir("agent-auto-approve");
  let (panel, cx) = add_panel_window(cx);
  panel.update(cx, |panel, cx| {
    panel.state_dir = Some(dir.clone());
    panel.items = vec![user_message("hi")];
    panel.toggle_auto_approve(cx);
  });
  let conv_id = panel.read_with(cx, |panel, _| panel.current_conv.id.clone());
  let (_, _, _, _, auto_approve) =
    load_conversation_file(&dir.join(format!("{conv_id}.json"))).expect("reloads");
  assert!(auto_approve, "the toggle persists with the conversation");
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
fn list_conversations_sorted_by_updated_at_desc() {
  let dir = temp_dir("agent-sort");
  let mk = |id: &str, started: u64, updated: u64| {
    let conv = PersistedConversation {
      meta: ConversationMeta {
        id: id.to_string(),
        started_at_secs: started,
        updated_at_secs: updated,
        title: id.to_string(),
        message_count: 1,
        session_id: None,
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
fn selectable_config_options_skips_model_and_mode_categories() {
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
    select_option("effort", "Reasoning effort", "low", &["low", "high"], None),
  ];

  let selectors = selectable_config_options(&options);

  assert_eq!(selectors.len(), 1);
  assert_eq!(selectors[0].name.as_ref(), "Reasoning effort");
  assert_eq!(selectors[0].current_label, "low");
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
    select_option("effort", "Effort", "high", &["low", "high"], None),
    select_option("sandbox", "Sandbox", "off", &["off", "on"], None),
  ];

  let selectors = selectable_config_options(&options);

  assert_eq!(config_summary(&selectors), "high · off");
  assert_eq!(config_summary(&[]), "");
}

#[test]
fn config_customized_only_when_a_value_left_its_advertised_default() {
  let options = vec![select_option(
    "effort",
    "Effort",
    "low",
    &["low", "high"],
    None,
  )];
  let selectors = selectable_config_options(&options);
  let mut defaults = HashMap::new();
  defaults.insert(
    selectors[0].id.clone(),
    SessionConfigValueId::new(std::sync::Arc::from("low")),
  );

  assert!(!config_customized(&selectors, &defaults));

  let changed = selectable_config_options(&[select_option(
    "effort",
    "Effort",
    "high",
    &["low", "high"],
    None,
  )]);
  assert!(config_customized(&changed, &defaults));
}

#[test]
fn config_customized_is_false_without_a_recorded_default() {
  let selectors = selectable_config_options(&[select_option(
    "effort",
    "Effort",
    "high",
    &["low", "high"],
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
          options: vec![PermissionPromptOption {
            option_id: "allow".into(),
            label: "Allow".into(),
            kind: PermissionOptionKind::AllowOnce,
          }],
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
    cx.debug_bounds("perm-invocation").is_some(),
    "the command is painted on the card"
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
    panel.state_dir = Some(dir.clone());
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
    panel.persist_state();
    cx.notify();
  });

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
    panel.persist_state();
    cx.notify();
  });
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
    panel.state_dir = Some(dir.clone());
    panel.items = vec![
      user_message("go"),
      ChatItem::Tool(tool_view("t1", ToolKind::Read, ToolCallStatus::Completed)),
      ChatItem::Tool(tool_view("t2", ToolKind::Read, ToolCallStatus::Completed)),
    ];
    panel.sync_list_count();
    panel.toggle_tool_group(1, cx);
  });
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

  let image = gpui::Image::from_bytes(gpui::ImageFormat::Png, vec![9, 9, 9]);
  cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image)));
  cx.simulate_keystrokes("ctrl-v");
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
  cx.simulate_keystrokes("ctrl-v");
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
