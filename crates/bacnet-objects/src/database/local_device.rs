//! Which Device a database speaks for, and which device-qualified references
//! stay inside it (#1183, #1184).
//!
//! Many references carry an optional Device member: a Trend Log's
//! Log_DeviceObjectProperty, an Event Enrollment's monitored and setpoint
//! references, a Schedule's targets and more. Without the member, or with one
//! naming this device, the reference points at an object in this database.
//! With one naming any other device it points somewhere this database can't
//! read, even when an object with the same identifier exists here. The object
//! layer (the Trend Log poller) and the server (Event Enrollment evaluation,
//! local reference rewrites) ask that question of the same objects, so the
//! answer lives with the database rather than in each caller.

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::ObjectDatabase;

/// This database's own Device, as a device-qualified reference names it.
///
/// Taken from [`ObjectDatabase::local_device`] under the caller's database
/// guard, so it holds only while that guard does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalDevice(Option<ObjectIdentifier>);

impl LocalDevice {
    /// The Device identifier that names this device. `None` when the
    /// database holds no Device.
    pub fn identifier(self) -> Option<ObjectIdentifier> {
        self.0
    }

    /// Whether a reference whose optional Device member is `device` points
    /// inside this device: it names no device, or it names this one.
    pub fn is_local(self, device: Option<ObjectIdentifier>) -> bool {
        device.is_none_or(|device| Some(device) == self.0)
    }
}

impl ObjectDatabase {
    /// The Device object this database represents, for wildcard reads,
    /// discovery, notifications and reference resolution.
    ///
    /// Clause 12.11 expects one Device per BACnet device. When a database
    /// holds several, the lowest instance wins: a deterministic local policy,
    /// independent of hash or insertion order. No Device yields `None`. The
    /// wildcard instance can't be among them: [`add`](Self::add) refuses it
    /// (#2171).
    pub fn selected_device(&self) -> Option<ObjectIdentifier> {
        select_device(
            self.type_index
                .get(&ObjectType::DEVICE)
                .into_iter()
                .flatten()
                .copied(),
        )
    }

    /// The Device a reference must name to stay inside this device: the
    /// [selected Device](Self::selected_device).
    pub fn local_device(&self) -> LocalDevice {
        LocalDevice(self.selected_device())
    }
}

fn select_device(
    candidates: impl IntoIterator<Item = ObjectIdentifier>,
) -> Option<ObjectIdentifier> {
    candidates
        .into_iter()
        .filter(|candidate| candidate.object_type() == ObjectType::DEVICE)
        .min_by_key(|candidate| candidate.instance_number())
}

#[cfg(test)]
#[path = "local_device_tests.rs"]
mod tests;
