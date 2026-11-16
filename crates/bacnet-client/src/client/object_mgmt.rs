use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Delete an object on the station `to` (DeleteObject, Clause 15.4).
    pub async fn delete_object(
        &self,
        to: &(impl ToStation + ?Sized),
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
    ) -> Result<(), Error> {
        let request = bacnet_services::object_mgmt::DeleteObjectRequest { object_identifier };
        self.call(to, request).await
    }

    /// Create an object on the station `to` (CreateObject, Clause 15.3).
    ///
    /// Returns the identifier the device's CreateObject-ACK names, which
    /// tells a create by type the instance the device chose. An ACK that
    /// doesn't decode is an error, though the device may have created the
    /// object.
    pub async fn create_object(
        &self,
        to: &(impl ToStation + ?Sized),
        object_specifier: bacnet_services::object_mgmt::ObjectSpecifier,
        initial_values: Vec<bacnet_services::common::BACnetPropertyValue>,
    ) -> Result<bacnet_types::primitives::ObjectIdentifier, Error> {
        let request = bacnet_services::object_mgmt::CreateObjectRequest {
            object_specifier,
            list_of_initial_values: initial_values,
        };
        self.call(to, request).await
    }
    /// Delete an object on a discovered device, routing when needed.
    ///
    /// Uses the same confirmation and error semantics as [`Self::delete_object`].
    /// A missing device fails before sending.
    pub async fn delete_object_from_device(
        &self,
        device_instance: u32,
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
    ) -> Result<(), Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        if let Some((network, address)) = routing {
            let request = bacnet_services::object_mgmt::DeleteObjectRequest { object_identifier };
            let mut buf = BytesMut::new();
            request.encode(&mut buf);
            self.confirmed_request_routed(
                &mac,
                network,
                &address,
                ConfirmedServiceChoice::DELETE_OBJECT,
                &buf,
            )
            .await?;
            Ok(())
        } else {
            self.delete_object(&mac, object_identifier).await
        }
    }

    /// Create an object on a discovered device, routing when needed.
    ///
    /// Retains the initial values and decoded acknowledgement of [`Self::create_object`].
    /// A missing device fails before sending.
    pub async fn create_object_from_device(
        &self,
        device_instance: u32,
        object_specifier: bacnet_services::object_mgmt::ObjectSpecifier,
        initial_values: Vec<bacnet_services::common::BACnetPropertyValue>,
    ) -> Result<bacnet_types::primitives::ObjectIdentifier, Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        if let Some((network, address)) = routing {
            let request = bacnet_services::object_mgmt::CreateObjectRequest {
                object_specifier,
                list_of_initial_values: initial_values,
            };
            let mut buf = BytesMut::new();
            request.encode(&mut buf)?;
            let ack = self
                .confirmed_request_routed(
                    &mac,
                    network,
                    &address,
                    ConfirmedServiceChoice::CREATE_OBJECT,
                    &buf,
                )
                .await?;
            created_object(&ack)
        } else {
            self.create_object(&mac, object_specifier, initial_values)
                .await
        }
    }
}

/// The object a CreateObject-ACK names.
fn created_object(ack: &[u8]) -> Result<bacnet_types::primitives::ObjectIdentifier, Error> {
    bacnet_services::object_mgmt::CreateObjectAck::decode(ack).map(|ack| ack.object_identifier)
}

#[cfg(test)]
#[path = "object_mgmt_tests.rs"]
mod tests;
