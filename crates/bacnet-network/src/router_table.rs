//! The routing table a [`BACnetRouter`](crate::router::BACnetRouter) keeps
//! (Clause 6.6.1), which is `bacnet-router-core`'s [`Table`] (#1679).
//!
//! The table maps each network the router can reach to a [`Route`]: the
//! port it is reached through, the next router's MAC (none for a directly
//! attached network) and a [`Reachability`] that keeps a lasting failure
//! apart from congestion. [`RouterTable`] is the size a hosted router
//! keeps, and [`BACnetRouter::table`](crate::router::BACnetRouter::table)
//! lends its live table.
//!
//! Times in the table are the router's own: [`Instant`]s on Tokio time
//! since the router started, which
//! [`BACnetRouter::now`](crate::router::BACnetRouter::now) reads. Calls on
//! the table are local management, which no wire-control policy sees.

pub use bacnet_router_core::message::RejectReason;
pub use bacnet_router_core::table::{
    ClaimCounters, Convergence, LearnRefusal, Learned, Pending, Reachability, RejectEffect, Route,
    Table, Timing,
};
pub use bacnet_router_core::{Duration, Instant, Mac, MacTooLong, PortId};

/// The most routes a hosted router's table holds, direct ones included:
/// its route cap. Learning, Initialize-Routing-Table updates and stored
/// I-Could-Be-Router offers stop at it. It is the route cap of the
/// engine's `Options::HOSTED`, and a compile-time check keeps the two the
/// same.
pub const ROUTE_CAP: usize = 256;

/// The reject hold-down records a hosted router's table keeps, one per
/// network and arrival port. A full pool gives up its oldest record and
/// counts it in [`ClaimCounters::hold_evicted`].
pub const HOLD_RECORDS: usize = 1024;

/// A hosted router's routing table: up to [`ROUTE_CAP`] routes and
/// [`HOLD_RECORDS`] hold-down records.
pub type RouterTable = Table<ROUTE_CAP, HOLD_RECORDS>;
