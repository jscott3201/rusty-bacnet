---
section: Security
---
- The frame decode tree (TUI capture, MCP `bacnet_decode`) shows all the
  parameters of a DeviceCommunicationControl or ReinitializeDevice request
  its decoder refuses, or of its first segment, as one hidden leaf with a
  fixed reason: the password shows only in whether the decoder takes it
  (#2618).
