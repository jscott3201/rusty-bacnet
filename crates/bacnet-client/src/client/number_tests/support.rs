use super::super::{BACnetClient, ClientConfig};
use bacnet_transport::port::{ReceivedNpdu, TransportPort, TransportProvenance};
use bacnet_types::{MacAddr, error::Error};
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
pub struct Capture {
    inbound: Option<mpsc::Receiver<ReceivedNpdu>>,
    outbound: mpsc::Sender<Sent>,
    gates: Arc<Gates>,
    hold_number: bool,
}
impl TransportPort for Capture {
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        true
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.inbound.take().unwrap())
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.gates.stop_entered.add_permits(1);
        if self.gates.hold_stop.load(Ordering::SeqCst) {
            self.gates.stop_release.acquire().await.unwrap().forget();
        }
        Ok(())
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.outbound
            .send(Sent {
                npdu: Bytes::copy_from_slice(npdu),
                destination: MacAddr::from_slice(mac),
            })
            .await
            .unwrap();
        Ok(())
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        if self.hold_number {
            let gates = &self.gates;
            let _drop = Signal(&gates.dropped);
            gates.entered.add_permits(1);
            gates.release.acquire().await.unwrap().forget();
        }
        self.send_unicast(npdu, &[]).await
    }
    fn local_mac(&self) -> &[u8] {
        &[1]
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }
    fn egress_apdu_limit(&self) -> u16 {
        1476
    }
}
pub async fn harness(
    hold: bool,
) -> (
    BACnetClient<Capture>,
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
        Capture {
            inbound: Some(rx),
            outbound,
            gates: gates.clone(),
            hold_number: hold,
        },
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

impl Drop for Capture {
    fn drop(&mut self) {
        self.gates.transport_dropped.add_permits(1);
    }
}
pub fn number(value: u16, flag: u8) -> Vec<u8> {
    vec![1, 0x80, 0x13, (value >> 8) as u8, value as u8, flag]
}
pub async fn reply(outbound: &mut mpsc::Receiver<Sent>, value: u16) {
    let sent = bounded(outbound.recv()).await.unwrap();
    assert!(sent.destination.is_empty());
    assert_eq!(sent.npdu.as_ref(), number(value, 0));
}
