use crate::sc_frame::{is_broadcast_vmac, ScMessage, Vmac, BROADCAST_VMAC};

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
