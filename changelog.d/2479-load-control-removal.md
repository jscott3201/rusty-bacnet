---
section: Fixed
---
- Removing a Load Control from its database, or replacing it there, during a
  shed request calls `LoadShedder::stop` through a new
  `Timed::retire(Retirement)` hook, and a server start that fails or is
  cancelled releases what it began with `ObjectDatabase::release_outputs`
  (#2479).
