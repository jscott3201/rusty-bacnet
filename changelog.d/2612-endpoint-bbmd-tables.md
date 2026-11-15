---
section: Added
---
- **Python API:** `BipEndpoint.read_bdt()` and `read_fdt()` read a BBMD's
  tables through the endpoint's own socket, as `BACnetClient`'s do; in Rust,
  `EndpointSession::bvlc_tables()` hands out a `BvlcTableReader` for the same
  reads (#2612).
