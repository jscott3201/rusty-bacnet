//! BACnetObject trait — the interface all BACnet objects implement.

#[path = "object_storage.rs"]
mod object_storage;
use crate::prelude::*;
pub(crate) use object_storage::ObjectStorageAccess;

use alloc::borrow::Cow;
use alloc::sync::Arc;
use core::time::Duration;

use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, BACnetObjectPropertyReference,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};

use crate::audit::{AuditLogNotificationSink, AuditLogStorage};

use crate::file::{FileConfiguration, FileStorage};
use crate::log_buffer::{LogBufferRecords, LogRecordIdentity};
use crate::schedule::{ScheduleTargetOutcome, ScheduleWrite};

pub mod capabilities;
#[doc(hidden)]
pub use capabilities::{
    AccessEvents, CommandRunner, EnrollmentEval, ReliabilityEval, StagingSource,
};
pub use capabilities::{
    Clocks, DeadlineWaker, DurableWrites, EventReporting, LifeSafety, LocalInput, LocalInputValue,
    MonotonicClock, Timed, no_alarm_configured, not_supported,
};
pub(crate) use capabilities::{lend_enrollment_eval, lend_event_reporting, lend_reliability_eval};

mod defaults;
use defaults::{array_property_default, cov_reported_properties_default, list_property_default};

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
        Err(not_supported())
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
    /// `Error::protocol` there names the first element the list would gain.
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
    /// The rows are the object's property model: one row for every property
    /// the current instance serves, `PROPERTY_LIST` and the universal
    /// Object_Identifier, Object_Name and Object_Type among them. Each row
    /// carries the property's `R`/`W`/`O` classification, the condition that
    /// makes an optional row present, and the write route the object
    /// implements. They are the only property list an object writes: the
    /// Property_List it serves, its required properties and whether it has
    /// a property all derive from them, through
    /// [`property_list`](crate::property_metadata::property_list),
    /// [`required_properties`](crate::property_metadata::required_properties)
    /// and [`has_property`](crate::property_metadata::has_property). The
    /// default of [`Self::is_writable_property`] derives from them too, and
    /// the bundled server reads them for property presence,
    /// ReadPropertyMultiple's ALL, REQUIRED and OPTIONAL, the indexed-write
    /// gate and the PICS. A property missing from the rows is absent to all
    /// of those, whatever `read_property` answers. Every built-in object
    /// type supplies rows, and a custom object supplies its own. The
    /// smallest honest set is the universal rows plus one for each property
    /// the object serves:
    ///
    /// ```
    /// use std::borrow::Cow;
    /// use bacnet_objects::property_metadata::{
    ///     PropertyConformance as C, PropertyMetadata, PropertyWriteCapability as Wr,
    ///     property_list, required_properties,
    /// };
    /// use bacnet_objects::traits::BACnetObject;
    /// use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
    /// use bacnet_types::error::Error;
    /// use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
    ///
    /// // The universal properties, plus a writable Present_Value.
    /// const ROWS: &[PropertyMetadata] = &[
    ///     PropertyMetadata::new(P::OBJECT_IDENTIFIER, C::RequiredRead, None, Wr::ReadOnly),
    ///     PropertyMetadata::new(P::OBJECT_NAME, C::RequiredRead, None, Wr::ReadOnly),
    ///     PropertyMetadata::new(P::OBJECT_TYPE, C::RequiredRead, None, Wr::ReadOnly),
    ///     PropertyMetadata::new(P::PRESENT_VALUE, C::RequiredWrite, None, Wr::Always),
    ///     PropertyMetadata::new(P::PROPERTY_LIST, C::RequiredRead, None, Wr::ReadOnly),
    /// ];
    ///
    /// struct Setpoint(f32);
    ///
    /// impl BACnetObject for Setpoint {
    ///     fn object_identifier(&self) -> ObjectIdentifier {
    ///         ObjectIdentifier::new(ObjectType::from_raw(130), 1).unwrap()
    ///     }
    ///     fn object_name(&self) -> &str {
    ///         "setpoint"
    ///     }
    ///     fn read_property(&self, p: P, _: Option<u32>) -> Result<PropertyValue, Error> {
    ///         match p {
    ///             P::PRESENT_VALUE => Ok(PropertyValue::Real(self.0)),
    ///             // ... and the universal rows ...
    ///             _ => Err(Error::protocol(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
    ///         }
    ///     }
    ///     fn write_property(
    ///         &mut self,
    ///         p: P,
    ///         _: Option<u32>,
    ///         value: PropertyValue,
    ///         _: Option<u8>,
    ///     ) -> Result<(), Error> {
    ///         match (p, value) {
    ///             (P::PRESENT_VALUE, PropertyValue::Real(v)) => {
    ///                 self.0 = v;
    ///                 Ok(())
    ///             }
    ///             _ => Err(Error::protocol(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
    ///         }
    ///     }
    ///     fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
    ///         Cow::Borrowed(ROWS)
    ///     }
    /// }
    ///
    /// let object = Setpoint(21.0);
    /// assert!(object.is_writable_property(P::PRESENT_VALUE));
    /// assert!(!object.is_writable_property(P::OBJECT_NAME));
    /// let rows = object.property_metadata();
    /// assert_eq!(required_properties(&rows).count(), 5);
    /// // Property_List leaves out the identity properties and itself.
    /// assert_eq!(property_list(&rows).collect::<Vec<_>>(), [P::PRESENT_VALUE]);
    /// ```
    ///
    /// Return borrowed rows for a fixed or object-owned set, and owned rows
    /// for a set assembled per instance, such as one that grows optional rows
    /// with the object's configuration. There is no default: an object that
    /// doesn't say what it serves doesn't build.
    ///
    /// ```compile_fail,E0046
    /// # use std::borrow::Cow;
    /// # use bacnet_objects::traits::BACnetObject;
    /// # use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    /// # use bacnet_types::error::Error;
    /// # use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
    /// struct NoRows;
    ///
    /// impl BACnetObject for NoRows {
    /// #   fn object_identifier(&self) -> ObjectIdentifier {
    /// #       ObjectIdentifier::new(ObjectType::from_raw(130), 1).unwrap()
    /// #   }
    /// #   fn object_name(&self) -> &str { "no-rows" }
    /// #   fn read_property(&self, _: PropertyIdentifier, _: Option<u32>)
    /// #       -> Result<PropertyValue, Error> { unimplemented!() }
    /// #   fn write_property(&mut self, _: PropertyIdentifier, _: Option<u32>,
    /// #       _: PropertyValue, _: Option<u8>) -> Result<(), Error> { unimplemented!() }
    ///     // Every required method but `property_metadata`.
    /// }
    /// ```
    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]>;

    /// Borrow the object's time and clock capability, if it has one.
    ///
    /// An object whose state moves with time (it stamps the wall clock, or
    /// runs timers, fades, ramps or expiries) implements [`Timed`] and
    /// returns `Some(self)`. The default has no time behaviour. See
    /// [`capabilities`] for the pattern.
    fn timed(&mut self) -> Option<&mut dyn Timed> {
        None
    }

    /// Whether a property write route can accept `property` for this object.
    ///
    /// PICS generation and runtime dispatch MUST consult this (or
    /// `write_property_from` where source authority is required) rather than a
    /// separate heuristic, so the PICS
    /// writable flags cannot drift from the actual write routes. The default
    /// answers from the property's [`Self::property_metadata`] row: writable
    /// when its [`PropertyWriteCapability`](crate::property_metadata::PropertyWriteCapability)
    /// names a write route, and not writable for a property without a row.
    /// Keep the rows' write capabilities true to `write_property`, including
    /// required command origin and ownership, rather than overriding this.
    ///
    /// `OBJECT_IDENTIFIER`, `OBJECT_TYPE` and `PROPERTY_LIST` are never
    /// writable over the network; their rows should say `ReadOnly`.
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
        crate::property_metadata::is_writable_in_metadata(
            self.property_metadata().as_ref(),
            property,
        )
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
    /// The bundled server's CreateObject reaches this hook on the types in
    /// [`crate::creation`] and on a custom object whose type the
    /// application registered a constructor for.
    fn initialize_property(
        &mut self,
        _property: PropertyIdentifier,
        _value: PropertyValue,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    /// Whether this object can be deleted at runtime via DeleteObject.
    ///
    /// Default `true`: the bundled server's DeleteObject handler asks this of
    /// the named object and removes it when the answer is `true`, a custom
    /// object included. When it is `false` the request is refused with
    /// OBJECT / OBJECT_DELETION_NOT_PERMITTED and the object stays (#2171).
    /// The PICS reports the answer as the type's deleteability. Override it
    /// to `false` for an object a client must not remove, such as one that
    /// models a fixed part of the device; the built-in Device and Network
    /// Port do.
    fn is_deleteable(&self) -> bool {
        true
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

    /// Mutably borrow a Staging object's capability, which queues target
    /// writes when its stage changes. Not public API yet (#2179).
    ///
    /// The default has none. The bundled server takes a plan under the
    /// database mutation guard and makes its writes after releasing it.
    #[doc(hidden)]
    fn staging_source(&mut self) -> Option<&mut dyn StagingSource> {
        None
    }

    /// Borrow a Command or Channel object's run capability for the
    /// generation check a host makes under a shared guard. Not public API
    /// yet (#2179).
    ///
    /// An object with the capability answers here and from
    /// [`command_runner_mut`](Self::command_runner_mut) alike; the default
    /// has none.
    #[doc(hidden)]
    fn command_runner(&self) -> Option<&dyn CommandRunner> {
        None
    }

    /// Mutably borrow a Command or Channel object's run capability: the
    /// writes its Present_Value write queues. Not public API yet (#2179).
    ///
    /// The default has none. The bundled server takes a run under the guard
    /// that committed the write, then makes the writes after releasing it.
    #[doc(hidden)]
    fn command_runner_mut(&mut self) -> Option<&mut dyn CommandRunner> {
        None
    }

    /// Borrow the object's local input capability, if it has one: the
    /// values the application in the same device supplies, such as an
    /// Input's Present_Value or a Loop's Controlled_Variable_Value.
    ///
    /// The default has none; a host then refuses every input with
    /// [`not_supported`]. See [`LocalInput`] for the built-in
    /// objects that lend it.
    fn local_input(&mut self) -> Option<&mut dyn LocalInput> {
        None
    }

    /// Mutably borrow an Access Point's capability to install a completed
    /// authorization's events as one batch. Not public API yet (#2179).
    ///
    /// The default has none, and the bundled server then refuses the
    /// authorization before evaluating it.
    #[doc(hidden)]
    fn access_events(&mut self) -> Option<&mut dyn AccessEvents> {
        None
    }

    /// Mutably borrow a Life Safety Point's or Zone's capability: the
    /// LifeSafetyOperation service and the operation it expects next.
    ///
    /// The default has none, and the bundled server then refuses both with
    /// [`not_supported`]. See [`LifeSafety`].
    fn life_safety(&mut self) -> Option<&mut dyn LifeSafety> {
        None
    }

    /// Borrow this object's event reporting for the queries a host makes
    /// under a shared database guard: its Event_Algorithm_Inhibit_Ref and
    /// its GetEnrollmentSummary capability, and a BUFFER_READY report's counts.
    ///
    /// The default has none. An object that lends this lends
    /// [`event_reporting_mut`](Self::event_reporting_mut) too. See
    /// [`EventReporting`].
    fn event_reporting(&self) -> Option<&dyn EventReporting> {
        None
    }

    /// Mutably borrow this object's event reporting: intrinsic reporting's
    /// proposals and commits, alarm acknowledgment and
    /// Event_Algorithm_Inhibit following.
    ///
    /// The default has none: the bundled server then proposes no intrinsic
    /// transition for the object, and refuses an AcknowledgeAlarm naming it
    /// with [`no_alarm_configured`]. See [`EventReporting`].
    fn event_reporting_mut(&mut self) -> Option<&mut dyn EventReporting> {
        None
    }

    /// Borrow an Event Enrollment's evaluation capability for the snapshots
    /// a host takes under a shared guard: its private evaluation state and
    /// monitored source. Not public API yet (#2179).
    ///
    /// The default has none, which says the object keeps no evaluation
    /// state. An object that lends this lends
    /// [`enrollment_eval_mut`](Self::enrollment_eval_mut) too.
    #[doc(hidden)]
    fn enrollment_eval(&self) -> Option<&dyn EnrollmentEval> {
        None
    }

    /// Mutably borrow an Event Enrollment's evaluation capability: storing
    /// its evaluation state and monitored source, and committing Reliability
    /// with a transition. Not public API yet (#2179).
    ///
    /// The default has none: the bundled server's evaluator then keeps no
    /// state on the object and its Reliability commits fail closed.
    #[doc(hidden)]
    fn enrollment_eval_mut(&mut self) -> Option<&mut dyn EnrollmentEval> {
        None
    }

    /// Mutably borrow this object's reliability evaluation: its own
    /// judgement of Reliability, the inhibit that pauses it, and the
    /// Reliability its application derives. Not public API yet (#2179).
    ///
    /// The default has none, and the bundled server's fault detector then
    /// leaves the object's Reliability alone.
    #[doc(hidden)]
    fn reliability_eval(&mut self) -> Option<&mut dyn ReliabilityEval> {
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

    /// Claim the Averaging sample the object's schedule has due at the
    /// process-local monotonic instant `now`.
    ///
    /// When one is due the object steps its schedule past it and returns the
    /// property to read; the caller reads it while it still holds exclusive
    /// access and records the outcome as a
    /// [`LocalInputValue::AveragingSample`] through [`Self::local_input`].
    /// An object that answers here reports the due time through
    /// [`Timed::next_monotonic_deadline`],
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
        Err(not_supported())
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
    /// do. The server's monotonic pass also calls
    /// [`DurableWrites::expire_staged`] through it. See [`crate::durable`].
    fn durable_writes(&mut self) -> Option<&mut dyn DurableWrites> {
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
    /// AtomicReadFile handler uses to reach file contents. It bypasses the
    /// property model on purpose: Table 12-16 (ASHRAE 135-2020
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
        Err(not_supported())
    }

    /// Submit a Trend Log Multiple record, one value per monitored member, to
    /// an object's log lifecycle.
    ///
    /// The outcomes match [`add_trend_record`](Self::add_trend_record).
    /// Objects without Trend Log Multiple insertion return
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    fn add_trend_multiple_record(&mut self, _record: BACnetLogMultipleRecord) -> Result<(), Error> {
        Err(not_supported())
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
        Err(not_supported())
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

    /// Whether a log keeps the ordinary records it is given now: Enable is
    /// TRUE and its Start_Time / Stop_Time window admitted the local time at
    /// the last look. A Trend Log logging by COV starts over from the
    /// current value when collection resumes, so the trend poller asks this
    /// before each look (#1480). The default, for objects that aren't logs,
    /// says yes and leaves the gate to the insertion hook.
    #[doc(hidden)]
    fn log_collecting_internal(&self) -> bool {
        true
    }

    /// Whether this Event Log takes the event notifications the device
    /// receives, through
    /// [`ObjectDatabase::log_received_event_notification`](crate::database::ObjectDatabase::log_received_event_notification).
    /// The default, and an Event Log that hasn't opted in, says no.
    #[doc(hidden)]
    fn logs_received_event_notifications_internal(&self) -> bool {
        false
    }
}
