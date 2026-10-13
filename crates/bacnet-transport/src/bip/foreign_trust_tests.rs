//! A foreign device takes a Forwarded-NPDU only from its BBMD by unicast,
//! the way a BBMD hands its foreign devices the network's broadcasts; every
//! other one is dropped and counted in `forwarded_untrusted` (#1622). A
//! node in normal mode still takes one from any sender.

use std::time::Duration;

use super::registration_schedule_tests::{Device, BBMD};
use super::*;

/// A device elsewhere, whose broadcast the Forwarded-NPDU carries.
const ORIGIN: [u8; 6] = [203, 0, 113, 7, 0xBA, 0xC0];
const NPDU: [u8; 3] = [0x01, 0x00, 0x42];

fn forwarded() -> BvllMessage {
    BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: bytes::Bytes::from([&ORIGIN[..], &NPDU[..]].concat()),
    }
}

#[tokio::test(start_paused = true)]
async fn a_foreign_device_takes_forwards_only_from_its_bbmd_by_unicast() {
    let mut device = Device::new(Duration::from_secs(10)).await;
    let from_bbmd_ip_other_port = (BBMD.0, BBMD.1 + 1);
    let from_elsewhere = ([192, 0, 2, 30], BBMD.1);
    for (sender, delivery) in [
        (BBMD, Delivery::Broadcast),
        (from_elsewhere, Delivery::Unicast),
        (from_elsewhere, Delivery::Broadcast),
        (from_bbmd_ip_other_port, Delivery::Unicast),
    ] {
        device.hears(&forwarded(), sender, delivery).await;
    }
    assert!(device.npdus.try_recv().is_err(), "nothing handed up");
    let refused = device.ctx.drops.snapshot();
    assert_eq!(refused.forwarded_untrusted, 4);
    assert_eq!(
        ReceiveCounters {
            forwarded_untrusted: 0,
            ..refused
        },
        ReceiveCounters::default(),
        "counted once, under that reason alone"
    );

    device.hears(&forwarded(), BBMD, Delivery::Unicast).await;
    let delivered = device.npdus.try_recv().expect("the BBMD's forward");
    assert_eq!(delivered.npdu.as_ref(), NPDU);
    assert_eq!(delivered.source_mac.as_slice(), ORIGIN);
    assert!(delivered.link_layer_group);
    assert_eq!(device.ctx.drops.snapshot().forwarded_untrusted, 4);
}

#[tokio::test]
async fn a_normal_node_takes_forwards_from_any_sender() {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let socket = Arc::new(BipSocket::recording(socket, send_sink::SendSink::default()));
    let (npdu_tx, mut npdus) = mpsc::channel(4);
    let ctx = RecvContext::for_test(
        socket,
        npdu_tx,
        SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 0xBAC0),
        SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 255), 0xBAC0),
        None,
    );
    for delivery in [Delivery::Unicast, Delivery::Broadcast] {
        handle_bvll_message(&forwarded(), ([192, 0, 2, 30], 0xBAC0), delivery, &ctx).await;
        let delivered = npdus.try_recv().expect("handed up");
        assert_eq!(delivered.source_mac.as_slice(), ORIGIN);
    }
    assert_eq!(ctx.drops.snapshot(), ReceiveCounters::default());
}
