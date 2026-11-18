---
section: Changed
---
- **Breaking (wire):** An endpoint session given no `with_server` builder is
  read-only: writes, creates, deletes, ReinitializeDevice and
  DeviceCommunicationControl get an Error (SERVICE_REQUEST_DENIED) where
  they got a Reject, and time synchronization is ignored (#2617).
