//! The local input capability: values the application in the same device
//! pushes into an object, outside any network write.

use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::access_control::AccessControlInput;
use crate::command_source::CommandOrigin;

/// One locally sourced input, named for what it sets.
///
/// The application measures, derives or reads these values itself; none of
/// them is a network write. An object takes the inputs it models through
/// [`LocalInput::apply_local_input`] and refuses the rest with
/// [`not_supported`](super::not_supported). New inputs may be added, so a match on it
/// needs a wildcard arm, and a Present_Value is built with
/// [`present_value`](Self::present_value) or
/// [`present_value_from`](Self::present_value_from).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LocalInputValue {
    /// The logical Present_Value, as an Input's sensor, a noncommandable
    /// Value's or Loop's algorithm or a Life Safety object's application
    /// supplies it.
    #[non_exhaustive]
    PresentValue {
        /// The new Present_Value.
        value: PropertyValue,
        /// The local Device the application runs in, when the host knows it.
        /// A noncommandable Value that tracks its Present_Value's source
        /// (#1552) publishes it as Value_Source, as Clause 19.5 asks of a
        /// write the local device makes; with `None` it publishes none.
        origin: Option<CommandOrigin>,
    },
    /// The Tracking_Value the application derived for a Life Safety Point
    /// or Zone, a BACnetLifeSafetyState.
    TrackingValue(PropertyValue),
    /// The Controlled_Variable_Value the application measured for a Loop,
    /// a finite REAL.
    ControlledVariableValue(PropertyValue),
    /// One sample for an Averaging object: the reading of the property it
    /// averages, or `None` for an attempt that produced no value.
    AveragingSample(Option<PropertyValue>),
    /// A record for an access-control object: an Access Point's access
    /// event, a Credential Data Input's read or an Access Door's hardware
    /// state, whose values change together (#1132).
    AccessControl(AccessControlInput),
    /// Input pulses an Accumulator's meter produced since the last feed,
    /// counted by the application (#2259). They advance Present_Value
    /// through Prescale and count toward Pulse_Rate.
    Pulses(u64),
    /// The Program_State a Program object's program reports, a
    /// BACnetProgramState. The property is read-only on the network
    /// (Clause 12.22.4), so this is the application's only route (#2258).
    ProgramState(PropertyValue),
}

impl LocalInputValue {
    /// A Present_Value with no known origin.
    pub fn present_value(value: PropertyValue) -> Self {
        Self::PresentValue {
            value,
            origin: None,
        }
    }

    /// A Present_Value on behalf of `origin`, the local Device the
    /// application runs in.
    pub fn present_value_from(value: PropertyValue, origin: CommandOrigin) -> Self {
        Self::PresentValue {
            value,
            origin: Some(origin),
        }
    }
}

/// An object that takes values the local application supplies.
///
/// The object returns itself from
/// [`BACnetObject::local_input`](crate::traits::BACnetObject::local_input).
/// Hosts call it with the object borrowed exclusively (the server holds the
/// database's write guard), then run the same COV and event processing a
/// network write gets.
///
/// The built-in objects that take an input:
///
/// - Present_Value: Analog, Binary and Multi-state Inputs, Analog, Binary
///   and Multi-state Values and Integer Values whose Present_Value isn't
///   commandable, the Loop, and the Life Safety Point and Zone;
/// - Tracking_Value: the Life Safety Point and Zone;
/// - Controlled_Variable_Value: the Loop;
/// - a sample: the Averaging object;
/// - an access-control record: the Access Point, Credential Data Input and
///   Access Door, each its own kind of record;
/// - input pulses: the Accumulator;
/// - Program_State: the Program.
pub trait LocalInput {
    /// Apply `input`, or leave the object unchanged and return an error.
    ///
    /// An input the object doesn't model fails with
    /// [`not_supported`](super::not_supported), the error a host also returns for an
    /// object without this capability. The built-in Inputs, Values and Loop
    /// refuse a Present_Value with PROPERTY / WRITE_ACCESS_DENIED while
    /// Out_Of_Service is TRUE, so a client's simulated value stays served;
    /// a value of the wrong datatype or out of range fails as the
    /// property's network write would.
    fn apply_local_input(&mut self, input: LocalInputValue) -> Result<(), Error>;
}
