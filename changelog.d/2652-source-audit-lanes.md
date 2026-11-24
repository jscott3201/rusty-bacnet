---
section: Changed
---
- A source-audited session's 64 operation slots are shared out: at most 4
  to one station on the link or one other network, so a burst to a silent
  device no longer stalls audited requests elsewhere, and a request takes
  its pacing turn once admitted (#2652).
