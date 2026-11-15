//! Standalone client Number controls over actual constrained TLS SC sockets.
#![cfg(feature = "sc-tls")]
#[path = "support/port_retry.rs"]
mod port_retry;
#[path = "sc_client_network_numbers/support.rs"]
mod support;
#[path = "sc_network_numbers/tls.rs"]
mod tls;
use bacnet_transport::sc_frame::BROADCAST_VMAC;
use bacnet_types::enums::ConfirmedServiceChoice;
use port_retry::rerun_on_lost_port;
use support::*;

#[tokio::test]
async fn sc_client_number_hub_direct_wire_and_confirmed_progress() {
    // The closing release probes can lose a port to another process (#1070).
    rerun_on_lost_port(async || {
        let mut f = Fixture::start().await;
        // UNKNOWN client has no configured number or Network Port authority.
        f.peer.send(NODE, QUERY).await;
        f.peer.send(BROADCAST_VMAC, QUERY).await;
        // Keep this initial batch within the default four-per-origin SC intake
        // quota. Unicast NNI refusal has its own later positive reply fence.
        f.peer
            .send(BROADCAST_VMAC, &[1, 0x80, 0x13, 0, 77, 0])
            .await;
        // Clause 6.4.19 allows a question to the node alone, and it is
        // answered at once. A broadcast one's answer waits up to ten seconds
        // (#2597), which is tested on paused time, not on this wall clock.
        f.peer.send(NODE, &[1, 0x80, 0x12]).await;
        f.peer.number(77).await;
        // Learned values update until configured evidence takes precedence.
        for (number, flag, expected) in [(78, 0, 78), (79, 1, 79), (80, 0, 79), (81, 1, 81)] {
            f.peer
                .send(BROADCAST_VMAC, &[1, 0x80, 0x13, 0, number, flag])
                .await;
            f.peer.send(NODE, &[1, 0x80, 0x12]).await;
            f.peer.number(expected).await;
        }
        for (dest, invalid) in [
            (NODE, vec![1, 0x80, 0x13, 0, 200, 1]), // unicast cannot teach
            (BROADCAST_VMAC, vec![1, 0x80, 0x13, 0, 0, 1]),
            (BROADCAST_VMAC, vec![1, 0x80, 0x13, 255, 255, 1]),
            (BROADCAST_VMAC, vec![1, 0x80, 0x13, 0, 200, 2]),
            (BROADCAST_VMAC, vec![1, 0x80, 0x13, 0, 200]),
            (BROADCAST_VMAC, vec![1, 0x80, 0x13, 0, 200, 1, 0]),
            (BROADCAST_VMAC, vec![1, 0x88, 0, 4, 1, 9, 0x13, 0, 200, 1]),
            (
                BROADCAST_VMAC,
                vec![1, 0xa0, 0xff, 0xff, 0, 255, 0x13, 0, 200, 1],
            ),
        ] {
            f.peer.send(dest, &invalid).await;
            f.peer.send(NODE, &[1, 0x80, 0x12]).await;
            f.peer.number(81).await;
        }
        // Any forbidden response here would carry the old number and be observed
        // before the positive new-number response on this same control FIFO.
        for (i, invalid) in [
            vec![1, 0x80, 0x12, 0],
            vec![1, 0x88, 0, 4, 1, 9, 0x12],
            vec![1, 0xa0, 0xff, 0xff, 0, 255, 0x12],
        ]
        .iter()
        .enumerate()
        {
            f.peer.send(NODE, invalid).await;
            let number = 82 + i as u8;
            f.peer
                .send(BROADCAST_VMAC, &[1, 0x80, 0x13, 0, number, 1])
                .await;
            f.peer.send(NODE, &[1, 0x80, 0x12]).await;
            f.peer.number(number.into()).await;
        }
        // A direct query replies through Hub broadcast, with the direct peer
        // still connected during a second query and the ordinary APDU exchange.
        f.direct.direct_query().await;
        f.peer.number(84).await;
        f.direct.direct_query().await;
        f.peer.number(84).await;
        {
            let request = f.client.as_ref().unwrap().confirmed_request(
                &PEER,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &[0],
            );
            tokio::pin!(request);
            let frame = bounded(async {
                tokio::select! {
                    result = &mut request => {
                        panic!("request completed without peer ACK: {result:?}")
                    }
                    frame = f.peer.receive() => frame,
                }
            })
            .await;
            // Hub-to-peer unicast removes the destination VMAC; broadcasts above
            // retain all-FF. The requester advertises segmented-response support.
            assert_eq!(frame.destination_vmac, None);
            let npdu = frame.payload.as_ref();
            assert_eq!(&npdu[..4], &[1, 4, 2, 5]);
            assert_eq!(&npdu[5..], &[15, 0]);
            let invoke = npdu[4];
            f.peer.send(NODE, QUERY).await;
            f.peer.number(84).await;
            f.peer.send(NODE, &[1, 0, 0x20, invoke, 15]).await;
            assert!(bounded(&mut request).await.unwrap().is_empty());
        }
        f.peer.send(NODE, QUERY).await;
        f.peer.number(84).await;
        f.shutdown(false).await
    })
    .await;
}

#[tokio::test]
async fn sc_client_number_real_connections_stop_and_drop_release() {
    for bare_drop in [false, true] {
        // The closing release probes can lose a port to another process
        // (#1070).
        rerun_on_lost_port(async || {
            let mut f = Fixture::start().await;
            f.peer
                .send(BROADCAST_VMAC, &[1, 0x80, 0x13, 0, 77, 1])
                .await;
            // Establish learning on the same Hub reader before the direct query.
            f.peer.send(NODE, QUERY).await;
            f.peer.number(77).await;
            f.direct.direct_query().await;
            f.peer.number(77).await;
            f.shutdown(bare_drop).await
        })
        .await;
    }
}
