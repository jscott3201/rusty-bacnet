//! Explicit evidence and stage results for local accompaniment evaluation.
use super::AccessRightsEvaluation;
use crate::prelude::*;
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{AccessEvent, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use core::time::Duration;

/// One caller-identified credential presentation. Times share a caller-chosen
/// monotonic origin; they are not BACnet wall-clock timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialPresentation {
    /// Unique increasing presentation identity/order at this access point.
    /// Distinct presentations may have the same `at`, but not this sequence.
    pub sequence: u64,
    /// Local Access Credential object presented.
    pub credential: ObjectIdentifier,
    /// Local Access Point at which it was presented.
    pub point: ObjectIdentifier,
    /// Monotonic elapsed time of presentation.
    pub at: Duration,
}

/// The application's required presentation order (Clause 12.34.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccompanimentOrder {
    /// Companion sequence must precede the dependent presentation.
    Before,
    /// Companion sequence must follow the dependent presentation.
    After,
    /// Either order is acceptable; identity and timing still matter.
    Either,
}

/// Whether a presentation exactly one period away remains in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccompanimentDeadline {
    /// A difference equal to the period is eligible; expiry is strictly later.
    Inclusive,
    /// A difference equal to the period is already outside the window.
    Exclusive,
}

/// Mandatory local timing choices; this helper supplies no security defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccompanimentTiming {
    /// Permitted presentation order, including ties resolved by sequence.
    pub order: AccompanimentOrder,
    /// Used only when the point omits Accompaniment_Time. A served Unsigned
    /// value, in seconds, takes precedence (Clause 12.31.26). Invalid reads
    /// never fall back. Zero is an actual zero-sized period, not unlimited.
    pub fallback_period: Duration,
    /// Explicit treatment of the period's boundary.
    pub deadline: AccompanimentDeadline,
}

/// What the caller has observed for one accompaniment dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccompanimentEvidence {
    /// No candidate has arrived. The result stays Waiting until `now` reaches
    /// expiry according to the explicit period and deadline edge.
    Waiting,
    /// The caller reports an expired wait. An inconsistent early report is
    /// unresolved; the helper verifies the deadline against `now`.
    DeadlineReached,
    /// An attempted companion. Wrong-point, future or inconsistent evidence
    /// is unresolved and does not create an accompaniment event at this point.
    Presented(CredentialPresentation),
}

/// Evidence attached to one presentation, including a chain's child links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccompanimentWitness {
    /// The dependent presentation's unique sequence.
    pub original: u64,
    /// An explicit waiting, expired or received state.
    pub evidence: AccompanimentEvidence,
}

/// Inputs to one pure accompaniment-stage evaluation.
#[derive(Debug, Clone, Copy)]
pub struct AccompanimentRequest<'a> {
    /// The transaction whose stage result and event intents are requested.
    pub original: CredentialPresentation,
    /// Caller-supplied monotonic time, sharing every presentation's origin.
    pub now: Duration,
    /// One witness per evaluated dependency. Missing or duplicate witnesses
    /// are unresolved, never interpreted as a timeout or successful absence.
    pub witnesses: &'a [AccompanimentWitness],
    /// Required application policy, also used at each child dependency.
    pub timing: AccompanimentTiming,
    /// Maximum dependency links to inspect. Exhaustion is unresolved. The
    /// iterative traversal also stops on repeated credentials/presentations;
    /// neither this limit nor a cycle can produce a successful stage result.
    pub max_links: usize,
}

/// Why a reference cannot be established locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccompanimentReferenceFailure {
    /// The reference names another device; no remote read is attempted.
    Remote,
    /// A partially unspecified reference names no concrete local object/device.
    Wildcard,
    /// Its object or device member has the wrong object type.
    WrongType,
    /// The named local object is absent.
    Missing,
}

/// Inconsistent or ineligible caller presentation evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccompanimentPresentationFailure {
    /// The presentation names another access point.
    WrongPoint,
    /// A supplied presentation time lies after the evaluation time.
    Future,
    /// Presentation sequence and time order disagree.
    NonMonotonic,
    /// One presentation sequence was reused with inconsistent content.
    ReusedSequence,
    /// A DeadlineReached witness was supplied before the verified expiry.
    PrematureDeadline,
    /// A previously presented candidate is older than the allowed period.
    OutsideWindow,
}

/// Evidence preventing an accompaniment-stage conclusion. These conditions
/// do not invent BACnet denial events or override the existing rights policy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccompanimentProblem {
    /// A selected requirement or relevant ownership reference is unavailable.
    Reference {
        /// The reference that could not be established.
        reference: BACnetDeviceObjectReference,
        /// The local resolution limit.
        reason: AccompanimentReferenceFailure,
    },
    /// A served property could not be read or decoded as its required type.
    Property {
        /// Object serving the property.
        object: ObjectIdentifier,
        /// Property read.
        property: PropertyIdentifier,
        /// Original error or datatype diagnostic.
        detail: String,
    },
    /// No rights result could be obtained for a received companion. Excluded
    /// assignments in a successful rights result are diagnostics instead.
    Rights {
        /// Presentation whose rights read failed.
        presentation: u64,
        /// Error from the existing rights evaluator.
        detail: String,
    },
    /// The caller supplied no state for a required dependency.
    MissingWitness {
        /// Presentation needing a witness.
        presentation: u64,
    },
    /// More than one witness names the same dependency.
    AmbiguousWitness {
        /// Presentation with conflicting witness rows.
        presentation: u64,
    },
    /// A presentation or deadline assertion is inconsistent or ineligible.
    Presentation {
        /// Presentation whose evidence is at issue.
        presentation: u64,
        /// The failed consistency check.
        reason: AccompanimentPresentationFailure,
    },
    /// A dependency returns to an earlier credential/presentation after a
    /// distinct companion. No cyclic fixed-point authorization is assumed.
    CyclicRequirement {
        /// Traversed presentations followed by the repeated dependency.
        path: Vec<CredentialPresentation>,
    },
    /// The caller's finite traversal budget was exhausted.
    LinkLimit {
        /// Maximum permitted dependency links.
        max_links: usize,
    },
}

/// The accompaniment stage, never a whole-request authorization decision.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccompanimentOutcome {
    /// Original rights denied; accompaniment was not evaluated.
    NotEvaluated,
    /// Original rights are exempt, or their selected object has no requirement.
    NotRequired,
    /// The required local chain is established. Authentication, credential
    /// status and every other authorization criterion remain outside this stage.
    Satisfied,
    /// A dependency is waiting within its period. No denial intent is emitted.
    Waiting {
        /// Presentation awaiting its companion (possibly a child dependency).
        presentation: u64,
    },
    /// The original transaction's own wait expired without an eligible companion.
    DeniedNoCompanion,
    /// A known invalid attempt or child dependency invalidates the original's
    /// companion. Only the original transaction receives the denial intent.
    DeniedIncorrectCompanion {
        /// Dependent presentation at which the known failure was established.
        presentation: u64,
    },
    /// Local evidence is insufficient; no terminal event is inferred.
    Unresolved(AccompanimentProblem),
}

/// An event the caller may publish for the result's **original transaction**.
/// A companion presentation identifies the associated credential; it does not
/// create a second transaction (Clauses 12.31.27.1 and 12.31.30).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccompanimentEventIntent {
    /// Original transaction identity and point, retained even when this intent
    /// is copied out of its evaluation or names an associated companion.
    pub original: CredentialPresentation,
    /// ACCOMPANIMENT_BY or an accompaniment denial; never GRANTED.
    pub access_event: AccessEvent,
    /// Companion for BY, original for a denial.
    pub presentation: CredentialPresentation,
}

/// Child evidence inspected to establish the original's companion. It carries
/// no event intents: the caller evaluates a child's transaction separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccompanimentCompanion {
    /// Received companion presentation.
    pub presentation: CredentialPresentation,
    /// Existing rights result, including excluded-assignment diagnostics.
    pub rights: AccessRightsEvaluation,
    /// The child's selected requirement when successfully read. None also
    /// accompanies an unresolved read, distinguished by the overall outcome.
    pub requirement: Option<BACnetDeviceObjectReference>,
}

/// Pure local stage evidence plus ordered intents for one original transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccompanimentEvaluation {
    /// The original request/presentation identity and access point.
    pub original: CredentialPresentation,
    /// Rights recomputed under the same database borrow. Its decision and
    /// excluded-assignment policy remain those of evaluate_access_rights.
    pub original_rights: AccessRightsEvaluation,
    /// Requirement selected by the original's first winning Access Rights
    /// object. No later positive rule is tried to avoid this requirement.
    pub requirement: Option<BACnetDeviceObjectReference>,
    /// Rights/requirement evidence for distinct companions, in traversal order.
    pub companions: Vec<AccompanimentCompanion>,
    /// Accompaniment-stage outcome, not complete authorization.
    pub outcome: AccompanimentOutcome,
    /// Original transaction intents, in order: BY before any resulting denial.
    /// Child inspections produce no publication intents. Repeated evaluation
    /// returns the same intents; exactly-once publication is the caller's job.
    pub events: Vec<AccompanimentEventIntent>,
}
