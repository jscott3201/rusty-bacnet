---
section: Fixed
---
- **Wire:** A confirmed COV notification that finds no invoke ID free no
  longer waits for the next change: it is looked at again, in the order
  refused, once an ID and an in-flight slot come back. A first notification
  refused by an in-flight limit still waits for the next change (#2661).
