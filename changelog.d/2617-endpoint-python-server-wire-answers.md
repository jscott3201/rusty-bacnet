---
section: Changed
---
- **Breaking (wire):** a Python endpoint answers DeviceCommunicationControl,
  and ReinitializeDevice without a handler, with an Error
  (SERVICE_REQUEST_DENIED) where it sent a Reject, and its COV subscribers
  hear a peer's allowed writes; time synchronization stays off unless asked
  for (#2617).
