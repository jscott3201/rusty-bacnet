use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::sync::Mutex;

use super::{Clients, Vmac, WsSink};

pub(super) const NO_PENDING_HEARTBEAT: u16 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HubHeartbeatSweepDecision {
    Keep,
    SendRequest,
    RemoveTimedOut,
}

pub(super) fn hub_heartbeat_sweep_decision(
    now_secs: u64,
    last_activity_secs: u64,
    pending_message_id: u16,
    pending_sent_at_secs: u64,
    idle_threshold_secs: u64,
    ack_timeout_secs: u64,
) -> HubHeartbeatSweepDecision {
    if pending_message_id != NO_PENDING_HEARTBEAT {
        return if now_secs.saturating_sub(pending_sent_at_secs) > ack_timeout_secs {
            HubHeartbeatSweepDecision::RemoveTimedOut
        } else {
            HubHeartbeatSweepDecision::Keep
        };
    }

    if now_secs.saturating_sub(last_activity_secs) > idle_threshold_secs {
        HubHeartbeatSweepDecision::SendRequest
    } else {
        HubHeartbeatSweepDecision::Keep
    }
}

pub(super) fn heartbeat_ack_matches_pending(message_id: u16, pending_message_id: u16) -> bool {
    pending_message_id != NO_PENDING_HEARTBEAT && pending_message_id == message_id
}

pub(super) async fn clear_matching_heartbeat_ack(
    clients: &Clients,
    registered_vmac: Vmac,
    sink: &Arc<Mutex<WsSink>>,
    message_id: u16,
) {
    let map = clients.lock().await;
    let Some(client) = map
        .get(&registered_vmac)
        .filter(|client| Arc::ptr_eq(&client.sink, sink))
    else {
        return;
    };

    let pending = client.pending_heartbeat_id.load(Ordering::Acquire);
    if heartbeat_ack_matches_pending(message_id, pending) {
        client
            .pending_heartbeat_id
            .store(NO_PENDING_HEARTBEAT, Ordering::Release);
        client.pending_heartbeat_sent_at.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_keeps_active_client_without_pending_heartbeat() {
        assert_eq!(
            hub_heartbeat_sweep_decision(100, 80, NO_PENDING_HEARTBEAT, 0, 60, 5),
            HubHeartbeatSweepDecision::Keep
        );
    }

    #[test]
    fn sweep_sends_request_to_idle_client_without_pending_heartbeat() {
        assert_eq!(
            hub_heartbeat_sweep_decision(100, 39, NO_PENDING_HEARTBEAT, 0, 60, 5),
            HubHeartbeatSweepDecision::SendRequest
        );
    }

    #[test]
    fn sweep_keeps_client_while_pending_heartbeat_is_within_ack_timeout() {
        assert_eq!(
            hub_heartbeat_sweep_decision(104, 30, 0x8000, 100, 60, 5),
            HubHeartbeatSweepDecision::Keep
        );
    }

    #[test]
    fn sweep_removes_client_when_pending_heartbeat_exceeds_ack_timeout() {
        assert_eq!(
            hub_heartbeat_sweep_decision(106, 30, 0x8000, 100, 60, 5),
            HubHeartbeatSweepDecision::RemoveTimedOut
        );
    }

    #[test]
    fn heartbeat_ack_must_match_pending_message_id() {
        assert!(heartbeat_ack_matches_pending(0x8000, 0x8000));
        assert!(!heartbeat_ack_matches_pending(0x8001, 0x8000));
        assert!(!heartbeat_ack_matches_pending(0x8000, NO_PENDING_HEARTBEAT));
    }
}
