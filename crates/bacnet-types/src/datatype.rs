//! What a property's value is: its Clause 21 datatype and its shape (#2177).
//!
//! A [`ValueType`] names the datatype of a property's value, or of each
//! element when the property is a BACnetARRAY or a BACnetLIST, and that
//! shape. [`standard_value_type`] gives the one the standard assigns to a
//! property of an object type, so a caller that knows only the type and the
//! property (a client shaping a remote value, a server answering for a
//! missing object) gets the same answer as an object's metadata rows.
//!
//! No allocation: plain `Copy` values and a `const` table, so this builds
//! without `std` or `alloc`.

mod standard;

pub use standard::standard_value_type;

/// The Clause 21 datatype of a value, or of each element of an array or list.
///
/// The application primitives of Clause 20.2, [`Datatype::Any`] for a value
/// of any datatype, and the constructed productions the standard object
/// types use. An enumeration or bit string is named by its encoding
/// ([`Datatype::Enumerated`], [`Datatype::BitString`]), not by which one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Datatype {
    /// An application NULL.
    Null,
    /// BOOLEAN.
    Boolean,
    /// Unsigned Integer.
    Unsigned,
    /// Signed Integer.
    Integer,
    /// REAL.
    Real,
    /// Double.
    Double,
    /// OCTET STRING.
    OctetString,
    /// CharacterString.
    CharacterString,
    /// BIT STRING, including the named-bit types such as BACnetStatusFlags.
    BitString,
    /// ENUMERATED, including every BACnet enumeration.
    Enumerated,
    /// Date.
    Date,
    /// Time.
    Time,
    /// BACnetObjectIdentifier.
    ObjectIdentifier,
    /// Any application datatype: a Schedule's Present_Value and
    /// Schedule_Default (Clause 12.24).
    Any,
    /// BACnetAccessRule.
    AccessRule,
    /// BACnetActionList.
    ActionList,
    /// BACnetAddressBinding.
    AddressBinding,
    /// BACnetAssignedAccessRights.
    AssignedAccessRights,
    /// BACnetAssignedLandingCalls.
    AssignedLandingCalls,
    /// BACnetAuditLogRecord.
    AuditLogRecord,
    /// BACnetAuthenticationFactor.
    AuthenticationFactor,
    /// BACnetAuthenticationFactorFormat.
    AuthenticationFactorFormat,
    /// BACnetAuthenticationPolicy.
    AuthenticationPolicy,
    /// BACnetBDTEntry.
    BdtEntry,
    /// BACnetCalendarEntry.
    CalendarEntry,
    /// BACnetChannelValue.
    ChannelValue,
    /// BACnetClientCOV.
    ClientCov,
    /// BACnetColorCommand.
    ColorCommand,
    /// BACnetCOVMultipleSubscription.
    CovMultipleSubscription,
    /// BACnetCOVSubscription.
    CovSubscription,
    /// BACnetCredentialAuthenticationFactor.
    CredentialAuthenticationFactor,
    /// BACnetDailySchedule.
    DailySchedule,
    /// BACnetDateRange.
    DateRange,
    /// BACnetDateTime.
    DateTime,
    /// BACnetDestination.
    Destination,
    /// BACnetDeviceObjectPropertyReference.
    DeviceObjectPropertyReference,
    /// BACnetDeviceObjectReference.
    DeviceObjectReference,
    /// BACnetEventLogRecord.
    EventLogRecord,
    /// BACnetEventNotificationSubscription.
    EventNotificationSubscription,
    /// BACnetEventParameter.
    EventParameter,
    /// BACnetFaultParameter.
    FaultParameter,
    /// BACnetFDTEntry.
    FdtEntry,
    /// BACnetHostNPort.
    HostNPort,
    /// BACnetLandingCallStatus.
    LandingCallStatus,
    /// BACnetLandingDoorStatus.
    LandingDoorStatus,
    /// BACnetLiftCarCallList.
    LiftCarCallList,
    /// BACnetLightingCommand.
    LightingCommand,
    /// BACnetLogMultipleRecord.
    LogMultipleRecord,
    /// BACnetLogRecord.
    LogRecord,
    /// BACnetNameValue.
    NameValue,
    /// BACnetNameValueCollection.
    NameValueCollection,
    /// BACnetObjectPropertyReference.
    ObjectPropertyReference,
    /// BACnetObjectSelector.
    ObjectSelector,
    /// BACnetOptionalBinaryPV.
    OptionalBinaryPv,
    /// BACnetOptionalCharacterString.
    OptionalCharacterString,
    /// BACnetOptionalPriorityFilter.
    OptionalPriorityFilter,
    /// BACnetOptionalREAL.
    OptionalReal,
    /// BACnetOptionalUnsigned.
    OptionalUnsigned,
    /// BACnetPortPermission.
    PortPermission,
    /// BACnetPrescale.
    Prescale,
    /// BACnetPriorityValue.
    PriorityValue,
    /// BACnetProcessIdSelection.
    ProcessIdSelection,
    /// BACnetPropertyAccessResult.
    PropertyAccessResult,
    /// ReadAccessResult.
    ReadAccessResult,
    /// ReadAccessSpecification.
    ReadAccessSpecification,
    /// BACnetRecipient.
    Recipient,
    /// BACnetRouterEntry.
    RouterEntry,
    /// BACnetScale.
    Scale,
    /// BACnetSetpointReference.
    SetpointReference,
    /// BACnetShedLevel.
    ShedLevel,
    /// BACnetSpecialEvent.
    SpecialEvent,
    /// BACnetStageLimitValue.
    StageLimitValue,
    /// BACnetTimerStateChangeValue.
    TimerStateChangeValue,
    /// BACnetTimeStamp.
    TimeStamp,
    /// BACnetValueSource.
    ValueSource,
    /// BACnetVMACEntry.
    VmacEntry,
    /// BACnetVTSession.
    VtSession,
    /// BACnetxyColor.
    XyColor,
}

impl Datatype {
    /// Whether an application-tagged NULL is a value of this datatype.
    ///
    /// True for NULL itself, for [`Datatype::Any`], and for the CHOICEs with
    /// an untagged NULL member: BACnetPriorityValue, BACnetChannelValue,
    /// BACnetClientCOV, BACnetProcessIdSelection, BACnetObjectSelector,
    /// BACnetTimerStateChangeValue and the BACnetOptional types. A CHOICE
    /// whose NULL member is context-tagged, such as BACnetValueSource, has
    /// none.
    pub const fn admits_null(self) -> bool {
        matches!(
            self,
            Self::Null
                | Self::Any
                | Self::ChannelValue
                | Self::PriorityValue
                | Self::ClientCov
                | Self::ProcessIdSelection
                | Self::ObjectSelector
                | Self::TimerStateChangeValue
                | Self::OptionalBinaryPv
                | Self::OptionalCharacterString
                | Self::OptionalPriorityFilter
                | Self::OptionalReal
                | Self::OptionalUnsigned
        )
    }
}

/// The size of a BACnetARRAY.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ArraySize {
    /// Always this many elements, such as a Priority_Array's 16.
    Fixed(u16),
    /// As many elements as the object holds.
    Variable,
}

/// Whether a property holds one value, a BACnetARRAY or a BACnetLIST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ValueShape {
    /// One value.
    Single,
    /// A BACnetARRAY: index 0 is its size, and each index from 1 an element.
    Array(ArraySize),
    /// A BACnetLIST, which takes no array index (Clause 12.1.5.2).
    List,
}

/// A property's datatype and shape: what its value is, whatever a server
/// does when it is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ValueType {
    /// The value's datatype, or each element's for an array or a list.
    pub datatype: Datatype,
    /// One value, an array or a list.
    pub shape: ValueShape,
}

impl ValueType {
    /// One value of `datatype`.
    pub const fn single(datatype: Datatype) -> Self {
        Self::new(datatype, ValueShape::Single)
    }

    /// A BACnetARRAY of `datatype` with as many elements as the object holds.
    pub const fn array(datatype: Datatype) -> Self {
        Self::new(datatype, ValueShape::Array(ArraySize::Variable))
    }

    /// A BACnetARRAY of `datatype` with exactly `size` elements.
    pub const fn fixed_array(datatype: Datatype, size: u16) -> Self {
        Self::new(datatype, ValueShape::Array(ArraySize::Fixed(size)))
    }

    /// A BACnetLIST of `datatype`.
    pub const fn list(datatype: Datatype) -> Self {
        Self::new(datatype, ValueShape::List)
    }

    const fn new(datatype: Datatype, shape: ValueShape) -> Self {
        Self { datatype, shape }
    }

    /// Whether the property is a BACnetARRAY.
    pub const fn is_array(self) -> bool {
        matches!(self.shape, ValueShape::Array(_))
    }

    /// Whether the property is a BACnetLIST.
    pub const fn is_list(self) -> bool {
        matches!(self.shape, ValueShape::List)
    }

    /// Whether an application NULL is a value of the property at
    /// `array_index`: of its datatype, element by element for an array or
    /// list. Index 0 of an array is its size, an Unsigned, so it never is.
    pub const fn admits_null(self, array_index: Option<u32>) -> bool {
        !matches!(array_index, Some(0)) && self.datatype.admits_null()
    }
}
