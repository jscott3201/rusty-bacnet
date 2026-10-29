//! A transport's owned group rules, installed with its router port.

use bacnet_router_core::router::{Group, GroupClassifier};
use bacnet_transport::port::GroupDestinations;

/// All group destinations and the subset the local broadcast reaches.
/// Clones retain the transport's predicates, including shared mutable state.
#[derive(Debug, Clone, Default)]
pub(super) struct PortGroups {
    pub(super) groups: GroupDestinations,
    pub(super) local_broadcasts: GroupDestinations,
}

impl GroupClassifier for PortGroups {
    const NONE: Self = Self {
        groups: GroupDestinations::unknown(),
        local_broadcasts: GroupDestinations::unknown(),
    };

    fn classify(&self, mac: &[u8]) -> Group {
        if !self.groups.contains(mac) {
            Group::No
        } else if self.local_broadcasts.contains(mac) {
            Group::Broadcast
        } else {
            Group::Refuse
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};

    use bacnet_npdu::Mac;
    use bacnet_npdu::NpduLimits;
    use bacnet_router_core::PortId;
    use bacnet_router_core::router::{Action, Dest, Discard, Frame, Permissive, Received};

    use super::*;
    use crate::router::engine::{Engine, tokio_clock};

    #[tokio::test]
    async fn engine_keeps_custom_transport_predicates_live_after_installation() {
        let state = Arc::new(AtomicU8::new(0));
        let groups = Arc::clone(&state);
        let local = Arc::clone(&state);
        let rules = PortGroups {
            groups: GroupDestinations::new(move |mac| {
                mac == [42] && groups.load(Ordering::SeqCst) != 0
            }),
            local_broadcasts: GroupDestinations::new(move |mac| {
                mac == [42] && local.load(Ordering::SeqCst) == 2
            }),
        };
        let engine = Engine::new(
            &[
                Engine::port(1, Mac::from_array([1]), NpduLimits::default(), rules),
                Engine::port(
                    2,
                    Mac::from_array([2]),
                    NpduLimits::default(),
                    PortGroups::default(),
                ),
            ],
            tokio_clock(),
            super::super::engine::OPTIONS,
        )
        .unwrap();
        let npdu = [1, 0x20, 0, 1, 1, 42, 255, 0x10, 8];
        let mut drops = 0;
        for value in [0, 1, 2, 1, 0] {
            state.store(value, Ordering::SeqCst);
            let mut router = engine.lock();
            let outcome = router.receive(
                Frame {
                    port: PortId(1),
                    source: &[7],
                    link_group: false,
                    npdu: &npdu,
                },
                engine.now(),
                &mut Permissive,
            );
            if value == 1 {
                drops += 1;
                assert_eq!(outcome, Received::Discarded(Discard::GroupDadr));
            } else {
                assert_eq!(outcome, Received::Handled);
                let Action::Send(send) = router.poll_action() else {
                    panic!("missing relay")
                };
                assert_eq!(send.port, PortId(0));
                assert_eq!(
                    send.to,
                    if value == 2 {
                        Dest::Broadcast
                    } else {
                        Dest::Unicast(&[42])
                    }
                );
            }
            assert!(matches!(router.poll_action(), Action::Idle));
            drop(router);
            // The totals since start, which dropping the guard published.
            let totals = engine.counters();
            assert_eq!(totals.group_dadr_drops, drops);
            assert_eq!(totals.refused, 0);
        }
    }
}
