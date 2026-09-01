use super::super::*;

use bacnet_objects::file::FileConfiguration;
use bacnet_types::enums::{FileAccessMethod, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::types::{PyBytes, PyList};

const INVALID_ACCESS_METHOD: &str = "access_method must be 'stream' or 'record'";
const STARTED_CONFIGURATION: &str =
    "cannot configure files after start() — server is already running";

fn parse_access_method(access_method: &str) -> PyResult<FileAccessMethod> {
    match access_method {
        "stream" => Ok(FileAccessMethod::STREAM_ACCESS),
        "record" => Ok(FileAccessMethod::RECORD_ACCESS),
        _ => Err(PyValueError::new_err(INVALID_ACCESS_METHOD)),
    }
}

fn file_oid(instance: u32) -> PyResult<ObjectIdentifier> {
    ObjectIdentifier::new(ObjectType::FILE, instance).map_err(to_py_err)
}

fn missing_file(instance: u32) -> PyErr {
    PyValueError::new_err(format!("no pending File object with instance {instance}"))
}

fn missing_configuration(instance: u32) -> PyErr {
    PyTypeError::new_err(format!(
        "pending File object with instance {instance} does not support built-in File configuration"
    ))
}

impl BACnetServer {
    fn with_pending_file_configuration<R>(
        &self,
        instance: u32,
        operation: impl FnOnce(&dyn FileConfiguration) -> PyResult<R>,
    ) -> PyResult<R> {
        let oid = file_oid(instance)?;
        let guard = self.lock_pending()?;
        if self.started.load(Ordering::Acquire) {
            return Err(PyRuntimeError::new_err(STARTED_CONFIGURATION));
        }
        let object = guard
            .iter()
            .rev()
            .find(|object| object.object_identifier() == oid)
            .ok_or_else(|| missing_file(instance))?;
        let configuration = object
            .file_configuration_internal()
            .ok_or_else(|| missing_configuration(instance))?;
        operation(configuration)
    }

    fn with_pending_file_configuration_mut<R>(
        &self,
        instance: u32,
        operation: impl FnOnce(&mut dyn FileConfiguration) -> PyResult<R>,
    ) -> PyResult<R> {
        let oid = file_oid(instance)?;
        let mut guard = self.lock_pending()?;
        if self.started.load(Ordering::Acquire) {
            return Err(PyRuntimeError::new_err(STARTED_CONFIGURATION));
        }
        let object = guard
            .iter_mut()
            .rev()
            .find(|object| object.object_identifier() == oid)
            .ok_or_else(|| missing_file(instance))?;
        let configuration = object
            .file_configuration_internal_mut()
            .ok_or_else(|| missing_configuration(instance))?;
        operation(configuration)
    }
}

#[pymethods]
impl BACnetServer {
    /// Select a pending built-in File's access method before start().
    ///
    /// Choose the access method before loading the corresponding payload;
    /// changing it does not convert stream data to records or vice versa.
    #[pyo3(signature = (instance, access_method))]
    fn set_file_access_method(&self, instance: u32, access_method: &str) -> PyResult<()> {
        let access_method = parse_access_method(access_method)?;
        self.with_pending_file_configuration_mut(instance, |configuration| {
            configuration.set_access_method(access_method);
            Ok(())
        })
    }

    /// Copy bytes into a pending stream-access built-in File before start().
    #[pyo3(signature = (instance, data))]
    fn set_file_data(&self, instance: u32, data: &Bound<'_, PyBytes>) -> PyResult<()> {
        let data = data.as_bytes().to_vec();
        self.with_pending_file_configuration_mut(instance, |configuration| {
            configuration.set_stream_data(data).map_err(to_py_err)
        })
    }

    /// Return a fresh bytes copy from a pending stream-access File before start().
    #[pyo3(signature = (instance))]
    fn get_file_data<'py>(&self, py: Python<'py>, instance: u32) -> PyResult<Bound<'py, PyBytes>> {
        let data = self.with_pending_file_configuration(instance, |configuration| {
            configuration
                .stream_data()
                .map(|data| data.to_vec())
                .map_err(to_py_err)
        })?;
        Ok(PyBytes::new(py, &data))
    }

    /// Copy a list of bytes records into a pending record-access File before start().
    #[pyo3(signature = (instance, records))]
    fn set_file_records(&self, instance: u32, records: &Bound<'_, PyList>) -> PyResult<()> {
        let records = records
            .iter()
            .map(|record| {
                record
                    .cast::<PyBytes>()
                    .map(|record| record.as_bytes().to_vec())
                    .map_err(Into::into)
            })
            .collect::<PyResult<Vec<_>>>()?;
        self.with_pending_file_configuration_mut(instance, |configuration| {
            configuration.set_record_data(records).map_err(to_py_err)
        })
    }

    /// Return a fresh list with fresh bytes values from a pending record File before start().
    #[pyo3(signature = (instance))]
    fn get_file_records<'py>(
        &self,
        py: Python<'py>,
        instance: u32,
    ) -> PyResult<Bound<'py, PyList>> {
        let records = self.with_pending_file_configuration(instance, |configuration| {
            configuration
                .record_data()
                .map(|records| records.to_vec())
                .map_err(to_py_err)
        })?;
        let result = PyList::empty(py);
        for record in records {
            result.append(PyBytes::new(py, &record))?;
        }
        Ok(result)
    }

    /// Set and return a pending File's effective octet growth cap before start().
    ///
    /// The cap is clamped by FileObject and does not truncate preloaded content.
    #[pyo3(signature = (instance, max_octets))]
    fn set_max_file_size(&self, instance: u32, max_octets: u64) -> PyResult<u64> {
        self.with_pending_file_configuration_mut(instance, |configuration| {
            Ok(configuration.set_max_file_size(max_octets))
        })
    }

    /// Set and return a pending File's effective record growth cap before start().
    ///
    /// The cap is clamped by FileObject and does not truncate preloaded records.
    #[pyo3(signature = (instance, max_records))]
    fn set_max_record_count(&self, instance: u32, max_records: u64) -> PyResult<u64> {
        self.with_pending_file_configuration_mut(instance, |configuration| {
            Ok(configuration.set_max_record_count(max_records))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_objects::binary::BinaryValueObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
    use bacnet_types::error::Error;
    use std::borrow::Cow;

    fn test_server() -> BACnetServer {
        BACnetServer {
            inner: Arc::new(Mutex::new(None)),
            device_instance: 1,
            device_name: "Test Device".into(),
            transport_type: "bip".into(),
            interface: "0.0.0.0".into(),
            port: 0,
            broadcast_address: "255.255.255.255".into(),
            sc_hub: None,
            sc_vmac: None,
            sc_ca_cert: None,
            sc_client_cert: None,
            sc_client_key: None,
            sc_heartbeat_interval_ms: None,
            sc_heartbeat_timeout_ms: None,
            ipv6_interface: None,
            serial_port: None,
            mstp_baud: 38_400,
            mstp_mac: 1,
            mstp_max_master: 127,
            mstp_max_info_frames: 1,
            dcc_password: None,
            reinit_password: None,
            started: Arc::new(AtomicBool::new(false)),
            pending_objects: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn file(instance: u32, name: &str, data: &[u8]) -> FileObject {
        let mut file = FileObject::new(instance, name, "application/octet-stream").unwrap();
        file.set_data(data.to_vec());
        file
    }

    fn py_message(py: Python<'_>, error: &PyErr) -> String {
        error
            .value(py)
            .str()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    struct NoConfigurationFile;

    impl BACnetObject for NoConfigurationFile {
        fn object_identifier(&self) -> ObjectIdentifier {
            ObjectIdentifier::new(ObjectType::FILE, 7).unwrap()
        }

        fn object_name(&self) -> &str {
            "NO-CONFIGURATION"
        }

        fn read_property(
            &self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            Err(Error::Encoding("not used by this test".into()))
        }

        fn write_property(
            &mut self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
            _value: PropertyValue,
            _priority: Option<u8>,
        ) -> Result<(), Error> {
            Err(Error::Encoding("not used by this test".into()))
        }

        fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
            Cow::Borrowed(&[])
        }
    }

    #[test]
    fn pending_lookup_uses_file_identity_and_reverse_replacement_order() {
        let server = test_server();
        server
            .push_pending(Box::new(file(4, "FIRST", b"first")))
            .unwrap();
        server
            .push_pending(Box::new(BinaryValueObject::new(4, "NON-FILE").unwrap()))
            .unwrap();
        server
            .push_pending(Box::new(file(4, "SECOND", b"second")))
            .unwrap();

        Python::attach(|py| {
            server
                .set_file_data(4, &PyBytes::new(py, b"effective"))
                .unwrap();
        });

        let guard = server.lock_pending().unwrap();
        assert_eq!(
            guard[0]
                .file_configuration_internal()
                .unwrap()
                .stream_data()
                .unwrap(),
            b"first"
        );
        assert_eq!(
            guard[2]
                .file_configuration_internal()
                .unwrap()
                .stream_data()
                .unwrap(),
            b"effective"
        );
    }

    #[test]
    fn missing_wrong_capability_invalid_method_and_mode_errors_are_atomic() {
        let server = test_server();
        server
            .push_pending(Box::new(BinaryValueObject::new(5, "NON-FILE").unwrap()))
            .unwrap();
        server
            .push_pending(Box::new(file(6, "FILE", b"kept")))
            .unwrap();
        server.push_pending(Box::new(NoConfigurationFile)).unwrap();

        Python::attach(|py| {
            let missing = server.get_file_data(py, 5).unwrap_err();
            assert!(missing.is_instance_of::<PyValueError>(py));
            assert_eq!(
                py_message(py, &missing),
                "no pending File object with instance 5"
            );

            let wrong_capability = server.get_file_data(py, 7).unwrap_err();
            assert!(wrong_capability.is_instance_of::<PyTypeError>(py));

            let invalid = server.set_file_access_method(6, "STREAM").unwrap_err();
            assert!(invalid.is_instance_of::<PyValueError>(py));
            assert_eq!(py_message(py, &invalid), INVALID_ACCESS_METHOD);
            assert_eq!(server.get_file_data(py, 6).unwrap().as_bytes(), b"kept");

            server.set_file_access_method(6, "record").unwrap();
            let records = PyList::new(py, [PyBytes::new(py, b"one")]).unwrap();
            server.set_file_records(6, &records).unwrap();
            let mismatch = server
                .set_file_data(6, &PyBytes::new(py, b"changed"))
                .unwrap_err();
            assert!(mismatch.is_instance_of::<crate::errors::BacnetProtocolError>(py));
            assert_eq!(
                mismatch
                    .value(py)
                    .getattr("error_class")
                    .unwrap()
                    .extract::<u32>()
                    .unwrap(),
                ErrorClass::SERVICES.to_raw() as u32
            );
            assert_eq!(
                mismatch
                    .value(py)
                    .getattr("error_code")
                    .unwrap()
                    .extract::<u32>()
                    .unwrap(),
                ErrorCode::INVALID_FILE_ACCESS_METHOD.to_raw() as u32
            );
            assert_eq!(
                server
                    .get_file_records(py, 6)
                    .unwrap()
                    .get_item(0)
                    .unwrap()
                    .cast::<PyBytes>()
                    .unwrap()
                    .as_bytes(),
                b"one"
            );
        });
    }

    #[test]
    fn started_and_drained_states_reject_without_mutation() {
        let started = test_server();
        started
            .push_pending(Box::new(file(8, "STARTED", b"kept")))
            .unwrap();
        started.started.store(true, Ordering::Release);
        Python::attach(|py| {
            let error = started
                .set_file_data(8, &PyBytes::new(py, b"changed"))
                .unwrap_err();
            assert!(error.is_instance_of::<PyRuntimeError>(py));
            assert_eq!(py_message(py, &error), STARTED_CONFIGURATION);
        });
        assert_eq!(
            started.lock_pending().unwrap()[0]
                .file_configuration_internal()
                .unwrap()
                .stream_data()
                .unwrap(),
            b"kept"
        );

        let drained = test_server();
        drained
            .push_pending(Box::new(file(9, "DRAINED", b"gone")))
            .unwrap();
        drained.lock_pending().unwrap().drain(..);
        Python::attach(|py| {
            let error = drained.get_file_data(py, 9).unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py));
            assert_eq!(
                py_message(py, &error),
                "no pending File object with instance 9"
            );
        });
    }

    #[test]
    fn cap_methods_return_the_file_objects_effective_values() {
        let server = test_server();
        server
            .push_pending(Box::new(file(10, "CAPPED", b"preloaded")))
            .unwrap();
        assert_eq!(
            server.set_max_file_size(10, u64::MAX).unwrap(),
            i32::MAX as u64
        );
        assert_eq!(
            server.set_max_record_count(10, u64::MAX).unwrap(),
            bacnet_objects::file::DEFAULT_MAX_RECORD_COUNT
        );
        assert_eq!(server.set_max_file_size(10, 2).unwrap(), 2);
        assert_eq!(
            server
                .with_pending_file_configuration(10, |configuration| {
                    Ok(configuration.stream_data().unwrap().to_vec())
                })
                .unwrap(),
            b"preloaded"
        );
    }
}
