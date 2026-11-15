---
section: Fixed
---
- The TUI capture and MCP `bacnet_decode` report a truncated BACnet/SC
  BVLC-Result, an unassigned BVLC function (in the field tree `unassigned`,
  or `withdrawn` for a removed code) or another link's octets as malformed,
  and keep SNET, DNET and broadcast of a routed frame whose APDU fails
  (#2614).
