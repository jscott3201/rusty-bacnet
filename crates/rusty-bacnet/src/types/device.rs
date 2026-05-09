use super::*;

// ---------------------------------------------------------------------------
// DiscoveredDevice — read-only wrapper for discovered BACnet devices
// ---------------------------------------------------------------------------

/// A discovered BACnet device from WhoIs/IAm.
#[pyclass(name = "DiscoveredDevice", frozen)]
pub struct PyDiscoveredDevice {
    inner: DiscoveredDevice,
    created: Instant,
}

#[pymethods]
impl PyDiscoveredDevice {
    #[getter]
    fn object_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.inner.object_identifier)
    }

    #[getter]
    fn mac_address(&self) -> Vec<u8> {
        self.inner.mac_address.to_vec()
    }

    #[getter]
    fn max_apdu_length(&self) -> u32 {
        self.inner.max_apdu_length
    }

    #[getter]
    fn segmentation_supported(&self) -> PySegmentation {
        PySegmentation {
            inner: self.inner.segmentation_supported,
        }
    }

    #[getter]
    fn vendor_id(&self) -> u16 {
        self.inner.vendor_id
    }

    #[getter]
    fn seconds_since_seen(&self) -> f64 {
        self.created.elapsed().as_secs_f64()
    }

    #[getter]
    fn source_network(&self) -> Option<u16> {
        self.inner.source_network
    }

    #[getter]
    fn source_address(&self) -> Option<Vec<u8>> {
        self.inner.source_address.as_ref().map(|m| m.to_vec())
    }

    fn __repr__(&self) -> String {
        format!(
            "DiscoveredDevice({}, instance={}, vendor={})",
            self.inner.object_identifier.object_type(),
            self.inner.object_identifier.instance_number(),
            self.inner.vendor_id
        )
    }
}

impl PyDiscoveredDevice {
    pub fn from_rust(dev: DiscoveredDevice) -> Self {
        Self {
            created: dev.last_seen,
            inner: dev,
        }
    }
}
