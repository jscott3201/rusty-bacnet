//! The close an outbound direct dial gives a socket it gives up (#2520), on
//! tokio's paused clock with sockets that record each close: no status for
//! a peer that never answers the Connect-Request, 1000 after its refusal,
//! and a Disconnect-Request then 1000 for a peer that turns out to be
//! another node.
use super::*;
use crate::sc::close_test_support::{Probe, Recorder, eventually, recorder};
use crate::sc_frame::{ScMessage, decode_sc_message, encode_sc_message};
use bytes::Bytes;

const LOCAL: Vmac = [0x11; 6];
const REMOTE: Vmac = [0x22; 6];

/// How the dialled peer answers the Connect-Request.
#[derive(Debug, Clone, Copy)]
enum Answer {
    Silence,
    Nak,
    /// A Connect-Accept from a node other than the one dialled.
    WrongPeer,
}

fn reply(function: ScFunction, message_id: u16, payload: Vec<u8>) -> Vec<u8> {
    let message = ScMessage {
        function,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &message);
    buf.to_vec()
}

#[tokio::test(start_paused = true)]
async fn a_direct_dial_closes_the_socket_it_gives_up() {
    for (answer, expected) in [
        (Answer::Silence, None),
        (Answer::Nak, Some(1000)),
        (Answer::WrongPeer, Some(1000)),
    ] {
        let (dialled_tx, mut dialled) = tokio::sync::mpsc::unbounded_channel::<Probe>();
        let direct = DirectShared::<Recorder>::new(Arc::new(DirectMembership::default()));
        *direct.dialer.lock().await = Some(DirectDialer::Custom(Arc::new(move |_uri| {
            let (socket, probe) = recorder();
            let _ = dialled_tx.send(probe);
            Box::pin(async move { Ok(socket) })
        })));
        let peer = tokio::spawn(async move {
            let probe = dialled.recv().await.unwrap();
            let request = decode_sc_message(&probe.hub.recv().await.unwrap()).unwrap();
            assert_eq!(request.function, ScFunction::ConnectRequest);
            let id = request.message_id;
            match answer {
                Answer::Silence => {}
                Answer::Nak => {
                    let nak = vec![0x06, 0x01, 0x00, 0x00, 0x03, 0x00, 0x00];
                    probe
                        .hub
                        .send(&reply(ScFunction::Result, id, nak))
                        .await
                        .unwrap();
                }
                Answer::WrongPeer => {
                    let mut accept = vec![0x33; 6];
                    accept.extend_from_slice(&[0x44; 16]);
                    accept.extend_from_slice(&[0x05, 0xc4, 0x05, 0xc4]);
                    let wire = reply(ScFunction::ConnectAccept, id, accept);
                    probe.hub.send(&wire).await.unwrap();
                    let goodbye = decode_sc_message(&probe.hub.recv().await.unwrap()).unwrap();
                    assert_eq!(goodbye.function, ScFunction::DisconnectRequest);
                }
            }
            probe
        });
        let mut conn = ScConnection::new(LOCAL, [1; 16]);
        conn.state = ScConnectionState::Connected;
        let sent = direct
            .try_direct_uris(
                &["wss://peer.example/sc".to_owned()],
                REMOTE,
                &[1, 0, 0x30],
                &[],
                &Arc::new(Mutex::new(conn)),
                DirectTiming {
                    connect_timeout_ms: 100,
                    heartbeat_ms: (30_000, 60_000),
                },
            )
            .await;
        assert!(sent.is_err(), "{answer:?}: no route was made");
        let probe = peer.await.unwrap();
        eventually("the dial's close", || !probe.closes().is_empty()).await;
        assert_eq!(probe.closes(), [expected], "{answer:?}");
        direct.disable();
    }
}
