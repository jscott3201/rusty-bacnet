//! Encoding an [`Npdu`] into the caller's buffer: [`Npdu::encoded_len`]
//! and [`Npdu::encode_into`].

use core::fmt;

use crate::view::{DESTINATION, EXPECTING_REPLY, NETWORK_MESSAGE, SOURCE};
use crate::{Address, Npdu, GLOBAL_NETWORK, MAX_MAC_LEN, VERSION};

/// Why [`Npdu::encode_into`] refused an NPDU. Nothing was written.
///
/// It is `#[non_exhaustive]`, so a new refusal isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodeError {
    /// The buffer is shorter than the NPDU.
    NoRoom {
        /// The NPDU's length, [`Npdu::encoded_len`].
        needed: usize,
        /// The buffer's length.
        available: usize,
    },
    /// A DNET of 0, or an SNET of 0 or 0xFFFF, which no decoder accepts.
    ReservedNetwork,
    /// An empty SADR.
    EmptySource,
    /// A DADR or SADR longer than [`MAX_MAC_LEN`].
    AddressTooLong,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRoom { needed, available } => {
                write!(f, "an NPDU of {needed} octets doesn't fit in {available}")
            }
            Self::ReservedNetwork => f.write_str("NPDU names a reserved network number"),
            Self::EmptySource => f.write_str("NPDU source SLEN=0 is invalid"),
            Self::AddressTooLong => write!(f, "NPDU address exceeds the {MAX_MAC_LEN}-octet limit"),
        }
    }
}

impl core::error::Error for EncodeError {}

impl Npdu<'_> {
    /// The octets [`encode_into`](Self::encode_into) produces: the NPCI the
    /// fields call for, then the payload.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        let address = |address: Option<Address<'_>>| address.map_or(0, |a| 3 + a.mac.len());
        let message = match self.message_type {
            Some(kind) if kind.is_proprietary() => 3,
            Some(_) => 1,
            None => 0,
        };
        2 + address(self.destination)
            + address(self.source)
            + usize::from(self.destination.is_some())
            + message
            + self.payload.len()
    }

    /// Encode the NPDU at the start of `out` and return its length.
    ///
    /// Nothing is written unless all of it fits and its fields are ones
    /// [`Npdu::decode`] accepts. The hop count is written only with a
    /// destination, and the vendor ID only with a proprietary message type
    /// (0 when it is `None`).
    pub fn encode_into(&self, out: &mut [u8]) -> Result<usize, EncodeError> {
        if let Some(destination) = self.destination {
            if destination.network == 0 {
                return Err(EncodeError::ReservedNetwork);
            }
            if destination.mac.len() > MAX_MAC_LEN {
                return Err(EncodeError::AddressTooLong);
            }
        }
        if let Some(source) = self.source {
            if source.network == 0 || source.network == GLOBAL_NETWORK {
                return Err(EncodeError::ReservedNetwork);
            }
            if source.mac.is_empty() {
                return Err(EncodeError::EmptySource);
            }
            if source.mac.len() > MAX_MAC_LEN {
                return Err(EncodeError::AddressTooLong);
            }
        }
        let needed = self.encoded_len();
        let available = out.len();
        let out = out
            .get_mut(..needed)
            .ok_or(EncodeError::NoRoom { needed, available })?;

        let mut control = self.priority.to_raw();
        if self.message_type.is_some() {
            control |= NETWORK_MESSAGE;
        }
        if self.destination.is_some() {
            control |= DESTINATION;
        }
        if self.source.is_some() {
            control |= SOURCE;
        }
        if self.expecting_reply {
            control |= EXPECTING_REPLY;
        }
        let mut cursor = Cursor { out, at: 0 };
        cursor.put(&[VERSION, control]);
        if let Some(destination) = self.destination {
            cursor.put_address(destination);
        }
        if let Some(source) = self.source {
            cursor.put_address(source);
        }
        if self.destination.is_some() {
            cursor.put(&[self.hop_count]);
        }
        if let Some(kind) = self.message_type {
            cursor.put(&[kind.to_raw()]);
            if kind.is_proprietary() {
                cursor.put(&self.vendor_id.unwrap_or(0).to_be_bytes());
            }
        }
        cursor.put(self.payload);
        debug_assert_eq!(cursor.at, needed);
        Ok(needed)
    }
}

/// Appends to a buffer already sized to what goes in it.
struct Cursor<'o> {
    out: &'o mut [u8],
    at: usize,
}

impl Cursor<'_> {
    fn put(&mut self, octets: &[u8]) {
        self.out[self.at..self.at + octets.len()].copy_from_slice(octets);
        self.at += octets.len();
    }

    fn put_address(&mut self, address: Address<'_>) {
        self.put(&address.network.to_be_bytes());
        // At most MAX_MAC_LEN, checked before anything is written.
        self.put(&[address.mac.len() as u8]);
        self.put(address.mac);
    }
}
