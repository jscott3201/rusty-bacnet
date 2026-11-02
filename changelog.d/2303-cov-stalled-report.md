---
section: Changed
---
- **Wire:** A confirmed COV notification whose sends stall ends at its retry
  budget and holds its subscription off, as one whose retries ran out, so a
  stale retry can't follow a newer report (#2303).
