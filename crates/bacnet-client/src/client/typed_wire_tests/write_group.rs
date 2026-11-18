//! WriteGroup to each destination: one device, this network, a remote
//! network, the client's own network once it knows the number, and every
//! network. The pins were recorded from `write_group` to each
//! `WriteGroupDestination` before WriteGroup took a `Broadcast` scope;
//! `wg/routed`, to a station behind a router, is newer: no destination
//! named one.

use std::num::NonZeroU32;

use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
use bacnet_types::network_number::NetworkNumber;

use super::*;

/// The client's own network, once it knows the number.
const OWN: u16 = 7;

/// Group 23 at priority 8: channel 12 to REAL 67.0, channel 13 relinquished
/// at priority 10.
fn request() -> WriteGroupRequest {
    WriteGroupRequest {
        group_number: NonZeroU32::new(23).unwrap(),
        write_priority: 8,
        change_list: vec![
            GroupChannelValue {
                channel: 12,
                override_priority: None,
                value: vec![0x44, 0x42, 0x86, 0x00, 0x00],
            },
            GroupChannelValue {
                channel: 13,
                override_priority: Some(10),
                value: vec![0x00],
            },
        ],
        inhibit_delay: Some(false),
    }
}

#[tokio::test]
async fn write_group_to_each_destination() {
    let request = request();
    pin!("wg/device", |c| c.write_group_directed(PEER, &request));
    pin!("wg/local", |c| c.write_group(Broadcast::Local, &request));
    pin!("wg/network", |c| c
        .write_group(Broadcast::Network(DNET), &request));
    pin!("wg/own", |c| async {
        c.network
            .local_network_number()
            .publish(NetworkNumber::configured(OWN).unwrap());
        c.write_group(Broadcast::Network(OWN), &request).await
    });
    pin!("wg/global", |c| c.write_group(Broadcast::Global, &request));
    // A station behind a router, which no WriteGroupDestination named.
    pin!("wg/routed", |c| c.write_group_directed(
        &Station::routed(ROUTER, DNET, DADR),
        &request
    ));
}

/// Network 0 and 65535 are refused before anything is sent; a node that
/// may not initiate, and then a closed seat, refuse every scope and the
/// directed form, as for the other unconfirmed calls.
#[tokio::test]
async fn write_group_is_refused_as_the_other_scoped_calls_are() {
    let wire = wire().await;
    let c = &wire.client;
    let request = request();
    for network in [0, u16::MAX] {
        let refused = c.write_group(Broadcast::Network(network), &request).await;
        assert!(
            matches!(&refused, Err(Error::Encoding(text)) if text.contains("Broadcast::Global")),
            "{refused:?}"
        );
    }
    for sealed in [false, true] {
        if sealed {
            c.device_table.asking.restrict(false);
            c.device_table.asking.seal();
        } else {
            c.device_table.asking.restrict(true);
        }
        let mut results = Vec::new();
        for scope in [
            Broadcast::Local,
            Broadcast::Network(DNET),
            Broadcast::Global,
        ] {
            results.push(c.write_group(scope, &request).await);
        }
        results.push(c.write_group_directed(PEER, &request).await);
        for result in results {
            if sealed {
                assert!(
                    matches!(&result, Err(Error::Encoding(text)) if text.contains("seat has closed")),
                    "{result:?}"
                );
            } else {
                assert!(
                    matches!(result, Err(Error::InitiationDisabled { sent: false })),
                    "{result:?}"
                );
            }
        }
    }
    assert_eq!(wire.sent.len(), 0, "nothing sent");
}
