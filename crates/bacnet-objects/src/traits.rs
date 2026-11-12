//! BACnetObject trait — the interface all BACnet objects implement.

#[path = "object_storage.rs"]
mod object_storage;
use crate::prelude::*;
pub(crate) use object_storage::ObjectStorageAccess;

use alloc::borrow::Cow;

use bacnet_types::enums::{PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::file::{FileConfiguration, FileStorage};

pub mod capabilities;
#[doc(hidden)]
pub use capabilities::{
    AccessEvents, AuditLog, AuditPolicy, CalendarState, CommandRunner, EnrollmentEval, LogBuffer,
    StagingSource,
};
pub use capabilities::{
    Clocks, CovReportedProperty, CovReporting, DeadlineWaker, DurableWrites, EventReporting,
    LifeSafety, LocalInput, LocalInputValue, MonotonicClock, ReliabilityEval, Retirement,
    ScheduleRunner, Timed, no_alarm_configured, not_supported,
};
pub(crate) use capabilities::{
    impl_audit_policy, impl_log_buffer, lend_audit_policy, lend_cov_reporting,
    lend_enrollment_eval, lend_event_reporting, lend_log_buffer, lend_reliability_eval,
    lend_schedule_runner,
};

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

/// One BACnet object: the surface a custom object implements.
///
/// # What an implementer writes
///
/// Five methods have no default: [`object_identifier`](Self::object_identifier),
/// [`object_name`](Self::object_name), [`read_property`](Self::read_property),
/// [`write_property`](Self::write_property) and
/// [`property_metadata`](Self::property_metadata). The metadata rows are the
/// object's property model: Property_List, the required properties and
/// property presence all derive from them
/// ([`property_list`](crate::property_metadata::property_list),
/// [`required_properties`](crate::property_metadata::required_properties),
/// [`has_property`](crate::property_metadata::has_property)), and so do
/// whether a property is writable, an array or a list
/// ([`is_writable_property`](crate::property_metadata::is_writable_property),
/// [`is_array_property`](crate::property_metadata::is_array_property),
/// [`is_list_property`](crate::property_metadata::is_list_property)). A row
/// states its write route and, where the standard gives none, its value type
/// ([`with_value_type`](crate::property_metadata::PropertyMetadata::with_value_type)).
/// [`write_property_from`](Self::write_property_from)
/// defaults to `write_property`; an object that tracks who commanded it
/// overrides it.
///
/// # Capabilities
///
/// Every other behaviour is a capability: a trait of its own, in
/// [`capabilities`], with an accessor here that defaults to `None`. `None`
/// means the object doesn't have the behaviour, and the host leaves it out:
/// the server's time pass skips an object whose [`timed`](Self::timed) is
/// `None`, and SubscribeCOV refuses one whose [`cov`](Self::cov) is `None`.
/// Each accessor says what its `None` does. To lend a capability, implement
/// its trait and override the accessor to return `Some(self)` (or `Some` of
/// the field that holds the behaviour). Most capability methods have
/// defaults, so an object writes only the parts it uses.
///
/// An accessor that only hands out `&mut` has the capability's plain name.
/// A capability the host also reads under a shared database guard has a
/// `&self` accessor with the plain name and a `_mut` one beside it; an
/// object that lends one lends both. One the host only ever reads has the
/// `&self` accessor alone: [`cov`](Self::cov) and
/// [`calendar_state`](Self::calendar_state). An adapter that wraps another object
/// forwards every accessor to the object inside. The public capabilities are
/// [`Timed`], [`CovReporting`], [`EventReporting`], [`LocalInput`],
/// [`LifeSafety`], [`ReliabilityEval`], [`ScheduleRunner`],
/// [`DurableWrites`] and [`FileStorage`]. The `#[doc(hidden)]` ones serve
/// built-in types and aren't public API yet.
///
/// # What isn't here
///
/// Answers that belong to the object type, not to an instance, aren't trait
/// methods. Whether CreateObject builds a type, and which properties an
/// initial value sets only at creation, come from the constructor table in
/// [`crate::creation`] and its [`Creatable`](crate::creation::Creatable)
/// trait, which an application implements for a type it registers. Whether
/// DeleteObject removes a type comes from
/// [`builtin_deleteable`](crate::creation::builtin_deleteable) and the
/// server's registry.
///
/// # Example
///
/// A custom setpoint that lends one capability, [`CovReporting`], so it
/// takes COV subscriptions and reports changes of half a unit:
///
/// ```
/// use std::borrow::Cow;
/// use bacnet_objects::property_metadata::{
///     PropertyConformance as C, PropertyMetadata, PropertyWriteCapability as Wr,
/// };
/// use bacnet_objects::traits::{BACnetObject, CovReporting};
/// use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
/// use bacnet_types::error::Error;
/// use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
///
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
/// // Every CovReporting method has a default; this one sets the increment.
/// impl CovReporting for Setpoint {
///     fn cov_increment(&self) -> Option<f64> {
///         Some(0.5)
///     }
/// }
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
///     // Lend the capability; every other accessor stays `None`.
///     fn cov(&self) -> Option<&dyn CovReporting> {
///         Some(self)
///     }
/// }
///
/// let mut object = Setpoint(21.0);
/// assert_eq!(object.cov().and_then(|cov| cov.cov_increment()), Some(0.5));
/// assert!(object.timed().is_none());
/// ```
///
/// Stored objects have crate-authorized concrete access; borrowed read views
/// remain valid implementors. Applications cannot manufacture its access permit:
/// ```compile_fail
/// use bacnet_objects::traits::ObjectStorageAccess;
/// let permit = ObjectStorageAccess(());
/// ```
pub trait BACnetObject: Send + Sync + object_storage::StoredObject {
    /// Typed local Device authority for composition-owned configuration.
    /// This does not grant network write access to the object database.
    #[doc(hidden)]
    fn device_authority_internal(&mut self) -> Option<crate::device::DeviceAuthority<'_>> {
        None
    }

    /// Borrow this object's own Audit overrides for the snapshots a host
    /// takes under a shared guard. Not public API yet (#2179).
    ///
    /// The default has none, so every field inherits the selected Reporter.
    /// An object that lends this lends
    /// [`audit_policy_mut`](Self::audit_policy_mut) too. See [`AuditPolicy`].
    #[doc(hidden)]
    fn audit_policy(&self) -> Option<&dyn AuditPolicy> {
        None
    }

    /// Mutably borrow this object's own Audit overrides, for the sealed
    /// pre-commit check of a write that changes them. Not public API yet
    /// (#2179).
    #[doc(hidden)]
    fn audit_policy_mut(&mut self) -> Option<&mut dyn AuditPolicy> {
        None
    }

    /// Borrow the built-in Audit Reporter this object is or wraps. Not
    /// public API yet (#2179).
    ///
    /// The default has none. An adapter around a Reporter hands this and
    /// [`audit_reporter_mut`](Self::audit_reporter_mut) on to it.
    #[doc(hidden)]
    fn audit_reporter(&self) -> Option<&crate::audit::AuditReporterObject> {
        None
    }

    /// The built-in Audit Reporter's scoped mutation authority, which also
    /// replaces its trusted local settings in one step
    /// ([`AuditReporterAuthority::configure`](crate::audit::AuditReporterAuthority::configure)).
    /// Not public API yet (#2179).
    ///
    /// The default has none.
    #[doc(hidden)]
    fn audit_reporter_mut(&mut self) -> Option<crate::audit::AuditReporterAuthority<'_>> {
        None
    }

    /// The object's identifier (type + instance).
    fn object_identifier(&self) -> ObjectIdentifier;

    /// The object's name.
    fn object_name(&self) -> &str;

    /// Read a property value.
    ///
    /// The array index is checked before the object is called on every read
    /// path the bundled server and `bacnet-device-core` run, custom objects
    /// included: an index on a property that isn't an array, as
    /// [`is_array_property`](crate::property_metadata::is_array_property)
    /// answers, is PROPERTY_IS_NOT_AN_ARRAY. A caller that reads the object
    /// directly skips that gate, so the
    /// built-in objects run
    /// [`check_read_array_index`](crate::property_metadata::check_read_array_index)
    /// first themselves; a custom object called directly should do the same
    /// (#2274).
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
    ///
    /// The array index is checked before the object is called on every
    /// service path the bundled server and `bacnet-device-core` run, custom
    /// objects included: an index on a property with no row is
    /// UNKNOWN_PROPERTY, and on a property that isn't an array
    /// PROPERTY_IS_NOT_AN_ARRAY. A caller that writes the object directly
    /// skips that gate, so the built-in objects run
    /// [`check_write_array_index`](crate::property_metadata::check_write_array_index)
    /// first themselves; a custom object called directly should do the same
    /// (#2258).
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
    /// and [`has_property`](crate::property_metadata::has_property), and so
    /// do the writability and the array and list shapes
    /// ([`is_writable_property`](crate::property_metadata::is_writable_property)
    /// and its siblings). The bundled server reads them for property presence,
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
    ///     is_writable_property, property_list, required_properties,
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
    /// assert!(is_writable_property(&object, P::PRESENT_VALUE));
    /// assert!(!is_writable_property(&object, P::OBJECT_NAME));
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
    /// Reliability its application works out.
    ///
    /// The default has none, and the bundled server's fault detector then
    /// leaves the object's Reliability alone. See [`ReliabilityEval`].
    fn reliability_eval(&mut self) -> Option<&mut dyn ReliabilityEval> {
        None
    }

    /// Mutably borrow a Schedule's runner: its evaluation, the writes it
    /// owes its controlled references, how the targets took them, and the
    /// retry it owes an object just added.
    ///
    /// The default has none, and the bundled server's schedule pass then
    /// writes nothing for the object. See [`ScheduleRunner`].
    fn schedule_runner(&mut self) -> Option<&mut dyn ScheduleRunner> {
        None
    }

    /// Borrow a Calendar's evaluation of its Date_List for any day. Not
    /// public API yet (#2179).
    ///
    /// The default has none, and the bundled server's schedule pass then
    /// reads the object's Present_Value for its state.
    #[doc(hidden)]
    fn calendar_state(&self) -> Option<&dyn CalendarState> {
        None
    }

    /// Borrow this object's COV reporting: whether it takes SubscribeCOV
    /// and which properties a property subscription may monitor, its
    /// COV_Increment, and the extra properties its notifications carry.
    ///
    /// The default has none, and the bundled server then refuses every COV
    /// subscription naming the object with OBJECT /
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. See [`CovReporting`].
    fn cov(&self) -> Option<&dyn CovReporting> {
        None
    }

    /// Borrow an Audit Log for the reads the bundled server makes under a
    /// shared database guard: AuditLogQuery and ReadRange over its retained
    /// records, and its forwarding profile. Not public API yet (#2179).
    ///
    /// The default has none: the server answers an AuditLogQuery naming the
    /// object with SERVICES / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. An object
    /// that lends this lends [`audit_log_mut`](Self::audit_log_mut) too. See
    /// [`AuditLog`].
    #[doc(hidden)]
    fn audit_log(&self) -> Option<&dyn AuditLog> {
        None
    }

    /// Mutably borrow an Audit Log, for the trusted local parent a binding
    /// sets and the Audit notifications the server stores. Not public API
    /// yet (#2179).
    #[doc(hidden)]
    fn audit_log_mut(&mut self) -> Option<&mut dyn AuditLog> {
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

    /// Borrow this object's trusted local File configuration, which bindings
    /// use to set the pending payload, access method and growth limits
    /// without downcasting the object. Not public API yet (#2179).
    ///
    /// The default has none. The built-in [`crate::file::FileObject`] lends
    /// it through this and
    /// [`file_configuration_mut`](Self::file_configuration_mut); an object
    /// that lends one lends the other.
    #[doc(hidden)]
    fn file_configuration(&self) -> Option<&dyn FileConfiguration> {
        None
    }

    /// Mutably borrow this object's trusted local File configuration. Its
    /// payload mutations stay object-owned and keep the accounting and
    /// metadata behaviour of the built-in File setters. Not public API yet
    /// (#2179).
    #[doc(hidden)]
    fn file_configuration_mut(&mut self) -> Option<&mut dyn FileConfiguration> {
        None
    }

    /// Borrow this object's File storage, which the bundled server's
    /// AtomicReadFile handler reads under a shared database guard.
    ///
    /// File contents bypass the property model on purpose: Table 12-16
    /// (ASHRAE 135-2020 Clause 12.13) defines no File Data property, so they
    /// are reachable only through the Clause 14 File Access Services.
    ///
    /// The default has none, so an object type without a file opts out; the
    /// server reports a File-typed object that lends none as SERVICES /
    /// FILE_ACCESS_DENIED (Clause 18 covers locked or inaccessible files)
    /// rather than reading it as empty. An application backing a File object
    /// with its own storage, a disk file or a firmware partition, implements
    /// [`FileStorage`] and lends it here and from
    /// [`file_storage_mut`](Self::file_storage_mut).
    fn file_storage(&self) -> Option<&dyn FileStorage> {
        None
    }

    /// Mutably borrow this object's File storage, which the AtomicWriteFile
    /// handler writes after its read-only and access-method gates have
    /// passed. The default has none. See
    /// [`file_storage`](Self::file_storage).
    fn file_storage_mut(&mut self) -> Option<&mut dyn FileStorage> {
        None
    }

    /// Borrow a log's buffer for the ReadRange reads the bundled server makes
    /// under a shared database guard. Not public API yet (#2179).
    ///
    /// The default has none, which leaves ReadRange reading Log_Buffer
    /// through [`read_property`](Self::read_property) with no record
    /// identities to select by. See [`LogBuffer`].
    #[doc(hidden)]
    fn log_buffer(&self) -> Option<&dyn LogBuffer> {
        None
    }

    /// Mutably borrow a log's buffer, for the records the trend poller and
    /// the event logger add and the window the poller looks at. Not public
    /// API yet (#2179).
    ///
    /// The default has none: the trend poller adds nothing to the object,
    /// and the event logger skips it. See [`LogBuffer`].
    #[doc(hidden)]
    fn log_buffer_mut(&mut self) -> Option<&mut dyn LogBuffer> {
        None
    }
}
