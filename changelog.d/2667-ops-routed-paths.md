---
section: Added
---
- `bacnet_ops::Client` shows and sets routed path size limits in decoded form (`routed_paths`, `set_routed_path_limit`, `clear_routed_path_limit`), and `Device::routed_path` names the path a device behind a router is reached by. `bacnet_ops::raw::routed_paths` and `raw::routed_path_to` do the same from a bare `BACnetClient`, and `bacnet_ops::client::DEFAULT_ROUTED_PATH_MAX_NPDU` is the default limit (228 octets) (#2667).
