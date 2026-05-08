//! NetworkLayer for local BACnet packet assembly and dispatch.
//!
//! The network layer wraps a transport and provides APDU-level send/receive
//! by handling NPDU encoding/decoding. This is a non-router implementation:
//! it does not forward messages between networks, but it can address remote
//! devices through local routers via NPDU destination fields (DNET/DADR).

use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::NetworkPriority;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

/// A received APDU with source addressing information.
pub struct ReceivedApdu {
    /// Raw APDU bytes.
    pub apdu: Bytes,
    /// Source MAC address in transport-native format.
    pub source_mac: MacAddr,
    /// Source network address if the APDU was routed (NPDU had source field).
    pub source_network: Option<NpduAddress>,
    /// Optional reply channel for MS/TP DataExpectingReply flows.
    /// The application layer can send NPDU-wrapped reply bytes through this channel.
    pub reply_tx: Option<oneshot::Sender<Bytes>>,
}

impl Clone for ReceivedApdu {
    fn clone(&self) -> Self {
        Self {
            apdu: self.apdu.clone(),
            source_mac: self.source_mac.clone(),
            source_network: self.source_network.clone(),
            reply_tx: None,
        }
    }
}

impl std::fmt::Debug for ReceivedApdu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceivedApdu")
            .field("apdu", &self.apdu)
            .field("source_mac", &self.source_mac)
            .field("source_network", &self.source_network)
            .field("reply_tx", &self.reply_tx.as_ref().map(|_| "Some(...)"))
            .finish()
    }
}

/// Non-router BACnet network layer.
///
/// Wraps a [`TransportPort`] and provides APDU-level send/receive by handling
/// NPDU framing. This layer does not act as a router (it does not forward
/// messages between networks), but it can send to remote devices through
/// local routers using NPDU destination addressing.
pub struct NetworkLayer<T: TransportPort> {
    transport: T,
    dispatch_task: Option<JoinHandle<()>>,
}

impl<T: TransportPort + 'static> NetworkLayer<T> {
    /// Create a new network layer wrapping the given transport.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            dispatch_task: None,
        }
    }

    /// Start the network layer. Returns a receiver for incoming APDUs.
    ///
    /// This starts the underlying transport and spawns a dispatch task that
    /// decodes incoming NPDUs and extracts APDUs.
    pub async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedApdu>, Error> {
        let mut npdu_rx = self.transport.start().await?;

        let (apdu_tx, apdu_rx) = mpsc::channel(256);

        let dispatch_task = tokio::spawn(async move {
            while let Some(received) = npdu_rx.recv().await {
                match decode_npdu(received.npdu.clone()) {
                    Ok(npdu) => {
                        if npdu.is_network_message {
                            debug!(
                                message_type = npdu.message_type,
                                "Ignoring network layer message (non-router mode)"
                            );
                            continue;
                        }

                        // Non-routing node: discard messages with a specific DNET.
                        if let Some(ref dest) = npdu.destination {
                            if dest.network != 0xFFFF {
                                debug!(
                                    dnet = dest.network,
                                    "Discarding routed message (non-router)"
                                );
                                continue;
                            }
                        }

                        let source_network = npdu.source.clone();

                        let apdu = ReceivedApdu {
                            apdu: npdu.payload,
                            source_mac: received.source_mac,
                            source_network,
                            reply_tx: received.reply_tx,
                        };

                        if apdu_tx.send(apdu).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to decode NPDU");
                    }
                }
            }
        });

        self.dispatch_task = Some(dispatch_task);

        Ok(apdu_rx)
    }

    /// Send an APDU to a specific local destination by MAC address.
    pub async fn send_apdu(
        &self,
        apdu: &[u8],
        destination_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply,
            priority,
            destination: None,
            source: None,
            payload: Bytes::copy_from_slice(apdu),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(2 + apdu.len());
        encode_npdu(&mut buf, &npdu)?;

        self.transport.send_unicast(&buf, destination_mac).await
    }

    /// Broadcast an APDU on the local network.
    pub async fn broadcast_apdu(
        &self,
        apdu: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply,
            priority,
            destination: None,
            source: None,
            payload: Bytes::copy_from_slice(apdu),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(2 + apdu.len());
        encode_npdu(&mut buf, &npdu)?;

        self.transport.send_broadcast(&buf).await
    }

    /// Broadcast an APDU globally (DNET=0xFFFF, hop_count=255).
    ///
    /// Unlike `broadcast_apdu()` which only reaches the local subnet, this
    /// sets DNET=0xFFFF so routers will forward to all reachable networks.
    pub async fn broadcast_global_apdu(
        &self,
        apdu: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply,
            priority,
            destination: Some(NpduAddress {
                network: 0xFFFF,
                mac_address: MacAddr::new(),
            }),
            source: None,
            hop_count: 255,
            payload: Bytes::copy_from_slice(apdu),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(8 + apdu.len());
        encode_npdu(&mut buf, &npdu)?;
        self.transport.send_broadcast(&buf).await
    }

    /// Broadcast an APDU to a specific remote network via routers.
    ///
    /// Like `broadcast_global_apdu()` but targets a single network number
    /// instead of all networks (DNET=0xFFFF).
    pub async fn broadcast_to_network(
        &self,
        apdu: &[u8],
        dest_network: u16,
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        if dest_network == 0xFFFF {
            return Err(Error::Encoding(
                "dest_network 0xFFFF is reserved for global broadcasts; use broadcast_global_apdu instead".into(),
            ));
        }
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply,
            priority,
            destination: Some(NpduAddress {
                network: dest_network,
                mac_address: MacAddr::new(),
            }),
            source: None,
            hop_count: 255,
            payload: Bytes::copy_from_slice(apdu),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(8 + apdu.len());
        encode_npdu(&mut buf, &npdu)?;
        self.transport.send_broadcast(&buf).await
    }

    /// Send an APDU to a remote device through a local router.
    ///
    /// The NPDU is sent via unicast to `router_mac` (the next-hop router on
    /// the local network), but the NPDU header addresses the final destination
    /// with `dest_network` / `dest_mac`.
    pub async fn send_apdu_routed(
        &self,
        apdu: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        router_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply,
            priority,
            destination: Some(NpduAddress {
                network: dest_network,
                mac_address: MacAddr::from_slice(dest_mac),
            }),
            source: None,
            hop_count: 255,
            payload: Bytes::copy_from_slice(apdu),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(8 + dest_mac.len() + apdu.len());
        encode_npdu(&mut buf, &npdu)?;

        self.transport.send_unicast(&buf, router_mac).await
    }

    /// Access the underlying transport.
    ///
    /// Useful for transport-specific operations like BBMD registration
    /// after the network layer has been started.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Get the transport's local MAC address.
    pub fn local_mac(&self) -> &[u8] {
        self.transport.local_mac()
    }

    /// Stop the network layer and underlying transport.
    pub async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.dispatch_task.take() {
            task.abort();
            let _ = task.await;
        }
        self.transport.stop().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_transport::bip::BipTransport;
    use std::net::Ipv4Addr;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn send_receive_apdu_unicast() {
        let transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

        let mut net_a = NetworkLayer::new(transport_a);
        let mut net_b = NetworkLayer::new(transport_b);

        let _rx_a = net_a.start().await.unwrap();
        let mut rx_b = net_b.start().await.unwrap();

        let test_apdu = vec![0x10, 0x08];

        net_a
            .send_apdu(
                &test_apdu,
                net_b.local_mac(),
                false,
                NetworkPriority::NORMAL,
            )
            .await
            .unwrap();

        let received = timeout(Duration::from_secs(2), rx_b.recv())
            .await
            .expect("Timed out waiting for APDU")
            .expect("Channel closed");

        assert_eq!(received.apdu, test_apdu);
        assert_eq!(received.source_mac.as_slice(), net_a.local_mac());
        assert!(received.source_network.is_none());

        net_a.stop().await.unwrap();
        net_b.stop().await.unwrap();
    }

    #[tokio::test]
    async fn end_to_end_who_is() {
        use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, UnconfirmedRequest};
        use bacnet_types::enums::UnconfirmedServiceChoice;

        let transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

        let mut net_a = NetworkLayer::new(transport_a);
        let mut net_b = NetworkLayer::new(transport_b);

        let _rx_a = net_a.start().await.unwrap();
        let mut rx_b = net_b.start().await.unwrap();

        let who_is_apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::WHO_IS,
            service_request: Bytes::new(),
        });
        let mut apdu_buf = BytesMut::new();
        encode_apdu(&mut apdu_buf, &who_is_apdu).expect("valid APDU encoding");

        net_a
            .send_apdu(&apdu_buf, net_b.local_mac(), false, NetworkPriority::NORMAL)
            .await
            .unwrap();

        let received = timeout(Duration::from_secs(2), rx_b.recv())
            .await
            .expect("Timed out waiting for APDU")
            .expect("Channel closed");

        let decoded_apdu = decode_apdu(received.apdu.clone()).unwrap();
        match decoded_apdu {
            Apdu::UnconfirmedRequest(req) => {
                assert_eq!(req.service_choice, UnconfirmedServiceChoice::WHO_IS);
                assert!(req.service_request.is_empty());
            }
            other => panic!("Expected UnconfirmedRequest, got {:?}", other),
        }

        net_a.stop().await.unwrap();
        net_b.stop().await.unwrap();
    }

    #[test]
    fn global_broadcast_npdu_has_dnet_ffff() {
        use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
        use bacnet_types::enums::NetworkPriority;

        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: NetworkPriority::NORMAL,
            destination: Some(NpduAddress {
                network: 0xFFFF,
                mac_address: MacAddr::new(),
            }),
            source: None,
            hop_count: 255,
            payload: Bytes::from_static(&[0xAA]),
            ..Npdu::default()
        };

        let mut buf = bytes::BytesMut::new();
        encode_npdu(&mut buf, &npdu).unwrap();
        let decoded = decode_npdu(Bytes::from(buf)).unwrap();
        let dest = decoded.destination.unwrap();
        assert_eq!(dest.network, 0xFFFF);
        assert!(dest.mac_address.is_empty());
        assert_eq!(decoded.hop_count, 255);
    }

    #[test]
    fn transport_accessor() {
        let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let net = NetworkLayer::new(transport);
        let mac = net.transport().local_mac();
        assert_eq!(mac.len(), 6);
    }

    #[test]
    fn routed_send_encodes_dnet_dadr() {
        use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
        use bacnet_types::enums::NetworkPriority;

        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: true,
            priority: NetworkPriority::NORMAL,
            destination: Some(NpduAddress {
                network: 100,
                mac_address: MacAddr::from_slice(&[1, 2, 3, 4, 5, 6]),
            }),
            source: None,
            hop_count: 255,
            payload: Bytes::from_static(&[0xAA, 0xBB]),
            ..Npdu::default()
        };

        let mut buf = bytes::BytesMut::new();
        encode_npdu(&mut buf, &npdu).unwrap();
        let decoded = decode_npdu(Bytes::from(buf)).unwrap();
        let dest = decoded.destination.unwrap();
        assert_eq!(dest.network, 100);
        assert_eq!(dest.mac_address.as_slice(), &[1, 2, 3, 4, 5, 6]);
        assert_eq!(decoded.hop_count, 255);
        assert!(decoded.expecting_reply);
    }

    #[test]
    fn broadcast_to_network_encodes_specific_dnet() {
        use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
        use bacnet_types::enums::NetworkPriority;

        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: NetworkPriority::NORMAL,
            destination: Some(NpduAddress {
                network: 42,
                mac_address: MacAddr::new(),
            }),
            source: None,
            hop_count: 255,
            payload: Bytes::from_static(&[0xCC]),
            ..Npdu::default()
        };

        let mut buf = bytes::BytesMut::new();
        encode_npdu(&mut buf, &npdu).unwrap();
        let decoded = decode_npdu(Bytes::from(buf)).unwrap();
        let dest = decoded.destination.unwrap();
        assert_eq!(dest.network, 42);
        assert!(dest.mac_address.is_empty());
        assert_eq!(decoded.hop_count, 255);
        assert!(!decoded.expecting_reply);
    }

    #[test]
    fn broadcast_to_network_rejects_dnet_ffff() {
        use bacnet_types::enums::NetworkPriority;

        let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let net = NetworkLayer::new(transport);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(async {
            net.broadcast_to_network(&[0xAA], 0xFFFF, false, NetworkPriority::NORMAL)
                .await
        });
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("0xFFFF"),
            "Error should mention 0xFFFF: {err_msg}"
        );
    }
}
