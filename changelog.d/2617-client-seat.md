---
section: Added
---
- `EndpointSession::client_with(options)` gives the session's `BACnetClient` with startup options, and
  the client has every call a standalone one has on the endpoint's one transport: discovery, segmented
  reads, files, COV and the link's own calls. `bacnet_ops::Client::over` takes it (#2617).
