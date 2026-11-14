use super::*;
use bacnet_transport::test_transport::{SendMode, TestTransport};
use std::future::{Future, poll_fn};
use std::sync::atomic::AtomicUsize;
use std::task::Poll;

/// A port whose unicast sends hold until dropped, or fail when `fail`: the
/// send log has each frame by the time the hook asserts issuance came first.
fn layer(fail: bool) -> (NetworkLayer<TestTransport>, Arc<AtomicUsize>) {
    let issued = Arc::new(AtomicUsize::new(0));
    let seen = issued.clone();
    let port = TestTransport::builder()
        .local_mac(&[2])
        .broadcast(SendMode::Panic("a broadcast was not expected"))
        .on_send(move |_| {
            assert_eq!(
                seen.load(Ordering::SeqCst),
                1,
                "issuance must precede transport polling"
            );
            async move {
                if fail {
                    Err(Error::Encoding("injected transport failure".into()))
                } else {
                    std::future::pending().await
                }
            }
        })
        .build();
    (NetworkLayer::new(port), issued)
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
        assert!(network.transport.sent().is_empty());
        tokio::pin!(operation);
        poll_fn(|cx| {
            assert!(operation.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(issued.load(Ordering::SeqCst), 1);
        let frames = network.transport.sent().frames();
        assert_eq!(frames.len(), 1);
        let npdu = frames[0].decode_npdu();
        assert_eq!(npdu.destination, destination);
        assert_eq!(npdu.payload.as_ref(), &[0x20, 1, 15]);
        assert_eq!(npdu.priority, NetworkPriority::Urgent);
        assert!(!npdu.expecting_reply);
        assert_eq!(frames[0].mac.as_slice(), &[1]);
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
    assert!(network.transport.sent().is_empty());
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
    assert_eq!(network.transport.sent().len(), 1);
}
