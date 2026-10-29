use bacnet_types::error::Error;

use bytes::BytesMut;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

use super::{ScConnection, WebSocketPort};
use crate::sc_frame::{ScFunction, ScMessage, encode_sc_message};

pub(super) const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 30_000;
pub(super) const DEFAULT_HEARTBEAT_TIMEOUT_MS: u64 = 60_000;

/// When the node's heartbeat timer next fires, from the last accepted inbound
/// BVLC (#2124): one interval after it while no heartbeat is outstanding (send
/// one), or one timeout after it while one is (the link is dead). Measuring
/// both from the same instant gives an ACK the full timeout minus interval.
pub(super) fn deadline(
    last_bvlc: Instant,
    outstanding: bool,
    (interval_ms, timeout_ms): (u64, u64),
) -> Instant {
    last_bvlc + Duration::from_millis(if outstanding { timeout_ms } else { interval_ms })
}

/// Builds the next Heartbeat-Request under the connection lock, releases the
/// lock, sends it and returns its message ID for ACK matching.
pub(super) async fn send_request<W: WebSocketPort>(
    conn: &Mutex<ScConnection>,
    ws: &W,
) -> Result<u16, Error> {
    let request = conn.lock().await.build_heartbeat();
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &request);
    ws.send(&buf).await.map(|()| request.message_id)
}

pub(super) fn is_bvlc_result_wire(data: &[u8]) -> bool {
    data.first()
        .is_some_and(|function| *function == ScFunction::Result.to_raw())
}

pub(super) fn ack_matches_outstanding(msg: &ScMessage, expected_message_id: Option<u16>) -> bool {
    msg.function == ScFunction::HeartbeatAck
        && expected_message_id.is_some_and(|message_id| msg.message_id == message_id)
        && msg.originating_vmac.is_none()
        && msg.destination_vmac.is_none()
        && msg.data_options.is_empty()
        && msg.payload.is_empty()
}

pub(super) fn validate_heartbeat_timing_ms(interval_ms: u64, timeout_ms: u64) -> Result<(), Error> {
    if !(3_000..=300_000).contains(&interval_ms) {
        return Err(Error::OutOfRange(format!(
            "BACnet/SC heartbeat interval must be in the configurable Annex AB.6.3 range \
             of 3..300 seconds (3000..=300000 ms), got {interval_ms} ms"
        )));
    }

    if timeout_ms <= interval_ms {
        return Err(Error::OutOfRange(format!(
            "BACnet/SC heartbeat disconnect timeout must be greater than the heartbeat \
             interval so a Heartbeat-ACK can arrive, got interval={interval_ms} ms \
             timeout={timeout_ms} ms"
        )));
    }

    Ok(())
}
