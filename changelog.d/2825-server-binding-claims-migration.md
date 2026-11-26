---
section: Migration notes
---
- **Server device bindings (wire, #2825):** the first write or notification to
  a device the server has not asked about sends a Who-Is and waits for the
  answer, even if the device announced itself. Device_Address_Binding lists
  only configured bindings and devices that answered the server, and a write
  from a device the server has not bound names its address as the source. A
  device that moves is followed after one check (the APDU timeout); configure
  a `DeviceBinding` for one that must never move.
