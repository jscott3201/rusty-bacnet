//! What the watch hub needs of devices (#2560): a [`Device`]'s points, the
//! instance a device at an address claims, and where a device that
//! stopped answering went.

use std::sync::Weak;

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;

use bacnet_types::error::Error;

use super::follow::Elsewhere;
use super::{Device, DeviceAddress, Pin};
use crate::client::{Client, device_error, failed_locally, lock, unreachable};
use crate::error::OpsError;
use crate::reading::{IntoProperty, one_property};
use crate::watch::{Binding, CovSupport, Point, Watched};

impl<T: TransportPort + 'static> Device<T> {
    /// A point of this device to watch ([`Client::watch`]): `property` in
    /// the grammar of [`PropertyRef`](crate::PropertyRef) (`"ai:1"`, `"bv:3
    /// pv"`, `"ao:2 priority-array[8]"`) or a tuple. A Present_Value is
    /// watched by a subscription to its object (SubscribeCOV), which the
    /// standard objects report it to; any other property, or an element,
    /// by SubscribeCOVProperty.
    ///
    /// A pinned handle's point is at its address
    /// ([`Binding::At`]), and a watch of it follows the device that address
    /// names when the watch starts: when that device answers elsewhere the
    /// watch reports the move ([`Why::Moved`](crate::watch::Why::Moved))
    /// once the device there names itself so to the hub's read (#2812).
    /// A following handle's point follows its instance
    /// ([`Binding::Follows`], #2795): the hub watches it where the handle
    /// reaches the device, and moves the watch, with the handle, once its
    /// own reads name the device at a new address and its old one no
    /// longer answers as it
    /// ([`Update::moved`](crate::watch::Update::moved)). Another device
    /// taking the address is never watched as this one
    /// ([`Why::Replaced`](crate::watch::Why::Replaced)). Sends nothing.
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
            device: self.binding(),
            object: wanted.object,
            watched,
        })
    }

    /// What this handle's points are bound to: its address when pinned,
    /// its instance (on the network it was named on) when following.
    pub fn binding(&self) -> Binding {
        match &self.pin {
            Pin::At(_) => Binding::At(self.address()),
            Pin::Follows {
                instance, network, ..
            } => Binding::Follows {
                instance: *instance,
                network: *network,
            },
        }
    }
}

impl<T: TransportPort + 'static> Device<T> {
    /// What is known of COV at this device, at its address now: of
    /// SubscribeCOV, which an object watch sends, what the device did once
    /// a watch asked, else what its Protocol_Services_Supported lists once
    /// [`describe_device`](Self::describe_device) read it. Sends nothing.
    pub fn cov_support(&self) -> CovSupport {
        self.client.cov_known(&self.address(), &Watched::Object)
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

    /// The instance the device at `address` claims, by one ReadProperty of
    /// the Device object's Object_Identifier by the wildcard instance,
    /// remembered for the address as a handle's
    /// ([`Device::resolve_instance`]) is. A device that answers it with an
    /// error is remembered too, and not asked so again
    /// ([`wildcard_refused`](Client::wildcard_refused)).
    ///
    /// # Errors
    ///
    /// The device's error, or none from a device gone quiet.
    pub(crate) async fn ask_instance(&self, address: &DeviceAddress) -> Result<u32, Error> {
        let mut reader = self.reader(address, None);
        match reader.instance().await.map_err(device_error) {
            Ok(instance) => {
                self.remember(address.clone(), instance);
                Ok(instance)
            }
            Err(error) => {
                if !unreachable(&error) && !failed_locally(&error) {
                    self.refuse_wildcard(address);
                }
                Err(error)
            }
        }
    }

    /// The instance the device at `address` last named itself in an answer
    /// to this client (a read by the wildcard instance, or the watch hub's
    /// question of which device answers): what it said, not what the
    /// discovery table holds. It goes when the table loses its row there or
    /// a device there announces another instance (#2603). Sends nothing.
    pub(crate) fn said(&self, address: &DeviceAddress) -> Option<u32> {
        self.remembered(address)
    }

    /// Note that the device at `address` named itself `instance`.
    pub(crate) fn note_said(&self, address: &DeviceAddress, instance: u32) {
        if self.remembered(address) != Some(instance) {
            self.remember(address.clone(), instance);
        }
    }

    /// Where the device that was at `from`, claiming `instance`, may
    /// answer now, after a request to it went unanswered, as far as a
    /// handle's rule or the discovery table can say: a candidate the watch
    /// hub reads which device answers at before it takes it (#2812), and
    /// moves nothing (#2795). When a handle (or a hub's following point)
    /// follows the instance at `from`, as that handle would after a
    /// timeout: it may send one Who-Is for the instance (at most one every
    /// [`QUIET`](crate::device::QUIET) for the handles following it).
    /// Otherwise by the discovery table alone, sending nothing, and only on
    /// positive evidence: the row at `from` names another instance now, and
    /// one other address claims this one. A row merely gone (aged out of a
    /// quiet device, which a duplicate claimant elsewhere outlives) is no
    /// move, nor are the announcement times [`Device::moved_to`] reports:
    /// a duplicate starting up and one missed request look the same.
    /// `None` while the device may still be at `from`, or two other
    /// addresses claim the instance.
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
            return match self.candidate(key, &followed, from, wait).await {
                Ok(to) => to.filter(|to| to != from),
                Err(_) => None,
            };
        }
        match self.elsewhere(instance, None, from).await {
            // `Moved` with a row at `from`: that row names another instance.
            Elsewhere::Moved(to) if from.row(self.low_level()).await.is_some() => Some(to),
            Elsewhere::Moved(_)
            | Elsewhere::Nowhere
            | Elsewhere::Shared { .. }
            | Elsewhere::Contested(_) => None,
        }
    }
}
