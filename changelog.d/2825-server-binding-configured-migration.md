---
section: Migration notes
---
- **Server device bindings (#2825):** configure a `DeviceBinding`
  (`add_device_binding` in Python) for a device that only announces itself,
  or whose alarm notifications must not wait for the server's Who-Is.
