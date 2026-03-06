//! Java/Kotlin BACnetClient — UniFFI wrapper around the Rust BACnetClient.

use std::sync::Arc;

use bytes::BytesMut;
use tokio::sync::Mutex;

use bacnet_client::client::BACnetClient as RustClient;
use bacnet_encoding::primitives::{decode_application_value, encode_property_value as core_encode};
use bacnet_services::common::{BACnetPropertyValue as SvcPropertyValue, PropertyReference};
use bacnet_services::file::{FileAccessMethod, FileWriteAccessMethod};
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_services::private_transfer::PrivateTransferRequest;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::mstp::NoSerial;
use bacnet_types::enums::{
    ConfirmedServiceChoice, EnableDisable, LifeSafetyOperation, ObjectType, PropertyIdentifier,
    ReinitializedState,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::errors::BacnetError;
use crate::transport::{build_transport, parse_address};
use crate::types::*;

type ClientInner = Arc<Mutex<Option<Arc<RustClient<AnyTransport<NoSerial>>>>>>;

/// BACnet client for Java/Kotlin consumers.
#[derive(uniffi::Object)]
pub struct BacnetClient {
    inner: ClientInner,
    config: TransportConfig,
    apdu_timeout_ms: u64,
}

#[uniffi::export(async_runtime = "tokio")]
impl BacnetClient {
    /// Create a new BACnet client with the given transport configuration.
    #[uniffi::constructor]
    pub fn new(config: TransportConfig, apdu_timeout_ms: Option<u64>) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Mutex::new(None)),
            config,
            apdu_timeout_ms: apdu_timeout_ms.unwrap_or(6000),
        })
    }

    /// Connect to the BACnet network. Must be called before any service methods.
    pub async fn connect(&self) -> Result<(), BacnetError> {
        let transport = build_transport(&self.config)?;
        let client = RustClient::generic_builder()
            .transport(transport)
            .apdu_timeout_ms(self.apdu_timeout_ms)
            .build()
            .await
            .map_err(BacnetError::from)?;
        *self.inner.lock().await = Some(Arc::new(client));
        Ok(())
    }

    /// Disconnect and release resources.
    pub async fn stop(&self) -> Result<(), BacnetError> {
        let mut guard = self.inner.lock().await;
        if let Some(client) = guard.take() {
            // stop() requires &mut self, so we need to unwrap the Arc
            if let Ok(mut client) = Arc::try_unwrap(client) {
                client.stop().await.map_err(BacnetError::from)?;
            }
            // If Arc has other refs, just drop it
        }
        Ok(())
    }

    /// Read a single property from a remote device.
    pub async fn read_property(
        &self,
        address: String,
        object_type: u32,
        object_instance: u32,
        property_id: u32,
        array_index: Option<u32>,
    ) -> Result<BacnetPropertyValue, BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), object_instance)
            .map_err(BacnetError::from)?;
        let pid = PropertyIdentifier::from_raw(property_id);

        let ack = client
            .read_property(&mac, oid, pid, array_index)
            .await
            .map_err(BacnetError::from)?;

        decode_property_value(&ack.property_value)
    }

    /// Write a single property on a remote device.
    #[allow(clippy::too_many_arguments)]
    pub async fn write_property(
        &self,
        address: String,
        object_type: u32,
        object_instance: u32,
        property_id: u32,
        value: BacnetPropertyValue,
        priority: Option<u8>,
        array_index: Option<u32>,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), object_instance)
            .map_err(BacnetError::from)?;
        let pid = PropertyIdentifier::from_raw(property_id);
        let encoded = encode_property_value(&value)?;

        client
            .write_property(&mac, oid, pid, array_index, encoded, priority)
            .await
            .map_err(BacnetError::from)?;

        Ok(())
    }

    /// Read multiple properties from one or more objects.
    pub async fn read_property_multiple(
        &self,
        address: String,
        specs: Vec<ReadAccessSpec>,
    ) -> Result<Vec<ObjectReadResult>, BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;

        let rust_specs: Result<Vec<_>, BacnetError> = specs
            .into_iter()
            .map(|s| {
                let oid = ObjectIdentifier::new(ObjectType::from_raw(s.object_type), s.instance)
                    .map_err(BacnetError::from)?;
                Ok(bacnet_services::rpm::ReadAccessSpecification {
                    object_identifier: oid,
                    list_of_property_references: s
                        .properties
                        .into_iter()
                        .map(|p| PropertyReference {
                            property_identifier: PropertyIdentifier::from_raw(p.property_id),
                            property_array_index: p.array_index,
                        })
                        .collect(),
                })
            })
            .collect();

        let ack = client
            .read_property_multiple(&mac, rust_specs?)
            .await
            .map_err(BacnetError::from)?;

        let mut results = Vec::new();
        for obj_result in ack.list_of_read_access_results {
            let mut read_results = Vec::new();
            for elem in obj_result.list_of_results {
                let (value, error_class, error_code) = if let Some(ref data) = elem.property_value {
                    (Some(decode_property_value(data)?), None, None)
                } else if let Some((ec, ecd)) = elem.error {
                    (None, Some(ec.to_raw()), Some(ecd.to_raw()))
                } else {
                    (Some(BacnetPropertyValue::Null), None, None)
                };
                read_results.push(ReadResult {
                    property_id: elem.property_identifier.to_raw(),
                    array_index: elem.property_array_index,
                    value,
                    error_class,
                    error_code,
                });
            }
            results.push(ObjectReadResult {
                object_type: obj_result.object_identifier.object_type().to_raw(),
                instance: obj_result.object_identifier.instance_number(),
                results: read_results,
            });
        }
        Ok(results)
    }

    /// Write multiple properties to one or more objects.
    pub async fn write_property_multiple(
        &self,
        address: String,
        specs: Vec<WriteAccessSpec>,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;

        let rust_specs: Result<Vec<_>, BacnetError> = specs
            .into_iter()
            .map(|s| {
                let oid = ObjectIdentifier::new(ObjectType::from_raw(s.object_type), s.instance)
                    .map_err(BacnetError::from)?;
                let props: Result<Vec<_>, BacnetError> = s
                    .properties
                    .into_iter()
                    .map(|p| {
                        let encoded = encode_property_value(&p.value)?;
                        Ok(SvcPropertyValue {
                            property_identifier: PropertyIdentifier::from_raw(p.property_id),
                            property_array_index: p.array_index,
                            value: encoded,
                            priority: p.priority,
                        })
                    })
                    .collect();
                Ok(bacnet_services::wpm::WriteAccessSpecification {
                    object_identifier: oid,
                    list_of_properties: props?,
                })
            })
            .collect();

        client
            .write_property_multiple(&mac, rust_specs?)
            .await
            .map_err(BacnetError::from)?;

        Ok(())
    }

    /// Send a WhoIs broadcast to discover devices.
    pub async fn who_is(
        &self,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        client
            .who_is(low_limit, high_limit)
            .await
            .map_err(BacnetError::from)?;
        Ok(())
    }

    /// Get all discovered devices.
    pub async fn discovered_devices(&self) -> Result<Vec<DiscoveredDevice>, BacnetError> {
        let client = self.get_client().await?;
        let devices = client.discovered_devices().await;
        Ok(devices
            .into_iter()
            .map(|d| DiscoveredDevice {
                object_type: d.object_identifier.object_type().to_raw(),
                instance: d.object_identifier.instance_number(),
                mac_address: d.mac_address.to_vec(),
                max_apdu_length: d.max_apdu_length,
                segmentation: d.segmentation_supported.to_raw(),
                vendor_id: d.vendor_id,
                seconds_since_seen: d.last_seen.elapsed().as_secs_f64(),
            })
            .collect())
    }

    /// Get a specific device by instance number.
    pub async fn get_device(&self, instance: u32) -> Result<Option<DiscoveredDevice>, BacnetError> {
        let client = self.get_client().await?;
        let device = client.get_device(instance).await;
        Ok(device.map(|d| DiscoveredDevice {
            object_type: d.object_identifier.object_type().to_raw(),
            instance: d.object_identifier.instance_number(),
            mac_address: d.mac_address.to_vec(),
            max_apdu_length: d.max_apdu_length,
            segmentation: d.segmentation_supported.to_raw(),
            vendor_id: d.vendor_id,
            seconds_since_seen: d.last_seen.elapsed().as_secs_f64(),
        }))
    }

    /// Clear the discovered device table.
    pub async fn clear_devices(&self) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        client.clear_devices().await;
        Ok(())
    }

    /// Subscribe to COV notifications for an object.
    pub async fn subscribe_cov(
        &self,
        address: String,
        subscriber_process_id: u32,
        object_type: u32,
        object_instance: u32,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), object_instance)
            .map_err(BacnetError::from)?;
        client
            .subscribe_cov(&mac, subscriber_process_id, oid, confirmed, lifetime)
            .await
            .map_err(BacnetError::from)?;
        Ok(())
    }

    /// Unsubscribe from COV notifications.
    pub async fn unsubscribe_cov(
        &self,
        address: String,
        subscriber_process_id: u32,
        object_type: u32,
        object_instance: u32,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), object_instance)
            .map_err(BacnetError::from)?;
        client
            .unsubscribe_cov(&mac, subscriber_process_id, oid)
            .await
            .map_err(BacnetError::from)?;
        Ok(())
    }

    /// Get an async iterator for incoming COV notifications.
    /// Each call to `CovNotificationStream.next()` awaits the next notification.
    pub async fn cov_notifications(&self) -> Result<Arc<CovNotificationStream>, BacnetError> {
        let client = self.get_client().await?;
        let rx = client.cov_notifications();
        Ok(Arc::new(CovNotificationStream {
            rx: tokio::sync::Mutex::new(rx),
        }))
    }

    /// Create a new object on a remote device.
    pub async fn create_object(
        &self,
        address: String,
        object_type: u32,
        object_instance: Option<u32>,
    ) -> Result<Vec<u8>, BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let specifier = if let Some(inst) = object_instance {
            let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), inst)
                .map_err(BacnetError::from)?;
            ObjectSpecifier::Identifier(oid)
        } else {
            ObjectSpecifier::Type(ObjectType::from_raw(object_type))
        };
        let result = client
            .create_object(&mac, specifier, vec![])
            .await
            .map_err(BacnetError::from)?;
        Ok(result.to_vec())
    }

    /// Delete an object on a remote device.
    pub async fn delete_object(
        &self,
        address: String,
        object_type: u32,
        object_instance: u32,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), object_instance)
            .map_err(BacnetError::from)?;
        client
            .delete_object(&mac, oid)
            .await
            .map_err(BacnetError::from)?;
        Ok(())
    }

    /// Control device communication state.
    pub async fn device_communication_control(
        &self,
        address: String,
        enable_disable: u32,
        time_duration: Option<u16>,
        password: Option<String>,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let ed = EnableDisable::from_raw(enable_disable);
        client
            .device_communication_control(&mac, ed, time_duration, password)
            .await
            .map_err(BacnetError::from)?;
        Ok(())
    }

    /// Reinitialize a remote device.
    pub async fn reinitialize_device(
        &self,
        address: String,
        reinitialized_state: u32,
        password: Option<String>,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let state = ReinitializedState::from_raw(reinitialized_state);
        client
            .reinitialize_device(&mac, state, password)
            .await
            .map_err(BacnetError::from)?;
        Ok(())
    }

    /// Get event information from a remote device.
    pub async fn get_event_information(
        &self,
        address: String,
        last_object_type: Option<u32>,
        last_object_instance: Option<u32>,
    ) -> Result<Vec<u8>, BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let last_oid = match (last_object_type, last_object_instance) {
            (Some(t), Some(i)) => {
                Some(ObjectIdentifier::new(ObjectType::from_raw(t), i).map_err(BacnetError::from)?)
            }
            _ => None,
        };
        let result = client
            .get_event_information(&mac, last_oid)
            .await
            .map_err(BacnetError::from)?;
        Ok(result.to_vec())
    }

    /// Acknowledge an alarm on a remote device.
    pub async fn acknowledge_alarm(
        &self,
        address: String,
        acknowledging_process_id: u32,
        event_object_type: u32,
        event_object_instance: u32,
        event_state_acknowledged: u32,
        acknowledgment_source: String,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let oid = ObjectIdentifier::new(
            ObjectType::from_raw(event_object_type),
            event_object_instance,
        )
        .map_err(BacnetError::from)?;
        client
            .acknowledge_alarm(
                &mac,
                acknowledging_process_id,
                oid,
                event_state_acknowledged,
                &acknowledgment_source,
            )
            .await
            .map_err(BacnetError::from)?;
        Ok(())
    }

    /// Read a range of values from a list or log.
    pub async fn read_range(
        &self,
        address: String,
        object_type: u32,
        object_instance: u32,
        property_id: u32,
        array_index: Option<u32>,
    ) -> Result<Vec<u8>, BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), object_instance)
            .map_err(BacnetError::from)?;
        let pid = PropertyIdentifier::from_raw(property_id);
        let ack = client
            .read_range(&mac, oid, pid, array_index, None)
            .await
            .map_err(BacnetError::from)?;
        // Encode the ACK back to bytes for the Java consumer
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        Ok(buf.to_vec())
    }

    /// Read a file from a remote device.
    pub async fn atomic_read_file(
        &self,
        address: String,
        file_object_type: u32,
        file_object_instance: u32,
        access_method: String,
        start_position: i32,
        requested_count: u32,
    ) -> Result<Vec<u8>, BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let oid =
            ObjectIdentifier::new(ObjectType::from_raw(file_object_type), file_object_instance)
                .map_err(BacnetError::from)?;
        let access = match access_method.as_str() {
            "stream" => FileAccessMethod::Stream {
                file_start_position: start_position,
                requested_octet_count: requested_count,
            },
            "record" => FileAccessMethod::Record {
                file_start_record: start_position,
                requested_record_count: requested_count,
            },
            _ => {
                return Err(BacnetError::InvalidArgument {
                    msg: format!(
                        "invalid access method: {access_method}, use 'stream' or 'record'"
                    ),
                });
            }
        };
        let result = client
            .atomic_read_file(&mac, oid, access)
            .await
            .map_err(BacnetError::from)?;
        Ok(result.to_vec())
    }

    /// Write to a file on a remote device.
    pub async fn atomic_write_file(
        &self,
        address: String,
        file_object_type: u32,
        file_object_instance: u32,
        access_method: String,
        start_position: i32,
        file_data: Vec<u8>,
    ) -> Result<Vec<u8>, BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let oid =
            ObjectIdentifier::new(ObjectType::from_raw(file_object_type), file_object_instance)
                .map_err(BacnetError::from)?;
        let access = match access_method.as_str() {
            "stream" => FileWriteAccessMethod::Stream {
                file_start_position: start_position,
                file_data,
            },
            "record" => FileWriteAccessMethod::Record {
                file_start_record: start_position,
                record_count: 1,
                file_record_data: vec![file_data],
            },
            _ => {
                return Err(BacnetError::InvalidArgument {
                    msg: format!(
                        "invalid access method: {access_method}, use 'stream' or 'record'"
                    ),
                });
            }
        };
        let result = client
            .atomic_write_file(&mac, oid, access)
            .await
            .map_err(BacnetError::from)?;
        Ok(result.to_vec())
    }

    /// Add elements to a list property.
    pub async fn add_list_element(
        &self,
        address: String,
        object_type: u32,
        object_instance: u32,
        property_id: u32,
        array_index: Option<u32>,
        list_of_elements: Vec<u8>,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), object_instance)
            .map_err(BacnetError::from)?;
        let pid = PropertyIdentifier::from_raw(property_id);
        client
            .add_list_element(&mac, oid, pid, array_index, list_of_elements)
            .await
            .map_err(BacnetError::from)?;
        Ok(())
    }

    /// Remove elements from a list property.
    pub async fn remove_list_element(
        &self,
        address: String,
        object_type: u32,
        object_instance: u32,
        property_id: u32,
        array_index: Option<u32>,
        list_of_elements: Vec<u8>,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), object_instance)
            .map_err(BacnetError::from)?;
        let pid = PropertyIdentifier::from_raw(property_id);
        client
            .remove_list_element(&mac, oid, pid, array_index, list_of_elements)
            .await
            .map_err(BacnetError::from)?;
        Ok(())
    }

    /// Send a confirmed private transfer request.
    pub async fn confirmed_private_transfer(
        &self,
        address: String,
        vendor_id: u32,
        service_number: u32,
        service_parameters: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let mut buf = BytesMut::new();
        PrivateTransferRequest {
            vendor_id,
            service_number,
            service_parameters,
        }
        .encode(&mut buf);
        let result = client
            .confirmed_request(
                &mac,
                ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER,
                &buf,
            )
            .await
            .map_err(BacnetError::from)?;
        Ok(result.to_vec())
    }

    /// Send a life safety operation.
    pub async fn life_safety_operation(
        &self,
        address: String,
        requesting_process_id: u32,
        requesting_source: String,
        operation: u32,
        object_type: Option<u32>,
        object_instance: Option<u32>,
    ) -> Result<(), BacnetError> {
        let client = self.get_client().await?;
        let mac = parse_address(&address)?;
        let obj_id = match (object_type, object_instance) {
            (Some(t), Some(i)) => {
                Some(ObjectIdentifier::new(ObjectType::from_raw(t), i).map_err(BacnetError::from)?)
            }
            _ => None,
        };
        let mut buf = BytesMut::new();
        bacnet_services::life_safety::LifeSafetyOperationRequest {
            requesting_process_identifier: requesting_process_id,
            requesting_source,
            request: LifeSafetyOperation::from_raw(operation),
            object_identifier: obj_id,
        }
        .encode(&mut buf)?;
        let _ = client
            .confirmed_request(&mac, ConfirmedServiceChoice::LIFE_SAFETY_OPERATION, &buf)
            .await
            .map_err(BacnetError::from)?;
        Ok(())
    }
}

// ---- Internal helpers ----

impl BacnetClient {
    async fn get_client(&self) -> Result<Arc<RustClient<AnyTransport<NoSerial>>>, BacnetError> {
        let guard = self.inner.lock().await;
        guard.clone().ok_or(BacnetError::NotStarted)
    }
}

// ---- COV notification stream ----

/// Async iterator for incoming COV notifications.
#[derive(uniffi::Object)]
pub struct CovNotificationStream {
    rx: tokio::sync::Mutex<
        tokio::sync::broadcast::Receiver<bacnet_services::cov::COVNotificationRequest>,
    >,
}

#[uniffi::export(async_runtime = "tokio")]
impl CovNotificationStream {
    /// Await the next COV notification. Returns None if the channel is closed.
    pub async fn next(&self) -> Option<CovNotification> {
        use tokio::sync::broadcast::error::RecvError;
        let mut rx = self.rx.lock().await;
        loop {
            match rx.recv().await {
                Ok(notif) => {
                    let values: Vec<CovValue> = notif
                        .list_of_values
                        .iter()
                        .filter_map(|v| {
                            let pv = decode_application_value(&v.value, 0).ok()?;
                            Some(CovValue {
                                property_id: v.property_identifier.to_raw(),
                                array_index: v.property_array_index,
                                value: property_value_to_java(&pv.0),
                            })
                        })
                        .collect();
                    return Some(CovNotification {
                        process_identifier: notif.subscriber_process_identifier,
                        device_instance: notif.initiating_device_identifier.instance_number(),
                        object_type: notif.monitored_object_identifier.object_type().to_raw(),
                        object_instance: notif.monitored_object_identifier.instance_number(),
                        time_remaining: notif.time_remaining,
                        values,
                    });
                }
                Err(RecvError::Lagged(n)) => {
                    eprintln!("COV notification stream lagged, skipped {n} messages");
                    continue;
                }
                Err(RecvError::Closed) => return None,
            }
        }
    }
}

/// Decode property value bytes from a ReadPropertyACK into our enum.
pub(crate) fn decode_property_value(data: &[u8]) -> Result<BacnetPropertyValue, BacnetError> {
    if data.is_empty() {
        return Ok(BacnetPropertyValue::Null);
    }

    let (pv, _) = decode_application_value(data, 0).map_err(BacnetError::from)?;
    Ok(property_value_to_java(&pv))
}

/// Convert a core PropertyValue to our UniFFI-compatible enum.
pub(crate) fn property_value_to_java(pv: &PropertyValue) -> BacnetPropertyValue {
    match pv {
        PropertyValue::Null => BacnetPropertyValue::Null,
        PropertyValue::Boolean(v) => BacnetPropertyValue::Boolean { value: *v },
        PropertyValue::Unsigned(v) => BacnetPropertyValue::Unsigned { value: *v },
        PropertyValue::Signed(v) => BacnetPropertyValue::Signed { value: *v as i64 },
        PropertyValue::Real(v) => BacnetPropertyValue::Real { value: *v },
        PropertyValue::Double(v) => BacnetPropertyValue::Double { value: *v },
        PropertyValue::CharacterString(v) => {
            BacnetPropertyValue::CharacterString { value: v.clone() }
        }
        PropertyValue::OctetString(v) => BacnetPropertyValue::OctetString { value: v.clone() },
        PropertyValue::BitString { data, .. } => BacnetPropertyValue::BitString {
            value: data.clone(),
        },
        PropertyValue::Enumerated(v) => BacnetPropertyValue::Enumerated { value: *v },
        PropertyValue::Date(d) => BacnetPropertyValue::Date {
            year: d.year,
            month: d.month,
            day: d.day,
            day_of_week: d.day_of_week,
        },
        PropertyValue::Time(t) => BacnetPropertyValue::Time {
            hour: t.hour,
            minute: t.minute,
            second: t.second,
            hundredths: t.hundredths,
        },
        PropertyValue::ObjectIdentifier(oid) => BacnetPropertyValue::ObjectId {
            object_type: oid.object_type().to_raw(),
            instance: oid.instance_number(),
        },
        PropertyValue::List(items) => {
            // Return first item or null for lists
            items
                .first()
                .map_or(BacnetPropertyValue::Null, property_value_to_java)
        }
    }
}

/// Encode a BacnetPropertyValue into application-tagged bytes.
pub(crate) fn encode_property_value(value: &BacnetPropertyValue) -> Result<Vec<u8>, BacnetError> {
    let pv = java_to_property_value(value)?;
    let mut buf = BytesMut::new();
    core_encode(&mut buf, &pv).map_err(BacnetError::from)?;
    Ok(buf.to_vec())
}

/// Convert our UniFFI enum to a core PropertyValue.
pub(crate) fn java_to_property_value(
    value: &BacnetPropertyValue,
) -> Result<PropertyValue, BacnetError> {
    Ok(match value {
        BacnetPropertyValue::Null => PropertyValue::Null,
        BacnetPropertyValue::Boolean { value } => PropertyValue::Boolean(*value),
        BacnetPropertyValue::Unsigned { value } => PropertyValue::Unsigned(*value),
        BacnetPropertyValue::Signed { value } => PropertyValue::Signed(*value as i32),
        BacnetPropertyValue::Real { value } => PropertyValue::Real(*value),
        BacnetPropertyValue::Double { value } => PropertyValue::Double(*value),
        BacnetPropertyValue::CharacterString { value } => {
            PropertyValue::CharacterString(value.clone())
        }
        BacnetPropertyValue::OctetString { value } => PropertyValue::OctetString(value.clone()),
        BacnetPropertyValue::BitString { value } => PropertyValue::BitString {
            unused_bits: 0,
            data: value.clone(),
        },
        BacnetPropertyValue::Enumerated { value } => PropertyValue::Enumerated(*value),
        BacnetPropertyValue::ObjectId {
            object_type,
            instance,
        } => {
            let oid = ObjectIdentifier::new(ObjectType::from_raw(*object_type), *instance)
                .map_err(BacnetError::from)?;
            PropertyValue::ObjectIdentifier(oid)
        }
        BacnetPropertyValue::Date {
            year,
            month,
            day,
            day_of_week,
        } => PropertyValue::Date(bacnet_types::primitives::Date {
            year: *year,
            month: *month,
            day: *day,
            day_of_week: *day_of_week,
        }),
        BacnetPropertyValue::Time {
            hour,
            minute,
            second,
            hundredths,
        } => PropertyValue::Time(bacnet_types::primitives::Time {
            hour: *hour,
            minute: *minute,
            second: *second,
            hundredths: *hundredths,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_construction() {
        let cfg = TransportConfig::Bip {
            address: "0.0.0.0".into(),
            port: 47808,
            broadcast_address: "255.255.255.255".into(),
        };
        let _client = BacnetClient::new(cfg, None);
    }

    #[test]
    fn test_encode_decode_null() {
        let encoded = encode_property_value(&BacnetPropertyValue::Null).unwrap();
        let decoded = decode_property_value(&encoded).unwrap();
        assert!(matches!(decoded, BacnetPropertyValue::Null));
    }

    #[test]
    fn test_encode_decode_unsigned() {
        let encoded = encode_property_value(&BacnetPropertyValue::Unsigned { value: 42 }).unwrap();
        let decoded = decode_property_value(&encoded).unwrap();
        match decoded {
            BacnetPropertyValue::Unsigned { value } => assert_eq!(value, 42),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_encode_decode_real() {
        let encoded = encode_property_value(&BacnetPropertyValue::Real { value: 72.5 }).unwrap();
        let decoded = decode_property_value(&encoded).unwrap();
        match decoded {
            BacnetPropertyValue::Real { value } => assert!((value - 72.5).abs() < f32::EPSILON),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_encode_decode_string() {
        let encoded = encode_property_value(&BacnetPropertyValue::CharacterString {
            value: "hello".into(),
        })
        .unwrap();
        let decoded = decode_property_value(&encoded).unwrap();
        match decoded {
            BacnetPropertyValue::CharacterString { value } => assert_eq!(value, "hello"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_encode_decode_boolean() {
        let encoded = encode_property_value(&BacnetPropertyValue::Boolean { value: true }).unwrap();
        let decoded = decode_property_value(&encoded).unwrap();
        match decoded {
            BacnetPropertyValue::Boolean { value } => assert!(value),
            _ => panic!("wrong variant"),
        }
    }
}
