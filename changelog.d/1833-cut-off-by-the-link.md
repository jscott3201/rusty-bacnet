---
section: Changed
---
- **Breaking (Rust and Python API):** a request that went out before its link
  stopped (a retry or later segment refused, a BVLC request's wait dropped)
  ends as `Error::RequestCutOff { sent: true }`; Python's message for it reads
  "the client or its link stopped while it waited" (#1833).
