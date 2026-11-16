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
}

#[cfg(test)]
#[path = "object_mgmt_tests.rs"]
mod tests;
