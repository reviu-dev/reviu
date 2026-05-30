//! Minimal ACP agent used by integration tests.

use agent_client_protocol::schema::{
  AgentCapabilities, ContentBlock, Implementation, InitializeRequest, InitializeResponse,
  NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse, SessionId,
  SessionNotification, SessionUpdate, StopReason, TextContent,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Dispatch};
use smol::Unblock;

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
              .agent_capabilities(AgentCapabilities::new())
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
          let _ = cx.send_notification(SessionNotification::new(
            session_id,
            SessionUpdate::AgentMessageChunk(agent_client_protocol::schema::ContentChunk::new(
              ContentBlock::Text(TextContent::new("ack")),
            )),
          ));
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
