---
section: Migration notes
---
- **Client (Rust API, #2588):** `read_property_routed(router, net, mac, ..)`
  is `read_property(&Station::routed(router, net, mac), ..)`, and
  `confirmed_request_routed` is `confirmed_request` to such a `Station`. A
  call's first argument is now `&impl ToStation`: a slice, array, `Vec` or
  `MacAddr` passes as before, but one typed only by inference, such as
  `mac.as_ref()`, needs to be written `&mac[..]`.
