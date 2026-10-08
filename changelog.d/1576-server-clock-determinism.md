---
section: Fixed
---
- Server reassembly and synchronized Device clocks share the runtime clock, making paused-time tests independent of runner stalls (#1576). Existing receive-expiry policy is unchanged; its protocol correction remains tracked in #1593.
