//! Registration of the Channel object with its members, their execution
//! delays, its control groups and Allow_Group_Delay_Inhibit (#1262). Peers
//! can write all of these too; the keyword arguments set them before the
//! server starts, and `storage_path` keeps what peers write (#2312).
use super::super::*;
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::write_config::FileWriteConfigPersistence;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::error::Error;

#[pymethods]
impl BACnetServer {
    /// Add a Channel object to the server (before starting).
    ///
    /// `channel_number` is the number a WriteGroup names (0 to 65535; above
    /// that raises VALUE_OUT_OF_RANGE). `members` sets
    /// List_Of_Object_Property_References: each element is an
    /// `(object, property)` or `(object, property, array_index)` tuple for a
    /// property in this device, or a mapping of `object_identifier`,
    /// `property_identifier` and the optional `property_array_index` and
    /// `device_identifier`. A member naming this server's Device is kept as
    /// the local reference it stands for, as a network write of the list
    /// keeps it; one naming another Device is written there through the
    /// server's device bindings.
    /// `execution_delay` gives one delay in milliseconds per member (zeros
    /// when omitted), `control_groups` the groups the Channel answers a
    /// WriteGroup for (`[0]`, no group, when omitted), and
    /// `allow_group_delay_inhibit` whether a WriteGroup asking for no delays
    /// skips them.
    ///
    /// A wrong shape or type raises TypeError, an unknown or missing mapping
    /// key or a device that isn't a Device raises ValueError, and an integer
    /// outside its type (unsigned16 for `channel_number`, unsigned32 for the
    /// rest), in a tuple or a mapping alike, raises OverflowError (#1360).
    /// The Channel's setters refuse the rest as a protocol error: a
    /// delay count that differs from the member count or an empty group list
    /// (VALUE_OUT_OF_RANGE), and more than 1024 members or 64 groups
    /// (NO_SPACE_TO_WRITE_PROPERTY). Nothing is registered after any of them.
    ///
    /// With `storage_path` (a `str`), the Channel keeps in that file what a
    /// peer writes to its members, Execution_Delay, Channel_Number,
    /// Control_Groups, Allow_Group_Delay_Inhibit and Description (#2364),
    /// and each saved write wins over its argument at the next start
    /// (#2312). A write that cannot
    /// be saved is refused with DEVICE / OPERATIONAL_PROBLEM.
    /// `clear_written_config` drops what was saved first, so the arguments
    /// apply. A file that doesn't load raises BacnetError.
    #[pyo3(signature = (
        instance,
        name,
        channel_number,
        members=None,
        execution_delay=None,
        control_groups=None,
        *,
        allow_group_delay_inhibit=false,
        storage_path=None,
        clear_written_config=false
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_channel(
        &self,
        instance: u32,
        name: &str,
        channel_number: u16,
        members: Option<Bound<'_, PyAny>>,
        execution_delay: Option<Vec<u32>>,
        control_groups: Option<Vec<u32>>,
        allow_group_delay_inhibit: bool,
        storage_path: Option<&str>,
        clear_written_config: bool,
    ) -> PyResult<()> {
        let members = members
            .map(|members| self.members_from_py(&members, "members"))
            .transpose()?;
        let settings = ChannelSettings {
            members,
            execution_delay,
            control_groups,
            allow_group_delay_inhibit,
            storage_path,
            clear_written_config,
        };
        let obj = channel(instance, name, channel_number, settings).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }
}

/// The optional `add_channel` arguments; `None` keeps the Channel's default.
#[derive(Default)]
struct ChannelSettings<'a> {
    members: Option<Vec<BACnetDeviceObjectPropertyReference>>,
    execution_delay: Option<Vec<u32>>,
    control_groups: Option<Vec<u32>>,
    allow_group_delay_inhibit: bool,
    /// The file a peer's writes are kept in.
    storage_path: Option<&'a str>,
    /// Drop what that file holds first.
    clear_written_config: bool,
}

/// Build a Channel through its validating setters, members before their
/// delays, on its storage if it has one.
fn channel(
    instance: u32,
    name: &str,
    channel_number: u16,
    settings: ChannelSettings<'_>,
) -> Result<ChannelObject, Error> {
    let mut obj = match settings.storage_path {
        Some(path) => {
            let storage = Arc::new(FileWriteConfigPersistence::new(path)?);
            let open =
                || ChannelObject::with_persistence(instance, name, channel_number, storage.clone());
            let mut obj = open()?;
            if settings.clear_written_config {
                obj.clear_written_config()?;
                // A cleared Channel keeps serving the written values, and
                // Channel_Number has no setter: one opened again on the
                // cleared file takes the arguments. Dropping the first
                // waits for its saves.
                drop(obj);
                obj = open()?;
            }
            obj
        }
        None => ChannelObject::new(instance, name, channel_number)?,
    };
    if let Some(members) = settings.members {
        obj.set_members(members)?;
    }
    if let Some(delays) = settings.execution_delay {
        obj.set_execution_delay(delays)?;
    }
    if let Some(groups) = settings.control_groups {
        obj.set_control_groups(groups)?;
    }
    obj.set_allow_group_delay_inhibit(settings.allow_group_delay_inhibit);
    Ok(obj)
}

#[cfg(test)]
#[path = "channel_methods_tests.rs"]
mod tests;
