---
section: Changed
---
- `BACnetObject`'s hidden Averaging sample hook moves to a hidden
  `Timed::take_due_sample`, and the Pulse Converter input hooks leave the
  trait: the database counts the built-in Pulse Converter only, so a wrapped
  one is no longer judged or counted (#2179).
