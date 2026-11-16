---
section: Changed
---
- **Breaking (Rust API):** `bacnet_ops::decode_npdu_layer` keeps an I-Am's decoded
  parameters (`DecodedApdu::i_am`, replacing `device`) and the SADR, and the frame
  decoder's `Link` the typed origin and source MAC (#2612).
