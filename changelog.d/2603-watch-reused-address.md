---
section: Fixed
---
- A watched point no longer takes another device's values at its address:
  only the device's answers to the hub's reads set or change its device;
  notifications and I-Ams are values or cues for a paced read. A sender
  that can't be told apart (another answers for it, or none can say) is
  polled (#2603).
