//! Confirming a write on a client older than 2026-07-28 (#2158; design
//! §7.4, §11). Such a client can't retry with a form's answer, but one
//! that declared the elicitation capability takes the server's own
//! `elicitation/create` request. The server sends it the form the
//! `input_required` result would have carried, waits for the answer, and
//! hands that answer to the retry path as if the client had retried:
//! the state is opened, a decline is recorded as declined, the targets
//! are read again under the workspace's lock and a stale preview is
//! refused. The state never leaves the server.
//!
//! The wait holds no lock: not the workspace's (the preview reads the
//! ledger without it, and the retry takes it after), nor the write
//! state's, so other calls go on while the person thinks. It lasts at
//! most the confirmation's lifetime ([`LIFETIME`]); then the SDK tells
//! the client the request was cancelled. Only the `elicitation/create`
//! request type goes out, through the SDK's `server` feature: its
//! `elicitation` feature only adds typed helpers over the same request.

use rmcp::RoleServer;
use rmcp::model::{
    ClientResult, ElicitationAction, InputRequest, InputRequiredResult, ServerRequest,
};
use rmcp::service::{Peer, PeerRequestOptions, ServiceError};
use serde_json::json;

use super::write::{CONFIRM, Retry};
use crate::writes::seal::LIFETIME;

/// Ask `session` to confirm the write `ask` describes, and turn its
/// answer into the retry a form's answer would have made: accept or
/// decline. A cancel, no answer within [`LIFETIME`], or a session gone is
/// `Err` with why; nothing is sent for it.
pub(crate) async fn confirm(
    session: &Peer<RoleServer>,
    mut ask: InputRequiredResult,
) -> Result<Retry, &'static str> {
    let Some(InputRequest::Elicitation(request)) = ask
        .input_requests
        .as_mut()
        .and_then(|requests| requests.remove(CONFIRM))
    else {
        return Err("there was no confirmation form to send");
    };
    let sent = session
        .send_request_with_option(
            ServerRequest::ElicitRequest(request),
            PeerRequestOptions::with_timeout(LIFETIME),
        )
        .await
        .map_err(|_| "the session ended")?;
    let answer = match sent.await_response().await {
        Ok(ClientResult::ElicitResult(answer)) => answer,
        Ok(_) => return Err("the client answered with something else"),
        Err(ServiceError::Timeout { .. }) => return Err("no answer within 120 s"),
        Err(_) => return Err("the session ended"),
    };
    let action = match answer.action {
        ElicitationAction::Accept => "accept",
        ElicitationAction::Decline => "decline",
        _ => return Err("the user cancelled"),
    };
    let mut responses = rmcp::model::InputResponses::new();
    responses.insert(
        CONFIRM.into(),
        json!({ "action": action, "content": answer.content }),
    );
    Ok(Retry {
        state: ask.request_state,
        responses: Some(responses),
        server: true,
    })
}
