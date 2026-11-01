---
section: Changed
---
- **Wire:** `bacnet_device_core::SERVICES` now lists ReadPropertyMultiple
  and WritePropertyMultiple, so a database built with
  `with_services(SERVICES)` declares them in Protocol_Services_Supported
  (#2178).
