---
section: Migration notes
---
- **Breaking (Rust API, #2689):** `BACnetClient::who_is_router_to_network(network)` only broadcasts; send to one router with `who_is_router_to_network_directed(router, network)` or collect with `discover_routers_directed(router, network, wait)`, which refuse an address that names no one station. `bacnet_ops::Client::discover_routers` takes a `RouterDiscovery` (`local()` or `directed(router)`, then `.network(n)`, `.wait(..)`, `.deadline(..)`).
