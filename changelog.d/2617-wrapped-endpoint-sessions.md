---
section: Added
---
- **Rust API:** `build_session_wrapped` on the B/IP and MS/TP endpoint
  builders and `connect_hub_session_wrapped` on the SC one build a session
  over a wrapper of the builder's transport, keeping its link calls (#2617).
