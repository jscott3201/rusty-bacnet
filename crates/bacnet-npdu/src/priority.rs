//! [`NetworkPriority`]: the two priority bits of an NPDU's control octet.

use core::fmt;

/// An NPDU's network priority (Clause 6.2.2): the control octet's two low
/// bits, in increasing order of urgency.
///
/// The field is two bits wide, so the set is closed: every value has a
/// variant. [`from_control`](Self::from_control) reads the bits from a
/// control octet, `try_from` takes a value that must be 0 to 3, and
/// [`to_raw`](Self::to_raw) (or `u8::from`) gives the value back. The order
/// is the numeric one, so [`LifeSafety`](Self::LifeSafety) compares
/// greatest, and a queue that serves the greatest first serves it first.
///
/// ```
/// use bacnet_npdu::NetworkPriority;
///
/// assert_eq!(NetworkPriority::try_from(3), Ok(NetworkPriority::LifeSafety));
/// assert_eq!(NetworkPriority::from_control(0x06), NetworkPriority::CriticalEquipment);
/// assert_eq!(u8::from(NetworkPriority::Urgent), 1);
/// assert!(NetworkPriority::try_from(4).is_err());
/// assert!(NetworkPriority::LifeSafety > NetworkPriority::Normal);
/// ```
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum NetworkPriority {
    /// Normal traffic (`0`), the default.
    #[default]
    Normal = 0,
    /// Urgent traffic (`1`).
    Urgent = 1,
    /// Critical-equipment traffic (`2`).
    CriticalEquipment = 2,
    /// Life-safety traffic (`3`).
    LifeSafety = 3,
}

impl NetworkPriority {
    /// Every priority, in increasing order.
    pub const ALL: [Self; 4] = [
        Self::Normal,
        Self::Urgent,
        Self::CriticalEquipment,
        Self::LifeSafety,
    ];

    /// The priority a control octet carries in its two low bits; the other
    /// bits are ignored.
    #[must_use]
    pub const fn from_control(control: u8) -> Self {
        match control & 0x03 {
            0 => Self::Normal,
            1 => Self::Urgent,
            2 => Self::CriticalEquipment,
            _ => Self::LifeSafety,
        }
    }

    /// The numeric value, 0 to 3: the control octet's two low bits.
    #[must_use]
    pub const fn to_raw(self) -> u8 {
        self as u8
    }
}

/// The numeric value, 0 to 3; anything else is refused.
impl TryFrom<u8> for NetworkPriority {
    type Error = PriorityError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value > 3 {
            return Err(PriorityError { value });
        }
        Ok(Self::from_control(value))
    }
}

/// The numeric value, 0 to 3.
impl From<NetworkPriority> for u8 {
    fn from(priority: NetworkPriority) -> Self {
        priority.to_raw()
    }
}

/// A value [`NetworkPriority::try_from`] refuses: a network priority is two
/// bits, 0 to 3.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub struct PriorityError {
    /// The value offered.
    pub value: u8,
}

impl fmt::Display for PriorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "network priority {} is not 0 to 3", self.value)
    }
}

impl core::error::Error for PriorityError {}
