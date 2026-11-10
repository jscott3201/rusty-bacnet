---
section: Changed
---
- **Rust API:** with `cov-multiple`, `bacnet-device-core`'s `Reporter`
  answers SubscribeCOVPropertyMultiple and queues its reports as
  `NotifyService::CovMultiple` notifications, with the
  Max_Notification_Delay deadline in `next_deadline` (#2320).
