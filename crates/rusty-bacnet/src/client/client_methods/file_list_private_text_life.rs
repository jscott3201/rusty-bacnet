use super::super::*;

#[pymethods]
impl BACnetClient {
    // -----------------------------------------------------------------------
    // File services
    // -----------------------------------------------------------------------

    /// Read from a file object (stream or record access).
    ///
    /// `access_method` is `"stream"` or `"record"`.
    #[pyo3(signature = (address, file_identifier, access_method, start_position=0, requested_octet_count=0, start_record=0, requested_record_count=0))]
    fn atomic_read_file<'py>(
        &self,
        py: Python<'py>,
        address: String,
        file_identifier: PyObjectIdentifier,
        access_method: String,
        start_position: i32,
        requested_octet_count: u32,
        start_record: i32,
        requested_record_count: u32,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let fid = file_identifier.to_rust();

        let access = match access_method.as_str() {
            "stream" => FileAccessMethod::Stream {
                file_start_position: start_position,
                requested_octet_count,
            },
            "record" => FileAccessMethod::Record {
                file_start_record: start_record,
                requested_record_count,
            },
            other => {
                return Err(PyValueError::new_err(format!(
                    "access_method must be 'stream' or 'record', got '{other}'"
                )));
            }
        };

        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            let raw = c
                .atomic_read_file(&mac, fid, access)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| Ok(PyBytes::new(py, &raw).into_any().unbind()))
        })
    }

    /// Write to a file object (stream or record access).
    ///
    /// `access_method` is `"stream"` or `"record"`.
    #[pyo3(signature = (address, file_identifier, access_method, start_position=0, file_data=vec![], start_record=0, record_count=0, file_record_data=None))]
    fn atomic_write_file<'py>(
        &self,
        py: Python<'py>,
        address: String,
        file_identifier: PyObjectIdentifier,
        access_method: String,
        start_position: i32,
        file_data: Vec<u8>,
        start_record: i32,
        record_count: u32,
        file_record_data: Option<Vec<Vec<u8>>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let fid = file_identifier.to_rust();

        let access = match access_method.as_str() {
            "stream" => FileWriteAccessMethod::Stream {
                file_start_position: start_position,
                file_data,
            },
            "record" => FileWriteAccessMethod::Record {
                file_start_record: start_record,
                record_count,
                file_record_data: file_record_data.unwrap_or_default(),
            },
            other => {
                return Err(PyValueError::new_err(format!(
                    "access_method must be 'stream' or 'record', got '{other}'"
                )));
            }
        };

        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            let raw = c
                .atomic_write_file(&mac, fid, access)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| Ok(PyBytes::new(py, &raw).into_any().unbind()))
        })
    }

    // -----------------------------------------------------------------------
    // List manipulation
    // -----------------------------------------------------------------------

    /// Add elements to a list property.
    #[pyo3(signature = (address, object_id, property_id, list_of_elements, array_index=None))]
    fn add_list_element<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        list_of_elements: Vec<u8>,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let request = bacnet_services::list_manipulation::ListElementRequest {
            object_identifier: object_id.to_rust(),
            property_identifier: property_id.to_rust(),
            property_array_index: array_index,
            list_of_elements,
        };
        request
            .validate()
            .map_err(|error| PyValueError::new_err(error.to_string()))?;

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            c.add_list_element(
                &mac,
                request.object_identifier,
                request.property_identifier,
                request.property_array_index,
                request.list_of_elements,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Remove elements from a list property.
    #[pyo3(signature = (address, object_id, property_id, list_of_elements, array_index=None))]
    fn remove_list_element<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        list_of_elements: Vec<u8>,
        array_index: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let request = bacnet_services::list_manipulation::ListElementRequest {
            object_identifier: object_id.to_rust(),
            property_identifier: property_id.to_rust(),
            property_array_index: array_index,
            list_of_elements,
        };
        request
            .validate()
            .map_err(|error| PyValueError::new_err(error.to_string()))?;

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            c.remove_list_element(
                &mac,
                request.object_identifier,
                request.property_identifier,
                request.property_array_index,
                request.list_of_elements,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    // -----------------------------------------------------------------------
    // PrivateTransfer
    // -----------------------------------------------------------------------

    /// Send a ConfirmedPrivateTransfer request.
    ///
    /// Returns a dict with `vendor_id`, `service_number`, and optional `result_block` (bytes).
    #[pyo3(signature = (address, vendor_id, service_number, service_parameters=None))]
    fn confirmed_private_transfer<'py>(
        &self,
        py: Python<'py>,
        address: String,
        vendor_id: u32,
        service_number: u32,
        service_parameters: Option<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            let req = PrivateTransferRequest {
                vendor_id,
                service_number,
                service_parameters,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            let resp = c
                .confirmed_request(
                    &mac,
                    ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER,
                    &buf,
                )
                .await
                .map_err(to_py_err)?;
            let ack = PrivateTransferAck::decode(&resp).map_err(to_py_err)?;
            crate::py_async::attach(|py| {
                let dict = PyDict::new(py);
                dict.set_item("vendor_id", ack.vendor_id)?;
                dict.set_item("service_number", ack.service_number)?;
                match ack.result_block {
                    Some(ref data) => {
                        dict.set_item("result_block", PyBytes::new(py, data))?;
                    }
                    None => {
                        dict.set_item("result_block", py.None())?;
                    }
                }
                Ok(dict.into_any().unbind())
            })
        })
    }

    /// Send an UnconfirmedPrivateTransfer request.
    #[pyo3(signature = (address, vendor_id, service_number, service_parameters=None))]
    fn unconfirmed_private_transfer<'py>(
        &self,
        py: Python<'py>,
        address: String,
        vendor_id: u32,
        service_number: u32,
        service_parameters: Option<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            let req = PrivateTransferRequest {
                vendor_id,
                service_number,
                service_parameters,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            c.unconfirmed_request(
                &mac,
                UnconfirmedServiceChoice::UNCONFIRMED_PRIVATE_TRANSFER,
                &buf,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    // -----------------------------------------------------------------------
    // TextMessage
    // -----------------------------------------------------------------------

    /// Send a ConfirmedTextMessage request.
    ///
    /// `message_class_type` is `"numeric"` or `"text"` (or None for no class).
    /// `message_class_value` is the numeric value or text string.
    #[pyo3(signature = (address, source_device, message_priority, message, message_class_type=None, message_class_value=None))]
    fn confirmed_text_message<'py>(
        &self,
        py: Python<'py>,
        address: String,
        source_device: PyObjectIdentifier,
        message_priority: PyMessagePriority,
        message: String,
        message_class_type: Option<String>,
        message_class_value: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let src = source_device.to_rust();
        let priority = message_priority.to_rust();
        let mc = build_message_class(message_class_type, message_class_value)?;

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            let req = TextMessageRequest {
                source_device: src,
                message_class: mc,
                message_priority: priority,
                message,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf).map_err(to_py_err)?;
            c.confirmed_request(&mac, ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE, &buf)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Send an UnconfirmedTextMessage request.
    #[pyo3(signature = (address, source_device, message_priority, message, message_class_type=None, message_class_value=None))]
    fn unconfirmed_text_message<'py>(
        &self,
        py: Python<'py>,
        address: String,
        source_device: PyObjectIdentifier,
        message_priority: PyMessagePriority,
        message: String,
        message_class_type: Option<String>,
        message_class_value: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let src = source_device.to_rust();
        let priority = message_priority.to_rust();
        let mc = build_message_class(message_class_type, message_class_value)?;

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            let req = TextMessageRequest {
                source_device: src,
                message_class: mc,
                message_priority: priority,
                message,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf).map_err(to_py_err)?;
            c.unconfirmed_request(
                &mac,
                UnconfirmedServiceChoice::UNCONFIRMED_TEXT_MESSAGE,
                &buf,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    // -----------------------------------------------------------------------
    // LifeSafetyOperation
    // -----------------------------------------------------------------------

    /// Send a LifeSafetyOperation request.
    #[pyo3(signature = (address, requesting_process_identifier, requesting_source, operation, object_identifier=None))]
    fn life_safety_operation<'py>(
        &self,
        py: Python<'py>,
        address: String,
        requesting_process_identifier: u32,
        requesting_source: String,
        operation: PyLifeSafetyOperation,
        object_identifier: Option<PyObjectIdentifier>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let op = operation.to_rust();
        let oid = object_identifier.map(|o| o.to_rust());

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            let req = LifeSafetyOperationRequest {
                requesting_process_identifier,
                requesting_source,
                request: op,
                object_identifier: oid,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf).map_err(to_py_err)?;
            c.confirmed_request(&mac, ConfirmedServiceChoice::LIFE_SAFETY_OPERATION, &buf)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}
