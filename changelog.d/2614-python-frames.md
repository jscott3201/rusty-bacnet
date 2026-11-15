---
section: Added
---
- **Python API:** `BipEndpoint.frames()` and `BACnetClient.frames()` (B/IP,
  B/IPv6, SC and MS/TP clients) observe raw frames on the owner's own
  transport, in bounded queues that report each gap, with capture times on
  `time.monotonic()` and the wall clock (#2614).
