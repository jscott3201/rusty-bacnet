//! The close frame the hub sends as it ends a connection (#2463): status
//! 1000 when it ends one on purpose (a same-UUID replacement retires the
//! old connection, or the peer's Disconnect-Request has been answered), and
//! the status-less frame it always sent when the peer's fault ends it. The
//! graceful shutdown and Connect-wait closes are pinned in their own tests.
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::*;
use tokio::io::AsyncWriteExt;

/// Register `peer` as `vmac` with Device UUID `uuid`.
async fn register(peer: &mut ClientWs, vmac: Vmac, uuid: DeviceUuid) {
    peer.send(request(vmac, uuid)).await.unwrap();
    assert!(matches!(poll_io(peer.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));
}

/// The next frame `peer` reads.
async fn next(peer: &mut ClientWs) -> Message {
    poll_io(peer.next())
        .await
        .expect("peer stream ended")
        .expect("peer transport error")
}

#[tokio::test]
async fn a_replaced_registration_is_closed_with_status_1000() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut old = tls.websocket(hub.address).await;
    register(&mut old, [0x42; 6], [0x42; 16]).await;
    // The same device connects again under another VMAC: the hub retires
    // the old connection, an orderly end.
    let mut new = tls.websocket(hub.address).await;
    register(&mut new, [0x43; 6], [0x42; 16]).await;
    assert_eq!(next(&mut old).await, closed_normally());
    old.flush().await.unwrap();
    hub.hub.stop().await;
}

#[tokio::test]
async fn a_peers_disconnect_request_is_acked_then_closed_with_status_1000() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = tls.websocket(hub.address).await;
    register(&mut peer, [0x42; 6], [0x42; 16]).await;
    let request = ScMessage {
        function: ScFunction::DisconnectRequest,
        message_id: 0x1234,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &request);
    peer.send(Message::Binary(buf.freeze())).await.unwrap();
    let Message::Binary(ack) = next(&mut peer).await else {
        panic!("expected the Disconnect-ACK first");
    };
    assert_eq!(
        decode_sc_message(&ack).unwrap().function,
        ScFunction::DisconnectAck
    );
    assert_eq!(next(&mut peer).await, closed_normally());
    peer.flush().await.unwrap();
    hub.hub.stop().await;
}

/// What a registered peer does wrong.
#[derive(Debug, Clone, Copy)]
enum Fault {
    /// Declares a frame past the hub's size limit: the hub's read fails.
    Oversize,
    /// Sends a second Connect-Request on a connected link.
    SecondConnect,
}

/// The peer's fault ends the connection: not an orderly end, so the close
/// frame keeps no status, as before #2463.
#[tokio::test]
async fn a_connection_the_peers_fault_ends_keeps_its_status_less_close() {
    for fault in [Fault::Oversize, Fault::SecondConnect] {
        let tls = TestTls::new();
        let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
        let mut peer = tls.websocket(hub.address).await;
        register(&mut peer, [0x42; 6], [0x42; 16]).await;
        match fault {
            Fault::Oversize => {
                // A masked binary header declaring 5706 bytes; no body.
                let header = [0x82, 0xfe, 0x16, 0x4a, 1, 2, 3, 4];
                peer.get_mut().write_all(&header).await.unwrap();
                peer.get_mut().flush().await.unwrap();
            }
            Fault::SecondConnect => peer.send(request([0x42; 6], [0x42; 16])).await.unwrap(),
        }
        assert_eq!(next(&mut peer).await, Message::Close(None), "{fault:?}");
        hub.hub.stop().await;
    }
}

/// A Connect-Request the hub turns away, or a Disconnect-Request before
/// any Connect.
#[derive(Debug, Clone, Copy)]
pub(super) enum Refusal {
    /// Another device holds the VMAC.
    DuplicateVmac,
    /// The VMAC is the hub's own.
    HubVmac,
    /// The hub has no room for another client.
    MaxClients,
    /// The operator's admission policy says no.
    Denied,
    /// A Disconnect-Request before any Connect: answered, then closed.
    DisconnectFirst,
}

impl Refusal {
    pub(super) const ALL: [Self; 5] = [
        Self::DuplicateVmac,
        Self::HubVmac,
        Self::MaxClients,
        Self::Denied,
        Self::DisconnectFirst,
    ];

    /// The VMAC the refused request claims (unused for `DisconnectFirst`).
    pub(super) fn vmac(self) -> Vmac {
        match self {
            Self::DuplicateVmac => INCUMBENT,
            Self::HubVmac => [0x10; 6],
            Self::MaxClients | Self::DisconnectFirst => [0x43; 6],
            Self::Denied => DENIED,
        }
    }

    /// The NAK's class and code, when the refusal sends one.
    pub(super) fn nak(self) -> Option<(ErrorClass, ErrorCode)> {
        match self {
            Self::DuplicateVmac | Self::HubVmac => {
                Some((ErrorClass::COMMUNICATION, ErrorCode::NODE_DUPLICATE_VMAC))
            }
            Self::MaxClients | Self::Denied => Some((ErrorClass::RESOURCES, ErrorCode::OTHER)),
            Self::DisconnectFirst => None,
        }
    }
}

/// The VMAC of the device registered on a [`refusing_hub`].
pub(super) const INCUMBENT: Vmac = [0x42; 6];
/// The VMAC a [`refusing_hub`]'s admission policy denies.
const DENIED: Vmac = [0xbb; 6];

/// A hub with room for one client, which an incumbent holds, and a policy
/// that denies [`DENIED`]. Returns the hub and the incumbent.
pub(super) async fn refusing_hub(tls: &TestTls) -> (ScHub, ClientWs) {
    let config = tls
        .hub_config
        .clone()
        .with_admission_limits(ScHubAdmissionLimits::new(1, 8).unwrap())
        .with_admission_policy(|input| {
            if input.claimed_vmac == DENIED {
                ScHubAdmissionDecision::Deny
            } else {
                ScHubAdmissionDecision::Allow
            }
        });
    let hub = ScHub::start("127.0.0.1:0", config, [0x10; 6], [0x10; 16])
        .await
        .unwrap();
    let mut incumbent = tls.websocket(hub.local_addr().unwrap()).await;
    register(&mut incumbent, INCUMBENT, [0x42; 16]).await;
    (hub, incumbent)
}

/// A refusal ends the connection on purpose (Annex AB.6.2.3): the NAK, or
/// the Disconnect-ACK, goes first, then a close with 1000. Before #2463
/// the hub dropped the connection with no close frame.
#[tokio::test]
async fn a_refusal_answers_first_then_closes_with_status_1000() {
    for refusal in Refusal::ALL {
        let tls = TestTls::new();
        let (mut hub, _incumbent) = refusing_hub(&tls).await;
        let mut peer = tls.websocket(hub.local_addr().unwrap()).await;
        let sent = match refusal {
            Refusal::DisconnectFirst => {
                let request = ScMessage {
                    function: ScFunction::DisconnectRequest,
                    message_id: 0x0102,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::new(),
                };
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &request);
                Message::Binary(buf.freeze())
            }
            _ => request(refusal.vmac(), [0x43; 16]),
        };
        peer.send(sent).await.unwrap();
        match refusal.nak() {
            Some((class, code)) => {
                let Message::Binary(nak) = next(&mut peer).await else {
                    panic!("{refusal:?}: expected the NAK first");
                };
                let nak = decode_sc_message(&nak).unwrap();
                match crate::sc_frame::decode_sc_bvlc_result(&nak).unwrap() {
                    crate::sc_frame::ScBvlcResult::Nak {
                        result_for,
                        error_class,
                        error_code,
                        ..
                    } => {
                        assert_eq!(result_for, ScFunction::ConnectRequest, "{refusal:?}");
                        assert_eq!((error_class, error_code), (class.to_raw(), code.to_raw()));
                    }
                    other => panic!("{refusal:?}: expected a NAK, got {other:?}"),
                }
            }
            None => {
                let Message::Binary(ack) = next(&mut peer).await else {
                    panic!("expected the Disconnect-ACK first");
                };
                let ack = decode_sc_message(&ack).unwrap();
                assert_eq!(ack.function, ScFunction::DisconnectAck);
            }
        }
        assert_eq!(next(&mut peer).await, closed_normally(), "{refusal:?}");
        hub.stop().await;
    }
}

/// A Connect-Request with a reserved VMAC is malformed, a peer fault: the
/// request check NAKs it with PARAMETER_OUT_OF_RANGE before any refusal
/// is weighed, and then the hub closes with no status, as for its other
/// peer faults (#2463).
#[tokio::test]
async fn a_malformed_connect_request_is_naked_then_closed_without_a_status() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = tls.websocket(hub.address).await;
    peer.send(request([0; 6], [0x43; 16])).await.unwrap();
    let Message::Binary(nak) = next(&mut peer).await else {
        panic!("expected the NAK");
    };
    match crate::sc_frame::decode_sc_bvlc_result(&decode_sc_message(&nak).unwrap()).unwrap() {
        crate::sc_frame::ScBvlcResult::Nak { error_code, .. } => {
            assert_eq!(error_code, ErrorCode::PARAMETER_OUT_OF_RANGE.to_raw());
        }
        other => panic!("expected a NAK, got {other:?}"),
    }
    assert_eq!(next(&mut peer).await, Message::Close(None));
    hub.hub.stop().await;
}

/// A hub whose graceful shutdown waits at most a second for each step.
async fn graceful_hub(tls: &TestTls) -> ScHub {
    let timeouts = ScHubGracefulTimeouts::new(
        std::time::Duration::from_secs(1),
        std::time::Duration::from_secs(1),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    let config = tls.hub_config.clone().with_graceful_timeouts(timeouts);
    ScHub::start("127.0.0.1:0", config, [0x10; 6], [0x10; 16])
        .await
        .unwrap()
}

/// How a registered peer answers the shutdown's Disconnect-Request.
#[derive(Debug, Clone, Copy)]
enum Answer {
    /// Never: the exchange fails at the Ack wait.
    Silence,
    /// With a frame past the size limit: the hub's read fails, a fault.
    Oversize,
}

/// A graceful exchange that fails still closes as a shutdown, with 1001,
/// unless the peer's fault ended it: then the close keeps no status, as a
/// fault does in dispatch (#2463).
#[tokio::test]
async fn a_failed_graceful_exchange_closes_as_going_away_unless_the_peer_faulted() {
    for (answer, expected) in [
        (Answer::Silence, closed_going_away()),
        (Answer::Oversize, Message::Close(None)),
    ] {
        let tls = TestTls::new();
        let mut hub = graceful_hub(&tls).await;
        let mut peer = tls.websocket(hub.local_addr().unwrap()).await;
        register(&mut peer, [0x42; 6], [0x42; 16]).await;
        let peer_side = async {
            let Message::Binary(request) = next(&mut peer).await else {
                panic!("{answer:?}: expected the Disconnect-Request");
            };
            assert_eq!(
                decode_sc_message(&request).unwrap().function,
                ScFunction::DisconnectRequest
            );
            if let Answer::Oversize = answer {
                let header = [0x82, 0xfe, 0x16, 0x4a, 1, 2, 3, 4];
                peer.get_mut().write_all(&header).await.unwrap();
                peer.get_mut().flush().await.unwrap();
            }
            let close = tokio::time::timeout(std::time::Duration::from_secs(5), peer.next())
                .await
                .expect("no close in time")
                .expect("peer stream ended")
                .expect("peer transport error");
            assert_eq!(close, expected, "{answer:?}");
        };
        let (outcome, ()) = tokio::join!(hub.shutdown_gracefully(), peer_side);
        assert_eq!(outcome, ScHubShutdownOutcome::Forced, "{answer:?}");
    }
}
