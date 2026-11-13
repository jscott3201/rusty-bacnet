---
section: Changed
---
- **Wire:** A destination's event notifications go out in the order their
  transitions committed, whichever task committed them, and a stalled one
  holds up only its own. With Staging plans a request's notifications no
  longer precede its response, and recipient windows are judged at commit
  (#2537).
