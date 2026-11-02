---
section: Changed
---
- `BACnetObject`'s hidden Averaging sample and Pulse Converter input hooks
  leave the trait: the database takes them from the built-in objects only, so
  a wrapped Averaging or Pulse Converter is no longer sampled or counted
  (#2179).
