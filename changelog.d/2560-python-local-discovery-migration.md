---
section: Migration notes
---
- **Python discovery (#2560):** pass `scope="global"` to `who_is()` or
  `discover()` to reach devices behind routers as before, or a network
  number for one network. A client on `transport="router"` must pass one:
  a local Who-Is there raises `ValueError`, as the router's application has
  no network of its own.
