---
section: Migration notes
---
- **Server device bindings (wire, #2825):** Device_Address_Binding lists only
  configured bindings and devices that answered the server, and a write from
  a device the server never asked about names its address as the source
  (Value_Source, an Audit record's source device), not a Device.
