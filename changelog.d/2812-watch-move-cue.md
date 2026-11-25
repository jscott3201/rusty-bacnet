---
section: Fixed
---
- A watch no longer ends as moved because a following handle moved on
  unsolicited I-Ams, or because the discovery table points elsewhere: the
  hub reads the Device object at the new address, paced, and gives the
  points up as moved only when that answer names their instance (#2812).
