//! Value rules behind the Access Credential (Clause 12.35): its validity
//! window, the disable reasons each source contributes, and the checks on the
//! enumerations and array elements it stores (#1073).

use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{BACnetAssignedAccessRights, BACnetCredentialAuthenticationFactor};
use bacnet_types::enums::{
    AccessAuthenticationFactorDisable, AccessCredentialDisable, AccessCredentialDisableReason,
    AuthenticationFactorType, AuthorizationExemption, ObjectType,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

use crate::clock::ClockFrame;
use crate::common;

/// The vendor range of the extensible access-control enumerations (Clause
/// 21): ASHRAE owns 0 to 63.
const VENDOR: std::ops::RangeInclusive<u32> = 64..=65535;

/// A moment to the hundredth of a second; ordering is chronological.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Instant {
    day: SpecificDate,
    hour: u8,
    minute: u8,
    second: u8,
    hundredths: u8,
}

impl Instant {
    /// The moment `date` and `time` name, or `None` unless the date is a real
    /// day and every time field is specified. The weekday octet is ignored,
    /// as the calendar code does.
    fn new(date: &Date, time: &Time) -> Option<Self> {
        let day = SpecificDate::from_date(date)?;
        time.is_specific().then_some(Self {
            day,
            hour: time.hour,
            minute: time.minute,
            second: time.second,
            hundredths: time.hundredths,
        })
    }
}

/// One end of the credential's validity window: Activation_Time or
/// Expiration_Time, a BACnetDateTime.
///
/// Clauses 12.35.11 and 12.35.12 give a meaning to two forms only: every
/// octet X'FF', which leaves that end open (the start or end of time), and a
/// specific moment. A partly specified value names no single moment to
/// compare against, so it is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WindowLimit {
    date: Date,
    time: Time,
    instant: Option<Instant>,
}

impl WindowLimit {
    /// The open end: every octet X'FF'.
    pub(super) const OPEN: Self = Self {
        date: Date {
            year: Date::UNSPECIFIED,
            month: Date::UNSPECIFIED,
            day: Date::UNSPECIFIED,
            day_of_week: Date::UNSPECIFIED,
        },
        time: Time {
            hour: Time::UNSPECIFIED,
            minute: Time::UNSPECIFIED,
            second: Time::UNSPECIFIED,
            hundredths: Time::UNSPECIFIED,
        },
        instant: None,
    };

    /// Check and keep a limit, refusing a partly specified one with
    /// VALUE_OUT_OF_RANGE. The value is stored as given, weekday included.
    pub(super) fn new(date: Date, time: Time) -> Result<Self, Error> {
        if date.encode() == Self::OPEN.date.encode() && time.encode() == Self::OPEN.time.encode() {
            return Ok(Self::OPEN);
        }
        let instant = Instant::new(&date, &time).ok_or_else(common::value_out_of_range_error)?;
        Ok(Self {
            date,
            time,
            instant: Some(instant),
        })
    }

    /// Decode a written BACnetDateTime: the application-tagged Date and Time
    /// arrive as a two-element list.
    pub(super) fn from_property(value: PropertyValue) -> Result<Self, Error> {
        match value {
            PropertyValue::List(items) => match items.as_slice() {
                [PropertyValue::Date(date), PropertyValue::Time(time)] => Self::new(*date, *time),
                _ => Err(common::invalid_data_type_error()),
            },
            _ => Err(common::invalid_data_type_error()),
        }
    }

    /// The value as read: Date then Time.
    pub(super) fn to_property(self) -> PropertyValue {
        PropertyValue::List(vec![
            PropertyValue::Date(self.date),
            PropertyValue::Time(self.time),
        ])
    }

    /// The stored Date and Time.
    pub(super) fn parts(self) -> (Date, Time) {
        (self.date, self.time)
    }
}

/// The disable reasons the validity window adds at the clock's current
/// moment: DISABLED_NOT_YET_ACTIVE before Activation_Time, DISABLED_EXPIRED
/// after Expiration_Time (Clauses 12.35.11 and 12.35.12).
///
/// Without a clock frame naming a real moment the window can't be judged, so
/// it adds nothing; Credential_Status then rests on the other sources.
pub(super) fn window_reasons(
    frame: Option<ClockFrame>,
    activation: WindowLimit,
    expiration: WindowLimit,
) -> impl Iterator<Item = AccessCredentialDisableReason> {
    let now = frame.and_then(|frame| Instant::new(&frame.local_date, &frame.local_time));
    let not_yet_active =
        matches!((now, activation.instant), (Some(now), Some(start)) if now < start);
    let expired = matches!((now, expiration.instant), (Some(now), Some(end)) if now > end);
    [
        not_yet_active.then_some(AccessCredentialDisableReason::DISABLED_NOT_YET_ACTIVE),
        expired.then_some(AccessCredentialDisableReason::DISABLED_EXPIRED),
    ]
    .into_iter()
    .flatten()
}

/// The disable reason a Credential_Disable value adds (Clause 12.35.13).
/// Which reason a vendor value adds is a local matter; this implementation
/// adds the generic DISABLED.
pub(super) fn credential_disable_reason(
    value: AccessCredentialDisable,
) -> Option<AccessCredentialDisableReason> {
    match value {
        AccessCredentialDisable::NONE => None,
        AccessCredentialDisable::DISABLE_MANUAL => {
            Some(AccessCredentialDisableReason::DISABLED_MANUAL)
        }
        AccessCredentialDisable::DISABLE_LOCKOUT => {
            Some(AccessCredentialDisableReason::DISABLED_LOCKOUT)
        }
        _ => Some(AccessCredentialDisableReason::DISABLED),
    }
}

/// Refuse a Credential_Disable value outside the four named ones and the
/// vendor range with VALUE_OUT_OF_RANGE.
pub(super) fn check_credential_disable(value: AccessCredentialDisable) -> Result<(), Error> {
    let raw = value.to_raw();
    if raw <= AccessCredentialDisable::DISABLE_LOCKOUT.to_raw() || VENDOR.contains(&raw) {
        Ok(())
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// Refuse a disable reason the application may not raise itself with
/// VALUE_OUT_OF_RANGE: anything outside the named reasons and the vendor
/// range, and the two the validity window owns.
pub(super) fn check_local_reason(reason: AccessCredentialDisableReason) -> Result<(), Error> {
    let raw = reason.to_raw();
    let named = raw <= AccessCredentialDisableReason::DISABLED_MANUAL.to_raw();
    let window_owned = matches!(
        reason,
        AccessCredentialDisableReason::DISABLED_NOT_YET_ACTIVE
            | AccessCredentialDisableReason::DISABLED_EXPIRED
    );
    if (named && !window_owned) || VENDOR.contains(&raw) {
        Ok(())
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// Refuse an Authentication_Factors element whose disable value is neither
/// named nor vendor, or whose format type is outside its closed set.
pub(super) fn check_authentication_factor(
    factor: &BACnetCredentialAuthenticationFactor,
) -> Result<(), Error> {
    let disable = factor.disable.to_raw();
    let disable_ok = disable <= AccessAuthenticationFactorDisable::DISABLED_DESTROYED.to_raw()
        || VENDOR.contains(&disable);
    let format = factor.authentication_factor.format_type.to_raw();
    if disable_ok && format <= AuthenticationFactorType::USER_PASSWORD.to_raw() {
        Ok(())
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// The vendor range of BACnetAuthorizationExemption. Unlike the other
/// access-control enumerations it tops out at 255: the comment on its Clause
/// 21 production and Table 23-1 reserve 0 to 63 for ASHRAE and leave 64 to
/// 255 to vendors.
const EXEMPTION_VENDOR: std::ops::RangeInclusive<u32> = 64..=255;

/// Refuse an Authorization_Exemptions value outside the named checks and the
/// vendor range with VALUE_OUT_OF_RANGE.
pub(super) fn check_authorization_exemption(
    exemption: AuthorizationExemption,
) -> Result<(), Error> {
    let raw = exemption.to_raw();
    if raw <= AuthorizationExemption::AUTHORIZATION_DELAY.to_raw()
        || EXEMPTION_VENDOR.contains(&raw)
    {
        Ok(())
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// Refuse an Assigned_Access_Rights element that references anything but an
/// Access Rights object, or names a device with a non-Device identifier.
/// Instance 4194303 marks an unused element (Clause 12.35.18) and is
/// accepted whatever its object type.
pub(super) fn check_assigned_access_rights(
    element: &BACnetAssignedAccessRights,
) -> Result<(), Error> {
    let reference = &element.assigned_access_rights;
    crate::device_reference::check_device_member(reference.device_identifier)?;
    let object = reference.object_identifier;
    if object.object_type() == ObjectType::ACCESS_RIGHTS
        || object.instance_number() == ObjectIdentifier::MAX_INSTANCE
    {
        Ok(())
    } else {
        Err(common::value_out_of_range_error())
    }
}
