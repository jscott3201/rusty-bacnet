---
section: Added
---
- `ClientOptions::with_data_attributes` gives a client whose confirmed
  requests carry those data attributes (BACnet/SC header data options) on
  every frame, retries and segments included; an endpoint session takes
  them through `client_with` (#2723).
