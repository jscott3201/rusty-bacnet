---
section: Fixed
---
- `BACnetClient::add_device`, and Python's `add_device`, refuse an empty MAC or one longer than 18 octets, as `add_routed_device` does, instead of storing a row whose device-list cursor the MCP server refuses (#2606).
