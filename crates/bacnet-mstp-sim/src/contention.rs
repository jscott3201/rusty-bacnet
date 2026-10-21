//! Synthetic digital observations on the shared physical character grid.

use crate::line::{Character, Contributor, WindowOutcome};
use crate::Driver;
use crate::SimRng;

/// Opt-in receiver policy for real overlapping drivers. Uncontended input
/// is unchanged. All contributors must span the same complete character
/// window; misalignment, truncation, explicit framing errors and receivers
/// enabled midway through a character remain errors. These policies do not
/// model electrical arbitration, UART resynchronization or clock drift.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ContentionPolicy {
    /// Conservative default: every overlap produces a framing error.
    #[default]
    FramingError,
    /// Observe this contributor's actual characters when aligned with the
    /// common window; an absent contributor produces a framing error.
    Capture(Driver),
    /// Capture a contributor, but XOR one indexed character while retaining
    /// a clean UART indication. CRC checks still run normally. An explicit
    /// corruption already applied to this character takes precedence, so a
    /// second XOR cannot undo that fault.
    Xor {
        /// Contributor to observe.
        driver: Driver,
        /// Index within that contributor's transmission.
        offset: usize,
        /// Bits to flip at the selected index.
        mask: u8,
    },
    /// Independently choose framing error, capture or altered data per
    /// aligned contended window, from a stable per-receiver seeded stream.
    /// The three choices have equal weight. Capture/alter choose uniformly
    /// among actual contributors; alteration uses a nonzero mask. Unaligned
    /// or ineligible input consumes no draws. Streams derive from the run
    /// seed and receiver station, persist across reboot, and are independent
    /// of scenario, timer-jitter, chunk-size and other receivers' draws.
    Seeded,
}

/// Compact per-receiver accounting, retained across power cycles. The
/// separate bounded codec instance describes actual forwarded input, not a
/// second MAC or an independently implemented decoder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContentionStats {
    /// Eligible contended windows observed under an opt-in policy.
    pub windows: u64,
    /// UART-clean observations equal to the selected contributor's octet.
    pub captured: u64,
    /// UART-clean observations altered by the contention policy.
    pub altered: u64,
    /// Framing-error observations under an opt-in policy.
    pub framing_errors: u64,
    /// Valid frames decoded by the reference instance from forwarded input.
    pub decoded: u64,
    /// Malformed candidates found by that bounded reference decoder.
    pub malformed: u64,
    /// Complete, intact, valid contributor streams involving contention.
    pub intact: u64,
    /// Terminal inputs whose reference decode qualified a receipt check.
    pub checked: u64,
    /// Qualified terminal inputs with exactly the expected receipt count.
    pub matched: u64,
    /// Intact candidates whose effective stream was not independently valid.
    pub unqualified: u64,
    /// Candidates explicitly discarded by echo policy or power loss.
    pub cancelled: u64,
    /// Candidate receipt checks currently awaiting host forwarding.
    pub pending: usize,
    /// Highest pending candidate count, capped at 513 per receiver.
    pub max_pending: usize,
    /// Candidates refused at the 513-entry bound, with an explicit violation.
    pub overflows: u64,
}

pub(crate) fn resolve(
    policy: ContentionPolicy,
    outcome: &WindowOutcome,
    rng: &mut SimRng,
) -> (Character, Option<Contributor>) {
    if !outcome.contended {
        return (outcome.character, outcome.contributors.first().copied());
    }
    if !outcome.resolvable || outcome.contributors.is_empty() {
        return (Character::Garbled, None);
    }
    let (driver, alteration) = match policy {
        ContentionPolicy::FramingError => return (Character::Garbled, None),
        ContentionPolicy::Capture(driver) => (driver, None),
        ContentionPolicy::Xor {
            driver,
            offset,
            mask,
        } => (driver, Some((offset, mask))),
        ContentionPolicy::Seeded => {
            let choice = rng.below(3);
            if choice == 0 {
                return (Character::Garbled, None);
            }
            let contributor =
                outcome.contributors[rng.below(outcome.contributors.len() as u64) as usize];
            let alteration = (choice == 2).then(|| (contributor.index, rng.range(1, 255) as u8));
            (contributor.driver, alteration)
        }
    };
    let Some(contributor) = outcome
        .contributors
        .iter()
        .find(|c| c.driver == driver)
        .copied()
    else {
        return (Character::Garbled, None);
    };
    let alteration = alteration.filter(|_| !contributor.faulted);
    let character = match (contributor.character, alteration) {
        (Character::Clean(byte), Some((index, mask))) if index == contributor.index => {
            Character::Clean(byte ^ mask)
        }
        (character, _) => character,
    };
    (character, Some(contributor))
}
