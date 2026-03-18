//! BTL (BACnet Testing Laboratories) compliance test harness.
//!
//! Implements the BTL Test Plan 26.1 as an automated test suite for verifying
//! BACnet device compliance. Can test our own server (self-test) or external IUTs.

pub mod engine;
pub mod iut;
pub mod report;
pub mod self_test;
pub mod tests;
