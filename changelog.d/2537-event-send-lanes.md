---
section: Changed
---
- **Wire:** A destination's event notifications go out in commit order,
  whichever task committed them, and a stalled one holds up only its own.
  With Staging plans the SimpleACK now precedes the plans' event and COV
  notifications, and recipient windows are judged at commit (#2537).
