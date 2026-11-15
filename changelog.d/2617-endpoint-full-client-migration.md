---
section: Migration notes
---
- **Endpoint client (Python API, #2617):** `EndpointClient`'s calls keep
  their names on the `BACnetClient` from `endpoint.client()`; drop
  `write_property`'s `commandability=` and `service_scope()` (table in
  [docs/python-api.md](docs/python-api.md#migration-endpointclient-to-the-endpoints-bacnetclient-2617)).
  With `source_audit_reporter`, `client()` raises `NotImplementedError` until
  #2617 step 3, a gate for the next release.
