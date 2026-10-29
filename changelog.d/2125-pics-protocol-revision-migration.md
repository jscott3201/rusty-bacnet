---
section: Migration notes
---
- **PICS (Rust API, #2125, #2137):** Remove `protocol_version`,
  `protocol_revision`, `vendor_name`, `model_name`, `firmware_revision` and
  `application_software_version` from your `PicsConfig` and set them on the
  Device object; `generate_pics` reads them from it, printing them empty
  without one. The vendor ID is the Device's Vendor_Identifier.
  `bacnet_types::PROTOCOL_VERSION` and `PROTOCOL_REVISION` hold the bundled
  values.
