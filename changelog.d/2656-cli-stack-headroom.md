---
section: Fixed
---
- One-shot `bacnet` commands need much less of the main thread's stack: each
  command and session is boxed, so a `read` over BACnet/SC needs 288 KiB in a
  debug build on macOS rather than 640 KiB (#2656).
