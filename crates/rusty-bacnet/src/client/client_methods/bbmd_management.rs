//! BBMD management by hand (Annex J, #1841): read and write a BBMD's
//! tables, delete a foreign device's entry, and register once as a foreign
//! device. Each sends one BVLC request to `address` and waits up to three
//! seconds for its answer; a failure raises what
//! [`crate::errors::bvlc_exchange_err`] makes of it.

use super::super::*;
use crate::bip_options::{bdt_entry, bdt_row, parse_address};
use crate::errors::bvlc_exchange_err;

/// The six-octet B/IP MAC of `"ip"` or `"ip:port"`.
fn bip_mac(text: &str, what: &str) -> PyResult<[u8; 6]> {
    let address = parse_address(text, what)?;
    let mut mac = [0u8; 6];
    mac[..4].copy_from_slice(&address.ip().octets());
    mac[4..].copy_from_slice(&address.port().to_be_bytes());
    Ok(mac)
}

#[pymethods]
impl BACnetClient {
    /// Read the Broadcast Distribution Table of the BBMD at `address`, as
    /// `("ip:port", "mask")` rows.
    fn read_bdt<'py>(&self, py: Python<'py>, address: String) -> PyResult<Bound<'py, PyAny>> {
        let target = bip_mac(&address, "address")?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let rows = started(&inner)
                .await?
                .read_bdt(&target)
                .await
                .map_err(bvlc_exchange_err)?;
            Ok(rows.iter().map(bdt_row).collect::<Vec<_>>())
        })
    }

    /// Ask the BBMD at `address` to take `entries` (rows as `Bbmd(bdt=)`
    /// takes them) as its Broadcast Distribution Table. A BBMD that follows
    /// the 2020 standard refuses: `BacnetBvlcNakError`, result code 0x0010.
    fn write_bdt<'py>(
        &self,
        py: Python<'py>,
        address: String,
        entries: Vec<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let target = bip_mac(&address, "address")?;
        let entries = entries
            .iter()
            .map(bdt_entry)
            .collect::<PyResult<Vec<_>>>()?;
        let inner = self.inner.clone();
        let future = async move {
            started(&inner)
                .await?
                .write_bdt(&target, &entries)
                .await
                .map_err(bvlc_exchange_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Read the Foreign Device Table of the BBMD at `address`: a dict per
    /// entry with `address` (`"ip:port"`), `ttl` and `seconds_remaining`.
    fn read_fdt<'py>(&self, py: Python<'py>, address: String) -> PyResult<Bound<'py, PyAny>> {
        let target = bip_mac(&address, "address")?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let entries = started(&inner)
                .await?
                .read_fdt(&target)
                .await
                .map_err(bvlc_exchange_err)?;
            crate::py_async::attach(|py| {
                let list = pyo3::types::PyList::empty(py);
                for entry in entries {
                    let dict = PyDict::new(py);
                    let at = std::net::SocketAddrV4::new(Ipv4Addr::from(entry.ip), entry.port);
                    dict.set_item("address", at.to_string())?;
                    dict.set_item("ttl", entry.ttl)?;
                    dict.set_item("seconds_remaining", entry.seconds_remaining)?;
                    list.append(dict)?;
                }
                Ok(list.into_any().unbind())
            })
        })
    }

    /// Ask the BBMD at `address` to delete the Foreign Device Table entry
    /// of the foreign device at `entry` (`"ip:port"`). A refusal, such as
    /// no such entry or a sender the BBMD doesn't allow, raises
    /// `BacnetBvlcNakError`, result code 0x0050.
    fn delete_fdt_entry<'py>(
        &self,
        py: Python<'py>,
        address: String,
        entry: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let target = bip_mac(&address, "address")?;
        let entry = parse_address(&entry, "entry")?;
        let inner = self.inner.clone();
        let future = async move {
            started(&inner)
                .await?
                .delete_fdt_entry(&target, entry.ip().octets(), entry.port())
                .await
                .map_err(bvlc_exchange_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Register once as a foreign device with the BBMD at `address`, for
    /// `ttl` seconds. A refusal raises `BacnetBvlcNakError`, result code
    /// 0x0030. This doesn't renew or route broadcasts through the BBMD:
    /// `foreign_device=` does that.
    fn register_foreign_device_bvlc<'py>(
        &self,
        py: Python<'py>,
        address: String,
        ttl: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let target = bip_mac(&address, "address")?;
        let inner = self.inner.clone();
        let future = async move {
            started(&inner)
                .await?
                .register_foreign_device_bvlc(&target, ttl)
                .await
                .map_err(bvlc_exchange_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}
