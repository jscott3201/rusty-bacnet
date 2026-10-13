//! Who-Am-I and You-Are services per ASHRAE 135-2020 Clause 16.11.
//!
//! Both requests use only application tags, as in the Who-Am-I-Request and You-Are-Request
//! productions of Clause 21.3.3.

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use bacnet_encoding::constructed::{check_decoded_mac_len, check_encoded_mac_len};
use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use bacnet_encoding::constructed::tagged::{
    decode_app_character_string, decode_app_object_id, decode_app_primitive, decode_app_unsigned,
    next_is_application,
};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// What a request calls the members that start it in errors, each written
/// out whole, so a request that decodes builds no label (#1645).
struct IdentityLabels {
    vendor_id: &'static str,
    model_name: &'static str,
    serial_number: &'static str,
}

/// A Who-Am-I's.
const WHO_AM_I: IdentityLabels = IdentityLabels {
    vendor_id: "WhoAmI vendor-id",
    model_name: "WhoAmI model-name",
    serial_number: "WhoAmI serial-number",
};

/// A You-Are's.
const YOU_ARE: IdentityLabels = IdentityLabels {
    vendor_id: "YouAre vendor-id",
    model_name: "YouAre model-name",
    serial_number: "YouAre serial-number",
};

/// Decode the vendor ID, model name and serial number that start both requests.
fn decode_identity(
    data: &[u8],
    labels: &IdentityLabels,
) -> Result<(u16, String, String, usize), Error> {
    let (vendor_id, offset) = decode_app_unsigned::<u16>(data, 0, labels.vendor_id)?;
    let (model_name, offset) = decode_app_character_string(data, offset, labels.model_name)?;
    let (serial_number, offset) = decode_app_character_string(data, offset, labels.serial_number)?;
    Ok((vendor_id, model_name, serial_number, offset))
}

fn encode_identity(
    buf: &mut BytesMut,
    vendor_id: u16,
    model_name: &str,
    serial_number: &str,
) -> Result<(), Error> {
    primitives::encode_app_unsigned(buf, u64::from(vendor_id));
    primitives::encode_app_character_string(buf, model_name)?;
    primitives::encode_app_character_string(buf, serial_number)
}

// ---------------------------------------------------------------------------
// WhoAmIRequest
// ---------------------------------------------------------------------------

/// Who-Am-I-Request service parameters (Clause 16.11.1, Table 16-13; encoding in Clause 21.3.3).
///
/// Fields, in order, all application-tagged and all mandatory: vendor identifier (Unsigned16),
/// model name (CharacterString) and serial number (CharacterString).
///
/// Vendor 260, model "M" and serial "S" encode as
/// `22 01 04 72 00 4D 72 00 53`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoAmIRequest {
    /// Vendor identifier of the requesting device; equals its Vendor_Identifier property.
    pub vendor_id: u16,
    /// Model name of the requesting device; equals its Model_Name property.
    pub model_name: String,
    /// Serial number of the requesting device; equals its Serial_Number property.
    pub serial_number: String,
}

impl WhoAmIRequest {
    /// Encode the request parameters into `buf`; fails, leaving `buf` untouched, if a character
    /// string cannot be encoded.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        let mut scratch = BytesMut::new();
        encode_identity(
            &mut scratch,
            self.vendor_id,
            &self.model_name,
            &self.serial_number,
        )?;
        buf.extend_from_slice(&scratch);
        Ok(())
    }

    /// Decode the request from service-request octets.
    ///
    /// Fails on missing, mistagged or truncated fields and on trailing data.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (vendor_id, model_name, serial_number, offset) = decode_identity(data, &WHO_AM_I)?;
        if offset != data.len() {
            return Err(Error::decoding(offset, "WhoAmI has trailing data"));
        }
        Ok(Self {
            vendor_id,
            model_name,
            serial_number,
        })
    }
}

// ---------------------------------------------------------------------------
// YouAreRequest
// ---------------------------------------------------------------------------

/// You-Are-Request service parameters (Clause 16.11.3, Table 16-14; encoding in Clause 21.3.3).
///
/// Fields, in order, all application-tagged: vendor identifier (Unsigned16), model name
/// (CharacterString) and serial number (CharacterString), all mandatory; then a device
/// identifier (ObjectIdentifier) and a device MAC address (OctetString), both optional.
///
/// At least one of `device_identifier` and
/// `device_mac_address` must be present; both [`encode`](Self::encode) and
/// [`decode`](Self::decode) reject a request with neither. The identifier, when present,
/// must name a Device object.
///
/// The MAC address is the one the matching device takes on the port the request arrived on
/// (Clauses 16.11.3.1.5 and 16.11.4; Clause 19.7.2), so it names a node on one of
/// that device's data links. Like a `BACnetAddress` MAC it is therefore at most
/// [`BACnetAddress::MAX_MAC_LEN`] octets in both directions (#1200): no data link this stack
/// serves needs more, and a longer one could not be valid for any receiving device.
///
/// [`BACnetAddress::MAX_MAC_LEN`]: bacnet_types::constructed::BACnetAddress::MAX_MAC_LEN
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YouAreRequest {
    /// Vendor identifier (Unsigned16) of the device that should act on the request.
    pub vendor_id: u16,
    /// Model name the target device reports in its Device object.
    pub model_name: String,
    /// Serial number the target device reports in its Device object.
    pub serial_number: String,
    /// Device object identifier to assign to the target device; `None` leaves it unchanged.
    pub device_identifier: Option<ObjectIdentifier>,
    /// MAC address to configure on the target device, at most
    /// [`BACnetAddress::MAX_MAC_LEN`] octets; `None` leaves it unchanged.
    ///
    /// [`BACnetAddress::MAX_MAC_LEN`]: bacnet_types::constructed::BACnetAddress::MAX_MAC_LEN
    pub device_mac_address: Option<Vec<u8>>,
}

impl YouAreRequest {
    /// Encode the request parameters into `buf`.
    ///
    /// Fails, leaving `buf` untouched, if neither optional field is present, the identifier
    /// is not a Device object, the MAC address is longer than 18 octets, or a character string
    /// cannot be encoded.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        if self.device_identifier.is_none() && self.device_mac_address.is_none() {
            return Err(Error::Encoding(
                "YouAre needs a device identifier, a device MAC address, or both".into(),
            ));
        }
        if let Some(oid) = &self.device_identifier {
            if oid.object_type() != ObjectType::DEVICE {
                return Err(Error::Encoding(
                    "YouAre device identifier must name a Device object".into(),
                ));
            }
        }
        if let Some(mac) = &self.device_mac_address {
            check_encoded_mac_len(mac, "YouAre device-mac-address")?;
        }
        // Encode into a scratch buffer so a string error leaves `buf` unchanged.
        let mut scratch = BytesMut::new();
        encode_identity(
            &mut scratch,
            self.vendor_id,
            &self.model_name,
            &self.serial_number,
        )?;
        if let Some(oid) = &self.device_identifier {
            primitives::encode_app_object_id(&mut scratch, oid);
        }
        if let Some(mac) = &self.device_mac_address {
            primitives::encode_app_octet_string(&mut scratch, mac);
        }
        buf.extend_from_slice(&scratch);
        Ok(())
    }

    /// Decode the request from service-request octets.
    ///
    /// Fails on missing, mistagged or truncated fields, on a request carrying neither optional
    /// field, on a MAC address longer than 18 octets, and on trailing data.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (vendor_id, model_name, serial_number, mut offset) = decode_identity(data, &YOU_ARE)?;

        let mut device_identifier = None;
        if next_is_application(data, offset, tags::app_tag::OBJECT_IDENTIFIER)? {
            let (oid, end) = decode_app_object_id(data, offset, "YouAre device-identifier")?;
            if oid.object_type() != ObjectType::DEVICE {
                return Err(Error::decoding(
                    offset,
                    "YouAre device identifier must name a Device object",
                ));
            }
            device_identifier = Some(oid);
            offset = end;
        }

        let mut device_mac_address = None;
        if next_is_application(data, offset, tags::app_tag::OCTET_STRING)? {
            let what = "YouAre device-mac-address";
            let (content, end) =
                decode_app_primitive(data, offset, tags::app_tag::OCTET_STRING, what)?;
            check_decoded_mac_len(content.len(), offset, what)?;
            device_mac_address = Some(content.to_vec());
            offset = end;
        }

        if offset != data.len() {
            return Err(Error::decoding(
                offset,
                "YouAre has trailing or unexpected data",
            ));
        }
        if device_identifier.is_none() && device_mac_address.is_none() {
            return Err(Error::decoding(
                offset,
                "YouAre needs a device identifier, a device MAC address, or both",
            ));
        }

        Ok(Self {
            vendor_id,
            model_name,
            serial_number,
            device_identifier,
            device_mac_address,
        })
    }
}

#[cfg(test)]
#[path = "who_am_i_tests.rs"]
mod tests;
