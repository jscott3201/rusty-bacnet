---
section: Migration notes
---
- **B/IP registration (Rust API, #939):** a custom `TransportPort` that
  overrode `normal_bip_endpoint` overrides `bip_port` instead, returning a
  `BipPort` with the endpoint and a `BipPortMode`; a wrapper delegates it.
  Lock `bbmd_state()` with `.lock()` rather than `.lock().await`, never
  across an await; it returns a `LockResult`, so `unwrap()` it or take
  `into_inner` on poison. Drop `.await` from `fdt_counters()`.
  `ObjectDatabase::publish_bip_port_internal` takes the mode as a fifth
  argument.
