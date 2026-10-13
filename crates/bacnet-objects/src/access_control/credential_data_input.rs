use std::sync::Arc;

use bacnet_encoding::constructed::{
    encode_authentication_factor, encode_authentication_factor_format,
};
use bacnet_types::constructed::{BACnetAuthenticationFactor, BACnetAuthenticationFactorFormat};

use super::credential_data_input_formats::{
    drop_undeclared, is_declared, is_well_formed, undefined_factor, SupportedFormat,
};
use super::credential_data_input_out_of_service::Reading;
use super::*;

// CredentialDataInputObject (type 37)
// ---------------------------------------------------------------------------

/// BACnet Credential Data Input object (type 37).
///
/// Represents a credential reader device (card reader, biometric scanner, etc.).
/// Its Present_Value is the last factor read, a `BACnetAuthenticationFactor`
/// (Clause 12.36.4), and Update_Time a `BACnetTimeStamp` (Clause 12.36.11);
/// both go out in their Clause 21 forms. The enumerated
/// BACnetAuthenticationStatus belongs to the Access Point's
/// Authentication_Status (Clause 12.31, Table 12-36), not here.
///
/// Supported_Formats and Supported_Format_Classes are BACnetARRAYs of the
/// same size (Clause 12.36.9.1), so the object keeps them as one list of
/// pairs, set by [`Self::set_supported_formats`]. Both take an array index.
/// Present_Value always names one of those formats with its class, or is the
/// UNDEFINED or ERROR factor (Clause 12.36.4; #1249): the module
/// `credential_data_input_formats` has the rule and how each route keeps it.
///
/// While Out_Of_Service is TRUE a client can simulate Present_Value and
/// Reliability by writing them, and the reader's own values come back on the
/// return to service (Table 12-43 footnote 1; #1168). The module
/// `credential_data_input_out_of_service` has the details.
pub struct CredentialDataInputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present_Value, Update_Time and Reliability as served: the reader's,
    /// or a client's simulation while Out_Of_Service is TRUE.
    reading: Reading,
    /// The reader's own values, put aside while Out_Of_Service is TRUE.
    device_reading: Option<Reading>,
    supported_formats: Vec<SupportedFormat>,
    out_of_service: bool,
    /// The Device clock, and the sequence numbers that stand in for it.
    clock: UpdateClock,
}

impl CredentialDataInputObject {
    /// Create a new Credential Data Input object.
    ///
    /// Until the first read, Present_Value is the UNDEFINED factor (format
    /// class 0, no value octets) and Update_Time the unspecified date and
    /// time (Clauses 12.36.4 and 12.36.11). It supports no format until
    /// [`Self::set_supported_formats`] declares some.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::CREDENTIAL_DATA_INPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            reading: Reading {
                present_value: undefined_factor(),
                update_time: never_updated(),
                reliability: Reliability::NO_FAULT_DETECTED,
            },
            device_reading: None,
            supported_formats: Vec::new(),
            out_of_service: false,
            clock: UpdateClock::default(),
        })
    }

    /// Record a factor the reader has read: Present_Value takes `factor` and
    /// Update_Time `update_time`, together, since Clause 12.36.11 moves
    /// Update_Time on every Present_Value update. Reading the same factor
    /// again is such an update.
    ///
    /// The factor must name a declared format with the class declared beside
    /// it, or be the UNDEFINED or ERROR factor with class 0 (Clause 12.36.4);
    /// any other is refused with VALUE_OUT_OF_RANGE and nothing changes.
    ///
    /// A change of Update_Time triggers a SubscribeCOV notification
    /// (Table 13-1). While Out_Of_Service is TRUE a client's simulated values
    /// keep being served, and these take over on the return to service. A
    /// running server's route, `BACnetServer::report_credential_read_local`,
    /// stores a read the same way (#1132).
    pub fn set_present_value(
        &mut self,
        factor: BACnetAuthenticationFactor,
        update_time: BACnetTimeStamp,
    ) -> Result<(), Error> {
        if !is_declared(&factor, &self.supported_formats) {
            return Err(common::value_out_of_range_error());
        }
        let reading = self.device_reading_mut();
        reading.present_value = factor;
        reading.update_time = update_time;
        Ok(())
    }

    /// Declare the formats this reader reads: Supported_Formats takes each
    /// pair's format and Supported_Format_Classes, at the same position, the
    /// format class a factor read in it carries (Clauses 12.36.9 and
    /// 12.36.10). Zero is the class for a format that needs no
    /// differentiation.
    ///
    /// Each format must be a named BACnetAuthenticationFactorType. A CUSTOM
    /// format must name its vendor and that vendor's format number; any other
    /// format may carry those members only as zero (Clause 12.36.9). A list
    /// breaking either rule is refused with VALUE_OUT_OF_RANGE and the
    /// declared formats are kept.
    ///
    /// A Present_Value whose format and class the new list doesn't declare
    /// goes back to the UNDEFINED factor, with Update_Time stamped from the
    /// Device clock, or the object's next sequence number when there is no
    /// usable clock (Clause 12.36.4). Out of service that covers both the
    /// simulated factor served and the reader's factor put aside.
    pub fn set_supported_formats(
        &mut self,
        formats: impl IntoIterator<Item = (BACnetAuthenticationFactorFormat, u32)>,
    ) -> Result<(), Error> {
        let formats: Vec<SupportedFormat> = formats.into_iter().collect();
        if !formats.iter().all(|(format, _)| is_well_formed(format)) {
            return Err(common::value_out_of_range_error());
        }
        self.supported_formats = formats;
        drop_undeclared(&mut self.reading, &self.supported_formats, &mut self.clock);
        if let Some(device) = &mut self.device_reading {
            drop_undeclared(device, &self.supported_formats, &mut self.clock);
        }
        Ok(())
    }

    /// The reader's own values: the ones put aside while out of service,
    /// else the ones served.
    fn device_reading_mut(&mut self) -> &mut Reading {
        self.device_reading.as_mut().unwrap_or(&mut self.reading)
    }
}

impl BACnetObject for CredentialDataInputObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if let Some(result) = crate::common::read_identity_properties!(self, property, array_index)
        {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::CREDENTIAL_DATA_INPUT.to_raw(),
            )),
            // The object runs no intrinsic reporting, so IN_ALARM stays clear.
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(common::compute_status_flags(
                StatusFlags::empty(),
                self.reading.reliability,
                self.out_of_service,
                EventState::NORMAL,
            )),
            p if p == PropertyIdentifier::OUT_OF_SERVICE => {
                Ok(PropertyValue::Boolean(self.out_of_service))
            }
            p if p == PropertyIdentifier::RELIABILITY => {
                Ok(PropertyValue::Enumerated(self.reading.reliability.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                let mut buf = BytesMut::new();
                encode_authentication_factor(&mut buf, &self.reading.present_value);
                Ok(PropertyValue::ApplicationData(buf.to_vec()))
            }
            p if p == PropertyIdentifier::UPDATE_TIME => timestamp_value(&self.reading.update_time),
            p if p == PropertyIdentifier::SUPPORTED_FORMATS => common::read_array(
                self.supported_formats
                    .iter()
                    .map(|(format, _)| {
                        let mut buf = BytesMut::new();
                        encode_authentication_factor_format(&mut buf, format);
                        PropertyValue::ApplicationData(buf.to_vec())
                    })
                    .collect(),
                array_index,
            ),
            p if p == PropertyIdentifier::SUPPORTED_FORMAT_CLASSES => common::read_array(
                self.supported_formats
                    .iter()
                    .map(|&(_, class)| PropertyValue::Unsigned(class.into()))
                    .collect(),
                array_index,
            ),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        // The reader's entry edge is always seen (it starts in service and
        // only this write moves Out_Of_Service), so there is no fallback.
        if let Some(result) = common::write_out_of_service_with_restore(
            &mut self.out_of_service,
            &mut self.reading,
            &mut self.device_reading,
            None,
            property,
            &value,
        ) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if let Some(result) = self.reading.write(
            self.out_of_service,
            &self.supported_formats,
            &mut self.clock,
            property,
            &value,
        ) {
            return result;
        }
        Err(common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        super::metadata_identity::for_credential_data_input_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    /// Table 13-1 lists Credential Data Input, so it takes SubscribeCOV.
    fn supports_cov(&self) -> bool {
        true
    }

    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock.clock = clock;
    }

    /// Take a read the application reports (#1132): Present_Value and
    /// Update_Time together, the time stamped from the Device clock, or the
    /// object's next sequence number, when the report has none. Checked and
    /// stored as [`Self::set_present_value`] stores a read: while
    /// Out_Of_Service is TRUE it replaces the reader's values put aside, and
    /// a client's simulated ones stay served. Any other record is
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    fn report_access_input_internal(&mut self, input: AccessControlInput) -> Result<(), Error> {
        let AccessControlInput::CredentialRead(report) = input else {
            return Err(common::optional_functionality_not_supported_error());
        };
        if !is_declared(&report.factor, &self.supported_formats) {
            return Err(common::value_out_of_range_error());
        }
        let update_time = report.update_time.unwrap_or_else(|| self.clock.stamp());
        self.set_present_value(report.factor, update_time)
    }

    /// The application's Reliability for the reader. Like the other
    /// Reliability carriers, the reader refuses it with WRITE_ACCESS_DENIED
    /// while Out_Of_Service is TRUE, so a client's simulated value stays
    /// served; the return to service brings back the value from before.
    /// A value outside the BACnetReliability production is refused with
    /// VALUE_OUT_OF_RANGE.
    fn set_reliability_internal(&mut self, reliability: Reliability) -> Result<(), Error> {
        if self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        self.reading.reliability = super::checked_reliability(reliability)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
