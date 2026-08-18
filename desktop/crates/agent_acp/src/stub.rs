//! Minimal ACP agent for tests and the driver: acks every prompt.

use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::{
  AgentCapabilities, CancelNotification, ContentBlock, Implementation, InitializeRequest,
  InitializeResponse, NewSessionRequest, NewSessionResponse, PermissionOption,
  PermissionOptionKind, PromptRequest, PromptResponse, RequestPermissionRequest, SessionId,
  SessionNotification, SessionUpdate, StopReason, TextContent, ToolCallId, ToolCallUpdate,
  ToolCallUpdateFields, ToolKind,
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
              // load_session advertised but not handled: session/load errors with
              // method_not_found, which exercises the fall-back-to-new-session path.
              .agent_capabilities(AgentCapabilities::new().load_session(true))
              .agent_info(Implementation::new("stub-agent", "0.0.1")),
          )
        },
        agent_client_protocol::on_receive_request!(),
      )
      .on_receive_request(
        async move |_req: NewSessionRequest, responder, _: ConnectionTo<Client>| {
          responder.respond(NewSessionResponse::new(SessionId::new("stub-session")))
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
          // "wait" parks the turn until the client cancels it.
          if prompt_contains("wait") {
            let mut slot = waiting.lock().expect("waiting slot");
            if slot.1 {
              return responder.respond(PromptResponse::new(StopReason::Cancelled));
            }
            slot.0 = Some(responder);
            return Ok(());
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
          message.respond_with_error(agent_client_protocol::Error::method_not_found(), cx)
        },
        agent_client_protocol::on_receive_dispatch!(),
      )
      .connect_to(agent_client_protocol::ByteStreams::new(stdout, stdin))
      .await?;
    Ok(())
  })
}
