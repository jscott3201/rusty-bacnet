---
section: Migration notes
---
- **SC reconnect (Rust and Python API):** Wrap `ScReconnectConfig.max_retries`
  in `Some(..)`; Python's `max_retries` takes an int or None. A count of 0 now
  turns recovery off where it allowed one failover dial: use 1 to keep that
  turn. With defaults the failover hub is dialled about 10 s after a loss, not
  about 50 min. A raw transport with no connector sends one Connect-Request on
  a lost socket, not one per round.
