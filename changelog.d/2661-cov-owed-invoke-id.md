---
section: Fixed
---
- **Wire:** A confirmed COV notification that finds no invoke ID free no
  longer waits for the next change: it is looked at again, in the order
  refused, once an ID and an in-flight slot come back (#2661).
