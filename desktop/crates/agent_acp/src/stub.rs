//! Minimal ACP agent for tests and the driver: acks every prompt.

use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::{
  AgentCapabilities, CancelNotification, ContentBlock, Diff, Implementation, InitializeRequest,
  InitializeResponse, NewSessionRequest, NewSessionResponse, PermissionOption,
  PermissionOptionKind, PromptRequest, PromptResponse, RequestPermissionRequest, SessionId,
  SessionNotification, SessionUpdate, StopReason, TextContent, ToolCall, ToolCallContent,
  ToolCallId, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Dispatch, Responder};
use smol::Unblock;

/// A thought chunk then the "ack" reply, closing the turn's notifications.
fn finish_turn(cx: &ConnectionTo<Client>, session_id: SessionId) {
  let _ = cx.send_notification(SessionNotification::new(
    session_id.clone(),
    SessionUpdate::AgentThoughtChunk(agent_client_protocol::schema::ContentChunk::new(
      ContentBlock::Text(TextContent::new("stub thinking")),
    )),
  ));
  let _ = cx.send_notification(SessionNotification::new(
    session_id,
    SessionUpdate::AgentMessageChunk(agent_client_protocol::schema::ContentChunk::new(
      ContentBlock::Text(TextContent::new("ack")),
    )),
  ));
}

/// Serves the stub over stdio until the client hangs up.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
  smol::block_on(async {
    let stdin = Unblock::new(std::io::stdin());
    let stdout = Unblock::new(std::io::stdout());

    // A prompt mentioning "wait" parks its responder here until session/cancel.
    // The bool remembers a cancel that raced ahead of the prompt itself.
    type WaitSlot = (Option<Responder<PromptResponse>>, bool);
    let waiting: Arc<Mutex<WaitSlot>> = Arc::new(Mutex::new((None, false)));
    let waiting_for_cancel = waiting.clone();
    let waiting_for_steer = waiting.clone();

    Agent
      .builder()
      .name("stub-agent")
      .on_receive_notification(
        async move |_notif: CancelNotification, _: ConnectionTo<Client>| {
          let parked = {
            let mut slot = waiting_for_cancel.lock().expect("waiting slot");
            slot.1 = true;
            slot.0.take()
          };
          if let Some(responder) = parked {
            let _ = responder.respond(PromptResponse::new(StopReason::Cancelled));
          }
          Ok(())
        },
        agent_client_protocol::on_receive_notification!(),
      )
      .on_receive_request(
        async move |req: InitializeRequest, responder, _: ConnectionTo<Client>| {
          responder.respond(
            InitializeResponse::new(req.protocol_version)
              .agent_capabilities(
                AgentCapabilities::new()
                  .load_session(true)
                  .prompt_capabilities(
                    agent_client_protocol::schema::PromptCapabilities::new().image(true),
                  ),
              )
              .agent_info(Implementation::new("stub-agent", "0.0.1")),
          )
        },
        agent_client_protocol::on_receive_request!(),
      )
      .on_receive_request(
        async move |req: agent_client_protocol::schema::LoadSessionRequest,
                    responder,
                    cx: ConnectionTo<Client>| {
          // Only the session this stub ever creates can be loaded; an unknown
          // id errors so clients exercise the fall-back-to-new-session path.
          if req.session_id.0.as_ref() != "stub-session" {
            return responder.respond_with_error(agent_client_protocol::Error::invalid_params());
          }
          // Real agents replay the whole history during a load; the client
          // must drop that replayed content instead of appending it again.
          let session_id = req.session_id.clone();
          let _ = cx.send_notification(SessionNotification::new(
            session_id.clone(),
            SessionUpdate::AgentThoughtChunk(agent_client_protocol::schema::ContentChunk::new(
              ContentBlock::Text(TextContent::new("stub thinking")),
            )),
          ));
          let _ = cx.send_notification(SessionNotification::new(
            session_id,
            SessionUpdate::AgentMessageChunk(agent_client_protocol::schema::ContentChunk::new(
              ContentBlock::Text(TextContent::new("ack")),
            )),
          ));
          responder.respond(agent_client_protocol::schema::LoadSessionResponse::new())
        },
        agent_client_protocol::on_receive_request!(),
      )
      .on_receive_request(
        async move |_req: NewSessionRequest, responder, cx: ConnectionTo<Client>| {
          let session_id = SessionId::new("stub-session");
          let result = responder.respond(NewSessionResponse::new(session_id.clone()));
          let _ = cx.send_notification(SessionNotification::new(
            session_id,
            SessionUpdate::AvailableCommandsUpdate(
              agent_client_protocol::schema::AvailableCommandsUpdate::new(vec![
                agent_client_protocol::schema::AvailableCommand::new(
                  "compact",
                  "Compact the conversation",
                ),
                agent_client_protocol::schema::AvailableCommand::new(
                  "review",
                  "Review the changes",
                ),
              ]),
            ),
          ));
          result
        },
        agent_client_protocol::on_receive_request!(),
      )
      .on_receive_request(
        async move |req: PromptRequest, responder, cx: ConnectionTo<Client>| {
          let session_id = req.session_id.clone();
          let prompt_contains = |needle: &str| {
            req
              .prompt
              .iter()
              .any(|block| matches!(block, ContentBlock::Text(t) if t.text.contains(needle)))
          };
          // An image block gets its own acknowledgement, for clients testing
          // the attachment path.
          if req
            .prompt
            .iter()
            .any(|block| matches!(block, ContentBlock::Image(_)))
          {
            let _ = cx.send_notification(SessionNotification::new(
              session_id.clone(),
              SessionUpdate::AgentMessageChunk(agent_client_protocol::schema::ContentChunk::new(
                ContentBlock::Text(TextContent::new("image received")),
              )),
            ));
          }
          // "edit" streams an edit tool call carrying a diff, so clients can
          // exercise the per-turn edit summary.
          if prompt_contains("edit") {
            let mut call = ToolCall::new(ToolCallId::new("stub-edit"), "Editing files".to_string());
            call.kind = ToolKind::Edit;
            call.status = ToolCallStatus::Completed;
            call.content = vec![ToolCallContent::Diff(
              Diff::new("src/stub.rs", "line one\nline two\n")
                .old_text(Some("line one\n".to_string())),
            )];
            let _ = cx.send_notification(SessionNotification::new(
              session_id.clone(),
              SessionUpdate::ToolCall(call),
            ));
          }
          // "wait" streams a partial reply then parks the turn until the
          // client cancels it.
          if prompt_contains("wait") {
            let _ = cx.send_notification(SessionNotification::new(
              session_id.clone(),
              SessionUpdate::AgentMessageChunk(agent_client_protocol::schema::ContentChunk::new(
                ContentBlock::Text(TextContent::new("partial reply")),
              )),
            ));
            let mut slot = waiting.lock().expect("waiting slot");
            if slot.1 {
              return responder.respond(PromptResponse::new(StopReason::Cancelled));
            }
            slot.0 = Some(responder);
            return Ok(());
          }
          // "fail" errors the turn, so clients can exercise the error path.
          if prompt_contains("fail") {
            return responder.respond_with_error(agent_client_protocol::Error::internal_error());
          }
          // A prompt mentioning "permission" first round-trips a permission
          // request carrying a real command, so clients can exercise the flow.
          // Awaiting the outcome inline would park the connection's task queue
          // and deadlock; the turn finishes in the response callback instead.
          let wants_permission = prompt_contains("permission");
          if wants_permission {
            let fields = ToolCallUpdateFields::new()
              .kind(ToolKind::Execute)
              .title("Run cargo test".to_string())
              .raw_input(serde_json::json!({ "command": "cargo test --workspace" }));
            let request = RequestPermissionRequest::new(
              session_id.clone(),
              ToolCallUpdate::new(ToolCallId::new("stub-tool"), fields),
              vec![
                PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
                PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
              ],
            );
            return cx.send_request(request).on_receiving_result({
              let cx = cx.clone();
              async move |_outcome| {
                finish_turn(&cx, session_id);
                responder.respond(PromptResponse::new(StopReason::EndTurn))
              }
            });
          }
          finish_turn(&cx, session_id);
          responder.respond(PromptResponse::new(StopReason::EndTurn))
        },
        agent_client_protocol::on_receive_request!(),
      )
      .on_receive_dispatch(
        async move |message: Dispatch, cx: ConnectionTo<Client>| {
          let _ = &cx;
          // Responses to requests this stub sent (e.g. request_permission)
          // route to their waiting task.
          if let Dispatch::Response(result, router) = message {
            return router.respond_with_result(result);
          }
          // The steering extension: injects into a parked turn ("wait"),
          // errors on "fail", asks for a plain prompt when nothing runs.
          if let Dispatch::Request(request, responder) = message {
            if request.method() == "_session/steering" {
              let session_id = request
                .params()
                .get("sessionId")
                .and_then(|v| v.as_str())
                .unwrap_or("stub-session")
                .to_string();
              let text = request
                .params()
                .get("prompt")
                .and_then(|p| p.get(0))
                .and_then(|b| b.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
              if text.contains("fail") {
                return responder
                  .respond_with_error(agent_client_protocol::Error::internal_error());
              }
              let parked = {
                let mut slot = waiting_for_steer.lock().expect("waiting slot");
                slot.0.take()
              };
              let Some(parked) = parked else {
                return responder.respond(serde_json::json!({ "outcome": "promptRequired" }));
              };
              let _ = cx.send_notification(SessionNotification::new(
                SessionId::new(std::sync::Arc::<str>::from(session_id.as_str())),
                SessionUpdate::AgentMessageChunk(agent_client_protocol::schema::ContentChunk::new(
                  ContentBlock::Text(TextContent::new("steered reply")),
                )),
              ));
              let _ = parked.respond(PromptResponse::new(StopReason::EndTurn));
              return responder.respond(serde_json::json!({ "outcome": "injected" }));
            }
            return responder.respond_with_error(agent_client_protocol::Error::method_not_found());
          }
          Ok(())
        },
        agent_client_protocol::on_receive_dispatch!(),
      )
      .connect_to(agent_client_protocol::ByteStreams::new(stdout, stdin))
      .await?;
    Ok(())
  })
}
