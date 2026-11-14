use super::super::{BACnetClient, ClientConfig};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::MacAddr;
use bytes::Bytes;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Semaphore, mpsc};

pub const QUERY: &[u8] = &[1, 0x80, 0x12];
pub async fn bounded<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(3), f)
        .await
        .expect("client Number fixture made no progress")
}
pub struct Sent {
    pub npdu: Bytes,
    pub destination: MacAddr,
}
pub struct Gates {
    pub entered: Semaphore,
    pub release: Semaphore,
    pub dropped: Semaphore,
    pub stop_entered: Semaphore,
    pub stop_release: Semaphore,
    pub hold_stop: AtomicBool,
    pub transport_dropped: Semaphore,
}
struct Signal<'a>(&'a Semaphore);
impl Drop for Signal<'_> {
    fn drop(&mut self) {
        self.0.add_permits(1);
    }
}
/// A link that takes the local Number controls, records every send (either
/// kind) in `outbound`, and holds the stop and, with `hold_number`, each
/// broadcast, on `gates`.
pub fn capture(
    inbound: mpsc::Receiver<ReceivedNpdu>,
    outbound: mpsc::Sender<Sent>,
    gates: Arc<Gates>,
    hold_number: bool,
) -> TestTransport {
    let (stop_gates, drop_gates) = (gates.clone(), gates.clone());
    TestTransport::builder()
        .local_mac(&[1])
        .inbound(inbound)
        .number_controls()
        .on_stop(move || {
            let gates = stop_gates.clone();
            async move {
                gates.stop_entered.add_permits(1);
                if gates.hold_stop.load(Ordering::SeqCst) {
                    gates.stop_release.acquire().await.unwrap().forget();
                }
                Ok(())
            }
        })
        .on_send(move |frame| {
            let (outbound, gates) = (outbound.clone(), gates.clone());
            async move {
                if frame.broadcast && hold_number {
                    let _drop = Signal(&gates.dropped);
                    gates.entered.add_permits(1);
                    gates.release.acquire().await.unwrap().forget();
                }
                outbound
                    .send(Sent {
                        npdu: frame.npdu,
                        destination: frame.mac,
                    })
                    .await
                    .unwrap();
                Ok(())
            }
        })
        .on_drop(move || drop_gates.transport_dropped.add_permits(1))
        .build()
}
pub async fn harness(
    hold: bool,
) -> (
    BACnetClient<TestTransport>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::Receiver<Sent>,
    Arc<Gates>,
) {
    let (inbound, rx) = mpsc::channel(64);
    let (outbound, tx) = mpsc::channel(64);
    let gates = Arc::new(Gates {
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
        dropped: Semaphore::new(0),
        stop_entered: Semaphore::new(0),
        stop_release: Semaphore::new(0),
        hold_stop: AtomicBool::new(false),
        transport_dropped: Semaphore::new(0),
    });
    let client = BACnetClient::start(
        ClientConfig {
            apdu_timeout_ms: 5000,
            ..Default::default()
        },
        capture(rx, outbound, gates.clone(), hold),
    )
    .await
    .unwrap();
    (client, inbound, tx, gates)
}
pub async fn inject(inbound: &mpsc::Sender<ReceivedNpdu>, npdu: &[u8], group: bool) {
    deliver(inbound, npdu, &[2], group).await;
}
/// Hand the client `npdu` by unicast from link MAC `source`.
pub async fn inject_from(inbound: &mpsc::Sender<ReceivedNpdu>, npdu: &[u8], source: &[u8]) {
    deliver(inbound, npdu, source, false).await;
}
async fn deliver(inbound: &mpsc::Sender<ReceivedNpdu>, npdu: &[u8], source: &[u8], group: bool) {
    inbound
        .send(ReceivedNpdu {
            npdu: Bytes::copy_from_slice(npdu),
            source_mac: MacAddr::from_slice(source),
            link_layer_group: group,
            data_attributes: vec![],
            provenance: TransportProvenance::unverified(),
            direct_response: None,
            reply_tx: None,
        })
        .await
        .unwrap();
}

pub fn number(value: u16, flag: u8) -> Vec<u8> {
    vec![1, 0x80, 0x13, (value >> 8) as u8, value as u8, flag]
}
pub async fn reply(outbound: &mut mpsc::Receiver<Sent>, value: u16) {
    let sent = bounded(outbound.recv()).await.unwrap();
    assert!(sent.destination.is_empty());
    assert_eq!(sent.npdu.as_ref(), number(value, 0));
}
