---
section: Added
---
- **Python API:** `BipEndpoint`, `ScEndpoint` and `MstpEndpoint` take a
  keyword-only `cov` (an `EndpointCov` dict) that turns on the three COV
  subscribe services, and gain `set_present_value_local` and
  `set_reliability_local`, whose changes the subscribers hear (#2490).
