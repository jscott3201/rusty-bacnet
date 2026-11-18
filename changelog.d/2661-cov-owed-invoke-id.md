---
section: Fixed
---
- **Wire:** A confirmed COV notification that finds no invoke ID free no
  longer waits for the next change: its subscription is looked at again once
  an ID comes back, in the order subscriptions were refused, and reports its
  current value if that still differs from the last acknowledged (#2661).
