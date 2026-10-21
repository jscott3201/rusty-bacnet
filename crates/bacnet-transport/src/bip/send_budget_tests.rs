//! The transport sends NPDUs up to the 1497 octets B/IP carries (Table
//! 6-1), and refuses longer ones with an error, counted in
//! `send_counters()` (#1622).

use std::time::Duration;

use super::registration_schedule_tests::Device;
use super::*;

const LONGEST: usize = bacnet_npdu::MAX_NPDU;

#[tokio::test]
async fn an_npdu_past_1497_octets_is_refused_and_counted() {
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let to = encode_bip_mac(
        Ipv4Addr::LOCALHOST.octets(),
        peer.local_addr().unwrap().port(),
    );
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    // What it reports to a router, from the running node's link (#1665).
    assert_eq!(transport.max_npdu(), NpduLimits::both(LONGEST as u16));

    transport.send_unicast(&[0x01; LONGEST], &to).await.unwrap();
    let mut packet = [0; 2048];
    let (len, _) = tokio::time::timeout(Duration::from_secs(2), peer.recv_from(&mut packet))
        .await
        .expect("the longest NPDU goes")
        .unwrap();
    assert_eq!(len, 4 + LONGEST);

    let over = [0x01; LONGEST + 1];
    for refused in [
        transport.send_unicast(&over, &to).await,
        transport.send_broadcast(&over).await,
    ] {
        let error = refused.unwrap_err();
        assert!(matches!(error, Error::Encoding(_)), "{error:?}");
        assert!(error.to_string().contains("1497 octets"), "{error}");
    }
    assert_eq!(transport.send_counters().npdu_too_long, 2);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), peer.recv_from(&mut packet))
            .await
            .is_err(),
        "nothing more was sent"
    );
    // The totals survive a restart.
    transport.stop().await.unwrap();
    let _rx = transport.start().await.unwrap();
    assert_eq!(transport.send_counters().npdu_too_long, 2);
    transport.stop().await.unwrap();
}

/// A foreign device refuses a broadcast past the budget before it would
/// hold it for the first registration.
#[tokio::test(start_paused = true)]
async fn a_foreign_device_refuses_a_long_broadcast_before_holding_it() {
    let device = Device::new(Duration::from_secs(10)).await;
    let over = [0x01; LONGEST + 1];
    assert!(device.ctx.node.send_broadcast(&over, None).await.is_err());
    device
        .ctx
        .node
        .send_broadcast(&[0x01; LONGEST], None)
        .await
        .unwrap();
    let registration = device.ctx.node.snapshot().foreign_registration;
    assert_eq!(registration.broadcasts_held, 1);
    assert_eq!(registration.broadcasts_hold_full, 0);
}
