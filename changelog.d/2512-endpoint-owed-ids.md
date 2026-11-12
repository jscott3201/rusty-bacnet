---
section: Fixed
---
- The endpoint's COV reporting sends each confirmed report waiting for an
  invoke ID as one comes back to its pool, not only the first, and no longer
  misses an ID that comes back while it is sending (#2512).
