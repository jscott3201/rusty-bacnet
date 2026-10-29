//! Hardened convergence end to end (RB-06): claims from I-Am-Router and
//! Init-ACK hold a cross-port move until a second claim, while the old
//! route keeps forwarding. Three ports on 1000, 2000 and 4000, on the
//! router's engine through [`super::envelope_harness`].

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu};
use bacnet_npdu::NetworkMessageType;
use bacnet_types::MacAddr;
use bytes::Bytes;

use super::SendRequest;
use super::envelope_harness::*;
use crate::router_table::{ClaimCounters, Convergence, PortId, Reachability};

const I_AM: NetworkMessageType = NetworkMessageType::I_AM_ROUTER_TO_NETWORK;
const ACK: NetworkMessageType = NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK;

struct DebugCounter(Arc<AtomicUsize>);

impl DebugCounter {
    fn new(events: Arc<AtomicUsize>) -> Self {
        // While at most one dispatcher is registered, tracing-core 0.1.36 takes
        // a cold callsite's interest from the registering thread's default. A
        // parallel test reaching the disconnect callsite first would cache
        // "never" and hide our events (#866). As in bacnet-server's
        // dcc_trace_tests, keep a second, disabled dispatcher registered (not
        // installed as a default) so registration consults every live
        // dispatcher. It retains no events.
        static REGISTRATION_PEER: std::sync::OnceLock<tracing::Dispatch> =
            std::sync::OnceLock::new();
        REGISTRATION_PEER
            .get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
        Self(events)
    }
}

impl tracing::Subscriber for DebugCounter {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        // The engine reports each ignored removal, and the router's host
        // logs it there (#1679).
        *metadata.level() == tracing::Level::DEBUG
            && metadata.target() == "bacnet_network::router::hosted_host"
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
    h: Harness,
}

impl Fixture {
    /// Ports 0, 1 and 2 on 1000, 2000 and 4000.
    fn new() -> Self {
        Self {
            h: Harness::with_networks(&[1000, 2000, 4000]),
        }
    }

    /// Hardened convergence, with 3000 learned through [1] on port 0.
    fn learned() -> Self {
        let fixture = Self::new();
        fixture.h.table().set_convergence(Convergence::Hardened);
        fixture.h.learned(3000, 0, &[1]);
        fixture
    }

    async fn deliver(&mut self, port: usize, mac: &[u8], kind: NetworkMessageType, payload: &[u8]) {
        let npdu = control_npdu(kind, payload);
        self.h.handle(self.h.ctx(port, mac, npdu.clone())).await;
        for index in 0..3 {
            let rx = &mut self.h.rxs[index];
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
                assert_eq!(ack.message_type, Some(ACK));
                assert!(ack.payload.is_empty());
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

    fn claims(&self) -> ClaimCounters {
        self.h.table().claim_counters()
    }

    /// An APDU for 3000 from [7] on port 2 goes out `expected_port` to
    /// `expected_mac`, one hop spent, and nowhere else.
    async fn assert_forwarded(&mut self, expected_port: usize, expected_mac: &[u8]) {
        assert_eq!(
            self.h.table().effective_reachability(3000, self.h.now()),
            Some(Reachability::Reachable)
        );
        let npdu = Npdu {
            destination: Some(NpduAddress {
                network: 3000,
                mac_address: MacAddr::from_slice(&[9]),
            }),
            hop_count: 255,
            payload: Bytes::from_static(&[0x10, 0x20]),
            ..Default::default()
        };
        self.h.dispatch(self.h.ctx(2, &[7], npdu.clone())).await;
        for index in 0..3 {
            let rx = &mut self.h.rxs[index];
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
                assert_eq!(fixture.claims(), claim_counts(|c| c.pending_started = 1));
                fixture.claim(1, repeat_mac, second).await;
                fixture.assert_forwarded(1, repeat_mac).await;
                assert_eq!(
                    fixture.claims(),
                    claim_counts(|c| {
                        c.learned_ok = 1;
                        c.pending_started = 1;
                        c.corroborated_applied = 1;
                    })
                );
                // Same-port MAC updates still apply on the very next claim.
                fixture.claim(1, &[4], second).await;
                fixture.assert_forwarded(1, &[4]).await;
                assert_eq!(fixture.claims().learned_ok, 2);
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
        assert_eq!(fixture.claims(), claim_counts(|c| c.pending_started = 1));
        fixture.claim(1, &[2], kind).await;
        fixture.assert_forwarded(1, &[2]).await;
        assert_eq!(fixture.claims().corroborated_applied, 1);

        // Duplicates of absent/current-port entries still take the immediate
        // learning/refresh path, including its original successful-entry count.
        let mut fixture = Fixture::new();
        fixture.deliver(1, &[2], kind, payload).await;
        fixture.assert_forwarded(1, &[2]).await;
        fixture.deliver(1, &[3], kind, payload).await;
        fixture.assert_forwarded(1, &[3]).await;
        assert_eq!(fixture.claims(), claim_counts(|c| c.learned_ok = 4));
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
        assert_eq!(fixture.claims(), claim_counts(|c| c.pending_started = 20));
    }
}

#[tokio::test]
async fn unrelated_management_and_absent_only_messages_preserve_pending() {
    // RB-05: Initialize-Routing-Table is management, not absent-only — it can
    // replace or purge learned entries. Writes to unrelated networks (and
    // absent-only I-Could-Be-Router) still leave another network's pending
    // challenger alone, and the pending claim still corroborates after.
    for kind in [I_AM, ACK] {
        for other in [
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK,
        ] {
            let mut fixture = Fixture::learned();
            fixture.claim(1, &[2], kind).await;
            let payload: &[u8] = if other == NetworkMessageType::INITIALIZE_ROUTING_TABLE {
                // Adds unrelated 5001 via Port ID 2 (local port 1) and purges
                // absent 5000: touches neither 3000 nor its challenger.
                &[2, 0x13, 0x89, 2, 0, 0x13, 0x88, 0, 0]
            } else {
                &[0x0b, 0xb8, 0]
            };
            for port in [1, 0] {
                fixture.deliver(port, &[3], other, payload).await;
                fixture.assert_forwarded(0, &[1]).await;
            }
            if other == NetworkMessageType::INITIALIZE_ROUTING_TABLE {
                assert!(fixture.h.table().lookup(5001).is_some());
            } else {
                assert!(fixture.h.table().lookup(5001).is_none());
            }
            fixture.claim(1, &[2], kind).await;
            fixture.assert_forwarded(1, &[2]).await;
            assert_eq!(
                fixture.claims(),
                claim_counts(|c| {
                    c.learned_ok = 1;
                    c.pending_started = 1;
                    c.corroborated_applied = 1;
                })
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
        let mut fixture = Fixture::new();
        let payload: &[u8] = if kind == I_AM {
            &[0x0b, 0xb8]
        } else if kind == NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK {
            &[0x0b, 0xb8, 0]
        } else if kind == NetworkMessageType::INITIALIZE_ROUTING_TABLE {
            // Management update via Port ID 2 (local port 1): a purge-style
            // Port ID 0 would remove rather than install.
            &[1, 0x0b, 0xb8, 2, 0]
        } else {
            &[1, 0x0b, 0xb8, 0, 0]
        };
        fixture.deliver(1, &[2], kind, payload).await;
        fixture.assert_forwarded(1, &[2]).await;
        assert_eq!(
            fixture.claims(),
            claim_counts(|c| c.learned_ok = u64::from(kind == I_AM || kind == ACK))
        );
    }
}

#[tokio::test]
async fn disconnect_retains_route_state_and_pending_and_counts_only_complete_requests() {
    let mut fixture = Fixture::learned();
    fixture.claim(1, &[2], I_AM).await;
    let debug_events = Arc::new(AtomicUsize::new(0));
    // This test's current-thread runtime keeps the scoped subscriber local.
    let _subscriber = tracing::subscriber::set_default(DebugCounter::new(debug_events.clone()));
    let before = fixture.h.table().lookup(3000).unwrap();
    let kind = NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK;
    for net in [3000u16, 4000, 5000, 0, 0xffff, 3000] {
        fixture.deliver(1, &[2], kind, &net.to_be_bytes()).await;
        fixture.assert_forwarded(0, &[1]).await;
    }
    for payload in [&[][..], &[0x0b][..]] {
        fixture.deliver(1, &[2], kind, payload).await;
    }
    {
        let table = fixture.h.table();
        let route = table.lookup(3000).unwrap();
        assert_eq!(route.last_seen(), before.last_seen());
        assert_eq!(route.busy_until(), before.busy_until());
        assert_eq!(route.flap_count(), before.flap_count());
        assert_eq!(route.last_port_change(), before.last_port_change());
        assert!(table.lookup(4000).unwrap().is_direct());
        assert_eq!(table.lookup(4000).unwrap().port(), PortId(2));
        // The three direct routes and 3000.
        assert_eq!(table.len(), 4);
        assert_eq!(
            table.claim_counters(),
            claim_counts(|c| {
                c.pending_started = 1;
                c.disconnect_removal_ignored = 6;
            })
        );
    }
    // One DEBUG event per ignored removal; malformed requests log nothing.
    assert_eq!(debug_events.load(Ordering::Relaxed), 6);
    fixture.claim(1, &[2], ACK).await; // Disconnect did not clear the pending slot.
    fixture.assert_forwarded(1, &[2]).await;
    assert_eq!(fixture.claims().corroborated_applied, 1);
}

/// A parallel test reaching the disconnect DEBUG callsite first must not hide
/// this thread's events, and its own events are not counted here (#866).
#[tokio::test]
async fn debug_counter_counts_this_thread_whoever_registers_the_callsite() {
    let kind = NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK;
    let payload = 3000u16.to_be_bytes();
    let debug_events = Arc::new(AtomicUsize::new(0));
    let _subscriber = tracing::subscriber::set_default(DebugCounter::new(debug_events.clone()));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async { Fixture::learned().deliver(1, &[2], kind, &payload).await });
    })
    .join()
    .unwrap();
    Fixture::learned().deliver(1, &[2], kind, &payload).await;
    assert_eq!(debug_events.load(Ordering::Relaxed), 1);
}
