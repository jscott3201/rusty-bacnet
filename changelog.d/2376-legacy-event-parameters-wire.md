---
section: Changed
---
- **Wire:** A legacy-form Event_Parameters value from an older peer with more
  than one possible end is read up to the first instead of refused, and in a
  standalone property value it now ends before a one-octet priority followed
  by the end or the next value (#2376).
