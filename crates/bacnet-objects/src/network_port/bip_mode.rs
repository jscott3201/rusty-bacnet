//! The B/IP mode of a registered port and the properties that mode adds
//! (Clause 12.56.21, Table 12-71 footnotes 11 to 13).
//!
//! FOREIGN mode adds FD_BBMD_Address and FD_Subscription_Lifetime, from the
//! owner's configuration, which cannot change while it owns the port. BBMD
//! mode adds BBMD_Broadcast_Distribution_Table, BBMD_Accept_FD_Registrations
//! and BBMD_Foreign_Device_Table, each read from the transport's own tables
//! at the moment of the read.
//!
//! The mode is the last one published. After its owner stops, the port keeps
//! reporting it, as it keeps the last published bind; a BBMD's rows then show
//! the stopped transport's tables as they were left, with registrations
//! still expiring on schedule.

use std::sync::Arc;

use bacnet_encoding::constructed::{encode_bdt_entry, encode_fdt_entry, encode_host_n_port};
use bacnet_types::bip_port::{BbmdTables, BipPortMode};
use bacnet_types::constructed::BACnetHostNPort;
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
    /// A foreign device registering with `bbmd` for `lifetime` seconds.
    Foreign {
        bbmd: BACnetHostNPort,
        lifetime: u16,
    },
    /// A BBMD, whose tables are read through the transport's live view.
    Bbmd(Arc<dyn BbmdTables>),
}

impl LiveMode {
    /// The mode a started owner reported. A BBMD must lend its tables by now.
    pub(super) fn from_port(mode: BipPortMode) -> Result<Self, Error> {
        Ok(match mode {
            BipPortMode::Normal => Self::Normal,
            BipPortMode::Foreign {
                bbmd,
                subscription_lifetime,
            } => Self::Foreign {
                bbmd,
                lifetime: subscription_lifetime,
            },
            BipPortMode::Bbmd {
                tables: Some(tables),
            } => Self::Bbmd(tables),
            BipPortMode::Bbmd { tables: None } => {
                return Err(Error::Encoding(
                    "a registered BBMD must lend its tables once started".into(),
                ))
            }
        })
    }

    pub(super) fn ip_mode(&self) -> IPMode {
        match self {
            Self::Normal => IPMode::NORMAL,
            Self::Foreign { .. } => IPMode::FOREIGN,
            Self::Bbmd(_) => IPMode::BBMD,
        }
    }

    /// Read a property this mode adds; `None` for any other property,
    /// including one another mode adds.
    pub(super) fn read(&self, property: P) -> Option<Result<PropertyValue, Error>> {
        match (self, property) {
            (Self::Foreign { bbmd, .. }, P::FD_BBMD_ADDRESS) => {
                let mut buf = BytesMut::new();
                Some(encode_host_n_port(&mut buf, bbmd).map(|()| framed(buf)))
            }
            (Self::Foreign { lifetime, .. }, P::FD_SUBSCRIPTION_LIFETIME) => {
                Some(Ok(PropertyValue::Unsigned((*lifetime).into())))
            }
            (Self::Bbmd(tables), P::BBMD_BROADCAST_DISTRIBUTION_TABLE) => Some(
                tables
                    .broadcast_distribution_table()
                    .iter()
                    .map(|entry| {
                        let mut buf = BytesMut::new();
                        encode_bdt_entry(&mut buf, entry).map(|()| framed(buf))
                    })
                    .collect::<Result<_, _>>()
                    .map(PropertyValue::List),
            ),
            (Self::Bbmd(tables), P::BBMD_ACCEPT_FD_REGISTRATIONS) => Some(Ok(
                PropertyValue::Boolean(tables.accepts_foreign_device_registrations()),
            )),
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

/// One constructed value, already encoded, as the property value.
fn framed(buf: BytesMut) -> PropertyValue {
    PropertyValue::ApplicationData(buf.to_vec())
}
