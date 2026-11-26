---
section: Security
---
- A following device handle no longer trusts the discovery table alone: a
  write goes only where the device is confirmed, else nowhere, and a read
  moves it only after its old address, asked, is not the device and the new
  one names it (#2795; open: #2823, #2824).
