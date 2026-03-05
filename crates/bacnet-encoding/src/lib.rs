//! BACnet ASN.1/BER encoding and APDU/NPDU codecs per ASHRAE 135-2020.
//!
//! This crate provides the wire-format encoding and decoding for BACnet:
//! - ASN.1 tag encode/decode (Clause 20.2.1)
//! - Application-tagged primitive codecs (Clause 20.2)
//! - APDU encode/decode (Clause 20.1)
//! - NPDU encode/decode (Clause 6)

pub mod apdu;
pub mod npdu;
pub mod primitives;
pub mod segmentation;
pub mod tags;
