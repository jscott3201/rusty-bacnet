---
section: Fixed
---
- **Breaking (Rust API, wire):** Tags now accept one primitive value per name,
  following the corrected standard. Combined date-and-time tags are rejected
  on writes and snapshot loads; the obsolete value wrapper is removed (#1583).
