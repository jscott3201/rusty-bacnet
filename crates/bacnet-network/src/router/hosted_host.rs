//! What the engine asks of the hosted router during a call: its
//! [`Host`] hooks (#1679).
//!
//! [`HostedHost`] answers for one arrival frame, or for the sweep:
//!
//! - **Authorization** (RB-09): each protected control goes to the
//!   [`ControlGate`], with the arrival's provenance.
//! - **Logs**: each event the engine reports is logged with the message and
//!   level the router used before the engine. The two that can come once
//!   per frame, a spent hop count and an NPDU too long for its egress port
//!   (#1665), warn at most once per interval, with the count since, as a
//!   failed send does (#1672), and log the rest at debug.
//!
//! The hooks run under the engine's lock, so they never block and never
//! call into the router.
//!
use bacnet_router_core::router::{Claim, ControlClass, Event, Host};
use bacnet_transport::port::TransportProvenance;
use bacnet_transport::warn_throttle::WarnThrottle;
use bacnet_types::MacAddr;
use tracing::{debug, info, warn};

use super::control_policy::{ControlAuthContext, ControlGate, ControlTrust};
use crate::router_table::ROUTE_CAP;

/// The router's throttles for the engine's warnings that can come once
/// per frame.
#[derive(Debug, Default)]
pub(super) struct EngineWarnings {
    /// Relays dropped for a spent hop count.
    hop_count: WarnThrottle,
    /// NPDUs too long for an egress port (R1, #1665).
    too_long: WarnThrottle,
}

/// The [`Host`] for one call into the engine.
pub(super) struct HostedHost<'a> {
    /// The wire-control gate.
    gate: &'a ControlGate,
    /// The router's warning throttles.
    warnings: &'a EngineWarnings,
    /// How the transport learned of the arrival's origin; unverified for
    /// the sweep, which authorizes nothing.
    provenance: TransportProvenance,
}

impl<'a> HostedHost<'a> {
    /// The host for a frame that arrived with `provenance`.
    pub(super) fn new(
        gate: &'a ControlGate,
        warnings: &'a EngineWarnings,
        provenance: TransportProvenance,
    ) -> Self {
        Self {
            gate,
            warnings,
            provenance,
        }
    }
}

impl Host for HostedHost<'_> {
    fn authorize(&mut self, claim: &Claim<'_>) -> bool {
        let ctx = ControlAuthContext {
            port_idx: usize::from(claim.port.0),
            port_network: claim.port_network,
            source_mac: MacAddr::from_slice(claim.source),
            provenance: self.provenance,
            trust: ControlTrust::from_provenance(self.provenance),
            message_type: claim.message_type,
            vendor_id: claim.vendor_id,
            source_net: claim.snet,
            dest_net: claim.dnet,
            target_nets: claim.targets.iter().collect(),
            targets_omitted: claim.targets_omitted,
        };
        self.gate.decide(&ctx, claim.class)
    }

    fn observe(&mut self, event: Event) {
        match event {
            Event::HopCountSpent { network } => match self.warnings.hop_count.note() {
                Some(dropped) => warn!(
                    network,
                    dropped,
                    "Router dropped NPDUs whose hop count was spent (since the last warning)"
                ),
                None => debug!(network, "Router dropped an NPDU whose hop count was spent"),
            },
            Event::TooLong {
                port,
                octets,
                limit,
                rejected,
            } => {
                let port = port.0;
                match self.warnings.too_long.note() {
                    Some(refused) => warn!(
                        port,
                        octets,
                        limit,
                        ?rejected,
                        refused,
                        "Router refused NPDUs too long for their egress port (since the last warning)"
                    ),
                    None => debug!(
                        port,
                        octets,
                        limit,
                        ?rejected,
                        "Router refused an NPDU too long for its egress port"
                    ),
                }
            }
            event => log(event),
        }
    }
}

/// Log `event` as the router logged it before the engine.
fn log(event: Event) {
    match event {
        Event::Learned {
            network,
            port,
            class,
        } => {
            let port = port.0;
            if class == ControlClass::InitAck {
                debug!(network, port, "Learned route from Init-Routing-Table-Ack");
            } else {
                debug!(network, port, "Learned route from I-Am-Router-To-Network");
            }
        }
        Event::Flapping { network, flaps } => warn!(
            network,
            flap_count = flaps,
            "Route flapping detected — network changed ports {} times in 60s",
            flaps
        ),
        Event::RouteCap { class } => match class {
            ControlClass::IAm => warn!(
                "Router table learned routes cap ({ROUTE_CAP}) reached, ignoring further networks"
            ),
            ControlClass::InitMgmt => {
                warn!("Init-Routing-Table: route cap reached, ignoring further entries")
            }
            ControlClass::ICouldBe => warn!(
                "Router table routes cap ({ROUTE_CAP}) reached, I-Could-Be-Router offer not stored"
            ),
            // An Init-ACK at the cap stops its learning without a log.
            _ => {}
        },
        Event::RejectReceived {
            network, reason, ..
        } => warn!(
            network,
            reason = reason.to_raw(),
            "Received Reject-Message-To-Network"
        ),
        Event::Marked {
            network,
            busy: true,
        } => debug!(
            network,
            "Router busy — marked network as congested (30s timer)"
        ),
        Event::Marked {
            network,
            busy: false,
        } => debug!(network, "Router available — cleared congestion"),
        Event::UpdateApplied { network, port } => debug!(
            network,
            port = port.0,
            "Applied route from Init-Routing-Table management update"
        ),
        Event::UpdateRemoved { network } => {
            debug!(network, "Init-Routing-Table purged learned route")
        }
        Event::UnknownPortId { network, port_id } => warn!(
            network,
            port_id, "Init-Routing-Table: unknown Port ID, entry skipped"
        ),
        Event::ICouldBe {
            network,
            performance_index,
            stored,
        } => {
            debug!(
                network,
                performance_index, "Received I-Could-Be-Router-To-Network"
            );
            if stored {
                debug!(
                    network,
                    "Stored potential route from I-Could-Be-Router-To-Network"
                );
            }
        }
        Event::EstablishConnection {
            network,
            termination_time,
        } => info!(
            network,
            termination_time_minutes = termination_time,
            "Received Establish-Connection-To-Network (PTP not implemented)"
        ),
        Event::DisconnectIgnored { network } => debug!(
            network,
            "Received Disconnect-Connection-To-Network (PTP not implemented; route removal ignored)"
        ),
        Event::SecurityMessage { message_type } => debug!(
            message_type = message_type.to_raw(),
            "Router received security network message (not implemented)"
        ),
        Event::UnknownMessage { message_type } => debug!(
            message_type = message_type.to_raw(),
            "Router rejecting unknown network message type"
        ),
        Event::NumberConflict {
            local,
            reported,
            configured: true,
            ..
        } => warn!(
            local_network = local,
            peer_network = reported,
            "Network number conflict: port configured as {} but peer reports {} (configured)",
            local,
            reported
        ),
        Event::NumberConflict {
            local, reported, ..
        } => debug!(
            local_network = local,
            peer_network = reported,
            "Network-Number-Is from peer (learned, differs from local)"
        ),
        Event::OwnOrigin { network } => debug!(
            network,
            "Router refused an NPDU it originated itself; no reject sent"
        ),
        Event::Purged { network } => debug!(network, "Purged stale route"),
        Event::PendingExpired { network } => {
            debug!(network, "Expired hardened pending challenger")
        }
        Event::DiscoveryExpired { network } => {
            debug!(network, "Expired discovery solicitation")
        }
        // An event a later engine adds.
        _ => debug!(?event, "Router engine event"),
    }
}
