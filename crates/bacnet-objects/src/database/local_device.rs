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
/// guard. A bound Device holds for as long as the binding does; the lowest
/// Device of an unbound database only while that guard does.
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
    /// The lowest-instance Device this database holds: what server startup
    /// and endpoint preflight check against their identity, the standalone
    /// PICS reports, a wildcard read with no executor resolves to, and what
    /// [`local_device`](Self::local_device) falls back to while unbound. Once a
    /// server or endpoint has bound its identity's Device, notifications and
    /// reference resolution follow that binding instead (#2227).
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

    /// The Device this database speaks for: the one a reference must name to
    /// stay inside this device, that a notification it builds names as its
    /// initiator, and whose live lists a host serves.
    ///
    /// A running server or endpoint binds it to its frozen identity's Device
    /// (#2227), so a lower-instance Device the application adds later doesn't
    /// take over, and it holds even while that Device object is missing. An
    /// unbound database falls back to its [selected Device](Self::selected_device).
    pub fn local_device(&self) -> LocalDevice {
        LocalDevice(self.bound_device.or_else(|| self.selected_device()))
    }

    /// Bind [`local_device`](Self::local_device) to `device`, or with `None`
    /// release it to the lowest Device. The server and the endpoint call this
    /// at start with their frozen identity's Device; nothing else should.
    #[doc(hidden)]
    pub fn bind_local_device_internal(&mut self, device: Option<ObjectIdentifier>) {
        debug_assert!(device.is_none_or(|device| device.object_type() == ObjectType::DEVICE));
        self.bound_device = device;
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
