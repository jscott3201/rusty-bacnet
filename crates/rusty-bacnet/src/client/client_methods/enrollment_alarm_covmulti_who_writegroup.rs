use super::super::*;

/// Python `(property, array_index, cov_increment, timestamped)` COV reference.
type PyCovReference = (PyPropertyIdentifier, Option<u32>, Option<f32>, bool);
/// Python `(object, [cov_reference, ...])` SubscribeCOVPropertyMultiple spec.
type PyCovSubscriptionSpec = (PyObjectIdentifier, Vec<PyCovReference>);

#[pymethods]
impl BACnetClient {
    // -----------------------------------------------------------------------
    // GetEnrollmentSummary
    // -----------------------------------------------------------------------

    /// Get enrollment summary from a remote device.
    ///
    /// `acknowledgment_filter` defaults to `AcknowledgmentFilter.ALL`.
    /// Returns a list of dicts with `object_id`, `event_type`, `event_state`, `priority`, `notification_class`.
    #[pyo3(signature = (address, acknowledgment_filter=PyAcknowledgmentFilter { inner: AcknowledgmentFilter::ALL }, event_state_filter=None, event_type_filter=None, min_priority=None, max_priority=None, notification_class_filter=None))]
    fn get_enrollment_summary<'py>(
        &self,
        py: Python<'py>,
        address: String,
        acknowledgment_filter: PyAcknowledgmentFilter,
        event_state_filter: Option<PyEnrollmentSummaryEventStateFilter>,
        event_type_filter: Option<PyEventType>,
        min_priority: Option<u8>,
        max_priority: Option<u8>,
        notification_class_filter: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let af = acknowledgment_filter.to_rust();
        let es = event_state_filter.map(|e| e.to_rust());
        let et = event_type_filter.map(|e| e.to_rust());
        let pf = match (min_priority, max_priority) {
            (Some(min), Some(max)) => Some(PriorityFilter {
                min_priority: min,
                max_priority: max,
            }),
            _ => None,
        };

        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            let req = GetEnrollmentSummaryRequest {
                acknowledgment_filter: af,
                enrollment_filter: None, // not exposed in Python API
                event_state_filter: es,
                event_type_filter: et,
                priority_filter: pf,
                notification_class_filter,
            };
            let mut buf = BytesMut::new();
            req.try_encode(&mut buf).map_err(to_py_err)?;
            let resp = c
                .confirmed_request(&mac, ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY, &buf)
                .await
                .map_err(to_py_err)?;
            let ack = GetEnrollmentSummaryAck::decode(&resp).map_err(to_py_err)?;
            crate::py_async::attach(|py| {
                let list = pyo3::types::PyList::empty(py);
                for entry in &ack.entries {
                    let dict = PyDict::new(py);
                    dict.set_item(
                        "object_id",
                        PyObjectIdentifier::from_rust(entry.object_identifier),
                    )?;
                    dict.set_item(
                        "event_type",
                        PyEventType {
                            inner: entry.event_type,
                        },
                    )?;
                    dict.set_item(
                        "event_state",
                        PyEventState {
                            inner: entry.event_state,
                        },
                    )?;
                    dict.set_item("priority", entry.priority)?;
                    dict.set_item("notification_class", entry.notification_class)?;
                    list.append(dict)?;
                }
                Ok(list.into_any().unbind())
            })
        })
    }

    // -----------------------------------------------------------------------
    // GetAlarmSummary
    // -----------------------------------------------------------------------

    /// Get alarm summary from a remote device (deprecated service).
    ///
    /// Returns a list of dicts with `object_id`, `alarm_state`, `acknowledged_transitions`.
    #[pyo3(signature = (address,))]
    fn get_alarm_summary<'py>(
        &self,
        py: Python<'py>,
        address: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            let resp = c
                .confirmed_request(&mac, ConfirmedServiceChoice::GET_ALARM_SUMMARY, &[])
                .await
                .map_err(to_py_err)?;
            let ack = GetAlarmSummaryAck::decode(&resp).map_err(to_py_err)?;
            crate::py_async::attach(|py| {
                let list = pyo3::types::PyList::empty(py);
                for entry in &ack.entries {
                    let dict = PyDict::new(py);
                    dict.set_item(
                        "object_id",
                        PyObjectIdentifier::from_rust(entry.object_identifier),
                    )?;
                    dict.set_item(
                        "alarm_state",
                        PyEventState {
                            inner: entry.alarm_state,
                        },
                    )?;
                    // Keep the raw bit-string shape: three defined bits, five unused.
                    let trans_dict = PyDict::new(py);
                    trans_dict.set_item("unused_bits", 5u8)?;
                    trans_dict.set_item(
                        "data",
                        PyBytes::new(py, &[entry.acknowledged_transitions.to_bacnet()]),
                    )?;
                    dict.set_item("acknowledged_transitions", trans_dict)?;
                    list.append(dict)?;
                }
                Ok(list.into_any().unbind())
            })
        })
    }

    // -----------------------------------------------------------------------
    // SubscribeCOVPropertyMultiple
    // -----------------------------------------------------------------------

    /// Subscribe to COV notifications for multiple properties on multiple objects.
    ///
    /// `specs` is a list of `(ObjectIdentifier, [(PropertyIdentifier, array_index, cov_increment, timestamped), ...])`.
    /// `cov_increment` is an optional float; `timestamped` is a bool.
    /// `issue_confirmed_notifications` is required, including for cancellations.
    /// For subscriptions and re-subscriptions, `lifetime` and `max_notification_delay` are both required.
    /// A whole-context cancellation uses an empty `specs` list and omits both timing fields.
    #[pyo3(signature = (address, subscriber_process_identifier, specs, issue_confirmed_notifications, max_notification_delay=None, lifetime=None))]
    fn subscribe_cov_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: String,
        subscriber_process_identifier: u32,
        specs: Vec<PyCovSubscriptionSpec>,
        issue_confirmed_notifications: bool,
        max_notification_delay: Option<u32>,
        lifetime: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let rust_specs: Vec<COVSubscriptionSpecification> = specs
            .into_iter()
            .map(|(oid, refs)| COVSubscriptionSpecification {
                monitored_object_identifier: oid.to_rust(),
                list_of_cov_references: refs
                    .into_iter()
                    .map(|(pid, idx, inc, ts)| COVReference {
                        monitored_property: bacnet_types::constructed::PropertyReference {
                            property_identifier: pid.to_rust(),
                            property_array_index: idx,
                        },
                        cov_increment: inc,
                        timestamped: ts,
                    })
                    .collect(),
            })
            .collect();

        let req = SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier,
            issue_confirmed_notifications,
            lifetime,
            max_notification_delay,
            list_of_cov_subscription_specifications: rust_specs,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            c.confirmed_request(
                &mac,
                ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
                &buf,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    // -----------------------------------------------------------------------
    // Who-Am-I
    // -----------------------------------------------------------------------

    /// Broadcast a Who-Am-I request announcing this device's identity.
    ///
    /// `vendor_id`, `model_name` and `serial_number` should match the sender's Device object
    /// properties (Clause 16.11.1). Raises `ValueError` if a string cannot be encoded.
    #[pyo3(signature = (vendor_id, model_name, serial_number))]
    fn who_am_i<'py>(
        &self,
        py: Python<'py>,
        vendor_id: u16,
        model_name: String,
        serial_number: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let req = WhoAmIRequest {
            vendor_id,
            model_name,
            serial_number,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let inner = self.inner.clone();
        let future = async move {
            let c = started(&inner).await?;
            c.broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_AM_I, &buf)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    // -----------------------------------------------------------------------
    // WriteGroup
    // -----------------------------------------------------------------------

    /// Send a WriteGroup request (unconfirmed).
    ///
    /// With an `address` the request goes to that one device. With
    /// `address=None` it is broadcast: on the local network, or with `network`
    /// on that remote network (1..65534), or on every network when `network`
    /// is 65535. Give `network` only with `address=None`.
    ///
    /// `group_number` is 1..4294967295 (group 0 is reserved) and `write_priority` is 1..16.
    /// `change_list` is a non-empty list of `(channel, override_priority_or_none, value)`
    /// tuples: `channel` is a channel number 0..65535, `override_priority_or_none` is 1..16 or
    /// `None`, and `value` is a `PropertyValue`, which the binding encodes, or a `bytes` or
    /// `bytearray` holding one encoded BACnetChannelValue (a single application-tagged
    /// primitive, or a context-0 lighting command, context-1 xy colour or context-2 colour
    /// command, #1474) with no extra wrapper tag (#1359). Raises
    /// `ValueError`, or `OverflowError` for integers that don't fit, for an argument outside
    /// those rules.
    #[pyo3(signature = (address, group_number, write_priority, change_list, inhibit_delay=None, *, network=None))]
    fn write_group<'py>(
        &self,
        py: Python<'py>,
        address: Option<String>,
        group_number: u32,
        write_priority: u8,
        change_list: Vec<(u16, Option<u8>, ChannelValueArg)>,
        inhibit_delay: Option<bool>,
        network: Option<u16>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let group_number = std::num::NonZeroU32::new(group_number).ok_or_else(|| {
            PyValueError::new_err("group_number must be 1-4294967295 (group 0 is reserved)")
        })?;
        let req = WriteGroupRequest {
            group_number,
            write_priority,
            change_list: change_list
                .into_iter()
                .map(|(channel, override_priority, value)| {
                    Ok(GroupChannelValue {
                        channel,
                        override_priority,
                        value: value.encoded()?,
                    })
                })
                .collect::<PyResult<_>>()?,
            inhibit_delay,
        };
        // The request's own check: each value is one BACnetChannelValue.
        req.encode(&mut BytesMut::new())
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let broadcast =
            write_group_broadcast(address.is_some(), network).map_err(PyValueError::new_err)?;

        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let future = async move {
            let destination = match (broadcast, address) {
                (Some(broadcast), _) => broadcast,
                (None, Some(address)) => WriteGroupDestination::Device(
                    bacnet_types::MacAddr::from_slice(&peers.parse(&address)?),
                ),
                (None, None) => unreachable!("no address means a broadcast"),
            };
            let c = started(&inner).await?;
            c.write_group(&destination, &req).await.map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}

/// One `write_group` change-list value as Python gives it: a `PropertyValue`,
/// encoded here, or the BACnetChannelValue already encoded in a `bytes` or
/// `bytearray`, as a lighting command usually is. Either way the request's
/// own check refuses anything that isn't one BACnetChannelValue. A list of
/// ints is neither, and raises TypeError.
#[derive(FromPyObject)]
enum ChannelValueArg {
    Value(PyPropertyValue),
    Encoded(pyo3::pybacked::PyBackedBytes),
}

impl ChannelValueArg {
    /// The value's octets, application-tagged as a WriteProperty carries them.
    fn encoded(self) -> PyResult<Vec<u8>> {
        match self {
            Self::Value(value) => {
                let mut octets = BytesMut::new();
                encode_property_value(&mut octets, &value.inner)
                    .map_err(|error| PyValueError::new_err(error.to_string()))?;
                Ok(octets.to_vec())
            }
            Self::Encoded(octets) => Ok(octets.to_vec()),
        }
    }
}

/// The broadcast `write_group`'s arguments ask for, or `None` for the one
/// device at the given address. Python's `address=None` is `has_address`
/// false; `network` 65535 means every network.
fn write_group_broadcast(
    has_address: bool,
    network: Option<u16>,
) -> Result<Option<WriteGroupDestination>, &'static str> {
    match (has_address, network) {
        (true, Some(_)) => Err("give an address or a network, not both"),
        (true, None) => Ok(None),
        (false, None) => Ok(Some(WriteGroupDestination::LocalBroadcast)),
        (false, Some(0)) => Err("network must be 1-65535"),
        (false, Some(u16::MAX)) => Ok(Some(WriteGroupDestination::GlobalBroadcast)),
        (false, Some(network)) => Ok(Some(WriteGroupDestination::RemoteBroadcast(network))),
    }
}

#[cfg(test)]
mod write_group_destination_tests {
    use super::*;

    /// Python's broadcast tests can't read what a broadcast puts on the link,
    /// so this pins which destination each argument form picks; the
    /// bacnet-client WriteGroup tests pin each destination's NPDU.
    #[test]
    fn write_group_address_and_network_pick_the_destination() {
        use WriteGroupDestination::{GlobalBroadcast, LocalBroadcast, RemoteBroadcast};
        assert_eq!(write_group_broadcast(true, None), Ok(None));
        assert_eq!(write_group_broadcast(false, None), Ok(Some(LocalBroadcast)));
        for network in [1, 5, 65534] {
            assert_eq!(
                write_group_broadcast(false, Some(network)),
                Ok(Some(RemoteBroadcast(network)))
            );
        }
        assert_eq!(
            write_group_broadcast(false, Some(65535)),
            Ok(Some(GlobalBroadcast))
        );
        assert!(write_group_broadcast(false, Some(0)).is_err());
        assert!(write_group_broadcast(true, Some(5)).is_err());
    }
}
