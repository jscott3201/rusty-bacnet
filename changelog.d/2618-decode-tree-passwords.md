---
section: Security
---
- In a DeviceCommunicationControl or ReinitializeDevice request its decoder
  refused, the frame decode tree (TUI capture, MCP `bacnet_decode`) shows
  nothing read from the password's octets: from the password on it is one
  hidden leaf, and the decoder's reason is hidden (#2618).
