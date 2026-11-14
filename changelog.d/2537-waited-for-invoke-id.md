---
section: Changed
---
- **Breaking (Rust API):** `EventNotificationCounters::confirmed_no_invoke_id`
  is replaced by `confirmed_waited_for_invoke_id`, the confirmed event
  notifications that waited for an invoke ID. One that can lease none at all,
  which takes a panic, is logged and dropped uncounted (#2537).
