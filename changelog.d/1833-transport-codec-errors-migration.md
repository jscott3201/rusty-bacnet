---
section: Migration notes
---
- **Transport encode and decode errors (Rust API, #1833):** the BVLL and
  BVLL6 encoders' refusals are `Error::Encode`, with the same text; a
  persisted BDT or B/IP multicast-group file that doesn't decode, and a
  refused BVLC response, are `Error::Decoding`, whose text starts `decoding
  error at offset N: `; an SC direct response too long for its peer is
  `Error::SendTooLong`, with that variant's text.
