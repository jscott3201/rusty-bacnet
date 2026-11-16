---
section: Changed
---
- **Breaking (Rust API):** every typed `BACnetClient` call takes the station
  it goes to: a MAC as before, a `BACnetAddress`, or a `Station`, which can
  also name the router. One service description sends each, and the requests
  on the wire are unchanged (#2588).
