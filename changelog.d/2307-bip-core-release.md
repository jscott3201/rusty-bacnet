---
section: Added
---
- `bacnet-bip-core` can release the BBMD a foreign device moves away from: with `Foreign::release_previous`, `Node::retarget` asks it once to delete the device's entry and counts the answer. The B/IP transport's release runs on it (#2307).
