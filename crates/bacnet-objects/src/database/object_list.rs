//! The local Device's Object_List and Database_Revision, which the database
//! keeps (#2669).
//!
//! An object can't see the database it is in, so a Device object can't list
//! the objects beside it or tell when that set changed. The database holds
//! the one record of its objects, so it keeps both properties itself: every
//! [`add`](ObjectDatabase::add) of a new identifier and every
//! [`remove`](ObjectDatabase::remove) updates the list and moves the
//! revision, whether a peer's CreateObject or DeleteObject or the
//! application made it. The read services (`bacnet-device-core`'s read
//! plan) serve the two properties of the [local
//! Device](ObjectDatabase::local_device) from here
//! ([`keeps_device_property`](ObjectDatabase::keeps_device_property)), so a
//! running server, an endpoint and a firmware responder all answer from it.
//! A Device object read on its own, outside those services, knows only
//! itself: its Object_List names just the Device, and its
//! Database_Revision stays 0.

use bacnet_types::enums::{
    BackupAndRestoreState, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::ObjectDatabase;
use crate::traits::BACnetObject;

impl ObjectDatabase {
    /// Every object the database holds, in Object_List order: the Devices
    /// first (normally just the one) in the order they were added, then
    /// every other object in the order it was added. An object replaced
    /// under its identifier keeps its place; one removed and added again
    /// goes last among its kind. It yields [`len`](Self::len) identifiers.
    ///
    /// The local Device's Object_List (Clause 12.11.16) reads as this,
    /// through every read service. A database built with
    /// [`build_database_with_extra`](crate::device_identity::build_database_with_extra)
    /// lists the Device, its Network Ports, then the extra objects.
    pub fn object_list(&self) -> impl Iterator<Item = ObjectIdentifier> + '_ {
        self.devices().iter().chain(&self.order).copied()
    }

    /// Take every object out, in [`object_list`](Self::object_list) order,
    /// consuming the database. No object is retired: each leaves as it would
    /// if the database were dropped, keeping what it runs and what it holds
    /// in storage. For moving the objects of a database that never served,
    /// such as one a refused server start hands back, into another (#2752).
    pub fn into_objects(mut self) -> alloc::vec::Vec<alloc::boxed::Box<dyn BACnetObject>> {
        let order: alloc::vec::Vec<ObjectIdentifier> = self.object_list().collect();
        order
            .into_iter()
            .filter_map(|oid| self.objects.remove(&oid))
            .collect()
    }

    /// The Devices, which lead Object_List, in the order they were added:
    /// the type index's. Keeping them out of `order` lets a Device join
    /// without an insert ahead of the rest, which would link one more copy
    /// routine into firmware images (`memmove_functions`, #2669).
    fn devices(&self) -> &[ObjectIdentifier] {
        self.type_index
            .get(&ObjectType::DEVICE)
            .map_or(&[], |devices| devices.as_slice())
    }

    /// Database_Revision (Clause 12.11.35): it moves by one each time an
    /// object is added under a new identifier or removed, and each time an
    /// object takes a new name ([`update_name_index`](Self::update_name_index),
    /// or [`add`](Self::add) replacing it under another name), from 0 for a
    /// new database, so a database built with three objects starts at 3. It
    /// wraps past `u32::MAX`. Replacing an object under its identifier and
    /// name doesn't move it, and neither does adding, removing or renaming a
    /// File while the local Device is in a backup or a restore: its
    /// Backup_And_Restore_State isn't IDLE (Clause 12.11.35, the
    /// configuration Files of Clauses 19.1.2.2, 19.1.2.5 and 19.1.3.2).
    pub fn database_revision(&self) -> u32 {
        self.revision
    }

    /// Set [`database_revision`](Self::database_revision), for an
    /// application that keeps the revision across restarts: set the saved
    /// value once the database is built, and save it again whenever it
    /// changes. Later adds and removes count on from it.
    pub fn set_database_revision(&mut self, revision: u32) {
        self.revision = revision;
    }

    /// Whether the read services serve `property` of `object` from the
    /// database rather than the object: Object_List and Database_Revision
    /// of the [local Device](Self::local_device). Read the value with
    /// [`read_device_property`](Self::read_device_property). Any other
    /// property is answered without asking `object` anything. For the read
    /// plan; an application reads with
    /// [`read_object_property`](Self::read_object_property).
    #[doc(hidden)]
    pub fn keeps_device_property(
        &self,
        object: &dyn BACnetObject,
        property: PropertyIdentifier,
    ) -> bool {
        if !matches!(
            property,
            PropertyIdentifier::OBJECT_LIST | PropertyIdentifier::DATABASE_REVISION
        ) {
            return false;
        }
        let oid = object.object_identifier();
        oid.object_type() == ObjectType::DEVICE && self.local_device().identifier() == Some(oid)
    }

    /// Read `property` (at `array_index`) of `object`, an object this
    /// database holds, as a peer reads it: from the database for a property
    /// it keeps ([`keeps_device_property`](Self::keeps_device_property)),
    /// from the object for any other. Whatever follows a reference to a
    /// local property reads through this: the Trend Log poller, the
    /// Averaging sampler, a Pulse Converter's Input_Reference, an
    /// Event_Algorithm_Inhibit_Ref, an Event Enrollment's monitored and
    /// setpoint values, and the server's Channel members and audit records
    /// (#2669).
    // Out of line: one copy for its many callers keeps firmware images
    // smaller.
    #[inline(never)]
    pub fn read_object_property(
        &self,
        object: &dyn BACnetObject,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if self.keeps_device_property(object, property) {
            self.read_device_property(property, array_index)
        } else {
            object.read_property(property, array_index)
        }
    }

    /// The local Device's Object_List or Database_Revision as a read of it
    /// answers. Object_List whole lists [`object_list`](Self::object_list);
    /// index 0 is its length and index `n` its `n`th entry, neither of
    /// which builds the list; past the end is PROPERTY /
    /// INVALID_ARRAY_INDEX. Database_Revision takes no index. Any other
    /// property is PROPERTY / UNKNOWN_PROPERTY.
    // Out of line and cold: the read services call it only for the two
    // properties, and keeping it out of their per-row code keeps a row that
    // reads anything else as cheap as before (#2669).
    #[doc(hidden)]
    #[cold]
    #[inline(never)]
    pub fn read_device_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match (property, array_index) {
            (PropertyIdentifier::OBJECT_LIST, None) => {
                #[cfg(test)]
                tests::count_whole_read();
                Ok(PropertyValue::List(
                    self.object_list()
                        .map(PropertyValue::ObjectIdentifier)
                        .collect(),
                ))
            }
            (PropertyIdentifier::OBJECT_LIST, Some(0)) => {
                Ok(PropertyValue::Unsigned(self.len() as u64))
            }
            (PropertyIdentifier::OBJECT_LIST, Some(index)) => {
                let devices = self.devices();
                (index as usize)
                    .checked_sub(1)
                    .and_then(|at| match at.checked_sub(devices.len()) {
                        None => devices.get(at),
                        Some(rest) => self.order.get(rest),
                    })
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .ok_or_else(|| property_error(ErrorCode::INVALID_ARRAY_INDEX))
            }
            (PropertyIdentifier::DATABASE_REVISION, None) => {
                Ok(PropertyValue::Unsigned(u64::from(self.revision)))
            }
            (PropertyIdentifier::DATABASE_REVISION, Some(_)) => {
                Err(property_error(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY))
            }
            _ => Err(property_error(ErrorCode::UNKNOWN_PROPERTY)),
        }
    }

    /// List `oid`, just added under a new identifier, after the rest; a
    /// Device is listed by the type index already.
    pub(super) fn list_added(&mut self, oid: ObjectIdentifier) {
        if oid.object_type() != ObjectType::DEVICE {
            self.order.push(oid);
        }
        self.revision_moved(&oid);
    }

    /// `oid` took a new name: Clause 12.11.35 counts a rename too.
    pub(super) fn list_renamed(&mut self, oid: &ObjectIdentifier) {
        self.revision_moved(oid);
    }

    /// Take `oid`, just removed, off the list. `retain`, as the type index
    /// does, so a removal links no copy routine of its own into firmware.
    pub(super) fn list_removed(&mut self, oid: &ObjectIdentifier) {
        if oid.object_type() != ObjectType::DEVICE {
            self.order.retain(|listed| listed != oid);
        }
        self.revision_moved(oid);
    }

    /// Move Database_Revision for a change to `oid`, unless `oid` is a File
    /// and the local Device is in a backup or a restore: the configuration
    /// Files those procedures create and delete leave it alone (Clauses
    /// 12.11.35, 19.1.2.2, 19.1.2.5, 19.1.3.2).
    fn revision_moved(&mut self, oid: &ObjectIdentifier) {
        if oid.object_type() == ObjectType::FILE && self.backup_or_restore() {
            return;
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// Whether the local Device's Backup_And_Restore_State is anything but
    /// IDLE. A Device that serves none is never in either procedure.
    fn backup_or_restore(&self) -> bool {
        let idle = BackupAndRestoreState::IDLE.to_raw();
        self.local_device()
            .identifier()
            .and_then(|device| self.get(&device))
            .and_then(|device| {
                device
                    .read_property(PropertyIdentifier::BACKUP_AND_RESTORE_STATE, None)
                    .ok()
            })
            .is_some_and(|state| !matches!(state, PropertyValue::Enumerated(raw) if raw == idle))
    }
}

fn property_error(code: ErrorCode) -> Error {
    Error::protocol(ErrorClass::PROPERTY, code)
}

#[cfg(test)]
#[path = "object_list_tests.rs"]
mod tests;
