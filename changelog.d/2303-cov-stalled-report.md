---
section: Changed
---
- **Wire:** A confirmed COV notification whose sends stall ends half a
  timeout after its retry cycle and holds its subscription off, as one whose
  retries ran out, so a stale retry can't follow a newer report (#2303).
