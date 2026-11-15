//! A network layer stopped through a shared reference (#2617): the owner of
//! an `Arc<NetworkLayer>` stops it while another holder, such as a client,
//! still has it, and from then on every send through it fails.

use std::sync::Arc;

use super::*;
use bacnet_transport::test_transport::{StartMode, TestTransport};
use tokio::time::{Duration, timeout};

const WHO_IS: &[u8] = &[0x10, 0x08];

fn stopped(sent: &Result<(), Error>) -> bool {
    matches!(sent, Err(Error::Transport(io)) if io.kind() == std::io::ErrorKind::NotConnected)
}

#[tokio::test(start_paused = true)]
async fn a_layer_stopped_with_another_holder_alive_sends_nothing() {
    let transport = TestTransport::builder().start(StartMode::Fresh(4)).build();
    let handle = transport.handle();
    let mut network = NetworkLayer::new(transport);
    let mut apdus = network.start().await.unwrap();
    let network = Arc::new(network);
    let holder = Arc::clone(&network);
    holder
        .broadcast_apdu(WHO_IS, false, NetworkPriority::Normal)
        .await
        .unwrap();

    // Stops beside each other both return once the transport has stopped.
    let (first, second) = tokio::join!(network.stop(), holder.stop());
    first.unwrap();
    second.unwrap();
    assert!(handle.stops() >= 1);
    for sent in [
        holder
            .broadcast_apdu(WHO_IS, false, NetworkPriority::Normal)
            .await,
        holder
            .send_apdu(WHO_IS, &[0x02], false, NetworkPriority::Normal)
            .await,
        holder
            .broadcast_global_apdu(WHO_IS, false, NetworkPriority::Normal)
            .await,
    ] {
        assert!(stopped(&sent), "{sent:?}");
    }
    assert_eq!(
        handle.sent().len(),
        1,
        "nothing reached the link after the stop"
    );
    // The receiver ends; a stop again changes nothing.
    assert!(
        timeout(Duration::from_secs(1), apdus.recv())
            .await
            .unwrap()
            .is_none()
    );
    holder.stop().await.unwrap();
    // The inbound sender the fresh channel had is gone.
    assert!(handle.inbound().is_none());
}

/// The dispatch task's future is gone: its watch sender went with it.
fn dispatch_ended<T: TransportPort>(network: &NetworkLayer<T>) -> bool {
    let dispatch = network.dispatch.as_ref().expect("a started layer");
    dispatch.ended.has_changed().is_err()
}

/// A second stop beside the first returns only once the dispatch task has
/// ended, as the first does, not as soon as it finds the task aborted.
#[tokio::test]
async fn a_second_stop_waits_for_the_dispatch_task_too() {
    let mut network = NetworkLayer::new(TestTransport::builder().build());
    let _apdus = network.start_with_admission().await.unwrap();
    assert!(!dispatch_ended(&network));
    let (first, second) = tokio::join!(
        async {
            network.stop().await.unwrap();
            dispatch_ended(&network)
        },
        async {
            network.stop().await.unwrap();
            dispatch_ended(&network)
        },
    );
    assert!(first, "the first stop returned before the task ended");
    assert!(second, "the second stop returned before the task ended");
}
