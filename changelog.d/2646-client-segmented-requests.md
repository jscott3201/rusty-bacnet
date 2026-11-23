---
section: Fixed
---
- **Wire:** A client that receives segments (the default, and an endpoint session's client
  when its Device does) reassembles a segmented COV, COV-multiple or event notification and
  answers it once; it used to abort the first segment. A requester's Abort ends the transfer
  (#2646).
