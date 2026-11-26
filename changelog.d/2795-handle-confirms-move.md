---
section: Security
---
- A write through a following handle never moves it: it goes only where
  the handle holds the device, else nowhere. A read moves it only after its
  old address, asked, isn't the device (one missed question counts) and the
  new one names it (#2795; open: #2823, #2824).
