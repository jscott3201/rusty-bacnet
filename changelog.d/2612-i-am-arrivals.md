---
section: Added
---
- **Python API:** `BACnetClient.i_am_arrivals()` and `BipEndpoint.i_am_arrivals()` yield
  one `IAmArrival` per I-Am received, repeats and duplicate instances kept, with its
  arrival time, link sender, forwarding BBMD and next-hop router; Rust has
  `bacnet_ops::arrival` (#2612).
