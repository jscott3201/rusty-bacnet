---
section: Changed
---
- A BVLC management request cut off by its B/IP transport stopping fails
  as one made after the stop does, with a not-connected transport error
  (Python: `BacnetTransportError`), not an encoding error (#2612).
