use bacnet_types::error::Error;

use crate::sc_frame::{ScFunction, ScMessage};

pub(super) const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 30_000;
pub(super) const DEFAULT_HEARTBEAT_TIMEOUT_MS: u64 = 60_000;

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
