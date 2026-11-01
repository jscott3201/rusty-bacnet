---
section: Fixed
---
- A B/IP broadcast or multicast listener that fails to bind or join its group
  still names the listener, and now keeps the OS error as the I/O error's
  `source()`, so callers can read its error code (#2231).
