//! Decode observations without activating a protocol participant.
use bacnet_ops::target::AddressStyle;

use crate::tui::network::discovery::DiscoveryEvent;
use bacnet_client::client::ReceivedIHave;
use bacnet_encoding::{
    apdu::{Apdu, decode_apdu},
    npdu::decode_npdu,
};
use bacnet_network::layer::{NetworkReport, ReceivedNetworkReport};
use bacnet_services::who_has::IHaveRequest;
use bacnet_transport::bvll::{Bvll, address_to_octets, decode_bvll};
use bacnet_types::{
    MacAddr,
    enums::{ObjectType, UnconfirmedServiceChoice},
};
use bytes::Bytes;
use std::net::SocketAddrV4;

/// Original and forwarded data only; management/distribution requests are not
/// deliveries. Forwarded origin and routed SNET/SADR retain their claimed identity.
pub(super) fn decode(
    bytes: &[u8],
    peer: SocketAddrV4,
    groups: &bacnet_bip_core::node::Groups,
) -> Option<DiscoveryEvent> {
    if groups.contains_ip(*peer.ip()) {
        return None;
    }
    let frame = decode_bvll(bytes).ok()?;
    let (payload, origin, group) = match frame.message().ok()? {
        Bvll::OriginalUnicastNpdu(npdu) => (npdu, peer, false),
        Bvll::OriginalBroadcastNpdu(npdu) => (npdu, peer, true),
        Bvll::ForwardedNpdu { origin, npdu } if !groups.contains(origin) => (npdu, origin, true),
        _ => return None,
    };
    let npdu = decode_npdu(Bytes::copy_from_slice(payload)).ok()?;
    let source_mac = MacAddr::from_slice(&address_to_octets(origin));
    if npdu.is_network_message() {
        let report = NetworkReport::parse_npdu(&npdu, group)?;
        let source = npdu.source.clone();
        let mut event = super::discovery::report(
            ReceivedNetworkReport {
                source_mac,
                source_network: npdu.source,
                report,
            },
            AddressStyle::Bip,
        )?;
        if let Some(source) = source {
            let (address, via) = AddressStyle::Bip.locate(
                &address_to_octets(origin),
                Some((source.network, source.mac_address.as_slice())),
            );
            let identity = format!("{address} via {}", via.unwrap_or_default());
            match &mut event {
                DiscoveryEvent::Router { router, .. } | DiscoveryEvent::Rejected { router, .. } => {
                    *router = identity
                }
                _ => {}
            }
        }
        return Some(event);
    }
    let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).ok()? else {
        return None;
    };
    if request.service_choice != UnconfirmedServiceChoice::I_HAVE {
        return None;
    }
    let i_have = IHaveRequest::decode(&request.service_request).ok()?;
    if i_have.device_identifier.object_type() != ObjectType::DEVICE {
        return None;
    }
    Some(DiscoveryEvent::IHave(super::discovery::found(
        &ReceivedIHave {
            i_have,
            source_mac,
            source_network: npdu.source,
        },
        AddressStyle::Bip,
    )))
}

#[cfg(test)]
#[path = "passive_decode_tests.rs"]
mod tests;
