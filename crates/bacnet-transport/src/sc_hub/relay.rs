use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bytes::{BufMut, BytesMut};
use futures_util::SinkExt;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

use crate::sc_frame::{
    decode_sc_bvlc_result, is_broadcast_vmac, ScBvlcResult, ScFunction, ScMessage, Vmac,
    BROADCAST_VMAC,
};

use super::helpers::registered_client_matches_sink_in_map;
use super::{Clients, WsSink};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HubRelayTarget {
    Unicast(Vmac),
    Broadcast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HubRelayReject {
    OriginatingVmacPresent,
    MissingDestinationVmac,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResultRelayDisposition {
    Continue,
    CloseSource,
}

pub(super) fn hub_relay_target(msg: &ScMessage) -> Result<HubRelayTarget, HubRelayReject> {
    if msg.originating_vmac.is_some() {
        return Err(HubRelayReject::OriginatingVmacPresent);
    }
    let destination = msg
        .destination_vmac
        .ok_or(HubRelayReject::MissingDestinationVmac)?;
    if is_broadcast_vmac(&destination) {
        Ok(HubRelayTarget::Broadcast)
    } else {
        Ok(HubRelayTarget::Unicast(destination))
    }
}

pub(super) fn build_hub_relay_message(
    inbound: &ScMessage,
    sender_vmac: Vmac,
    target: HubRelayTarget,
) -> ScMessage {
    let mut relay = inbound.clone();
    relay.originating_vmac = Some(sender_vmac);
    relay.destination_vmac = match target {
        HubRelayTarget::Unicast(_) => None,
        HubRelayTarget::Broadcast => Some(BROADCAST_VMAC),
    };
    relay
}

pub(super) fn encode_hub_relay_frame(
    inbound_wire: &[u8],
    inbound: &ScMessage,
    sender_vmac: Vmac,
    target: HubRelayTarget,
) -> Option<BytesMut> {
    let body_offset = 4
        + usize::from(inbound.originating_vmac.is_some()) * 6
        + usize::from(inbound.destination_vmac.is_some()) * 6;
    let body = inbound_wire.get(body_offset..)?;
    let message_id = inbound_wire.get(2..4)?;

    let mut control = *inbound_wire.get(1)? & !(0x08 | 0x04);
    control |= 0x08;
    if target == HubRelayTarget::Broadcast {
        control |= 0x04;
    }

    let mut relay = BytesMut::with_capacity(inbound_wire.len() + 6);
    relay.put_u8(*inbound_wire.first()?);
    relay.put_u8(control);
    relay.put_slice(message_id);
    relay.put_slice(&sender_vmac);
    if target == HubRelayTarget::Broadcast {
        relay.put_slice(&BROADCAST_VMAC);
    }
    // Copy options and payload from the validated frame so marker bits that
    // ScOption cannot represent survive hub forwarding.
    relay.put_slice(body);
    Some(relay)
}

pub(super) fn hub_relay_recipient_vmacs(
    target: HubRelayTarget,
    sender_vmac: Vmac,
    connected_vmacs: impl IntoIterator<Item = Vmac>,
) -> Vec<Vmac> {
    match target {
        HubRelayTarget::Unicast(destination) => connected_vmacs
            .into_iter()
            .filter(|vmac| *vmac == destination)
            .collect(),
        HubRelayTarget::Broadcast => connected_vmacs
            .into_iter()
            .filter(|vmac| *vmac != sender_vmac)
            .collect(),
    }
}

pub(super) async fn relay_result(
    wire: &[u8],
    msg: &ScMessage,
    registered_vmac: Vmac,
    clients: &Clients,
    source_sink: &Arc<Mutex<WsSink>>,
    close_requested: &Arc<AtomicBool>,
) -> ResultRelayDisposition {
    let result_for = match decode_sc_bvlc_result(msg) {
        Ok(ScBvlcResult::Ack { result_for }) | Ok(ScBvlcResult::Nak { result_for, .. }) => {
            result_for
        }
        Err(e) => {
            debug!("Hub: malformed peer Result from {registered_vmac:02x?}, dropping: {e}");
            return ResultRelayDisposition::Continue;
        }
    };
    if result_for != ScFunction::EncapsulatedNpdu {
        debug!(
            "Hub: peer Result for {:?} from {registered_vmac:02x?}, dropping",
            result_for
        );
        return ResultRelayDisposition::Continue;
    }

    let destination = match hub_relay_target(msg) {
        Ok(HubRelayTarget::Unicast(destination)) => destination,
        Ok(HubRelayTarget::Broadcast) => {
            debug!("Hub: broadcast Result from {registered_vmac:02x?}, dropping");
            return ResultRelayDisposition::Continue;
        }
        Err(HubRelayReject::OriginatingVmacPresent) => {
            debug!("Hub: Result from {registered_vmac:02x?} had Originating VMAC, dropping");
            return ResultRelayDisposition::Continue;
        }
        Err(HubRelayReject::MissingDestinationVmac) => {
            debug!("Hub: Result from {registered_vmac:02x?} had no relay destination, dropping");
            return ResultRelayDisposition::Continue;
        }
    };

    let Some(relay_buf) = encode_hub_relay_frame(
        wire,
        msg,
        registered_vmac,
        HubRelayTarget::Unicast(destination),
    ) else {
        warn!("Hub: failed to preserve peer Result frame from {registered_vmac:02x?}");
        return ResultRelayDisposition::Continue;
    };
    let relay_len = relay_buf.len();

    let target = {
        let map = clients.lock().await;
        if !registered_client_matches_sink_in_map(&map, registered_vmac, source_sink) {
            return ResultRelayDisposition::CloseSource;
        }
        map.get(&destination).map(|client| {
            (
                Arc::clone(&client.sink),
                Arc::clone(&client.closed),
                client.max_bvlc,
            )
        })
    };

    let Some((sink, target_closed, max_bvlc)) = target else {
        debug!("Hub: no client with vmac {destination:02x?} for Result relay");
        return ResultRelayDisposition::Continue;
    };
    if relay_len > max_bvlc as usize {
        warn!(
            "Hub: Result BVLC ({relay_len} bytes) exceeds target max_bvlc ({max_bvlc}) for {destination:02x?}, dropping"
        );
        return ResultRelayDisposition::Continue;
    }
    if close_requested.load(Ordering::Acquire) {
        return ResultRelayDisposition::CloseSource;
    }
    if target_closed.load(Ordering::Acquire) {
        return ResultRelayDisposition::Continue;
    }

    let send = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut target = sink.lock().await;
        if close_requested.load(Ordering::Acquire) {
            return ResultRelayDisposition::CloseSource;
        }
        if target_closed.load(Ordering::Acquire) {
            return ResultRelayDisposition::Continue;
        }
        if target
            .send(Message::Binary(relay_buf.to_vec().into()))
            .await
            .is_err()
        {
            warn!("Hub: Result relay failed to {destination:02x?}");
        }
        ResultRelayDisposition::Continue
    })
    .await;
    if let Ok(disposition) = send {
        if disposition == ResultRelayDisposition::CloseSource
            || close_requested.load(Ordering::Acquire)
        {
            return ResultRelayDisposition::CloseSource;
        }
        return ResultRelayDisposition::Continue;
    }
    if close_requested.load(Ordering::Acquire) {
        return ResultRelayDisposition::CloseSource;
    }
    warn!("Hub: Result relay timed out to {destination:02x?}");
    ResultRelayDisposition::Continue
}
