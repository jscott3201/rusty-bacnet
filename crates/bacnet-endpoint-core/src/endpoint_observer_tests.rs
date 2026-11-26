//! What the source observer is shown of the answers from another network
//! (#2831): only those admission could take.

use super::*;
use crate::coordinator::{LeaseMetadata, OutboundTransactionCoordinator};
use bacnet_encoding::apdu::{Apdu, SimpleAck};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::enums::ConfirmedServiceChoice;

const SERVICE: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION;
const ROUTER: [u8; 1] = [0x10];
const STRANGER: [u8; 1] = [0x66];
const DEVICE: [u8; 1] = [5];

fn answer(from: &[u8], invoke_id: u8) -> ReceivedApdu {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: SERVICE,
        }),
    )
    .unwrap();
    ReceivedApdu {
        router_response: None,
        direct_response: None,
        apdu: apdu.freeze(),
        source_mac: MacAddr::from_slice(from),
        ingress_network: None,
        source_network: Some(NpduAddress {
            network: 2,
            mac_address: MacAddr::from_slice(&DEVICE),
        }),
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

/// An answer for a lease whose attempt went through the router is shown
/// only from the router; one for a lease sent by broadcast, from any
/// station.
#[test]
fn an_answer_from_off_its_leases_path_is_not_shown() {
    let coordinator = OutboundTransactionCoordinator::new();
    let lease = || {
        coordinator
            .reserve(LeaseMetadata::notification(
                CanonicalPeer::routed(2, &DEVICE),
                SERVICE,
            ))
            .unwrap()
    };
    let (through, broadcast) = (lease(), lease());
    coordinator.route_through(through, Some(&ROUTER)).unwrap();
    coordinator.route_through(broadcast, None).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let observer: SourceObserver = {
        let seen = Arc::clone(&seen);
        Arc::new(move |received: &ReceivedApdu| {
            seen.locked()
                .push((received.source_mac.clone(), received.apdu[1]));
        })
    };
    for (from, token) in [
        (STRANGER, through),
        (ROUTER, through),
        (STRANGER, broadcast),
    ] {
        observe_source(
            &observer,
            Some(&coordinator),
            IngressRoute::TerminalOrSegment,
            &answer(&from, token.invoke_id()),
            None,
        );
    }
    assert_eq!(
        *seen.locked(),
        [
            (MacAddr::from_slice(&ROUTER), through.invoke_id()),
            (MacAddr::from_slice(&STRANGER), broadcast.invoke_id()),
        ]
    );
}
