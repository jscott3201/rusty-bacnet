---
section: Fixed
---
- A watched point no longer takes another device's values at its address:
  only the device's answers to the hub's own reads set or change the
  point's device; notifications and announcements are values or cues for
  a paced read. A sender that answers there too is polled (#2603).
