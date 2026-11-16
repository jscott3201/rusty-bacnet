---
section: Migration notes
---
- **Endpoint bursts and older peers (#2635):** a request or answer a full
  endpoint queue drops costs its requester one APDU timeout, in full for a
  peer with a long timeout or no retries. The endpoints' client roles
  default to no retries, so a dropped answer fails that call when its
  timeout runs out; where bursts are expected, raise `queue_capacity` and
  `apdu_retries`.
