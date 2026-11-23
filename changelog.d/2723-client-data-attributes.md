---
section: Added
---
- `BACnetClient::with_data_attributes` gives a view whose confirmed
  requests carry those data attributes (BACnet/SC header data options) on
  every frame, retries and segments included, as the endpoint's removed
  requester's reads took them (#2723).
