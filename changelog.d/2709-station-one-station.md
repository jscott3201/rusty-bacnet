---
section: Added
---
- **Breaking (Rust API):** `Station::one_station()` marks a remote station's
  address one station, the way past the broadcast forms
  `bacnet_npdu::RemoteBroadcast` names for a station that is at one, such as
  ARCNET's 255; `Station`'s remote variants gain a `one_station` field
  (#2709, #2738).
