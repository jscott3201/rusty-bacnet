---
section: Security
---
- The client accepts an answer for a routed device only through the router
  its request went to, so another station on the link can no longer answer
  for that device by naming it as the source; such answers are counted in
  `off_path_answer_drops` (#2831).
