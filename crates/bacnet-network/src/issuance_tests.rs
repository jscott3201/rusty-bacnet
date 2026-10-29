use super::*;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::port::ReceivedNpdu;
use std::future::{Future, poll_fn};
use std::sync::{Mutex, atomic::AtomicUsize};
use std::task::Poll;
use tokio::sync::mpsc;

struct HeldPort {
    issued: Arc<AtomicUsize>,
    frames: Mutex<Vec<(Npdu, Vec<u8>)>>,
    fail: bool,
}
impl TransportPort for HeldPort {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(mpsc::channel(1).1)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, wire: &[u8], mac: &[u8]) -> Result<(), Error> {
        assert_eq!(
            self.issued.load(Ordering::SeqCst),
            1,
            "issuance must precede transport polling"
        );
        self.frames.lock().unwrap().push((
            decode_npdu(Bytes::copy_from_slice(wire)).unwrap(),
            mac.to_vec(),
        ));
        if self.fail {
            Err(Error::Encoding("injected transport failure".into()))
        } else {
            std::future::pending().await
        }
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        unreachable!()
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &[2]
    }
}
fn layer(fail: bool) -> (NetworkLayer<HeldPort>, Arc<AtomicUsize>) {
    let issued = Arc::new(AtomicUsize::new(0));
    (
        NetworkLayer::new(HeldPort {
            issued: issued.clone(),
            frames: Mutex::new(Vec::new()),
            fail,
        }),
        issued,
    )
}

#[tokio::test]
async fn issuance_waits_for_poll_and_complete_encoding_but_not_transport_result() {
    for destination in [
        None,
        Some(NpduAddress {
            network: 123,
            mac_address: MacAddr::from_slice(&[3]),
        }),
    ] {
        let (network, issued) = layer(false);
        let operation = network.send_apdu_on_issuance(
            &[0x20, 1, 15],
            &[1],
            destination.as_ref(),
            false,
            NetworkPriority::Urgent,
            || {
                issued.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert_eq!(issued.load(Ordering::SeqCst), 0);
        assert!(network.transport.frames.lock().unwrap().is_empty());
        tokio::pin!(operation);
        poll_fn(|cx| {
            assert!(operation.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(issued.load(Ordering::SeqCst), 1);
        let frames = network.transport.frames.lock().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0.destination, destination);
        assert_eq!(frames[0].0.payload.as_ref(), &[0x20, 1, 15]);
        assert_eq!(frames[0].0.priority, NetworkPriority::Urgent);
        assert!(!frames[0].0.expecting_reply);
        assert_eq!(frames[0].1, vec![1]);
    }
}

#[tokio::test]
async fn encoding_failure_never_issues_and_transport_failure_does_not_undo_issuance() {
    let (network, issued) = layer(true);
    let invalid = NpduAddress {
        network: 0,
        mac_address: MacAddr::from_slice(&[3]),
    };
    let error = network
        .send_apdu_on_issuance(
            &[0x20, 1, 15],
            &[1],
            Some(&invalid),
            false,
            NetworkPriority::Normal,
            || {
                issued.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Encoding(s) if s.contains("DNET")));
    assert_eq!(issued.load(Ordering::SeqCst), 0);
    assert!(network.transport.frames.lock().unwrap().is_empty());
    assert!(
        network
            .send_apdu_on_issuance(
                &[0x20, 1, 15],
                &[1],
                None,
                false,
                NetworkPriority::Normal,
                || {
                    issued.fetch_add(1, Ordering::SeqCst);
                }
            )
            .await
            .is_err()
    );
    assert_eq!(issued.load(Ordering::SeqCst), 1);
    assert_eq!(network.transport.frames.lock().unwrap().len(), 1);
}
