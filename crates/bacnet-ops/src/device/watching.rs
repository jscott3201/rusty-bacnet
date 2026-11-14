//! What the watch hub needs of devices (#2560): a [`Device`]'s points, the
//! instance a device at an address claims, and where a device that
//! stopped answering went.

use std::sync::Weak;

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;

use super::{Device, DeviceAddress};
use crate::client::{Client, lock};
use crate::error::OpsError;
use crate::reading::{IntoProperty, one_property};
use crate::watch::{Point, Watched};

impl<T: TransportPort + 'static> Device<T> {
    /// A point of this device to watch ([`Client::watch`]): `property` in
    /// the grammar of [`PropertyRef`](crate::PropertyRef) (`"ai:1"`, `"bv:3
    /// pv"`, `"ao:2 priority-array[8]"`) or a tuple. A Present_Value is
    /// watched by a subscription to its object (SubscribeCOV), which the
    /// standard objects report it to; any other property, or an element,
    /// by SubscribeCOVProperty.
    ///
    /// The point is at the device's address now
    /// ([`address`](Self::address)). A following handle that moves later
    /// makes its points at the new address, and a watch of the old one
    /// reports the move ([`Why::Moved`](crate::watch::Why::Moved)). Sends
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`OpsError::Parse`] for text that doesn't name one property, ALL,
    /// REQUIRED and OPTIONAL included.
    pub fn point(&self, property: impl IntoProperty) -> Result<Point, OpsError> {
        let wanted = one_property(property.into_property()?)?;
        let watched = match (wanted.property, wanted.index) {
            (PropertyIdentifier::PRESENT_VALUE, None) => Watched::Object,
            (property, index) => Watched::Property { property, index },
        };
        Ok(Point {
            device: self.address(),
            object: wanted.object,
            watched,
        })
    }
}

impl<T: TransportPort + 'static> Client<T> {
    /// The instance the device at `address` claims: its row's in the
    /// discovery table, or one read earlier by the wildcard instance.
    /// Sends nothing.
    pub(crate) async fn instance_at(&self, address: &DeviceAddress) -> Option<u32> {
        match address.row(self.low_level()).await {
            Some(row) => Some(row.object_identifier.instance_number()),
            None => self.remembered(address),
        }
    }

    /// Where the device that was at `from`, claiming `instance`, answers
    /// now, after a request to it went unanswered. When a handle follows
    /// the instance at `from`, as that handle would after a timeout: it
    /// may send one Who-Is for the instance (at most one every
    /// [`QUIET`](crate::device::QUIET) for the handles following it), and
    /// it moves, telling [`Client::events`]. Otherwise by the discovery
    /// table alone, as [`Device::moved_to`] says, sending nothing. `None`
    /// while the device may still be at `from`, or two other addresses
    /// claim the instance.
    pub(crate) async fn relocated(
        &self,
        instance: u32,
        from: &DeviceAddress,
    ) -> Option<DeviceAddress> {
        for network in [None, Some(from.network())] {
            let key = self.key(network, instance);
            let followed = lock(&self.shared.follows).get(&key).and_then(Weak::upgrade);
            let Some(followed) = followed else {
                continue;
            };
            // A handle following the instance elsewhere may follow another
            // device sharing it: only one here speaks for this device.
            if self.localized(lock(&followed).at.clone()) != *from {
                continue;
            }
            let wait = self.resolve_wait();
            return match self.relocate(key, &followed, from, wait).await {
                Ok(to) => to.filter(|to| to != from),
                Err(_) => None,
            };
        }
        self.elsewhere(instance, None, from)
            .await
            .by_announcements()
    }
}
