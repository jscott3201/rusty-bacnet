---
section: Fixed
---
- **Wire:** A Schedule removed by the application's own `ObjectDatabase::remove`
  now relinquishes its slots, and a Schedule's relinquish leaves a slot that
  another writer has taken since (#2264).
