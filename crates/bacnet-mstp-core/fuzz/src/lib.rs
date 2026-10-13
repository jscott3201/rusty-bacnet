//! A harness that drives one `bacnet-mstp-core` `MasterCore` from arbitrary
//! input, as an adapter would, and checks it after every step against
//! invariants worked out independently of the core.
//!
//! - [`input`]: the fuzz input, a station's setup and a list of steps.
//! - [`driver`]: feeds the steps to the core with injected time, plays the
//!   adapter (polls every action, starts and completes frames, hears its own
//!   echo), and checks the adapter contract on each action.
//! - [`line`]: the line as the station hears it, rebuilt with a decoder of
//!   the checker's own.
//! - [`checker`]: token custody, the limits on frames and polls, answers and
//!   deliveries, from the line and the station's own frames.
//! - [`queues`]: the core's queues, wrapped to check their bounds and to
//!   see what they hand out.
//!
//! The checker never reads the core's state or calls its predicates: it
//! learns only what an observer on the line and at the adapter would, and
//! judges it by Clause 9.5 and the core's documented contract. The target's
//! doc comment lists what it checks.

pub mod checker;
pub mod driver;
pub mod input;
pub mod line;
pub mod model;
pub mod queues;

#[cfg(test)]
mod regressions;

pub use driver::run;
pub use input::Input;
