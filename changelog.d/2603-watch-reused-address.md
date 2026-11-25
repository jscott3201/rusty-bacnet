---
section: Fixed
---
- A watched point no longer takes another device's values at its address:
  notifications and RPM polls are checked against the point's device, the
  device's own word beats a stale or forged discovery row, and a claim of
  another device there is checked by a read before a point is given up
  (#2603).
