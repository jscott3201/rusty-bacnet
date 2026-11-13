---
section: Changed
---
- A write of a kept reporting row, State_Text included, copies the object's
  kept rows twice as it is made, not three or four times, so a large staged
  request holds the database lock for less time (#2526).
