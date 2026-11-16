---
section: Changed
---
- `bacnet_ops` reads, batches and watches treat a request the node's
  DeviceCommunicationControl stopped as the client's own failure, and the
  `bacnet` tool's write ledger records one stopped before it went out as not
  sent (#2617).
