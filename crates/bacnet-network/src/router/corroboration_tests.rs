use super::*;
use crate::router::forwarding::forward_unicast;
use crate::router_table::{ReachabilityStatus, RoutingClaimSnapshot};
use bacnet_encoding::npdu::decode_npdu;
use bytes::Bytes;
use std::sync::atomic::{AtomicUsize, Ordering};

const I_AM: NetworkMessageType = NetworkMessageType::I_AM_ROUTER_TO_NETWORK;
const ACK: NetworkMessageType = NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK;

struct DebugCounter(Arc<AtomicUsize>);

impl tracing::Subscriber for DebugCounter {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::DEBUG
            && metadata.target() == "bacnet_network::router::control_messages"
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if self.enabled(event.metadata()) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

struct Fixture {
    table: Arc<Mutex<RouterTable>>,
    txs: [mpsc::Sender<SendRequest>; 3],
    rxs: [mpsc::Receiver<SendRequest>; 3],
}

impl Fixture {
    fn new(table: RouterTable) -> Self {
        let (tx0, rx0) = mpsc::channel(16);
        let (tx1, rx1) = mpsc::channel(16);
        let (tx2, rx2) = mpsc::channel(16);
        Self {
            table: Arc::new(Mutex::new(table)),
            txs: [tx0, tx1, tx2],
            rxs: [rx0, rx1, rx2],
        }
    }

    fn learned() -> Self {
        let mut table = RouterTable::new();
        table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
        Self::new(table)
    }

    async fn deliver(&mut self, port: usize, mac: &[u8], kind: NetworkMessageType, payload: &[u8]) {
        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(kind.to_raw()),
            payload: Bytes::copy_from_slice(payload),
            ..Default::default()
        };
        handle_network_message(&self.table, &self.txs, port, 1000, mac, &npdu).await;
        for (index, rx) in self.rxs.iter_mut().enumerate() {
            if kind == I_AM && index != port {
                let SendRequest::Broadcast { npdu: data, .. } = rx.try_recv().unwrap() else {
                    panic!("expected unchanged I-Am rebroadcast");
                };
                let relayed = decode_npdu(data).unwrap();
                assert_eq!(relayed.payload, npdu.payload);
                assert_eq!(relayed.message_type, npdu.message_type);
            } else if kind == NetworkMessageType::INITIALIZE_ROUTING_TABLE && index == port {
                let SendRequest::Unicast {
                    npdu: data,
                    mac: dest,
                    ..
                } = rx.try_recv().unwrap()
                else {
                    panic!("expected Init ACK");
                };
                assert_eq!(dest.as_slice(), mac);
                let ack = decode_npdu(data).unwrap();
                assert_eq!(ack.message_type, Some(ACK.to_raw()));
                assert_eq!(ack.payload.as_ref(), &[0]);
            }
            assert!(rx.try_recv().is_err());
        }
    }

    async fn claim(&mut self, port: usize, mac: &[u8], kind: NetworkMessageType) {
        let payload: &[u8] = if kind == I_AM {
            &[0x0b, 0xb8]
        } else {
            &[1, 0x0b, 0xb8, 0, 0]
        };
        self.deliver(port, mac, kind, payload).await;
    }

    async fn assert_forwarded(&mut self, expected_port: usize, expected_mac: &[u8]) {
        let table = self.table.lock().await;
        assert_eq!(
            table.effective_reachability(3000),
            Some(ReachabilityStatus::Reachable)
        );
        let route = table.lookup(3000).unwrap().clone();
        drop(table);
        let npdu = Npdu {
            destination: Some(NpduAddress {
                network: 3000,
                mac_address: MacAddr::from_slice(&[9]),
            }),
            hop_count: 255,
            payload: Bytes::from_static(&[0x10, 0x20]),
            ..Default::default()
        };
        forward_unicast(&self.txs, &route, 1000, &[7], npdu.clone(), 2, &[]);
        for (index, rx) in self.rxs.iter_mut().enumerate() {
            if index == expected_port {
                let SendRequest::Unicast {
                    npdu: data, mac, ..
                } = rx.try_recv().unwrap()
                else {
                    panic!("expected learned-route forwarding");
                };
                assert_eq!(mac.as_slice(), expected_mac);
                let forwarded = decode_npdu(data).unwrap();
                assert_eq!(forwarded.destination, npdu.destination);
                assert_eq!(forwarded.payload, npdu.payload);
                assert_eq!(forwarded.hop_count, 254);
            }
            assert!(rx.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn both_claim_paths_keep_forwarding_old_route_until_repeat() {
    for first in [I_AM, ACK] {
        for second in [I_AM, ACK] {
            for repeat_mac in [&[2][..], &[3][..]] {
                let mut fixture = Fixture::learned();
                fixture.claim(1, &[2], first).await;
                fixture.assert_forwarded(0, &[1]).await;
                assert_eq!(
                    fixture.table.lock().await.claim_snapshot(),
                    RoutingClaimSnapshot {
                        pending_started: 1,
                        ..Default::default()
                    }
                );
                fixture.claim(1, repeat_mac, second).await;
                fixture.assert_forwarded(1, repeat_mac).await;
                assert_eq!(
                    fixture.table.lock().await.claim_snapshot(),
                    RoutingClaimSnapshot {
                        learned_ok: 1,
                        pending_started: 1,
                        corroborated_applied: 1,
                        ..Default::default()
                    }
                );
                // Same-port MAC updates still apply on the very next claim.
                fixture.claim(1, &[4], second).await;
                fixture.assert_forwarded(1, &[4]).await;
                assert_eq!(fixture.table.lock().await.claim_snapshot().learned_ok, 2);
            }
        }
    }
}

#[tokio::test]
async fn duplicate_entries_in_one_message_cannot_corroborate_a_replacement() {
    for kind in [I_AM, ACK] {
        let payload: &[u8] = if kind == I_AM {
            &[0x0b, 0xb8, 0x0b, 0xb8]
        } else {
            &[2, 0x0b, 0xb8, 0, 0, 0x0b, 0xb8, 0, 0]
        };
        let mut fixture = Fixture::learned();
        fixture.deliver(1, &[2], kind, payload).await;
        fixture.assert_forwarded(0, &[1]).await;
        assert_eq!(
            fixture.table.lock().await.claim_snapshot(),
            RoutingClaimSnapshot {
                pending_started: 1,
                ..Default::default()
            }
        );
        fixture.claim(1, &[2], kind).await;
        fixture.assert_forwarded(1, &[2]).await;
        assert_eq!(
            fixture
                .table
                .lock()
                .await
                .claim_snapshot()
                .corroborated_applied,
            1
        );

        // Duplicates of absent/current-port entries still take the immediate
        // learning/refresh path, including its original successful-entry count.
        let mut fixture = Fixture::new(RouterTable::new());
        fixture.deliver(1, &[2], kind, payload).await;
        fixture.assert_forwarded(1, &[2]).await;
        fixture.deliver(1, &[3], kind, payload).await;
        fixture.assert_forwarded(1, &[3]).await;
        assert_eq!(
            fixture.table.lock().await.claim_snapshot(),
            RoutingClaimSnapshot {
                learned_ok: 4,
                ..Default::default()
            }
        );
    }
}

#[tokio::test]
async fn alternating_ports_in_both_handlers_never_replace_forwarding_route() {
    for kind in [I_AM, ACK] {
        let mut fixture = Fixture::learned();
        for index in 0..20 {
            fixture.claim(1 + index % 2, &[2], kind).await;
            fixture.assert_forwarded(0, &[1]).await;
        }
        assert_eq!(
            fixture.table.lock().await.claim_snapshot(),
            RoutingClaimSnapshot {
                pending_started: 20,
                ..Default::default()
            }
        );
    }
}

#[tokio::test]
async fn absent_only_messages_neither_replace_nor_corroborate_pending() {
    for kind in [I_AM, ACK] {
        for other in [
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK,
        ] {
            let mut fixture = Fixture::learned();
            fixture.claim(1, &[2], kind).await;
            let payload: &[u8] = if other == NetworkMessageType::INITIALIZE_ROUTING_TABLE {
                &[2, 0x0b, 0xb8, 0, 0, 0x0f, 0xa0, 0, 0]
            } else {
                &[0x0b, 0xb8, 0]
            };
            // A claim on the current port also must not clear pending when the
            // message's policy is absent-only (it is not a refresh).
            for port in [1, 0] {
                fixture.deliver(port, &[3], other, payload).await;
                fixture.assert_forwarded(0, &[1]).await;
            }
            fixture.claim(1, &[2], kind).await;
            fixture.assert_forwarded(1, &[2]).await;
            assert_eq!(
                fixture.table.lock().await.claim_snapshot(),
                RoutingClaimSnapshot {
                    learned_ok: if other == NetworkMessageType::INITIALIZE_ROUTING_TABLE {
                        2
                    } else {
                        1
                    },
                    pending_started: 1,
                    corroborated_applied: 1,
                    ..Default::default()
                }
            );
        }
    }
}

#[tokio::test]
async fn all_absent_learning_paths_apply_on_first_message() {
    for kind in [
        I_AM,
        ACK,
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK,
    ] {
        let mut fixture = Fixture::new(RouterTable::new());
        let payload: &[u8] = if kind == I_AM {
            &[0x0b, 0xb8]
        } else if kind == NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK {
            &[0x0b, 0xb8, 0]
        } else {
            &[1, 0x0b, 0xb8, 0, 0]
        };
        fixture.deliver(1, &[2], kind, payload).await;
        fixture.assert_forwarded(1, &[2]).await;
        assert_eq!(
            fixture.table.lock().await.claim_snapshot(),
            RoutingClaimSnapshot {
                learned_ok: u64::from(kind != NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK),
                ..Default::default()
            }
        );
    }
}

#[tokio::test]
async fn disconnect_retains_route_state_and_pending_and_counts_only_complete_requests() {
    let mut fixture = Fixture::learned();
    fixture.table.lock().await.add_direct(4000, 2);
    fixture.claim(1, &[2], I_AM).await;
    let debug_events = Arc::new(AtomicUsize::new(0));
    // This test's current-thread runtime keeps the scoped subscriber local.
    let _subscriber = tracing::subscriber::set_default(DebugCounter(debug_events.clone()));
    let before = fixture.table.lock().await.lookup(3000).unwrap().clone();
    let kind = NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK;
    for net in [3000u16, 4000, 5000, 0, 0xffff, 3000] {
        fixture.deliver(1, &[2], kind, &net.to_be_bytes()).await;
        fixture.assert_forwarded(0, &[1]).await;
    }
    for payload in [&[][..], &[0x0b][..]] {
        fixture.deliver(1, &[2], kind, payload).await;
    }
    assert_eq!(debug_events.load(Ordering::Relaxed), 6);
    {
        let table = fixture.table.lock().await;
        let route = table.lookup(3000).unwrap();
        assert_eq!(route.last_seen, before.last_seen);
        assert_eq!(route.busy_until, before.busy_until);
        assert_eq!(route.flap_count, before.flap_count);
        assert_eq!(route.last_port_change, before.last_port_change);
        assert!(table.lookup(4000).unwrap().directly_connected);
        assert_eq!(table.len(), 2);
        assert_eq!(
            table.claim_snapshot(),
            RoutingClaimSnapshot {
                pending_started: 1,
                disconnect_removal_ignored: 6,
                ..Default::default()
            }
        );
    }
    fixture.claim(1, &[2], ACK).await; // Disconnect did not clear the pending slot.
    fixture.assert_forwarded(1, &[2]).await;
    assert_eq!(
        fixture
            .table
            .lock()
            .await
            .claim_snapshot()
            .corroborated_applied,
        1
    );
}
