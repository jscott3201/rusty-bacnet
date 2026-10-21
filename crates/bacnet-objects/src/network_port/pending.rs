//! Writes that wait for activation: a BBMD port's
//! BBMD_Broadcast_Distribution_Table and BBMD_Accept_FD_Registrations
//! (Clause 12.56.34, 12.56.35, #1578) and, behind a NAT router,
//! BACnet_IP_NAT_Traversal and BACnet_IP_Global_Address (Clause 12.56.32,
//! 12.56.33, #940), and a foreign device's
//! FD_BBMD_Address and FD_Subscription_Lifetime (Clause 12.56.37,
//! 12.56.38, #1825).
//!
//! A write doesn't take effect at once: the value waits, Changes_Pending
//! reads TRUE, and a read of the property returns the value written, while
//! the transport keeps working with the old one, so a Read-BDT answers with
//! the BDT in use and the foreign device keeps registering where it did
//! (Clause 12.56.13, 12.56.34). A ReinitializeDevice ACTIVATE_CHANGES or
//! WARMSTART puts the waiting values into effect through the transport
//! ([`NetworkPortObject::activate_changes`]).
//!
//! A written value is checked by the transport when it is written, so one
//! it would refuse is refused then and changes nothing: a BBMD's value on
//! its own (a BDT against the row policy, a global address as an IPv4
//! unicast host), and a BBMD address and time-to-live as the registration
//! they would make together with the other value waiting. A BBMD's values
//! are checked together at activation (a NAT BBMD's BDT must list its
//! global address and be two-hop only), so they can be written in any
//! order. The rows take
//! writes only when the port's owner answers ReinitializeDevice
//! (`set_activation_internal`); otherwise a written value could never take
//! effect, and the write is WRITE_ACCESS_DENIED.

use bacnet_types::bip_port::{BbmdChanges, ForeignDeviceChanges};
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::bip_mode::{bdt_value, host_n_port_value, LiveMode};
use super::NetworkPortObject;
use crate::common;

/// The values written to a port and waiting for activation. Only the
/// current mode's half is ever set: publishing a mode clears both.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Pending {
    /// A BBMD's rows.
    bbmd: BbmdChanges,
    /// A foreign device's rows.
    foreign: ForeignDeviceChanges,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.bbmd.is_empty() && self.foreign.is_empty()
    }
}

impl NetworkPortObject {
    /// The value written to `property` and waiting for activation, if any.
    pub(super) fn pending_value(&self, property: P) -> Option<Result<PropertyValue, Error>> {
        let pending = &self.pending;
        match property {
            P::BBMD_BROADCAST_DISTRIBUTION_TABLE => pending
                .bbmd
                .broadcast_distribution_table
                .as_deref()
                .map(bdt_value),
            P::BBMD_ACCEPT_FD_REGISTRATIONS => pending
                .bbmd
                .accept_foreign_device_registrations
                .map(|accept| Ok(PropertyValue::Boolean(accept))),
            P::BACNET_IP_NAT_TRAVERSAL => pending
                .bbmd
                .nat_traversal
                .map(|nat| Ok(PropertyValue::Boolean(nat))),
            P::BACNET_IP_GLOBAL_ADDRESS => {
                pending.bbmd.global_address.as_ref().map(host_n_port_value)
            }
            P::FD_BBMD_ADDRESS => pending.foreign.bbmd.as_ref().map(host_n_port_value),
            P::FD_SUBSCRIPTION_LIFETIME => pending
                .foreign
                .subscription_lifetime
                .map(|ttl| Ok(PropertyValue::Unsigned(ttl.into()))),
            _ => None,
        }
    }

    /// Take a write to a property the port's mode lets clients change;
    /// `None` for any other property, or a port in another mode.
    pub(super) fn write_pending(
        &mut self,
        property: P,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        let row = match &self.mode {
            LiveMode::Bbmd(_) => {
                matches!(
                    property,
                    P::BBMD_BROADCAST_DISTRIBUTION_TABLE | P::BBMD_ACCEPT_FD_REGISTRATIONS
                ) || (nat_row(property) && self.serves_nat())
            }
            LiveMode::Foreign(_) => {
                matches!(property, P::FD_BBMD_ADDRESS | P::FD_SUBSCRIPTION_LIFETIME)
            }
            LiveMode::Normal => false,
        };
        if !row {
            return None;
        }
        // A value nothing could activate would wait for ever: without an
        // owner that answers ReinitializeDevice, or a transport that takes
        // changes, the rows stay read-only (Clause 16.4 ties pending changes
        // to ACTIVATE_CHANGES).
        if !self.activates_changes() {
            return Some(Err(common::write_access_denied_error()));
        }
        if array_index.is_some() {
            return Some(Err(common::property_is_not_an_array_error()));
        }
        Some(match &self.mode {
            LiveMode::Bbmd(tables) => {
                // The value written, alone.
                let mut changes = BbmdChanges::default();
                let decoded = match (property, value) {
                    (
                        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
                        PropertyValue::ApplicationData(octets),
                    ) => bacnet_encoding::constructed::decode_bdt_entry_list(octets)
                        .map_err(|_| common::invalid_data_encoding_error())
                        .map(|table| changes.broadcast_distribution_table = Some(table)),
                    (P::BBMD_ACCEPT_FD_REGISTRATIONS, PropertyValue::Boolean(accept)) => {
                        changes.accept_foreign_device_registrations = Some(*accept);
                        Ok(())
                    }
                    (P::BACNET_IP_NAT_TRAVERSAL, PropertyValue::Boolean(nat)) => {
                        changes.nat_traversal = Some(*nat);
                        Ok(())
                    }
                    (P::BACNET_IP_GLOBAL_ADDRESS, PropertyValue::ApplicationData(octets)) => {
                        decode_host_n_port(octets)
                            .map(|global| changes.global_address = Some(global))
                    }
                    _ => Err(common::invalid_data_type_error()),
                };
                // Checked alone; the rules that tie the values together (a
                // NAT BBMD's BDT lists its global address) wait for
                // activation, so they can be written in any order.
                decoded
                    .and_then(|()| tables.check_value(&changes))
                    .map(|()| merge(&mut self.pending.bbmd, changes))
            }
            LiveMode::Foreign(registration) => {
                let mut changes = self.pending.foreign.clone();
                let decoded = match (property, value) {
                    (P::FD_BBMD_ADDRESS, PropertyValue::ApplicationData(octets)) => {
                        decode_host_n_port(octets).map(|bbmd| changes.bbmd = Some(bbmd))
                    }
                    (P::FD_SUBSCRIPTION_LIFETIME, PropertyValue::Unsigned(ttl)) => {
                        // Unsigned16 (Clause 12.56.38).
                        u16::try_from(*ttl)
                            .map(|ttl| changes.subscription_lifetime = Some(ttl))
                            .map_err(|_| common::value_out_of_range_error())
                    }
                    _ => Err(common::invalid_data_type_error()),
                };
                // The pair is checked as it would register, so a value that
                // only works with the other one waiting is judged with it.
                decoded
                    .and_then(|()| registration.check(&changes))
                    .map(|()| self.pending.foreign = changes)
            }
            // No row is writable in NORMAL mode, so this isn't reached.
            LiveMode::Normal => Err(common::write_access_denied_error()),
        })
    }

    /// Whether the pending values would still be taken, changing nothing:
    /// the check a ReinitializeDevice makes before its handler runs.
    pub fn check_pending_changes(&self) -> Result<(), Error> {
        self.check_multicast()?;
        if self.pending.is_empty() {
            return Ok(());
        }
        match &self.mode {
            LiveMode::Bbmd(tables) => tables.check(&self.pending.bbmd),
            LiveMode::Foreign(registration) => registration.check(&self.pending.foreign),
            LiveMode::Normal => Err(common::write_access_denied_error()),
        }
    }

    /// Whether written values wait for activation: Changes_Pending.
    pub(super) fn changes_pending(&self) -> bool {
        !self.pending.is_empty() || self.multicast.is_pending()
    }

    /// Whether written values can be activated, so the pending-activation
    /// rows take writes: a ReinitializeDevice owner activates them, and the
    /// transport takes them.
    pub(super) fn activates_changes(&self) -> bool {
        self.activation && self.mode.can_change()
    }

    /// Put the values written to this port into effect, as a
    /// ReinitializeDevice ACTIVATE_CHANGES or WARMSTART does (Clause
    /// 12.56.13), and clear Changes_Pending. Whether there were any.
    ///
    /// The transport takes all of them or none: when it refuses, the error
    /// comes back and they stay pending. Every waiting value is checked
    /// first, so a refused multicast group leaves the mode's values waiting
    /// too; the mode's values then take effect, and the group last.
    pub fn activate_changes(&mut self) -> Result<bool, Error> {
        if !self.changes_pending() {
            return Ok(false);
        }
        self.check_pending_changes()?;
        // Only a NORMAL port lends a group that can change, and NORMAL has
        // no mode rows to write, so the two halves never wait together and
        // the order below can't leave one half applied and the other
        // refused. A BBMD that serves a group (#941) lends it read-only,
        // since its own BDT row and its group go together (Annex J.8.3):
        // making it writable needs one activation for both (#1963).
        debug_assert!(
            self.pending.is_empty() || !self.multicast.is_pending(),
            "mode values and a multicast group pending together"
        );
        if !self.pending.is_empty() {
            self.activate_mode_changes()?;
        }
        self.activate_multicast()?;
        Ok(true)
    }

    /// Put the mode's waiting values into effect, and clear them.
    fn activate_mode_changes(&mut self) -> Result<(), Error> {
        match &self.mode {
            LiveMode::Bbmd(tables) => tables.activate(&self.pending.bbmd)?,
            LiveMode::Foreign(registration) => registration.activate(&self.pending.foreign)?,
            // A port whose mode changed dropped its pending values then.
            LiveMode::Normal => return Err(common::write_access_denied_error()),
        }
        self.pending = Pending::default();
        Ok(())
    }
}

/// Add `written`'s values to `pending`.
fn merge(pending: &mut BbmdChanges, written: BbmdChanges) {
    if written.broadcast_distribution_table.is_some() {
        pending.broadcast_distribution_table = written.broadcast_distribution_table;
    }
    if written.accept_foreign_device_registrations.is_some() {
        pending.accept_foreign_device_registrations = written.accept_foreign_device_registrations;
    }
    if written.nat_traversal.is_some() {
        pending.nat_traversal = written.nat_traversal;
    }
    if written.global_address.is_some() {
        pending.global_address = written.global_address;
    }
}

/// A written FD_BBMD_Address or BACnet_IP_Global_Address: exactly one
/// BACnetHostNPort.
fn decode_host_n_port(octets: &[u8]) -> Result<bacnet_types::constructed::BACnetHostNPort, Error> {
    match bacnet_encoding::constructed::decode_host_n_port(octets, 0) {
        Ok((address, end)) if end == octets.len() => Ok(address),
        _ => Err(common::invalid_data_encoding_error()),
    }
}

/// The rows a write leaves pending for activation, in whichever mode has
/// them.
pub(super) fn writable_pending_row(property: P) -> bool {
    matches!(
        property,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE
            | P::BBMD_ACCEPT_FD_REGISTRATIONS
            | P::FD_BBMD_ADDRESS
            | P::FD_SUBSCRIPTION_LIFETIME
    ) || nat_row(property)
}

/// The two NAT rows (Clause 12.56.32, 12.56.33).
pub(super) fn nat_row(property: P) -> bool {
    matches!(
        property,
        P::BACNET_IP_NAT_TRAVERSAL | P::BACNET_IP_GLOBAL_ADDRESS
    )
}

impl NetworkPortObject {
    /// Whether this port serves the NAT rows: an IPv4 port in BBMD mode
    /// whose transport can run behind a NAT router.
    pub(super) fn serves_nat(&self) -> bool {
        self.ip
            .as_ref()
            .is_some_and(super::ip_config::IpConfig::is_ipv4)
            && self.mode.has_nat_rows()
    }
}
