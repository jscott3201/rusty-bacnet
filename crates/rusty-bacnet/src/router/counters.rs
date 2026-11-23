//! `BACnetRouter.counters()` (#2725): the engine's counts since the router
//! started and its wire-control decisions, each under its Rust name.

use bacnet_network::router::{
    BACnetRouter, ControlDecisionCounters, ControlServiceCounters, Counters,
};
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// What one `counters()` call copies from the running router: owned, so
/// the dict is built after the lifecycle's lock is gone.
pub(super) struct Snapshot {
    engine: Counters,
    control: ControlDecisionCounters,
}

impl Snapshot {
    /// The counts of `router` now. Neither read takes the engine's lock.
    pub(super) fn of(router: &BACnetRouter) -> Self {
        Self {
            engine: router.counters(),
            control: router.control_snapshot(),
        }
    }

    /// The dict `counters()` returns: each engine count, and under
    /// `"control"` each decision class with its three totals.
    pub(super) fn into_dict(self, py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
        let dict = PyDict::new(py);
        for (name, count) in engine_entries(&self.engine) {
            dict.set_item(name, count)?;
        }
        let control = PyDict::new(py);
        for (class, totals) in control_entries(self.control) {
            let row = PyDict::new(py);
            for (name, count) in service_entries(totals) {
                row.set_item(name, count)?;
            }
            control.set_item(class, row)?;
        }
        dict.set_item("control", control)?;
        Ok(dict)
    }
}

/// Every `Counters` field under its Rust name. `Counters` is
/// `#[non_exhaustive]`, so this can't destructure it to be sure it names
/// each: the tests compare these names with the fields its `Debug` lists,
/// so a count added there is noticed here.
fn engine_entries(c: &Counters) -> [(&'static str, u64); 24] {
    [
        ("address_length_drops", c.address_length_drops),
        ("global_broadcast_dadr_drops", c.global_broadcast_dadr_drops),
        ("broadcast_pdu_type_drops", c.broadcast_pdu_type_drops),
        ("group_dadr_drops", c.group_dadr_drops),
        ("malformed", c.malformed),
        ("bad_messages", c.bad_messages),
        ("unknown_port", c.unknown_port),
        ("hop_count_drops", c.hop_count_drops),
        ("denied", c.denied),
        ("unroutable_rejects", c.unroutable_rejects),
        ("rejects", c.rejects),
        ("solicitations", c.solicitations),
        ("write_failures", c.write_failures),
        ("egress_too_long", c.egress_too_long),
        ("refused", c.refused),
        ("undrained", c.undrained),
        ("busy_announced", c.busy_announced),
        ("available_announced", c.available_announced),
        ("duplicate_drops", c.duplicate_drops),
        ("own_source_drops", c.own_source_drops),
        ("not_next_hop", c.not_next_hop),
        ("originated", c.originated),
        ("isolated_drops", c.isolated_drops),
        ("isolated_refusals", c.isolated_refusals),
    ]
}

/// Every decision class under its Rust field name. The pattern has no
/// `..`, so a class added to `ControlDecisionCounters` stops this compiling
/// until it is named here.
fn control_entries(c: ControlDecisionCounters) -> [(&'static str, ControlServiceCounters); 8] {
    let ControlDecisionCounters {
        i_am,
        init_ack,
        init_mgmt,
        busy,
        available,
        reject,
        i_could_be,
        init_query,
    } = c;
    [
        ("i_am", i_am),
        ("init_ack", init_ack),
        ("init_mgmt", init_mgmt),
        ("busy", busy),
        ("available", available),
        ("reject", reject),
        ("i_could_be", i_could_be),
        ("init_query", init_query),
    ]
}

/// One class's totals under their Rust names, exhaustively as above.
fn service_entries(c: ControlServiceCounters) -> [(&'static str, u64); 3] {
    let ControlServiceCounters {
        allow_total,
        deny_total,
        policy_deny_total,
    } = c;
    [
        ("allow_total", allow_total),
        ("deny_total", deny_total),
        ("policy_deny_total", policy_deny_total),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fields `Debug` lists for a flat struct, with their values.
    fn debug_fields(debug: &str) -> Vec<(String, String)> {
        let body = &debug[debug.find('{').unwrap() + 1..debug.rfind('}').unwrap()];
        body.split(',')
            .filter_map(|field| field.split_once(':'))
            .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
            .collect()
    }

    /// Every engine count is in the dict, in `Counters`' order, each read
    /// from its own field: a field added to `Counters` and not here, or a
    /// name that reads another field, fails this.
    #[test]
    fn every_engine_count_is_named_and_reads_its_own_field() {
        let fields = debug_fields(&format!("{:?}", Counters::default()));
        let names: Vec<_> = engine_entries(&Counters::default())
            .iter()
            .map(|(name, _)| name.to_string())
            .collect();
        let field_names: Vec<_> = fields.iter().map(|(name, _)| name.clone()).collect();
        assert_eq!(names, field_names);
        for (index, name) in names.iter().enumerate() {
            let mut counters = Counters::default();
            set(&mut counters, name, 7);
            // `set` changed the field of that name and no other ...
            for (field, value) in debug_fields(&format!("{counters:?}")) {
                assert_eq!(value, if field == *name { "7" } else { "0" }, "{name}");
            }
            // ... and the dict reads it under that name alone.
            for (other, (_, count)) in engine_entries(&counters).iter().enumerate() {
                assert_eq!(*count, if other == index { 7 } else { 0 }, "{name}");
            }
        }
    }

    /// Set the field `name` of `counters` to `value`.
    fn set(counters: &mut Counters, name: &str, value: u64) {
        let field = match name {
            "address_length_drops" => &mut counters.address_length_drops,
            "global_broadcast_dadr_drops" => &mut counters.global_broadcast_dadr_drops,
            "broadcast_pdu_type_drops" => &mut counters.broadcast_pdu_type_drops,
            "group_dadr_drops" => &mut counters.group_dadr_drops,
            "malformed" => &mut counters.malformed,
            "bad_messages" => &mut counters.bad_messages,
            "unknown_port" => &mut counters.unknown_port,
            "hop_count_drops" => &mut counters.hop_count_drops,
            "denied" => &mut counters.denied,
            "unroutable_rejects" => &mut counters.unroutable_rejects,
            "rejects" => &mut counters.rejects,
            "solicitations" => &mut counters.solicitations,
            "write_failures" => &mut counters.write_failures,
            "egress_too_long" => &mut counters.egress_too_long,
            "refused" => &mut counters.refused,
            "undrained" => &mut counters.undrained,
            "busy_announced" => &mut counters.busy_announced,
            "available_announced" => &mut counters.available_announced,
            "duplicate_drops" => &mut counters.duplicate_drops,
            "own_source_drops" => &mut counters.own_source_drops,
            "not_next_hop" => &mut counters.not_next_hop,
            "originated" => &mut counters.originated,
            "isolated_drops" => &mut counters.isolated_drops,
            "isolated_refusals" => &mut counters.isolated_refusals,
            other => panic!("no count {other}"),
        };
        *field = value;
    }

    /// Each decision class reads its own field, and each total its own.
    #[test]
    fn every_control_class_and_total_reads_its_own_field() {
        let mut control = ControlDecisionCounters::default();
        let rows = [
            &mut control.i_am,
            &mut control.init_ack,
            &mut control.init_mgmt,
            &mut control.busy,
            &mut control.available,
            &mut control.reject,
            &mut control.i_could_be,
            &mut control.init_query,
        ];
        for (index, row) in rows.into_iter().enumerate() {
            let base = index as u64 * 10;
            *row = ControlServiceCounters {
                allow_total: base + 1,
                deny_total: base + 2,
                policy_deny_total: base + 3,
            };
        }
        let names: Vec<_> = control_entries(control)
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(
            names,
            [
                "i_am",
                "init_ack",
                "init_mgmt",
                "busy",
                "available",
                "reject",
                "i_could_be",
                "init_query"
            ]
        );
        for (index, (_, totals)) in control_entries(control).into_iter().enumerate() {
            let base = index as u64 * 10;
            assert_eq!(
                service_entries(totals),
                [
                    ("allow_total", base + 1),
                    ("deny_total", base + 2),
                    ("policy_deny_total", base + 3),
                ]
            );
        }
    }
}
