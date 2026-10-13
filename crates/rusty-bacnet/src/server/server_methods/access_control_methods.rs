//! Registration of the access-control objects whose arrays and lists the
//! application configures: Access Door Door_Members, Access Point
//! Access_Doors and Credential Data Input Supported_Formats with
//! Supported_Format_Classes (#1249), Access Zone Entry_Points and
//! Exit_Points (#1306), and Access User Credentials, Members and Member_Of
//! (#1394). Each is read-only over the network, so these keyword arguments
//! are the Python route to it. The Access Point's policy count,
//! supported authorization modes and Priority_For_Writing are read-only
//! too, and take keyword arguments the same way (#1307), as do its
//! Authentication_Policy_List and Authentication_Policy_Names (#1325). The
//! Access Door's Alarm_Values, Fault_Values and Masked_Alarm_Values (#1149),
//! and the Access Zone's Alarm_Values (#1421), are writable over the network
//! as well; their keyword arguments set the starting lists.
use super::super::*;
use bacnet_types::constructed::{BACnetAuthenticationFactorFormat, BACnetAuthenticationPolicy};
use bacnet_types::enums::{
    AccessZoneOccupancyState, AuthenticationFactorType, AuthorizationMode, DoorAlarmState,
};
use bacnet_types::error::Error;
use pyo3::types::PyTuple;

/// One element of a reference array or list (Door_Members, Access_Doors,
/// Entry_Points, Exit_Points and the Access User lists) as Python gives it:
/// an object in this device, or a `(device, object)` pair naming an object
/// in another device.
#[derive(FromPyObject)]
enum PyDeviceObjectReference {
    Local(PyObjectIdentifier),
    Remote(PyObjectIdentifier, PyObjectIdentifier),
}

impl From<PyDeviceObjectReference> for BACnetDeviceObjectReference {
    fn from(reference: PyDeviceObjectReference) -> Self {
        match reference {
            PyDeviceObjectReference::Local(object) => object.to_rust().into(),
            PyDeviceObjectReference::Remote(device, object) => Self {
                device_identifier: Some(device.to_rust()),
                object_identifier: object.to_rust(),
            },
        }
    }
}

/// The references Python gave for the keyword argument `name`, each checked
/// as it converts: a pair whose device isn't a Device object identifier is
/// no reference (#1285), so the shared `check_device` raises ValueError,
/// naming the element, before any setter sees it.
fn device_references(
    references: Option<Vec<PyDeviceObjectReference>>,
    name: &str,
) -> PyResult<Option<Vec<BACnetDeviceObjectReference>>> {
    references
        .map(|references| {
            references
                .into_iter()
                .enumerate()
                .map(|(index, reference)| {
                    let reference = BACnetDeviceObjectReference::from(reference);
                    crate::types::check_device(
                        reference.device_identifier,
                        &format!("{name}[{index}]"),
                    )?;
                    Ok(reference)
                })
                .collect()
        })
        .transpose()
}

/// One Supported_Formats element as Python gives it: a format type number,
/// or a `(format_type, vendor_id, vendor_format)` triple for a format that
/// names its vendor members (a CUSTOM format must name both). Each vendor
/// member is optional in the datatype, so either may be `None`, the form a
/// read gives a format that carries only one of them.
enum PyFactorFormat {
    Standard(u32),
    Vendor(u32, Option<u16>, Option<u16>),
}

impl PyFactorFormat {
    /// Read one format. A number outside its type (unsigned32 for the format
    /// type, unsigned16 for a vendor member) raises OverflowError (#1360),
    /// a triple of another length ValueError, and anything else TypeError.
    fn from_py(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        if value.is_instance_of::<PyTuple>() {
            let (format_type, vendor_id, vendor_format) = value.extract()?;
            return Ok(Self::Vendor(format_type, vendor_id, vendor_format));
        }
        Ok(Self::Standard(value.extract()?))
    }
}

impl From<PyFactorFormat> for BACnetAuthenticationFactorFormat {
    fn from(format: PyFactorFormat) -> Self {
        match format {
            PyFactorFormat::Standard(format_type) => {
                Self::standard(AuthenticationFactorType::from_raw(format_type))
            }
            PyFactorFormat::Vendor(format_type, vendor_id, vendor_format) => Self {
                format_type: AuthenticationFactorType::from_raw(format_type),
                vendor_id,
                vendor_format,
            },
        }
    }
}

#[pymethods]
impl BACnetServer {
    /// Add an Access Door object to the server (before starting).
    ///
    /// `door_members` sets Door_Members, the objects that make up the door.
    /// Each element is an `ObjectIdentifier` in this device or a
    /// `(device, object)` pair of identifiers for one in another device; a
    /// pair whose device isn't a Device raises ValueError.
    ///
    /// `alarm_values`, `fault_values` and `masked_alarm_values` set
    /// Alarm_Values, Fault_Values and Masked_Alarm_Values as
    /// BACnetDoorAlarmState numbers other than NORMAL (1 to 8, or 256 to
    /// 65535). Any other number raises VALUE_OUT_OF_RANGE.
    #[pyo3(signature = (
        instance,
        name,
        *,
        door_members=None,
        alarm_values=None,
        fault_values=None,
        masked_alarm_values=None
    ))]
    fn add_access_door(
        &self,
        instance: u32,
        name: &str,
        door_members: Option<Vec<PyDeviceObjectReference>>,
        alarm_values: Option<Vec<u32>>,
        fault_values: Option<Vec<u32>>,
        masked_alarm_values: Option<Vec<u32>>,
    ) -> PyResult<()> {
        let settings = DoorSettings {
            door_members: device_references(door_members, "door_members")?,
            alarm_values,
            fault_values,
            masked_alarm_values,
        };
        let obj = access_door(instance, name, settings).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Point object to the server (before starting).
    ///
    /// `access_doors` sets Access_Doors, in the same element forms as an
    /// Access Door's `door_members`; a pair whose device isn't a Device
    /// raises ValueError, and a reference to anything but an Access Door
    /// raises VALUE_OUT_OF_RANGE.
    ///
    /// `number_of_authentication_policies` sets the policy count (1 when
    /// omitted, never 0), `supported_authorization_modes` the
    /// BACnetAuthorizationMode numbers the application carries out, which a
    /// write of Authorization_Mode can take (AUTHORIZE alone when omitted;
    /// AUTHORIZE must be among them, and a proprietary mode runs from 64 to
    /// 65535), and `priority_for_writing` the door command priority (16 when
    /// omitted, else 1 to 16). A value outside those raises
    /// VALUE_OUT_OF_RANGE.
    ///
    /// `authentication_policies` sets Authentication_Policy_List and
    /// Authentication_Policy_Names as `(name, policy)` pairs, and the policy
    /// count to their number (#1325). A policy is `(entries, order_enforced,
    /// timeout)`, the form a read of a list element gives: `entries` lists
    /// `(reference, index)` pairs naming the Credential Data Inputs and the
    /// step each serves, from 1, and `timeout` is in seconds, 0 for no limit.
    /// A reference takes the element forms of `door_members`. An empty list,
    /// or more than 256 pairs, raises VALUE_OUT_OF_RANGE. A policy with no
    /// entries, a reference to anything but a Credential Data Input, or
    /// indexes that don't start at 1 and climb by at most one is kept but
    /// can't be in effect: while it is, Active_Authentication_Policy reads 0.
    /// While any such policy is listed, or the active policy is 0,
    /// Reliability reads CONFIGURATION_ERROR. A
    /// `number_of_authentication_policies` given too is applied afterwards,
    /// resizes both arrays and can be at most 256 (VALUE_OUT_OF_RANGE
    /// above).
    #[pyo3(signature = (
        instance,
        name,
        *,
        access_doors=None,
        number_of_authentication_policies=None,
        authentication_policies=None,
        supported_authorization_modes=None,
        priority_for_writing=None
    ))]
    fn add_access_point(
        &self,
        instance: u32,
        name: &str,
        access_doors: Option<Vec<PyDeviceObjectReference>>,
        number_of_authentication_policies: Option<u32>,
        authentication_policies: Option<Bound<'_, PyAny>>,
        supported_authorization_modes: Option<Vec<u32>>,
        priority_for_writing: Option<u8>,
    ) -> PyResult<()> {
        let settings = PointSettings {
            access_doors: device_references(access_doors, "access_doors")?,
            number_of_authentication_policies,
            authentication_policies: authentication_policies
                .map(|policies| {
                    crate::types::authentication_policies_from_py(
                        &policies,
                        "authentication_policies",
                    )
                })
                .transpose()?,
            supported_authorization_modes,
            priority_for_writing,
        };
        let obj = access_point(instance, name, settings).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Zone object to the server (before starting).
    ///
    /// `entry_points` and `exit_points` set Entry_Points and Exit_Points, the
    /// Access Points leading into and out of the zone, in the element forms
    /// an Access Door's `door_members` takes; a pair whose device isn't a
    /// Device raises ValueError, and a reference to anything but an Access
    /// Point raises VALUE_OUT_OF_RANGE.
    ///
    /// `alarm_values` sets Alarm_Values (#1421) as
    /// BACnetAccessZoneOccupancyState numbers other than NORMAL (1 to 6, or
    /// 64 to 65535). Any other number raises VALUE_OUT_OF_RANGE naming the
    /// element.
    #[pyo3(signature = (
        instance,
        name,
        *,
        entry_points=None,
        exit_points=None,
        alarm_values=None
    ))]
    fn add_access_zone(
        &self,
        instance: u32,
        name: &str,
        entry_points: Option<Vec<PyDeviceObjectReference>>,
        exit_points: Option<Vec<PyDeviceObjectReference>>,
        alarm_values: Option<Vec<u32>>,
    ) -> PyResult<()> {
        let settings = ZoneSettings {
            entry_points: device_references(entry_points, "entry_points")?,
            exit_points: device_references(exit_points, "exit_points")?,
            alarm_values,
        };
        let obj = access_zone(instance, name, settings).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access User object to the server (before starting).
    ///
    /// `credentials` sets Credentials, the Access Credential objects the
    /// user holds, and `members` and `member_of` set Members and Member_Of,
    /// the Access Users one level below and above this one, in the element
    /// forms an Access Door's `door_members` takes (#1394). A pair whose
    /// device isn't a Device raises ValueError, and a reference to anything
    /// but an Access Credential in `credentials`, or an Access User in the
    /// other two, raises VALUE_OUT_OF_RANGE.
    #[pyo3(signature = (instance, name, *, credentials=None, members=None, member_of=None))]
    fn add_access_user(
        &self,
        instance: u32,
        name: &str,
        credentials: Option<Vec<PyDeviceObjectReference>>,
        members: Option<Vec<PyDeviceObjectReference>>,
        member_of: Option<Vec<PyDeviceObjectReference>>,
    ) -> PyResult<()> {
        let settings = UserSettings {
            credentials: device_references(credentials, "credentials")?,
            members: device_references(members, "members")?,
            member_of: device_references(member_of, "member_of")?,
        };
        let obj = access_user(instance, name, settings).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Credential Data Input object to the server (before starting).
    ///
    /// `supported_formats` sets Supported_Formats and Supported_Format_Classes
    /// as `(format, format_class)` pairs. A format is a format type number,
    /// or a `(format_type, vendor_id, vendor_format)` triple whose vendor
    /// members may each be `None`, as a read gives them. A format outside
    /// the closed production, a CUSTOM format without its vendor members or
    /// a nonzero vendor member on another format raises VALUE_OUT_OF_RANGE;
    /// a vendor member above 65535 raises OverflowError (#1360).
    #[pyo3(signature = (instance, name, *, supported_formats=None))]
    fn add_credential_data_input(
        &self,
        instance: u32,
        name: &str,
        supported_formats: Option<Vec<(Bound<'_, PyAny>, u32)>>,
    ) -> PyResult<()> {
        let supported_formats = supported_formats
            .map(|formats| {
                formats
                    .iter()
                    .map(|(format, class)| Ok((PyFactorFormat::from_py(format)?, *class)))
                    .collect::<PyResult<Vec<_>>>()
            })
            .transpose()?;
        let obj = credential_data_input(instance, name, supported_formats).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }
}

/// The optional `add_access_door` keyword arguments; `None` keeps the
/// door's default.
#[derive(Default)]
struct DoorSettings {
    door_members: Option<Vec<BACnetDeviceObjectReference>>,
    alarm_values: Option<Vec<u32>>,
    fault_values: Option<Vec<u32>>,
    masked_alarm_values: Option<Vec<u32>>,
}

/// Build an Access Door, applying Door_Members and the alarm lists through
/// its validating setters.
fn access_door(
    instance: u32,
    name: &str,
    settings: DoorSettings,
) -> Result<AccessDoorObject, Error> {
    let mut obj = AccessDoorObject::new(instance, name)?;
    if let Some(members) = settings.door_members {
        obj.set_door_members(members)?;
    }
    let states = |raw: Vec<u32>| raw.into_iter().map(DoorAlarmState::from_raw);
    if let Some(values) = settings.alarm_values {
        obj.set_alarm_values(states(values))?;
    }
    if let Some(values) = settings.fault_values {
        obj.set_fault_values(states(values))?;
    }
    if let Some(values) = settings.masked_alarm_values {
        obj.set_masked_alarm_values(states(values))?;
    }
    Ok(obj)
}

/// The optional `add_access_point` keyword arguments; `None` keeps the
/// point's default.
#[derive(Default)]
struct PointSettings {
    access_doors: Option<Vec<BACnetDeviceObjectReference>>,
    number_of_authentication_policies: Option<u32>,
    authentication_policies: Option<Vec<(String, BACnetAuthenticationPolicy)>>,
    supported_authorization_modes: Option<Vec<u32>>,
    priority_for_writing: Option<u8>,
}

/// Build an Access Point through its validating setters.
fn access_point(
    instance: u32,
    name: &str,
    settings: PointSettings,
) -> Result<AccessPointObject, Error> {
    let mut obj = AccessPointObject::new(instance, name)?;
    if let Some(doors) = settings.access_doors {
        obj.set_access_doors(doors)?;
    }
    if let Some(policies) = settings.authentication_policies {
        obj.set_authentication_policies(policies)?;
    }
    if let Some(count) = settings.number_of_authentication_policies {
        obj.set_number_of_authentication_policies(count)?;
    }
    if let Some(modes) = settings.supported_authorization_modes {
        obj.set_supported_authorization_modes(modes.into_iter().map(AuthorizationMode::from_raw))?;
    }
    if let Some(priority) = settings.priority_for_writing {
        obj.set_priority_for_writing(priority)?;
    }
    Ok(obj)
}

/// The optional `add_access_zone` keyword arguments; `None` keeps the
/// zone's default.
#[derive(Default)]
struct ZoneSettings {
    entry_points: Option<Vec<BACnetDeviceObjectReference>>,
    exit_points: Option<Vec<BACnetDeviceObjectReference>>,
    alarm_values: Option<Vec<u32>>,
}

/// Build an Access Zone, applying Entry_Points, Exit_Points and
/// Alarm_Values through its validating setters.
fn access_zone(
    instance: u32,
    name: &str,
    settings: ZoneSettings,
) -> Result<AccessZoneObject, Error> {
    let mut obj = AccessZoneObject::new(instance, name)?;
    if let Some(entry) = settings.entry_points {
        obj.set_entry_points(entry)?;
    }
    if let Some(exit) = settings.exit_points {
        obj.set_exit_points(exit)?;
    }
    if let Some(values) = settings.alarm_values {
        obj.set_alarm_values(values.into_iter().map(AccessZoneOccupancyState::from_raw))?;
    }
    Ok(obj)
}

/// The optional `add_access_user` keyword arguments; `None` keeps the
/// user's empty list.
#[derive(Default)]
struct UserSettings {
    credentials: Option<Vec<BACnetDeviceObjectReference>>,
    members: Option<Vec<BACnetDeviceObjectReference>>,
    member_of: Option<Vec<BACnetDeviceObjectReference>>,
}

/// Build an Access User, applying its three reference lists through its
/// validating setters.
fn access_user(
    instance: u32,
    name: &str,
    settings: UserSettings,
) -> Result<AccessUserObject, Error> {
    let mut obj = AccessUserObject::new(instance, name)?;
    if let Some(credentials) = settings.credentials {
        obj.set_credentials(credentials)?;
    }
    if let Some(members) = settings.members {
        obj.set_members(members)?;
    }
    if let Some(groups) = settings.member_of {
        obj.set_member_of(groups)?;
    }
    Ok(obj)
}

/// Build a Credential Data Input, applying the formats through its validating
/// setter.
fn credential_data_input(
    instance: u32,
    name: &str,
    formats: Option<Vec<(PyFactorFormat, u32)>>,
) -> Result<CredentialDataInputObject, Error> {
    let mut obj = CredentialDataInputObject::new(instance, name)?;
    if let Some(formats) = formats {
        obj.set_supported_formats(
            formats
                .into_iter()
                .map(|(format, class)| (BACnetAuthenticationFactorFormat::from(format), class))
                .collect::<Vec<_>>(),
        )?;
    }
    Ok(obj)
}

#[cfg(test)]
#[path = "access_control_methods_tests.rs"]
mod tests;
