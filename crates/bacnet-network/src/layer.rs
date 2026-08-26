//! NetworkLayer for local BACnet packet assembly and dispatch.
//!
//! The network layer wraps a transport and provides APDU-level send/receive
//! by handling NPDU encoding/decoding. This is a non-router implementation:
//! it does not forward messages between networks, but it can address remote
//! devices through local routers via NPDU destination fields (DNET/DADR).

use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_transport::port::{DataAttribute, TransportPort};
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
    /// Whether the NPDU arrived through a data-link multicast or broadcast.
    ///
    /// This preserves raw data-link provenance independently of [`Self::is_group`].
    pub link_layer_group: bool,
    /// Whether the APDU's effective BACnet destination was multicast or broadcast.
    ///
    /// A specific DNET/DADR remains a unicast even when a router used a
    /// group data-link destination to reach that remote device.
    pub is_group: bool,
    /// Data-link attributes associated with the NPDU, if the transport supplied any.
    pub data_attributes: Vec<DataAttribute>,
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
            link_layer_group: self.link_layer_group,
            is_group: self.is_group,
            data_attributes: self.data_attributes.clone(),
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
            .field("link_layer_group", &self.link_layer_group)
            .field("is_group", &self.is_group)
            .field("data_attributes", &self.data_attributes)
            .field("reply_tx", &self.reply_tx.as_ref().map(|_| "Some(...)"))
            .finish()
    }
}

pub(crate) fn is_group_delivery(link_layer_group: bool, destination: Option<&NpduAddress>) -> bool {
    match destination {
        None => link_layer_group,
        Some(destination) => destination.network == 0xFFFF || destination.mac_address.is_empty(),
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
                        let is_group =
                            is_group_delivery(received.link_layer_group, npdu.destination.as_ref());

                        let apdu = ReceivedApdu {
                            apdu: npdu.payload,
                            source_mac: received.source_mac,
                            source_network,
                            link_layer_group: received.link_layer_group,
                            is_group,
                            data_attributes: received.data_attributes,
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
        self.send_apdu_with_data_attributes(apdu, destination_mac, expecting_reply, priority, &[])
            .await
    }

    /// Send an APDU with data attributes to a specific local destination.
    pub async fn send_apdu_with_data_attributes(
        &self,
        apdu: &[u8],
        destination_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
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

        self.transport
            .send_unicast_with_data_attributes(&buf, destination_mac, data_attributes)
            .await
    }

    /// Broadcast an APDU on the local network.
    pub async fn broadcast_apdu(
        &self,
        apdu: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        self.broadcast_apdu_with_data_attributes(apdu, expecting_reply, priority, &[])
            .await
    }

    /// Broadcast an APDU with data attributes on the local network.
    pub async fn broadcast_apdu_with_data_attributes(
        &self,
        apdu: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
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

        self.transport
            .send_broadcast_with_data_attributes(&buf, data_attributes)
            .await
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
        self.broadcast_global_apdu_with_data_attributes(apdu, expecting_reply, priority, &[])
            .await
    }

    /// Broadcast an APDU globally with data attributes (DNET=0xFFFF, hop_count=255).
    pub async fn broadcast_global_apdu_with_data_attributes(
        &self,
        apdu: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
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
        self.transport
            .send_broadcast_with_data_attributes(&buf, data_attributes)
            .await
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
        self.broadcast_to_network_with_data_attributes(
            apdu,
            dest_network,
            expecting_reply,
            priority,
            &[],
        )
        .await
    }

    /// Broadcast an APDU with data attributes to a specific remote network via routers.
    pub async fn broadcast_to_network_with_data_attributes(
        &self,
        apdu: &[u8],
        dest_network: u16,
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
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
        self.transport
            .send_broadcast_with_data_attributes(&buf, data_attributes)
            .await
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
        self.send_apdu_routed_with_data_attributes(
            apdu,
            dest_network,
            dest_mac,
            router_mac,
            expecting_reply,
            priority,
            &[],
        )
        .await
    }

    /// Send an APDU with data attributes to a remote device through a local router.
    pub async fn send_apdu_routed_with_data_attributes(
        &self,
        apdu: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        router_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        let buf =
            Self::encode_routed_npdu_buf(apdu, dest_network, dest_mac, expecting_reply, priority)?;
        self.transport
            .send_unicast_with_data_attributes(&buf, router_mac, data_attributes)
            .await
    }

    /// Send a routed APDU with a broadcast link DA, for when the next-hop
    /// router's MAC is unknown.
    ///
    /// Clause 6.5.3: the data link DA "shall be the MAC address of the BACnet
    /// router corresponding to the DNET parameter or the appropriate
    /// broadcast DA if the address of the router is initially unknown". The
    /// NPDU still addresses one device via DNET/DADR, which is why Clause
    /// 6.3's broadcast restriction does not bite: "a MAC layer multicast or
    /// broadcast address may be used for other PDU types when the network
    /// layer address restricts the destination to a single device".
    pub async fn send_apdu_routed_via_local_broadcast(
        &self,
        apdu: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        self.send_apdu_routed_via_local_broadcast_with_data_attributes(
            apdu,
            dest_network,
            dest_mac,
            expecting_reply,
            priority,
            &[],
        )
        .await
    }

    /// Send a routed APDU with data attributes and a broadcast link DA.
    pub async fn send_apdu_routed_via_local_broadcast_with_data_attributes(
        &self,
        apdu: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        let buf =
            Self::encode_routed_npdu_buf(apdu, dest_network, dest_mac, expecting_reply, priority)?;
        self.transport
            .send_broadcast_with_data_attributes(&buf, data_attributes)
            .await
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

    /// Encode an APDU into an NPDU whose destination is `dest_network` /
    /// `dest_mac`, ready for whichever link send the caller chooses.
    fn encode_routed_npdu_buf(
        apdu: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<BytesMut, Error> {
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
        Ok(buf)
    }

    /// Stop the network layer and underlying transport.
    pub async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.abort_dispatch_task() {
            let _ = task.await;
        }
        self.transport.stop().await
    }
}

impl<T: TransportPort> NetworkLayer<T> {
    fn abort_dispatch_task(&mut self) -> Option<JoinHandle<()>> {
        let task = self.dispatch_task.take()?;
        task.abort();
        Some(task)
    }
}

impl<T: TransportPort> Drop for NetworkLayer<T> {
    fn drop(&mut self) {
        let _ = self.abort_dispatch_task();
        self.transport.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_transport::bip::BipTransport;
    use bacnet_transport::sc::{LoopbackWebSocket, ScTransport, WebSocketPort};
    use bacnet_transport::sc_frame::{
        decode_sc_message, encode_sc_message, ScFunction, ScMessage, ScOption, Vmac,
    };
    use std::net::Ipv4Addr;
    use tokio::time::{timeout, Duration};

    async fn sc_hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
        let data = ws_hub.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        assert_eq!(req.function, ScFunction::ConnectRequest);

        let mut accept_payload = Vec::with_capacity(26);
        accept_payload.extend_from_slice(&hub_vmac);
        accept_payload.extend_from_slice(&[0u8; 16]);
        accept_payload.extend_from_slice(&1476u16.to_be_bytes());
        accept_payload.extend_from_slice(&1476u16.to_be_bytes());

        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: req.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(accept_payload),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &accept);
        ws_hub.send(&buf).await.unwrap();
    }

    async fn assert_sc_socket_closed_after_drop(ws_hub: &LoopbackWebSocket, context: &str) {
        timeout(Duration::from_secs(1), async {
            loop {
                match ws_hub.recv().await {
                    Ok(data) => {
                        let msg = decode_sc_message(&data).unwrap();
                        assert_ne!(
                            msg.function,
                            ScFunction::HeartbeatAck,
                            "{context} must not leave SC answering heartbeats"
                        );
                    }
                    Err(_) => break,
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{context} did not close the SC WebSocket"));

        let heartbeat = ScMessage {
            function: ScFunction::HeartbeatRequest,
            message_id: 0x66,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &heartbeat);
        assert!(
            ws_hub.send(&buf).await.is_err(),
            "{context} must reject post-drop Heartbeat-Request on the closed socket"
        );
    }

    #[tokio::test]
    async fn sc_data_options_reach_received_apdu_data_attributes() {
        let (ws_client, ws_hub) = LoopbackWebSocket::pair();
        let hub_vmac = [0x10; 6];
        let mut net = NetworkLayer::new(ScTransport::new(ws_client, [0x01; 6]));

        let hub_accept_task = tokio::spawn(async move {
            sc_hub_accept(&ws_hub, hub_vmac).await;
            ws_hub
        });

        let mut rx = net.start().await.unwrap();
        let ws_hub = hub_accept_task.await.unwrap();

        let apdu = Bytes::from_static(&[0x10, 0x08]);
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: NetworkPriority::NORMAL,
            destination: None,
            source: None,
            payload: apdu.clone(),
            ..Npdu::default()
        };
        let mut npdu_buf = BytesMut::new();
        encode_npdu(&mut npdu_buf, &npdu).unwrap();

        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 0x2345,
            originating_vmac: Some(hub_vmac),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: vec![
                ScOption {
                    option_type: 1,
                    must_understand: true,
                    data: Vec::new(),
                },
                ScOption {
                    option_type: 31,
                    must_understand: false,
                    data: vec![0x12, 0x34, 0x56],
                },
            ],
            payload: npdu_buf.freeze(),
        };
        let mut sc_buf = BytesMut::new();
        encode_sc_message(&mut sc_buf, &msg);
        ws_hub.send(&sc_buf).await.unwrap();

        let received = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for APDU")
            .expect("APDU channel closed");

        assert_eq!(received.apdu, apdu);
        assert_eq!(received.source_mac.as_slice(), hub_vmac);
        assert!(received.source_network.is_none());
        assert_eq!(received.data_attributes.len(), 2);
        assert_eq!(received.data_attributes[0].option_type, 1);
        assert!(received.data_attributes[0].must_understand);
        assert!(received.data_attributes[0].data.is_empty());
        assert_eq!(received.data_attributes[1].option_type, 31);
        assert!(!received.data_attributes[1].must_understand);
        assert_eq!(received.data_attributes[1].data, vec![0x12, 0x34, 0x56]);

        net.stop().await.unwrap();
    }

    #[tokio::test]
    async fn network_layer_drop_releases_sc_transport_socket() {
        let (ws_client, ws_hub) = LoopbackWebSocket::pair();
        let hub_vmac = [0x10; 6];
        let mut net = NetworkLayer::new(ScTransport::new(ws_client, [0x01; 6]));

        let hub_accept_task = tokio::spawn(async move {
            sc_hub_accept(&ws_hub, hub_vmac).await;
            ws_hub
        });

        let _rx = net.start().await.unwrap();
        let ws_hub = hub_accept_task.await.unwrap();

        drop(net);

        assert_sc_socket_closed_after_drop(&ws_hub, "dropped NetworkLayer").await;
    }

    #[tokio::test]
    async fn send_apdu_data_attributes_reach_sc_data_options() {
        let (ws_client, ws_hub) = LoopbackWebSocket::pair();
        let hub_vmac = [0x10; 6];
        let dest_vmac: Vmac = [0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
        let mut net = NetworkLayer::new(ScTransport::new(ws_client, [0x01; 6]));
        let data_attributes = vec![
            DataAttribute {
                option_type: 1,
                must_understand: true,
                data: Vec::new(),
            },
            DataAttribute {
                option_type: 31,
                must_understand: false,
                data: vec![0x12, 0x34, 0x56],
            },
        ];

        let hub_accept_task = tokio::spawn(async move {
            sc_hub_accept(&ws_hub, hub_vmac).await;
            ws_hub
        });

        let _rx = net.start().await.unwrap();
        let ws_hub = hub_accept_task.await.unwrap();

        let apdu = Bytes::from_static(&[0x10, 0x08]);
        net.send_apdu_with_data_attributes(
            &apdu,
            &dest_vmac,
            false,
            NetworkPriority::NORMAL,
            &data_attributes,
        )
        .await
        .unwrap();

        let data = ws_hub.recv().await.unwrap();
        let msg = decode_sc_message(&data).unwrap();
        assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
        assert_eq!(msg.destination_vmac, Some(dest_vmac));
        assert_eq!(msg.data_options.len(), 2);
        assert_eq!(msg.data_options[0].option_type, 1);
        assert!(msg.data_options[0].must_understand);
        assert_eq!(msg.data_options[1].option_type, 31);
        assert!(!msg.data_options[1].must_understand);
        assert_eq!(msg.data_options[1].data, vec![0x12, 0x34, 0x56]);

        let npdu = decode_npdu(msg.payload).unwrap();
        assert_eq!(npdu.payload, apdu);
        assert!(!npdu.expecting_reply);
        assert_eq!(npdu.priority, NetworkPriority::NORMAL);

        net.stop().await.unwrap();
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
}

#[cfg(test)]
#[path = "layer_delivery_tests.rs"]
mod delivery_tests;
