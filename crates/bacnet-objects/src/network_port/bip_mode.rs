//! The B/IP mode of a registered port and the properties that mode adds
//! (Clause 12.56.21, Table 12-71 footnotes 11 to 13).
//!
//! FOREIGN mode adds FD_BBMD_Address and FD_Subscription_Lifetime, read
//! from the transport's registration at the moment of the read. BBMD mode
//! adds BBMD_Broadcast_Distribution_Table, BBMD_Accept_FD_Registrations and
//! BBMD_Foreign_Device_Table, each read from the transport's own tables at
//! the moment of the read, and, on an IPv4 port whose BBMD can run behind a
//! NAT router, BACnet_IP_NAT_Traversal and BACnet_IP_Global_Address
//! (Clause 12.56.32, 12.56.33; Table 12-71 footnotes 9 and 10). In either mode a written value waiting for
//! activation is read instead (`pending`).
//!
//! The mode is the last one published. After its owner stops, the port keeps
//! reporting it, as it keeps the last published bind; a BBMD's rows then show
//! the stopped transport's tables as they were left, with registrations
//! still expiring on schedule.

use alloc::sync::Arc;

use bacnet_encoding::constructed::{encode_bdt_entry, encode_fdt_entry, encode_host_n_port};
use bacnet_types::bip_port::{BbmdTables, BipPortMode, ForeignRegistration};
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostNPort};
use bacnet_types::enums::{IPMode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

/// The mode a published owner reported.
#[derive(Clone, Default)]
pub(super) enum LiveMode {
    /// No owner, or a NORMAL one.
    #[default]
    Normal,
    /// A foreign device, whose registration is read through the
    /// transport's live view.
    Foreign(Arc<dyn ForeignRegistration>),
    /// A BBMD, whose tables are read through the transport's live view.
    Bbmd(Arc<dyn BbmdTables>),
}

impl LiveMode {
    /// The mode a started owner reported. A BBMD must lend its tables by now.
    pub(super) fn from_port(mode: BipPortMode) -> Result<Self, Error> {
        Ok(match mode {
            BipPortMode::Normal => Self::Normal,
            BipPortMode::Foreign { registration } => Self::Foreign(registration),
            BipPortMode::Bbmd {
                tables: Some(tables),
            } => Self::Bbmd(tables),
            BipPortMode::Bbmd { tables: None } => {
                return Err(Error::Encoding(
                    "a registered BBMD must lend its tables once started".into(),
                ));
            }
        })
    }

    /// Whether this mode is a BBMD that can run behind a NAT router, so an
    /// IPv4 port serves the two NAT rows.
    pub(super) fn has_nat_rows(&self) -> bool {
        matches!(self, Self::Bbmd(tables) if tables.nat_traversal().is_some())
    }

    /// Whether this mode is a BBMD running behind a NAT router now.
    pub(super) fn nat_on(&self) -> bool {
        matches!(self, Self::Bbmd(tables) if tables.nat_traversal() == Some(true))
    }

    /// Whether the transport behind this mode takes written values, so its
    /// pending-activation rows are writable at all.
    pub(super) fn can_change(&self) -> bool {
        match self {
            Self::Normal => false,
            Self::Foreign(registration) => registration.can_change(),
            Self::Bbmd(tables) => tables.can_change(),
        }
    }

    pub(super) fn ip_mode(&self) -> IPMode {
        match self {
            Self::Normal => IPMode::NORMAL,
            Self::Foreign(_) => IPMode::FOREIGN,
            Self::Bbmd(_) => IPMode::BBMD,
        }
    }

    /// Read a property this mode adds; `None` for any other property,
    /// including one another mode adds.
    pub(super) fn read(&self, property: P) -> Option<Result<PropertyValue, Error>> {
        match (self, property) {
            (Self::Foreign(registration), P::FD_BBMD_ADDRESS) => {
                Some(host_n_port_value(&registration.bbmd()))
            }
            (Self::Foreign(registration), P::FD_SUBSCRIPTION_LIFETIME) => Some(Ok(
                PropertyValue::Unsigned(registration.subscription_lifetime().into()),
            )),
            (Self::Bbmd(tables), P::BBMD_BROADCAST_DISTRIBUTION_TABLE) => {
                Some(bdt_value(&tables.broadcast_distribution_table()))
            }
            (Self::Bbmd(tables), P::BBMD_ACCEPT_FD_REGISTRATIONS) => Some(Ok(
                PropertyValue::Boolean(tables.accepts_foreign_device_registrations()),
            )),
            (Self::Bbmd(tables), P::BACNET_IP_NAT_TRAVERSAL) => tables
                .nat_traversal()
                .map(|nat| Ok(PropertyValue::Boolean(nat))),
            (Self::Bbmd(tables), P::BACNET_IP_GLOBAL_ADDRESS) => tables
                .global_address()
                .map(|address| host_n_port_value(&address)),
            (Self::Bbmd(tables), P::BBMD_FOREIGN_DEVICE_TABLE) => Some(Ok(PropertyValue::List(
                tables
                    .foreign_device_table()
                    .iter()
                    .map(|entry| {
                        let mut buf = BytesMut::new();
                        encode_fdt_entry(&mut buf, entry);
                        framed(buf)
                    })
                    .collect(),
            ))),
            _ => None,
        }
    }
}

/// A BDT as the BBMD_Broadcast_Distribution_Table value: one encoded
/// entry per list element.
pub(super) fn bdt_value(table: &[BACnetBDTEntry]) -> Result<PropertyValue, Error> {
    table
        .iter()
        .map(|entry| {
            let mut buf = BytesMut::new();
            encode_bdt_entry(&mut buf, entry).map(|()| framed(buf))
        })
        .collect::<Result<_, _>>()
        .map(PropertyValue::List)
}

/// A host and port as the FD_BBMD_Address value.
pub(super) fn host_n_port_value(address: &BACnetHostNPort) -> Result<PropertyValue, Error> {
    let mut buf = BytesMut::new();
    encode_host_n_port(&mut buf, address).map(|()| framed(buf))
}

/// One constructed value, already encoded, as the property value.
fn framed(buf: BytesMut) -> PropertyValue {
    PropertyValue::ApplicationData(buf.to_vec())
}
