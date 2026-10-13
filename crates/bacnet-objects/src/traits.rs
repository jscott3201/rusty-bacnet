//! BACnetObject trait — the interface all BACnet objects implement.

#[path = "object_storage.rs"]
mod object_storage;
pub(crate) use object_storage::ObjectStorageAccess;

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, BACnetObjectPropertyReference,
};
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, LifeSafetyOperation, ObjectType, PropertyIdentifier,
    Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, Time};

use crate::audit::{AuditLogNotificationSink, AuditLogStorage};
use crate::clock::ClockReader;
use crate::durable::DurableWrites;
use crate::event::{
    EnrollmentSummaryCapability, EventStateChange, EventTransitionCommit,
    EventTransitionCommitError, TransitionOutcome,
};
use crate::event_enrollment::{
    EventEnrollmentEvalState, EventEnrollmentMonitoredSource, EventEnrollmentReliabilityCommit,
};
use crate::file::{FileConfiguration, FileStorage};
use crate::log_buffer::{LogBufferRecords, LogRecordIdentity};
use crate::log_reporting::BufferReadyReport;
use crate::schedule::{ScheduleTargetOutcome, ScheduleWrite};

/// Process-local monotonic time source used by internal object lifecycles.
#[doc(hidden)]
pub type MonotonicClock = dyn Fn() -> Duration + Send + Sync;

/// Wakes the server's monotonic task to read object deadlines again (#1384).
#[doc(hidden)]
pub type DeadlineWaker = dyn Fn() + Send + Sync;

mod defaults;
use defaults::{
    array_property_default, cov_reported_properties_default, historical_writable_default,
    list_property_default,
};

/// One property a whole-object (SubscribeCOV) notification reports after
/// Present_Value and Status_Flags, from the object type's Table 13-1 row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovReportedProperty {
    /// Carried in each notification; a change of it alone sends none.
    Value(PropertyIdentifier),
    /// Carried in each notification, and any change of it sends one.
    Trigger(PropertyIdentifier),
}

impl CovReportedProperty {
    /// The reported property.
    pub const fn property(self) -> PropertyIdentifier {
        match self {
            Self::Value(property) | Self::Trigger(property) => property,
        }
    }

    /// Whether any change of the property triggers a notification.
    pub const fn triggers(self) -> bool {
        matches!(self, Self::Trigger(_))
    }
}

/// Whether the stack's classification table marks `property` as a BACnetARRAY
/// on `object_type`: the answer the default [`BACnetObject::is_array_property`]
/// gives, and every built-in object's. The table holds every property a Clause
/// 12 object table of the 2020 standard types as an array, served or not. A
/// client can ask it about a remote object, before or without any value, to
/// learn the property's shape; a vendor-defined array isn't in it.
pub fn standard_array_property(object_type: ObjectType, property: PropertyIdentifier) -> bool {
    array_property_default(object_type, property)
}

/// Whether the stack's classification table marks `property` as a BACnetLIST
/// on `object_type`: the answer the default [`BACnetObject::is_list_property`]
/// gives, and every built-in object's, covering every list of the 2020 object
/// tables. Usable for a remote object as [`standard_array_property`] is.
pub fn standard_list_property(object_type: ObjectType, property: PropertyIdentifier) -> bool {
    list_property_default(object_type, property)
}

/// Result of applying a LifeSafetyOperation to an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeSafetyOperationEffect {
    /// The operation changed object state.
    Applied,
    /// The requested idempotent state was already present.
    AlreadyApplied,
}

/// Result of applying a `LifeSafetyOperation` to object-owned state.
///
/// `changed_properties` contains exact committed readback changes in the object's
/// stable reporting order, with each property at most once. Implementations own
/// this projection; the server does not infer custom object mutations.
/// [`LifeSafetyOperationEffect::AlreadyApplied`] means no state changed and must
/// carry an empty property list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifeSafetyOperationOutcome {
    /// Whether the operation committed a change or was already applied.
    pub effect: LifeSafetyOperationEffect,
    /// Exact properties whose committed readback changed.
    pub changed_properties: Vec<PropertyIdentifier>,
}

/// Result of one object-owned reliability evaluation pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReliabilityEvaluation {
    /// The object made no reliability-related mutation.
    Unchanged,
    /// The object successfully mutated its `Reliability` value.
    Changed {
        /// Reliability before the successful mutation.
        old_reliability: Reliability,
        /// Reliability after the successful mutation.
        new_reliability: Reliability,
    },
}

/// The core trait for all BACnet objects.
///
/// Stored objects have crate-authorized concrete access; borrowed read views
/// remain valid implementors. Applications cannot manufacture its access permit:
/// ```compile_fail
/// use bacnet_objects::traits::ObjectStorageAccess;
/// let permit = ObjectStorageAccess(());
/// ```
/// Implementors represent a single BACnet object (Device, AnalogInput, etc.)
/// and provide read/write access to their properties.
pub trait BACnetObject: Send + Sync + object_storage::StoredObject {
    /// Typed local Device authority for composition-owned configuration.
    /// This does not grant network write access to the object database.
    #[doc(hidden)]
    fn device_authority_internal(&mut self) -> Option<crate::device::DeviceAuthority<'_>> {
        None
    }

    /// Snapshot of object-owned target Audit overrides. Absent fields inherit
    /// the selected Reporter; source reporting does not inspect remote policies.
    #[doc(hidden)]
    fn audit_object_policy_internal(&self) -> crate::audit::ObjectAuditPolicy {
        crate::audit::ObjectAuditPolicy::default()
    }

    /// Optional sealed built-in policy assignment authority (AV/BV, Color and
    /// Color Temperature). Custom objects
    /// remain on their ordinary writer path; adapters may forward this capability.
    #[doc(hidden)]
    fn audit_policy_authority_internal(
        &mut self,
    ) -> Option<crate::audit::AuditPolicyAuthority<'_>> {
        None
    }

    /// The optional built-in Reporter authority.
    #[doc(hidden)]
    fn audit_reporter_internal(&self) -> Option<&crate::audit::AuditReporterObject> {
        None
    }

    /// Scoped built-in Reporter mutation authority; absent for other objects.
    #[doc(hidden)]
    fn audit_reporter_authority_internal(
        &mut self,
    ) -> Option<crate::audit::AuditReporterAuthority<'_>> {
        None
    }

    /// Atomically replace all trusted local Reporter settings without a network write.
    ///
    /// The default opts out. Supporting implementations must reject invalid
    /// settings before any mutation. `None` selectors remove Monitored_Objects
    /// (catch-all); `Some(vec![])` retains the property and selects no ordinary
    /// targets. An installed target owner prepares required records and lifecycle admission.
    ///
    /// Supply every field together, including the optional paired delay capability.
    /// Active target ownership fixes pair presence; source ownership rejects it.
    /// `None` omits both delay/control properties; a present zero is immediate.
    #[doc(hidden)]
    fn configure_audit_reporter_internal(
        &mut self,
        _level: bacnet_types::enums::AuditLevel,
        _operations: bacnet_types::bitstring::AuditOperationFlags,
        _confirmed: bool,
        _selectors: Option<Vec<bacnet_types::constructed::BACnetObjectSelector>>,
        _priorities: bacnet_types::bitstring::BACnetPriorityFilter,
        _maximum_send_delay: Option<crate::audit::AuditSendDelay>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// The object's identifier (type + instance).
    fn object_identifier(&self) -> ObjectIdentifier;

    /// The object's name.
    fn object_name(&self) -> &str;

    /// Read a property value.
    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error>;

    /// Write a property value.
    ///
    /// Returning `Err` MUST leave the object unchanged. WritePropertyMultiple
    /// retains earlier successful writes and cannot undo a mutation made by the
    /// currently failing write, including for a write-only property.
    ///
    /// Refusing one element of a list value, an implementation may name it:
    /// `Error::Structured` with `ErrorDetail::FirstFailedElementNumber`
    /// holding the element's position, from 1, in the list `value` carries.
    /// The server's AddListElement handler then reports the request element
    /// at that position in its ChangeList-Error (#1048); a plain
    /// `Error::Protocol` there names the first element the list would gain.
    ///
    /// Make the checks that don't depend on the value (the property exists,
    /// it is writable now, the array index is in range) before judging the
    /// value's datatype, and refuse a value of the wrong datatype with
    /// PROPERTY / INVALID_DATA_TYPE. The bundled server reads that refusal of
    /// a NULL, on a property that isn't commandable and has no NULL in its
    /// datatype, as a success that leaves the property as it is (Clauses
    /// 15.9.2 and 19.2.1, #1396), so an access check made after the datatype
    /// check would never be seen for a NULL.
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error>;

    /// Write with explicit actual command provenance. Standalone callers assert
    /// the origin; server boundaries additionally validate ingress or membership.
    /// Decorators must forward this hook without discarding the origin.
    /// The default preserves custom/unaffected object behavior. First-party
    /// source-tracked commands require this hook and deny context-free writing.
    fn write_property_from(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        _origin: &crate::command_source::CommandOrigin,
    ) -> Result<(), Error> {
        self.write_property(property, array_index, value, priority)
    }

    /// Return canonical metadata for this object's effective property rows.
    ///
    /// Migrated implementations return every supported standard row for the
    /// current instance, including `PROPERTY_LIST`. Borrowed rows cover static
    /// or object-owned metadata; owned rows support dynamically assembled
    /// per-instance sets. An empty borrowed default marks an object as unmigrated.
    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        Cow::Borrowed(&[])
    }

    /// List all properties this object supports in the legacy projection.
    ///
    /// For migrated objects this includes Object_Identifier, Object_Name, and
    /// Object_Type but omits Property_List. Reading the BACnet Property_List
    /// property applies the additional wire-level universal-property filter.
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]>;

    /// Bind or remove the database's shared wall-clock reader.
    ///
    /// Objects that do not expose clock-derived state ignore this internal
    /// lifecycle hook.
    #[doc(hidden)]
    fn bind_clock_internal(&mut self, _clock: Option<Arc<dyn ClockReader>>) {}

    /// Advance object-owned operations that use monotonic elapsed time.
    ///
    /// The server calls this internal hook from a dedicated lifecycle task.
    /// `true` means readable state changed and generic COV processing is
    /// required. The default is a source-compatible no-op for downstream
    /// object implementations.
    #[doc(hidden)]
    fn advance_time_internal(&mut self, _elapsed: Duration) -> bool {
        false
    }

    /// Bind the process-local monotonic source used when operations arm.
    #[doc(hidden)]
    fn bind_monotonic_clock_internal(&mut self, _clock: Option<Arc<MonotonicClock>>) {}

    /// Bind the waker an object calls when a write arms a monotonic deadline,
    /// so the server's task looks again sooner than its next routine pass.
    #[doc(hidden)]
    fn bind_deadline_waker_internal(&mut self, _waker: Option<Arc<DeadlineWaker>>) {}

    /// Advance operations to an absolute process-local monotonic instant.
    #[doc(hidden)]
    fn advance_monotonic_time_internal(&mut self, _now: Duration) -> bool {
        false
    }

    /// Return the next absolute process-local operation deadline, if any.
    #[doc(hidden)]
    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        None
    }

    /// Take the finest COV increment an active property subscription to
    /// Tracking_Value asks for, or `None` when no such subscription gives
    /// one (#1510).
    ///
    /// The server's monotonic task passes it before each advance, so an
    /// object whose fade or ramp samples Tracking_Value for COV samples it
    /// at least that finely, still no more often than its sample grid
    /// allows. The value is finite and not negative. Objects without such a
    /// Tracking_Value ignore it.
    #[doc(hidden)]
    fn set_tracking_cov_increment_internal(&mut self, _finest: Option<f64>) {}

    /// Freeze COV-readable state while the object's mutation lock is held.
    #[doc(hidden)]
    fn cov_snapshot_internal(&self) -> Option<Box<dyn BACnetObject>> {
        None
    }

    /// Return how many blink-warn notifications a lighting object has
    /// requested, for objects that model them.
    ///
    /// This is an internal conformance-test channel, not a BACnet property,
    /// host callback, or physical-output claim.
    #[doc(hidden)]
    fn lighting_blink_count_internal(&self) -> u64 {
        0
    }

    /// Whether a property write route can accept `property` for this object.
    ///
    /// PICS generation and runtime dispatch MUST consult this (or
    /// `write_property_from` where source authority is required) rather than a
    /// separate heuristic, so the PICS
    /// writable flags cannot drift from the actual write routes. The default
    /// reproduces the historical PICS heuristic (see
    /// `historical_writable_default`) so unmigrated object types keep their
    /// current PICS output. Object implementations override to mirror their
    /// real write capabilities, including required command origin and ownership.
    ///
    /// Universal read-only properties (`OBJECT_IDENTIFIER`, `OBJECT_TYPE`,
    /// `PROPERTY_LIST`, `STATUS_FLAGS`) are always non-writable and are
    /// excluded by the default; overrides should preserve that invariant.
    ///
    /// Built-in analog, binary, multi-state, Event Enrollment, and Alert
    /// Enrollment objects also deny network writes to `ACKED_TRANSITIONS`.
    /// The property descriptions accompanying ASHRAE 135-2020 Tables 12-2/3/4,
    /// 12-6/8/10, 12-21/22/23, 12-14, and 12-61 explicitly make it read-only;
    /// the table's `R`/`O` classification alone is not a write prohibition.
    /// Clause 13.2.3 assigns acknowledgment-bit maintenance to internal
    /// transition and acknowledgment processing. Those lifecycle hooks and
    /// supported AcknowledgeAlarm paths do not use `write_property`, so this
    /// network restriction does not prevent their internal updates.
    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        let metadata = self.property_metadata();
        if metadata.is_empty() {
            historical_writable_default(self.object_identifier().object_type(), property)
        } else {
            crate::property_metadata::is_writable_in_metadata(metadata.as_ref(), property)
        }
    }

    /// Whether `property` accepts an array index on this object.
    ///
    /// Per Clause 12.1.5.1, only BACnetARRAY (and BACnetARRAY of BACnetLIST)
    /// properties accept an array index; Clause 12.1.5.2 makes ReadRange the
    /// only positional access to a BACnetLIST. The RP/RPM/WP/WPM service
    /// handlers gate the request's array index on this query and reject a
    /// supplied index on a non-array property with PROPERTY /
    /// PROPERTY_IS_NOT_AN_ARRAY (Clause 15.5.1.3, Clause 15.9.1.3).
    ///
    /// The default reproduces the standard's classification (see
    /// `array_property_default`): identifier-stable arrays are admitted
    /// without consulting the object type, the identifiers whose datatype
    /// changes with the object type (ACTION, ALARM_VALUES / FAULT_VALUES,
    /// LIST_OF_OBJECT_PROPERTY_REFERENCES, LOG_DEVICE_OBJECT_PROPERTY,
    /// PRESENT_VALUE) classify by `object_identifier().object_type()`, and
    /// everything else — scalars and BACnetLIST properties — rejects the
    /// index. The built-in objects keep the default, so clients that classify
    /// with [`standard_array_property`] agree with them; object
    /// implementations with vendor array properties override.
    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        array_property_default(self.object_identifier().object_type(), property)
    }

    /// Whether `property` is a BACnetLIST on this object.
    ///
    /// AddListElement and RemoveListElement (Clauses 15.1 and 15.2) edit only
    /// BACnetLIST properties, and ReadRange (Clause 15.8) reads only them. The
    /// server's handlers ask this before decoding any element or selecting any
    /// item, and refuse every other target, including an array element, with
    /// SERVICES / PROPERTY_IS_NOT_A_LIST. Like
    /// [`Self::is_array_property`], the answer follows the property's datatype,
    /// not the shape of a value read: a whole array also reads as a list, and
    /// constructed single values often read as framed bytes.
    ///
    /// The default reproduces the standard's classification (see
    /// `list_property_default`): identifier-stable lists are admitted on every
    /// object type, and the identifiers whose datatype changes with the object
    /// type (ALARM_VALUES / FAULT_VALUES, LIST_OF_OBJECT_PROPERTY_REFERENCES,
    /// PRESENT_VALUE, MEMBER_OF) classify by `object_identifier().object_type()`.
    /// The built-in objects keep the default; object implementations with
    /// vendor list properties override.
    fn is_list_property(&self, property: PropertyIdentifier) -> bool {
        list_property_default(self.object_identifier().object_type(), property)
    }

    /// Whether this object type can be created at runtime via CreateObject.
    ///
    /// Default `false`; override `true` only for types the network factory
    /// (`handle_create_object`) actually constructs, so PICS createability
    /// matches the runtime factory with no separate list to drift.
    fn is_createable(&self) -> bool {
        false
    }

    /// The properties whose whole value a CreateObject initial value may set
    /// on this object although WriteProperty can't change it afterwards.
    ///
    /// Clause 15.3 doesn't limit initial values to the properties
    /// WriteProperty can change, so a device may take some of its read-only
    /// properties at creation. The bundled server hands such an initial value,
    /// written without an array index, to [`Self::initialize_property`]
    /// instead of the write route; every other initial value goes the way a
    /// WriteProperty does. The PICS lists the set for each createable type.
    /// Default empty: no property is set only at creation.
    fn creation_only_properties(&self) -> &'static [PropertyIdentifier] {
        &[]
    }

    /// Apply a CreateObject initial value to one of
    /// [`Self::creation_only_properties`], written whole.
    ///
    /// The server calls this only on the object it is creating, before any
    /// other request can see it. Check the value as a write would, and on
    /// `Err` leave the object unchanged: the server then refuses the request,
    /// naming the initial value, and drops the object. As on the write
    /// route, an initial value that is one application NULL and that this
    /// refuses with PROPERTY / INVALID_DATA_TYPE leaves the property as it
    /// is and counts as applied, so refuse a NULL that way. The default
    /// refuses every property with PROPERTY / WRITE_ACCESS_DENIED, the
    /// answer for a property that can't be initialized at creation (Clause
    /// 15.3.1.3.1).
    ///
    /// The bundled server's CreateObject builds only built-in objects today
    /// (Analog Input and Output, the binary types and the multi-state
    /// types), so on a custom object this hook is reached only by a caller
    /// that builds the object itself.
    fn initialize_property(
        &mut self,
        _property: PropertyIdentifier,
        _value: PropertyValue,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    /// Whether this object type can be deleted at runtime via DeleteObject.
    ///
    /// Default `true`; override `false` on object types that are not
    /// deleteable (e.g. `Device`, `NetworkPort`).
    fn is_deleteable(&self) -> bool {
        true
    }

    /// List the REQUIRED properties for this object type.
    ///
    /// Migrated objects derive this set from canonical `R` and `W` rows,
    /// including Property_List. Unmigrated objects retain the historical four
    /// universal properties. Service-specific consumers may exclude
    /// Property_List where their protocol contract requires it.
    fn required_properties(&self) -> Cow<'static, [PropertyIdentifier]> {
        let metadata = self.property_metadata();
        if !metadata.is_empty() {
            return crate::property_metadata::required_properties_from_metadata(metadata.as_ref());
        }
        static UNIVERSAL: [PropertyIdentifier; 4] = [
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PROPERTY_LIST,
        ];
        Cow::Borrowed(&UNIVERSAL)
    }

    /// Whether this object type supports COV notifications.
    ///
    /// Override to return `true` for object types that can generate COV
    /// notifications (analog, binary, multi-state I/O/V). Default is `false`.
    /// This answer admits SubscribeCOV, the whole-object form, and is the
    /// default for
    /// [`supports_subscribe_cov_property`](Self::supports_subscribe_cov_property).
    fn supports_cov(&self) -> bool {
        false
    }

    /// Whether SubscribeCOVProperty and SubscribeCOVPropertyMultiple may
    /// monitor this object's properties.
    ///
    /// Defaults to [`supports_cov`](Self::supports_cov). An object that has
    /// no whole-object COV criteria, because Clause 13.1's Table 13-1 doesn't
    /// list its type, can still return `true` here while `supports_cov`
    /// stays `false`: its property subscriptions then follow Table 13-1a and
    /// SubscribeCOV is refused. The built-in Averaging object does this.
    /// [`supports_cov_property`](Self::supports_cov_property) still decides
    /// each property.
    fn supports_subscribe_cov_property(&self) -> bool {
        self.supports_cov()
    }

    /// Take pending local target work from a Staging object.
    ///
    /// The default keeps every non-Staging object source-compatible. The
    /// bundled server calls this only while it owns the database mutation
    /// guard, then performs the returned writes after releasing that guard.
    #[doc(hidden)]
    fn take_staging_write_plan_internal(&mut self) -> Option<crate::staging::StagingWritePlan> {
        None
    }

    /// Return the current Staging transition generation, if applicable.
    #[doc(hidden)]
    fn staging_generation_internal(&self) -> Option<u64> {
        None
    }

    /// Commit one guarded Staging plan's success or failure to Reliability.
    ///
    /// Returns whether readable source state changed. Implementations ignore a
    /// stale generation so older work cannot fault a newer transition.
    #[doc(hidden)]
    fn complete_staging_write_plan_internal(&mut self, _generation: u64, _success: bool) -> bool {
        false
    }

    /// Take the writes a Command or Channel object's Present_Value write
    /// queued: a Command's selected list, or a Channel's value for its
    /// members.
    ///
    /// The default keeps every other object source-compatible. The bundled
    /// server takes it under the guard that committed the write, then makes
    /// the writes after releasing that guard.
    #[doc(hidden)]
    fn take_command_run_internal(&mut self) -> Option<crate::command::CommandRun> {
        None
    }

    /// Return the current Command or Channel run generation, if applicable.
    #[doc(hidden)]
    fn command_generation_internal(&self) -> Option<u64> {
        None
    }

    /// Record how command `command` of a Command's running list fared.
    ///
    /// Returns whether the run is still the current one. Implementations
    /// ignore a stale generation, so older work can't mark a newer run.
    #[doc(hidden)]
    fn record_command_write_internal(
        &mut self,
        _generation: u64,
        _command: usize,
        _success: bool,
    ) -> bool {
        false
    }

    /// Keep `datatype`, learned for a Channel's member in another device, for
    /// the distributions after this one, or forget the one kept with `None`:
    /// the member at zero-based `slot` of List_Of_Object_Property_References,
    /// if that slot still holds `reference`. Writing the member, or the whole
    /// list, forgets it too.
    #[doc(hidden)]
    fn remember_member_datatype_internal(
        &mut self,
        _slot: usize,
        _reference: &bacnet_types::constructed::BACnetDeviceObjectPropertyReference,
        _datatype: Option<crate::channel::MemberDatatype>,
    ) {
    }

    /// End the current run, setting a Command's All_Writes_Successful or a
    /// Channel's Write_Status and Reliability. `outcome` is `Ok` when every
    /// write was made and succeeded, otherwise the run's first failure.
    ///
    /// Returns whether readable state changed; a stale generation is ignored.
    #[doc(hidden)]
    fn complete_command_run_internal(
        &mut self,
        _generation: u64,
        _outcome: Result<(), crate::command::WriteFailure>,
    ) -> bool {
        false
    }

    /// Return this object's GetEnrollmentSummary event capability.
    ///
    /// The default opts custom and downstream objects out. Implementations opt
    /// in only when they own an actual configured or implied event algorithm
    /// and shared committed-transition history.
    #[doc(hidden)]
    fn enrollment_summary_capability_internal(&self) -> Option<EnrollmentSummaryCapability> {
        None
    }

    /// Whether a readable property supports property-specific COV.
    ///
    /// The default preserves the existing behavior of every other COV-capable
    /// object family while enforcing the bounded standardized Life Safety
    /// surface for source-compatible custom Point and Zone implementations.
    /// Other types answer
    /// [`supports_subscribe_cov_property`](Self::supports_subscribe_cov_property)
    /// for every property.
    fn supports_cov_property(&self, property: PropertyIdentifier) -> bool {
        match self.object_identifier().object_type() {
            ObjectType::LIFE_SAFETY_POINT | ObjectType::LIFE_SAFETY_ZONE => matches!(
                property,
                PropertyIdentifier::PRESENT_VALUE
                    | PropertyIdentifier::STATUS_FLAGS
                    | PropertyIdentifier::TRACKING_VALUE
                    | PropertyIdentifier::SILENCED
                    | PropertyIdentifier::OPERATION_EXPECTED
            ),
            _ => self.supports_subscribe_cov_property(),
        }
    }

    /// COV increment for this object (objects with a COV_Increment property).
    ///
    /// Returns `Some(increment)` for objects that use COV_Increment filtering
    /// (e.g., AnalogInput, AnalogOutput, AnalogValue, Loop, Staging, and the
    /// Integer, Positive Integer and Large Analog Value types). A notification
    /// fires only when the numeric Present_Value delta reaches the increment.
    /// Property COV inherits this increment only for numeric Present_Value;
    /// other selected properties use their own supplied increment or typed
    /// change reporting.
    ///
    /// The increment is an `f64` so every COV_Increment datatype fits without
    /// rounding: a REAL widens exactly, a Large Analog Value's Double is kept
    /// as is, and an Unsigned increment is exact up to 2^53.
    ///
    /// Returns `None` for objects that notify on any state change (binary, multi-state).
    fn cov_increment(&self) -> Option<f64> {
        None
    }

    /// Properties a whole-object (SubscribeCOV) notification reports after
    /// its leading value and Status_Flags, in report order.
    ///
    /// The leading value is Present_Value, except on Access Point, whose
    /// Table 13-1 row leads with Access_Event; the server chooses it by object
    /// type. The default follows the object type's Table 13-1 row:
    ///
    /// - Access Door: Door_Alarm_State, a trigger.
    /// - Access Point: Access_Event_Tag, Access_Event_Time (a trigger),
    ///   Access_Event_Credential and Access_Event_Authentication_Factor.
    /// - Credential Data Input: Update_Time, a trigger.
    /// - Load Control: Requested_Shed_Level, Start_Time, Shed_Duration and
    ///   Duty_Window, all triggers.
    /// - Loop: Setpoint and Controlled_Variable_Value.
    /// - Pulse Converter: Update_Time.
    /// - Staging: Present_Stage, a trigger.
    ///
    /// A trigger's change sends a notification by itself; any other listed
    /// value only rides along. Every other type reports nothing more. The
    /// server leaves out a listed property the object's Property_List lacks.
    /// Property subscriptions (SubscribeCOVProperty and
    /// SubscribeCOVPropertyMultiple) report their own property instead.
    fn cov_reported_properties(&self) -> &'static [CovReportedProperty] {
        cov_reported_properties_default(self.object_identifier().object_type())
    }

    /// Set the OVERRIDDEN bit in StatusFlags.
    ///
    /// For software-only objects this is always FALSE per spec. Hardware
    /// integrations can override to set TRUE when present_value is overridden
    /// by physical means (e.g., a manual switch on an output).
    fn set_overridden(&mut self, _overridden: bool) {}

    /// Evaluate intrinsic reporting after a present_value change.
    ///
    /// This is the per-write entry point: it seeds (or cancels) a pending
    /// delayed transition and fires immediately only when `Time_Delay == 0`.
    /// It never advances the `Time_Delay` countdown — repeated writes to the
    /// same value do not shorten the delay (ASHRAE 135-2020 Clause 13.3 counts
    /// the delay in seconds, so the countdown advances once per elapsed second
    /// via [`tick_intrinsic_reporting`](Self::tick_intrinsic_reporting)).
    ///
    /// Returns `Some(TransitionOutcome)` whenever a transition is ready to be
    /// committed, or
    /// `None` when none did (no change, delay seeded, or the object does not
    /// support intrinsic reporting). A cleared `Event_Enable` bit sets the
    /// outcome's `distribute` flag to false rather than withholding the
    /// transition. Implementations must leave `Event_State`,
    /// `Acked_Transitions`, event history, and fire-ready detector state
    /// unchanged until [`commit_event_transition_internal`](Self::commit_event_transition_internal)
    /// succeeds. Clause 13.2.2.1.4's transition actions run either way, and
    /// `Event_Enable` disables only external distribution, downstream in the
    /// notification-distribution process (Clause 13.2.5).
    fn evaluate_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        None
    }

    /// Advance the `Time_Delay` countdown for a pending transition.
    ///
    /// Called by the server's one-second intrinsic-reporting task. Fires the
    /// pending transition when its delay elapses this tick, cancels it if the
    /// triggering condition reverted, and returns `Some(TransitionOutcome)`
    /// when a transition is ready. A fire-ready proposal must remain
    /// retryable until the commit hook succeeds. As with
    /// [`evaluate_intrinsic_reporting`](Self::evaluate_intrinsic_reporting),
    /// `Event_Enable` is reported via `distribute`, not by returning `None`.
    /// Objects without a delayed transition return `None`.
    fn tick_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        None
    }

    /// Atomically commit all object-owned state for one event transition.
    ///
    /// The caller supplies an exact state change, its transition coordinate,
    /// the resolved Notification Class `Ack_Required` value, a typed
    /// timestamp, and an optional message. Implementations must validate the
    /// coordinate and source state before changing `Event_State`,
    /// `Acked_Transitions`, `Event_Time_Stamps`, or stored message state, and
    /// must leave every value unchanged on error. A successful
    /// implementation also finalizes the detector's pending and fault-edge
    /// state; a failed commit leaves that state retryable.
    ///
    /// This internal channel is deliberately separate from network property
    /// writes and notification distribution. The default fails closed so an
    /// object participates only after implementing this contract. Custom objects
    /// can own and commit their state directly using the public
    /// [`EventTransitionCommit`] and [`EventTransitionCommitError`] types;
    /// no built-in commit kernel is required. Both server intrinsic paths
    /// require success before distributing a notification.
    #[doc(hidden)]
    fn commit_event_transition_internal(
        &mut self,
        _commit: EventTransitionCommit,
    ) -> Result<(), EventTransitionCommitError> {
        Err(EventTransitionCommitError::Unsupported)
    }

    /// Atomically commit Event Enrollment Reliability and transition state.
    ///
    /// This Event Enrollment-specific channel joins `Reliability` to the
    /// existing atomic `Event_State`, `Acked_Transitions`, and
    /// `Event_Time_Stamps` commit. Implementations must stage every supplied
    /// value before assigning object-owned fields and leave them unchanged on
    /// error. The default fails closed for custom objects that have not adopted
    /// the stronger contract.
    #[doc(hidden)]
    fn commit_event_enrollment_reliability_internal(
        &mut self,
        _commit: EventEnrollmentReliabilityCommit,
    ) -> Result<(), EventTransitionCommitError> {
        Err(EventTransitionCommitError::Unsupported)
    }

    /// Evaluate this object's schedule at `time` on `today` (Clause 12.24.4).
    ///
    /// `calendar_active` answers for a special event whose period references
    /// a Calendar: whether that Calendar is TRUE on `today`. Returns the
    /// writes owed when Present_Value changed, the object has just entered
    /// its Effective_Period, or its references or priority changed since the
    /// last write, with the complete local references, target array indices
    /// included. Otherwise, while some references refused their last write,
    /// returns a [`retry`](ScheduleWrite::retry) of the current value to
    /// those references only (#1436). Only meaningful for Schedule objects;
    /// default returns `None`.
    fn tick_schedule(
        &mut self,
        _today: SpecificDate,
        _time: Time,
        _calendar_active: &dyn Fn(ObjectIdentifier) -> bool,
    ) -> Option<ScheduleWrite> {
        None
    }

    /// Take the writes this schedule owes its targets apart from its
    /// calculation, each once, in order: a NULL to every priority slot a
    /// change of List_Of_Object_Property_References or Priority_For_Writing
    /// left behind (#1088), then the Present_Value owed out of service, for a
    /// client's write (Clause 12.24.14) or a change of those two properties.
    ///
    /// A schedule pass collects them before calling
    /// [`tick_schedule`](Self::tick_schedule), so they reach the targets ahead
    /// of any calculated value; they need no clock. Only meaningful for
    /// Schedule objects; default returns none.
    fn take_owed_schedule_writes(&mut self) -> Vec<ScheduleWrite> {
        Vec::new()
    }

    /// Report how each target took `write`, one of this schedule's writes:
    /// `outcomes` holds one entry per member of `write.references`, in order.
    ///
    /// A Schedule judges the reference half of its Reliability from these
    /// (Clause 12.24.13, #1086). Returns whether a readable property, such as
    /// Reliability, changed. Only meaningful for Schedule objects; default
    /// returns `false`.
    fn complete_schedule_write(
        &mut self,
        _write: &ScheduleWrite,
        _outcomes: &[ScheduleTargetOutcome],
    ) -> bool {
        false
    }

    /// The [`retry`](ScheduleWrite::retry) of the current value to the
    /// references whose last write was refused and that name `target`, an
    /// object just added to the database (#1440), so the refusal can clear
    /// without waiting for the next pass. `None` when no such refusal
    /// stands, or when the pass itself would retry nothing: out of service,
    /// outside Effective_Period, or with a NULL Present_Value. Only
    /// meaningful for Schedule objects; default returns `None`.
    fn retry_refusals_naming(&self, _target: ObjectIdentifier) -> Option<ScheduleWrite> {
        None
    }

    /// Whether this Calendar's Date_List matches `day`: its Present_Value on
    /// that day. `None` for an object that does not evaluate a date list; the
    /// schedule tick then reads its Present_Value instead.
    #[doc(hidden)]
    fn calendar_state_internal(&self, _day: SpecificDate) -> Option<bool> {
        None
    }

    /// Acknowledge an alarm transition. Sets the `transition_bit` flags in acked_transitions.
    /// Returns Ok(()) if the object supports event detection, Err otherwise.
    fn acknowledge_alarm(
        &mut self,
        _transition_bit: EventTransitionBits,
    ) -> Result<(), bacnet_types::error::Error> {
        Err(bacnet_types::error::Error::Protocol {
            class: bacnet_types::enums::ErrorClass::OBJECT.to_raw() as u32,
            code: bacnet_types::enums::ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw()
                as u32,
        })
    }

    /// Correlate an acknowledgment with the latest committed transition.
    #[doc(hidden)]
    fn acknowledge_alarm_correlated_internal(
        &mut self,
        _event_state: EventState,
        _timestamp: &BACnetTimeStamp,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::NO_ALARM_CONFIGURED.to_raw() as u32,
        })
    }

    /// Correlate an acknowledgment and report exact object-owned history.
    ///
    /// The default delegates to the source-compatible coarse hook and returns
    /// no notification context. Custom objects that implement only the coarse
    /// hook therefore retain their accepted-acknowledgment behavior without the
    /// bundled server guessing historical states for an ACK notification.
    #[doc(hidden)]
    fn acknowledge_alarm_correlated_detailed_internal(
        &mut self,
        event_state: EventState,
        timestamp: &BACnetTimeStamp,
    ) -> Result<Option<EventStateChange>, Error> {
        self.acknowledge_alarm_correlated_internal(event_state, timestamp)
            .map(|()| None)
    }

    /// Apply a LifeSafetyOperation atomically to this object.
    ///
    /// Implementations must leave the object unchanged when returning `Err`.
    /// They run synchronously under the object-database write lock and must be
    /// fast, nonblocking, and panic-free. External or irreversible actuation
    /// also needs an application-owned idempotency/replay contract. The default
    /// reports that the object does not support this service. Successful
    /// implementations return an outcome with exact, ordered, deduplicated
    /// committed property changes. `AlreadyApplied` leaves all state unchanged
    /// and reports no deltas. The bundled server uses these changes for COV
    /// after releasing the database lock and dispatching the service response.
    fn apply_life_safety_operation(
        &mut self,
        _operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationOutcome, Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Set the next LifeSafetyOperation expected by trusted local logic.
    ///
    /// This is an application-facing state channel, not a network property
    /// write. The default reports that the object does not support the state.
    fn set_life_safety_operation_expected_internal(
        &mut self,
        _operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Apply an internally-detected `Event_State` transition.
    ///
    /// This is the **internal** lifecycle path for the algorithmically-derived
    /// `Event_State` on objects such as Event Enrollment (ASHRAE 135-2020
    /// Clause 12.12). It is deliberately distinct from the network
    /// [`write_property`](Self::write_property) route: `Event_State` is
    /// read-only over the network, so network writes are rejected while the
    /// server's evaluator reaches the field through this method. Objects
    /// without an algorithmic `Event_State` return
    /// `OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`.
    ///
    /// The **default** returns `Err`, so objects without an algorithmic
    /// `Event_State` opt out. Objects that do model one (e.g.
    /// `EventEnrollmentObject`) override this to store the value verbatim:
    /// the only caller is a trusted internal evaluator that passes a modeled
    /// [`EventState`], mirroring the inherent `set_event_state` builder and
    /// the existing read arm. Network-facing validation — rejecting all
    /// `Event_State` writes — lives in [`write_property`](Self::write_property),
    /// not here. Implementations must leave `Event_State` unchanged when they
    /// return `Err`.
    fn set_event_state_internal(&mut self, _state: EventState) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Snapshot this object's Event Enrollment evaluation state, if it models one.
    ///
    /// This is the read half of the internal channel the server's Event
    /// Enrollment evaluator uses to persist per-enrollment algorithm state
    /// across evaluation cycles: the pending (delayed) transition countdown,
    /// the CHANGE_OF_VALUE detection baseline (the monitored sample at the
    /// latest NORMAL indication, per Clause 13.3.3), and
    /// the monitored value behind the most recent OFFNORMAL transition (Clause
    /// 13.3.2 condition (c)). Like [`set_event_state_internal`](Self::set_event_state_internal)
    /// it deliberately bypasses the network property model: none of the three
    /// slots is a BACnet property, and 135-2020 assigns their initialization
    /// to local matters.
    ///
    /// The default returns `None` — objects without algorithmic event
    /// detection carry no such state, and the evaluator treats `None` as an
    /// empty state it cannot write back (delay honoring and the COV baseline
    /// then stay unavailable, matching this crate's pre-delay behavior).
    fn enrollment_eval_state_internal(&self) -> Option<EventEnrollmentEvalState> {
        None
    }

    /// Store this object's Event Enrollment evaluation state.
    ///
    /// The write half of [`enrollment_eval_state_internal`](Self::enrollment_eval_state_internal).
    /// The only caller is the trusted server evaluator, passing a state it
    /// derived from a prior snapshot plus the current cycle's evaluation.
    /// Implementations enforce the Clause 13.2.2.1 invariant by construction:
    /// transitions are prohibited while `Event_Detection_Enable` is FALSE,
    /// so a write arriving then is refused rather than queued (and the
    /// detection-disable reset has already cleared the fields).
    ///
    /// The **default** returns `Err`, so objects without enrollment evaluation
    /// state opt out and the evaluator's write-back is dropped, never stored
    /// into an object that does not model it.
    fn set_enrollment_eval_state_internal(
        &mut self,
        _state: EventEnrollmentEvalState,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Snapshot the monitored source that owns Event Enrollment private state.
    ///
    /// The outer `Option` indicates whether the object supports this channel;
    /// the inner `Option` is empty before a source has been established.
    /// The server stores source ownership in its object database when an
    /// object implements evaluation state but leaves this channel unsupported.
    fn enrollment_eval_source_internal(&self) -> Option<Option<EventEnrollmentMonitoredSource>> {
        None
    }

    /// Store or clear the monitored source that owns Event Enrollment state.
    fn set_enrollment_eval_source_internal(
        &mut self,
        _source: Option<EventEnrollmentMonitoredSource>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Set or clear one `Acked_Transitions` bit on a received event-state
    /// transition.
    ///
    /// Implements the alarm-acknowledgment half of Clause 13.2.2.1.4's fourth
    /// transition action: pass the transition to alarm-acknowledgment handling.
    /// Under Clause 13.2.3, receiving a transition updates its Acked_Transitions
    /// bit to the inverse of the corresponding Ack_Required bit. The caller (the
    /// server evaluator) resolves `Ack_Required` from the referenced
    /// Notification Class object and passes the outcome as `acknowledged`;
    /// this method performs only the bit maintenance.
    ///
    /// `transition_bit` is the transition direction's flag, as returned by
    /// [`EventTransition::bit_mask`](crate::event::EventTransition::bit_mask). The set half overlaps the
    /// network-reachable [`acknowledge_alarm`](Self::acknowledge_alarm), which
    /// also ORs the bit in per Clause 13.2.3's acknowledgment-indication
    /// paragraph; the clear half has no network route by design (a property
    /// write could fabricate or erase acknowledgments — see the
    /// `write_generic_event_properties!` denial comment).
    ///
    /// The **default** returns `Err`, so objects without an algorithmic
    /// `Acked_Transitions` opt out.
    fn set_acked_transitions_internal(
        &mut self,
        _transition_bit: EventTransitionBits,
        _acknowledged: bool,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Evaluate and, when necessary, mutate this object's `Reliability`.
    ///
    /// Reliability evaluation is object-owned: an implementation decides
    /// whether and how its internal conditions affect `Reliability`, performs
    /// the mutation itself, and returns [`ReliabilityEvaluation::Changed`] only
    /// after that mutation succeeds. [`ReliabilityEvaluation::Unchanged`] means
    /// the object made no mutation. Returning `Err` MUST leave all object state
    /// unchanged.
    ///
    /// The default opts out without changing state, preserving source
    /// compatibility for object implementations that do not own a reliability
    /// evaluation algorithm.
    fn evaluate_reliability_internal(&mut self) -> Result<ReliabilityEvaluation, Error> {
        Ok(ReliabilityEvaluation::Unchanged)
    }

    /// Whether periodic object-owned reliability evaluation is currently
    /// inhibited.
    ///
    /// The default is FALSE so existing and downstream object implementations
    /// remain source-compatible. Objects that elect the optional
    /// Reliability_Evaluation_Inhibit property override this internal
    /// predicate; reporting of an already-applied Reliability transition is
    /// intentionally unaffected.
    #[doc(hidden)]
    fn reliability_evaluation_inhibited_internal(&self) -> bool {
        false
    }

    /// Apply an internally-derived `Reliability` value.
    ///
    /// This is the **internal** reliability-evaluation path, distinct from the
    /// network [`write_property`](Self::write_property) route. Implementations
    /// enforce symmetric ownership: clients may write while `Out_Of_Service`
    /// is TRUE, and internal evaluation may write while it is FALSE. ASHRAE
    /// 135-2020 Clause 3.2 describes reliability evaluation as the object's
    /// assessment of its reliability, producing its Reliability property value.
    ///
    /// The default rejects the operation, so object types without an internal
    /// reliability-evaluation process remain unaffected.
    fn set_reliability_internal(&mut self, _reliability: Reliability) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Apply the logical `Present_Value` supplied by the local application.
    ///
    /// This narrow internal hook is distinct from the network
    /// [`write_property`](Self::write_property) route. Built-in Analog Input,
    /// Binary Input, and Multi-state Input objects opt in. Their implementations
    /// accept application updates only while in service; rejecting them while
    /// `Out_Of_Service` is TRUE is a local ownership policy that protects the
    /// client's simulation value, not a requirement imposed by the Standard.
    ///
    /// Analog, Binary, and Multi-state Value objects opt in unless their
    /// [`PresentValueAccess`](crate::present_value_access::PresentValueAccess)
    /// is `Commandable`. For them, rejecting updates while `Out_Of_Service` is
    /// TRUE is required: their Out_Of_Service clause keeps software local to
    /// the device from changing Present_Value.
    ///
    /// Life Safety Point and Zone opt in with a BACnetLifeSafetyState. Clients
    /// never write their Present_Value, so they take the application's value in
    /// service and out of service alike, and the hook leaves Tracking_Value,
    /// Silenced and Operation_Expected as they are: any latching rule belongs
    /// to the application.
    ///
    /// The default fails closed so commandable and other object families do not
    /// acquire privileged `Present_Value` write authority through this hook.
    fn set_present_value_internal(&mut self, _value: PropertyValue) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// [`set_present_value_internal`](Self::set_present_value_internal) on
    /// behalf of `origin`, the local Device the application runs in.
    ///
    /// A noncommandable Analog, Binary or Multi-state Value that tracks its
    /// Present_Value's source (#1552) publishes `origin` as Value_Source
    /// after the update, as Clause 19.5 asks of a write the local device
    /// makes; plain `set_present_value_internal` leaves it NONE. The server's
    /// `set_present_value_local` calls this when it knows its Device. The
    /// default ignores `origin`, and decorators must forward it.
    fn set_present_value_from_internal(
        &mut self,
        value: PropertyValue,
        _origin: &crate::command_source::CommandOrigin,
    ) -> Result<(), Error> {
        self.set_present_value_internal(value)
    }

    /// Apply the `Tracking_Value` the local application derived.
    ///
    /// Only the built-in Life Safety Point and Zone opt in, with a
    /// BACnetLifeSafetyState; the property stays read-only over the network in
    /// service. While `Out_Of_Service` is TRUE a client's simulated value keeps
    /// being served and this one takes over on the return to service. The
    /// default fails closed with the same error as
    /// [`set_present_value_internal`](Self::set_present_value_internal).
    fn set_tracking_value_internal(&mut self, _value: PropertyValue) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Apply an input the application reports for an access-control object
    /// the server holds (#1132): an Access Point's access event, a
    /// Credential Data Input's read or an Access Door's hardware state, each
    /// a record whose values change together. See
    /// [`AccessControlInput`](crate::access_control::AccessControlInput).
    ///
    /// Only the built-in Access Point, Credential Data Input and Access Door
    /// opt in, each for its own kind of record. While Out_Of_Service is TRUE
    /// the point refuses an event with PROPERTY / WRITE_ACCESS_DENIED, since
    /// it performs no authentication then, and the door and the reader store
    /// the input with their device values put aside, leaving a client's
    /// simulated values served. The default, and an object offered another
    /// kind of record, fails closed with the same error as
    /// [`set_present_value_internal`](Self::set_present_value_internal).
    fn report_access_input_internal(
        &mut self,
        _input: crate::access_control::AccessControlInput,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Apply the Controlled_Variable_Value measured by the local application.
    ///
    /// Only the built-in Loop opts in. Its control algorithm runs in the
    /// application, which feeds the measured value here while it does so; the
    /// property stays read-only over the network. The default fails closed
    /// with the same error as [`set_present_value_internal`](Self::set_present_value_internal).
    fn set_controlled_variable_value_internal(
        &mut self,
        _value: PropertyValue,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Record one sample for an Averaging object.
    ///
    /// Only the built-in Averaging object opts in. The value is the reading
    /// of the referenced property, or `None` when the attempt produced no
    /// value, and the object updates its sample window, statistics and counts
    /// together. The application calls it for the samples it takes, and the
    /// database for the ones the object's own schedule asks for (see
    /// [`take_due_averaging_sample_internal`](Self::take_due_averaging_sample_internal)).
    /// The default fails closed with the same error as
    /// [`set_present_value_internal`](Self::set_present_value_internal).
    fn add_averaging_sample_internal(
        &mut self,
        _sample: Option<PropertyValue>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Claim the Averaging sample the object's schedule has due at the
    /// process-local monotonic instant `now`.
    ///
    /// When one is due the object steps its schedule past it and returns the
    /// property to read; the caller reads it while it still holds exclusive
    /// access and records the outcome with
    /// [`add_averaging_sample_internal`](Self::add_averaging_sample_internal).
    /// An object that answers here reports the due time through
    /// [`next_monotonic_deadline_internal`](Self::next_monotonic_deadline_internal),
    /// so the server's monotonic task wakes for it. The built-in Averaging
    /// object opts in while it holds an Object_Property_Reference and a
    /// monotonic clock is bound; the default has nothing to sample.
    #[doc(hidden)]
    fn take_due_averaging_sample_internal(
        &mut self,
        _now: Duration,
    ) -> Option<BACnetObjectPropertyReference> {
        None
    }

    /// The local property a Pulse Converter counts its input from, its
    /// Input_Reference (Clause 12.23.6): `Some(None)` while it holds none,
    /// and `None` for an object that counts no referenced input, which the
    /// default is.
    ///
    /// The database judges the reference against its own objects
    /// (`ObjectDatabase::check_input_reference`) and reports the verdict
    /// through [`set_input_usable_internal`](Self::set_input_usable_internal).
    #[doc(hidden)]
    fn input_reference_internal(&self) -> Option<Option<&BACnetObjectPropertyReference>> {
        None
    }

    /// Take the database's verdict on the reference
    /// [`input_reference_internal`](Self::input_reference_internal) returned:
    /// `false` when it names a property the object can't count from, which
    /// Clause 12.23.9 reports as a CONFIGURATION_ERROR Reliability. Returns
    /// whether a readable property changed, so a caller owes COV. The
    /// default has no input to judge.
    #[doc(hidden)]
    fn set_input_usable_internal(&mut self, _usable: bool) -> bool {
        false
    }

    /// Take a reading of the property
    /// [`input_reference_internal`](Self::input_reference_internal) names,
    /// or `None` when there is nothing to read. A Pulse Converter counts each
    /// increase over the last reading into Count (Clause 12.23.14), across a
    /// wrap when the reading carries its bound. Returns whether a readable
    /// property changed, so a caller owes COV. The default counts nothing.
    #[doc(hidden)]
    fn take_input_reading_internal(
        &mut self,
        _reading: Option<crate::accumulator::InputReading>,
    ) -> bool {
        false
    }

    /// Borrow this object's Audit Log query storage, if it has any.
    ///
    /// This read-only, type-erased channel lets the bundled server execute an
    /// AuditLogQuery against an object's already-loaded retained records. It
    /// deliberately does not expose persistence, mutation, or object
    /// downcasting. The **default** returns `None`; an Audit Log object that
    /// does not opt in is reported as SERVICES /
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    fn audit_log_storage_internal(&self) -> Option<&dyn AuditLogStorage> {
        None
    }

    /// Instance-owned optional Audit Log forwarding profile, never durable pending work.
    #[doc(hidden)]
    fn audit_log_forwarding_internal(&self) -> Option<Arc<crate::audit::AuditLogForwarding>> {
        None
    }

    /// Set a trusted local Audit Log parent without downcasting or a second object store.
    ///
    /// The default opts out. This is not a network property-write capability;
    /// callers own configuration admission, and the server still validates the route.
    #[doc(hidden)]
    fn set_audit_log_parent_internal(
        &mut self,
        _parent: bacnet_types::constructed::BACnetDeviceObjectReference,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Mutably borrow this object's Audit notification receiver capability.
    ///
    /// The default opts out. Implementations own their persistence transaction
    /// and must leave memory unchanged when the batch cannot be committed.
    fn audit_log_notification_sink_internal(
        &mut self,
    ) -> Option<&mut dyn AuditLogNotificationSink> {
        None
    }

    /// Mutably borrow this object's staged-write capability (#1270).
    ///
    /// An object that saves a written state before serving it opts in, so the
    /// server can stage the write and run the save without the database
    /// guard. The default opts out: the object's writes run as they always
    /// do. See [`crate::durable`].
    #[doc(hidden)]
    fn durable_writes_internal(&mut self) -> Option<&mut dyn DurableWrites> {
        None
    }

    /// Borrow this object's trusted local File configuration capability.
    ///
    /// The default returns `None`. The built-in [`crate::file::FileObject`]
    /// opts in so bindings can configure its pending payload, access method,
    /// and growth limits without general object downcasting or a parallel
    /// state cache.
    #[doc(hidden)]
    fn file_configuration_internal(&self) -> Option<&dyn FileConfiguration> {
        None
    }

    /// Mutably borrow this object's trusted local File configuration capability.
    ///
    /// The default returns `None`; payload mutations remain object-owned and
    /// must preserve the same accounting and metadata behavior as the built-in
    /// File setters.
    #[doc(hidden)]
    fn file_configuration_internal_mut(&mut self) -> Option<&mut dyn FileConfiguration> {
        None
    }

    /// Borrow this object's File storage, if it has any.
    ///
    /// This is the read half of the **internal** channel the server's
    /// AtomicReadFile handler uses to reach file contents. Like
    /// [`set_event_state_internal`](Self::set_event_state_internal) it
    /// bypasses the property model on purpose: Table 12-16 (ASHRAE 135-2020
    /// Clause 12.13) defines no File Data property, so file contents are
    /// reachable only through the Clause 14 File Access Services.
    ///
    /// The **default** returns `None`, so object types without a file opt
    /// out; the server reports `None` on a File-typed object as SERVICES /
    /// FILE_ACCESS_DENIED (Clause 18 covers locked or inaccessible files)
    /// rather than reading it as empty.
    /// Applications backing a File object with their own storage — a disk
    /// file, a firmware partition — implement [`FileStorage`] and return
    /// `Some`.
    fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
        None
    }

    /// Mutably borrow this object's File storage, if it has any.
    ///
    /// The write half of
    /// [`file_storage_internal`](Self::file_storage_internal), used by the
    /// AtomicWriteFile handler after the read-only and access-method gates
    /// have passed. The **default** returns `None`.
    fn file_storage_internal_mut(&mut self) -> Option<&mut dyn FileStorage> {
        None
    }

    /// Return stable identities aligned with this object's resident log records.
    ///
    /// Implementing log objects return identities oldest-to-newest, in the
    /// same order as their public resident-record view and their
    /// [`log_buffer_internal`](Self::log_buffer_internal) records. Sequence
    /// numbers are object-owned metadata and never part of an encoded BACnet
    /// record. Non-log objects return `None`.
    fn log_record_identities_internal(&self) -> Option<Vec<LogRecordIdentity>> {
        None
    }

    /// Borrow this object's Log_Buffer records as ReadRange pages them.
    ///
    /// The built-in Trend Log, Trend Log Multiple and Event Log objects
    /// answer a ReadProperty of Log_Buffer with PROPERTY / READ_ACCESS_DENIED,
    /// since their clauses (12.25.14, 12.30.19, 12.27.13) open the buffer to
    /// ReadRange only; the server's ReadRange handler reads it through this
    /// channel instead, one encoded record per item. The **default** returns
    /// `None`, which leaves ReadRange reading the property through
    /// [`read_property`](Self::read_property).
    fn log_buffer_internal(&self) -> Option<&dyn LogBufferRecords> {
        None
    }

    /// Submit a trend record to an object's log lifecycle.
    ///
    /// `Ok(())` means the operation was accepted, not that an ordinary record
    /// became resident: disabled logs ignore it and zero-capacity logs may only
    /// increment their count. A required status transition can replace the
    /// ordinary record. Timestamp failures return an error without mutation.
    /// Objects without trend insertion return OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    fn add_trend_record(&mut self, _record: BACnetLogRecord) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Submit a Trend Log Multiple record, one value per monitored member, to
    /// an object's log lifecycle.
    ///
    /// The outcomes match [`add_trend_record`](Self::add_trend_record).
    /// Objects without Trend Log Multiple insertion return
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    fn add_trend_multiple_record(&mut self, _record: BACnetLogMultipleRecord) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Submit an Event Log record to an object's log lifecycle.
    ///
    /// The server hands the event notifications this device builds to its
    /// Event Logs through this hook
    /// ([`ObjectDatabase::log_event_notification`](crate::database::ObjectDatabase::log_event_notification)).
    /// The outcomes match [`add_trend_record`](Self::add_trend_record).
    /// Objects without Event Log insertion return
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    fn add_event_log_record(&mut self, _record: BACnetEventLogRecord) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// Look at a log's Start_Time / Stop_Time window against the bound
    /// clock, recording the LOG_DISABLED change when it opened or closed
    /// since the last look while Enable is TRUE.
    ///
    /// The trend poller calls this for every Trend Log, Trend Log Multiple
    /// and Event Log on each pass, so a window that moves while no record
    /// arrives is still logged within one pass. Returns whether a record was
    /// added. The default, for objects without a window, does nothing and
    /// returns `false`.
    #[doc(hidden)]
    fn refresh_log_window_internal(&mut self) -> bool {
        false
    }

    /// Whether this Event Log takes the event notifications the device
    /// receives, through
    /// [`ObjectDatabase::log_received_event_notification`](crate::database::ObjectDatabase::log_received_event_notification).
    /// The default, and an Event Log that hasn't opted in, says no.
    #[doc(hidden)]
    fn logs_received_event_notifications_internal(&self) -> bool {
        false
    }

    /// The counts the last committed BUFFER_READY transition reported, for
    /// the server to carry as its event values (Clause 13.3.7). `None` for an
    /// object that hasn't reported one, or doesn't run that algorithm.
    #[doc(hidden)]
    fn buffer_ready_report_internal(&self) -> Option<BufferReadyReport> {
        None
    }

    /// The local property this object's Event_Algorithm_Inhibit follows, its
    /// Event_Algorithm_Inhibit_Ref (#1329); `None` while it holds none, and
    /// for an object without one, which the default is. The database reads
    /// the property and hands the value over through
    /// [`follow_event_algorithm_inhibit_internal`](Self::follow_event_algorithm_inhibit_internal).
    #[doc(hidden)]
    fn event_algorithm_inhibit_reference_internal(&self) -> Option<BACnetObjectPropertyReference> {
        None
    }

    /// Take the value the database read from the property
    /// [`event_algorithm_inhibit_reference_internal`](Self::event_algorithm_inhibit_reference_internal)
    /// names as Event_Algorithm_Inhibit; returns whether it changed. The
    /// default follows nothing.
    #[doc(hidden)]
    fn follow_event_algorithm_inhibit_internal(&mut self, _inhibit: bool) -> bool {
        false
    }
}
