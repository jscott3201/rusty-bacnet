---
section: Fixed
---
- Removing a Load Control from its database, or replacing it there, during a
  shed request ends the request and calls `LoadShedder::stop`, through a new
  `Timed::retire` hook the database calls on an object leaving it (#2479).
