//! Explicit command origins and object-owned source tracking (Clause 19.5):
//! per priority for a commandable Present_Value, and a single source for one
//! with no priority array (#1552).
//!
//! Standalone callers assert their origin; validation establishes syntax, not
//! authentication or database membership. Full servers derive remote origins
//! from ingress and validate local membership under their mutation guard.
use bacnet_types::constructed::{BACnetAddress, BACnetDeviceObjectReference, BACnetValueSource};
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

mod single;
mod tracking;
pub(crate) use single::SingleValueSource;
pub(crate) use tracking::write_sourced_priority;
pub(crate) use tracking::ValueSourceTracking;

/// Address-to-Device correlation, never authentication of a principal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandDeviceBinding {
    /// No fresh unambiguous binding matches the actual address.
    Unknown,
    /// Exactly one Device is correlated with the address.
    Unique(ObjectIdentifier),
    /// More than one Device matches; corrections fail closed.
    Ambiguous,
}

/// Actual command writer, kept separately from its correctable source claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOrigin {
    /// Actual network writer with an admission-time correlation snapshot.
    Remote {
        /// Original source address; routed senders retain SNET/SADR.
        actual_address: BACnetAddress,
        /// Correlation does not authenticate the writer.
        binding: CommandDeviceBinding,
    },
    /// Trusted local producer owned by the selected concrete Device.
    Local {
        /// Device that retains correction ownership.
        owner_device: ObjectIdentifier,
        /// Optional local initiator published as the command source.
        initiating_object: Option<ObjectIdentifier>,
    },
}

impl CommandOrigin {
    /// Reject malformed actual writer identities; generic ValueSource claims
    /// deliberately have a broader grammar than an actual command origin. A
    /// remote writer's MAC must fit [`BACnetAddress::MAX_MAC_LEN`] octets, so
    /// the Value_Source published from it always encodes (#1156).
    pub fn validate(&self) -> Result<(), Error> {
        let concrete =
            |oid: ObjectIdentifier| oid.instance_number() != ObjectIdentifier::WILDCARD_INSTANCE;
        let device =
            |oid: ObjectIdentifier| concrete(oid) && oid.object_type() == ObjectType::DEVICE;
        let valid = match self {
            Self::Local {
                owner_device,
                initiating_object,
            } => device(*owner_device) && initiating_object.is_none_or(concrete),
            Self::Remote {
                actual_address,
                binding,
            } => {
                actual_address.network_number != u16::MAX
                    && !actual_address.mac_address.is_empty()
                    && actual_address.mac_address.len() <= BACnetAddress::MAX_MAC_LEN
                    && match binding {
                        CommandDeviceBinding::Unique(oid) => device(*oid),
                        _ => true,
                    }
            }
        };
        if valid {
            Ok(())
        } else {
            Err(crate::common::write_access_denied_error())
        }
    }

    pub(crate) fn published_source(&self) -> BACnetValueSource {
        match self {
            Self::Local {
                owner_device,
                initiating_object,
            } => BACnetValueSource::Object(BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: initiating_object.unwrap_or(*owner_device),
            }),
            Self::Remote {
                binding: CommandDeviceBinding::Unique(device),
                ..
            } => BACnetValueSource::Object(BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: *device,
            }),
            Self::Remote { actual_address, .. } => {
                BACnetValueSource::Address(actual_address.clone())
            }
        }
    }

    pub(crate) fn permits_correction_by(&self, incoming: &Self) -> bool {
        match (self, incoming) {
            (
                Self::Local {
                    owner_device: old, ..
                },
                Self::Local {
                    owner_device: new, ..
                },
            ) => old == new,
            (
                Self::Remote {
                    actual_address: old_address,
                    binding: old,
                },
                Self::Remote {
                    actual_address: new_address,
                    binding: new,
                },
            ) => {
                use CommandDeviceBinding::*;
                match (old, new) {
                    (Ambiguous, _) | (_, Ambiguous) => false,
                    (Unique(a), Unique(b)) => a == b,
                    _ => old_address == new_address,
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn test_origin() -> crate::command_source::CommandOrigin {
    crate::command_source::CommandOrigin::Local {
        owner_device: bacnet_types::primitives::ObjectIdentifier::new(
            bacnet_types::enums::ObjectType::DEVICE,
            1,
        )
        .unwrap(),
        initiating_object: None,
    }
}
