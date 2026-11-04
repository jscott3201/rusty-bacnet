---
section: Added
---
- `bacnet-device-core`'s `Reporter::remove_for_object` drops a deleted
  object's COV subscriptions, queued and owed reports and confirmed
  notifications, giving their invoke IDs back, so an object created again
  under its identifier reports to nobody (#2384).
