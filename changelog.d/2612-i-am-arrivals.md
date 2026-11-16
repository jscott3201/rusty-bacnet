---
section: Added
---
- **Python API:** `BACnetClient.i_am_arrivals()` (standalone, or on any endpoint's link)
  and `BipEndpoint.i_am_arrivals()` yield one `IAmArrival` per I-Am received, repeats
  and duplicate instances kept, with arrival time, sender, forwarding BBMD and router;
  Rust has `bacnet_ops::arrival` (#2612).
