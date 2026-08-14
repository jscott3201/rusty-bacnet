use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bytes::BytesMut;
use futures_util::SinkExt;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

use crate::sc_frame::{encode_sc_message, is_broadcast_vmac, ScMessage, Vmac, BROADCAST_VMAC};

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
    msg: &ScMessage,
    registered_vmac: Vmac,
    clients: &Clients,
    source_sink: &Arc<Mutex<WsSink>>,
    close_requested: &Arc<AtomicBool>,
) -> ResultRelayDisposition {
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

    let relay = build_hub_relay_message(msg, registered_vmac, HubRelayTarget::Unicast(destination));
    let mut relay_buf = BytesMut::new();
    encode_sc_message(&mut relay_buf, &relay);
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

    let mut target = sink.lock().await;
    if close_requested.load(Ordering::Acquire) {
        return ResultRelayDisposition::CloseSource;
    }
    if target_closed.load(Ordering::Acquire) {
        return ResultRelayDisposition::Continue;
    }
    if let Err(e) = target
        .send(Message::Binary(relay_buf.to_vec().into()))
        .await
    {
        warn!("Hub: Result relay error to {destination:02x?}: {e}");
    }
    ResultRelayDisposition::Continue
}
