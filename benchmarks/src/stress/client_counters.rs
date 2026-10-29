//! Named observations from existing admission and B/IP counters.
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    pub totals: BTreeMap<String, u64>,
    pub active: BTreeMap<String, u64>,
}

/// Endpoint samples are not peaks; totals are events, not successful requests.
#[derive(Debug, Serialize)]
pub struct CounterDelta {
    pub totals: BTreeMap<String, u64>,
    pub active_before: BTreeMap<String, u64>,
    pub active_after: BTreeMap<String, u64>,
    pub invalid: Vec<String>,
}

impl CounterDelta {
    pub fn between(before: Snapshot, after: Snapshot) -> Self {
        let mut totals = BTreeMap::new();
        let mut invalid = Vec::new();
        for (name, start) in &before.totals {
            match after
                .totals
                .get(name)
                .and_then(|end| end.checked_sub(*start))
            {
                Some(delta) => {
                    totals.insert(name.clone(), delta);
                }
                None => invalid.push(name.clone()),
            }
        }
        for name in after.totals.keys() {
            if !before.totals.contains_key(name) {
                invalid.push(name.clone());
            }
        }
        Self {
            totals,
            active_before: before.active,
            active_after: after.active,
            invalid,
        }
    }
}

pub fn bip(transport: &BipTransport) -> Option<Snapshot> {
    let r = transport.receive_counters();
    let s = transport.send_counters();
    let mut snapshot = Snapshot::default();
    macro_rules! receive { ($($field:ident),* $(,)?) => { $(
        snapshot.totals.insert(concat!("receive.", stringify!($field)).into(), r.$field);
    )* }; }
    receive!(
        receive_errors,
        listener_mismatch,
        too_short,
        not_bip,
        length_mismatch,
        destination_mismatch,
        rate_limited,
        unknown_function,
        malformed_payload,
        own_echo,
        unanswerable_request,
        forwarded_not_from_peer,
        forwarded_untrusted,
        npdu_channel_full,
        npdu_channel_closed
    );
    // This is a size refusal, not a general socket-send error counter.
    snapshot
        .totals
        .insert("send.npdu_too_long".into(), s.npdu_too_long);
    Some(snapshot)
}

pub fn server(server: &BACnetServer<BipTransport>) -> Option<Snapshot> {
    let mut snapshot = server.transport().and_then(bip)?;
    let a = server.request_admission_counters();
    macro_rules! totals { ($($field:ident),* $(,)?) => { $(
        snapshot.totals.insert(concat!("admission.", stringify!($field)).into(), a.$field);
    )* }; }
    totals!(
        recovery_admitted_total,
        recovery_overloaded_total,
        confirmed_admitted_total,
        confirmed_overloaded_total,
        confirmed_global_overloaded_total,
        confirmed_peer_overloaded_total,
        confirmed_shutdown_rejected_total,
        unconfirmed_admitted_total,
        unconfirmed_overloaded_total,
        unconfirmed_global_overloaded_total,
        unconfirmed_peer_overloaded_total,
        unconfirmed_shutdown_rejected_total,
        abort_admitted_total,
        confirmed_fallback_dropped_total,
        abort_shutdown_rejected_total
    );
    macro_rules! active { ($($field:ident),* $(,)?) => { $(
        snapshot.active.insert(concat!("admission.", stringify!($field)).into(), a.$field as u64);
    )* }; }
    active!(
        recovery_active,
        confirmed_active,
        unconfirmed_active,
        abort_active
    );
    Some(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn totals_are_deltas_but_active_counts_are_endpoint_samples() {
        let before = Snapshot {
            totals: BTreeMap::from([("admitted".into(), 7)]),
            active: BTreeMap::from([("active".into(), 3)]),
        };
        let after = Snapshot {
            totals: BTreeMap::from([("admitted".into(), 12)]),
            active: BTreeMap::from([("active".into(), 1)]),
        };
        let delta = CounterDelta::between(before, after);
        assert_eq!(delta.totals["admitted"], 5);
        assert_eq!(delta.active_before["active"], 3);
        assert_eq!(delta.active_after["active"], 1);
        assert!(delta.invalid.is_empty());
    }
    #[test]
    fn counter_decrease_or_missing_field_is_invalid_not_a_zero() {
        let before = Snapshot {
            totals: BTreeMap::from([("decreased".into(), 7), ("missing".into(), 1)]),
            ..Snapshot::default()
        };
        let after = Snapshot {
            totals: BTreeMap::from([("decreased".into(), 2)]),
            ..Snapshot::default()
        };
        let delta = CounterDelta::between(before, after);
        assert_eq!(delta.invalid, ["decreased", "missing"]);
        assert!(delta.totals.is_empty());
    }
}
