---
section: Migration notes
---
- **Endpoint timing (Rust and Python, #2617):** `SessionConfig::default()`
  now has `apdu_timeout_ms: 6000, apdu_retries: 3` (was 1000 and 0);
  `BipEndpoint` and `MstpEndpoint` default to `apdu_retries=3` (was 0, the
  timeout stays 6000), and `ScEndpoint` takes the session's. Set the old
  values where a call must fail fast; a seated server and the Device take
  them too.
