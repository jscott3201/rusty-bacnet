use std::num::NonZeroUsize;

use super::service::{Encoded, device_target};
use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Read a property from the station `to` (a MAC on this link, an
    /// address or a [`Station`]; see [`Self::confirmed_request`]).
    ///
    /// The ACK must match the requested object, property and array index.
    /// Device/Network Port wildcard requests accept only a same-type concrete
    /// peer-reported object; malformed or mismatched ACKs return a decoding error.
    pub async fn read_property(
        &self,
        to: &(impl ToStation + ?Sized),
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        let request = bacnet_services::read_property::ReadPropertyRequest {
            object_identifier,
            property_identifier,
            property_array_index,
        };
        self.call(to, request).await
    }

    /// Read a property from a discovered device, auto-routing if needed.
    pub async fn read_property_from_device(
        &self,
        device_instance: u32,
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        let request = bacnet_services::read_property::ReadPropertyRequest {
            object_identifier,
            property_identifier,
            property_array_index,
        };
        self.call_via(device_target(&mac, &routing), request).await
    }

    /// Read multiple properties from one or more objects on the station
    /// `to`.
    pub async fn read_property_multiple(
        &self,
        to: &(impl ToStation + ?Sized),
        specs: Vec<bacnet_types::constructed::ReadAccessSpecification>,
    ) -> Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error> {
        let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
            list_of_read_access_specs: specs,
        };
        self.call(to, request).await
    }

    /// Read multiple properties from a discovered device, auto-routing if needed.
    pub async fn read_property_multiple_from_device(
        &self,
        device_instance: u32,
        specs: Vec<bacnet_types::constructed::ReadAccessSpecification>,
    ) -> Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
            list_of_read_access_specs: specs,
        };
        self.call_via(device_target(&mac, &routing), request).await
    }

    /// Read one property from multiple discovered devices concurrently.
    ///
    /// All reads are dispatched concurrently (up to `max_concurrent`,
    /// default 32). The nonzero limit prevents a stalled batch. Results are
    /// returned in completion order; `request_index` identifies the original input
    /// occurrence, including duplicates. Dropping the future cancels pending requests
    /// and returns no partial vector.
    pub async fn read_property_from_devices(
        &self,
        requests: Vec<DeviceReadRequest>,
        max_concurrent: Option<NonZeroUsize>,
    ) -> Vec<DeviceReadResult> {
        use futures_util::stream::{self, StreamExt};

        let concurrency = max_concurrent.map_or(DEFAULT_BATCH_CONCURRENCY, NonZeroUsize::get);

        stream::iter(requests.into_iter().enumerate())
            .map(|(request_index, req)| async move {
                let result = self
                    .read_property_from_device(
                        req.device_instance,
                        req.object_identifier,
                        req.property_identifier,
                        req.property_array_index,
                    )
                    .await;
                DeviceReadResult {
                    request_index,
                    device_instance: req.device_instance,
                    result,
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await
    }

    /// Read multiple properties from multiple devices concurrently (RPM batch).
    ///
    /// Sends an RPM to each device concurrently. This is the most efficient
    /// way to poll many properties across many devices — RPM batches within
    /// a single device, and this method batches across devices. A nonzero limit
    /// bounds concurrency (None uses 32). Results use completion order and carry the
    /// original input occurrence in `request_index`. Dropping the future cancels
    /// pending requests and returns no partial vector.
    pub async fn read_property_multiple_from_devices(
        &self,
        requests: Vec<DeviceRpmRequest>,
        max_concurrent: Option<NonZeroUsize>,
    ) -> Vec<DeviceRpmResult> {
        use futures_util::stream::{self, StreamExt};

        let concurrency = max_concurrent.map_or(DEFAULT_BATCH_CONCURRENCY, NonZeroUsize::get);

        stream::iter(requests.into_iter().enumerate())
            .map(|(request_index, req)| async move {
                let result = self
                    .read_property_multiple_from_device(req.device_instance, req.specs)
                    .await;
                DeviceRpmResult {
                    request_index,
                    device_instance: req.device_instance,
                    result,
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await
    }

    /// Write a property on the station `to`.
    pub async fn write_property(
        &self,
        to: &(impl ToStation + ?Sized),
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        property_value: Vec<u8>,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        let request = bacnet_services::write_property::WritePropertyRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            property_value,
            priority,
        };
        self.call(to, request).await
    }

    /// Write multiple properties on one or more objects on the station
    /// `to`.
    pub async fn write_property_multiple(
        &self,
        to: &(impl ToStation + ?Sized),
        specs: Vec<bacnet_services::wpm::WriteAccessSpecification>,
    ) -> Result<(), Error> {
        let request = bacnet_services::wpm::WritePropertyMultipleRequest {
            list_of_write_access_specs: specs,
        };
        self.call(to, request).await
    }

    // -----------------------------------------------------------------------
    // Auto-routing _from_device variants (RPM, WP, WPM)
    // -----------------------------------------------------------------------

    /// Write a property on a discovered device, auto-routing if needed.
    pub async fn write_property_to_device(
        &self,
        device_instance: u32,
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        property_value: Vec<u8>,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        bacnet_services::write_property::validate_priority(priority)?;
        let (mac, routing) = self.resolve_device(device_instance).await?;
        let request = bacnet_services::write_property::WritePropertyRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            property_value,
            priority,
        };
        self.call_via(device_target(&mac, &routing), request).await
    }

    /// Write multiple properties on a discovered device, auto-routing if needed.
    pub async fn write_property_multiple_to_device(
        &self,
        device_instance: u32,
        specs: Vec<bacnet_services::wpm::WriteAccessSpecification>,
    ) -> Result<(), Error> {
        // Refused before the lookup, as a request that can't be encoded.
        let request = Encoded::new(bacnet_services::wpm::WritePropertyMultipleRequest {
            list_of_write_access_specs: specs,
        })?;
        let (mac, routing) = self.resolve_device(device_instance).await?;
        self.send_via(device_target(&mac, &routing), request).await
    }

    /// Write one property on multiple discovered devices concurrently.
    ///
    /// All writes are dispatched concurrently (up to `max_concurrent`,
    /// default 32). The nonzero limit prevents a stalled batch. Results are
    /// returned in completion order; `request_index` identifies the original input
    /// occurrence, including duplicates. Dropping the future cancels pending requests
    /// and returns no partial vector.
    pub async fn write_property_to_devices(
        &self,
        requests: Vec<DeviceWriteRequest>,
        max_concurrent: Option<NonZeroUsize>,
    ) -> Vec<DeviceWriteResult> {
        use futures_util::stream::{self, StreamExt};

        let concurrency = max_concurrent.map_or(DEFAULT_BATCH_CONCURRENCY, NonZeroUsize::get);

        stream::iter(requests.into_iter().enumerate())
            .map(|(request_index, req)| async move {
                let result = self
                    .write_property_to_device(
                        req.device_instance,
                        req.object_identifier,
                        req.property_identifier,
                        req.property_array_index,
                        req.property_value,
                        req.priority,
                    )
                    .await;
                DeviceWriteResult {
                    request_index,
                    device_instance: req.device_instance,
                    result,
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await
    }
}

#[cfg(test)]
mod doc_tests {
    const SOURCE: &str = include_str!("property.rs");

    fn doc_block_for(fn_name: &str) -> String {
        let needle = format!("pub async fn {fn_name}(");
        let lines: Vec<_> = SOURCE.lines().collect();
        let signature_index = lines
            .iter()
            .position(|line| line.trim_start().starts_with(&needle))
            .unwrap_or_else(|| panic!("missing function {fn_name}"));
        let mut docs = Vec::new();
        let mut index = signature_index;
        while index > 0 {
            index -= 1;
            let line = lines[index].trim_start();
            if let Some(doc) = line.strip_prefix("///") {
                docs.push(doc.trim());
            } else {
                break;
            }
        }
        docs.reverse();
        docs.join(" ")
    }

    #[test]
    fn property_method_docblocks_match_their_methods() {
        let cases = [
            (
                "read_property_multiple",
                "Read multiple properties from one or more objects on the station",
                "Write",
            ),
            (
                "read_property_multiple_from_device",
                "Read multiple properties from a discovered device, auto-routing if needed.",
                "Write",
            ),
            (
                "read_property_from_devices",
                "Read one property from multiple discovered devices concurrently.",
                "Write",
            ),
            (
                "write_property",
                "Write a property on the station",
                "multiple devices concurrently",
            ),
            (
                "write_property_multiple",
                "Write multiple properties on one or more objects on the station",
                "Read",
            ),
            (
                "write_property_to_device",
                "Write a property on a discovered device, auto-routing if needed.",
                "Read",
            ),
            (
                "write_property_to_devices",
                "Write one property on multiple discovered devices concurrently.",
                "Read",
            ),
        ];

        for (fn_name, expected, forbidden) in cases {
            let docs = doc_block_for(fn_name);
            assert!(
                docs.contains(expected),
                "{fn_name} docs did not contain expected text: {docs}"
            );
            assert!(
                !docs.contains(forbidden),
                "{fn_name} docs still contain misattributed text: {docs}"
            );
        }
    }
}
