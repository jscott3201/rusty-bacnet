---
section: Fixed
---
- The MS/TP frame tap times frames as the master node does, so it no longer
  reports a frame the node gave up after a silence longer than T_frame_abort
  or when it started to send (#1856).
