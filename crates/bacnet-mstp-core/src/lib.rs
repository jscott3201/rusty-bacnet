//! A synchronous, platform-neutral BACnet MS/TP master core.
//!
//! The core owns no clock, serial port, executor, lock, or system resource.
//! Callers supply timestamped events and execute the actions returned by
//! [`MasterCore::poll_action`].

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod actions;
mod clock;
mod config;
mod counters;
mod queue;
mod state;

pub use actions::{Action, Direction, RequestId, TransmitFailure, TransmitId};
pub use clock::{Duration, Instant, Monotonic, SimClock};
pub use config::{ConfigError, MstpCoreConfig};
pub use counters::{MstpCounters, QueueCounters};
#[cfg(feature = "alloc")]
pub use queue::AllocQueues;
#[cfg(feature = "heapless")]
pub use queue::HeaplessQueues;
pub use queue::{DequeuedNpdu, NetworkPriority, QueueKind, QueueRejectReason, QueueStorage};
pub use state::{CoreError, MasterCore, MasterSnapshot, MasterState, ReceiveError, ReplyDecision};
