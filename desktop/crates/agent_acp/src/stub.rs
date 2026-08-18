//! Minimal ACP agent for tests and the driver: acks every prompt.

use agent_client_protocol::schema::{
  AgentCapabilities, ContentBlock, Implementation, InitializeRequest, InitializeResponse,
  NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind, PromptRequest,
  PromptResponse, RequestPermissionRequest, SessionId, SessionNotification, SessionUpdate,
  StopReason, TextContent, ToolCallId, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Dispatch};
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

    Agent
      .builder()
      .name("stub-agent")
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
          // A prompt mentioning "permission" first round-trips a permission
          // request carrying a real command, so clients can exercise the flow.
          // Awaiting the outcome inline would park the connection's task queue
          // and deadlock; the turn finishes in the response callback instead.
          let wants_permission = req.prompt.iter().any(
            |block| matches!(block, ContentBlock::Text(t) if t.text.contains("permission")),
          );
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
