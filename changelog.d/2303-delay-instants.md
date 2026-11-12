---
section: Changed
---
- **Wire:** A Time_Delay runs from the instant its condition began: a
  transition that a write seeds between the server's one-second sweeps goes
  out when its delay ends, up to a second later than before, rather than at
  the next whole-second sweep (#2303).
