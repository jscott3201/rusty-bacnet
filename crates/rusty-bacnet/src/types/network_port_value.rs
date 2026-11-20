//! Whole Network Port BDT and FDT properties, projected from their original octets.

use std::net::Ipv4Addr;

use bacnet_encoding::constructed::{decode_bdt_entry_list, decode_fdt_entry_list};
use bacnet_types::constructed::{BACnetBDTEntry, BACnetFDTEntry, BACnetHostAddress};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use super::PyPropertyValue;
use super::property_value::Form;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NetworkPortTable {
    Bdt,
    Fdt,
}

enum Rows {
    Bdt(Vec<BACnetBDTEntry>),
    Fdt(Vec<BACnetFDTEntry>),
}

impl NetworkPortTable {
    pub(super) fn of(
        object_type: ObjectType,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Option<Self> {
        if object_type != ObjectType::NETWORK_PORT || index.is_some() {
            return None;
        }
        match property {
            PropertyIdentifier::BBMD_BROADCAST_DISTRIBUTION_TABLE => Some(Self::Bdt),
            PropertyIdentifier::BBMD_FOREIGN_DEVICE_TABLE => Some(Self::Fdt),
            _ => None,
        }
    }

    pub(super) fn tag(self) -> &'static str {
        match self {
            Self::Bdt => "bdt",
            Self::Fdt => "fdt",
        }
    }

    fn decode(self, octets: &[u8]) -> Result<Rows, Error> {
        match self {
            Self::Bdt => decode_bdt_entry_list(octets).map(Rows::Bdt),
            Self::Fdt => decode_fdt_entry_list(octets).map(Rows::Fdt),
        }
    }

    pub(super) fn read(self, octets: &[u8]) -> Result<PyPropertyValue, Error> {
        self.decode(octets)?;
        Ok(PyPropertyValue {
            form: Form::NetworkPortTable(self),
            ..PyPropertyValue::from_rust(PropertyValue::ApplicationData(octets.to_vec()))
        })
    }

    pub(super) fn to_py(self, py: Python<'_>, octets: &[u8]) -> PyResult<Py<PyAny>> {
        let rows = PyList::empty(py);
        match self
            .decode(octets)
            .map_err(|e| PyValueError::new_err(e.to_string()))?
        {
            Rows::Bdt(entries) => {
                for entry in entries {
                    let row = PyDict::new(py);
                    let (kind, value) = match entry.bbmd_address.host {
                        BACnetHostAddress::Ip(ip) => ("ip", Some(ip.to_string())),
                        BACnetHostAddress::Name(name) => ("name", Some(name)),
                        BACnetHostAddress::None => ("none", None),
                    };
                    row.set_item("address_kind", kind)?;
                    row.set_item("address", value)?;
                    row.set_item("port", entry.bbmd_address.port)?;
                    row.set_item(
                        "broadcast_mask",
                        entry
                            .broadcast_mask
                            .map(|mask| Ipv4Addr::from(mask).to_string()),
                    )?;
                    rows.append(row)?;
                }
            }
            Rows::Fdt(entries) => {
                for entry in entries {
                    let row = PyDict::new(py);
                    row.set_item("address", entry.address.ip().to_string())?;
                    row.set_item("port", entry.address.port())?;
                    row.set_item("ttl", entry.time_to_live)?;
                    row.set_item("seconds_remaining", entry.remaining_time_to_live)?;
                    rows.append(row)?;
                }
            }
        }
        Ok(rows.into_any().unbind())
    }
}

#[pymethods]
impl PyPropertyValue {
    /// Rebuild a complete table read from the original application data.
    #[staticmethod]
    fn _network_port_table(tag: &str, octets: Vec<u8>) -> PyResult<Self> {
        let table = match tag {
            "bdt" => NetworkPortTable::Bdt,
            "fdt" => NetworkPortTable::Fdt,
            _ => return Err(PyValueError::new_err("expected a bdt or fdt table")),
        };
        table
            .read(&octets)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }
}
