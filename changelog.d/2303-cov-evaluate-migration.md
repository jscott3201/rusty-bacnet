---
section: Migration notes
---
- **COV reports (Rust API, #2303):** `CovTable::begin_confirmed` takes each
  carried reference with the `CovObservation` it reports, and `delivered`
  commits those on `Delivery::Acknowledged` and returns the references owed
  a fresh look; call `complete_observation` only for reports you track
  yourself. Match the new `Delivery::Sent`, which commits an unconfirmed
  report `evaluate` handed out.
