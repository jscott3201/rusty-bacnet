---
section: Migration notes
---
- **223 export (Rust API, #2560):** read with `device.inventory(&s223::inventory_options(types)?)`
  and write `to_turtle(&s223::Model::from(&inventory), ..)`. `Entry` is `Result<Point, Skipped>`,
  `Detail::Full` is `point_list_options()`, `inventory_object` is `point_record`,
  `read_inventory` is `bacnet_ops::raw::read_inventory`, and `WritePriority` is
  `bacnet_ops::Priority`. The table is in [the Rust API notes](docs/rust-api.md#inventory).
