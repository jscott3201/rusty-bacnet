---
section: Security
---
- In a DeviceCommunicationControl or ReinitializeDevice request its decoder
  refused, or its first segment, the frame decode tree (TUI capture, MCP
  `bacnet_decode`) shows only the fields before the password that match the
  layout; the rest, password and reason included, is hidden (#2618).
