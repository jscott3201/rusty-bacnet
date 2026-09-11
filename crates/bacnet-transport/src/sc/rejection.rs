//! Owner-local budget for node control/source/MU/missing-payload/unknown NAKs only.
//! Cancellation drops the send future, not bytes already buffered by the driver.

use std::future::{poll_fn, Future};
use std::task::Poll;
use std::time::{Duration, Instant};

use bacnet_types::error::Error;

use super::{control_admission, data_attributes, source_admission, WebSocketPort};
use crate::sc_frame::{first_must_understand_destination_option_marker, ScMessage};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RejectionExpired;

#[derive(Clone, Copy)]
pub(super) struct RejectionBudget {
    last_activity: Instant,
    timeout: Duration,
}

impl RejectionBudget {
    pub(super) fn new(last_activity: Instant, timeout_ms: u64) -> Self {
        Self {
            last_activity,
            timeout: Duration::from_millis(timeout_ms),
        }
    }

    fn remaining(self) -> Result<Duration, RejectionExpired> {
        let remaining = self.timeout.saturating_sub(self.last_activity.elapsed());
        if remaining.is_zero() {
            Err(RejectionExpired)
        } else {
            Ok(remaining)
        }
    }

    pub(super) async fn send<W: WebSocketPort>(
        self,
        ws: &W,
        bytes: &[u8],
    ) -> Result<Result<(), Error>, RejectionExpired> {
        self.remaining()?; // Do not even construct a send on an expired budget.
        let send = ws.send(bytes);
        tokio::pin!(send);
        let guarded = poll_fn(|cx| {
            if self.remaining().is_err() {
                return Poll::Ready(Err(RejectionExpired));
            }
            let result = send.as_mut().poll(cx);
            // Tokio timeouts poll ready futures even after expiry. Check both
            // sides of every poll, including a non-yielding completion.
            if self.remaining().is_err() {
                return Poll::Ready(Err(RejectionExpired));
            }
            result.map(Ok)
        });
        tokio::pin!(guarded);
        loop {
            // Never add an arbitrary u64-ms config to Instant (it may overflow).
            // Chunk only the timer registration, NOT the accepted-activity budget.
            let wait = self.remaining()?.min(Duration::from_secs(86_400));
            if let Ok(result) = tokio::time::timeout(wait, &mut guarded).await {
                return result;
            }
        }
    }
}

pub(super) async fn reject<W: WebSocketPort>(
    msg: &ScMessage,
    wire: &[u8],
    ws: &W,
    budget: RejectionBudget,
) -> Result<bool, RejectionExpired> {
    if control_admission::reject_invalid_control(msg, wire, ws, budget).await? {
        return Ok(true);
    }
    // Source and MU gates apply only to NPDUs; valid Heartbeat-ACK matching
    // remains the caller's next step. Silent decisions never consult the budget.
    if source_admission::reject_invalid_npdu_source(msg, ws, budget).await? {
        return Ok(true);
    }
    if data_attributes::reject_unsupported_must_understand_destination_option(
        msg,
        first_must_understand_destination_option_marker(wire),
        ws,
        budget,
    )
    .await?
    {
        return Ok(true);
    }
    if super::empty_npdu::reject(msg, ws, budget).await? {
        return Ok(true);
    }
    // All preceding gates exclude Unknown. Its identity wins over option or
    // payload diagnostics without changing any known-function admission.
    super::unknown_function::reject(msg, ws, budget).await
}
