---
section: Changed
---
- A WritePropertyMultiple of many Tags or kept Description writes to one
  object stages and makes each write in the time and memory of what it
  changes, not of the object's whole Tags, so a large request holds the
  database lock for milliseconds rather than seconds (#2546).
