---
section: Added
---
- `bacnet_npdu::Npdu::decode_inline`, the same decode always inlined; the
  router core's receive path uses it, so another caller of `Npdu::decode`
  no longer moves the decode of every received frame out of line (#2829).
