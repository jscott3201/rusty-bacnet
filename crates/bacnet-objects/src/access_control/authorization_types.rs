//! Explicit policies and outcomes for a pure local authorization pass.

use super::{
    AccessRightsEvaluation, AccompanimentEvaluation, AccompanimentProblem, CredentialPresentation,
};
use crate::prelude::*;
use bacnet_types::enums::{
    AccessCredentialDisableReason, AccessEvent, AuthorizationExemption, AuthorizationMode,
    BinaryPV, ErrorClass, ErrorCode, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

/// A required criterion in the application's chosen evaluation order.
/// Activity and the validity window share one coherent credential check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationCheck {
    /// Status, disable reasons, and inclusive validity limits.
    Credential,
    /// Assigned rights and the selected accompaniment dependency.
    AccessRights,
    /// Credential authority against the point's threat level.
    Threat,
    /// Enforced upper or lower occupancy bound for relevant zones.
    Occupancy,
    /// Established passback violation and successful-entry timeout.
    Passback,
    /// Point lockout, separate from credential disablement.
    Lockout,
    /// DENY_ALL enforcement. Other supported modes pass this criterion.
    Mode,
}

impl AuthorizationCheck {
    pub(super) const fn index(self) -> usize {
        match self {
            Self::Credential => 0,
            Self::AccessRights => 1,
            Self::Threat => 2,
            Self::Occupancy => 3,
            Self::Passback => 4,
            Self::Lockout => 5,
            Self::Mode => 6,
        }
    }
}

/// A complete, duplicate-free criterion order; there is no default order.
/// The mode's verification/delay phase follows all these criteria.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationOrder([AuthorizationCheck; 7]);

impl AuthorizationOrder {
    /// Validate that every criterion appears exactly once.
    ///
    /// # Errors
    /// Returns VALUE_OUT_OF_RANGE for a duplicate, which necessarily omits
    /// another required criterion from this seven-element order.
    pub fn new(checks: [AuthorizationCheck; 7]) -> Result<Self, Error> {
        let mut seen = [false; 7];
        for check in checks {
            if core::mem::replace(&mut seen[check.index()], true) {
                return Err(crate::common::value_out_of_range_error());
            }
        }
        Ok(Self(checks))
    }

    /// The validated evaluation order.
    pub fn checks(&self) -> &[AuthorizationCheck; 7] {
        &self.0
    }
}

/// Precedence among all ten standard credential disable reasons.
/// Unknown/vendor reasons remain unresolved even beside a known reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisableReasonPriority([AccessCredentialDisableReason; 10]);

impl DisableReasonPriority {
    /// Supply every standard reason exactly once, highest priority first.
    ///
    /// # Errors
    /// Returns VALUE_OUT_OF_RANGE for unknown or duplicate reasons.
    pub fn new(reasons: [AccessCredentialDisableReason; 10]) -> Result<Self, Error> {
        let mut seen = [false; 10];
        for reason in reasons {
            let index = usize::try_from(reason.to_raw())
                .map_err(|_| crate::common::value_out_of_range_error())?;
            let slot = seen
                .get_mut(index)
                .ok_or_else(crate::common::value_out_of_range_error)?;
            if core::mem::replace(slot, true) {
                return Err(crate::common::value_out_of_range_error());
            }
        }
        Ok(Self(reasons))
    }

    /// Standard reasons in the caller's chosen precedence.
    pub fn reasons(&self) -> &[AccessCredentialDisableReason; 10] {
        &self.0
    }
}

/// Explicit treatment of assignments excluded by the existing rights helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationRightsPolicy {
    /// Keep its established decision and all excluded-assignment diagnostics.
    HonorExistingDecision,
    /// Any excluded assignment makes this pass unresolved, including companions.
    RequireCompleteLocalEvidence,
}

/// Location of a decision or retained check result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationStage {
    /// Object, mode, exemption, or original-presentation admission.
    Admission,
    /// One member of the explicitly ordered criteria.
    Criterion(AuthorizationCheck),
    /// External verification after the ordered criteria.
    Verification,
    /// Authorization delay after the ordered criteria.
    Delay,
}

/// A served property's failure without losing its protocol error identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationReadFailure {
    /// A BACnet error returned by the property reader.
    Protocol {
        /// BACnet error class.
        class: ErrorClass,
        /// BACnet error code.
        code: ErrorCode,
    },
    /// A served value does not have the expected datatype or structure.
    Malformed(&'static str),
    /// Another reader failure, retained for diagnosis.
    Other(String),
}

/// Why a companion authentication attestation cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionAuthenticationProblem {
    /// No attestation matches this presentation's complete tuple.
    Missing,
    /// More than one attestation matches this presentation.
    Duplicate,
    /// A matching attestation explicitly reports unknown authentication.
    Unknown,
}

/// Missing, inconsistent, or unsupported authorization evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthorizationProblem {
    /// A required property failed or had the wrong structure.
    Property {
        /// Object that served the value.
        object: ObjectIdentifier,
        /// Property requested.
        property: PropertyIdentifier,
        /// Preserved read/decoding failure.
        failure: AuthorizationReadFailure,
    },
    /// The application explicitly lacks a required operational fact.
    MissingFact(&'static str),
    /// A standard range or supported enumeration does not include this value.
    UnsupportedValue {
        /// Object carrying the value.
        object: ObjectIdentifier,
        /// Property carrying the value.
        property: PropertyIdentifier,
        /// Observed raw value.
        value: u64,
    },
    /// Credential status/reasons or their time subset disagree at the supplied moment.
    InconsistentCredentialEvidence {
        /// Credential being checked.
        credential: ObjectIdentifier,
        /// The observed inconsistency; no retry or repaired grant is attempted.
        detail: &'static str,
    },
    /// Identity, topology, or monotonic evidence is inconsistent.
    Context(&'static str),
    /// The explicit policy requires all assignments to be locally available.
    ExcludedRights {
        /// Presentation whose preserved rights diagnostics prevent a conclusion.
        presentation: CredentialPresentation,
    },
    /// Existing accompaniment-stage evidence remains unresolved.
    Accompaniment(AccompanimentProblem),
    /// Authentication was not established for this exact companion presentation.
    CompanionAuthentication {
        /// The relied-on companion tuple.
        presentation: CredentialPresentation,
        /// Missing, duplicate, or unknown evidence.
        reason: CompanionAuthenticationProblem,
    },
}

/// What one criterion established, including an explicit exemption or stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationCheckOutcome {
    /// The criterion passed using its required evidence.
    Passed,
    /// Only this criterion was bypassed by the named exemption.
    Exempt(AuthorizationExemption),
    /// The selected mode or explicitly inapplicable context needs no such check.
    NotApplicable,
    /// A known failure has this specific BACnet denial event.
    Denied(AccessEvent),
    /// Required evidence is still pending within its verified period.
    Pending,
    /// Required evidence could not establish either permission or denial.
    Unresolved(AuthorizationProblem),
}

/// An ordered record of a criterion or external phase that was actually visited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationCheckEvidence {
    /// The check or phase evaluated.
    pub stage: AuthorizationStage,
    /// Its result; later checks are absent after the first non-passing result.
    pub outcome: AuthorizationCheckOutcome,
}

/// A coherent-credential read retained even when its classifications disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationCredentialEvidence {
    /// Original or companion presentation whose credential was inspected.
    pub presentation: CredentialPresentation,
    /// Served status, including an unsupported raw value.
    pub status: BinaryPV,
    /// Served reasons, without silently discarding unknown reasons.
    pub reasons: Vec<AccessCredentialDisableReason>,
    /// Reasons derived from the two window bounds at the supplied wall time.
    /// None means finite-time evidence was unavailable or invalid.
    pub window_reasons: Option<Vec<AccessCredentialDisableReason>>,
}

/// A known companion prerequisite failure; it is not a child transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompanionPrerequisiteFailure {
    /// The caller explicitly reports rejected authentication for this tuple.
    AuthenticationRejected,
    /// The local credential is coherently disabled or outside its validity window.
    CredentialDenied {
        /// Selected credential reason under the same explicit priority.
        reason: AccessCredentialDisableReason,
        /// Diagnostic child denial, never published as a child transaction.
        access_event: AccessEvent,
    },
}

/// Diagnostic attribution for a companion that invalidates the root transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionAuthorizationDiagnostic {
    /// Companion presentation whose prerequisite failed.
    pub presentation: CredentialPresentation,
    /// The known failure, preserved separately from the root denial event.
    pub failure: CompanionPrerequisiteFailure,
}

/// Complete result of this pure authorization pass. Only Granted permits access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationOutcome {
    /// Every required criterion and external phase passed.
    Granted,
    /// A known criterion failed; its event is last in the returned root intents.
    Denied {
        /// Criterion or phase that stopped the pass.
        stage: AuthorizationStage,
        /// Specific BACnet denial event.
        access_event: AccessEvent,
    },
    /// An applicable accompaniment or external phase is still waiting.
    Pending(AuthorizationStage),
    /// Mode NONE requests no authorization, and returns no event intents.
    NoAuthorization,
    /// Required evidence is unknown or inconsistent; this never permits access.
    Unresolved {
        /// Earliest visited stage that could not conclude.
        stage: AuthorizationStage,
        /// Retained evidence problem.
        problem: AuthorizationProblem,
    },
}

/// An intent for the original transaction, with its associated presentation.
/// Publication, transaction tags, deduplication, and physical action are external.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationEventIntent {
    /// Original authenticated presentation and access point.
    pub original: CredentialPresentation,
    /// Intermediate event or terminal grant/denial.
    pub access_event: AccessEvent,
    /// Companion for ACCOMPANIMENT_BY; original for all other intents.
    pub presentation: CredentialPresentation,
}

/// Retained evidence and bounded ordered intents from one shared database borrow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationEvaluation {
    /// The authenticated original presentation supplied by the application.
    pub original: CredentialPresentation,
    /// Served mode, when admission could decode it.
    pub mode: Option<AuthorizationMode>,
    /// Full pass result; only Granted is actionable permission.
    pub outcome: AuthorizationOutcome,
    /// Visited checks, in order, stopping at the first denial/pending/unknown.
    pub checks: Vec<AuthorizationCheckEvidence>,
    /// Original assigned-rights result, when that criterion was needed.
    pub rights: Option<AccessRightsEvaluation>,
    /// Existing accompaniment result, including child rights diagnostics.
    pub accompaniment: Option<AccompanimentEvaluation>,
    /// Original/relied-on companion prerequisite reads, in inspection order.
    pub credentials: Vec<AuthorizationCredentialEvidence>,
    /// Known invalid companion prerequisites; no child events are generated.
    pub companion_diagnostics: Vec<CompanionAuthorizationDiagnostic>,
    /// Why optional SOFT reporting could not be established during a visited
    /// PASSBACK-exempt check. This API retains unknown/malformed/failed evidence
    /// here without making exempt reporting a required authorization criterion.
    /// It neither invents a detection nor permits a failed read to use fallback.
    /// None means no reporting problem was encountered, including when that
    /// criterion was not visited or its known mode needed no reporting evidence.
    pub passback_reporting_problem: Option<AuthorizationProblem>,
    /// At most four root intents: soft passback, accompaniment BY, external
    /// phase, and terminal event. Their actual order follows the check order;
    /// a terminal grant/denial is always last. Re-evaluation repeats these intents.
    pub events: Vec<AuthorizationEventIntent>,
}
