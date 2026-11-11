---
section: Migration notes
---
- **223 export (Rust API, #2445):** `Inventory.device` is a `DeviceInfo` (`device_name` is now `device.name`) and `objects` is `entries`, each an `Entry::Point` or an `Entry::Failed`; `InventoryObject` is gone, its `priority_array` now the point's `Access`. `Engine::inventory_device_name` is `inventory_device(device, detail)`, and `inventory_object` takes a `Detail`.
