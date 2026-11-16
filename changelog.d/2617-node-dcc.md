---
section: Changed
---
- **Rust API:** A DeviceCommunicationControl the session's full server accepts
  binds the whole node: its client requests fail with the new
  `Error::InitiationDisabled` until the control ends, unless an operator starts
  a client with `ClientOptions::with_node_dcc_override` (#2617).
