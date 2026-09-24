//! Closed set of endpoint reads sharing transaction and source-reporting ownership.
use super::*;
use bacnet_services::read_range::{ReadRangeAck, ReadRangeRequest};

/// Read identity, validated when preparing the endpoint transaction.
#[doc(hidden)]
#[derive(Clone)]
pub enum EndpointReadRequest {
    Property(ReadPropertyRequest),
    Range(ReadRangeRequest),
}

/// Typed caller payload; source Audit records never retain these values.
#[doc(hidden)]
#[derive(Debug)]
pub enum EndpointReadAck {
    Property(ReadPropertyACK),
    Range(ReadRangeAck),
}

impl EndpointReadRequest {
    #[doc(hidden)]
    pub fn identity(&self) -> (ObjectIdentifier, PropertyIdentifier, Option<u32>) {
        match self {
            Self::Property(r) => (
                r.object_identifier,
                r.property_identifier,
                r.property_array_index,
            ),
            Self::Range(r) => (
                r.object_identifier,
                r.property_identifier,
                r.property_array_index,
            ),
        }
    }

    pub(super) fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        match self {
            Self::Property(r) => {
                r.encode(buf);
                Ok(())
            }
            Self::Range(r) => r.encode(buf),
        }
    }

    pub(super) fn service(&self) -> ConfirmedServiceChoice {
        match self {
            Self::Property(_) => ConfirmedServiceChoice::READ_PROPERTY,
            Self::Range(_) => ConfirmedServiceChoice::READ_RANGE,
        }
    }

    pub(super) fn decode(&self, bytes: &[u8]) -> Result<EndpointReadAck, Error> {
        match self {
            Self::Property(request) => {
                let ack = ReadPropertyACK::decode(bytes)?;
                if ack.object_identifier != request.object_identifier
                    || ack.property_identifier != request.property_identifier
                    || ack.property_array_index != request.property_array_index
                {
                    return Err(Error::Encoding(
                        "ReadProperty ACK does not match request identity".into(),
                    ));
                }
                Ok(EndpointReadAck::Property(ack))
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
    #[doc(hidden)]
    pub fn into_property(self) -> Result<ReadPropertyACK, Error> {
        match self {
            Self::Property(ack) => Ok(ack),
            Self::Range(_) => Err(Error::Encoding("endpoint read result kind mismatch".into())),
        }
    }

    #[doc(hidden)]
    pub fn into_range(self) -> Result<ReadRangeAck, Error> {
        match self {
            Self::Range(ack) => Ok(ack),
            Self::Property(_) => Err(Error::Encoding("endpoint read result kind mismatch".into())),
        }
    }
}
