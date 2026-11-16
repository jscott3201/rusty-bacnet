---
section: Migration notes
---
- **Client (Rust API, #2588):** `read_property_routed(router, net, mac, ..)`
  is `read_property(&Station::routed(router, net, mac), ..)`, and
  `confirmed_request_routed` is `confirmed_request` to such a `Station`. A
  call's first argument is now `&impl ToStation`: a slice, array, `Vec` or
  `MacAddr` passes as before; `mac.as_ref()` and a type that only derefs to
  `[u8]` (`Bytes`, `Box<[u8]>`, `Cow<[u8]>`) are written `&mac[..]`.
