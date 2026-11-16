//! WriteGroup to each destination: one device, this network, a remote
//! network, the client's own network once it knows the number, and every
//! network. The pins were recorded from `write_group` to each
//! `WriteGroupDestination` before WriteGroup took a `Broadcast` scope.

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
    pin!("wg/device", |c| c.write_group(
        &WriteGroupDestination::Device(MacAddr::from_slice(PEER)),
        &request
    ));
    pin!("wg/local", |c| c.write_group(
        &WriteGroupDestination::LocalBroadcast,
        &request
    ));
    pin!("wg/network", |c| c.write_group(
        &WriteGroupDestination::RemoteBroadcast(DNET),
        &request
    ));
    pin!("wg/own", |c| async {
        c.network
            .local_network_number()
            .publish(NetworkNumber::configured(OWN).unwrap());
        c.write_group(&WriteGroupDestination::RemoteBroadcast(OWN), &request)
            .await
    });
    pin!("wg/global", |c| c.write_group(
        &WriteGroupDestination::GlobalBroadcast,
        &request
    ));
}
