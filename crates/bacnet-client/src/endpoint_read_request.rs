//! Closed set of endpoint reads sharing transaction and source-reporting ownership.
use super::*;
use bacnet_services::read_range::{ReadRangeAck, ReadRangeRequest};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};

/// Read identity, validated when preparing the endpoint transaction.
#[doc(hidden)]
#[derive(Clone)]
pub enum EndpointReadRequest {
    Property(ReadPropertyRequest),
    Range(ReadRangeRequest),
    Multiple(ReadPropertyMultipleRequest),
}

/// Typed caller payload; source Audit records never retain these values.
#[doc(hidden)]
#[derive(Debug)]
pub enum EndpointReadAck {
    Property(ReadPropertyACK),
    Range(ReadRangeAck),
    Multiple(ReadPropertyMultipleACK),
}

impl EndpointReadRequest {
    /// Validate the endpoint's bounded explicit-reference profile before admission.
    #[doc(hidden)]
    pub fn validate(&self) -> Result<(), Error> {
        if let Self::Multiple(request) = self {
            crate::endpoint_rpm::validate_request(request)?;
        }
        Ok(())
    }

    /// Ordered attempted identities, including duplicate occurrences.
    #[doc(hidden)]
    pub fn identities(&self) -> Vec<(ObjectIdentifier, PropertyIdentifier, Option<u32>)> {
        match self {
            Self::Property(r) => vec![(
                r.object_identifier,
                r.property_identifier,
                r.property_array_index,
            )],
            Self::Range(r) => vec![(
                r.object_identifier,
                r.property_identifier,
                r.property_array_index,
            )],
            Self::Multiple(r) => r
                .list_of_read_access_specs
                .iter()
                .flat_map(|s| {
                    s.list_of_property_references.iter().map(|p| {
                        (
                            s.object_identifier,
                            p.property_identifier,
                            p.property_array_index,
                        )
                    })
                })
                .collect(),
        }
    }

    pub(super) fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        match self {
            Self::Property(r) => {
                r.encode(buf);
                Ok(())
            }
            Self::Range(r) => r.encode(buf),
            Self::Multiple(r) => r.encode(buf),
        }
    }

    pub(super) fn service(&self) -> ConfirmedServiceChoice {
        match self {
            Self::Property(_) => ConfirmedServiceChoice::READ_PROPERTY,
            Self::Range(_) => ConfirmedServiceChoice::READ_RANGE,
            Self::Multiple(_) => ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        }
    }

    pub(super) fn decode(&self, bytes: &[u8]) -> Result<EndpointReadAck, Error> {
        match self {
            Self::Property(request) => {
                crate::read_property::decode_ack(request, bytes).map(EndpointReadAck::Property)
            }
            Self::Multiple(request) => {
                crate::endpoint_rpm::decode_ack(request, bytes).map(EndpointReadAck::Multiple)
            }
            Self::Range(request) => {
                let ack = ReadRangeAck::decode(bytes)?;
                crate::read_range::validate_ack(request, &ack)?;
                Ok(EndpointReadAck::Range(ack))
            }
        }
    }
}

impl EndpointReadAck {
    /// Ordered value-free outcomes after complete ACK correlation.
    #[doc(hidden)]
    pub fn audit_results(&self) -> Vec<(ObjectIdentifier, Option<(ErrorClass, ErrorCode)>)> {
        match self {
            Self::Property(ack) => vec![(ack.object_identifier, None)],
            Self::Range(ack) => vec![(ack.object_identifier, None)],
            Self::Multiple(ack) => ack
                .list_of_read_access_results
                .iter()
                .flat_map(|s| {
                    s.list_of_results
                        .iter()
                        .map(|p| (s.object_identifier, p.error))
                })
                .collect(),
        }
    }

    /// Unique Device identity established by successful results in this operation.
    #[doc(hidden)]
    pub fn target_device(&self) -> Option<ObjectIdentifier> {
        let mut devices = self
            .audit_results()
            .into_iter()
            .filter_map(|(object, error)| {
                (error.is_none()
                    && object.object_type() == ObjectType::DEVICE
                    && object.instance_number() != ObjectIdentifier::WILDCARD_INSTANCE)
                    .then_some(object)
            });
        let first = devices.next()?;
        devices.all(|object| object == first).then_some(first)
    }

    #[doc(hidden)]
    pub fn into_multiple(self) -> Result<ReadPropertyMultipleACK, Error> {
        match self {
            Self::Multiple(ack) => Ok(ack),
            _ => Err(Error::Encoding("endpoint read result kind mismatch".into())),
        }
    }

    #[doc(hidden)]
    pub fn into_property(self) -> Result<ReadPropertyACK, Error> {
        match self {
            Self::Property(ack) => Ok(ack),
            _ => Err(Error::Encoding("endpoint read result kind mismatch".into())),
        }
    }

    #[doc(hidden)]
    pub fn into_range(self) -> Result<ReadRangeAck, Error> {
        match self {
            Self::Range(ack) => Ok(ack),
            _ => Err(Error::Encoding("endpoint read result kind mismatch".into())),
        }
    }
}
