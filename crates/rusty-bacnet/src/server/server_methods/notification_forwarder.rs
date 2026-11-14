//! Notification Forwarder registration, with the Recipient_List and
//! Port_Filter the application seeds, and the forwarders' save counters.

use super::super::*;

use std::collections::BTreeMap;

use bacnet_objects::notification_forwarder::{
    FileNotificationForwarderPersistence, NotificationForwarderObject,
};
use bacnet_types::constructed::BACnetPortPermission;

#[pymethods]
impl BACnetServer {
    /// Add a Notification Forwarder object to the server (before starting).
    ///
    /// With `storage_path`, Recipient_List and Subscribed_Recipients are kept
    /// in that file and restored when the server is built again.
    ///
    /// `recipients` seeds Recipient_List with `Destination` mappings, in
    /// order, through the object's `add_destination`, so the list holds the
    /// destinations a client could write: more than 32, or an address MAC
    /// past 18 octets, raises BacnetProtocolError. With `storage_path`, a
    /// Recipient_List a client wrote, once saved, wins: until a write sets
    /// the list, the seed applies at every start and is not saved. `port_filter` serves
    /// Port_Filter as `(port_id, enabled)` pairs, one per network port; the
    /// server receives through Port_ID 0. Without it the property is absent.
    #[pyo3(signature = (
        instance,
        name,
        process_identifier_filter=None,
        local_forwarding_only=false,
        storage_path=None,
        *,
        recipients=None,
        port_filter=None
    ))]
    fn add_notification_forwarder(
        &self,
        instance: u32,
        name: &str,
        process_identifier_filter: Option<u32>,
        local_forwarding_only: bool,
        storage_path: Option<&str>,
        recipients: Option<Vec<Bound<'_, PyAny>>>,
        port_filter: Option<Vec<(u8, bool)>>,
    ) -> PyResult<()> {
        let recipients = crate::types::destinations(recipients)?;
        let mut nf = match storage_path {
            Some(path) => {
                let storage =
                    Arc::new(FileNotificationForwarderPersistence::new(path).map_err(to_py_err)?);
                NotificationForwarderObject::with_persistence(instance, name, storage)
                    .map_err(to_py_err)?
            }
            None => NotificationForwarderObject::new(instance, name).map_err(to_py_err)?,
        };
        nf.set_process_identifier_filter(process_identifier_filter);
        nf.set_local_forwarding_only(local_forwarding_only);
        for destination in recipients {
            nf.add_destination(destination).map_err(to_py_err)?;
        }
        nf.set_port_filter(port_filter.map(|ports| {
            ports
                .into_iter()
                .map(|(port_id, enabled)| BACnetPortPermission { port_id, enabled })
                .collect()
        }));
        let counters = (instance, nf.save_counters());
        let mut pending = self.lock_pending()?;
        if self.started.load(Ordering::Acquire) {
            return Err(PyRuntimeError::new_err(
                "cannot add objects after start() — server is already running",
            ));
        }
        self.pending_forwarder_save_counters
            .lock()
            .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))?
            .push(counters);
        pending.push(Box::new(nf));
        Ok(())
    }

    /// Sample each Notification Forwarder's save counters, keyed by
    /// instance: `{"failed_saves": n}` per forwarder of
    /// the running server. The totals belong to the objects, so they count
    /// from registration and saturate at 2**64-1; a forwarder without
    /// `storage_path` stays at zero. Raises RuntimeError("server not
    /// started") before start and after stop.
    fn forwarder_save_counters<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let counters = Arc::clone(&self.forwarder_save_counters);
        crate::py_async::future_into_py(py, async move {
            let guard = inner.read().await;
            if guard.is_none() {
                return Err(PyRuntimeError::new_err("server not started"));
            }
            let entries = counters
                .lock()
                .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))?
                .iter()
                .map(|(instance, counters)| {
                    (
                        *instance,
                        BTreeMap::from([("failed_saves", counters.failed_saves())]),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            Ok(entries)
        })
    }
}
