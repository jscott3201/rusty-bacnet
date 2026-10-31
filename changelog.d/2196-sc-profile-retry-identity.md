---
section: Fixed
---
- Retrying a new BACnet/SC profile's save in the TUI after only its directory
  sync failed now succeeds: the editor keeps the VMAC and device UUID the
  first attempt wrote, instead of drawing new ones that the saved file refuses
  (#2196).
