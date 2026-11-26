---
section: Changed
---
- **Breaking (Python API):** a `BACnetClient` BBMD call (`read_bdt`,
  `write_bdt`, `read_fdt`, `delete_fdt_entry`, `register_foreign_device_bvlc`)
  that a stop cuts off after it went out raises `BacnetError` saying the table
  may have changed, not `BacnetTransportError` (#1833).
