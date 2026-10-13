//! Closed service family sharing one transaction owner.
use super::*;
use bacnet_services::write_property::{validate_priority, WritePropertyRequest};

/// Validated payload identity for the endpoint's supported operations.
#[doc(hidden)]
#[derive(Clone)]
pub enum EndpointOperationRequest {
    Read(EndpointReadRequest),
    Write(WritePropertyRequest),
}

/// Typed terminal success; writes never manufacture read data.
#[doc(hidden)]
#[derive(Debug)]
pub enum EndpointOperationAck {
    Read(EndpointReadAck),
    Write,
}

impl EndpointOperationRequest {
    /// Check framing and typed parameters without consuming an Invoke ID.
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Read(request) => request.validate(),
            Self::Write(request) => {
                validate_priority(request.priority)?;
                bacnet_encoding::constructed::validate_tlv_sequence(
                    &request.property_value,
                    "endpoint WriteProperty value",
                )
            }
        }
    }

    /// Ordered requested identities; writes have exactly one occurrence.
    pub fn identities(&self) -> Vec<(ObjectIdentifier, PropertyIdentifier, Option<u32>)> {
        match self {
            Self::Read(request) => request.identities(),
            Self::Write(request) => vec![(
                request.object_identifier,
                request.property_identifier,
                request.property_array_index,
            )],
        }
    }

    pub(super) fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        match self {
            Self::Read(request) => request.encode(buf),
            Self::Write(request) => request.encode(buf),
        }
    }
    pub(super) fn service(&self) -> ConfirmedServiceChoice {
        match self {
            Self::Read(request) => request.service(),
            Self::Write(_) => ConfirmedServiceChoice::WRITE_PROPERTY,
        }
    }
    pub(super) fn terminal_policy(&self) -> TerminalPolicy {
        match self {
            Self::Read(_) => TerminalPolicy::ComplexAck,
            Self::Write(_) => TerminalPolicy::SimpleAck,
        }
    }
    pub(super) fn decode(&self, bytes: &[u8]) -> Result<EndpointOperationAck, Error> {
        match self {
            Self::Read(request) => request.decode(bytes).map(EndpointOperationAck::Read),
            Self::Write(_) => Ok(EndpointOperationAck::Write),
        }
    }
}
impl EndpointOperationAck {
    /// Recover read-only result metadata for source projection.
    pub fn as_read(&self) -> Option<&EndpointReadAck> {
        match self {
            Self::Read(ack) => Some(ack),
            Self::Write => None,
        }
    }
    pub fn into_read(self) -> Result<EndpointReadAck, Error> {
        match self {
            Self::Read(ack) => Ok(ack),
            Self::Write => Err(Error::Encoding(
                "endpoint operation result kind mismatch".into(),
            )),
        }
    }
    pub fn into_write(self) -> Result<(), Error> {
        match self {
            Self::Write => Ok(()),
            Self::Read(_) => Err(Error::Encoding(
                "endpoint operation result kind mismatch".into(),
            )),
        }
    }
    pub fn into_property(self) -> Result<ReadPropertyACK, Error> {
        self.into_read()?.into_property()
    }
    pub fn into_range(self) -> Result<bacnet_services::read_range::ReadRangeReply, Error> {
        self.into_read()?.into_range()
    }
    pub fn into_multiple(self) -> Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error> {
        self.into_read()?.into_multiple()
    }
}
