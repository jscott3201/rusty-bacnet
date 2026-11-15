---
section: Added
---
- `EndpointSession::full_client` and `full_client_with` give a full `BACnetClient` on an endpoint's one
  transport, beside its server role: discovery, segmented reads, files, COV and the link's own calls,
  with one pool of invoke IDs for the node. `bacnet_ops::Client::over` takes it (#2617).
