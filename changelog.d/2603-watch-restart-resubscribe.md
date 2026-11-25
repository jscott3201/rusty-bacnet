---
section: Changed
---
- A watched device that announces itself (as one does after a restart) has
  its points read and subscribed again at once, at most once per
  subscription lifetime and within the request limits, instead of at their
  next renewal (#2603).
