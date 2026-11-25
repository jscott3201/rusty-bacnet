---
section: Fixed
---
- A watched point no longer takes another device's values at its address:
  each COV notification and RPM poll is checked against the device the
  point watches, and a claim of another device there is checked with one
  read before the point is given up (#2603).
