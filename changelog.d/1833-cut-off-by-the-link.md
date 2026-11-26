---
section: Changed
---
- **Breaking (Rust and Python API):** a request that went out before its link
  stopped or lost its connection (a retry or later segment refused, a BVLC
  request's wait dropped) ends as `Error::RequestCutOff { sent: true }`; in
  Python the cut-off message now says whether the request went out (#1833).
