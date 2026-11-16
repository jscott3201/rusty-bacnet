---
section: Migration notes
---
- **Endpoint full client timing (Rust and Python, #2617):** the session's
  timing also sets an endpoint full client's segment timeout, and how long
  the next request through the same router to the same network waits after
  a routed request that lost a frame: the timeout times (retries + 1), up to
  24 s at the new defaults where it was 1 s at the old Rust ones.
