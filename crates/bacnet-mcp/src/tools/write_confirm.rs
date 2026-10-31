//! Confirming a write on a client older than 2026-07-28 (#2158; design
//! §7.4, §11). Such a client can't retry with a form's answer, but one
//! that declared the elicitation capability takes the server's own
//! `elicitation/create` request. The server sends it the form the
//! `input_required` result would have carried, waits for the answer, and
//! hands that answer to the retry path as if the client had retried:
//! the state is opened, a decline or a cancel is recorded as declined, the
//! targets are read again under the workspace's lock and a stale preview
//! is refused. The state never leaves the server.
//!
//! The wait holds no lock: not the workspace's (the preview reads the
//! ledger without it, and the retry takes it after), nor the write
//! state's, so other calls go on while the person thinks. One such
//! request waits at a time ([`crate::writes::Writes::ask`]). The wait ends
//! [`MARGIN`] before the sealed state would expire, so an answer it takes
//! is one the retry still honours; when the call is cancelled, the server
//! shuts down or the time is up, the client is told the request was
//! cancelled. Only the `elicitation/create` request type goes out, through
//! the SDK's `server` feature: its `elicitation` feature only adds typed
//! helpers over the same request.

use std::time::Duration;

use rmcp::RoleServer;
use rmcp::model::{
    ClientResult, ElicitationAction, InputRequest, InputRequiredResult, ServerRequest,
};
use rmcp::service::{Peer, PeerRequestOptions, ServiceError};
use serde_json::json;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::write::{CONFIRM, Retry};

/// How long before the sealed state expires the wait for an answer ends:
/// room for the retry to open it.
pub(crate) const MARGIN: Duration = Duration::from_secs(5);

/// Why no answer to go on came.
pub(crate) struct Unanswered {
    /// What happened, for the refusal and the audit log.
    pub(crate) why: String,
    /// The tool call itself was cancelled.
    pub(crate) cancelled: bool,
}

impl Unanswered {
    fn because(why: impl Into<String>) -> Self {
        Self {
            why: why.into(),
            cancelled: false,
        }
    }
}

/// What ended the wait.
enum Woke {
    Answer(Result<Result<ClientResult, ServiceError>, tokio::sync::oneshot::error::RecvError>),
    Stop(Unanswered),
}

/// Ask `session` to confirm the write `ask` describes, its state good
/// until `good_until`, and turn the answer into the retry a form's answer
/// would have made: accept, decline or cancel. The tool call's `cancel`,
/// the server's `closing`, or no answer by [`MARGIN`] before `good_until`
/// cancels the request at the client and is `Err`; so is a session gone
/// or an error answer. Nothing is sent for any of them.
pub(crate) async fn confirm(
    session: &Peer<RoleServer>,
    mut ask: InputRequiredResult,
    good_until: Instant,
    cancel: &CancellationToken,
    closing: &CancellationToken,
) -> Result<Retry, Unanswered> {
    let Some(InputRequest::Elicitation(request)) = ask
        .input_requests
        .as_mut()
        .and_then(|requests| requests.remove(CONFIRM))
    else {
        return Err(Unanswered::because(
            "there was no confirmation form to send",
        ));
    };
    let deadline = good_until.checked_sub(MARGIN).unwrap_or(good_until);
    let waited = deadline.saturating_duration_since(Instant::now());
    let mut handle = session
        .send_request_with_option(
            ServerRequest::ElicitRequest(request),
            PeerRequestOptions::no_options(),
        )
        .await
        .map_err(|_| Unanswered::because("the session ended"))?;
    let woke = tokio::select! {
        biased;
        answer = &mut handle.rx => Woke::Answer(answer),
        () = cancel.cancelled() => Woke::Stop(Unanswered {
            why: "the tool call was cancelled".into(),
            cancelled: true,
        }),
        () = closing.cancelled() => Woke::Stop(Unanswered::because("the server is shutting down")),
        () = tokio::time::sleep_until(deadline) => Woke::Stop(Unanswered::because(format!(
            "no answer within {} s", waited.as_secs()
        ))),
    };
    let answer = match woke {
        Woke::Stop(why) => {
            // The SDK keeps no handle on its own; without this the
            // client's form stays open.
            let _ = handle.cancel(Some(why.why.clone())).await;
            return Err(why);
        }
        Woke::Answer(Ok(Ok(ClientResult::ElicitResult(answer)))) => answer,
        Woke::Answer(Ok(Ok(_))) => {
            return Err(Unanswered::because(
                "the client answered with something else",
            ));
        }
        Woke::Answer(Ok(Err(ServiceError::McpError(error)))) => {
            let (message, _) = crate::untrusted::clean(&error.message, 120);
            return Err(Unanswered::because(format!(
                "the client answered with an error ({}: {message})",
                error.code.0
            )));
        }
        Woke::Answer(Ok(Err(error))) => {
            return Err(Unanswered::because(format!("the request failed: {error}")));
        }
        Woke::Answer(Err(_)) => return Err(Unanswered::because("the session ended")),
    };
    let action = match answer.action {
        ElicitationAction::Accept => "accept",
        ElicitationAction::Decline => "decline",
        // A cancel is declined too, as a form's cancel is.
        _ => "cancel",
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
