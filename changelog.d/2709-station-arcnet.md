---
section: Added
---
- **Breaking (Rust API):** `Station::arcnet()` marks a station's network as
  ARCNET, so a confirmed request goes to its station 255 and refuses 0, its
  broadcast; `Station`'s remote variants gain an `arcnet` field, and
  `RemoteBroadcast` names the refused forms (#2709).
