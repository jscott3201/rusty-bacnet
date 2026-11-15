---
section: Fixed
---
- The TUI capture and MCP `bacnet_decode` report a truncated BACnet/SC
  BVLC-Result, an unassigned BVLC function or another link's octets as
  malformed, and keep SNET, DNET and the broadcast flag of a routed frame
  whose APDU fails to decode (#2614).
