---
section: Added
---
- `bacnet-device-core`'s `report::Reporter` reports COV for firmware: it
  answers SubscribeCOV and SubscribeCOVProperty, queues notifications in a
  fixed outbox, sends confirmed ones with retries, and lists the Device's
  subscriptions through `ActiveCovOverlay` (#2303).
