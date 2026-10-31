---
section: Changed
---
- **Breaking (Rust API):** `BACnetObject::property_metadata` is required. Rows
  alone drive the PICS, RPM ALL/REQUIRED/OPTIONAL and result index, the
  indexed-write gate (UNKNOWN_PROPERTY without a row) and the endpoint
  Device-write check; ReadProperty and WriteProperty still reach the object
  (#2181).
