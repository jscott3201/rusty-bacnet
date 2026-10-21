//! BACnet_IP_Multicast_Address (Clause 12.56.26, Annex J.8, #941): the
//! B/IP multicast group a port's broadcasts go to, or 0.0.0.0 while it
//! doesn't use one.
//!
//! An IPv4 port has the row when its transport can use multicast (Table
//! 12-71, footnote 7), in any mode, and lends its group as a live view
//! ([`IpMulticast`]). The clause makes the row writable whenever it is
//! present, with a written group waiting for a ReinitializeDevice like the
//! mode's rows (`pending`): Changes_Pending reads TRUE and a read returns
//! the value written, while the port keeps its group until activation.
//! The view checks a written group when it is written, and the row takes
//! writes only while the owner answers ReinitializeDevice and the view
//! can change; otherwise it is listed read-only and refuses them. The
//! B/IP transport's view of a BBMD that serves a group (Annex J.8.3) can't
//! change: its own BDT row and its group go together, and the BBMD rows
//! activate apart from this one (one activation for both is #1963).

use std::sync::Arc;

use bacnet_types::bip_port::IpMulticast;
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::NetworkPortObject;
use crate::common;

/// A port's lent group and the value written to it, if any.
#[derive(Clone, Default)]
pub(super) struct Multicast {
    /// The transport's live view, when it can use multicast.
    view: Option<Arc<dyn IpMulticast>>,
    /// A written group, waiting for activation.
    pending: Option<[u8; 4]>,
}

impl Multicast {
    /// A port whose transport lends `view`, nothing written.
    pub(super) fn lent(view: Option<Arc<dyn IpMulticast>>) -> Self {
        Self {
            view,
            pending: None,
        }
    }

    /// Whether a written group waits for activation.
    pub(super) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Drop the written group: the mode or owner changed.
    pub(super) fn clear_pending(&mut self) {
        self.pending = None;
    }
}

impl NetworkPortObject {
    /// Whether this port serves BACnet_IP_Multicast_Address: an IPv4 port
    /// whose transport lends its group.
    pub(super) fn serves_multicast(&self) -> bool {
        self.multicast.view.is_some()
            && self
                .ip
                .as_ref()
                .is_some_and(super::ip_config::IpConfig::is_ipv4)
    }

    /// Whether a written group could take effect, so the row is writable.
    pub(super) fn activates_multicast(&self) -> bool {
        self.activation
            && self
                .multicast
                .view
                .as_ref()
                .is_some_and(|view| view.can_change())
    }

    /// Read BACnet_IP_Multicast_Address: the written group while one
    /// waits, else the group in use. `None` for another property.
    pub(super) fn read_multicast(&self, property: P) -> Option<Result<PropertyValue, Error>> {
        if property != P::BACNET_IP_MULTICAST_ADDRESS {
            return None;
        }
        let view = self
            .multicast
            .view
            .as_ref()
            .filter(|_| self.serves_multicast());
        Some(match view {
            Some(view) => {
                let group = self.multicast.pending.unwrap_or_else(|| view.group());
                Ok(PropertyValue::OctetString(group.to_vec()))
            }
            None => Err(common::unknown_property_error()),
        })
    }

    /// Take a write to BACnet_IP_Multicast_Address: four octets, a class D
    /// address or 0.0.0.0, as the transport checks it. `None` for another
    /// property, or a port without the row.
    pub(super) fn write_multicast(
        &mut self,
        property: P,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if property != P::BACNET_IP_MULTICAST_ADDRESS || !self.serves_multicast() {
            return None;
        }
        if !self.activates_multicast() {
            return Some(Err(common::write_access_denied_error()));
        }
        if array_index.is_some() {
            return Some(Err(common::property_is_not_an_array_error()));
        }
        let PropertyValue::OctetString(octets) = value else {
            return Some(Err(common::invalid_data_type_error()));
        };
        let Ok(group) = <[u8; 4]>::try_from(octets.as_slice()) else {
            return Some(Err(common::value_out_of_range_error()));
        };
        let view = self.multicast.view.as_ref()?;
        Some(
            view.check(group)
                .map(|()| self.multicast.pending = Some(group)),
        )
    }

    /// Check the written group, changing nothing.
    pub(super) fn check_multicast(&self) -> Result<(), Error> {
        match (self.multicast.pending, &self.multicast.view) {
            (None, _) => Ok(()),
            (Some(group), Some(view)) => view.check(group),
            (Some(_), None) => Err(common::write_access_denied_error()),
        }
    }

    /// Put the written group into effect, and clear it.
    pub(super) fn activate_multicast(&mut self) -> Result<(), Error> {
        let Some(group) = self.multicast.pending else {
            return Ok(());
        };
        let view = self
            .multicast
            .view
            .as_ref()
            .ok_or_else(common::write_access_denied_error)?;
        view.activate(group)?;
        self.multicast.pending = None;
        Ok(())
    }
}
