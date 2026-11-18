//! Where a planned read row takes its value from.
//!
//! Most rows read the object itself (the host's view of it, when the host
//! serves one). Two kinds of row are built from the database instead,
//! because no object can see it: a Group's whole Present_Value, from its
//! members, and the local Device's Object_List and Database_Revision, which
//! the database keeps as objects come and go (#2669). Each row's planning
//! decides which, so a row reading an ordinary property pays nothing for
//! either when it is read.
use super::group::GroupMembers;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::PropertyIdentifier;

/// Where one planned row's value comes from.
pub enum RowSource {
    /// The object's own answer: the host's, when the object read is a
    /// served view.
    Object,
    /// A Group's whole Present_Value, rebuilt from the members the row
    /// planned.
    Group(GroupMembers),
    /// The local Device's Object_List or Database_Revision, which the
    /// database keeps
    /// ([`ObjectDatabase::read_device_property`]).
    Database,
}

impl RowSource {
    /// [`Database`](Self::Database) for a property the database keeps for
    /// `object` ([`ObjectDatabase::keeps_device_property`]), else
    /// [`Object`](Self::Object).
    // Out of line, so a row that reads Present_Value, which never asks,
    // plans with the same code as before (#2669).
    #[inline(never)]
    pub(crate) fn kept(
        db: &ObjectDatabase,
        object: &dyn BACnetObject,
        property: PropertyIdentifier,
    ) -> Self {
        if db.keeps_device_property(object, property) {
            Self::Database
        } else {
            Self::Object
        }
    }

    /// The members of a Group row.
    pub(crate) fn group(&self) -> Option<&GroupMembers> {
        match self {
            Self::Group(members) => Some(members),
            Self::Object | Self::Database => None,
        }
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
