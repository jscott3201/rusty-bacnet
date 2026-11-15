---
section: Added
---
- **Rust API:** `EndpointSession::frames` subscribes to a B/IP-built session's
  frame tap. `TappedFrame` gains `monotonic_timestamp` and `missed_before`, the
  frames that receiver's full queue dropped just before it, and
  `FrameTapReceiver` gains `poll_recv` (#2614).
