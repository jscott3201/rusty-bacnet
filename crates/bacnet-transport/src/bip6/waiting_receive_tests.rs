//! A foreign device waiting for its BBMD's host name (#2365) has no BBMD to
//! take forwarded messages from, so it takes none: not by unicast, and not
//! on a BACnet group, as a normal node would.

use std::sync::Arc;

use super::foreign_target::ForeignTarget;
use super::receive_counters_tests::{
    GROUP, LOCAL_VMAC, NPDU, Node, forwarded, forwarded_resolution, local_ip, only, peer, station,
};
use super::*;
use crate::bbmd_lookup::SystemResolver;

/// A node waiting for its BBMD's host name, which the receive path never
/// looks up.
async fn waiting() -> Node {
    let config = Bip6ForeignDeviceConfig {
        bbmd: "bbmd.test:47808".parse().unwrap(),
        ttl: 60,
    };
    let target = ForeignTarget::new(config, Arc::new(SystemResolver));
    Node::with_target(Some(target), 8).await
}

#[tokio::test]
async fn a_waiting_device_takes_no_forwarded_npdu() {
    let mut node = waiting().await;
    // A normal node takes this one, from a group.
    node.feed(&forwarded(station(), NPDU), peer(), GROUP).await;
    node.feed(&forwarded(station(), NPDU), peer(), local_ip())
        .await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.untrusted_forwarded_npdu = 2));
}

#[tokio::test]
async fn a_waiting_device_answers_no_forwarded_address_resolution() {
    let node = waiting().await;
    let request = forwarded_resolution(LOCAL_VMAC, station());
    // A normal node answers this one, from a group.
    assert_eq!(node.send(request, peer(), GROUP).await, None);
    assert_eq!(node.send(request, peer(), local_ip()).await, None);
    assert_eq!(
        node.counters(),
        only(|c| c.untrusted_forwarded_resolution = 2)
    );
}
