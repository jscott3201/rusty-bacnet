//! Socket-free router ports whose sends finish only when the test allows it.

use bacnet_npdu::NpduLimits;
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportPort};
use bacnet_types::{error::Error, MacAddr};
use bytes::Bytes;
use std::sync::{
    atomic::{AtomicU16, Ordering},
    Arc,
};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{timeout, Duration};

use super::{BACnetRouter, RouterOptions, RouterPort};
use crate::layer::ReceivedApdu;

/// One invocation of the transport, with completion controlled by the test.
pub(super) struct Sending {
    pub(super) npdu: Vec<u8>,
    pub(super) to: Option<Vec<u8>>,
    pub(super) attributes: Vec<DataAttribute>,
    pub(super) complete: oneshot::Sender<Result<(), Error>>,
}

pub(super) struct Peer {
    pub(super) frames: mpsc::Sender<ReceivedNpdu>,
    pub(super) limit: Arc<AtomicU16>,
    pub(super) sent: mpsc::UnboundedReceiver<Sending>,
}

struct Port {
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
    sent: mpsc::UnboundedSender<Sending>,
    mac: [u8; 1],
    limit: Arc<AtomicU16>,
}

impl Port {
    async fn send(
        &self,
        npdu: &[u8],
        to: Option<&[u8]>,
        attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        let (complete, completion) = oneshot::channel();
        self.sent
            .send(Sending {
                npdu: npdu.to_vec(),
                to: to.map(<[u8]>::to_vec),
                attributes: attributes.to_vec(),
                complete,
            })
            .map_err(|_| Error::Encoding("test wire closed".into()))?;
        completion
            .await
            .map_err(|_| Error::Encoding("test send refused".into()))?
    }
}

impl TransportPort for Port {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.incoming.take().unwrap())
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.send(npdu, Some(mac), &[]).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send(npdu, None, &[]).await
    }

    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.send(npdu, Some(mac), attributes).await
    }

    async fn send_broadcast_with_data_attributes(
        &self,
        npdu: &[u8],
        attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.send(npdu, None, attributes).await
    }

    fn max_npdu(&self) -> NpduLimits {
        NpduLimits::both(self.limit.load(Ordering::SeqCst))
    }

    fn local_mac(&self) -> &[u8] {
        &self.mac
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }
}

pub(super) async fn start(count: usize) -> (BACnetRouter, mpsc::Receiver<ReceivedApdu>, Vec<Peer>) {
    start_with(count, RouterOptions::new()).await
}

/// [`start`], with `options`.
pub(super) async fn start_with(
    count: usize,
    options: RouterOptions,
) -> (BACnetRouter, mpsc::Receiver<ReceivedApdu>, Vec<Peer>) {
    let (ports, mut peers): (Vec<_>, Vec<_>) = (0..count)
        .map(|index| {
            let (frames, incoming) = mpsc::channel(64);
            let (sent, wire) = mpsc::unbounded_channel();
            let limit = Arc::new(AtomicU16::new(1497));
            (
                RouterPort {
                    transport: Port {
                        incoming: Some(incoming),
                        sent,
                        mac: [index as u8 + 1],
                        limit: limit.clone(),
                    },
                    network_number: (index as u16 + 1) * 1000,
                },
                Peer {
                    frames,
                    sent: wire,
                    limit,
                },
            )
        })
        .unzip();
    let started = BACnetRouter::start(ports, options).await.unwrap();
    for peer in &mut peers {
        for kind in [0x13, 0x01] {
            let announcement = sending(peer).await;
            assert_eq!(&announcement.npdu[..3], &[1, 0x80, kind]);
            assert!(announcement.attributes.is_empty());
            announcement.complete.send(Ok(())).unwrap();
        }
    }
    (started.router, started.apdus, peers)
}

pub(super) async fn sending(peer: &mut Peer) -> Sending {
    timeout(Duration::from_secs(2), peer.sent.recv())
        .await
        .expect("transport send did not start")
        .expect("transport was dropped")
}

pub(super) fn attributes(tag: u8) -> Vec<DataAttribute> {
    vec![DataAttribute {
        option_type: 31,
        must_understand: tag & 1 != 0,
        data: vec![tag, 0xa5],
    }]
}

pub(super) async fn receive(peer: &Peer, npdu: Vec<u8>, tag: u8) {
    peer.frames
        .send(ReceivedNpdu::unverified(
            Bytes::from(npdu),
            MacAddr::from_slice(&[7]),
            false,
            attributes(tag),
            None,
        ))
        .await
        .unwrap();
}

/// Local delivery follows every preceding frame on this ingress port.
pub(super) async fn barrier(peer: &Peer, apdus: &mut mpsc::Receiver<ReceivedApdu>) {
    let marker = [0x10, 0xfd, 0x55];
    receive(peer, [vec![1, 0], marker.to_vec()].concat(), 0).await;
    timeout(Duration::from_secs(2), async {
        loop {
            let delivered = apdus.recv().await.unwrap();
            if delivered.apdu.as_ref() == marker {
                break;
            }
        }
    })
    .await
    .expect("ingress did not reach the barrier");
}
