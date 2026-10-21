//! What the node scenario reports in a good run. `tests/node.rs` compares
//! the native run's summary with it, and the emulation build of the `node`
//! image the emulated CPU's, so the two can't drift apart.

use bacnet_bip_codec::BvlcResultCode;
use bacnet_bip_core::node::{
    Admission, AttemptOutcome, Discard, ExchangeCounters, NodeCounters, SendRefusal,
};
use bacnet_bip_core::{Duration, Instant};

use super::{Attempt, Expected, Registration, Summary, BBMD, ORIGIN, PEER, TTL};
use crate::tally::Sent;

/// The hash over every datagram the port sends and where it goes. It was
/// worked out apart from the codec, from the 17 datagrams written out by
/// hand from the Annex J.2 layouts (120 octets): the six NAKs, eight
/// Register-Foreign-Device requests, and three Distribute-Broadcast-To-
/// Network messages. The port sends what the scenario sent by hand before
/// it (#1766), in the same order.
const SENT_FNV: u32 = 0x8F8B_EFDE;

const fn at(millis: u64) -> Instant {
    Instant::from_millis(millis)
}

const SUCCESS: BvlcResultCode = BvlcResultCode::SUCCESSFUL_COMPLETION;
const DBTN_NAK: BvlcResultCode = BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK;

/// What the node counts in a good run, but the newest attempt
/// ([`Expected::last_attempt`]). The counters are `#[non_exhaustive]`, so
/// they are built from their default.
fn counters() -> NodeCounters {
    // Eight attempts sent, five answered with success, three timed out.
    let mut register = ExchangeCounters::default();
    register.sent = 8;
    register.results = 5;
    register.last_result = Some(SUCCESS);
    register.timeouts = 3;
    let mut counters = NodeCounters::default();
    counters.register_foreign_device = register;
    counters.registration_attempts = 8;
    counters.registrations_accepted = 5;
    // The DBTN NAK, taken as the hint.
    counters.unmatched_responses = 1;
    counters.last_unmatched_result = Some(DBTN_NAK);
    counters.renewal_hints = 1;
    counters.lapses = 1;
    counters.broadcasts_held = 2;
    counters.broadcasts_released = 2;
    counters.broadcasts_hold_full = 1;
    counters
}

/// The summary [`run`](super::run) returns in a good run.
#[must_use]
pub fn expected() -> Expected {
    let summary = Summary {
        // A 4-octet header for a unicast; for a broadcast, 6 octets more, the
        // originating address a BBMD adds when it forwards it, out of the
        // 64-octet budget. The hosted transport's 1507 leaves room for 1497
        // either way.
        max_npdu: [60, 54],
        // Each request only a BBMD serves draws its own NAK, which the
        // port sends back to the peer.
        naks: [
            Some((PEER, BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK)),
            Some((PEER, BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK)),
            Some((PEER, BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK)),
            Some((PEER, BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK)),
            Some((PEER, BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK)),
            Some((PEER, DBTN_NAK)),
        ],
        // Two broadcasts held while the first attempt is out and a third
        // refused; once it succeeded, one sent at once (at 39.99 s).
        admissions: [
            Some(Ok(Admission::Hold)),
            Some(Ok(Admission::Hold)),
            Some(Err(SendRefusal::HoldFull)),
            Some(Ok(Admission::Send)),
        ],
        released: 2,
        // Delivered from where it started, as a broadcast; the stranger's
        // copy is dropped.
        forwarded: Some((ORIGIN, true)),
        untrusted: Some(Discard::ForwardedUntrusted),
        // At 0, and its renewal one 30 s interval into the lease. The DBTN
        // NAK at 40 s brings the next one forward from 60 s to then. Then
        // the BBMD goes quiet: 70 s, and after each 3 s timeout a wait that
        // doubles, 3 s then 6 s, so 76 s and 85 s. The next wait, 12 s, is
        // cut to the renewal after the 70 s attempt, so at 100 s, as the
        // lease lapses, it starts the attempt that is answered. That
        // answer's lease starts at the unanswered 70 s attempt, which it
        // could also be answering, and is due for renewal at once (100.01 s).
        attempts: [
            Some(at(0)),
            Some(at(30_000)),
            Some(at(40_000)),
            Some(at(70_000)),
            Some(at(76_000)),
            Some(at(85_000)),
            Some(at(100_000)),
            Some(at(100_010)),
        ],
        // Each lease runs one time-to-live, 60 s, from the earliest send its
        // success could answer. The three attempts in between go unanswered
        // and the lease lapses: the counters' three timeouts and one lapse.
        leases: [
            Some((at(0), at(60_000))),
            Some((at(30_000), at(90_000))),
            Some((at(40_000), at(100_000))),
            Some((at(70_000), at(130_000))),
            Some((at(100_000), at(160_000))),
            None,
        ],
        // In `Expected::registration`.
        registration: None,
        counters: counters(),
        // Six NAKs and eight registrations of 6 octets, and three
        // Distribute-Broadcast-To-Network messages of 12.
        sent: Sent {
            count: 17,
            octets: 120,
            fnv: SENT_FNV,
        },
        refused: 0,
        errors: 0,
    };
    Expected {
        summary,
        // At 100.02 s: the last renewal's lease, from the 100 s attempt, and
        // the next renewal one interval into it.
        registration: Registration {
            lease: Some((at(100_000), at(160_000))),
            registered: true,
            lease_remaining: Some(Duration::from_millis(59_980)),
            attempt_in_flight: false,
            next_attempt: Some(at(130_000)),
            last_outcome: Some(AttemptOutcome::Accepted),
            consecutive_failures: 0,
            first_resolved: true,
            held: 0,
        },
        last_attempt: Attempt {
            bbmd: BBMD,
            ttl: TTL,
            outcome: AttemptOutcome::Accepted,
            result: Some(SUCCESS),
        },
    }
}
